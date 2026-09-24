//! Quest wire structures: the **mark** pair (0x30D6 / 0x30D7), the
//! **progress packet** (0x30D5) and the **active-quest record** that both
//! CHARACTER_DATA (0x3013) and 0x30D5 carry.
//!
//! Idea, and the reason this module is three structs rather than seventeen:
//! the quest family is the one family whose body layouts are published
//! nowhere. The public sources index eleven quest opcodes and leave every body
//! page empty, so only the frames a real server sends and the original's own
//! reading of them can settle a layout. That covers exactly the two mark
//! opcodes, the progress packet and the active-quest record at the bottom of
//! this file, which rides inside CHARACTER_DATA rather than in an opcode of
//! its own. The rest of the family stays unwired, with its reasons, in
//! `docs/net-quest.md`.
//!
//! The pairing of the mark opcodes is not assumed: every `0x30D7` body is
//! byte-for-byte the leading `u32` of an *earlier* `0x30D6`. That is what
//! identifies the leading field as a **mark handle** and `0x30D7` as "drop
//! that mark".
//!
//! ## 0x30D5, the progress packet
//!
//! No frame of this opcode has been seen, so what settles it is the client's
//! own handler, which is a straight-line read sequence. The layout is
//! therefore certain and only its *semantics* are open:
//!
//! * it reads `u8 kind` then `u32 quest_id`, and then switches on the kind.
//! * The three switch arms are named by the client's own debug strings: `1` is
//!   an add request, `2` a modify request, and `3` **and** `4` share the delete
//!   arm, i.e. remove and abandon have the same client-side effect. Any other
//!   value reads no further bytes at all.
//! * `kind` 1 and 2 go on to call the **active-quest record parser** — the same
//!   one the CHARACTER_DATA quest section uses. So the progress packet carries
//!   a *whole record*, not a delta: `0x30D5` is `{u8 kind, u32 quest_id}` plus,
//!   for `kind` 1 and 2, the record body of [`ActiveQuest`] with its `id`
//!   already consumed by the prefix.
//!
//! That same parser settles two readings of `quest_type` that used to be
//! guesses: the `0x04` bit really does gate a `u32`, and the `0x08` bit gates
//! the `state` byte rather than being a constant with no job. `0x10` gates the
//! objective list (a count, then per entry `u8 enabled`, `u16 + ASCII` key,
//! `u8 count`, `count × u32`) and `0x40` the trailing NPC id list — exactly as
//! real records read.

use std::io::Read;

use bevy::prelude::Message;
use byteorder::ReadBytesExt;

use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

/// `QuestMark` — the icon the minimap/world draws for a mark. The published
/// enum lists four values; a server was seen to use 1, 3 and 4.
pub const QUEST_MARK_NEW: u8 = 1;
pub const QUEST_MARK_OPEN: u8 = 2;
pub const QUEST_MARK_COMPLETE: u8 = 3;
pub const QUEST_MARK_HARD: u8 = 4;

/// 0x30D6 — add a quest mark.
///
/// What each field is, and how sure:
///
/// * `mark_id` is the handle `0x30D7` later removes. It climbs with a fixed
///   stride, i.e. it is a server-side pool index and not a quest id: the same
///   quest's mark gets a new value each time it is re-added.
/// * `unknown0` is a **flag byte** and gates the last `u32`: the original reads
///   `u32, u8 flags, u8 mark, u16 region, 12 bytes` and then
///   `if ((flags & 2) != 0) read(4)`. In practice the bit is always set and the
///   24-byte body below fits — but a mark **without** it would be 20 bytes and
///   this struct would mis-read it. Reshaping the struct needs a frame of that
///   shape first.
/// * `mark` is one of the `QUEST_MARK_*` values.
/// * `region` is an ordinary region id for where the character stands.
/// * `unknown1`..`unknown4` are four `u32`s, the second of which is always `0`.
///   They look like a position triple plus one value, but "looks like" is not a
///   layout: naming them would be exactly the unsourced field ADR-0009
///   forbids.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct QuestMarkAdd {
    pub mark_id: u32,
    pub unknown0: u8,
    /// See the `QUEST_MARK_*` constants.
    pub mark: u8,
    pub region: u16,
    pub unknown1: u32,
    pub unknown2: u32,
    pub unknown3: u32,
    pub unknown4: u32,
}

/// 0x30D7 — remove the quest mark with this handle. The whole body is those
/// 4 bytes, and every one seen is a handle a previous `0x30D6` introduced.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct QuestMarkRemove {
    pub mark_id: u32,
}

