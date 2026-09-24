//! Alchemy (elixir / stone / manufacture / dismantle / socket) wire family.
//!
//! Idea: of the twenty opcodes in this block, **six** (three request/ack pairs)
//! have a body that can be stated. One of them, dismantle, is published; the
//! other two pairs are read off the original's own builders and handlers,
//! which write and read one field per call, so an ordered call sequence *is* a
//! layout. The leading bytes of `0x7150` are confirmed against a real server.
//!
//! Modelled here: dismantle (`0x7157`/`0xB157`, the family's one *published*
//! layout) and the two fuse verbs of the classic alchemy box — reinforce
//! (`0x7150`/`0xB150`) and stone attach (`0x7151`/`0xB151`). Still unmodelled:
//! manufacture/disjoin `0x7155`, where two builders of the original disagree
//! by four bytes so a decoder must not assume a fixed size, and socket
//! `0x716A`. Their bodies stay unwired, with their reasons, in
//! `docs/net-alchemy.md`. Inventing a body is the one defect ADR-0009 exists
//! to avoid.
//!
//! Two facts shape every type below:
//! - **The wire carries ordinary inventory slot numbers.** The alchemy window
//!   numbers its slots from the first *bag* slot, so the original adds 13 in
//!   its request builders and subtracts it again in its handlers. 13 is
//!   `EQUIP_SLOT_COUNT`, and our model already holds inventory slots
//!   (`net::inventory::BAG_FIRST_SLOT` is that same 13), so nothing is biased
//!   here — biasing again would move every slot 13 further into the bag.
//! - The acks re-emit the **mutated item record**, and it is the ordinary
//!   `character_data::InventoryItem` blob. It is carried here as an unparsed
//!   tail and decoded through that one parser (see
//!   [`AlchemyReinforceResponse::item`]) — a second item parser is exactly what
//!   this module must not grow.
use bevy::prelude::Message;
use bytes::Bytes;

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

/// `AlchemyAction` values the acks carry in their second byte: the fuse was
/// cancelled, which the original answers with
/// `UIIT_MSG_ALCHEMY_CANCELED_COMPOUND` …
pub const ALCHEMY_ACTION_CANCEL: u8 = 1;
/// … or it ran, which is the only arm that reads the item-delivery block. Any
/// other value ends the packet in the original, so it does here too.
pub const ALCHEMY_ACTION_FUSE: u8 = 2;

/// `AlchemyType` for `0x7151`: a magic stone. Which of the two stones the
/// player is holding is a client-side classification of the item in the
/// slot.
pub const ALCHEMY_TYPE_MAGIC_STONE: u8 = 4;
/// `AlchemyType` for `0x7151`: an attribute stone.
pub const ALCHEMY_TYPE_ATTRIBUTE_STONE: u8 = 5;

/// First byte of every *request* body on `0x7150`: a server compares it
/// against the literal `2` and sends everything else — including the bare `1`
/// the original uses — down the cancel arm. It is a tag, not a count and not a
/// free action id; sharing the value `2` with [`ALCHEMY_ACTION_FUSE`] is a
/// coincidence of the two directions, so it is spelled out separately.
pub const ALCHEMY_REINFORCE_TAG: u8 = 2;

/// Second byte: the worker selector. A server computes `op - 3` and refuses
/// anything above `5` by rewinding its cursor and answering *nothing*, so the
/// valid range is `3..=8`.
///
/// `3` is the value used here, and it is not a pick out of that range: it is
/// the only `op` whose arm is known to read the body this request builds.
/// Driven against a real server, `op = 3` walks the count check (`02 03 00`
/// answers error `0x5413`), then the slot bounds check (`02 03 01 0c` answers
/// `0x5414`) and finally the slot-to-item lookup (`02 03 01 46` answers result
/// `3`) — i.e. exactly `{u8 count; count × u8 inventory slot}`, answered on
/// `0xB150`, which is the ack [`AlchemyReinforceResponse`] models. `op = 6` is
/// the backwards control: same body, no answer at all.
///
/// Not claimed: `op = 8` also answers `0xB150`, and which of `3`/`8` is Elixir
/// and which Advanced Elixir is unconfirmed. `op` 4/5 (`0xB151`) and 7
/// (`0x34A8`) are other workers and are not modelled here.
pub const ALCHEMY_REINFORCE_OP_FUSE: u8 = 3;

