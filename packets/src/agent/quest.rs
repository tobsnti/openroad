//! Quest wire opcodes — currently the **mark** pair only (0x30D6 / 0x30D7).
//!
//! Idea, and the reason this module is three structs rather than seventeen:
//! the quest family is the one family in the round-2 ledger (#451, #758) whose
//! layouts are recorded **nowhere**. SilkroadDoc-wiki indexes eleven quest
//! opcodes and every one of its body pages is an empty stub; go-sro has no
//! quest handler at all; xBot declares the opcodes and dispatches none. So the
//! only usable evidence on this machine is our own `packet_dump/`, and it
//! covers exactly two of them — which is what is modelled here. The whole
//! family's ledger, per opcode with its handler VA and the reason it is not
//! wired, is `docs/net-quest.md`.
//!
//! **Capture-verified, not spec-derived** — the opposite of most of this crate.
//! The 12 `packet_dump/` trees on this machine hold 294 distinct `0x30D6`
//! bodies and 47 `0x30D7` bodies (2026-08-10 … 2026-09-01) of real v1.188
//! server traffic, and the tests below decode literal lines from them. That
//! corpus is also what resolved the add's trailing four `u32`s — see
//! [`QuestMarkAdd`] — by joining them against the `0x3019` spawn records of
//! the same sessions.
//!
//! The pairing is proven by the captures themselves, not assumed: every
//! `0x30D7` body in the dump is byte-for-byte the leading `u32` of an *earlier*
//! `0x30D6` (`f05ee652` @ 09:50:38 removes the mark added at 09:50:13, and so
//! on for all 47). That is what identifies the leading field as a **mark
//! handle** and `0x30D7` as "drop that mark".

use bevy::prelude::Message;

use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

/// `QuestMark` — the icon the minimap/world draws for a mark. Values from
/// SilkroadDoc-wiki's `QuestMark` enum page, and corroborated by the capture:
/// the dump carries all four and nothing else.
pub const QUEST_MARK_NEW: u8 = 1;
pub const QUEST_MARK_OPEN: u8 = 2;
pub const QUEST_MARK_COMPLETE: u8 = 3;
pub const QUEST_MARK_HARD: u8 = 4;

