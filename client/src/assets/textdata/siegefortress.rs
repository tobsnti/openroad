//! The siege tables the client loads, and the two that answer a question the
//! wire could not: `siegefortress.txt` and `siegefortressbattlerank.txt`.
//!
//! Idea: `0x385F` carries fortress **ids** and never a fortress **name** — the
//! name is on disk, and so is the ladder behind the rank byte. Both chains are
//! followed here rather than guessed:
//!
//! * `siegefortress.txt` row 1 is `1 \t 1 \t FORTRESS_JANGAN \t … \t
//!   SN_FORTRESS_JANGAN \t …`, and `SN_FORTRESS_JANGAN` resolves in
//!   `textdata_object.txt:5232` to **"Jangan Fortress"**. The ids the table
//!   carries are 1, 3 and 6 — the same three the capture shows in all 117
//!   bodies of `packet_dump/0x385f.log`.
//! * `siegefortressbattlerank.txt` is six rows with a rising point threshold
//!   (45, 75, 135, 210, 300, 450) and an icon per rank.
//!
//! One column is **not** what its position suggests. Its fifth field reads
//! 21234..=21239, which looks like a text id and is not: those numbers resolve
//! in `characterdata_25000.txt` as `COS_GUARD_EU_BOW_01_116`..`_121`, guard
//! units at levels 116-121. It is a character id, so it is named one.
//!
//! Columns with no source stay `unk_*`. `siegefortress.txt` ships no header,
//! so its six numeric middle columns are carried verbatim; the sibling
//! `siegefortressreward.txt` is the only table in this family that names its
//! own columns, and it names different ones.
//!
//! **Version note.** Every statement above is measured against one drop
//! (`Media/server_dep/silkroad/textdata`). Row counts and column counts are
//! the obvious place where another client version differs, so the parser reads
//! *by index with a minimum width* and ignores trailing fields instead of
//! demanding an exact column count.

use std::collections::BTreeMap;

/// One row of `siegefortress.txt`. Column 2 is the id `0x385F` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fortress {
    pub id: u32,
    /// `FORTRESS_JANGAN`. The sibling `siegefortressreward.txt` keys its own
    /// rows on this token, and its header calls it the fortress codename.
    pub codename: String,
    /// `SN_FORTRESS_JANGAN` — resolves in `textdata_object.txt`.
    pub name_key: String,
    /// `GATE_CH` / `GATE_KT`.
    pub gate_codename: String,
    /// `etc\fort_jangan.ddj`.
    pub icon: String,
    /// `NPC_CH_FORTRESS_OFFICIAL`.
    pub official_codename: String,
    /// Columns 7..=12, in file order. No header names them and no other table
    /// references them, so they are carried rather than christened. In this
    /// drop they read `0 300 30 25 63 1000000` for Jangan and Hotan and
    /// `1 200 15 20 96 500000` for Bijeokdan.
    pub unk: [u32; 6],
}

/// Keyed by fortress id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FortressTable(pub BTreeMap<u32, Fortress>);

impl FortressTable {
    pub fn parse(content: &str) -> Self {
        FortressTable(
            content
                .lines()
                .filter_map(parse_fortress)
                .map(|f| (f.id, f))
                .collect(),
        )
    }

    pub fn get(&self, id: u32) -> Option<&Fortress> {
        self.0.get(&id)
    }
}

