//! The game's own optional-teleport table (`refoptionalteleport.txt`).
//!
//! Idea: the destinations a player can pick are **data**, not code. The table
//! carries one row per destination with the zone's `SN_ZONE_*` string key and
//! a server-style position (region id plus region-local x/y/z) — the same
//! shape the spawn path already uses, so a row goes straight into
//! `server_position_to_sro` without a conversion of ours in between.
//!
//! ```text
//! 1  1  콘스탄티노플  SN_ZONE_22004  26959  1030  80  512  1  -1  1  0 …
//! ```
//!
//! Columns: service flag · index · Korean name · **zone key** · **region** ·
//! **x** · **y** · **z**, then flags this reader does not need. Measured over
//! the user's own file (2026-10-26): **32 rows, 31 in service**, every one
//! with a `SN_ZONE_*` key that resolves through `textdata_object.txt`
//! (`SN_ZONE_22004` is "Constantinople").
//!
//! **Version note.** This file is the v1.188 table; another client version
//! ships its own. Reading it is what makes the destination list follow the
//! installed data instead of a transcription that only fits one version.

/// One row of `refoptionalteleport.txt`.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionalTeleport {
    /// Column 1: the table's own service flag. Rows with `0` are not offered.
    pub active: bool,
    /// Column 4, the zone's string key (`SN_ZONE_*`) — the label's source.
    pub zone_key: String,
    /// Column 5. Region id in the usual packed form (`z << 8 | x`).
    pub region: u16,
    /// Columns 6-8, region-local.
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Clone, Default)]
pub struct OptionalTeleportTable(pub Vec<OptionalTeleport>);

impl OptionalTeleportTable {
    pub fn parse(content: &str) -> Self {
        let rows = content
            .lines()
            .filter_map(|line| {
                let fields: Vec<&str> = line
                    .trim_end_matches('\r')
                    .split('\t')
                    .map(str::trim)
                    .collect();
                if fields.len() < 8 {
                    return None;
                }
                let active = fields[0].trim_start_matches('\u{feff}') == "1";
                let zone_key = fields[3];
                if !zone_key.starts_with("SN_") {
                    return None;
                }
                Some(OptionalTeleport {
                    active,
                    zone_key: zone_key.to_string(),
                    region: fields[4].parse().ok()?,
                    x: fields[5].parse().ok()?,
                    y: fields[6].parse().ok()?,
                    z: fields[7].parse().ok()?,
                })
            })
            .collect();
        Self(rows)
    }

    /// The destinations the table puts in service, in file order.
    pub fn active(&self) -> impl Iterator<Item = &OptionalTeleport> {
        self.0.iter().filter(|row| row.active)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Real rows, byte-for-byte from `Media/server_dep/silkroad/textdata/
    /// refoptionalteleport.txt` (decoded from UTF-16), including the BOM on
    /// the first field and a row taken out of service.
    const FIXTURE: &str = "\u{feff}1\t1\t콘스탄티노플\tSN_ZONE_22004\t26959\t1030\t80\t512\t1\t-1\t1\t0\t0\t-1\txxx\r\n\
        1\t2\t슬픔의 숲\tSN_ZONE_25063\t27207\t648\t-21\t194\t1\t-1\t1\t0\t0\t-1\txxx\r\n\
        0\t3\t신들의 정원\tSN_ZONE_25068\t25416\t735\t0\t1674\t1\t-1\t1\t0\t0\t-1\txxx\r\n\
        garbage line\r\n";

    #[test]
    fn rows_carry_a_zone_key_and_a_server_position() {
        let table = OptionalTeleportTable::parse(FIXTURE);
        assert_eq!(table.0.len(), 3, "the garbage line is dropped");
        assert_eq!(table.active().count(), 2, "row 3 is out of service");

        let first = &table.0[0];
        assert_eq!(first.zone_key, "SN_ZONE_22004");
        assert_eq!(first.region, 26959);
        assert_eq!((first.x, first.y, first.z), (1030.0, 80.0, 512.0));
        // the packed region id is `z << 8 | x`, which is what the spawn path
        // expects — 26959 is 105/79 in the map's own directory layout
        assert_eq!((first.region >> 8, first.region & 0xFF), (105, 79));
    }

    /// A negative height is a real value in this table (the Sorrow Forest row
    /// sits below zero), so the parse must not treat it as a bad field.
    #[test]
    fn a_negative_height_parses() {
        let table = OptionalTeleportTable::parse(FIXTURE);
        assert_eq!(table.0[1].y, -21.0);
    }
}