// --- the active-quest record ------------------------------------------------
//
// Idea: the record below is the *only* self-contained quest structure a server
// sends today. It arrives inside CHARACTER_DATA (0x3013) after the
// completed-quest ids. Of the two published transcriptions only one fits real
// bodies at all, and this module departs from it twice on purpose:
//
// * `quest_type` is modelled as a **bit field**, not as the sentinel set
//   `{8, 28, 88}`. Equality tests happen to behave on the values seen so far
//   but cannot survive a fifth one.
// * the objective `name` is a **string-table key** (`SN_CON_<codename>[_NN]`),
//   not display text, so it is named `name_key` at the struct level. The text
//   lives in the client's own `textquest_speech&name.txt`.
//
// Note for whoever builds the journal next: a live server sends quest ids that
// have **no row** in the client's `questdata.txt`, so a journal must key
// **wire-first** — render from `name_key` and only *decorate* with
// `questdata`/`questcontentsdata` when the id happens to resolve. A
// table-first journal shows blanks for real quests. This module deliberately
// loads no quest table at all; that is the next stage.
//
// Everything here is little-endian, like the rest of the wire.

/// `quest_type` bits. The original's record parser tests exactly these four
/// bits and nothing else, each gate on its own — including the one no frame
/// could show, because `0x08` is set in every value seen. A fifth bit outside
/// this set would still falsify the reading as a whole;
/// [`ActiveQuest::unknown_type_bits`] surfaces it instead of the parser
/// rejecting the record.
pub const QUEST_TYPE_TIME_LIMITED: u8 = 0x04;
/// The `state` byte follows when this bit is set. It is set in every record
/// seen, which is why it once looked like a constant with no job.
pub const QUEST_TYPE_HAS_STATE: u8 = 0x08;
/// An objective list follows the `state` byte.
pub const QUEST_TYPE_HAS_OBJECTIVES: u8 = 0x10;
/// A count-prefixed list of NPC ref ids closes the record.
pub const QUEST_TYPE_HAS_NPC_LIST: u8 = 0x40;

/// `0x30D5` `kind` values — the switch arms of the original's handler, named
/// by its own debug strings. `REMOVE` and `ABANDON` share the delete arm.
pub const QUEST_UPDATE_ADD: u8 = 1;
pub const QUEST_UPDATE_UPDATE: u8 = 2;
pub const QUEST_UPDATE_REMOVE: u8 = 3;
pub const QUEST_UPDATE_ABANDON: u8 = 4;

/// One objective line of an active quest.
///
/// `name_key` is a **string-table key**, never prose: the wire carries keys
/// such as `SN_CON_QNO_CH_GENARAL_1_01`, and `questcontentsdata.txt` lists
/// exactly that key for the quest. The displayable sentence
/// (`"Hunt 15 TIger (%d)"`) is the client's, and its single `%d` is where
/// `tasks` goes — which is why the wire needs no text at all.
///
/// What `tasks` counts is not confirmed: it is `[0]` in nearly every record
/// and `[1]` in one whose `enabled` is 0. Progress counters is the reading
/// that fits.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, Default, PartialEq)]
pub struct QuestObjective {
    /// 1-based within the quest in every record seen.
    pub id: u8,
    /// Raw wire byte; 1 for every objective seen but one. Like the
    /// mastery/skill lists' `enabled`, what 0 means is not confirmed.
    pub enabled: u8,
    /// `SN_CON_<quest codename>[_NN]` — a key into
    /// `textquest_speech&name.txt`, not text.
    pub name_key: String,
    pub tasks: Vec<u32>,
}

/// An active quest as CHARACTER_DATA carries it; the layout fits every real
/// body seen.
///
/// Read/written by hand rather than by the derive because the trailing two
/// lists are gated by bits of `quest_type`, and the derive's `when` conditional
/// covers primitives and strings only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActiveQuest {
    pub id: u32,
    /// Always `16` so far, so what it counts is unknown.
    pub achievements: u8,
    /// Always `0` so far; the name is the published one.
    pub autoshare: u8,
    /// Raw wire byte, kept whole on purpose: the readable bits are the
    /// `QUEST_TYPE_*` constants, and anything else stays visible through
    /// [`Self::unknown_type_bits`] instead of being dropped.
    pub quest_type: u8,
    /// Present iff `QUEST_TYPE_TIME_LIMITED` is set. No frame has carried one,
    /// so its unit is unknown.
    pub remaining_time: Option<u32>,
    /// `1`, `7` or `8` so far; what each value means is unknown.
    pub state: u8,
    /// Empty when `QUEST_TYPE_HAS_OBJECTIVES` is clear.
    pub objectives: Vec<QuestObjective>,
    /// Quest-giver/target NPC ref ids; empty when `QUEST_TYPE_HAS_NPC_LIST` is
    /// clear.
    pub npcs: Vec<u32>,
}

