//! The region grid: region ids, the sector size, and the display-unit
//! coordinates the client's readouts use. Everything that converts between a
//! region id, an SRO-space position and an on-screen coordinate goes through
//! this module, so there is one place to check a value against its source.
//!
//! A 16-bit region id packs the overworld sector as X = bits 0-7,
//! Z = bits 8-14 — and bit 15 is the *dungeon flag*, not part of Z
//! (go-sro/RSBot `RID` convention; dungeon ids therefore read as negative
//! i16, cf. `zonenames.rs`). For a dungeon id the low byte is the
//! `dungeoninfo.txt` dungeon index and there is no sector grid at all, so
//! overworld math (tiling, minimap, environment) must gate on `is_dungeon()`
//! before consuming `to_x_z()`.
//!
//! Where the numbers come from, and how strong each one is:
//!
//! * **The grid step and the du scale are ours.** 1920 is the `.m`/`.o2`/`.nvm`
//!   file grid step and 192 du is the `worldmap_mapinfo.txt` axis, both read
//!   from the player's own data.
//! * **The packing is measured.** 37,085 independent movement frames
//!   (`0x7021` + `0xB021`, keyed by timestamp and body over 18 `packet_dump/`
//!   trees) carry 32 distinct region ids, sectors x 164-170 / z 92-103, and
//!   every one round-trips through `to_x_z()`/[`region_id`]. Only 137 of them
//!   were spoken by the *original* client (`proxy*/c2s`, 8 distinct ids) — the
//!   rest came from our own peer or from `skrillax`, which proves our
//!   arithmetic but not vanilla's (`tools/src/bin/dump_census.rs`).
//! * **[`COORD_ORIGIN_SECTOR`] has one named source**, not a measurement:
//!   Silkroad-Developer-Community/OpenSilkroadMap, `map/src/coord.ts` (MIT,
//!   portable). Mind its axis convention — its map-space Y sits one sector
//!   below the game's Z sector (it composes ids as
//!   `((floor(secY) + 1) << 8) | floor(secX)`), so its published origins 91
//!   and 127 read 92 and 128 in sector terms. Running its arithmetic against
//!   ours over those same 37,085 frames agrees to 3.7e-12 du on 37,071 of
//!   them; the other 14 carry a region-local coordinate outside 0..1920
//!   (measured, one of them from the original client), where *their* function
//!   switches branch and ours does not — we take server-global units, so the
//!   ambiguity cannot arise here.
//! * **The dungeon branch is unmeasured.** No captured frame carries the flag:
//!   every one of the 4,109 `0x7021` bodies is 4 or 9 bytes, and a dungeon
//!   destination would be 15 (its coordinates widen to `i32`). So the branch
//!   rests on the player's data (`textzonename.txt` negative ids,
//!   `worldmap_mapinfo.txt` `#section Dungeonmap`), not on the wire.

use bevy::prelude::Vec3;

/// World-space size of one region/sector edge. Also the `.m`/`.o2`/`.nvm`
/// grid step, so terrain streaming and the region grid cannot drift apart.
pub const REGION_SIZE: f32 = 1920.0;

/// World units per *display unit* — the client's coordinate readouts and the
/// `worldmap_mapinfo.txt` offsets count in tenths of a world unit, which puts
/// one region at [`REGION_SIZE`] / 10 = 192 du.
pub const WORLD_UNITS_PER_DU: f32 = 10.0;

/// Sector the displayed X/Y readout counts from. The one number here that no
/// file in the player's data carries — it rests on `coord.ts` alone (see the
/// module doc). With it a captured Jangan position (`0x61A8`, region-local
/// 867/925) reads 6422.7 / 1052.5, the magnitude the original shows there.
pub const COORD_ORIGIN_SECTOR: (f32, f32) = (135.0, 92.0);

