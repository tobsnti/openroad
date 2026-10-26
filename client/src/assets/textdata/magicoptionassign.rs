//! `magicoptionassign.txt` — which magic attribute may go on which item.
//!
//! Idea: alchemy's legality rule is authored data, not code. The file lists,
//! per item class, the `MATTR_*` options that item may carry, so "may this
//! elixir go on this sword?" is a table lookup rather than a rule we write
//! down. The attribute names join directly onto `magicoption.txt`, which the
//! client already loads: all 49 distinct names in this file have a row there.
//!
//! **The table names nothing itself** — no header, no column comment, only a
//! trailing `//DBtoMedia` line (the same literal `refquestrewarditems.txt`
//! carries). The column meanings below are therefore measured against the
//! player's own `itemdata_*.txt`, with our established column reader:
//!
//! * `[0]` service flag, `1` in all 74 rows
//! * `[1]` origin, the same 0 = Chinese / 1 = European column
//!   [`ItemDataRow::country`](super::itemdata::ItemDataRow::country) reads,
//!   plus `3` on the rows that belong to both (job suits, avatars, the devil
//!   spirit)
//! * `[2]`/`[3]` the item's `TID3`/`TID4` — equipment is `TID1/2 == 3/1`, and
//!   the 74 rows cover 68 of the 70 `(TID3, TID4)` pairs the player's 8650
//!   equipment rows actually use
//! * `[4..30]` 26 slots, each a `MATTR_*` codename or the `xxx` placeholder;
//!   the longest real list is 20 (the avatar row `13/2`), the shortest 2
//!
//! **The origin column is not part of the rule.** Five pairs appear twice,
//! once for each origin (`7/1`, `7/2`, `7/3`, `7/6`, `7/7` — job equipment),
//! and both rows carry the *same* list; no pair in the file disagrees across
//! origins. So the key is `(TID3, TID4)`, which is also what keeps the lookup
//! working for the avatar rows, where the file says origin 3 and 84 of the
//! matching items say 0.
//!
//! **Two answers, not one.** A pair the file does not list at all (fortress
//! weapons `6/16`, the free-PvP voucher `7/5` — 24 of 8650 items) and an
//! attribute the file never mentions anywhere (10 of the 29 attributes the
//! player's 300 magic stones carry, among them all 36 `MATTR_ASTRAL` stones)
//! are **undecided**, not forbidden: this table governs the ordinary magic
//! stone, and reading its silence as a prohibition would ban a stone the game
//! ships. Only a listed pair *plus* an attribute the file knows yields a hard
//! `false`.

use std::collections::{HashMap, HashSet};

/// Where the attribute list starts; `[0..4]` are the four numeric columns.
const FIRST_ATTR_COL: usize = 4;
/// `[2]` and `[3]`: the item's `TID3` and `TID4`.
const TID3_COL: usize = 2;
const TID4_COL: usize = 3;

/// The authored legality table, keyed by the item's `(TID3, TID4)`.
#[derive(Debug, Clone, Default)]
pub struct MagicOptionAssign {
    allowed: HashMap<(u32, u32), Vec<String>>,
    /// Every attribute the file mentions, so "the table says nothing about
    /// this attribute" can be told apart from "this item may not have it".
    known: HashSet<String>,
}

impl MagicOptionAssign {
    pub fn parse(content: &str) -> Self {
        let mut table = Self::default();
        for line in content.lines() {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() <= FIRST_ATTR_COL {
                continue;
            }
            let (Ok(tid3), Ok(tid4)) = (
                cols[TID3_COL].trim().parse::<u32>(),
                cols[TID4_COL].trim().parse::<u32>(),
            ) else {
                continue;
            };
            let attrs: Vec<String> = cols[FIRST_ATTR_COL..]
                .iter()
                .map(|a| a.trim())
                .filter(|a| a.starts_with("MATTR_"))
                .map(String::from)
                .collect();
            if attrs.is_empty() {
                continue;
            }
            table.known.extend(attrs.iter().cloned());
            // The second row of a doubly-listed pair carries the same list, so
            // overwriting is a no-op; it is not a merge, because a merge would
            // invent a union the file does not author.
            table.allowed.insert((tid3, tid4), attrs);
        }
        table
    }