/// A `u8` count and its rows must agree, so a list longer than 255 rows is cut
/// to what the count byte can name instead of being written past its own
/// length (`as u8` alone would truncate the count and keep the rows).
fn clamped_count(len: usize) -> usize {
    len.min(u8::MAX as usize)
}

impl ActiveQuest {
    /// See `QUEST_TYPE_TIME_LIMITED`.
    pub fn is_time_limited(&self) -> bool {
        self.quest_type & QUEST_TYPE_TIME_LIMITED != 0
    }

    /// See `QUEST_TYPE_HAS_STATE`: without this bit the client reads no
    /// `state` byte, so [`Self::state`] is `0` because nothing was on the wire.
    pub fn has_state(&self) -> bool {
        self.quest_type & QUEST_TYPE_HAS_STATE != 0
    }

    /// See `QUEST_TYPE_HAS_OBJECTIVES`.
    pub fn has_objective_list(&self) -> bool {
        self.quest_type & QUEST_TYPE_HAS_OBJECTIVES != 0
    }

    /// See `QUEST_TYPE_HAS_NPC_LIST`.
    pub fn has_npc_list(&self) -> bool {
        self.quest_type & QUEST_TYPE_HAS_NPC_LIST != 0
    }

    /// The bits this reading does not explain — `0` for every value seen so
    /// far. Non-zero here means the bit-field reading of `quest_type` needs
    /// revisiting (and the record still parsed, which is the point).
    pub fn unknown_type_bits(&self) -> u8 {
        self.quest_type
            & !(QUEST_TYPE_TIME_LIMITED
                | QUEST_TYPE_HAS_STATE
                | QUEST_TYPE_HAS_OBJECTIVES
                | QUEST_TYPE_HAS_NPC_LIST)
    }

    /// Everything after the `id`, in the order the original reads it. Split
    /// out because `0x30D5` sends the id in its own prefix and then this exact
    /// body — one function serves both call sites in the original, so it
    /// serves both here.
    pub fn read_body_from<T: Read + ReadBytesExt>(
        id: u32,
        reader: &mut T,
    ) -> Result<Self, SerializationError> {
        let achievements = u8::read_from(reader)?;
        let autoshare = u8::read_from(reader)?;
        let quest_type = u8::read_from(reader)?;
        let remaining_time = if quest_type & QUEST_TYPE_TIME_LIMITED != 0 {
            Some(u32::read_from(reader)?)
        } else {
            None
        };
        // the state byte is gated, not unconditional
        let state = if quest_type & QUEST_TYPE_HAS_STATE != 0 {
            u8::read_from(reader)?
        } else {
            0
        };
        let mut objectives = Vec::new();
        if quest_type & QUEST_TYPE_HAS_OBJECTIVES != 0 {
            let count = u8::read_from(reader)?;
            objectives.reserve(count as usize);
            for _ in 0..count {
                objectives.push(QuestObjective::read_from(reader)?);
            }
        }
        let mut npcs = Vec::new();
        if quest_type & QUEST_TYPE_HAS_NPC_LIST != 0 {
            let count = u8::read_from(reader)?;
            npcs.reserve(count as usize);
            for _ in 0..count {
                npcs.push(u32::read_from(reader)?);
            }
        }
        Ok(ActiveQuest {
            id,
            achievements,
            autoshare,
            quest_type,
            remaining_time,
            state,
            objectives,
            npcs,
        })
    }

    /// The mirror of [`Self::read_body_from`]: every field the type bits say is
    /// on the wire, and none that they do not, so a decoded record re-encodes
    /// byte-identically.
    pub fn serialize_body_to(&self, buf: &mut bytes::BytesMut) {
        self.achievements.serialize_to(buf);
        self.autoshare.serialize_to(buf);
        self.quest_type.serialize_to(buf);
        if self.is_time_limited() {
            // the bit is what the reader gates on, so the field is written
            // whenever the bit is set — a `None` here would otherwise drop
            // four bytes the reader goes on to demand
            self.remaining_time.unwrap_or_default().serialize_to(buf);
        }
        if self.has_state() {
            self.state.serialize_to(buf);
        }
        if self.has_objective_list() {
            // the count is one byte on the wire, so more than 255 rows cannot
            // be expressed; writing the truncated count *and* every row would
            // desync the reader, so the tail is cut with the count
            let count = clamped_count(self.objectives.len());
            (count as u8).serialize_to(buf);
            for objective in self.objectives.iter().take(count) {
                objective.serialize_to(buf);
            }
        }
        if self.has_npc_list() {
            let count = clamped_count(self.npcs.len());
            (count as u8).serialize_to(buf);
            for npc in self.npcs.iter().take(count) {
                npc.serialize_to(buf);
            }
        }
    }

