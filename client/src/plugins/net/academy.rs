//! Academy ("Training Camp") net side — the two academy requests the HUD can
//! address today, and the one ack the server may answer with.
//!
//! Idea: this is the same intent-funnel shape `net/guild.rs` uses — the window
//! writes an [`AcademyAction`] message, one system turns it into a packet, and
//! the receive arm lands in a resource the HUD reads. The direction matters
//! (AGENTS.md): nothing here touches a HUD resource, so the headless netcheck
//! harness can build this plugin without a HUD.
//!
//! What is **not** here, and why: `0x3C81` (the academy info push) is
//! deliberately unwired — the original's own handler `FUN_008986c0` reads zero
//! bytes of it, so there is no roster layout to model
//! (`plugins/net/plugin.rs`). That is why
//! [`AcademyMatchBoard`] below is the only academy state the client can hold:
//! the member roster has no wire source in this tree at all.

use bevy::prelude::*;

use packets::agent::academy::{AcademyMatchListRequest, AcademyMatchListResponse};
use packets::agent::ingame::AcademyInviteRequest;
use packets::Packet;

use crate::net::connection::SilkroadConnection;
use crate::plugins::net::agent::AgentConnection;

/// An academy command the UI wants sent.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub enum AcademyAction {
    /// 0x7472 — invite the selected player's spawn id into the academy. The
    /// body is a bare `u32`, and the original's own builder writes the same.
    Invite(u32),
    /// 0x747D — ask for one page of the matching board. The original writes
    /// exactly one byte; that the byte is a page index is a reading of its own
    /// code, not a confirmed fact (`packets/src/agent/academy.rs`).
    MatchListPage(u8),
}

/// The intent-to-opcode mapping, split out so it is testable without a live
/// connection — the same shape [`crate::plugins::net::guild::guild_action_packet`] has.
pub fn academy_action_packet(action: &AcademyAction) -> Packet {
    match action {
        AcademyAction::Invite(unique_id) => Packet::from(AcademyInviteRequest {
            unique_id: *unique_id,
        }),
        AcademyAction::MatchListPage(page) => Packet::from(AcademyMatchListRequest { page: *page }),
    }
}

/// Turn [`AcademyAction`]s into wire packets.
pub fn send_academy_actions(
    mut reader: MessageReader<AcademyAction>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
) {
    let mut pending = reader.read().peekable();
    if pending.peek().is_none() {
        return;
    }
    let Ok(conn) = conn.single() else {
        // Draining without a connection is the point: a queued invite must not
        // fire at a later, unrelated session (the rule `net/guild.rs` states).
        for action in pending {
            warn!("academy: no agent connection, dropping {action:?}");
        }
        return;
    };
    for action in pending {
        info!("academy: sending {action:?}");
        if let Err(e) = conn.get_sender().send(academy_action_packet(action).into()) {
            error!("network: failed to send academy action: {}", e.0);
        }
    }
}

/// What the last `0xB47D` said — the *shape* of it, not its contents.
///
/// The ack's record block is undecoded on purpose: nothing binds the twelve
/// per-record widths to names, and the read order alone cannot say which of
/// the three header bytes is the page index
/// (`packets/src/agent/academy.rs`). So this resource carries exactly what is
/// sourced — the result arm, the raw header, the record byte count and the
/// error code — and no window renders rows from it yet.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub struct AcademyMatchBoard {
    /// `result` of the last ack, `None` before the first one arrives.
    pub result: Option<u8>,
    /// The success arm's three unnamed header bytes, verbatim.
    pub header: Option<[u8; 3]>,
    /// How many bytes of records came with it. A count, not a row count: the
    /// per-record width is unconfirmed, so dividing would invent a number.
    pub record_bytes: usize,
    /// The failure arm's `u16`.
    pub error_code: Option<u16>,
}

/// Receive arm for `0xB47D`.
///
/// The ack is decoded, but what is in its records is not: nothing names those
/// fields yet. Until then this records the shape and logs it, rather than
/// drawing invented rows.
pub fn on_academy_match_list(
    mut reader: MessageReader<AcademyMatchListResponse>,
    mut board: ResMut<AcademyMatchBoard>,
) {
    for ack in reader.read() {
        *board = AcademyMatchBoard {
            result: Some(ack.result),
            header: ack.header,
            record_bytes: ack.records.len(),
            error_code: ack.error_code,
        };
        info!(
            "academy matching board: result={} header={:?} record_bytes={} error={:?} \
             (records stay undecoded until their fields are known)",
            ack.result,
            ack.header,
            ack.records.len(),
            ack.error_code
        );
    }
}

/// The academy net side. HUD-free by construction, so the netcheck harness can
/// build it (AGENTS.md).
pub struct AcademyPlugin;

impl Plugin for AcademyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AcademyMatchBoard>()
            .add_message::<AcademyAction>()
            .add_systems(Update, (send_academy_actions, on_academy_match_list));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    /// The invite is the family's bare-uid funnel, byte for byte.
    #[test]
    fn the_invite_is_a_bare_uid_on_0x7472() {
        let invite = academy_action_packet(&AcademyAction::Invite(0x0001_60AA)).into_serialize();
        assert_eq!(invite.0, 0x7472);
        assert_eq!(invite.1.as_ref(), &[0xAA, 0x60, 0x01, 0x00]);
    }

    /// The page request is the single byte the builder writes.
    #[test]
    fn the_match_list_request_is_one_byte_on_0x747d() {
        let page = academy_action_packet(&AcademyAction::MatchListPage(2)).into_serialize();
        assert_eq!(page.0, 0x747D);
        assert_eq!(page.1.as_ref(), &[2]);
    }

    /// The ack reaches the resource through the real deserializer, and the
    /// records stay a byte count rather than becoming rows.
    #[test]
    fn an_ack_lands_in_the_resource_without_naming_its_records() {
        let mut app = App::new();
        app.init_resource::<AcademyMatchBoard>()
            .add_message::<AcademyMatchListResponse>()
            .add_systems(Update, on_academy_match_list);

        let ack =
            AcademyMatchListResponse::try_from(Bytes::from_static(&[1, 0, 2, 5, 0xAA, 0xBB, 0xCC]))
                .expect("the ack decoder degrades instead of failing");
        app.world_mut().write_message(ack);
        app.update();

        let board = app.world().resource::<AcademyMatchBoard>();
        assert_eq!(board.result, Some(1));
        assert_eq!(board.header, Some([0, 2, 5]));
        assert_eq!(board.record_bytes, 3);
        assert_eq!(board.error_code, None);
    }
}
