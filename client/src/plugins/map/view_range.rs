//! The overworld's view, fog and cull distances as one derived resource.
//!
//! Every distance the overworld streams, fogs or culls by used to be the
//! compile-time `(VISIBLE_RANGE + FOG_RANGE) * REGION_SIZE`, recomputed at
//! each call site. They now come from `graphics.view` ([`ViewSettings`])
//! through [`ViewRange`], which [`apply_view_range`] derives under the
//! live-settings rule (`docs/settings-live-apply.md`). That way a weak
//! machine can stream and draw less and a strong one more, without a rebuild.
//!
//! There are two cull distances:
//! - `static_cull` is the configured ceiling. Spawn-time state such as each
//!   object part's `VisibilityRange` bakes it in, and [`apply_object_lod`]
//!   re-walks those parts when it changes.
//! - `live_cull` is what the per-frame culls (region roots, whole map
//!   objects, animations, effects) use. It equals `static_cull` unless
//!   `cull_follows_envi_fog` is on, in which case the environment system
//!   lowers it to the current ENVI fog end, so a short fogged night stops
//!   drawing what the fog already hides.

use bevy::camera::visibility::VisibilityRange;
use bevy::prelude::*;

use crate::plugins::config::graphics::{ObjectLodSettings, ViewSettings};
use crate::plugins::config::ClientConfig;
use crate::plugins::map::terrain::{REGION_SIZE, UNLOAD_BUFFER};

/// The derived view distances. Read it; only [`apply_view_range`] and the
/// environment's `live_cull` follow-up write it.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct ViewRange {
    /// The settings this was derived from, so a config edit elsewhere (a
    /// chat colour) does not re-derive and change-flag it.
    source: ViewSettings,
    /// How far terrain and objects are drawn at most.
    pub view_distance: f32,
    /// Regions streamed in around the camera's region, on each side.
    pub load_ring: i32,
    /// Fixed fog band, used when the environment profile's planes are off.
    pub fog_start: f32,
    /// Fully opaque fog, and the ceiling the profile fog may reach.
    pub fog_end: f32,
    /// Follow the environment profile's fog planes.
    pub envi_fog: bool,
    /// Multiplier on the profile-mapped fog distances.
    pub envi_fog_scale: f32,
    /// The configured cull ceiling — see the module docs.
    pub static_cull: f32,
    /// The cull distance in effect this frame — see the module docs.
    pub live_cull: f32,
    /// Whether `live_cull` follows the profile fog end.
    pub cull_follows_envi_fog: bool,
    /// The main cameras' far plane.
    pub far: f32,
    /// Where the half and the quarter terrain grid take over
    /// (`graphics.view.terrain_lod`); `None` = full grid everywhere.
    pub terrain_lod: Option<(f32, f32)>,
}

impl Default for ViewRange {
    fn default() -> Self {
        Self::from_settings(&ViewSettings::default())
    }
}

impl ViewRange {
    /// One region: anything less leaves the camera standing at the edge of
    /// what is drawn.
    pub const MIN_VIEW_DISTANCE: f32 = REGION_SIZE;
    /// Five regions, which streams a 13x13 block of regions. The far plane
    /// one region beyond it keeps the depth range within
    /// `camera::the_depth_range_stays_precise`.
    pub const MAX_VIEW_DISTANCE: f32 = 5.0 * REGION_SIZE;
    /// The nearest the fog may close in: the same floor the environment
    /// profiles map a -1 (darkest night) far plane to, so it stays playable.
    pub const MIN_FOG_END: f32 = 800.0;
    /// `live_cull` moves in steps of this, rounded up, so the smoothly
    /// animated profile fog does not change-flag the resource (and re-run
    /// the region visibility pass) every frame. An eighth of a region; past
    /// the fog end it is invisible either way.
    const LIVE_CULL_STEP: f32 = REGION_SIZE / 8.0;

