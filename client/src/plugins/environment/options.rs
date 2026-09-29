//! The Video pane's "Shadow Detail" row (`SROptionSet` id 1 /
//! `UIIT_STT_SHADOW_DETAIL`), wired to the Sun's shadow maps.
//!
//! Idea: the row's own hover help is "Able to control the degree of shadow
//! details" (`UIIT_STT_VIDIO_TTDESC_04`), and the one shadow source this
//! client has is the Sun's cascaded shadow maps. Toggling them at runtime is
//! already proven — the render-debug panel writes the very same field
//! (`dev::render_debug`, `enable_shadows`) — so without this wiring the row
//! renders but is inert, i.e. a control with no read site.
//!
//! Scope, stated rather than hidden: shadow maps only exist in
//! [`RenderMode::Pbr`]. In the default `vanilla` mode the Sun carries no
//! [`PbrModeActive`] marker, its illuminance is 0 and
//! `environment::apply_render_mode` keeps `shadow_maps_enabled` false — the
//! original has no dynamic shadows either. The query below is filtered on
//! that marker, so in vanilla this row changes nothing and must not claim to.
//!
//! Deliberately NOT invented (ADR-0009): the row's *value space*. The
//! `SROptionSet` cell is a `u16` and the name says "degree", so a graded
//! scale is conceivable, but nothing states what the steps would mean. This
//! reads `!= 0` as on, exactly like the Bloom and Effect Quality rows, and a
//! graded scale stays an UNKNOWN rather than a guessed ramp.
//!
//! One concrete UNKNOWN worth naming, because it decides how the cell should
//! be read: a shipped `SROptionSet.dat` does **not** carry a plain 0/1 here.
//! Its two bytes read `00 01` for this row (little-endian `0x0100`), `00 00`
//! for Water Reflection and `01 02` for Effect Quality. Whether that is one
//! number or a `(chosen, maximum)` pair is not established, so neither byte
//! is singled out: any non-zero cell is on, which is the only reading that
//! adds no assumption. The pane's own click writes a clean 0/1 back
//! (`options_video::spawn_detail_row`), so this only concerns an imported
//! file.
//!
//! [`RenderMode::Pbr`]: crate::plugins::config::graphics::RenderMode::Pbr

use bevy::prelude::*;

use crate::plugins::options_video::{GraphicProfileTab, VideoPane};
use crate::plugins::settings::options::GameOptions;

use super::{PbrModeActive, Sun};

/// Id of the "Shadow Detail" row within a profile bank (`DETAIL_ROWS`).
pub const SHADOW_DETAIL_ID: u16 = 1;

/// Reads the row out of the active graphic profile. `None` (row never
/// written) means on: the original ships the detail rows enabled.
pub fn shadow_detail_on(options: &GameOptions, profile: GraphicProfileTab) -> bool {
    let bank = match profile {
        GraphicProfileTab::One => &options.video.graphic1,
        GraphicProfileTab::Two => &options.video.graphic2,
    };
    bank.quality.get(&SHADOW_DETAIL_ID).copied().unwrap_or(1) != 0
}

/// Applies the Shadow Detail row to the Sun's shadow maps.
///
/// Gated on `options.is_changed()` like `options_video::apply_bloom_option`,
/// which is what keeps it from fighting the dev inspector: the panel writes
/// `shadow_maps_enabled` while it is open, and an ungated apply here would
/// stamp the option back over it on the next frame.
pub fn apply_shadow_detail_option(
    options: Res<GameOptions>,
    panes: Query<&VideoPane>,
    mut sun: Query<&mut DirectionalLight, (With<Sun>, With<PbrModeActive>)>,
) {
    if !options.is_changed() {
        return;
    }
    let profile = panes.iter().next().map(|p| p.profile).unwrap_or_default();
    let on = shadow_detail_on(&options, profile);
    for mut light in sun.iter_mut() {
        // Guarded write: an unconditional one dirties the light every frame
        // the options resource changes for an unrelated row.
        if light.shadow_maps_enabled != on {
            light.shadow_maps_enabled = on;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options_with(profile_two: bool, value: u16) -> GameOptions {
        let mut options = GameOptions::default();
        let bank = if profile_two {
            &mut options.video.graphic2
        } else {
            &mut options.video.graphic1
        };
        bank.quality.insert(SHADOW_DETAIL_ID, value);
        options
    }

    #[test]
    fn an_unwritten_row_reads_as_on() {
        let options = GameOptions::default();
        assert!(shadow_detail_on(&options, GraphicProfileTab::One));
        assert!(shadow_detail_on(&options, GraphicProfileTab::Two));
    }

    #[test]
    fn zero_is_off_and_every_other_value_is_on() {
        assert!(!shadow_detail_on(
            &options_with(false, 0),
            GraphicProfileTab::One
        ));
        assert!(shadow_detail_on(
            &options_with(false, 1),
            GraphicProfileTab::One
        ));
        // "degree of shadow details" is a u16 cell; while the value space is
        // unknown, any non-zero step means on (see the module doc).
        assert!(shadow_detail_on(
            &options_with(false, 2),
            GraphicProfileTab::One
        ));
    }

    #[test]
    fn the_two_profile_banks_are_read_separately() {
        let options = options_with(true, 0);
        assert!(shadow_detail_on(&options, GraphicProfileTab::One));
        assert!(!shadow_detail_on(&options, GraphicProfileTab::Two));
    }

    fn app_with_sun(pbr: bool) -> (App, Entity) {
        let mut app = App::new();
        app.init_resource::<GameOptions>()
            .add_systems(Update, apply_shadow_detail_option);
        let mut sun = app.world_mut().spawn((
            Sun,
            DirectionalLight {
                shadow_maps_enabled: true,
                ..default()
            },
        ));
        if pbr {
            sun.insert(PbrModeActive);
        }
        let sun = sun.id();
        (app, sun)
    }

    fn shadows(app: &App, sun: Entity) -> bool {
        app.world()
            .get::<DirectionalLight>(sun)
            .unwrap()
            .shadow_maps_enabled
    }

    /// The wiring as a behaviour test: the row must reach the Sun's shadow
    /// maps, both ways.
    #[test]
    fn turning_the_row_off_disables_the_suns_shadow_maps() {
        let (mut app, sun) = app_with_sun(true);
        app.update();
        assert!(shadows(&app, sun), "default is on");

        app.world_mut()
            .resource_mut::<GameOptions>()
            .video
            .graphic1
            .quality
            .insert(SHADOW_DETAIL_ID, 0);
        app.update();
        assert!(!shadows(&app, sun));

        app.world_mut()
            .resource_mut::<GameOptions>()
            .video
            .graphic1
            .quality
            .insert(SHADOW_DETAIL_ID, 1);
        app.update();
        assert!(shadows(&app, sun));
    }

    /// The control that keeps the claim honest: in vanilla mode the Sun has
    /// no `PbrModeActive` marker and there are no shadows to control, so the
    /// row must leave the light alone instead of pretending.
    #[test]
    fn a_vanilla_mode_sun_is_left_alone() {
        let (mut app, sun) = app_with_sun(false);
        app.world_mut()
            .resource_mut::<GameOptions>()
            .video
            .graphic1
            .quality
            .insert(SHADOW_DETAIL_ID, 0);
        app.update();
        assert!(
            shadows(&app, sun),
            "vanilla has no dynamic shadows; the row must not touch the light"
        );
    }
}
