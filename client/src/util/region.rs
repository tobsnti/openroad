//! The client's side of the region grid: the display-unit coordinates its
//! readouts use, and reading a region id or an SRO-space position as a
//! sector. The grid itself — region ids, the region edge and where those
//! numbers come from — is `packets::region`, which the server shares; its
//! [`region_id`] and [`REGION_SIZE`] are re-exported here. Everything that
//! converts between a region id, an SRO-space position and an on-screen
//! coordinate goes through this module, so there is one place to check a
//! value against its source.
//!
//! Where the display numbers come from:
//!
//! * **The du scale is ours.** 192 du is the `worldmap_mapinfo.txt` axis, read
//!   from the player's own data.
//! * **[`COORD_ORIGIN_SECTOR`] has one named source**, not a measurement:
//!   Silkroad-Developer-Community/OpenSilkroadMap, `map/src/coord.ts` (MIT,
//!   portable). Mind its axis convention — its map-space Y sits one sector
//!   below the game's Z sector (it composes ids as
//!   `((floor(secY) + 1) << 8) | floor(secX)`), so its published origins 91
//!   and 127 read 92 and 128 in sector terms. Running its arithmetic against
//!   ours over the 37,085 movement frames of `packets::region` agrees to
//!   3.7e-12 du on 37,071 of them; the other 14 carry a region-local coordinate outside 0..1920
//!   (measured, one of them from the original client), where *their* function
//!   switches branch and ours does not — we take server-global units, so the
//!   ambiguity cannot arise here.

use bevy::prelude::Vec3;
pub use packets::region::{region_id, REGION_SIZE};

/// World units per *display unit* — the client's coordinate readouts and the
/// `worldmap_mapinfo.txt` offsets count in tenths of a world unit, which puts
/// one region at [`REGION_SIZE`] / 10 = 192 du.
pub const WORLD_UNITS_PER_DU: f32 = 10.0;

/// Sector the displayed X/Y readout counts from. The one number here that no
/// file in the player's data carries — it rests on `coord.ts` alone (see the
/// module doc). With it a captured Jangan position (`0x61A8`, region-local
/// 867/925) reads 6422.7 / 1052.5, the magnitude the original shows there.
pub const COORD_ORIGIN_SECTOR: (f32, f32) = (135.0, 92.0);

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

/// A region id (`u16`, or the `i16` a dungeon id reads as) or an SRO-space
/// position, read as a sector; ids through `packets::region`.
pub trait RegionIdExt {
    fn to_x_z(&self) -> (u8, u8);
    fn is_dungeon(&self) -> bool;
}

macro_rules! impl_region_id_ext {
    ($ty:ty) => {
        impl RegionIdExt for $ty {
            fn to_x_z(&self) -> (u8, u8) {
                packets::region::sector(*self as u16)
            }

            fn is_dungeon(&self) -> bool {
                packets::region::is_dungeon(*self as u16)
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
        displayed_coords, RegionIdExt, COORD_ORIGIN_SECTOR, REGION_SIZE, WORLD_UNITS_PER_DU,
    };

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

    /// Donwhang cave, 0x8001, as the `i16` it reads as: the same sector and
    /// flag as its `u16` (`packets::region`).
    #[test]
    fn region_id_to_x_z() {
        assert_eq!((-32767i16).to_x_z(), (1, 0));
        assert!((-32767i16).is_dungeon());
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