    /// Size of [`Self::serialize_body_to`]'s output, i.e. [`ByteSize`] without
    /// the four id bytes.
    pub fn body_byte_size(&self) -> usize {
        let mut size = 3; // achievements, autoshare, type
        if self.is_time_limited() {
            size += 4;
        }
        if self.has_state() {
            size += 1;
        }
        if self.has_objective_list() {
            size += 1 + self
                .objectives
                .iter()
                .take(clamped_count(self.objectives.len()))
                .map(|objective| objective.byte_size())
                .sum::<usize>();
        }
        if self.has_npc_list() {
            size += 1 + 4 * clamped_count(self.npcs.len());
        }
        size
    }
}

impl Deserialize for ActiveQuest {
    fn read_from<T: Read + ReadBytesExt>(reader: &mut T) -> Result<Self, SerializationError> {
        let id = u32::read_from(reader)?;
        ActiveQuest::read_body_from(id, reader)
    }
}

impl Serialize for ActiveQuest {
    fn serialize_to(&self, buf: &mut bytes::BytesMut) {
        // The presence gates are the type bits, exactly as on read: a value
        // whose bit is clear is not written, so a decoded record re-encodes
        // byte-identically (the round-trip test in this module).
        self.id.serialize_to(buf);
        self.serialize_body_to(buf);
    }
}

/// Mirrors what the `Deserialize`/`Serialize` derives generate for every other
/// struct in this crate, so a hand-written record is used the same way.
impl TryFrom<bytes::Bytes> for ActiveQuest {
    type Error = SerializationError;

    fn try_from(data: bytes::Bytes) -> Result<Self, Self::Error> {
        use bytes::Buf;
        let mut reader = data.reader();
        ActiveQuest::read_from(&mut reader)
    }
}

impl From<ActiveQuest> for bytes::Bytes {
    fn from(quest: ActiveQuest) -> bytes::Bytes {
        let mut buffer = bytes::BytesMut::with_capacity(quest.byte_size());
        quest.serialize_to(&mut buffer);
        buffer.freeze()
    }
}

impl ByteSize for ActiveQuest {
    fn byte_size(&self) -> usize {
        4 + self.body_byte_size()
    }
}

// --- 0x30D5, the progress packet -------------------------------------------
//
// Idea: this opcode is modelled as "prefix + the record we already had"
// because that is literally how the original parses it — see the module
// header. Nothing here is invented: the two prefix fields are the two reads
// before the switch, and the optional record is the arm both `case 1` and
// `case 2` take.

/// 0x30D5 `AGENT_QUEST_UPDATE` — the packet that keeps a quest journal current.
///
/// * `kind` — `1` add, `2` update, `3` remove, `4` abandon; `3` and `4` share
///   the client's delete arm (`QUEST_UPDATE_*`).
/// * `quest_id` — a `u32` right after `kind`, and the value the client's own
///   debug line prints.
/// * `quest` — present exactly for `kind` 1 and 2: the [`ActiveQuest`] body,
///   with `id` copied from the prefix, because the record body itself has no
///   id. For a delete the client reads nothing further, so this is `None` and
///   re-encodes as nothing.
///
/// **Still open:** whether an `update` for a counter tick resends the whole
/// record (this is what the parse implies, since there is no delta form) and
/// which `state` values mark "ready to hand in".
#[derive(Message, Clone, Debug, Default, PartialEq)]
pub struct QuestUpdate {
    pub kind: u8,
    pub quest_id: u32,
    pub quest: Option<ActiveQuest>,
}

impl QuestUpdate {
    /// Only `case 1` and `case 2` call the record parser, and the default arm
    /// reads nothing at all — so an unknown `kind` is a two-field packet, not
    /// a parse error.
    pub fn carries_record(kind: u8) -> bool {
        matches!(kind, QUEST_UPDATE_ADD | QUEST_UPDATE_UPDATE)
    }

    /// `case 3` and `case 4` are one arm — both delete the quest from the
    /// client's list.
    pub fn is_removal(&self) -> bool {
        matches!(self.kind, QUEST_UPDATE_REMOVE | QUEST_UPDATE_ABANDON)
    }

    /// The `kind` values the handler's switch does not cover. The trailing
    /// bytes of such a frame are unknown, so callers should log rather than
    /// act.
    pub fn is_unknown_kind(&self) -> bool {
        !Self::carries_record(self.kind) && !self.is_removal()
    }
}

impl Deserialize for QuestUpdate {
    fn read_from<T: Read + ReadBytesExt>(reader: &mut T) -> Result<Self, SerializationError> {
        let kind = u8::read_from(reader)?;
        let quest_id = u32::read_from(reader)?;
        let quest = if Self::carries_record(kind) {
            Some(ActiveQuest::read_body_from(quest_id, reader)?)
        } else {
            None
        };
        Ok(QuestUpdate {
            kind,
            quest_id,
            quest,
        })
    }
}

