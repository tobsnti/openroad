//! The underwater view: while the camera stands below a drawn water plane, the
//! image takes the water's colour and loses its sight distance.
//!
//! Idea: the camera carries no collision and a shore is walkable right down
//! into the lake bed, so it does go under — the `[U]` left standing by the
//! spawn-under-water fix. Until now that looked like a flat grey-green sheet
//! with a hard edge, because nothing told the renderer it was looking *through*
//! water. The cheapest honest answer is the atmosphere the frame already has:
//! [`apply_environment`](super::apply_environment) rewrites `DistanceFog` every
//! frame, so a system ordered after it can replace colour and falloff with no
//! stored state at all — surfacing heals itself on the next frame. The sky goes
//! with it, for the same reason the fog changes: from under water there is no
//! sky to see, and the clear colour (already the fog colour by convention)
//! carries the water instead.
//!
//! Neither number is invented. The tint is the **live ENVI water colour** of
//! this frame — the same `profile.water_color` that `apply_environment` tints
//! the water surface with, per region and per time of day, so the colour seen
//! from above is the colour one gets inside. The reach is the water's own
//! extinction length, [`ABSORPTION_FULL_DEPTH`] (64 units: the depth at which
//! the surface absorbs transmitted light down to the water texture's own
//! colour once), read as an exponential fog density of `1/64` per unit. Both
//! already belong to the look.
//!
//! Deliberate deviation (ADR 0009): the v1.188 client has no underwater view
//! at all, so `graphics.water.underwater_view = false` is the faithful image
//! and this is the stated improvement. Scope of the smallest form, named
//! rather than hidden: water only (not ice, whose colour this is not),
//! tint and distance only (no caustics, no surface seen from below, no
//! particles), and the fog reaches whatever the fog reaches — the sky is
//! hidden, not fogged.

use bevy::pbr::DistanceFog;
use bevy::prelude::*;

use crate::assets::m::{TerrainBlock, WaterType};
use crate::plugins::config::ClientConfig;
use crate::plugins::map::terrain::REGION_SIZE;
use crate::plugins::map::water_hq_material::ABSORPTION_FULL_DEPTH;
use crate::plugins::skybox::Skybox;

/// One terrain block: 16 tiles of 20 units, the same extent
/// `terrain::water_patch_mesh` builds its quad from.
const BLOCK_SIZE: f32 = REGION_SIZE / 6.0;