/// Compose a region id from its sector pair — the inverse of
/// [`RegionIdExt::to_x_z`] on an overworld id. Z is masked to 7 bits so a
/// caller cannot accidentally set the dungeon flag from a sector number.
///
/// That mask is why `zonesound::pack_region_id` is the one place that still
/// shifts by hand: `regioninfo.txt` uses sector Z=128 to *mean* the dungeon
/// flag, so masking there would lose every dungeon zone.
pub fn region_id(x: u8, z: u8) -> u16 {
    ((z as u16 & 0x7F) << 8) | x as u16
}

/// Server-global world units to display units (the axis `worldmap_mapinfo.txt`
/// and the coordinate readouts share).
pub fn world_to_du(gx: f32, gz: f32) -> (f32, f32) {
    (gx / WORLD_UNITS_PER_DU, gz / WORLD_UNITS_PER_DU)
}

/// The overworld X/Y readout for a server-global position (the minimap's
/// mirrored-X convention, world units): display units counted from
/// [`COORD_ORIGIN_SECTOR`]. Dungeon-local positions have no overworld origin
/// to count from and display their raw [`world_to_du`] value instead.
pub fn displayed_coords(gx: f32, gz: f32) -> (f32, f32) {
    let (du_x, du_z) = world_to_du(gx, gz);
    let du_per_region = REGION_SIZE / WORLD_UNITS_PER_DU;
    (
        du_x - COORD_ORIGIN_SECTOR.0 * du_per_region,
        du_z - COORD_ORIGIN_SECTOR.1 * du_per_region,
    )
}

pub trait RegionIdExt {
    fn to_x_z(&self) -> (u8, u8);
    /// Bit 15 set = dungeon region (coordinates are dungeon-local, no
    /// sector tiling).
    fn is_dungeon(&self) -> bool;
}

macro_rules! impl_region_id_ext {
    ($ty:ty) => {
        impl RegionIdExt for $ty {
            fn to_x_z(&self) -> (u8, u8) {
                let id = *self as u16;
                let x: u8 = (id & 0xFF) as u8;
                // 7 bits — bit 15 is the dungeon flag, never part of Z.
                let z: u8 = (id >> 8 & 0x7F) as u8;
                (x, z)
            }

            fn is_dungeon(&self) -> bool {
                (*self as u16) & 0x8000 != 0
            }
        }
    };
}

impl_region_id_ext!(u16);
impl_region_id_ext!(i16);

impl RegionIdExt for Vec3 {
    fn to_x_z(&self) -> (u8, u8) {
        if self.x.is_sign_negative() {
            return ((-self.x / REGION_SIZE) as u8, (self.z / REGION_SIZE) as u8);
        }
        ((self.x / REGION_SIZE) as u8, (self.z / REGION_SIZE) as u8)
    }

    /// An SRO-space position is always overworld space; dungeon interiors
    /// live in their own local frame and never reach this conversion.
    fn is_dungeon(&self) -> bool {
        false
    }
}

#[allow(dead_code)]
pub trait RegionExt {
    fn region_offset(&self) -> Vec3;
}

impl RegionExt for Vec3 {
    fn region_offset(&self) -> Vec3 {
        Vec3::new(self.x % REGION_SIZE, 0.0, self.z % REGION_SIZE)
    }
}

#[cfg(test)]
mod test {
    use bevy::prelude::Vec3;
    use rand::Rng;

    use crate::util::region::{
        displayed_coords, region_id, RegionIdExt, COORD_ORIGIN_SECTOR, REGION_SIZE,
        WORLD_UNITS_PER_DU,
    };

    /// Four of the 32 region ids the capture corpus actually carries (module
    /// doc), with their independent-frame counts: the busiest, the one with
    /// the most original-client frames, and the two corners of the span.
    #[test]
    fn captured_region_ids_decode_to_their_measured_sectors() {
        for (id, sector) in [
            (0x60A8u16, (168u8, 96u8)), // 20,037 frames, 52 from the original client
            (0x5EA9, (169, 94)),        // 4,282 frames, 23 from the original client
            (0x5EA4, (164, 94)),        // 1 — westernmost sector seen
            (0x67A9, (169, 103)),       // 14 — northernmost sector seen
        ] {
            assert_eq!(id.to_x_z(), sector, "{id:#06x}");
            assert!(!id.is_dungeon(), "{id:#06x}");
        }
    }