/// 0x30D6 — add a quest mark.
///
/// The trailing four `u32`s were `unknown1`..`unknown4` until 2026-10-23. They
/// are the **target's spawn record**, and the capture proves it without a
/// decompile: all 294 dumped `0x30D6` bodies carry one of only 8 distinct
/// `(region, a, b, c, d)` combinations, and every one of those 8 is a
/// `0x3019` group-spawn record in the same dump — `ref_obj_id`, `unique_id`,
/// then `region` + three `f32` region-local coordinates, the layout
/// `character_data.rs` already models. `(a, b, c)` is that record's position
/// truncated to whole units and `d` is its `unique_id`.
///
/// The cleanest of the 8, both lines at the same millisecond:
///
/// ```text
/// 0x3019 2026-08-10T18:19:22.552Z  d5070000 f3000000 a861 8f02c644 00000000 48e9af44
///                                  ref 2005 uid  243  25000  1584.08      0.0  1407.29
/// 0x30d6 2026-08-10T18:19:22.552Z  e847e652 82 01    a861 30060000 00000000 7f050000 f3000000
///                                  mark_id      New  25000     1584        0     1407      243
/// ```
///
/// Field confidence, from 294 distinct samples (12 `packet_dump/` trees,
/// 2026-08-10 … 2026-09-01):
///
/// * `mark_id` **[V]** — the handle `0x30D7` later removes; 47 of 47 captured
///   removes name an earlier add of the same session. Monotonic with a stride
///   of 44 (278 of 293 gaps are exactly 44, the rest multiples of it), i.e. a
///   server-side pool index, not a quest id: the same target's mark gets a new
///   value each time it is re-added (114 distinct handles for one target).
/// * `unknown0` **[U]** — `0x82` in all 294 samples. Constant, so the capture
///   cannot say what it means; it may equally be the high byte of a `u16`
///   with `mark`.
/// * `mark` **[S]** — 1 / 2 / 3 / 4 in the capture, exactly the wiki's
///   `QuestMark` enum. (`Open` did appear after all — 88 samples from
///   2026-08-19 on, which the 18-sample reading had called "never".)
/// * `region` **[V]** — the *target's* region, not the player's: it is the
///   region of the matching spawn record. 25000 and 25255 in the capture
///   (`docs/formats/` region encoding: sectors 168/97 and 167/98, neighbours).
/// * `x`, `y`, `z` **[V]** — the target's region-local position, truncated
///   **toward zero** rather than floored: the NPC behind `unique_id` 343 sits
///   at y = -0.407 and the wire carries 0, the one behind 311 at y = 9.066 and
///   the wire carries 9. So a target below sea level arrives wrapped as a
///   large `u32` — read `y` back through `as i32`. No sample has a negative
///   height, so that half stays **[U]**.
/// * `target_unique_id` **[V]** — the target entity's server unique id, the
///   same key `NetworkEntities` indexes spawns by. All 8 captured targets are
///   NPCs (`characterdata_5000.txt` ref ids 2003/2005/2008/2009/2010/2019/
///   2021/2037 = `NPC_CH_SMITH`, `_POTION`, `_ACCESSORY`, `_HORSE`,
///   `_SPECIAL`, `_GENARAL`, `_GENARAL_SP`, `_SOLDIER_SO1`), which is what a
///   quest mark points at.
///
/// Not resolved, and **measured** not to be resolvable from what this machine
/// holds: a quest id is nowhere in the body, so a mark cannot carry a quest
/// name. Three routes were tried and all three fall short:
///
/// * **This field.** Its values (55, 186, 215, 243, 311, 343, 345, 346) looked
///   like `questdata.txt` ids and are not — three of the eight have no row in
///   that table at all, while all eight are spawn `unique_id`s exactly.
/// * **The player's own tables.** No quest table names an NPC.
///   `questdata.txt` (12 columns) and `questcontentsdata.txt` (17) have no NPC
///   field, and across all 154 `textdata` files exactly one row carries both a
///   `QNO_` and an `NPC_` token (`worldmapguidedata.txt`) — and spends its
///   `NPC_` on a place-name key, `SN_NPC_SD_M_AREA_BERENICE`. That table's
///   target column only ever names `MOB_*`.
/// * **The wire's own quest records.** CHARACTER_DATA does carry a
///   count-prefixed list of NPC **ref** ids at the end of an active-quest
///   record, behind `quest_type` bit `0x40`, and the capture confirms that
///   layout byte-for-byte (96 of the 98 `0x3013` bodies holding an `SN_CON_`
///   key parse strictly, every one with the same nine quest ids). It still
///   cannot name a mark: only 2 of those 9 quests carry the list, both name
///   the same ref `2037`, so that ref maps to **two** quests (3
///   `QNO_CH_SMITH_1` and 399 `QTUTORIAL2_CH_1`). One of the eight captured
///   mark targets is covered, and ambiguously. `0x30D5`, the journal delta
///   that would carry the rest, has **no captured frame at all** across 112
///   dump trees.
///
/// **Resolve:** decompile the mark handler at `008811e0` and read whether the
/// original labels a mark *at all*. The join it would need (`unique_id` → ref
/// id → quest) exists in no single packet, so "the original shows no name
/// either" is a live possibility rather than a gap in this client. If it does
/// label, the capture that shows how is a login whose active-quest block names
/// exactly one `0x40` NPC that a `0x30D6` of the same session also points at.
///
/// What **is** available, and is the better name source than the current
/// spawned-entity join: the mark's own position is a row of the player's
/// `npcpos.txt` (`ref_obj_id, region, x, y, z`) — all 8 captured marks match
/// one exactly on region/x/z, which yields the target's `ref_obj_id` and
/// through `characterdata_*.txt` its `SN_NPC_*` name without the target having
/// to be spawned. That is the NPC's name, not the quest's.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct QuestMarkAdd {
    pub mark_id: u32,
    pub unknown0: u8,
    /// See the `QUEST_MARK_*` constants.
    pub mark: u8,
    /// The target's region — see the type's docs; dungeon ids carry bit 15.
    pub region: u16,
    pub x: u32,
    /// Height, truncated toward zero; reinterpret as `i32` before use.
    pub y: u32,
    pub z: u32,
    pub target_unique_id: u32,
}

