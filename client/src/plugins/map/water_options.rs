//! The Video pane's "Water Reflection" row (`SROptionSet` id 4 /
//! `UIIT_STT_WATER_REFLECTION`), wired to the high-quality water's
//! reflection blend.
//!
//! Idea: the row is a two-value control (`UIIT_STT_OFF` / `UIIT_STT_ON`), and
//! the one reflection this client's water has is the screen-space reflection
//! the high-quality water shader blends in at grazing angles. Its blend
//! strength is a live uniform — [`WaterHqSettings::sky_tint`]`.a`, the factor
//! the shader multiplies its Fresnel term by (`shaders/water_hq.wgsl`) — so
//! zero means "no reflection contribution at all" and the row can be honoured
//! without rebuilding anything. Without this wiring the row renders but is
//! inert, i.e. a control with no read site.
//!
//! Scope, stated rather than hidden:
//!
//! * It only applies to the high water tier. With `graphics.water.quality =
//!   low` the [`WaterNormalMaterial`] resource is never inserted
//!   (`map::setup_terrain_mesh`), that tier has no reflection to control, and
//!   the system below leaves the scene alone instead of claiming otherwise.
//! * "Off" removes the reflection *contribution*, not its cost: the shader
//!   still marches the depth prepass and its result is then mixed in with
//!   factor zero. Whether the original's row was a performance switch is
//!   UNKNOWN.
//! * The strength used for "On" is the material default (`WaterHqSettings::
//!   default`). There is no configuration knob for it, and whether the
//!   original knew a strength at all is UNKNOWN — its list offers two values,
//!   which says on/off and nothing more.
//! * The day/night cycle writes this very field, and deliberately keeps its
//!   alpha (`environment::apply_environment` re-extends the sky colour with
//!   the strength it found). That is why the two systems can write in any
//!   order without fighting.

use bevy::prelude::*;

use crate::plugins::options_video::{GraphicProfileTab, VideoPane};
use crate::plugins::settings::options::GameOptions;

use super::terrain::WaterNormalMaterial;
use super::water_hq_material::{HighQualityWaterMaterial, WaterHqSettings};

/// Id of the "Water Reflection" row within a profile bank (`DETAIL_ROWS`).
pub const WATER_REFLECTION_ID: u16 = 4;

/// The row's step index in the active graphic profile.
pub fn water_reflection_step(options: &GameOptions, profile: GraphicProfileTab) -> u8 {
    let bank = match profile {
        GraphicProfileTab::One => &options.video.graphic1,
        GraphicProfileTab::Two => &options.video.graphic2,
    };
    bank.quality_step(WATER_REFLECTION_ID)
}

/// Whether the water should reflect at this step. The row has two values, so
/// step 0 is "Off" and everything else is "On".
pub fn water_reflection_on(options: &GameOptions, profile: GraphicProfileTab) -> bool {
    water_reflection_step(options, profile) != 0
}

/// Reflection blend strength for "On": whatever the material ships with.
fn reflection_strength() -> f32 {
    WaterHqSettings::default().sky_tint.w
}

/// Applies the Water Reflection row to the high-quality water material.
///
/// Gated on `options.is_changed()` like `options_video::apply_bloom_option`:
/// the material is shared with the day/night cycle, and an ungated write
/// would touch it every frame the options resource changes for an unrelated
/// row.
pub fn apply_water_reflection_option(
    options: Res<GameOptions>,
    panes: Query<&VideoPane>,
    handle: Option<Res<WaterNormalMaterial>>,
    mut materials: ResMut<Assets<HighQualityWaterMaterial>>,
) {
    if !options.is_changed() {
        return;
    }
    // Low water tier: no high-quality material exists, so there is nothing to
    // switch (see the module doc).
    let Some(handle) = handle else {
        return;
    };
    let profile = panes.iter().next().map(|p| p.profile).unwrap_or_default();
    let wanted = if water_reflection_on(&options, profile) {
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::plugins::map::water_hq_material::WaterExtension;

    fn options_with(profile_two: bool, value: u8) -> GameOptions {
        let mut options = GameOptions::default();
        let bank = if profile_two {
            &mut options.video.graphic2
        } else {
            &mut options.video.graphic1
        };
        bank.set_quality_step(WATER_REFLECTION_ID, value);
        options
    }

    #[test]
    fn an_unwritten_row_reads_as_on() {
        let options = GameOptions::default();
        assert!(water_reflection_on(&options, GraphicProfileTab::One));
        assert!(water_reflection_on(&options, GraphicProfileTab::Two));
    }

    #[test]
    fn step_zero_is_off_and_step_one_is_on() {
        assert!(!water_reflection_on(
            &options_with(false, 0),
            GraphicProfileTab::One
        ));
        assert!(water_reflection_on(
            &options_with(false, 1),
            GraphicProfileTab::One
        ));
    }

    /// A cell written by the original carries the step index in **both**
    /// bytes, so a written "On" reads 0x0101. Reading the cell as a number
    /// instead of a step would call that 257.
    #[test]
    fn a_cell_written_by_the_original_reads_as_its_step() {
        let mut options = GameOptions::default();
        options
            .video
            .graphic1
            .quality
            .insert(WATER_REFLECTION_ID, 0x0101);
        assert_eq!(
            water_reflection_step(&options, GraphicProfileTab::One),
            1,
            "both bytes carry the step"
        );
        options
            .video
            .graphic1
            .quality
            .insert(WATER_REFLECTION_ID, 0x0000);
        assert_eq!(water_reflection_step(&options, GraphicProfileTab::One), 0);
        // An install that was never written keeps the high byte from the
        // shipped file (`00 01`), which must not be mistaken for a step.
        options
            .video
            .graphic1
            .quality
            .insert(WATER_REFLECTION_ID, 0x0100);
        assert_eq!(water_reflection_step(&options, GraphicProfileTab::One), 0);
    }

    #[test]
    fn the_two_profile_banks_are_read_separately() {
        let options = options_with(true, 0);
        assert!(water_reflection_on(&options, GraphicProfileTab::One));
        assert!(!water_reflection_on(&options, GraphicProfileTab::Two));
    }

    /// `with_material: false` is the low water tier — the handle resource is
    /// simply absent there.
    fn app_with_water(with_material: bool) -> App {
        let mut app = App::new();
        app.init_resource::<GameOptions>()
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

    fn set_step(app: &mut App, step: u8) {
        app.world_mut()
            .resource_mut::<GameOptions>()
            .video
            .graphic1
            .set_quality_step(WATER_REFLECTION_ID, step);
    }

    /// The wiring as a behaviour test: the row must reach the water
    /// material's reflection strength, both ways.
    #[test]
    fn turning_the_row_off_zeroes_the_reflection_blend() {
        let mut app = app_with_water(true);
        app.update();
        assert_eq!(strength(&app), Some(reflection_strength()), "default is on");

        set_step(&mut app, 0);
        app.update();
        assert_eq!(strength(&app), Some(0.0));

        set_step(&mut app, 1);
        app.update();
        assert_eq!(strength(&app), Some(reflection_strength()));
    }

    /// The control that keeps the claim honest: on the low water tier there is
    /// no high-quality material, so the row must change nothing rather than
    /// pretend. It would panic on the missing resource if the system read it
    /// unconditionally.
    #[test]
    fn the_low_water_tier_is_left_alone() {
        let mut app = app_with_water(false);
        set_step(&mut app, 0);
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
