//! Alchemy (elixir / stone / manufacture / dismantle / socket) wire family.
//!
//! Idea: of the twenty opcodes in this block exactly **one** has a published
//! body — dismantle. `docs/re/systems/alchemy.md:44` states it plainly ("the
//! one published layout", `AGENT_ALCHEMY_DISMANTLE.md:1-18`), and the rest of
//! the family is named by two independent opcode catalogs but has **no**
//! recorded field layout anywhere: no `packet_dump/*.log`, no xBot parser
//! ("xBot implements none of these — zero ALCHEMY hits in `Network/Agent.cs`",
//! `alchemy.md:47`), and only call-site byte *counts* in the RE ledger. So this
//! module models dismantle plus the two **fuse requests** the alchemy box sends
//! (`0x7150` elixir, `0x7151` stone), and `docs/net-alchemy.md` carries the rest
//! as a written list with the reason each stays unwired. Inventing a missing
//! body is the one defect this whole effort exists to avoid, so the
//! two fuse requests below carry their byte layout in a test and their
//! **answers are not modelled here at all** — nothing has been seen to answer
//! them.
//!
//! When those layouts do arrive, the ack shape is already decided by the item
//! half: `character_data::EquipmentData` is byte-for-byte the original's item
//! blob, and every `0xB15x` ack re-emits the mutated item
//! (`alchemy.md:135-137`) — reuse that type rather than adding a second one.

use bevy::prelude::Message;
use bytes::Bytes;

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

/// First byte of **both** fuse request bodies: the server compares it against
/// the literal `2` and sends everything else down the *cancel* arm, so the tag
/// is what separates a fuse from an abort. It is not a count and not a free
/// action id — sharing the value with [`ALCHEMY_ACTION_CANCEL`]'s neighbour is a
/// coincidence of the two directions, so it is spelled out separately.
pub const ALCHEMY_REQUEST_TAG: u8 = 2;

/// The one-byte body that aborts a running fuse, on either opcode.
pub const ALCHEMY_ACTION_CANCEL: u8 = 1;

/// Second byte of `0x7150`: the worker selector. `3` is the elixir fuse, the
/// only value whose arm is known to read the body this request builds.
pub const ALCHEMY_REINFORCE_OP_FUSE: u8 = 3;

/// Second byte of `0x7151`: a magic stone. Which of the two kinds the player is
/// holding is a **client-side classification of the item in the slot**, read off
/// its itemdata type ids — not a choice the window offers.
pub const ALCHEMY_TYPE_MAGIC_STONE: u8 = 4;
/// Second byte of `0x7151`: an attribute stone.
pub const ALCHEMY_TYPE_ATTRIBUTE_STONE: u8 = 5;

/// Smallest fuse the box can express: the equipment plus one material.
///
/// This is a **send-side limit, not just a comment**. On `0x7151` a one-slot
/// body would differ from a bare *cancel* only in length, and the box cannot
/// express such a fuse anyway — the page needs the equipment plus at least one
/// material. Both constructors therefore refuse below this count instead of
/// relying on the caller.
pub const ALCHEMY_MIN_FUSE_SLOTS: usize = 2;

/// The body both fuse requests share: `{u8 tag, u8 selector, u8 count,
/// count × u8 inventory slot}`. The selector is the only difference between the
/// two opcodes — `op` on `0x7150`, `AlchemyType` on `0x7151`.
///
/// The slot list leads with the **equipment** slot and continues with the
/// materials in the window's own `CommandID` order. The slots are ordinary
/// inventory slots, the same numbering `character_data::InventoryItem::slot`
/// uses; the bag starts at 13, and the server refuses anything below that.
fn write_fuse_body(selector: u8, slots: &[u8]) -> Bytes {
    let mut out = Vec::with_capacity(slots.len() + 3);
    out.push(ALCHEMY_REQUEST_TAG);
    out.push(selector);
    out.push(slots.len() as u8);
    out.extend_from_slice(slots);
    Bytes::from(out)
}

/// Reads [`write_fuse_body`] back in the server's order: tag, selector, count,
/// slots. Reading the tag as a count is what makes a three-field body
/// unreadable — `02 03 00` then parses "successfully" as a two-slot fuse.
fn read_fuse_body(value: &Bytes, expected_selector: u8) -> Result<Vec<u8>, SerializationError> {
    let (&tag, rest) = value.split_first().ok_or_else(short_packet)?;
    if tag != ALCHEMY_REQUEST_TAG {
        return Err(short_packet());
    }
    let (&selector, rest) = rest.split_first().ok_or_else(short_packet)?;
    if selector != expected_selector {
        // other selectors are other workers with other answers; refusing beats
        // guessing a variant
        return Err(short_packet());
    }
    let (&count, rest) = rest.split_first().ok_or_else(short_packet)?;
    if count as usize != rest.len() {
        return Err(short_packet());
    }
    Ok(rest.to_vec())
}