    /// Derive and clamp. Out-of-range values warn once and fall back to the
    /// nearest valid value instead of producing a broken view.
    pub fn from_settings(settings: &ViewSettings) -> Self {
        let view_distance = valid(
            "view_distance",
            settings.view_distance,
            Self::MIN_VIEW_DISTANCE,
            Self::MAX_VIEW_DISTANCE,
        );
        let fog_end = valid(
            "fog_end",
            settings.fog_end,
            Self::MIN_FOG_END,
            view_distance,
        );
        let fog_start = valid("fog_start", settings.fog_start, 0.0, fog_end);
        let static_cull = if settings.fog_cull_distance > 0.0 {
            valid(
                "fog_cull_distance",
                settings.fog_cull_distance,
                Self::MIN_FOG_END,
                view_distance,
            )
        } else {
            fog_end
        };
        Self {
            source: settings.clone(),
            view_distance,
            load_ring: (view_distance / REGION_SIZE).ceil() as i32 + UNLOAD_BUFFER,
            fog_start,
            fog_end,
            envi_fog: settings.envi_fog,
            envi_fog_scale: if settings.envi_fog_scale.is_finite() {
                settings.envi_fog_scale.max(0.0)
            } else {
                1.5
            },
            static_cull,
            live_cull: static_cull,
            cull_follows_envi_fog: settings.cull_follows_envi_fog,
            far: view_distance + REGION_SIZE,
            terrain_lod: settings.terrain_lod.switch_distances(),
        }
    }

    /// The range a terrain LOD level (0 = full grid, 1 = half, 2 = quarter)
    /// is drawn over, measured to the region's AABB centre. Hard switches:
    /// the splat material has no dither crossfade, and at these distances
    /// the fog hides the step. With LOD off, the full grid covers every
    /// distance and the coarser levels none.
    pub fn terrain_lod_range(&self, level: u8) -> VisibilityRange {
        let range = |start: f32, end: f32| VisibilityRange {
            start_margin: start..start,
            end_margin: end..end,
            use_aabb: true,
        };
        match (self.terrain_lod, level) {
            (None, 0) => range(0.0, f32::MAX),
            (None, _) => range(f32::MAX, f32::MAX),
            (Some((half, _)), 0) => range(0.0, half),
            (Some((half, quarter)), 1) => range(half, quarter),
            (Some((_, quarter)), _) => range(quarter, f32::MAX),
        }
    }

    /// The cull distance for a frame whose fog is opaque at `fog_end`:
    /// that end rounded up to [`Self::LIVE_CULL_STEP`], never past the
    /// configured ceiling. The ceiling itself when following is off.
    pub fn live_cull_for(&self, fog_end: f32) -> f32 {
        if !self.cull_follows_envi_fog {
            return self.static_cull;
        }
        ((fog_end / Self::LIVE_CULL_STEP).ceil() * Self::LIVE_CULL_STEP).min(self.static_cull)
    }
}

/// `value` if it is finite and inside `min..=max`, else the nearest bound
/// with a one-time warning naming the key.
fn valid(key: &str, value: f32, min: f32, max: f32) -> f32 {
    if !value.is_finite() {
        bevy::log::warn_once!("graphics.view.{key}: {value} is not a number; using {max}");
        return max;
    }
    if value < min || value > max {
        let clamped = value.clamp(min, max);
        bevy::log::warn_once!(
            "graphics.view.{key}: {value} is outside {min}..={max}; using {clamped}"
        );
        return clamped;
    }
    value
}

/// Re-derives [`ViewRange`] from `graphics.view` whenever the config changes;
/// also seeds it on the frame the config is inserted. Leaves the resource
/// untouched while the view settings themselves are unchanged.
pub fn apply_view_range(config: Res<ClientConfig>, mut view: ResMut<ViewRange>) {
    if view.source != config.graphics.view {
        *view = ViewRange::from_settings(&config.graphics.view);
    }
}

/// A terrain ground mesh's LOD level: 0 = full grid, 1 = every 2nd vertex,
/// 2 = every 4th (`load_terrain_system`).
#[derive(Component, Debug, Clone, Copy)]
pub struct TerrainLodLevel(pub u8);