impl Serialize for QuestUpdate {
    fn serialize_to(&self, buf: &mut bytes::BytesMut) {
        self.kind.serialize_to(buf);
        self.quest_id.serialize_to(buf);
        // Gated the same way as on read; the record's own `id` is never written
        // twice because the prefix already carried it.
        if Self::carries_record(self.kind) {
            if let Some(quest) = &self.quest {
                quest.serialize_body_to(buf);
            }
        }
    }
}

impl ByteSize for QuestUpdate {
    fn byte_size(&self) -> usize {
        let mut size = 1 + 4;
        if Self::carries_record(self.kind) {
            if let Some(quest) = &self.quest {
                size += quest.body_byte_size();
            }
        }
        size
    }
}

impl TryFrom<bytes::Bytes> for QuestUpdate {
    type Error = SerializationError;

    fn try_from(data: bytes::Bytes) -> Result<Self, Self::Error> {
        use bytes::Buf;
        let mut reader = data.reader();
        QuestUpdate::read_from(&mut reader)
    }
}

impl From<QuestUpdate> for bytes::Bytes {
    fn from(update: QuestUpdate) -> bytes::Bytes {
        let mut buffer = bytes::BytesMut::with_capacity(update.byte_size());
        update.serialize_to(&mut buffer);
        buffer.freeze()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    /// A real 44-byte active record of quest id 3, as a server sends it inside
    /// CHARACTER_DATA.
    const REAL_RECORD_HEX: &str =
        "03000000100058010101011500534e5f434f4e5f514e4f5f43485f534d4954485f31010000000001f5070000";

    fn real_record() -> Vec<u8> {
        let clean: String = REAL_RECORD_HEX
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .collect();
        clean
            .as_bytes()
            .chunks(2)
            .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect()
    }

    /// A real `0x30D6` frame, decoded field by field and re-encoded
    /// byte-identically.
    #[test]
    fn a_real_quest_mark_add_round_trips() {
        let wire = Bytes::from_static(&[
            0x6c, 0x5e, 0xe6, 0x52, // mark_id
            0x82, // unknown0
            0x01, // mark = New
            0xa8, 0x61, // region 25000
            0x4c, 0x01, 0x00, 0x00, // 332
            0x00, 0x00, 0x00, 0x00, // always zero
            0x7e, 0x05, 0x00, 0x00, // 1406
            0x37, 0x00, 0x00, 0x00, // 55
        ]);
        let decoded = QuestMarkAdd::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.mark_id, 0x52e65e6c);
        assert_eq!(decoded.unknown0, 0x82);
        assert_eq!(decoded.mark, QUEST_MARK_NEW);
        assert_eq!(decoded.region, 25000);
        assert_eq!(
            (
                decoded.unknown1,
                decoded.unknown2,
                decoded.unknown3,
                decoded.unknown4
            ),
            (332, 0, 1406, 55)
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, wire, "24 bytes, nothing left over");
    }

    /// The pairing itself: `0x30D7`'s whole body is the handle an earlier
    /// `0x30D6` introduced.
    #[test]
    fn a_mark_remove_carries_an_earlier_adds_handle() {
        let add = Bytes::from_static(&[
            0xc4, 0x5e, 0xe6, 0x52, 0x82, 0x01, 0xa8, 0x61, 0x4c, 0x01, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x7e, 0x05, 0x00, 0x00, 0x37, 0x00, 0x00, 0x00,
        ]);
        let remove = Bytes::from_static(&[0xc4, 0x5e, 0xe6, 0x52]);

        let added = QuestMarkAdd::try_from(add).unwrap();
        let removed = QuestMarkRemove::try_from(remove.clone()).unwrap();
        assert_eq!(removed.mark_id, added.mark_id);

        let back: Bytes = removed.into();
        assert_eq!(back, remove);
    }

