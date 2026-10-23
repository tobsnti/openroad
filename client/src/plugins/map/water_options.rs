//! The Video pane's "Water Reflection" row (`SROptionSet` id 4 /
//! `UIIT_STT_WATER_REFLECTION`), wired to the high-quality water's reflection
//! blend.
//!
//! Idea: the row steps `graphics.water.reflection`, and the one factor that
//! decides how much reflection the water shows is `sky_tint.a` —
//! `water_hq.wesl:145` multiplies its whole Fresnel term by it and
//! `:153` mixes the result in, so zero means "no reflection contribution at
//! all" for both halves of that shader: the raymarched reflection and the
//! flat sky-tint fallback it compiles down to without a depth prepass
//! (`:23`). It is a live uniform, so the row can be honoured without
//! rebuilding anything.
//!
//! Scope, stated rather than hidden:
//!
//! * High tier only. With `graphics.water.quality = low` the
//!   [`WaterNormalMaterial`] resource is never inserted (`map::setup_terrain_mesh`),
//!   that tier drops the reflection term entirely, and the system below leaves
//!   the scene alone instead of claiming otherwise. The pane says the same
//!   thing through `Requirement::HighWater`.
//! * "Off" removes the reflection *contribution*, not its cost: the shader
//!   still marches the depth prepass and its result is then mixed in with
//!   factor zero. Whether the original's row was a performance switch is
//!   UNKNOWN.
//! * The strength used for "On" is the material default (`WaterHqSettings::
//!   default`, 0.55). There is no configuration knob for it, and whether the
//!   original knew a strength at all is UNKNOWN — its list offers two values,
//!   which says on/off and nothing more.
//! * The day/night cycle writes this very field and deliberately keeps its
//!   alpha (`environment::apply_environment` re-extends the sky colour with
//!   the strength it found), which is why the two systems can write in any
//!   order without fighting.

use bevy::prelude::*;

use crate::plugins::config::ClientConfig;

use super::terrain::WaterNormalMaterial;
use super::water_hq_material::{absorption_distance, HighQualityWaterMaterial, WaterHqSettings};

/// Reflection blend strength for "On": whatever the material ships with.
fn reflection_strength() -> f32 {
    WaterHqSettings::default().sky_tint.w
}

/// Follows `graphics.water.reflection` into the high-quality water material.
pub fn apply_water_reflection_option(
    config: Res<ClientConfig>,
    handle: Option<Res<WaterNormalMaterial>>,
    mut materials: ResMut<Assets<HighQualityWaterMaterial>>,
) {
    // Low water tier: no high-quality material exists, so there is nothing to
    // switch (see the module doc).
    let Some(handle) = handle else {
        return;
    };
    let wanted = if config.graphics.water.reflection {
        reflection_strength()
    } else {
        0.0
    };
    let Some(mut material) = materials.get_mut(&handle.0) else {
        return;
    };
    // Guarded write: a material change re-prepares the whole extended bind
    // group, so it must only happen when the strength actually moved.
    if material.extension.settings.sky_tint.w != wanted {
        material.extension.settings.sky_tint.w = wanted;
    }
}