/// Re-applies the terrain LOD switch distances when they change, so
/// `graphics.view.terrain_lod` is live for the regions built with LOD meshes.
/// Regions built while it was off have only the full grid: they keep it until
/// they are streamed in again.
pub fn apply_terrain_lod(
    view: Res<ViewRange>,
    mut levels: Query<(&TerrainLodLevel, &mut VisibilityRange)>,
    mut applied: Local<Option<Option<(f32, f32)>>>,
) {
    if *applied == Some(view.terrain_lod) {
        return;
    }
    *applied = Some(view.terrain_lod);
    for (level, mut range) in &mut levels {
        range.set_if_neq(view.terrain_lod_range(level.0));
    }
}

/// The shortest character sight range: entity picking reaches 1000
/// (`cursor::interactions`) and nameplates 600 (`hud::nameplates`), so a
/// character hidden here can be neither clicked nor labelled.
pub const MIN_CHARACTER_DISTANCE: f32 = 1000.0;

/// Marks a character [`cull_distant_characters`] hid, so it restores only
/// what it hid itself and never reveals one another system keeps hidden.
#[derive(Component)]
pub struct SightCulled;

/// Hides other players, NPCs and monsters past `graphics.view.
/// character_distance` (0 = never), measured from the camera, and shows them
/// again once they are back inside 90% of it. The 10% band keeps a character
/// walking along the edge from flickering. Only characters that are visible
/// get hidden, and only the ones hidden here get shown again.
pub fn cull_distant_characters(
    mut commands: Commands,
    config: Res<ClientConfig>,
    cameras: Query<(&Camera, &bevy::camera::RenderTarget, &GlobalTransform), With<Camera3d>>,
    mut characters: Query<(
        Entity,
        &GlobalTransform,
        &crate::plugins::net::entities::RemoteEntity,
        &mut Visibility,
        Has<SightCulled>,
    )>,
) {
    use crate::plugins::net::entities::RemoteEntity;
    let range = match config.graphics.view.character_distance {
        range if range > 0.0 => range.max(MIN_CHARACTER_DISTANCE),
        _ => 0.0,
    };
    let camera = crate::plugins::effects::systems::main_world_camera(&cameras)
        .map(GlobalTransform::translation);
    for (entity, global, kind, mut visibility, culled) in &mut characters {
        if *kind == RemoteEntity::Item {
            continue;
        }
        let dist_sq = camera.map_or(0.0, |c| global.translation().distance_squared(c));
        let beyond = range > 0.0 && dist_sq > range * range;
        let inside = range <= 0.0 || dist_sq < (range * 0.9) * (range * 0.9);
        if beyond && !culled && *visibility != Visibility::Hidden {
            *visibility = Visibility::Hidden;
            commands.entity(entity).insert(SightCulled);
        } else if inside && culled {
            visibility.set_if_neq(Visibility::Inherited);
            commands.entity(entity).remove::<SightCulled>();
        }
    }
}

/// What a map-object mesh part's distance LOD is computed from, kept so the
/// range can be recomputed when its inputs change instead of only applying
/// to parts spawned afterwards (`commands::mesh_visibility_range`).
#[derive(Component, Debug, Clone, Copy)]
pub struct PartLod {
    /// The part's largest bounding-box dimension.
    pub extent: f32,
    /// Whether the part belongs to a `res/nature/` resource, which
    /// `nature_view_distance` caps.
    pub nature: bool,
}