    /// The attributes this item class may carry, or `None` where the file has
    /// no row for it.
    pub fn allowed(&self, tid3: u32, tid4: u32) -> Option<&[String]> {
        self.allowed.get(&(tid3, tid4)).map(Vec::as_slice)
    }

    /// Whether this attribute may go on this item class. `None` means the file
    /// does not decide — no row for the class, or an attribute it never
    /// mentions (see the module note).
    pub fn permits(&self, tid3: u32, tid4: u32, attr: &str) -> Option<bool> {
        if !self.known.contains(attr) {
            return None;
        }
        let allowed = self.allowed(tid3, tid4)?;
        Some(allowed.iter().any(|a| a == attr))
    }

    /// Rows read, for the load log.
    pub fn len(&self) -> usize {
        self.allowed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.allowed.is_empty()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Four rows copied from the player's own file: a CH sword (`6/2`), light
    /// armour (`1/1`), and the job-equipment pair `7/1` that is listed twice,
    /// once per origin, with the same list. Shortened to six attribute slots —
    /// the parser takes the row's width as it finds it.
    const ROWS: &str = "1\t0\t6\t2\tMATTR_STR\tMATTR_HR\tMATTR_REPAIR\txxx\txxx\txxx\n\
                        1\t0\t1\t1\tMATTR_STR\tMATTR_HP\tMATTR_MP\txxx\txxx\txxx\n\
                        1\t0\t7\t1\tMATTR_STR_3JOB\tMATTR_INT_3JOB\txxx\txxx\txxx\txxx\n\
                        1\t1\t7\t1\tMATTR_STR_3JOB\tMATTR_INT_3JOB\txxx\txxx\txxx\txxx\n\
                        //DBtoMedia";

    #[test]
    fn the_padding_and_the_trailing_comment_are_not_rows() {
        let table = MagicOptionAssign::parse(ROWS);
        // three pairs, not four: the doubly-listed `7/1` is one class
        assert_eq!(table.len(), 3);
        assert_eq!(
            table.allowed(6, 2),
            Some(
                [
                    "MATTR_STR".to_string(),
                    "MATTR_HR".to_string(),
                    "MATTR_REPAIR".to_string()
                ]
                .as_slice()
            )
        );
        assert_eq!(table.allowed(7, 1).map(<[String]>::len), Some(2));
    }

    /// The claim the window needs, with the two attributes the player's file
    /// really separates: a hit-ratio stone belongs on the sword and not on the
    /// armour, and the HP stone the other way round.
    #[test]
    fn a_stones_attribute_is_decided_by_the_items_class() {
        let table = MagicOptionAssign::parse(ROWS);
        assert_eq!(table.permits(6, 2, "MATTR_HR"), Some(true));
        assert_eq!(table.permits(1, 1, "MATTR_HR"), Some(false));
        assert_eq!(table.permits(1, 1, "MATTR_HP"), Some(true));
        assert_eq!(table.permits(6, 2, "MATTR_HP"), Some(false));
    }

    /// Silence is not a prohibition, in both of its forms.
    #[test]
    fn what_the_table_does_not_cover_stays_undecided() {
        let table = MagicOptionAssign::parse(ROWS);
        // the fortress hammer's class has no row in the real file either
        assert_eq!(table.permits(6, 16, "MATTR_HR"), None);
        assert_eq!(table.allowed(6, 16), None);
        // and an attribute the file never mentions is outside its scope
        assert_eq!(table.permits(6, 2, "MATTR_ASTRAL"), None);
    }
}