    /// The mark byte a server sends is always one of the published enum
    /// values, which is the one point where the two sources can be held
    /// against each other.
    #[test]
    fn the_mark_values_on_the_wire_are_all_declared_values() {
        for mark in [QUEST_MARK_NEW, QUEST_MARK_COMPLETE, QUEST_MARK_HARD] {
            assert!(matches!(mark, 1 | 3 | 4));
        }
        assert_eq!(QUEST_MARK_OPEN, 2, "declared but never seen on the wire");
    }
    /// The bit reading of `quest_type`, exercised on the two values a server
    /// really sends (`0x18`, `0x58`) plus the two the reading predicts
    /// (`0x08` = no objective list, `0x1C` = time-limited). The last two are
    /// constructed, not observed: they are what the reading says the wire
    /// would look like, and they exist so a real frame can contradict them
    /// here instead of somewhere in a HUD.
    #[test]
    fn the_quest_type_bits_gate_the_two_trailing_lists() {
        // 0x18 = 0x08|0x10: objective list, no NPC list — the common shape.
        let objectives_only = ActiveQuest::try_from(Bytes::from_static(&[
            0x06, 0x00, 0x00, 0x00, // id 6
            0x10, // achievements
            0x00, // autoshare
            0x18, // type
            0x01, // state
            0x01, // 1 objective
            0x01, 0x01, // objective id, enabled
            0x16, 0x00, // 22-byte key
            b'S', b'N', b'_', b'C', b'O', b'N', b'_', b'Q', b'N', b'O', b'_', b'C', b'H', b'_',
            b'P', b'O', b'T', b'I', b'O', b'N', b'_', b'1', 0x01, // 1 task
            0x00, 0x00, 0x00, 0x00,
        ]))
        .expect("a record of that shape parses");
        assert!(objectives_only.has_objective_list());
        assert!(!objectives_only.has_npc_list());
        assert!(!objectives_only.is_time_limited());
        assert_eq!(objectives_only.unknown_type_bits(), 0);
        assert_eq!(objectives_only.objectives.len(), 1);
        assert_eq!(
            objectives_only.objectives[0].name_key,
            "SN_CON_QNO_CH_POTION_1"
        );
        assert!(objectives_only.npcs.is_empty());

        // 0x08 alone: no objective list at all — the value the published
        // sentinel set skips, and the reason this reads bits, not sentinels.
        let bare = ActiveQuest::try_from(Bytes::from_static(&[
            0x2a, 0x00, 0x00, 0x00, 0x10, 0x00, 0x08, 0x03,
        ]))
        .expect("an objective-less record parses");
        assert!(!bare.has_objective_list());
        assert!(bare.objectives.is_empty());
        assert_eq!(bare.state, 3);
        assert_eq!(bare.byte_size(), 8, "eight bytes, nothing implied");

        // 0x1C = 0x04|0x08|0x10: the time-limited form, whose `u32` sits
        // between the type and the state byte.
        let timed = ActiveQuest::try_from(Bytes::from_static(&[
            0x2b, 0x00, 0x00, 0x00, 0x10, 0x00, 0x1c, // type: timed + objectives
            0x2c, 0x01, 0x00, 0x00, // remaining_time 300
            0x01, // state
            0x00, // no objectives despite the bit — count is still on the wire
        ]))
        .expect("a timed record parses");
        assert!(timed.is_time_limited());
        assert_eq!(timed.remaining_time, Some(300));
        assert_eq!(timed.state, 1);
        assert_eq!(timed.unknown_type_bits(), 0);
    }

    /// The `TIME_LIMITED` bit is what the reader gates on, so the writer must
    /// follow the bit, not the `Option`: a record with the bit set and no
    /// `remaining_time` used to serialise four bytes short, and re-reading it
    /// then ate the state byte as the first byte of the time.
    #[test]
    fn a_timed_record_without_a_time_still_writes_its_four_bytes() {
        let quest = ActiveQuest {
            id: 42,
            achievements: 16,
            autoshare: 0,
            quest_type: QUEST_TYPE_TIME_LIMITED | QUEST_TYPE_HAS_STATE,
            remaining_time: None,
            state: 7,
            objectives: Vec::new(),
            npcs: Vec::new(),
        };
        let wire: Bytes = quest.clone().into();
        assert_eq!(
            wire.as_ref(),
            &[0x2a, 0x00, 0x00, 0x00, 0x10, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x07]
        );
        assert_eq!(wire.len(), quest.byte_size(), "byte_size must agree");
        let back = ActiveQuest::try_from(wire).expect("the record reads back");
        assert_eq!(back.remaining_time, Some(0));
        assert_eq!(back.state, 7, "the state byte was not eaten by the time");
    }

    /// A list longer than its one-byte count is cut with the count, so the
    /// reader never runs past the rows the count names.
    #[test]
    fn an_npc_list_longer_than_its_count_byte_is_cut_with_it() {
        let quest = ActiveQuest {
            id: 1,
            achievements: 16,
            autoshare: 0,
            quest_type: QUEST_TYPE_HAS_NPC_LIST,
            remaining_time: None,
            state: 0,
            objectives: Vec::new(),
            npcs: (0..300).collect(),
        };
        let wire: Bytes = quest.clone().into();
        assert_eq!(wire.len(), quest.byte_size());
        let back = ActiveQuest::try_from(wire).expect("the record reads back");
        assert_eq!(back.npcs.len(), 255);
        assert_eq!(back.npcs.last(), Some(&254));
    }