    /// The composition is the exact inverse of the decode across the whole
    /// overworld id space — the property the captures exercise 37,085 times.
    #[test]
    fn region_id_round_trips_every_overworld_id() {
        for id in 0u16..=0x7FFF {
            let (x, z) = id.to_x_z();
            assert_eq!(region_id(x, z), id, "{id:#06x}");
        }
        // A sector number can never smuggle the dungeon flag back in.
        assert_eq!(region_id(0xFF, 0xFF), 0x7FFF);
    }

    /// Readout scale and origin: a sector corner reads a whole number of
    /// regions' worth of display units away from `COORD_ORIGIN_SECTOR`, and a
    /// position inside the sector stays inside that sector's band.
    #[test]
    fn displayed_coords_count_from_the_origin_sector() {
        let (rx, rz) = 25000u16.to_x_z();
        let corner = displayed_coords(rx as f32 * REGION_SIZE, rz as f32 * REGION_SIZE);
        let du_per_region = REGION_SIZE / WORLD_UNITS_PER_DU;
        assert_eq!(
            corner,
            (
                (rx as f32 - COORD_ORIGIN_SECTOR.0) * du_per_region,
                (rz as f32 - COORD_ORIGIN_SECTOR.1) * du_per_region,
            )
        );
        assert_eq!(corner, (6336.0, 960.0));
        // The captured Jangan sample (`0xB021` 2026-08-11T09:43:44.635Z,
        // region-local 867/925) reads what `coord.ts` computes for it. The
        // tolerance is f32 rounding on the division, not slack in the formula.
        let (x, z) = displayed_coords(
            rx as f32 * REGION_SIZE + 867.0,
            rz as f32 * REGION_SIZE + 925.0,
        );
        assert!((x - 6422.7).abs() < 0.01, "{x}");
        assert_eq!(z, 1052.5);
    }

    #[test]
    fn region_id_to_x_z() {
        // Jangan: 25000 = 0x61A8 = sector 168x97.
        assert_eq!(25000u16.to_x_z(), (168, 97));
        assert!(!25000u16.is_dungeon());
        // Donwhang cave: 0x8001 (= -32767 as i16). The dungeon flag must not
        // fold into Z (the old decode returned z = 128 here).
        assert_eq!(0x8001u16.to_x_z(), (1, 0));
        assert!(0x8001u16.is_dungeon());
        assert_eq!((-32767i16).to_x_z(), (1, 0));
        assert!((-32767i16).is_dungeon());
        // jinsi_floor02 via gm_event collision id: 0x8006.
        assert_eq!(0x8006u16.to_x_z(), (6, 0));
    }

    #[test]
    fn vec3_to_x_z() {
        let x: u8 = rand::rng().random_range(0..255);
        let z: u8 = rand::rng().random_range(0..128);

        let v = Vec3::new(x as f32 * 1920.0, 0.0, z as f32 * 1920.0);

        let (actual_x, actual_z) = v.to_x_z();
        assert_eq!(actual_x, x);
        assert_eq!(actual_z, z);
    }

    #[test]
    fn vec3_to_x_z_with_fraction() {
        let x: u8 = rand::rng().random_range(0..255);
        let z: u8 = rand::rng().random_range(0..128);

        let mut v = Vec3::new(x as f32 * 1920.0, 0.0, z as f32 * 1920.0);
        v.x += rand::rng().random_range(0.0..1920.0);
        v.z += rand::rng().random_range(0.0..1920.0);

        let (actual_x, actual_z) = v.to_x_z();
        assert_eq!(actual_x, x);
        assert_eq!(actual_z, z);
    }
}
