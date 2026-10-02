//! Alchemy (elixir / stone / manufacture / dismantle / socket) wire family.
//!
//! Idea: of the twenty opcodes in this block exactly **one** has a published
//! body — dismantle. `docs/re/systems/alchemy.md:44` states it plainly ("the
//! one published layout", `AGENT_ALCHEMY_DISMANTLE.md:1-18`), and the rest of
//! the family is named by two independent opcode catalogs but has **no**
//! recorded field layout anywhere: no `packet_dump/*.log`, no xBot parser
//! ("xBot implements none of these — zero ALCHEMY hits in `Network/Agent.cs`",
//! `alchemy.md:47`), and only call-site byte *counts* in the RE ledger. So
//! this module models dismantle and nothing else, and
//! `docs/net-alchemy.md` carries the other nineteen as a written list with
//! their handler VAs and the reason each stays unwired. Inventing the missing
//! bodies is the one defect the whole RE program exists to avoid (ADR-0009).
//!
//! One of those nineteen is typed here anyway, and for a reason that does not
//! weaken the rule: the 0x7155 request has three bodies the original client sends
//! on it, all of which end on their last slot. Its answer stays unwired.
//!
//! When those layouts do arrive, the ack shape is already decided by the item
//! half: `character_data::EquipmentData` is byte-for-byte the original's item
//! blob, and every `0xB15x` ack re-emits the mutated item
//! (`alchemy.md:135-137`) — reuse that type rather than adding a second one.

use bevy::prelude::Message;
use bytes::{BufMut, Bytes, BytesMut};

use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

/// 0x7157 — client → server "dismantle these inventory slots".
///
/// `{u8 SlotCount, u8[] Slots}` (`AGENT_ALCHEMY_DISMANTLE.md:1-18`, the
/// family's only published body; builders `sro_client.exe@00820ff0`,
/// `@008259a0`). The slots are inventory slot indices, one byte each.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct AlchemyDismantleRequest {
    pub slot_count: u8,
    #[sro_packet(list_type = "by-size-field", size_field = "slot_count")]
    pub slots: Vec<u8>,
}

impl AlchemyDismantleRequest {
    /// Builds the request from the slots, keeping the count in step with them.
    pub fn new(slots: Vec<u8>) -> Self {
        Self {
            slot_count: slots.len() as u8,
            slots,
        }
    }
}

/// [`Alchemy7155Request::Close`]: the window went away with an operation open.
pub const ALCHEMY_7155_OP_CLOSE: u8 = 1;
/// [`Alchemy7155Request::Slots`]: the operation names the slots it works on.
pub const ALCHEMY_7155_OP_SLOTS: u8 = 2;

/// 0x7155 — client → server: run an alchemy operation on the slots laid out in
/// the window, or drop one that is open.
///
/// **Named after the opcode, not after a verb, because it carries two.** The
/// frames measured on 2026-10-01 come from the four-tab window, whose buttons
/// are *Disjoint*, *Dismantle*, *Manufacture* and *Strengthen*:
///
/// * *Disjoint* sends 4 bytes, `02 01 01 1b` (one slot, bag slot 27).
/// * *Manufacture* sends the **same opcode** with 8 bytes.
///
/// *Fuse* does not appear in that window at all — it is the alchemy box's
/// button, and that one sends `0x7150`/`0x7151` with a different body. The
/// earlier name `AlchemyFuseRequest` therefore described something this
/// opcode never did; a verb in the name would have to be wrong for one of the
/// two operations, so the opcode is the name.
///
/// A leading `u8` picks the operation. A body that does not end on the last
/// slot, and an operation this type does not know, are kept whole rather than
/// half-read.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum Alchemy7155Request {
    /// The slots the operation works on, as inventory slot indices.
    Slots {
        /// One byte ahead of the slot count. Its two known values go with the
        /// two buttons that reach this opcode, but what it enumerates is [U],
        /// so it is carried rather than named.
        unknown: u8,
        slots: Vec<u8>,
    },
    /// Sent when the window closes while an operation is still open.
    Close,
    /// Kept as it arrived, so nothing downstream reads a slot that is not there.
    Unknown { raw: Bytes },
}

impl TryFrom<Bytes> for Alchemy7155Request {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        // Without the leading byte there is no operation to keep.
        let op = *value.first().ok_or_else(|| {
            SerializationError::IoError(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "packet too short",
            ))
        })?;
        let body = &value[1..];
        match (op, body) {
            (ALCHEMY_7155_OP_CLOSE, []) => Ok(Alchemy7155Request::Close),
            (ALCHEMY_7155_OP_SLOTS, [unknown, count, slots @ ..])
                if slots.len() == *count as usize =>
            {
                Ok(Alchemy7155Request::Slots {
                    unknown: *unknown,
                    slots: slots.to_vec(),
                })
            }
            _ => Ok(Alchemy7155Request::Unknown { raw: value }),
        }
    }
}

impl From<Alchemy7155Request> for Bytes {
    fn from(p: Alchemy7155Request) -> Self {
        let mut buf = BytesMut::new();
        match p {
            Alchemy7155Request::Slots { unknown, slots } => {
                buf.put_u8(ALCHEMY_7155_OP_SLOTS);
                buf.put_u8(unknown);
                buf.put_u8(slots.len() as u8);
                buf.put_slice(&slots);
            }
            Alchemy7155Request::Close => buf.put_u8(ALCHEMY_7155_OP_CLOSE),
            Alchemy7155Request::Unknown { raw } => return raw,
        }
        buf.freeze()
    }
}