/// Smallest fuse the classic box can express: the equipment plus one material
/// (`ifalchemyreinforce.txt` — one equip slot, four stone slots).
///
/// This is a **send-side safety limit, not just a comment**. Two reasons, one
/// per opcode. On `0x7151` the fuse body leads with its own count, so a
/// one-slot fuse would go out as a byte-identical *cancel*, which the original
/// builds as a bare `1`. On `0x7150` the body leads with
/// [`ALCHEMY_REINFORCE_TAG`] instead, so that collision is gone — what is left
/// is the box itself: the Equip Enhance page needs the equipment plus at least
/// one material, and a server answers a `count == 0` body with error `0x5413`
/// anyway. Both constructors therefore refuse below this count instead of
/// relying on the caller.
pub const ALCHEMY_MIN_FUSE_SLOTS: usize = 2;

/// 0x7150 — client → server "fuse what is in the Equip Enhance page", or
/// "cancel the fusing".
///
/// Body, in the order a server reads it:
/// `{u8 tag = 2; u8 op; u8 count; count × u8 inventory slot}` for a fuse, and
/// a lone `{u8 1}` for a cancel — the cancel form is not a shorter fuse, it is
/// *anything whose first byte is not `2`*. See [`ALCHEMY_REINFORCE_TAG`] and
/// [`ALCHEMY_REINFORCE_OP_FUSE`] for the two leading bytes.
///
/// The slot bytes are ordinary inventory slots, the same numbering
/// `character_data::InventoryItem::slot` uses: a server rejects anything below
/// 13 outright with error `0x5414`, which is the equipment range, and the
/// `+13` the original applies before writing is its own window-index to
/// inventory-slot conversion, not a wire bias.
///
/// Note there is **no `AlchemyType` byte** here, unlike `0x7151`: `op` selects
/// one of five workers instead, and both `0xB150` arms (`op` 3 and 8) exist.
/// Which of the two is the advanced elixir is unconfirmed.
///
/// The original has a *second* builder for this opcode whose body is four
/// fields of a different shape, belonging to the four-tab window rather than
/// to the classic box. What settles the shape below is the server's reader: it
/// takes this body, whichever window built it.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum AlchemyReinforceRequest {
    /// Fuse these **inventory** slots (the wire numbering, bag from 13 up).
    Fuse { slots: Vec<u8> },
    /// Abort a running fuse — the one-byte form.
    Cancel,
}

impl AlchemyReinforceRequest {
    /// A fuse of `slots`, or `None` when there are fewer than
    /// [`ALCHEMY_MIN_FUSE_SLOTS`] — see that constant for why this is a hard
    /// refusal and not a debug assertion.
    pub fn fuse(slots: Vec<u8>) -> Option<Self> {
        (slots.len() >= ALCHEMY_MIN_FUSE_SLOTS).then_some(Self::Fuse { slots })
    }
}

impl From<AlchemyReinforceRequest> for Bytes {
    fn from(value: AlchemyReinforceRequest) -> Self {
        match value {
            AlchemyReinforceRequest::Cancel => Bytes::from_static(&[ALCHEMY_ACTION_CANCEL]),
            AlchemyReinforceRequest::Fuse { slots } => {
                let mut out = Vec::with_capacity(slots.len() + 3);
                out.push(ALCHEMY_REINFORCE_TAG);
                out.push(ALCHEMY_REINFORCE_OP_FUSE);
                out.push(slots.len() as u8);
                out.extend(slots.iter().copied());
                Bytes::from(out)
            }
        }
    }
}

impl TryFrom<Bytes> for AlchemyReinforceRequest {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        // Read in the server's order: tag, op, count, slots. Reading the tag
        // as a count is what made the earlier three-field body unreadable —
        // `02 03 00` then parses "successfully" as a two-slot fuse.
        let (&tag, rest) = value.split_first().ok_or_else(short_packet)?;
        if tag != ALCHEMY_REINFORCE_TAG {
            // everything that is not the tag is a cancel to the server
            // (`506e30`/`506e51`); we only ever build the original's bare `1`,
            // so anything longer is a truncated or foreign frame
            return if tag == ALCHEMY_ACTION_CANCEL && rest.is_empty() {
                Ok(Self::Cancel)
            } else {
                Err(short_packet())
            };
        }
        let (&op, rest) = rest.split_first().ok_or_else(short_packet)?;
        if op != ALCHEMY_REINFORCE_OP_FUSE {
            // ops 4/5/7/8 are other workers with other answers and are not
            // modelled by this enum; refusing beats guessing a variant
            return Err(short_packet());
        }
        let (&count, rest) = rest.split_first().ok_or_else(short_packet)?;
        if count as usize != rest.len() {
            return Err(short_packet());
        }
        Ok(Self::Fuse {
            slots: rest.to_vec(),
        })
    }
}