fn parse_fortress(line: &str) -> Option<Fortress> {
    let c: Vec<&str> = line.trim_end().split('\t').collect();
    if c.len() < 14 || c[0].trim_start_matches('\u{feff}') != "1" {
        return None;
    }
    let mut unk = [0u32; 6];
    for (slot, raw) in unk.iter_mut().zip(&c[6..12]) {
        *slot = raw.trim().parse().ok()?;
    }
    Some(Fortress {
        id: c[1].trim().parse().ok()?,
        codename: c[2].to_string(),
        name_key: c[4].to_string(),
        gate_codename: c[5].to_string(),
        icon: c[12].to_string(),
        official_codename: c[13].to_string(),
        unk,
    })
}

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

    /// Transcribed from the user's `textdata/siegefortress.txt`, tabs and all.
    /// The Korean column is replaced by `?` the way the file itself stores it
    /// in this drop.
    const FORTRESS_ROWS: &str = "\u{feff}1\t1\tFORTRESS_JANGAN\t????\tSN_FORTRESS_JANGAN\tGATE_CH\t0\t300\t30\t25\t63\t1000000\tetc\\fort_jangan.ddj\tNPC_CH_FORTRESS_OFFICIAL\t\n\
         1\t3\tFORTRESS_HOTAN\t????\tSN_FORTRESS_HOTAN\tGATE_KT\t0\t300\t30\t25\t63\t1000000\tetc\\fort_hotan.ddj\tNPC_KT_FORTRESS_OFFICIAL\t\n\
         1\t6\tFORTRESS_BIJEOKDAN\t?????\tSN_FORTRESS_BIJEOKDAN\tGATE_CH\t1\t200\t15\t20\t96\t500000\tetc\\fort_bijeokdan.ddj\tNPC_KT_FORTRESS_OFFICIAL\t";

    /// Transcribed from `textdata/siegefortressbattlerank.txt`.
    const RANK_ROWS: &str = "\u{feff}1\t1\t???\t45\t21234\tetc\\rank_assault_soilder.ddj\t\n\
         1\t2\t?????\t75\t21235\tetc\\rank_elite_assault_soilder.ddj\t\n\
         1\t3\t????\t135\t21236\tetc\\rank_centurion.ddj\t\n\
         1\t4\t????\t210\t21237\tetc\\rank_assault_leader.ddj\t\n\
         1\t5\t?????\t300\t21238\tetc\\rank_elite_guard.ddj\t\n\
         1\t6\t????\t450\t21239\tetc\\rank_combat_commander.ddj\t";

    /// The ids the table carries are the ids the wire carries: 1, 3 and 6 are
    /// what all 117 captured `0x385F` bodies hold.
    #[test]
    fn the_table_holds_the_three_ids_the_capture_shows() {
        let table = FortressTable::parse(FORTRESS_ROWS);
        assert_eq!(table.0.keys().copied().collect::<Vec<_>>(), vec![1, 3, 6]);
        let jangan = table.get(1).expect("id 1");
        assert_eq!(jangan.codename, "FORTRESS_JANGAN");
        assert_eq!(jangan.name_key, "SN_FORTRESS_JANGAN");
        assert_eq!(jangan.icon, "etc\\fort_jangan.ddj");
        assert_eq!(jangan.official_codename, "NPC_CH_FORTRESS_OFFICIAL");
    }

    /// The six unnamed columns are carried verbatim, not interpreted. Bijeokdan
    /// differs from the other two in every one of them, which is why guessing a
    /// meaning from a single row would have been easy and wrong.
    #[test]
    fn the_unnamed_columns_are_carried_not_interpreted() {
        let table = FortressTable::parse(FORTRESS_ROWS);
        assert_eq!(table.get(1).unwrap().unk, [0, 300, 30, 25, 63, 1_000_000]);
        assert_eq!(table.get(6).unwrap().unk, [1, 200, 15, 20, 96, 500_000]);
    }

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

    /// A row from another service, a short row and a comment are all skipped
    /// rather than parsed into nonsense — the next client version is the likely
    /// source of all three.
    #[test]
    fn foreign_short_and_comment_rows_are_skipped() {
        let mixed = format!(
            "//\tcomment\n0\t9\tDISABLED\t\t\t\t0\t0\t0\t0\t0\t0\t\t\n1\t7\tSHORT\n{FORTRESS_ROWS}"
        );
        let table = FortressTable::parse(&mixed);
        assert_eq!(table.0.keys().copied().collect::<Vec<_>>(), vec![1, 3, 6]);
    }
}