/// The `result` value that means "refused, an error code follows".
///
/// The published dismantle ack is `{u8 result, if result == 2 u16 errorCode}`,
/// the same `result == 2 → u16` idiom the storage and guild-storage acks use
/// (`agent::storage::StorageDataResponse`). `alchemy.md:45` notes it is the
/// *presumed* shape of every `0xB15x` ack — presumed for the siblings, but
/// published for this one, which is why only this one is modelled here.
pub const ALCHEMY_RESULT_ERROR: u8 = 2;

/// 0xB157 — server → client ack for [`AlchemyDismantleRequest`]
/// (handler `sro_client.exe@00872940`).
///
/// The `AlchemyErrorCode` table the published doc links is a dead page, so the
/// code is carried as a raw `u16` and named nowhere — an UNKNOWN kept as data
/// rather than guessed at (`alchemy.md` §9.7).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct AlchemyDismantleResponse {
    pub result: u8,
    #[sro_packet(when = "result == ALCHEMY_RESULT_ERROR")]
    pub error_code: Option<u16>,
}

impl AlchemyDismantleResponse {
    pub fn is_success(&self) -> bool {
        self.result != ALCHEMY_RESULT_ERROR
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    /// No `packet_dump/0xb157.log` exists — the server in reach never answered
    /// a dismantle — so both directions are asserted against the published
    /// byte layout instead of a capture, and the test says so.
    #[test]
    fn dismantle_request_is_a_count_prefixed_slot_list() {
        let request = AlchemyDismantleRequest::new(vec![13, 14, 42]);
        assert_eq!(request.slot_count, 3);
        let bytes: Bytes = request.clone().into();
        assert_eq!(bytes.as_ref(), &[0x03, 0x0D, 0x0E, 0x2A]);
        assert_eq!(AlchemyDismantleRequest::try_from(bytes).unwrap(), request);

        // the degenerate body the original can also build: nothing selected
        let empty = AlchemyDismantleRequest::new(Vec::new());
        let bytes: Bytes = empty.clone().into();
        assert_eq!(bytes.as_ref(), &[0x00]);
        assert_eq!(AlchemyDismantleRequest::try_from(bytes).unwrap(), empty);
    }

    /// The body the original client sends for a single selected slot: one
    /// slot, the inventory index it sits in.
    #[test]
    fn dismantle_request_matches_the_original_clients_body() {
        let wire = Bytes::from_static(&[0x01, 0x0D]);
        let decoded = AlchemyDismantleRequest::try_from(wire.clone()).unwrap();
        assert_eq!(decoded, AlchemyDismantleRequest::new(vec![13]));
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// The three bodies the original client sends on 0x7155: one slot, five
    /// slots, and the close.
    #[test]
    fn the_7155_request_reads_the_slot_and_close_bodies() {
        let wire = Bytes::from_static(&[0x02, 0x01, 0x01, 0x1B]);
        let decoded = Alchemy7155Request::try_from(wire.clone()).unwrap();
        assert_eq!(
            decoded,
            Alchemy7155Request::Slots {
                unknown: 0x01,
                slots: vec![27],
            }
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        let wire = Bytes::from_static(&[0x02, 0x02, 0x05, 0x12, 0x13, 0x17, 0x18, 0x19]);
        let decoded = Alchemy7155Request::try_from(wire.clone()).unwrap();
        assert_eq!(
            decoded,
            Alchemy7155Request::Slots {
                unknown: 0x02,
                slots: vec![18, 19, 23, 24, 25],
            }
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        let wire = Bytes::from_static(&[0x01]);
        let decoded = Alchemy7155Request::try_from(wire.clone()).unwrap();
        assert_eq!(decoded, Alchemy7155Request::Close);
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// A count that does not match the slots that follow is worth less than the
    /// bytes it came in as, so it keeps them.
    #[test]
    fn the_7155_request_keeps_a_body_it_cannot_close() {
        for body in [
            // one slot short of the count
            &[0x02, 0x01, 0x02, 0x1B][..],
            // one slot too many
            &[0x02, 0x01, 0x01, 0x1B, 0x1C][..],
            // the close with a body behind it
            &[0x01, 0x00][..],
            // an operation nobody has a body for
            &[0x03, 0x01, 0x01, 0x1B][..],
        ] {
            let wire = Bytes::copy_from_slice(body);
            let decoded = Alchemy7155Request::try_from(wire.clone()).unwrap();
            assert_eq!(decoded, Alchemy7155Request::Unknown { raw: wire.clone() });
            let back: Bytes = decoded.into();
            assert_eq!(back, wire);
        }
        // Without the leading byte there is not even an operation to keep.
        assert!(Alchemy7155Request::try_from(Bytes::new()).is_err());
    }

    /// `{u8 result, if result == 2 u16 errorCode}` — success is one byte, a
    /// refusal carries the (unnamed) code.
    #[test]
    fn dismantle_response_carries_an_error_code_only_on_result_two() {
        let ok = AlchemyDismantleResponse::try_from(Bytes::from_static(&[0x01])).unwrap();
        assert!(ok.is_success());
        assert_eq!(ok.error_code, None);

        let refused =
            AlchemyDismantleResponse::try_from(Bytes::from_static(&[0x02, 0x0E, 0x1C])).unwrap();
        assert!(!refused.is_success());
        assert_eq!(refused.error_code, Some(0x1C0E));
    }
}