/// Anything whose first byte is not the tag is a cancel to the server, and the
/// only cancel this client builds is the bare one-byte form.
fn read_cancel(value: &Bytes) -> Option<()> {
    (value.len() == 1 && value[0] == ALCHEMY_ACTION_CANCEL).then_some(())
}

fn short_packet() -> SerializationError {
    SerializationError::IoError(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "packet too short",
    ))
}

/// 0x7150 — client → server "fuse what is in the Equip Enhance page", or
/// "cancel the fusing".
///
/// Body `{u8 tag = 2, u8 op = 3, u8 count, count × u8 inventory slot}` for a
/// fuse, and a lone `{u8 1}` for a cancel. The cancel form is not a shorter
/// fuse: it is *anything whose first byte is not the tag*.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum AlchemyReinforceRequest {
    /// Fuse these inventory slots, equipment first.
    Fuse { slots: Vec<u8> },
    /// Abort a running fuse — the one-byte form.
    Cancel,
}

impl AlchemyReinforceRequest {
    /// A fuse of `slots`, or `None` below [`ALCHEMY_MIN_FUSE_SLOTS`].
    pub fn fuse(slots: Vec<u8>) -> Option<Self> {
        (slots.len() >= ALCHEMY_MIN_FUSE_SLOTS).then_some(Self::Fuse { slots })
    }
}

impl From<AlchemyReinforceRequest> for Bytes {
    fn from(value: AlchemyReinforceRequest) -> Self {
        match value {
            AlchemyReinforceRequest::Cancel => Bytes::from_static(&[ALCHEMY_ACTION_CANCEL]),
            AlchemyReinforceRequest::Fuse { slots } => {
                write_fuse_body(ALCHEMY_REINFORCE_OP_FUSE, &slots)
            }
        }
    }
}

impl TryFrom<Bytes> for AlchemyReinforceRequest {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        if read_cancel(&value).is_some() {
            return Ok(Self::Cancel);
        }
        Ok(Self::Fuse {
            slots: read_fuse_body(&value, ALCHEMY_REINFORCE_OP_FUSE)?,
        })
    }
}

/// 0x7151 — client → server "fuse the stone that is in the Att.Grant page", or
/// "cancel".
///
/// Body `{u8 tag = 2, u8 AlchemyType, u8 count, count × u8 inventory slot}` —
/// the same four fields `0x7150` has, with the stone kind in the selector's
/// place. The kind comes from the material's itemdata row, so a page holding a
/// magic stone and a page holding an attribute stone differ in exactly that one
/// byte.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum AlchemyStoneRequest {
    /// Fuse these inventory slots, equipment first.
    Fuse { stone_type: u8, slots: Vec<u8> },
    /// Abort a running fuse — the one-byte form.
    Cancel,
}

impl AlchemyStoneRequest {
    /// Same refusal as [`AlchemyReinforceRequest::fuse`], for the same reason.
    pub fn fuse(stone_type: u8, slots: Vec<u8>) -> Option<Self> {
        (slots.len() >= ALCHEMY_MIN_FUSE_SLOTS).then_some(Self::Fuse { stone_type, slots })
    }
}

impl From<AlchemyStoneRequest> for Bytes {
    fn from(value: AlchemyStoneRequest) -> Self {
        match value {
            AlchemyStoneRequest::Cancel => Bytes::from_static(&[ALCHEMY_ACTION_CANCEL]),
            AlchemyStoneRequest::Fuse { stone_type, slots } => write_fuse_body(stone_type, &slots),
        }
    }
}

