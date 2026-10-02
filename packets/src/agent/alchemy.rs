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

use crate::agent::character_data::{InventoryItem, ItemClassResolver};

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

/// What the ack says happened to the item, off its two discriminator bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlchemyOutcome {
    /// `A != 0`: the record that follows *replaces* the item — success.
    Success,
    /// `A == 0`, `C != 0`: no item follows, the item is gone.
    Breakdown,
    /// `A == 0`, `C == 0`: the record that follows is the *same* item, mutated —
    /// the fuse failed.
    Failure,
}

/// `AlchemyAction` in the ack's second byte: the fuse was cancelled, or it ran.
/// Any other value ends the packet.
pub const ALCHEMY_ACTION_CANCEL_ACK: u8 = 1;
/// … or it ran, and only then does an item-delivery block follow.
pub const ALCHEMY_ACTION_FUSE_ACK: u8 = 2;

/// 0xB150 — server → client ack for [`AlchemyReinforceRequest`].
///
/// ```text
/// u8 result (1 ok / 2 error)
///   result == 2 : u16 errorCode
///   result == 1 : u8 action (1 cancel / 2 fuse)
///     action == 2 : u8 A, u8 slot
///       A == 0 : u8 C ; C == 0 -> <ITEM RECORD>
///       A != 0 :          <ITEM RECORD>
/// ```
/// **Only the refusal arm has ever been seen on the wire**; the rest of this
/// layout is read off the client that answers it, so the success and breakdown
/// arms are unconfirmed. The ack carries **no** level delta and **no**
/// durability delta — a caller recomputes both by diffing against the item that
/// was in the slot.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct AlchemyReinforceResponse {
    pub result: u8,
    /// Only on `result == 2`. Unnamed on purpose: the error-code table is not in
    /// our data, so the number stays a number. One value is known — see
    /// [`ALCHEMY_ERROR_STONE_FAILED`].
    pub error_code: Option<u16>,
    /// Only on `result == 1`.
    pub action: Option<u8>,
    /// Only on a fuse: what happened, and which bag slot it happened to.
    pub outcome: Option<(AlchemyOutcome, u8)>,
    /// The unparsed item record, if one follows. Decode with [`Self::item`].
    pub item_tail: Bytes,
}

/// The one error code with a known meaning: "the stone did not take", which the
/// original answers with the ordinary failure line rather than a code.
pub const ALCHEMY_ERROR_STONE_FAILED: u16 = 0x5423;

impl AlchemyReinforceResponse {
    pub fn is_success(&self) -> bool {
        self.result != ALCHEMY_RESULT_ERROR
    }

    /// The mutated item, parsed by the one item parser this workspace has. The
    /// record's own leading byte is the inventory slot the ack already named, so
    /// it is prepended the way the pickup path does it.
    pub fn item(&self, resolver: &impl ItemClassResolver) -> Option<InventoryItem> {
        let (_, slot) = self.outcome?;
        if self.item_tail.is_empty() {
            return None;
        }
        let mut buf = Vec::with_capacity(self.item_tail.len() + 1);
        buf.push(slot);
        buf.extend_from_slice(&self.item_tail);
        InventoryItem::read_with(&mut std::io::Cursor::new(buf.as_slice()), resolver).ok()
    }
}

/// Shared body reader for the two fuse acks. `has_breakdown_flag` is the one
/// difference between them: `0xB150` reads a "no item follows" byte on the
/// `A == 0` branch, `0xB151` does not — it always has a record, so a stone
/// attach cannot destroy the item.
fn read_fuse_ack(
    value: Bytes,
    has_breakdown_flag: bool,
) -> Result<AlchemyReinforceResponse, SerializationError> {
    let mut out = AlchemyReinforceResponse {
        result: *value.first().ok_or_else(short_packet)?,
        error_code: None,
        action: None,
        outcome: None,
        item_tail: Bytes::new(),
    };
    if out.result == ALCHEMY_RESULT_ERROR {
        if value.len() < 3 {
            return Err(short_packet());
        }
        out.error_code = Some(u16::from_le_bytes([value[1], value[2]]));
        return Ok(out);
    }
    let Some(&action) = value.get(1) else {
        return Ok(out);
    };
    out.action = Some(action);
    if action != ALCHEMY_ACTION_FUSE_ACK {
        // cancel, and every unknown action, end the packet
        return Ok(out);
    }
    if value.len() < 4 {
        return Err(short_packet());
    }
    let delivery = value[2];
    let slot = value[3];
    let mut rest = value.slice(4..);
    let outcome = if delivery != 0 {
        AlchemyOutcome::Success
    } else if has_breakdown_flag {
        let (&flag, _) = rest.split_first().ok_or_else(short_packet)?;
        rest = rest.slice(1..);
        if flag != 0 {
            AlchemyOutcome::Breakdown
        } else {
            AlchemyOutcome::Failure
        }
    } else {
        AlchemyOutcome::Failure
    };
    out.outcome = Some((outcome, slot));
    out.item_tail = if outcome == AlchemyOutcome::Breakdown {
        Bytes::new()
    } else {
        rest
    };
    Ok(out)
}

