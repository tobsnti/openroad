//! `npcpos.txt`: where the data puts its NPCs, one row each — `ref_obj_id`,
//! region, x, y, z — with no header and no service column. The caller hands
//! over the file already decoded from UTF-16.
//!
//! Dungeon ids are stored signed: bit 15 is the dungeon flag, so `0x8004`
//! appears as `-32764`. A row reads the region as `i16` and keeps the `u16`
//! id, the convention of `packets::region`.

/// One authored NPC spawn point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NpcPos {
    pub ref_id: i32,
    pub region: u16,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// The rows of the file; a line that does not read as one is skipped.
pub fn parse(content: &str) -> Vec<NpcPos> {
    content.lines().filter_map(row).collect()
}

fn row(line: &str) -> Option<NpcPos> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() < 5 {
        return None;
    }
    let ref_id = fields[0]
        .trim_start_matches('\u{feff}')
        .trim()
        .parse()
        .ok()?;
    let region = fields[1].trim().parse::<i16>().ok()? as u16;
    let [x, y, z] = [2, 3, 4].map(|i| fields[i].trim().parse::<f32>());
    Some(NpcPos {
        ref_id,
        region,
        x: x.ok()?,
        y: y.ok()?,
        z: z.ok()?,
    })
}

#[cfg(test)]
mod test {
    use super::*;

    /// Literal rows from the player's `npcpos.txt`, the first behind the
    /// file's byte-order mark, and one dungeon row with its negative region.
    #[test]
    fn reads_each_row_with_its_dungeon_region_as_the_unsigned_id() {
        let rows = "\u{feff}2003\t25000\t332.73001\t0.0\t1406.7\r\n\
                    2010\t25000\t1760.3199\t11.0\t480.22\r\n\
                    15004\t-32764\t100.0\t0.0\t200.0\r\n";
        let at = |ref_id, region, x, y, z| NpcPos {
            ref_id,
            region,
            x,
            y,
            z,
        };
        assert_eq!(
            parse(rows),
            [
                at(2003, 25000, 332.73001, 0.0, 1406.7),
                at(2010, 25000, 1760.3199, 11.0, 480.22),
                at(15004, 0x8004, 100.0, 0.0, 200.0),
            ]
        );
    }

    #[test]
    fn a_line_that_is_not_a_row_is_skipped() {
        assert_eq!(
            parse("2003\t25000\t332.7\t0.0\r\n\r\nx\t25000\t1\t2\t3\r\n"),
            []
        );
    }
}