    /// A record whose type carries a bit this reading does not know must still
    /// parse, keep the raw byte, and *report* the surprise — dropping it would
    /// hide the one observation that can falsify the bit-field reading.
    #[test]
    fn an_unknown_type_bit_survives_the_parse_as_a_number() {
        let wire = Bytes::from_static(&[
            0x8f, 0x01, 0x00, 0x00, // id 399
            0x10, 0x00, 0xd8, // type 0xD8 = 0x58 + an unknown 0x80
            0x08, // state
            0x00, // 0 objectives
            0x01, 0xf5, 0x07, 0x00, 0x00, // 1 npc: 2037
        ]);
        let quest = ActiveQuest::try_from(wire.clone()).expect("unknown bits must not reject");
        assert_eq!(quest.quest_type, 0xd8);
        assert_eq!(quest.unknown_type_bits(), 0x80);
        assert!(quest.has_objective_list() && quest.has_npc_list());
        assert_eq!(quest.npcs, vec![2037]);

        // And it re-encodes byte-identically, so an unknown bit cannot corrupt
        // a relay of the record either.
        let back: Bytes = quest.into();
        assert_eq!(back, wire);
    }

    /// A `0x30D5` update whose record half is a real record body. The only
    /// bytes that are *not* from the wire are the two prefix fields, which are
    /// the two reads the original performs before its switch — with the id
    /// moved out of the record and into the prefix, exactly as the handler
    /// does.
    #[test]
    fn a_kind_2_update_carries_the_record_body() {
        // kind = 2 (modify), quest_id = 3, then the record's body.
        let record = real_record();
        let mut wire = vec![QUEST_UPDATE_UPDATE];
        wire.extend_from_slice(&record[..4]); // the record's own id, as prefix
        wire.extend_from_slice(&record[4..]); // achievements .. npc list
        let wire = Bytes::from(wire);

        let update = QuestUpdate::try_from(wire.clone()).expect("the handler's read order parses");
        assert_eq!(update.kind, QUEST_UPDATE_UPDATE);
        assert_eq!(update.quest_id, 3);
        let quest = update.quest.as_ref().expect("kind 2 carries a record");
        assert_eq!(quest.id, 3, "the id comes from the prefix, not the body");
        assert_eq!(quest.achievements, 16);
        assert_eq!(quest.quest_type, 0x58);
        assert!(quest.has_state() && quest.has_objective_list() && quest.has_npc_list());
        assert_eq!(quest.state, 1);
        assert_eq!(quest.objectives.len(), 1);
        assert_eq!(quest.objectives[0].name_key, "SN_CON_QNO_CH_SMITH_1");
        assert_eq!(quest.objectives[0].tasks, vec![0]);
        assert_eq!(quest.npcs, vec![2037]);
        assert_eq!(
            quest.byte_size(),
            record.len(),
            "the record body plus its id is the whole record, byte for byte"
        );

        // And it re-encodes byte-identically: 45 bytes, nothing implied.
        let back: Bytes = update.into();
        assert_eq!(back, wire);
        assert_eq!(back.len(), 1 + record.len());
    }

    /// The counter tick this lane exists for: the same record with `tasks[0]`
    /// one higher is a valid `kind 2` body of the same length, so a progress
    /// update is a whole record and the counter is the only byte that moves.
    /// Built from the real record by changing that one `u32`.
    #[test]
    fn a_counter_tick_moves_exactly_one_u32_of_the_record() {
        let record = real_record();
        let mut wire = vec![QUEST_UPDATE_UPDATE];
        wire.extend_from_slice(&record);
        let before = QuestUpdate::try_from(Bytes::from(wire.clone())).expect("parses");
        assert_eq!(
            before.quest.as_ref().expect("record").objectives[0].tasks,
            vec![0]
        );

        // the task list sits at a fixed place in this record: prefix(1) + id(4)
        // + achievements/autoshare/type/state(4) + objective count(1)
        // + objective id/enabled(2) + u16 length(2) + 21 key bytes + count(1)
        let task_at = 1 + 4 + 4 + 1 + 2 + 2 + 21 + 1;
        assert_eq!(&wire[task_at..task_at + 4], &[0, 0, 0, 0]);
        wire[task_at] = 1;
        let after = QuestUpdate::try_from(Bytes::from(wire.clone())).expect("parses");
        let quest = after.quest.as_ref().expect("record");
        assert_eq!(quest.objectives[0].tasks, vec![1], "little-endian u32");
        assert_eq!(
            quest.byte_size(),
            before.quest.as_ref().expect("record").byte_size(),
            "a counter tick does not change the record's length"
        );
        let back: Bytes = after.into();
        assert_eq!(back, Bytes::from(wire));
    }