/// 0x7151 — client → server "attach this stone", or "cancel" with the same
/// one-byte body as `0x7150`.
///
/// Body: `{u8 AlchemyType; u8 n; n × u8 inventory slot}` — the type byte is
/// written first and is [`ALCHEMY_TYPE_MAGIC_STONE`] or
/// [`ALCHEMY_TYPE_ATTRIBUTE_STONE`]. The slots are the same inventory
/// numbering `0x7150` uses.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum AlchemyStoneRequest {
    Fuse { stone_type: u8, slots: Vec<u8> },
    Cancel,
}

impl AlchemyStoneRequest {
    /// Same refusal as [`AlchemyReinforceRequest::fuse`], for the same reason:
    /// `@008210f0` builds this opcode's cancel as a bare `1` too.
    pub fn fuse(stone_type: u8, slots: Vec<u8>) -> Option<Self> {
        (slots.len() >= ALCHEMY_MIN_FUSE_SLOTS).then_some(Self::Fuse { stone_type, slots })
    }
}

impl From<AlchemyStoneRequest> for Bytes {
    fn from(value: AlchemyStoneRequest) -> Self {
        match value {
            AlchemyStoneRequest::Cancel => Bytes::from_static(&[ALCHEMY_ACTION_CANCEL]),
            AlchemyStoneRequest::Fuse { stone_type, slots } => {
                let mut out = Vec::with_capacity(slots.len() + 2);
                out.push(stone_type);
                out.push(slots.len() as u8);
                out.extend(slots.iter().copied());
                Bytes::from(out)
            }
        }
    }
}

impl TryFrom<Bytes> for AlchemyStoneRequest {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        if value.len() == 1 && value[0] == ALCHEMY_ACTION_CANCEL {
            return Ok(Self::Cancel);
        }
        if value.len() < 2 || value[1] as usize != value.len() - 2 {
            return Err(short_packet());
        }
        Ok(Self::Fuse {
            stone_type: value[0],
            slots: value[2..].to_vec(),
        })
    }
}

fn short_packet() -> SerializationError {
    SerializationError::IoError(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "packet too short",
    ))
}

/// What the `0xB150`/`0xB151` ack actually happened to the item, as the
/// original's own handlers classify it *before* they look at the item record.
///
/// The original turns these into presenter flags: `A != 0` is success,
/// `A == 0 && C != 0` is a breakdown, and `A == 0 && C == 0` is a failure with
/// the mutated item following. Which of the three failure sub-messages shows
/// is a *diff* against the item that was in the slot, not a wire field — it is
/// recomputed from the delivered record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlchemyOutcome {
    /// `A != 0`: the record that follows *replaces* the item — success.
    Success,
    /// `A == 0`, `C != 0`: no item follows, the item is gone.
    Breakdown,
    /// `A == 0`, `C == 0`: the record that follows is the *same* item,
    /// mutated — the fuse failed.
    Failure,
}

/// 0xB150 — server → client ack for [`AlchemyReinforceRequest`].
///
/// ```text
/// u8 result (1 ok / 2 error)
///   result == 2 : u16 errorCode
///   result == 1 : u8 action (1 cancel / 2 fuse)
///     action == 2 : u8 A, u8 slot
///       A == 0 : u8 C ; C == 0 → <ITEM RECORD>
///       A != 0 :          <ITEM RECORD>
/// ```
/// The slot byte is the inventory slot, as everywhere else in this module; the
/// original converts it to the alchemy window's own index, which is not what
/// this type carries. The ack carries **no** level-delta and **no**
/// durability-delta; the client recomputes both by diffing.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct AlchemyReinforceResponse {
    pub result: u8,
    /// Only on `result == 2`. Unnamed on purpose: the `AlchemyErrorCode` table
    /// the published doc links is a dead page, so the number stays a number.
    /// One value is known — see [`ALCHEMY_ERROR_STONE_FAILED`].
    pub error_code: Option<u16>,
    /// Only on `result == 1`.
    pub action: Option<u8>,
    /// Only on a fuse: what happened, and which bag slot it happened to.
    pub outcome: Option<(AlchemyOutcome, u8)>,
    /// The unparsed item record, if one follows. Decode with [`Self::item`].
    pub item_tail: Bytes,
}

