//! `siegefortress.txt`: one row per fortress, keyed by the id `0x385F`
//! carries. The client names the fortresses of `0x385F` with it; the server
//! needs its codenames, because the original client looks an NPC's
//! `CharacterDataRow::fortress` up here.
//!
//! Row 1 is `1 \t 1 \t FORTRESS_JANGAN \t … \t SN_FORTRESS_JANGAN \t …`, and
//! `SN_FORTRESS_JANGAN` resolves in `textdata_object.txt:5232` to **"Jangan
//! Fortress"**. The ids the table carries are 1, 3 and 6 — the same three the
//! capture shows in all 117 bodies of `packet_dump/0x385f.log`.
//!
//! The file ships no header, so its six numeric middle columns are carried
//! verbatim; the sibling `siegefortressreward.txt` is the only table in this
//! family that names its own columns, and it names different ones.

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Transcribed from the user's `textdata/siegefortress.txt`, tabs and all.
    /// The Korean column is replaced by `?` the way the file itself stores it
    /// in this drop.
    const FORTRESS_ROWS: &str = "\u{feff}1\t1\tFORTRESS_JANGAN\t????\tSN_FORTRESS_JANGAN\tGATE_CH\t0\t300\t30\t25\t63\t1000000\tetc\\fort_jangan.ddj\tNPC_CH_FORTRESS_OFFICIAL\t\n\
         1\t3\tFORTRESS_HOTAN\t????\tSN_FORTRESS_HOTAN\tGATE_KT\t0\t300\t30\t25\t63\t1000000\tetc\\fort_hotan.ddj\tNPC_KT_FORTRESS_OFFICIAL\t\n\
         1\t6\tFORTRESS_BIJEOKDAN\t?????\tSN_FORTRESS_BIJEOKDAN\tGATE_CH\t1\t200\t15\t20\t96\t500000\tetc\\fort_bijeokdan.ddj\tNPC_KT_FORTRESS_OFFICIAL\t";

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
