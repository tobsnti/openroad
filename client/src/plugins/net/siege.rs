//! Fortress war (`0x385F`) — the smallest honest consumer.
//!
//! Idea: `0x385F` arrives on **every world-join** and was decoded but dropped
//! (`packets/src/agent/siege.rs`, registered at `packets/src/lib.rs:266`) — the
//! whole client had no consumer for it. This module is that consumer, and it
//! deliberately stops at one chat line: the only thing the packet tells a
//! player today is *which siege period the server is in*, because in every body
//! seen so far each fortress is unowned (all strings empty, both optional flags
//! clear).
//!
//! Where the words come from: **only** `textdata/textuisystem.txt`, by key.
//! The period byte is `SiegePeriodFlag`, and the original's handler tests it as
//! **flags** (`& 1`, `& 2`, `& 4` separately), with `War=1` and
//! `Application=2` both seen live. Each bit therefore gets
//! the client's own label for that period and they are joined, rather than one
//! string per numeric value. A bit outside `0x07` has no label in the data, so
//! it is appended as its raw number instead of being described.
//!
//! What is deliberately **not** turned into prose:
//! - sub `0x34` (`ApplicationPeriodEnd`): its meaning is known (bodyless push,
//!   on the full hour) but **no textuisystem key names that transition**, so
//!   inventing "the application period has ended" would be exactly the
//!   unsourced string ADR-0009 forbids. It is logged with its sub code.
//! - all other 52 arms: undecoded by the packet layer, so they are logged with
//!   their sub code and body length, never translated.
//!
//! HUD-optional by construction: `client/src/plugins/net/**` must run in the
//! headless netcheck harness, which has no `ChatHistory` (see the same note in
//! `net/party.rs`).

use bevy::prelude::*;

use packets::agent::siege::SiegeUpdate;

use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::textdata::ClientUiStrings;

/// `SiegePeriodFlag` bits. `Default=0, War=1, Application=2, Tax=4`; three of
/// the four are confirmed by live pushes and the handler tests them as flags.
const PERIOD_WAR: u8 = 0x01;
const PERIOD_APPLICATION: u8 = 0x02;
const PERIOD_TAX: u8 = 0x04;
const PERIOD_KNOWN_BITS: u8 = PERIOD_WAR | PERIOD_APPLICATION | PERIOD_TAX;

/// `(key, shipped English)` — the fallback is the string that key carries in
/// `textdata/textuisystem.txt`, quoted so offline scenes and the
/// tests show the same words the data ships.
type UiText = (&'static str, &'static str);

/// "Within period of fortress war" — the client's own name for the war period
/// (item-use condition label). Used as a *label*, not as the announcement
/// `UIIT_MSG_FORT_WAR_BEGIN`, which belongs to arm `0x02` and is not what a
/// world-join Info push says.
const PERIOD_WAR_TEXT: UiText = (
    "UIIT_STT_FORT_ITEMUSE_CONDITION_TIME_FORTPERIOD",
    "Within period of fortress war",
);
/// "Application period for Fortress War" — the apply window's own caption for
/// this period (`iffortresswarapplywnd.txt`, §4).
const PERIOD_APPLICATION_TEXT: UiText = (
    "UIIT_STT_FORT_OFFICAL_TEXT2",
    "Application period for Fortress War",
);
/// "Period of tax levying" — the fortress administrator's caption.
const PERIOD_TAX_TEXT: UiText = (
    "UIIT_STT_FORT_MANAGER_TAXLEVY_PERIOD",
    "Period of tax levying",
);
/// "Period other than fortress war" — `Default`, i.e. no bit set.
const PERIOD_NONE_TEXT: UiText = (
    "UIIT_STT_FORT_ITEMUSE_CONDITION_TIME_NOTFORTPERIOD",
    "Period other than fortress war",
);

/// The line the Info arm produces, or `None` when there is nothing to say.
///
/// Never `None` today — `Default` has its own label — but kept as an `Option`
/// so a future "say nothing while nothing is happening" policy has a seam.
fn period_line(period: u8, strings: &ClientUiStrings) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for (bit, text) in [
        (PERIOD_WAR, PERIOD_WAR_TEXT),
        (PERIOD_APPLICATION, PERIOD_APPLICATION_TEXT),
        (PERIOD_TAX, PERIOD_TAX_TEXT),
    ] {
        if period & bit != 0 {
            parts.push(strings.get_or(text.0, text.1).to_string());
        }
    }
    // A bit the data has no word for is reported as a number, the way the
    // party acks report an unmapped error code.
    let unknown = period & !PERIOD_KNOWN_BITS;
    if unknown != 0 {
        parts.push(format!("period {period:#04X}"));
    }
    if parts.is_empty() {
        parts.push(
            strings
                .get_or(PERIOD_NONE_TEXT.0, PERIOD_NONE_TEXT.1)
                .to_string(),
        );
    }
    Some(parts.join(", "))
}

/// Push one line into the chat log when there is one; see `net/party.rs` for
/// why the HUD resource is optional here.
fn report(history: &mut Option<ResMut<ChatHistory>>, text: String) {
    match history {
        Some(history) => history.push(ChatLine::system(text)),
        None => info!("siege (headless): {text}"),
    }
}