impl TryFrom<Bytes> for AlchemyStoneRequest {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        if read_cancel(&value).is_some() {
            return Ok(Self::Cancel);
        }
        let (&tag, rest) = value.split_first().ok_or_else(short_packet)?;
        if tag != ALCHEMY_REQUEST_TAG {
            return Err(short_packet());
        }
        let &stone_type = rest.first().ok_or_else(short_packet)?;
        Ok(Self::Fuse {
            stone_type,
            slots: read_fuse_body(&value, stone_type)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    /// The stone fuse is the **five-byte** form: tag, the stone kind, the count,
    /// then the slots with the equipment leading. Four bytes without the tag
    /// would reach the server as a *cancel*, because the server takes anything
    /// whose first byte is not the tag as an abort.
    #[test]
    fn the_stone_fuse_leads_with_the_tag_then_the_kind() {
        let request = AlchemyStoneRequest::fuse(ALCHEMY_TYPE_MAGIC_STONE, vec![19, 15]).unwrap();
        let bytes: Bytes = request.clone().into();
        assert_eq!(bytes.as_ref(), &[0x02, 0x04, 0x02, 0x13, 0x0F]);
        assert_eq!(AlchemyStoneRequest::try_from(bytes).unwrap(), request);

        // the attribute stone differs in exactly one byte
        let attribute =
            AlchemyStoneRequest::fuse(ALCHEMY_TYPE_ATTRIBUTE_STONE, vec![19, 15]).unwrap();
        let bytes: Bytes = attribute.into();
        assert_eq!(bytes.as_ref(), &[0x02, 0x05, 0x02, 0x13, 0x0F]);
    }

    /// The elixir fuse has the same four fields, with the worker selector in the
    /// kind's place.
    #[test]
    fn the_elixir_fuse_has_the_same_shape_with_its_own_selector() {
        let request = AlchemyReinforceRequest::fuse(vec![19, 15, 16]).unwrap();
        let bytes: Bytes = request.clone().into();
        assert_eq!(bytes.as_ref(), &[0x02, 0x03, 0x03, 0x13, 0x0F, 0x10]);
        assert_eq!(AlchemyReinforceRequest::try_from(bytes).unwrap(), request);
    }

    /// Both bodies are byte-identical apart from the selector — the structural
    /// fact that lets one writer serve both opcodes.
    #[test]
    fn both_fuse_bodies_differ_only_in_the_selector() {
        let slots = vec![19, 15];
        let elixir: Bytes = AlchemyReinforceRequest::fuse(slots.clone()).unwrap().into();
        let stone: Bytes = AlchemyStoneRequest::fuse(ALCHEMY_TYPE_MAGIC_STONE, slots)
            .unwrap()
            .into();
        assert_eq!(elixir.len(), stone.len());
        assert_eq!(elixir[0], stone[0], "the same tag leads both");
        assert_ne!(elixir[1], stone[1], "only the selector differs");
        assert_eq!(elixir[2..], stone[2..], "count and slots are the same");
    }

    /// The cancel form is the bare one byte on both opcodes, and it is not a
    /// truncated fuse: a longer body that is not tagged is refused rather than
    /// read as an abort.
    #[test]
    fn the_cancel_form_is_the_bare_one_byte() {
        let bytes: Bytes = AlchemyReinforceRequest::Cancel.into();
        assert_eq!(bytes.as_ref(), &[0x01]);
        assert_eq!(
            AlchemyReinforceRequest::try_from(bytes).unwrap(),
            AlchemyReinforceRequest::Cancel
        );
        let bytes: Bytes = AlchemyStoneRequest::Cancel.into();
        assert_eq!(bytes.as_ref(), &[0x01]);
        assert_eq!(
            AlchemyStoneRequest::try_from(bytes).unwrap(),
            AlchemyStoneRequest::Cancel
        );
        // untagged and longer than the cancel: not ours
        assert!(AlchemyReinforceRequest::try_from(Bytes::from_static(&[0x01, 0x00])).is_err());
    }

    /// A one-slot fuse never becomes a packet: the page cannot express it, and
    /// on the stone opcode it would sit one byte away from a cancel.
    #[test]
    fn a_one_slot_fuse_is_refused_before_it_is_built() {
        assert!(AlchemyReinforceRequest::fuse(vec![19]).is_none());
        assert!(AlchemyReinforceRequest::fuse(Vec::new()).is_none());
        assert!(AlchemyStoneRequest::fuse(ALCHEMY_TYPE_MAGIC_STONE, vec![19]).is_none());
        assert!(AlchemyReinforceRequest::fuse(vec![19, 15]).is_some());
    }

    /// A count that does not match the slots that follow is refused, so a
    /// truncated frame cannot become a shorter fuse.
    #[test]
    fn a_count_that_disagrees_with_the_body_is_refused() {
        // says three slots, carries two
        assert!(AlchemyReinforceRequest::try_from(Bytes::from_static(&[
            0x02, 0x03, 0x03, 0x13, 0x0F
        ]))
        .is_err());
        // a foreign selector is another worker, not this request
        assert!(AlchemyReinforceRequest::try_from(Bytes::from_static(&[
            0x02, 0x06, 0x02, 0x13, 0x0F
        ]))
        .is_err());
    }

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