/// Tint the view and collapse its sight distance while the camera is under
/// water. Runs after [`apply_environment`](super::apply_environment), which
/// owns `DistanceFog` and restores it the moment this system stops writing.
/// `pub(super)`, not `pub`: the signature names the parent module's private
/// `EnvSmoothing`, and the only registration site is the plugin next door.
pub(super) fn apply_underwater_view(
    config: Res<ClientConfig>,
    smoothing: Res<super::EnvSmoothing>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    blocks: Query<(&GlobalTransform, &TerrainBlock)>,
    mut fog_query: Query<&mut DistanceFog>,
    mut clear_color: ResMut<ClearColor>,
    mut skybox: Query<&mut Visibility, With<Skybox>>,
) {
    let submerged = config.graphics.water.underwater_view
        && cameras
            .iter()
            .find(|(camera, _)| camera.is_active)
            .is_some_and(|(_, transform)| is_submerged(transform.translation(), blocks.iter()));

    // Outside the early return: the sky has to come back on surfacing even
    // though the fog does not need restoring (its writer runs every frame).
    for mut visibility in &mut skybox {
        visibility.set_if_neq(if submerged {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
    }
    if !submerged {
        return;
    }

    // No active environment profile yet means no water colour to borrow, and
    // inventing one is exactly what this slice avoids.
    let Some(sample) = smoothing.0 else {
        return;
    };
    let tint = Color::srgb(
        sample.water_color.x,
        sample.water_color.y,
        sample.water_color.z,
    );
    for mut fog in &mut fog_query {
        fog.color = tint;
        // Exponential, not the linear band the ENVI chain uses: absorption is
        // Beer-Lambert, and this density is the surface's own extinction.
        fog.falloff = FogFalloff::Exponential {
            density: 1.0 / ABSORPTION_FULL_DEPTH,
        };
    }
    // Normally hidden behind the skybox; with the sky off it is what the water
    // column reads as.
    clear_color.0 = tint;
}

/// Whether `camera` (render space) is below the drawn water plane of the block
/// it stands in.
///
/// The footprint test runs in block-local space on purpose. The terrain group's
/// transform mirrors X (regions sit at `x * -REGION_SIZE`, see
/// [`active_block`](super::active_block)), so comparing world coordinates here
/// would duplicate that convention; the inverse affine keeps it in the one
/// place that already owns it.
fn is_submerged<'a>(
    camera: Vec3,
    blocks: impl Iterator<Item = (&'a GlobalTransform, &'a TerrainBlock)>,
) -> bool {
    for (transform, block) in blocks {
        let WaterType::Water(_, level) = block.water_type else {
            continue;
        };
        let local = transform.affine().inverse().transform_point3(camera);
        if !(0.0..=BLOCK_SIZE).contains(&local.x) || !(0.0..=BLOCK_SIZE).contains(&local.z) {
            continue;
        }
        // A plane the mesh builder skipped is no surface to be under: 16% of
        // the corpus' water blocks have their whole terrain above it
        // (`TerrainBlock::terrain_is_above`).
        return local.y < level && !block.terrain_is_above(level);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::assets::m::MapVertex;

    /// A block whose terrain sits at `bed`, carrying a water plane at `level`.
    fn block(bed: f32, level: f32) -> TerrainBlock {
        let vertices = (0..17 * 17)
            .map(|i| MapVertex {
                x: (i % 17) as i32,
                z: (i / 17) as i32,
                height: bed,
                texture_id: 0,
                splat_scale: 0,
                splat_offset: 0,
                brightness: 0,
            })
            .collect();
        TerrainBlock {
            x: 0,
            z: 0,
            flag: 0,
            environment_id: 0,
            water_type: WaterType::Water(0, level),
            vertices,
            tiles: Vec::new(),
            aabb: bevy::camera::primitives::Aabb::from_min_max(Vec3::ZERO, Vec3::ONE),
        }
    }

    /// The real placement: a terrain group mirrors X, so a block's footprint
    /// runs from its world origin *backwards* in X. Getting that wrong is the
    /// one failure mode that leaves the tint on dry land next to the lake.
    fn mirrored(origin: Vec3) -> GlobalTransform {
        GlobalTransform::from(Transform {
            translation: origin,
            scale: Vec3::new(-1.0, 1.0, 1.0),
            ..default()
        })
    }

    #[test]
    fn a_camera_under_the_plane_of_its_own_block_is_submerged() {
        let placement = mirrored(Vec3::ZERO);
        let b = block(10.0, 50.0);
        let blocks = [(&placement, &b)];

        // Inside the mirrored footprint: world x runs 0..-320, z runs 0..320.
        assert!(is_submerged(
            Vec3::new(-160.0, 40.0, 160.0),
            blocks.iter().copied()
        ));
        assert!(
            !is_submerged(Vec3::new(-160.0, 60.0, 160.0), blocks.iter().copied()),
            "above the plane is not under water"
        );
        assert!(
            !is_submerged(Vec3::new(160.0, 40.0, 160.0), blocks.iter().copied()),
            "the unmirrored side of the origin is the neighbouring block"
        );
        assert!(
            !is_submerged(Vec3::new(-160.0, 40.0, -160.0), blocks.iter().copied()),
            "outside the footprint in z"
        );
    }

    /// The block carries water in the file but no plane is drawn, because its
    /// terrain is entirely above the level. Tinting there would mean tinting
    /// inside a hill.
    #[test]
    fn a_block_whose_plane_is_not_drawn_does_not_tint() {
        let placement = mirrored(Vec3::ZERO);
        let b = block(60.0, 50.0);
        let blocks = [(&placement, &b)];
        assert!(!is_submerged(
            Vec3::new(-160.0, 40.0, 160.0),
            blocks.iter().copied()
        ));
    }

    /// Ice is a water plane too, but the borrowed colour is the ENVI *water*
    /// colour and the extinction is the water surface's — neither is measured
    /// for a frozen sheet, so an ice block stays out of this.
    #[test]
    fn ice_is_left_alone() {
        let placement = mirrored(Vec3::ZERO);
        let mut b = block(10.0, 50.0);
        b.water_type = WaterType::Ice(50.0);
        let blocks = [(&placement, &b)];
        assert!(!is_submerged(
            Vec3::new(-160.0, 40.0, 160.0),
            blocks.iter().copied()
        ));
    }

    /// The fog density is the surface's own extinction length, not a number of
    /// its own: 64 units is where absorption has taken the transmitted light
    /// down to the water colour once, and 2% of the light is left at about
    /// four times that (`ln(50) * 64`).
    #[test]
    fn the_reach_is_the_waters_own_extinction() {
        let density = 1.0 / ABSORPTION_FULL_DEPTH;
        assert_eq!(density, 1.0 / 64.0);
        let two_percent = (1.0f32 / 0.02).ln() / density;
        assert!(
            (240.0..260.0).contains(&two_percent),
            "{two_percent} units to 2% visibility"
        );
    }
}