    /// The delete arm: `kind` 3 and 4 are five bytes total, because neither
    /// calls the record parser. A parser that expected a record here would
    /// consume the *next* packet's bytes.
    #[test]
    fn removal_and_abandon_are_a_five_byte_packet() {
        for kind in [QUEST_UPDATE_REMOVE, QUEST_UPDATE_ABANDON] {
            let wire = Bytes::from(vec![kind, 0x39, 0x00, 0x00, 0x00]);
            let update = QuestUpdate::try_from(wire.clone()).expect("five bytes are enough");
            assert_eq!(update.quest_id, 57);
            assert!(update.quest.is_none(), "no record on the delete arm");
            assert!(update.is_removal());
            assert!(!update.is_unknown_kind());
            assert_eq!(update.byte_size(), 5);
            let back: Bytes = update.into();
            assert_eq!(back, wire);
        }
    }

    /// A `kind` the handler's switch does not cover falls through and reads
    /// nothing at all. So an unknown kind must parse, be flagged, and not
    /// invent a record.
    #[test]
    fn an_unknown_kind_parses_as_the_prefix_alone() {
        let wire = Bytes::from_static(&[0x07, 0xdc, 0x00, 0x00, 0x00]);
        let update = QuestUpdate::try_from(wire.clone()).expect("unknown kinds must not reject");
        assert_eq!(update.kind, 7);
        assert_eq!(update.quest_id, 220);
        assert!(update.quest.is_none());
        assert!(update.is_unknown_kind());
        let back: Bytes = update.into();
        assert_eq!(back, wire);
    }

    /// The `add` arm carries the same record shape as `update` — one function,
    /// two call sites (`008812b0` cases 1 and 2 both call `008b94d0`). This is
    /// the shape a newly accepted quest arrives in, and the journal test in
    /// `client/src/plugins/net/quest.rs` consumes exactly this.
    #[test]
    fn the_add_arm_uses_the_same_record_parser() {
        let mut wire = vec![QUEST_UPDATE_ADD];
        wire.extend_from_slice(&real_record());
        let update = QuestUpdate::try_from(Bytes::from(wire.clone())).expect("parses");
        assert_eq!(update.kind, QUEST_UPDATE_ADD);
        assert!(QuestUpdate::carries_record(update.kind));
        assert!(!update.is_removal() && !update.is_unknown_kind());
        assert_eq!(
            update.quest.as_ref().expect("record").objectives[0].name_key,
            "SN_CON_QNO_CH_SMITH_1"
        );
        let back: Bytes = update.into();
        assert_eq!(back, Bytes::from(wire));
    }

    /// The `0x08` gate, which no real frame can show because every record seen
    /// has the bit set: without it the client reads **no** state byte. The
    /// falsifier lives here rather than in a HUD.
    #[test]
    fn a_record_without_the_state_bit_has_no_state_byte() {
        // type 0x50 = objectives + npc list, state bit clear.
        let wire = Bytes::from_static(&[
            0x2a, 0x00, 0x00, 0x00, // id 42
            0x10, 0x00, 0x50, // achievements, autoshare, type
            0x00, // objective count (no state byte in between)
            0x01, 0xf5, 0x07, 0x00, 0x00, // one npc
        ]);
        let quest = ActiveQuest::try_from(wire.clone()).expect("the gated form parses");
        assert!(!quest.has_state());
        assert_eq!(quest.state, 0, "absent on the wire, so zero — not read");
        assert!(quest.objectives.is_empty());
        assert_eq!(quest.npcs, vec![2037]);
        assert_eq!(quest.unknown_type_bits(), 0);
        assert_eq!(quest.byte_size(), wire.len());
        let back: Bytes = quest.into();
        assert_eq!(back, wire, "and a missing byte is not written back");
    }

    /// The objective key is a string-table key, and the struct says so: the
    /// wire never carries prose, so a consumer that wants text has to look it
    /// up.
    #[test]
    fn the_objective_name_is_a_string_table_key() {
        let objective = QuestObjective::try_from(Bytes::from_static(&[
            0x02, 0x01, 0x1a, 0x00, b'S', b'N', b'_', b'C', b'O', b'N', b'_', b'Q', b'N', b'O',
            b'_', b'C', b'H', b'_', b'G', b'E', b'N', b'A', b'R', b'A', b'L', b'_', b'1', b'_',
            b'0', b'2', 0x01, 0x00, 0x00, 0x00, 0x00,
        ]))
        .expect("an objective parses");
        assert_eq!(objective.name_key, "SN_CON_QNO_CH_GENARAL_1_02");
        assert!(
            objective.name_key.starts_with("SN_CON_"),
            "the wire sends the key, the client owns the sentence"
        );
        assert_eq!(objective.tasks, vec![0]);
    }
}
