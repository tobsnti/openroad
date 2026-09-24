//! Guild union (alliance) wire opcodes: the three membership requests
//! 0x70FB / 0x70FC / 0x70FD and their acks 0xB0FB / 0xB0FC / 0xB0FD.
//!
//! Idea: the union acks are members of the guild ack cluster
//!-, so they reuse the one body form defined next door
//! in [`crate::agent::guild`] instead of restating it. What is specific to the
//! union is the *request* side, and one shape surprise: **0x70FC has an empty
//! body**, which is easy to mis-file as "layout unknown".
//!
//! `0x3102` (the roster push) is here too: its record is read from the
//! original's own handler, which also settles the published misattribution of
//! that handler. Still true for the whole module: none of these opcodes has
//! been seen on the wire, so the two display-only `u8` of a roster entry stay
//! unnamed rather than being named on a hunch.

use bevy::prelude::Message;
use bytes::{Bytes, BytesMut};

use crate::agent::guild::guild_op_ack;

use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

/// 0x3102 — server → client: the alliance (union) roster push.
///
/// **Was deliberately unmodelled**, on the grounds that its record had never
/// been decoded. It has been now, statically and end to end: the handler is a
/// thunk onto the body reader, and each field name below is pinned by a
/// *consumer*, not by a table:
///
/// * `union_id` / `union_crest_rev` — the header's first two `u32`. The same
///   crest-path builder 0x30FF uses composes `"A%u_%u_%u.crb"` from them, so
///   the pair is an alliance crest, not a guild crest revision.
/// * `leader_guild_id` — the third `u32` is **not** a count: it is looked up
///   in the very map this packet fills, and the window paints that entry in
///   its header line.
/// * `master_name` / `master_object_id` — the entry's `+0x24` and `+0x40` land
///   on the header's labels `+0x794` and, via
///   `CGlobalDataManager::GetTIDFromObjectID`, on one of the four
///   `com_kindred_{china,europe}[16].ddj` race icons. So the `u32` is a
///   character object/ref id and the string beside it is the guild master.
///
/// The alliance ceiling of **8 guilds** comes out of the same window: the
/// literal `8` that feeds its `"%d/%d"` format.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct UnionRoster {
    pub union_id: u32,
    /// Revision of the union crest (`"A%u_%u_%u.crb"`).
    pub union_crest_rev: u32,
    /// The leading guild of the alliance — a key into [`Self::guilds`], not a
    /// count and not an index.
    pub leader_guild_id: u32,
    pub count: u8,
    #[sro_packet(list_type = "by-size-field", size_field = "count")]
    pub guilds: Vec<UnionGuild>,
}

impl UnionRoster {
    /// The alliance ceiling the original's own header line states: the `"%d/%d"`
    /// is fed the literal 8. Used for the
    /// header readout, not to reject a longer list — the server is
    /// authoritative and a truncating parser would lose data we would rather
    /// see in a log.
    pub const MAX_GUILDS: u8 = 8;
}

/// One member guild of the alliance. The original allocates a 0x48-byte record
/// per entry (`push 0x48` @, filled in and keys it by
/// `guild_id`.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct UnionGuild {
    pub guild_id: u32,
    pub guild_name: String,
    /// Record `+0x20`, read verbatim. **Not named on purpose**: the window's
    /// header line never reads it (it only collects the keys into the vector
    /// `wnd+0x7FC`), so nothing in the original says what it means; the two
    /// candidates are guild level and member count. What would settle it: the
    /// row renderer that reads `+0x20` off that vector, **or** a 0x3102 from a
    /// session with an existing alliance.
    pub unknown_a: u8,
    /// The guild master's name — record `+0x24`, drawn on the header label
    /// `+0x794`.
    pub master_name: String,
    /// The guild master's character object/ref id — record `+0x40`; the
    /// header's race icon is looked up from it.
    pub master_object_id: u32,
    /// Record `+0x44`, read verbatim, same story as [`Self::unknown_a`];
    /// candidates are a join status and a leader flag.
    pub unknown_b: u8,
}

/// 0x70FB — client → server: invite a guild into the union.
/// Builder, body `b4`.
///
/// Target-addressed like the guild invite 0x70F3: the `u32` is the invited
/// guild master's spawned entity id, which is why the original only offers the
/// command on a selected player.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct UnionInviteRequest {
    pub target_unique_id: u32,
}

/// 0x70FD — client → server: expel a guild from the union.
/// Builder, body `b4`.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct UnionExpelRequest {
    /// [U] — one `u32`; the builder shows the width, not whether it addresses
    /// the guild or its master.
    pub unk_u32_00: u32,
}

/// 0x70FC — client → server: leave the union. **Empty body.**
///
/// Builder writes no field at all — the server infers the
/// sender's guild. An empty body is a real layout, not a missing one, which is
/// why it is modelled rather than skipped: it is the cheapest possible
/// round-trip anchor for this family.
#[derive(Message, Clone, Debug, Default, PartialEq, Eq)]
pub struct UnionLeaveRequest;

impl TryFrom<Bytes> for UnionLeaveRequest {
    type Error = SerializationError;
    fn try_from(_: Bytes) -> Result<Self, SerializationError> {
        Ok(UnionLeaveRequest)
    }
}