/// Recomputes every map-object part's `VisibilityRange` when its inputs
/// change: the cull ceiling ([`ViewRange::static_cull`]) or
/// `graphics.objects`. That makes both live rather than spawn-time only. A
/// diff against the last applied inputs keeps unrelated config edits from
/// re-walking every part.
pub fn apply_object_lod(
    view: Res<ViewRange>,
    config: Res<ClientConfig>,
    mut parts: Query<(&PartLod, &mut VisibilityRange)>,
    mut applied: Local<Option<(f32, ObjectLodSettings)>>,
) {
    let inputs = (view.static_cull, config.graphics.objects.clone());
    let unchanged = applied.as_ref().is_some_and(|(cull, lod)| {
        *cull == inputs.0
            && lod.factor == inputs.1.factor
            && lod.min_distance == inputs.1.min_distance
            && lod.fade == inputs.1.fade
            && lod.nature_view_distance == inputs.1.nature_view_distance
    });
    if unchanged {
        return;
    }
    // The first run only records: parts spawned so far already carry the
    // range these inputs give.
    let first = applied.is_none();
    *applied = Some(inputs.clone());
    if first {
        return;
    }
    let (ceiling, lod) = inputs;
    for (part, mut range) in &mut parts {
        range.set_if_neq(lod.part_range(part.extent, part.nature, ceiling));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults must reproduce the distances the client used before
    /// they were configurable: fog 3840..5760, a ±4 region load ring and a
    /// 7680 far plane.
    #[test]
    fn the_defaults_are_the_old_constants() {
        let view = ViewRange::default();
        assert_eq!(view.fog_start, 3840.0);
        assert_eq!(view.fog_end, 5760.0);
        assert_eq!(view.static_cull, 5760.0);
        assert_eq!(view.live_cull, 5760.0);
        assert_eq!(view.load_ring, 4);
        assert_eq!(view.far, 7680.0);
    }

    #[test]
    fn the_load_ring_covers_the_view_distance() {
        let ring = |view_distance| {
            ViewRange::from_settings(&ViewSettings {
                view_distance,
                fog_end: view_distance,
                ..default()
            })
            .load_ring
        };
        assert_eq!(ring(1920.0), 2);
        assert_eq!(ring(2880.0), 3);
        assert_eq!(ring(3840.0), 3);
        assert_eq!(ring(9600.0), 6);
    }

    /// Fog cannot end past what is streamed, and the cull cannot reach
    /// past the view distance either.
    #[test]
    fn distances_clamp_to_the_view_distance() {
        let view = ViewRange::from_settings(&ViewSettings {
            view_distance: 2880.0,
            fog_start: 4000.0,
            fog_end: 5760.0,
            fog_cull_distance: 9000.0,
            ..default()
        });
        assert_eq!(view.fog_end, 2880.0);
        assert_eq!(view.fog_start, 2880.0);
        assert_eq!(view.static_cull, 2880.0);
        assert_eq!(view.far, 2880.0 + REGION_SIZE);

        let huge = ViewRange::from_settings(&ViewSettings {
            view_distance: 1.0e6,
            ..default()
        });
        assert_eq!(huge.view_distance, ViewRange::MAX_VIEW_DISTANCE);
        let nan = ViewRange::from_settings(&ViewSettings {
            view_distance: f32::NAN,
            ..default()
        });
        assert_eq!(nan.view_distance, ViewRange::MAX_VIEW_DISTANCE);
    }

    #[test]
    fn an_explicit_cull_distance_replaces_the_fog_end() {
        let view = ViewRange::from_settings(&ViewSettings {
            fog_cull_distance: 3000.0,
            ..default()
        });
        assert_eq!(view.static_cull, 3000.0);
        assert_eq!(view.fog_end, 5760.0);
    }

    #[test]
    fn the_live_cull_follows_the_fog_only_when_asked() {
        let fixed = ViewRange::default();
        assert_eq!(fixed.live_cull_for(1000.0), 5760.0);

        let follow = ViewRange::from_settings(&ViewSettings {
            cull_follows_envi_fog: true,
            ..default()
        });
        // rounded up to the next 240-unit step, never below the fog end
        assert_eq!(follow.live_cull_for(1000.0), 1200.0);
        assert_eq!(follow.live_cull_for(1200.0), 1200.0);
        // and never past the ceiling
        assert_eq!(follow.live_cull_for(9000.0), 5760.0);
    }

    /// The camera far plane must keep its depth range precise at the
    /// largest allowed view distance (see `camera::the_depth_range_stays_precise`).
    #[test]
    fn the_far_plane_stays_precise_at_the_maximum() {
        let view = ViewRange::from_settings(&ViewSettings {
            view_distance: ViewRange::MAX_VIEW_DISTANCE,
            ..default()
        });
        // NEAR is 1.0, so the far plane is the depth ratio
        assert!(view.far <= 20_000.0, "far {} too deep", view.far);
    }
}
