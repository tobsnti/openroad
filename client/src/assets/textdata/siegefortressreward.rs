//! `siegefortressreward.txt` — where the holding guild's crest is drawn on the
//! world map, and which city a fortress hangs off.
//!
//! Idea: this is the one table in the siege family that **names its own
//! columns**, twice. A Korean line explains the groups (fortress codename, the
//! town-entry message that shows the occupying guild's name "see regioncode",
//! and "coordinates for drawing the guild crest on the world map"), and the
//! line below it gives the names outright:
//!
//! ```text
//! // Service  FortressCodeName  DependentCityRegionCodeName  RGx  RGy  PosX  PosY
//! 1  FORTRESS_HOTAN      RN_OA_OASISKINGDOM  135  96  6   28
//! 1  FORTRESS_BIJEOKDAN  xxx                 163  91  9   17
//! 1  FORTRESS_JANGAN     RN_CH_JANGAN        168  91  3   15
//! ```
//!
//! `RGx`/`RGy` are main-world region cells in the same convention the rest of
//! our map chain uses ([`crate::util::region::region_id`]), and they agree with the world
//! map's own table: `worldmap_mapinfo.txt` gives the Jangan city map the region
//! rect x 166..=170, y 96..=99, and the Jangan crest sits at x 168 — inside
//! that x range and seven cells south of the city, which is where the fortress
//! is. The same holds for Hotan (city x 134..=136, crest x 135).
//!
//! The fortress *maps* are a different world: rows 1001..1003 of
//! `worldmap_mapinfo.txt` carry region rects around x 67..=81, y 57..=71 and a
//! `INS_FORT_*` id. So this table's coordinate is deliberately the **main
//! world** one — an icon on the overview map, not a position inside the
//! instance.
//!
//! `PosX`/`PosY` are the in-region offset. The unit is [S]: the same file
//! family measures in-region coordinates in display units (`worldmap_mapinfo`
//! row 0 spans a region 0..192 in its `좌표 LT/RB` columns, and our
//! [`crate::assets::textdata::worldmap`] reader uses the same 192-per-region
//! scale), so du is the consistent reading — but nothing in the file says so,
//! and 3..28 du puts all three crests near a cell corner. Carried verbatim.
//!
//! **Version note.** Read by index with a minimum width: a client version that
//! appends columns is fine, and both comment lines are skipped by prefix rather
//! than by line number.

use std::collections::BTreeMap;

use crate::util::region;

/// One row of `siegefortressreward.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FortressReward {
    /// `FORTRESS_JANGAN` — the same token `siegefortress.txt` column 3 carries.
    pub codename: String,
    /// `RN_CH_JANGAN`, or `xxx` where the file leaves it unset (Bijeokdan).
    /// The Korean header ties it to the town-entry message that announces the
    /// occupying guild.
    pub city_region_key: Option<String>,
    /// `RGx`: main-world region cell, x.
    pub region_x: u8,
    /// `RGy`: main-world region cell, y.
    pub region_y: u8,
    /// `PosX`, in-region offset. Unit [S], see the module note.
    pub pos_x: u32,
    /// `PosY`, same.
    pub pos_y: u32,
}

impl FortressReward {
    /// The region id the rest of the map chain uses. `RGx`/`RGy` are
    /// main-world cells, so this is the overworld composition.
    pub fn region_id(&self) -> u16 {
        region::region_id(self.region_x, self.region_y)
    }
}

/// Keyed by fortress codename, which is how `siegefortress.txt` names its rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FortressRewardTable(pub BTreeMap<String, FortressReward>);

impl FortressRewardTable {
    pub fn parse(content: &str) -> Self {
        FortressRewardTable(
            content
                .lines()
                .filter_map(parse_row)
                .map(|r| (r.codename.clone(), r))
                .collect(),
        )
    }

    pub fn get(&self, codename: &str) -> Option<&FortressReward> {
        self.0.get(codename)
    }
}

fn parse_row(line: &str) -> Option<FortressReward> {
    let line = line.trim_start_matches('\u{feff}');
    if line.starts_with("//") {
        return None;
    }
    let c: Vec<&str> = line.trim_end().split('\t').collect();
    if c.len() < 7 || c[0].trim() != "1" {
        return None;
    }
    let city = c[2].trim();
    Some(FortressReward {
        codename: c[1].trim().to_string(),
        // `xxx` is this corpus's written "no value"; it is not a key.
        city_region_key: (city != "xxx" && !city.is_empty()).then(|| city.to_string()),
        region_x: c[3].trim().parse().ok()?,
        region_y: c[4].trim().parse().ok()?,
        pos_x: c[5].trim().parse().ok()?,
        pos_y: c[6].trim().parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file verbatim, both comment lines included — the second one is the
    /// column naming this module leans on.
    const ROWS: &str = "\u{feff}//\tfortress codename (Korean)\n\
         // Service\tFortressCodeName\tDependentCityRegionCodeName\tRGx\tRGy\tPosX\tPosY\n\
         1\tFORTRESS_HOTAN\tRN_OA_OASISKINGDOM\t135\t96\t6\t28\n\
         1\tFORTRESS_BIJEOKDAN\txxx\t163\t91\t9\t17\n\
         1\tFORTRESS_JANGAN\tRN_CH_JANGAN\t168\t91\t3\t15";

    #[test]
    fn the_three_rows_parse_and_both_headers_are_skipped() {
        let table = FortressRewardTable::parse(ROWS);
        assert_eq!(table.0.len(), 3);
        let jangan = table.get("FORTRESS_JANGAN").expect("Jangan");
        assert_eq!(jangan.city_region_key.as_deref(), Some("RN_CH_JANGAN"));
        assert_eq!((jangan.region_x, jangan.region_y), (168, 91));
        assert_eq!((jangan.pos_x, jangan.pos_y), (3, 15));
    }

    /// `xxx` is the corpus's written "unset" and must not become a key — a
    /// lookup on it would miss in every string table.
    #[test]
    fn an_unset_city_key_is_none_not_the_literal_xxx() {
        let table = FortressRewardTable::parse(ROWS);
        assert_eq!(
            table.get("FORTRESS_BIJEOKDAN").unwrap().city_region_key,
            None
        );
    }

    /// The region id is the one the rest of the map chain uses, and it agrees
    /// with the world map's own table: `worldmap_mapinfo.txt` gives the Jangan
    /// city map the region rect x 166..=170, y 96..=99, and this crest sits at
    /// x 168 — inside that x range, south of the city. Hotan: city x 134..=136,
    /// crest x 135.
    #[test]
    fn the_crest_region_agrees_with_the_world_maps_own_city_rects() {
        let table = FortressRewardTable::parse(ROWS);
        let jangan = table.get("FORTRESS_JANGAN").unwrap();
        assert_eq!(jangan.region_id(), region::region_id(168, 91));
        assert!((166..=170).contains(&jangan.region_x));
        assert!(jangan.region_y < 96, "the fortress is south of the city");

        let hotan = table.get("FORTRESS_HOTAN").unwrap();
        assert!((134..=136).contains(&hotan.region_x));
    }

    /// A short row and a row from another service are skipped rather than
    /// half-parsed — the next client version is the likely source of both.
    #[test]
    fn short_and_foreign_rows_are_skipped() {
        let mixed = format!("{ROWS}\n0\tFORTRESS_X\txxx\t1\t1\t1\t1\n1\tFORTRESS_SHORT\txxx\t1");
        assert_eq!(FortressRewardTable::parse(&mixed).0.len(), 3);
    }
}