/// 0x385F — the one consumer the fortress-war channel has.
pub fn on_siege_update(
    mut reader: MessageReader<SiegeUpdate>,
    mut history: Option<ResMut<ChatHistory>>,
    strings: Option<Res<ClientUiStrings>>,
) {
    let default_strings = ClientUiStrings::default();
    for msg in reader.read() {
        match msg {
            SiegeUpdate::FortressList {
                fortresses,
                period,
                unk_u32_00,
            } => {
                info!(
                    "siege: 0x385F info — {} fortresses {:?}, period {period:#04X}, owning {unk_u32_00}",
                    fortresses.len(),
                    fortresses.iter().map(|f| f.fortress_id).collect::<Vec<_>>(),
                );
                let strings = strings.as_deref().unwrap_or(&default_strings);
                if let Some(line) = period_line(*period, strings) {
                    report(&mut history, line);
                }
            }
            // The meaning is known, but no textuisystem key names the
            // transition — number, not prose.
            SiegeUpdate::ApplicationPeriodEnd => {
                info!("siege: 0x385F sub 0x34 (period transition, no textdata key) — not shown");
            }
            SiegeUpdate::Other { sub, tail } => {
                info!(
                    "siege: 0x385F sub {sub:#04X} not decoded, {} body bytes",
                    tail.len()
                );
            }
        }
    }
}

/// Registers the consumer. Part of the networking core so the line appears
/// wherever siege packets can arrive.
pub struct SiegePlugin;

impl Plugin for SiegePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, on_siege_update);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use bytes::Bytes;
    use packets::agent::siege::FortressStatus;
    use packets::{NetworkExt, Packet};

    fn app_with_chat() -> App {
        let mut app = App::new();
        app.init_resource::<ChatHistory>()
            .add_message::<SiegeUpdate>()
            .add_systems(Update, on_siege_update);
        app
    }

    fn lines(app: &App) -> Vec<String> {
        app.world()
            .resource::<ChatHistory>()
            .iter()
            .map(|line| line.display())
            .collect()
    }

    fn info(period: u8) -> SiegeUpdate {
        SiegeUpdate::FortressList {
            fortresses: vec![FortressStatus {
                fortress_id: 1,
                ..Default::default()
            }],
            period,
            unk_u32_00: 0,
        }
    }

    /// The arm whose meaning is confirmed: one line, and it is the string the data
    /// ships for that key — no decoration of our own.
    #[test]
    fn the_application_period_produces_exactly_one_shipped_line() {
        let mut app = app_with_chat();
        app.world_mut().write_message(info(PERIOD_APPLICATION));
        app.update();
        assert_eq!(lines(&app), vec![PERIOD_APPLICATION_TEXT.1.to_string()]);
    }

    /// The flags reading, and the honest half of it: a bit the data has no word
    /// for arrives as its number, never as prose.
    #[test]
    fn unknown_period_bits_arrive_as_a_number() {
        let mut app = app_with_chat();
        app.world_mut().write_message(info(0x80));
        app.update();
        assert_eq!(lines(&app), vec!["period 0x80".to_string()]);

        // ...and a known bit next to an unknown one keeps both halves.
        let mut app = app_with_chat();
        app.world_mut().write_message(info(PERIOD_WAR | 0x80));
        app.update();
        assert_eq!(
            lines(&app),
            vec![format!("{}, period 0x81", PERIOD_WAR_TEXT.1)]
        );
    }

    /// Sub `0x34` and every undecoded arm stay out of the chat: no key names
    /// them, so there is nothing to say that would not be invented.
    #[test]
    fn undecoded_arms_produce_no_prose() {
        let mut app = app_with_chat();
        app.world_mut()
            .write_message(SiegeUpdate::ApplicationPeriodEnd);
        app.world_mut().write_message(SiegeUpdate::Other {
            sub: 0x0B,
            tail: Bytes::from_static(&[1, 2, 3]),
        });
        app.update();
        assert!(lines(&app).is_empty(), "{:?}", lines(&app));
    }

    /// The system must run where it actually lives: the netcheck harness has no
    /// HUD, and Bevy fails parameter validation rather than skipping a system
    /// with a missing `ResMut`.
    #[test]
    fn it_runs_without_a_hud() {
        let mut app = App::new();
        app.add_message::<SiegeUpdate>()
            .add_systems(Update, on_siege_update);
        assert!(!app.world().contains_resource::<ChatHistory>());
        app.world_mut().write_message(info(PERIOD_WAR));
        app.update();
    }

    /// The packet has to *arrive* at all: a raw 0x385F body goes through the
    /// real opcode registration (`packets/src/lib.rs:266`) and its
    /// `transform_net_event_SiegeUpdate` fan-out, and comes out as our line.
    /// Body: the 91-byte peacetime shape, period byte = 2.
    #[test]
    fn a_raw_0x385f_packet_reaches_the_consumer() {
        let mut body = vec![0x00u8, 3];
        for id in [1u32, 3, 6] {
            body.extend_from_slice(&id.to_le_bytes());
            body.extend_from_slice(&[0, 0]); // name
            body.extend_from_slice(&0u32.to_le_bytes());
            body.extend_from_slice(&[0, 0]); // guild
            body.extend_from_slice(&[0, 0]); // guild master
            body.extend_from_slice(&0u32.to_le_bytes());
            body.extend_from_slice(&0u32.to_le_bytes());
            body.extend_from_slice(&0u32.to_le_bytes());
            body.push(0); // flag a
            body.push(0); // flag b
        }
        body.push(PERIOD_APPLICATION);
        body.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(body.len(), 91, "the Info body is 91 bytes");

        let packet = Packet::deserialize(0x385F, Bytes::from(body)).expect("0x385F is registered");

        let mut app = App::new();
        app.init_resource::<ChatHistory>()
            .add_network_events()
            .add_systems(Update, on_siege_update);
        app.world_mut().write_message(packet);
        app.update();
        assert_eq!(lines(&app), vec![PERIOD_APPLICATION_TEXT.1.to_string()]);
    }
}