/// `0xB151` error code that means "the stone did not take": the original
/// special-cases exactly this value to the ordinary failure animation plus
/// `UIIT_MSG_REINFORCERR_FAIL`. It is the only member of the error table whose
/// meaning is known.
pub const ALCHEMY_ERROR_STONE_FAILED: u16 = 0x5423;

impl AlchemyReinforceResponse {
    pub fn is_success(&self) -> bool {
        self.result != ALCHEMY_RESULT_ERROR
    }

    /// The mutated item, parsed by the one item parser this workspace has
    /// (`InventoryItem::read_with`, the CHARACTER_DATA record). The record's
    /// own leading byte is the inventory slot the ack already named, so it is
    /// prepended the way `InventoryOperationResult::pickup_item` does it.
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
    if action != ALCHEMY_ACTION_FUSE {
        // cancel, and every unknown action, end the packet in the original
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
/// [`AlchemyReinforceResponse`] minus the breakdown flag: a stone attach
/// always delivers a record, and the three outcomes (append / change /
/// assimilation) are **not** on the wire at all — they are read out of the
/// delivered magic-option list.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct AlchemyStoneResponse(pub AlchemyReinforceResponse);

impl TryFrom<Bytes> for AlchemyStoneResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        read_fuse_ack(value, false).map(AlchemyStoneResponse)
    }
}

impl From<AlchemyStoneResponse> for Bytes {
    fn from(_: AlchemyStoneResponse) -> Self {
        // inbound-only: the client never builds an ack. Kept because the
        // packet table's `into_serialize` arm is generated for every entry.
        Bytes::new()
    }
}

