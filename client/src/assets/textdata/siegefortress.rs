//! The siege tables the client loads, and the two that answer a question the
//! wire could not: `siegefortress.txt` (read in `sro_textdata::siegefortress`,
//! which the server shares) and `siegefortressbattlerank.txt`.
//!
//! Idea: `0x385F` carries fortress **ids** and never a fortress **name** — the
//! name is on disk, and so is the ladder behind the rank byte. Both chains are
//! followed rather than guessed:
//!
//! * `siegefortress.txt` turns the ids into codenames and name keys.
//! * `siegefortressbattlerank.txt` is six rows with a rising point threshold
//!   (45, 75, 135, 210, 300, 450) and an icon per rank.
//!
//! One column is **not** what its position suggests. Its fifth field reads
//! 21234..=21239, which looks like a text id and is not: those numbers resolve
//! in `characterdata_25000.txt` as `COS_GUARD_EU_BOW_01_116`..`_121`, guard
//! units at levels 116-121. It is a character id, so it is named one.
//!
//! **Version note.** Every statement above is measured against one drop
//! (`Media/server_dep/silkroad/textdata`). Row counts and column counts are
//! the obvious place where another client version differs, so the parsers read
//! *by index with a minimum width* and ignore trailing fields instead of
//! demanding an exact column count.

use std::collections::BTreeMap;

/// One row of `siegefortressbattlerank.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattleRank {
    /// 1..=6 in this drop.
    pub rank: u8,
    /// Points at which the rank is reached: 45, 75, 135, 210, 300, 450.
    pub points: u32,
    /// `characterdata` id of the guard unit, **not** a text id — 21234..=21239
    /// are `COS_GUARD_EU_BOW_01_116`..`_121`.
    pub guard_character_id: u32,
    /// `etc\rank_centurion.ddj`. The file stem is the only English name this
    /// drop carries for a rank.
    pub icon: String,
}

/// Ordered by rank.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BattleRankTable(pub BTreeMap<u8, BattleRank>);

impl BattleRankTable {
    pub fn parse(content: &str) -> Self {
        BattleRankTable(
            content
                .lines()
                .filter_map(parse_rank)
                .map(|r| (r.rank, r))
                .collect(),
        )
    }

    /// The highest rank whose threshold `points` reaches, if any.
    pub fn for_points(&self, points: u32) -> Option<&BattleRank> {
        self.0.values().filter(|r| r.points <= points).next_back()
    }
}

fn parse_rank(line: &str) -> Option<BattleRank> {
    let c: Vec<&str> = line.trim_end().split('\t').collect();
    if c.len() < 6 || c[0].trim_start_matches('\u{feff}') != "1" {
        return None;
    }
    Some(BattleRank {
        rank: c[1].trim().parse().ok()?,
        points: c[3].trim().parse().ok()?,
        guard_character_id: c[4].trim().parse().ok()?,
        icon: c[5].to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Transcribed from `textdata/siegefortressbattlerank.txt`.
    const RANK_ROWS: &str = "\u{feff}1\t1\t???\t45\t21234\tetc\\rank_assault_soilder.ddj\t\n\
         1\t2\t?????\t75\t21235\tetc\\rank_elite_assault_soilder.ddj\t\n\
         1\t3\t????\t135\t21236\tetc\\rank_centurion.ddj\t\n\
         1\t4\t????\t210\t21237\tetc\\rank_assault_leader.ddj\t\n\
         1\t5\t?????\t300\t21238\tetc\\rank_elite_guard.ddj\t\n\
         1\t6\t????\t450\t21239\tetc\\rank_combat_commander.ddj\t";

    /// Six ranks, a rising threshold, and the fifth column read as what it
    /// resolves to: a guard character id.
    #[test]
    fn the_rank_ladder_rises_and_its_fifth_column_is_a_character_id() {
        let table = BattleRankTable::parse(RANK_ROWS);
        assert_eq!(
            table.0.values().map(|r| r.points).collect::<Vec<_>>(),
            vec![45, 75, 135, 210, 300, 450]
        );
        assert_eq!(
            table
                .0
                .values()
                .map(|r| r.guard_character_id)
                .collect::<Vec<_>>(),
            vec![21234, 21235, 21236, 21237, 21238, 21239]
        );
        assert_eq!(table.0[&3].icon, "etc\\rank_centurion.ddj");
    }

    /// Below the first threshold there is no rank at all — the ladder starts at
    /// 45, so 44 points is not rank 1 by rounding down.
    #[test]
    fn points_below_the_first_threshold_have_no_rank() {
        let table = BattleRankTable::parse(RANK_ROWS);
        assert!(table.for_points(44).is_none());
        assert_eq!(table.for_points(45).unwrap().rank, 1);
        assert_eq!(table.for_points(449).unwrap().rank, 5);
        assert_eq!(table.for_points(1_000).unwrap().rank, 6);
    }
}
