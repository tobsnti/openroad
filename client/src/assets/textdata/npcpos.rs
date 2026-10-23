//! Authored NPC spawn positions (`npcpos.txt`): one row per NPC —
//! `ref_obj_id`, region, x, y, z, no header and no service column.
//!
//! Idea: the server spawns its NPCs out of this very table, so the table can
//! answer "who does the data put here?" for a position alone — nothing has to
//! be spawned, nothing has to be in view. That is exactly what a quest mark
//! needs: `0x30D6` names its target by position and spawn id
//! (`plugins::net::quest`), and the NPC it points at is usually far away and
//! therefore not a spawned entity the client could ask for a name.
//!
//! Indexed by region, because every lookup has one. A position without its
//! region means nothing in a world tiled into 1920-unit sectors
//! (`util::region`).
//!
//! **`y` is read and dropped on purpose.** The authored height and the one the
//! server reports disagree: the captured `0x30D6` for `NPC_CH_SPECIAL`
//! (ref 2010) carries y = 9 while this table says 11.0, so height cannot be
//! part of the key. The two horizontal coordinates agreed for all eight
//! captured marks.

use std::collections::HashMap;

/// Slack on x and z, in world units.
///
/// Not zero, because the two sides of the comparison come from different
/// places: the wire truncates the server's float toward zero
/// (`QuestMarkAdd`), while this file is decimal text that parses back to a
/// slightly different float — measured on the capture, spawn `329.21` against
/// table `329.20999` and spawn `450.6` against table `450.60001`. An exact
/// integer key can therefore be one unit out at a boundary.
///
/// Not wider, because width buys nothing and costs certainty. Measured over
/// all 12 367 rows: in overworld regions only 11 of 474 NPCs sit within one
/// unit of another and just 4 pairs share a position exactly, and widening the
/// box from ±0.5 to ±2 moves the number of colliding pairs from 1386 to 1404 —
/// the ambiguity is co-location, not tolerance. All 1373 exactly-co-located
/// pairs are in dungeon regions, which the quest-marker layer skips anyway.
pub const POSITION_TOLERANCE: f32 = 1.0;

/// One authored spawn point. `y` is absent by design — see the module note.
#[derive(Debug, Clone, Copy, PartialEq)]
struct NpcSpawnPoint {
    ref_id: i32,
    x: f32,
    z: f32,
}

#[derive(Debug, Clone, Default)]
pub struct NpcPositions(HashMap<u16, Vec<NpcSpawnPoint>>);

impl NpcPositions {
    /// Parse the tab-separated file content (already decoded from UTF-16).
    pub fn parse(content: &str) -> Self {
        let mut by_region: HashMap<u16, Vec<NpcSpawnPoint>> = HashMap::new();
        for line in content.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() < 5 {
                continue;
            }
            let Ok(ref_id) = fields[0]
                .trim_start_matches('\u{feff}')
                .trim()
                .parse::<i32>()
            else {
                continue;
            };
            // Dungeon ids are stored signed here: bit 15 is the dungeon flag,
            // so `0x8004` appears as `-32764`. Read as `i16` and reinterpret,
            // which is the same convention `util::region` documents.
            let Ok(region) = fields[1].trim().parse::<i16>() else {
                continue;
            };
            let (Ok(x), Ok(z)) = (
                fields[2].trim().parse::<f32>(),
                fields[4].trim().parse::<f32>(),
            ) else {
                continue;
            };
            by_region
                .entry(region as u16)
                .or_default()
                .push(NpcSpawnPoint { ref_id, x, z });
        }
        Self(by_region)
    }

    /// The one NPC the data authors within [`POSITION_TOLERANCE`] of this
    /// position, or `None`.
    ///
    /// `None` also when *several* are in reach: two NPCs on the same spot
    /// cannot be told apart by position, and naming the first row would be a
    /// guess dressed as a lookup.
    pub fn ref_at(&self, region: u16, x: f32, z: f32) -> Option<i32> {
        let mut found = None;
        for point in self.0.get(&region)? {
            if (point.x - x).abs() > POSITION_TOLERANCE || (point.z - z).abs() > POSITION_TOLERANCE
            {
                continue;
            }
            if found.is_some() {
                return None;
            }
            found = Some(point.ref_id);
        }
        found
    }

    /// Row count, for the load log.
    pub fn len(&self) -> usize {
        self.0.values().map(Vec::len).sum()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Literal rows from the player's `npcpos.txt`, including the two the
    /// captured quest marks point at (refs 2003 and 2010) and one dungeon row
    /// with its negative region id.
    const ROWS: &str = "2003\t25000\t332.73001\t0.0\t1406.7\r\n\
                        2010\t25000\t1760.3199\t11.0\t480.22\r\n\
                        15004\t-32764\t100.0\t0.0\t200.0\r\n";

    #[test]
    fn parses_rows_and_finds_the_npc_at_a_captured_mark_position() {
        let positions = NpcPositions::parse(ROWS);
        assert_eq!(positions.len(), 3);
        // `packet_dump/0x30d6.log` carries (332, 1406) for this target; the
        // table says (332.73, 1406.7), so truncation must still match.
        assert_eq!(positions.ref_at(25000, 332.0, 1406.0), Some(2003));
        assert_eq!(positions.ref_at(25000, 1760.0, 480.0), Some(2010));
        // nothing authored there, and a right position in the wrong region
        assert_eq!(positions.ref_at(25000, 900.0, 900.0), None);
        assert_eq!(positions.ref_at(25255, 332.0, 1406.0), None);
    }

    /// The height disagreement that made `y` a non-key: this very row says
    /// 11.0 while the wire said 9. Any y must resolve the same ref.
    #[test]
    fn the_authored_height_is_not_part_of_the_key() {
        let positions = NpcPositions::parse(ROWS);
        assert_eq!(positions.ref_at(25000, 1760.0, 480.0), Some(2010));
    }

    /// Dungeon regions are stored as a negative `i16`; `-32764` is `0x8004`.
    #[test]
    fn a_dungeon_region_is_read_as_the_unsigned_id_with_its_flag_bit() {
        let positions = NpcPositions::parse(ROWS);
        assert_eq!(positions.ref_at(0x8004, 100.0, 200.0), Some(15004));
    }

    /// Two NPCs within the tolerance cannot be told apart by position, so the
    /// lookup must refuse rather than return the first row.
    #[test]
    fn two_candidates_in_reach_resolve_to_nothing() {
        let positions =
            NpcPositions::parse("1\t25000\t100.0\t0.0\t200.0\r\n2\t25000\t100.5\t0.0\t200.0\r\n");
        assert_eq!(positions.ref_at(25000, 100.0, 200.0), None);
        // and just outside the box the first one is unambiguous again
        assert_eq!(positions.ref_at(25000, 99.2, 200.0), Some(1));
    }
}