/// Follows `graphics.water.shore_fade` / `graphics.water.absorption` into the
/// high-quality water material.
///
/// Two keys, two homes, because the shader reads them from two places: the
/// fade is a uniform field it samples per fragment, while the absorption is
/// `StandardMaterial::attenuation_distance` — that is both the divisor of
/// bevy's Beer-Lambert term and the flag that removes the branch entirely when
/// it is infinite (`water_hq_material::absorption_distance`).
///
/// Same scope as [`apply_water_reflection_option`]: high tier only, and only
/// *visible* where the camera has a depth prepass, since without one the
/// shader cannot measure a water column.
pub fn apply_water_depth_options(
    config: Res<ClientConfig>,
    handle: Option<Res<WaterNormalMaterial>>,
    mut materials: ResMut<Assets<HighQualityWaterMaterial>>,
) {
    let Some(handle) = handle else {
        return;
    };
    let Some(mut material) = materials.get_mut(&handle.0) else {
        return;
    };
    let shore_fade = config.graphics.water.shore_fade.max(0.0);
    let attenuation = absorption_distance(config.graphics.water.absorption);
    // Guarded writes, as above: a material change re-prepares the whole
    // extended bind group.
    if material.extension.settings.shore_fade != shore_fade {
        material.extension.settings.shore_fade = shore_fade;
    }
    if material.base.attenuation_distance != attenuation {
        material.base.attenuation_distance = attenuation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::plugins::map::water_hq_material::{WaterExtension, ABSORPTION_FULL_DEPTH};

    /// The shipped file, as everywhere else in the tree: `ClientConfig` has no
    /// `Default`, and the example is the only config whose values are reviewed.
    fn test_config() -> ClientConfig {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../config.example")
            .to_str()
            .expect("the example path is utf-8")
            .to_string();
        ClientConfig::from_file(&path).expect("config.example.yaml matches ClientConfig")
    }

    /// `with_material: false` is the low water tier — the handle resource is
    /// simply absent there.
    fn app_with_water(with_material: bool) -> App {
        let mut app = App::new();
        app.insert_resource(test_config())
            .init_resource::<Assets<HighQualityWaterMaterial>>()
            .add_systems(Update, apply_water_reflection_option);
        if with_material {
            let handle = app
                .world_mut()
                .resource_mut::<Assets<HighQualityWaterMaterial>>()
                .add(HighQualityWaterMaterial {
                    base: StandardMaterial::default(),
                    extension: WaterExtension {
                        settings: WaterHqSettings::default(),
                        normal_map: Handle::default(),
                    },
                });
            app.world_mut().insert_resource(WaterNormalMaterial(handle));
        }
        app
    }

    fn strength(app: &App) -> Option<f32> {
        let handle = app.world().get_resource::<WaterNormalMaterial>()?;
        let materials = app.world().resource::<Assets<HighQualityWaterMaterial>>();
        Some(materials.get(&handle.0)?.extension.settings.sky_tint.w)
    }

    fn set_reflection(app: &mut App, on: bool) {
        app.world_mut()
            .resource_mut::<ClientConfig>()
            .graphics
            .water
            .reflection = on;
    }

    /// The shipped default is the look the client has always had: reflections
    /// on. A row that defaulted to off would silently change the water.
    #[test]
    fn the_shipped_default_reflects() {
        let mut app = app_with_water(true);
        assert!(test_config().graphics.water.reflection);
        app.update();
        assert_eq!(strength(&app), Some(reflection_strength()));
    }

    /// The wiring as a behaviour test: the key must reach the water
    /// material's reflection strength, both ways.
    #[test]
    fn turning_the_row_off_zeroes_the_reflection_blend() {
        let mut app = app_with_water(true);
        set_reflection(&mut app, false);
        app.update();
        assert_eq!(strength(&app), Some(0.0));

        set_reflection(&mut app, true);
        app.update();
        assert_eq!(strength(&app), Some(reflection_strength()));
    }

    fn set_depth_keys(app: &mut App, shore_fade: f32, absorption: f32) {
        let mut config = app.world_mut().resource_mut::<ClientConfig>();
        config.graphics.water.shore_fade = shore_fade;
        config.graphics.water.absorption = absorption;
    }

    /// The depth cues as a behaviour test, both ways: the two keys must reach
    /// the uniform and the base material's attenuation, and `0` must switch
    /// each off — infinity for the absorption, because that is what clears
    /// bevy's `ATTENUATION_ENABLED` and drops the branch.
    #[test]
    fn the_depth_keys_reach_the_material_and_zero_switches_them_off() {
        let mut app = App::new();
        app.insert_resource(test_config())
            .init_resource::<Assets<HighQualityWaterMaterial>>()
            .add_systems(Update, apply_water_depth_options);
        let handle = app
            .world_mut()
            .resource_mut::<Assets<HighQualityWaterMaterial>>()
            .add(HighQualityWaterMaterial {
                base: StandardMaterial::default(),
                extension: WaterExtension {
                    settings: WaterHqSettings::default(),
                    normal_map: Handle::default(),
                },
            });
        app.world_mut()
            .insert_resource(WaterNormalMaterial(handle.clone()));

        let read = |app: &App| {
            let materials = app.world().resource::<Assets<HighQualityWaterMaterial>>();
            let material = materials.get(&handle).expect("the material is there");
            (
                material.extension.settings.shore_fade,
                material.base.attenuation_distance,
            )
        };

        set_depth_keys(&mut app, 12.0, 0.5);
        app.update();
        assert_eq!(read(&app), (12.0, ABSORPTION_FULL_DEPTH * 2.0));

        set_depth_keys(&mut app, 0.0, 0.0);
        app.update();
        let (fade, attenuation) = read(&app);
        assert_eq!(fade, 0.0);
        assert!(
            attenuation.is_infinite(),
            "a finite distance keeps bevy's attenuation branch alive"
        );
    }

    /// The low water tier control, for the depth keys too: no high-quality
    /// material means nothing to write, not a panic on the missing resource.
    #[test]
    fn the_depth_keys_leave_the_low_water_tier_alone() {
        let mut app = App::new();
        app.insert_resource(test_config())
            .init_resource::<Assets<HighQualityWaterMaterial>>()
            .add_systems(Update, apply_water_depth_options);
        app.update();
        assert!(
            app.world()
                .resource::<Assets<HighQualityWaterMaterial>>()
                .is_empty(),
            "the low tier must not gain a high-quality material"
        );
    }

    /// The control that keeps the claim honest: on the low water tier there is
    /// no high-quality material, so the row must change nothing rather than
    /// pretend. It would panic on the missing resource if the system read it
    /// unconditionally.
    #[test]
    fn the_low_water_tier_is_left_alone() {
        let mut app = app_with_water(false);
        set_reflection(&mut app, false);
        app.update();
        assert!(strength(&app).is_none(), "no high-quality water material");
        assert!(
            app.world()
                .resource::<Assets<HighQualityWaterMaterial>>()
                .is_empty(),
            "the low tier must not gain a high-quality material"
        );
    }
}