impl From<UnionLeaveRequest> for Bytes {
    fn from(_: UnionLeaveRequest) -> Self {
        BytesMut::new().freeze()
    }
}

guild_op_ack! {
    /// 0xB0FB — ack for [`UnionInviteRequest`], handler.
    UnionInviteAck
}

guild_op_ack! {
    /// 0xB0FC — ack for [`UnionLeaveRequest`]
    /// (the server writer emits the single byte `01`).
    UnionLeaveAck
}

guild_op_ack! {
    /// 0xB0FD — ack for [`UnionExpelRequest`]
    /// (same body form).
    UnionExpelAck
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The roster's field order, byte for byte, in the order
    /// reads it: three header `u32`, the count, then per entry
    /// `u32 / strA / u8 / strA / u32 / u8`. A wrong order here would silently
    /// swap a guild name with its master's — both are `strA`, so only the
    /// byte-level assertion catches it.
    #[test]
    fn the_union_roster_reads_the_originals_field_order() {
        let mut wire = Vec::new();
        wire.extend_from_slice(&9u32.to_le_bytes()); // union_id
        wire.extend_from_slice(&3u32.to_le_bytes()); // union_crest_rev
        wire.extend_from_slice(&77u32.to_le_bytes()); // leader_guild_id
        wire.push(2); // count
        for (id, name, a, master, oid, b) in [
            (77u32, "OpenRoad", 5u8, "Mira", 0x2D877u32, 1u8),
            (78, "Roadmen", 4, "Grunt", 0x2D900, 0),
        ] {
            wire.extend_from_slice(&id.to_le_bytes());
            wire.extend_from_slice(&(name.len() as u16).to_le_bytes());
            wire.extend_from_slice(name.as_bytes());
            wire.push(a);
            wire.extend_from_slice(&(master.len() as u16).to_le_bytes());
            wire.extend_from_slice(master.as_bytes());
            wire.extend_from_slice(&oid.to_le_bytes());
            wire.push(b);
        }
        let bytes = Bytes::from(wire);

        let roster = UnionRoster::try_from(bytes.clone()).unwrap();
        assert_eq!(roster.union_id, 9);
        assert_eq!(roster.union_crest_rev, 3);
        assert_eq!(roster.leader_guild_id, 77);
        assert_eq!(roster.count, 2);
        assert_eq!(roster.guilds.len(), 2);
        assert_eq!(roster.guilds[0].guild_name, "OpenRoad");
        assert_eq!(roster.guilds[0].master_name, "Mira");
        assert_eq!(roster.guilds[0].master_object_id, 0x2D877);
        assert_eq!(roster.guilds[0].unknown_a, 5);
        assert_eq!(roster.guilds[0].unknown_b, 1);
        assert_eq!(roster.guilds[1].guild_id, 78);
        // The round trip is what pins the order: re-encoding must reproduce the
        // exact bytes, not merely a struct that compares equal.
        assert_eq!(Bytes::from(roster), bytes);
    }

    /// The ceiling comes from the original's own header format string, so it is
    /// stated once and asserted rather than sprinkled as an 8.
    #[test]
    fn the_alliance_ceiling_is_the_windows_own_eight() {
        assert_eq!(UnionRoster::MAX_GUILDS, 8);
    }

    /// The empty body is the point: it encodes to nothing and decodes from
    /// nothing, and a stray tail does not turn it into an error.
    #[test]
    fn the_union_leave_request_has_no_body() {
        let wire: Bytes = UnionLeaveRequest.into();
        assert!(wire.is_empty());
        assert_eq!(
            UnionLeaveRequest::try_from(wire).unwrap(),
            UnionLeaveRequest
        );
    }

    /// Invite and expel are one `u32` each, little-endian.
    #[test]
    fn invite_and_expel_are_a_single_u32() {
        let wire = Bytes::from_static(&[0x2A, 0x00, 0x00, 0x00]);

        let invite = UnionInviteRequest::try_from(wire.clone()).unwrap();
        assert_eq!(invite.target_unique_id, 0x2A);
        assert_eq!(Bytes::from(invite), wire);

        let expel = UnionExpelRequest::try_from(wire.clone()).unwrap();
        assert_eq!(expel.unk_u32_00, 0x2A);
        assert_eq!(Bytes::from(expel), wire);
    }

    /// The three acks are the guild cluster's shared form, not a union-specific
    /// one — same success byte, same refusal shape.
    #[test]
    fn the_union_acks_are_the_shared_guild_ack_form() {
        let ok = Bytes::from_static(&[0x01]);
        assert!(UnionInviteAck::try_from(ok.clone()).unwrap().is_success());
        assert!(UnionLeaveAck::try_from(ok.clone()).unwrap().is_success());
        assert!(UnionExpelAck::try_from(ok).unwrap().is_success());

        let refused = Bytes::from_static(&[0x02, 0x0E, 0x1C]);
        let ack = UnionExpelAck::try_from(refused.clone()).unwrap();
        assert!(!ack.is_success());
        assert_eq!(ack.error_code, Some(0x1C0E));
        assert_eq!(Bytes::from(ack), refused);
    }
}