impl From<AlchemyReinforceResponse> for Bytes {
    fn from(_: AlchemyReinforceResponse) -> Self {
        Bytes::new()
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

    /// The fuse body is `{u8 2, u8 op, u8 count, count × inventory slot}` —
    /// the two leading bytes the server tests before it ever looks at the
    /// count — and the slots go out unchanged, because the HUD already holds
    /// inventory slots.
    #[test]
    fn reinforce_request_leads_with_the_tag_and_the_op() {
        // 13 and 20 are inventory slots: the first bag slot and one further in
        let fuse = AlchemyReinforceRequest::fuse(vec![13, 20]).unwrap();
        let bytes: Bytes = fuse.clone().into();
        assert_eq!(bytes.as_ref(), &[0x02, 0x03, 0x02, 0x0D, 0x14]);
        assert_eq!(
            AlchemyReinforceRequest::try_from(bytes.clone()).unwrap(),
            fuse
        );

        // the body must not bias the slots a second time: `0x0D + 13` would be
        // slot 26, an item the player never selected
        assert!(!bytes.contains(&(13 + 13)));

        // the three-field body this used to send had `0x0D` in the op byte,
        // outside the server's `op ∈ 3..=8` window — dropped without an answer
        assert!(
            AlchemyReinforceRequest::try_from(Bytes::from_static(&[0x02, 0x0D, 0x14])).is_err()
        );
    }

    /// Positive control for the test above: the cancel form is a *different*
    /// arm of the same opcode (`506e30` fails, `jne 506e51`), so it must come
    /// through untouched — one byte, still `1`, still parsed back. On its own
    /// so that breaking the fuse body leaves it green.
    #[test]
    fn the_cancel_form_is_still_the_bare_one_byte() {
        let bytes: Bytes = AlchemyReinforceRequest::Cancel.into();
        assert_eq!(bytes.as_ref(), &[0x01]);
        assert_ne!(bytes.first(), Some(&ALCHEMY_REINFORCE_TAG));
        assert_eq!(
            AlchemyReinforceRequest::try_from(bytes).unwrap(),
            AlchemyReinforceRequest::Cancel
        );
    }

    /// The frames a server accepts, read back through this reader: `op = 3`
    /// is a fuse, `op = 6` is a worker this module does not model, and the
    /// slot byte is the inventory slot that went on the wire (`0x46`), not
    /// `0x46 - 13`.
    #[test]
    fn the_request_frames_read_back_as_a_server_reads_them() {
        assert_eq!(
            AlchemyReinforceRequest::try_from(Bytes::from_static(&[0x02, 0x03, 0x00])).unwrap(),
            AlchemyReinforceRequest::Fuse { slots: Vec::new() }
        );
        assert_eq!(
            AlchemyReinforceRequest::try_from(Bytes::from_static(&[0x02, 0x03, 0x01, 0x46]))
                .unwrap(),
            AlchemyReinforceRequest::Fuse { slots: vec![0x46] }
        );
        assert!(
            AlchemyReinforceRequest::try_from(Bytes::from_static(&[0x02, 0x06, 0x01, 0x46]))
                .is_err()
        );
    }

    /// The safety limit, not a comment: the box needs the equipment plus one
    /// material, and on `0x7151` a one-slot fuse would serialise to
    /// `[type, 0x01, slot]`, whose count byte is the cancel discriminator — so
    /// both constructors refuse to build one at all.
    #[test]
    fn a_one_slot_fuse_is_refused_because_it_would_read_as_a_cancel() {
        assert!(AlchemyReinforceRequest::fuse(vec![7]).is_none());
        assert!(AlchemyReinforceRequest::fuse(Vec::new()).is_none());
        assert!(AlchemyStoneRequest::fuse(ALCHEMY_TYPE_ATTRIBUTE_STONE, vec![7]).is_none());
        // two slots is the smallest thing the box can express, and it is fine
        assert!(AlchemyReinforceRequest::fuse(vec![7, 8]).is_some());
    }

    /// `0x7151` leads with the `AlchemyType` byte the reinforce request does
    /// not have (`@008215f0` maps its argument 2→5, 1→4 and writes it first).
    #[test]
    fn stone_request_leads_with_the_alchemy_type() {
        let fuse = AlchemyStoneRequest::fuse(ALCHEMY_TYPE_MAGIC_STONE, vec![13, 20]).unwrap();
        let bytes: Bytes = fuse.clone().into();
        assert_eq!(bytes.as_ref(), &[0x04, 0x02, 0x0D, 0x14]);
        assert_eq!(AlchemyStoneRequest::try_from(bytes).unwrap(), fuse);
    }

    /// The three outcomes the handler distinguishes before it reads the item:
    /// `A != 0` success, `A == 0 && C != 0` breakdown, `A == 0 && C == 0`
    /// failure-with-item (`@008729e0:60-105`).
    #[test]
    fn the_reinforce_ack_classifies_by_a_and_c_not_by_the_item() {
        let success =
            AlchemyReinforceResponse::try_from(Bytes::from_static(&[0x01, 0x02, 0x01, 0x14, 0xAA]))
                .unwrap();
        assert_eq!(success.outcome, Some((AlchemyOutcome::Success, 20)));
        assert_eq!(success.item_tail.as_ref(), &[0xAA]);

        let broken =
            AlchemyReinforceResponse::try_from(Bytes::from_static(&[0x01, 0x02, 0x00, 0x14, 0x01]))
                .unwrap();
        assert_eq!(broken.outcome, Some((AlchemyOutcome::Breakdown, 20)));
        assert!(broken.item_tail.is_empty(), "no item follows a breakdown");

        let failed = AlchemyReinforceResponse::try_from(Bytes::from_static(&[
            0x01, 0x02, 0x00, 0x14, 0x00, 0xAA, 0xBB,
        ]))
        .unwrap();
        assert_eq!(failed.outcome, Some((AlchemyOutcome::Failure, 20)));
        assert_eq!(failed.item_tail.as_ref(), &[0xAA, 0xBB]);
    }

    /// A cancel ack ends after the action byte, and an error ack carries the
    /// unnamed `u16` instead of an outcome.
    #[test]
    fn the_reinforce_ack_stops_where_the_original_stops() {
        let cancelled =
            AlchemyReinforceResponse::try_from(Bytes::from_static(&[0x01, 0x01])).unwrap();
        assert_eq!(cancelled.action, Some(ALCHEMY_ACTION_CANCEL));
        assert_eq!(cancelled.outcome, None);

        let refused =
            AlchemyReinforceResponse::try_from(Bytes::from_static(&[0x02, 0x23, 0x54])).unwrap();
        assert!(!refused.is_success());
        assert_eq!(refused.error_code, Some(ALCHEMY_ERROR_STONE_FAILED));
        assert_eq!(refused.outcome, None);
    }

    /// `0xB151` has no breakdown flag: on `A == 0` the record follows
    /// immediately, so the byte that would be `C` is already item data.
    #[test]
    fn the_stone_ack_has_no_breakdown_flag() {
        let failed = AlchemyStoneResponse::try_from(Bytes::from_static(&[
            0x01, 0x02, 0x00, 0x14, 0x01, 0xAA,
        ]))
        .unwrap()
        .0;
        assert_eq!(failed.outcome, Some((AlchemyOutcome::Failure, 20)));
        assert_eq!(failed.item_tail.as_ref(), &[0x01, 0xAA]);
    }
}