/// 0x30D7 — remove the quest mark with this handle. Whole body, 4 bytes
/// **[V]**: every captured sample is a handle a previous `0x30D6` introduced.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct QuestMarkRemove {
    pub mark_id: u32,
}

/// 0x30D5 — server → client journal delta, the packet that carries quest
/// progress *and* the countdown panel. Modelled from our own decompile of the
/// handler at `008812b0`, because no capture of this opcode exists in any of
/// our dump trees.
///
/// Idea of the shape: the handler reads the two prefix fields unconditionally
/// and then switches on `subtype & 0xff`. Subtypes 1 and 2 both call the same
/// field reader (`008b94d0`); subtypes 3 and 4 share a branch body whose three
/// calls (`00778d10`, `009d4c60`, `008b9910`) contain **no** packet read at
/// all, so those two carry nothing past the prefix. Anything outside 1..4
/// falls out of the switch, which is why an unknown subtype must decode to an
/// empty body instead of failing.
///
/// `opaque` is deliberately unnamed: the handler passes it to four different
/// consumers without ever narrowing it, so a name here would be a guess.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct QuestJournalUpdate {
    /// Only 1..4 reach a branch body. `008812b0` masks with `& 0xff`.
    pub subtype: u8,
    /// Read unconditionally before the switch; meaning is **[U]**.
    pub opaque: u32,
    #[sro_packet(when = "subtype == 1 || subtype == 2")]
    pub unk_08: Option<u8>,
    #[sro_packet(when = "subtype == 1 || subtype == 2")]
    pub unk_09: Option<u8>,
    /// The field selector for everything below it (record `+0x0A`).
    #[sro_packet(when = "subtype == 1 || subtype == 2")]
    pub flags: Option<u8>,
    /// Flag `0x04`. `0xFFFFFFFF` means "no timer": the handler compares the
    /// stored value against `-1` and rewrites it to 0 before anything reads it.
    #[sro_packet(when = "flags.is_some_and(|f| f & 0x04 != 0)")]
    pub timer: Option<u32>,
    /// Flag `0x08` (record `+0x10`); meaning **[U]**.
    #[sro_packet(when = "flags.is_some_and(|f| f & 0x08 != 0)")]
    pub unk_10: Option<u8>,
    /// Flag `0x10` — a counted list of single bytes, each passed through
    /// `008b8d80` and, when that returns non-zero, into `008b6ed0`.
    #[sro_packet(when = "flags.is_some_and(|f| f & 0x10 != 0)")]
    pub entry_count: Option<u8>,
    #[sro_packet(list_type = "by-size-field", size_field = "entry_count.unwrap_or(0)")]
    pub entries: Vec<u8>,
}

impl QuestJournalUpdate {
    /// `0xFFFFFFFF` is the handler's "inactive" sentinel, not a duration.
    pub const TIMER_INACTIVE: u32 = 0xFFFF_FFFF;