impl TryFrom<Bytes> for AlchemyReinforceResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        read_fuse_ack(value, true)
    }
}

/// 0xB151 — server → client ack for [`AlchemyStoneRequest`]. Same body as
/// [`AlchemyReinforceResponse`] minus the breakdown flag.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct AlchemyStoneResponse(pub AlchemyReinforceResponse);

impl TryFrom<Bytes> for AlchemyStoneResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        read_fuse_ack(value, false).map(Self)
    }
}

/// Server → client only. The macro wants both directions for every registered
/// opcode; building this ack is not a thing a client does.
impl From<AlchemyStoneResponse> for Bytes {
    fn from(_: AlchemyStoneResponse) -> Self {
        unreachable!("0xB151 is server -> client only")
    }
}

/// Server → client only, same reason.
impl From<AlchemyReinforceResponse> for Bytes {
    fn from(_: AlchemyReinforceResponse) -> Self {
        unreachable!("0xB150 is server -> client only")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    /// The ack classifies by its two discriminators, not by the item that
    /// follows. Both bodies are built here from the layout, because only the
    /// refusal arm has ever been seen on the wire.
    #[test]
    fn the_reinforce_ack_classifies_by_a_and_c_not_by_the_item() {
        // A != 0 -> success, and a record follows
        let ok = AlchemyReinforceResponse::try_from(Bytes::from_static(&[
            0x01, 0x02, 0x01, 0x0D, 0xAA, 0xBB,
        ]))
        .unwrap();
        assert_eq!(ok.outcome, Some((AlchemyOutcome::Success, 0x0D)));
        assert_eq!(ok.item_tail.as_ref(), &[0xAA, 0xBB]);

        // A == 0, C != 0 -> breakdown, and NO record follows
        let gone =
            AlchemyReinforceResponse::try_from(Bytes::from_static(&[0x01, 0x02, 0x00, 0x0D, 0x01]))
                .unwrap();
        assert_eq!(gone.outcome, Some((AlchemyOutcome::Breakdown, 0x0D)));
        assert!(gone.item_tail.is_empty());

        // A == 0, C == 0 -> failure, the same item comes back mutated
        let failed = AlchemyReinforceResponse::try_from(Bytes::from_static(&[
            0x01, 0x02, 0x00, 0x0D, 0x00, 0xAA,
        ]))
        .unwrap();
        assert_eq!(failed.outcome, Some((AlchemyOutcome::Failure, 0x0D)));
        assert_eq!(failed.item_tail.as_ref(), &[0xAA]);
    }

    /// The refusal arm is the only one with wire evidence: `result == 2` then a
    /// little-endian code, and nothing after it.
    #[test]
    fn the_refusal_arm_carries_a_code_and_stops() {
        let refused =
            AlchemyReinforceResponse::try_from(Bytes::from_static(&[0x02, 0x13, 0x54])).unwrap();
        assert!(!refused.is_success());
        assert_eq!(refused.error_code, Some(0x5413));
        assert_eq!(refused.action, None);
        assert_eq!(refused.outcome, None);
        // the one code with a known meaning
        let stone =
            AlchemyReinforceResponse::try_from(Bytes::from_static(&[0x02, 0x23, 0x54])).unwrap();
        assert_eq!(stone.error_code, Some(ALCHEMY_ERROR_STONE_FAILED));
        // a refusal without its code is not a refusal we can read
        assert!(AlchemyReinforceResponse::try_from(Bytes::from_static(&[0x02, 0x13])).is_err());
    }

    /// A cancel ack ends after the action byte, and carries no outcome at all.
    #[test]
    fn the_cancel_ack_stops_after_its_action_byte() {
        let cancelled =
            AlchemyReinforceResponse::try_from(Bytes::from_static(&[0x01, 0x01])).unwrap();
        assert!(cancelled.is_success());
        assert_eq!(cancelled.action, Some(ALCHEMY_ACTION_CANCEL_ACK));
        assert_eq!(cancelled.outcome, None);
        assert!(cancelled.item_tail.is_empty());
        // an unknown action is read as "ends here" rather than guessed at
        let unknown =
            AlchemyReinforceResponse::try_from(Bytes::from_static(&[0x01, 0x09])).unwrap();
        assert_eq!(unknown.outcome, None);
    }

    /// The stone ack has no breakdown flag: on `A == 0` the record starts
    /// immediately, so the same bytes mean different things on the two opcodes.
    #[test]
    fn the_stone_ack_has_no_breakdown_flag() {
        let bytes = &[0x01, 0x02, 0x00, 0x0D, 0x01, 0xAA];
        let stone = AlchemyStoneResponse::try_from(Bytes::from_static(bytes)).unwrap();
        assert_eq!(stone.0.outcome, Some((AlchemyOutcome::Failure, 0x0D)));
        assert_eq!(
            stone.0.item_tail.as_ref(),
            &[0x01, 0xAA],
            "no flag is eaten"
        );

        let reinforce = AlchemyReinforceResponse::try_from(Bytes::from_static(bytes)).unwrap();
        assert_eq!(reinforce.outcome, Some((AlchemyOutcome::Breakdown, 0x0D)));
        assert_ne!(stone.0.outcome, reinforce.outcome);
    }

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
