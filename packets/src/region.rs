//! The region grid: region ids and the region edge, shared by the client and
//! the server so both cut the world the same way.
//!
//! A 16-bit region id packs the overworld sector as X = bits 0-7,
//! Z = bits 8-14 — and bit 15 is the *dungeon flag*, not part of Z
//! (go-sro/RSBot `RID` convention; dungeon ids therefore read as negative
//! i16, cf. the client's `zonenames.rs`). For a dungeon id the low byte is the
//! `dungeoninfo.txt` dungeon index and there is no sector grid at all, so
//! overworld math (tiling, minimap, environment) must gate on [`is_dungeon`]
//! before consuming [`sector`].
//!
//! Where the numbers come from, and how strong each one is:
//!
//! * **The grid step is ours.** 1920 is the `.m`/`.o2`/`.nvm` file grid step,
//!   read from the player's own data.
//! * **The packing is measured.** 37,085 independent movement frames
//!   (`0x7021` + `0xB021`, keyed by timestamp and body over 18 `packet_dump/`
//!   trees) carry 32 distinct region ids, sectors x 164-170 / z 92-103, and
//!   every one round-trips through [`sector`]/[`region_id`]. Only 137 of them
//!   were spoken by the *original* client (`proxy*/c2s`, 8 distinct ids) — the
//!   rest came from our own peer or from `skrillax`, which proves our
//!   arithmetic but not vanilla's (`tools/src/bin/dump_census.rs`).
//! * **The dungeon branch is unmeasured.** No captured frame carries the flag:
//!   every one of the 4,109 `0x7021` bodies is 4 or 9 bytes, and a dungeon
//!   destination would be 15 (its coordinates widen to `i32`). So the branch
//!   rests on the player's data (`textzonename.txt` negative ids,
//!   `worldmap_mapinfo.txt` `#section Dungeonmap`), not on the wire.

/// World-space size of one region/sector edge. Also the `.m`/`.o2`/`.nvm`
/// grid step, so terrain streaming and the region grid cannot drift apart.
pub const REGION_SIZE: f32 = 1920.0;

/// Compose a region id from its sector pair — the inverse of [`sector`] on an
/// overworld id. Z is masked to 7 bits so a caller cannot accidentally set the
/// dungeon flag from a sector number.
///
/// That mask is why the client's `zonesound::pack_region_id` is the one place
/// that still shifts by hand: `regioninfo.txt` uses sector Z=128 to *mean* the
/// dungeon flag, so masking there would lose every dungeon zone.
pub fn region_id(x: u8, z: u8) -> u16 {
    ((z as u16 & 0x7F) << 8) | x as u16
}

/// The overworld sector `(x, z)` of a region id: 7 bits of Z, since bit 15 is
/// the dungeon flag, never part of Z.
pub fn sector(region: u16) -> (u8, u8) {
    ((region & 0xFF) as u8, (region >> 8 & 0x7F) as u8)
}

/// Whether a region id denotes a dungeon: dungeon-local coordinates, 4 bytes
/// each on the wire instead of the overworld's 2, and no sector tiling. xBot's
/// two helpers disagree at 0x7FFF and are not evidence.
pub fn is_dungeon(region: u16) -> bool {
    region & 0x8000 != 0
}

#[cfg(test)]
mod test {
    use super::{is_dungeon, region_id, sector};

    /// Four of the 32 region ids the capture corpus actually carries (module
    /// doc), with their independent-frame counts: the busiest, the one with
    /// the most original-client frames, and the two corners of the span.
    #[test]
    fn captured_region_ids_decode_to_their_measured_sectors() {
        for (id, measured) in [
            (0x60A8u16, (168u8, 96u8)), // 20,037 frames, 52 from the original client
            (0x5EA9, (169, 94)),        // 4,282 frames, 23 from the original client
            (0x5EA4, (164, 94)),        // 1 — westernmost sector seen
            (0x67A9, (169, 103)),       // 14 — northernmost sector seen
        ] {
            assert_eq!(sector(id), measured, "{id:#06x}");
            assert!(!is_dungeon(id), "{id:#06x}");
        }
    }

    /// The composition is the exact inverse of the decode across the whole
    /// overworld id space — the property the captures exercise 37,085 times.
    #[test]
    fn region_id_round_trips_every_overworld_id() {
        for id in 0u16..=0x7FFF {
            let (x, z) = sector(id);
            assert_eq!(region_id(x, z), id, "{id:#06x}");
        }
        // A sector number can never smuggle the dungeon flag back in.
        assert_eq!(region_id(0xFF, 0xFF), 0x7FFF);
    }

    #[test]
    fn region_id_to_x_z() {
        // Jangan: 25000 = 0x61A8 = sector 168x97.
        assert_eq!(sector(25000), (168, 97));
        assert!(!is_dungeon(25000));
        // Donwhang cave: 0x8001 (= -32767 as i16). The dungeon flag must not
        // fold into Z (the old decode returned z = 128 here).
        assert_eq!(sector(0x8001), (1, 0));
        assert!(is_dungeon(0x8001));
        // jinsi_floor02 via gm_event collision id: 0x8006.
        assert_eq!(sector(0x8006), (6, 0));
    }
}