    pub fn timer_secs(&self) -> Option<u32> {
        self.timer.filter(|t| *t != Self::TIMER_INACTIVE)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use bytes::Bytes;

    /// A literal line from `packet_dump/0x30d6.log` (2026-08-11T09:43:45Z),
    /// decoded field by field and re-encoded byte-identically.
    #[test]
    fn a_captured_quest_mark_add_round_trips() {
        let wire = Bytes::from_static(&[
            0x6c, 0x5e, 0xe6, 0x52, // mark_id
            0x82, // unknown0
            0x01, // mark = New
            0xa8, 0x61, // region 25000
            0x4c, 0x01, 0x00, 0x00, // x 332
            0x00, 0x00, 0x00, 0x00, // y 0
            0x7e, 0x05, 0x00, 0x00, // z 1406
            0x37, 0x00, 0x00, 0x00, // target unique id 55
        ]);
        let decoded = QuestMarkAdd::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.mark_id, 0x52e65e6c);
        assert_eq!(decoded.unknown0, 0x82);
        assert_eq!(decoded.mark, QUEST_MARK_NEW);
        assert_eq!(decoded.region, 25000);
        // `packet_dump/0x3019.log` 2026-09-01T14:50:11.763Z spawns unique id
        // 55 (ref 2003, NPC_CH_SMITH) at region 25000, (332.73, 0.0, 1406.70).
        assert_eq!((decoded.x, decoded.y, decoded.z), (332, 0, 1406));
        assert_eq!(decoded.target_unique_id, 55);
        let back: Bytes = decoded.into();
        assert_eq!(back, wire, "24 bytes, nothing left over");
    }

    /// The capture's own proof of the pairing: `0x30D7`'s whole body is the
    /// handle an earlier `0x30D6` introduced (`c45ee652`, added 09:50:06 and
    /// removed 09:51:17 on 2026-08-11).
    #[test]
    fn a_captured_mark_remove_carries_an_earlier_adds_handle() {
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

    /// The mark byte is the wiki's enum, and the capture only ever carries
    /// values from it — the one cross-check available between the two sources.
    /// All four now have samples (`Open` since 2026-08-19).
    #[test]
    fn the_captured_mark_values_are_all_wiki_enum_values() {
        assert_eq!(
            [
                QUEST_MARK_NEW,
                QUEST_MARK_OPEN,
                QUEST_MARK_COMPLETE,
                QUEST_MARK_HARD
            ],
            [1, 2, 3, 4]
        );
    }

    /// `0x30D5`'s branch structure, pinned against the decompile of
    /// `008812b0` + `008b94d0` — there is no captured frame to replay, so the
    /// test states what the handler reads rather than what a server sent.
    ///
    /// Subtypes 3 and 4 are the interesting half: their branch body holds no
    /// packet read at all, so the body must stop after the prefix. A model that
    /// read the flag byte unconditionally would consume two bytes that are not
    /// there.
    #[test]
    fn journal_update_reads_a_body_only_for_subtypes_one_and_two() {
        let decode = |wire: &[u8]| {
            let mut cursor = Cursor::new(wire);
            let decoded = QuestJournalUpdate::read_from(&mut cursor).expect("decodes");
            assert_eq!(
                cursor.position() as usize,
                wire.len(),
                "the body must come out even"
            );
            assert_eq!(decoded.byte_size(), wire.len());
            decoded
        };

        // Subtype 3: prefix only.
        let bare = decode(&[0x03, 0x2a, 0x00, 0x00, 0x00]);
        assert_eq!(bare.subtype, 3);
        assert_eq!(bare.opaque, 42);
        assert!(bare.flags.is_none());

        // Subtype 2 with no flag bits: the three bytes behind the prefix are
        // unconditional once the branch is taken.
        let plain = decode(&[0x02, 0x01, 0x00, 0x00, 0x00, 0x07, 0x08, 0x00]);
        assert_eq!(plain.flags, Some(0));
        assert!(plain.timer.is_none());
        assert!(plain.entries.is_empty());

        // Flag 0x04 pulls the timer, and 0xFFFFFFFF is the inactive sentinel
        // rather than a duration.
        let inactive = decode(&[
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0xff, 0xff, 0xff, 0xff,
        ]);
        assert_eq!(inactive.timer, Some(QuestJournalUpdate::TIMER_INACTIVE));
        assert_eq!(inactive.timer_secs(), None);

        // All three conditional groups at once, in the order the handler reads
        // them: timer (0x04), the single byte (0x08), then the counted list
        // (0x10).
        let full = decode(&[
            0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1c, 0x3c, 0x00, 0x00, 0x00, 0x09, 0x02,
            0x11, 0x22,
        ]);
        assert_eq!(full.timer_secs(), Some(60));
        assert_eq!(full.unk_10, Some(9));
        assert_eq!(full.entries, vec![0x11, 0x22]);
    }
}
