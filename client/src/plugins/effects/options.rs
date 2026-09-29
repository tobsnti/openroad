//! The Video pane's "Effect Quality" row (`SROptionSet` id 13 /
//! `UIIT_STT_EFFECT_QUALITY`), wired to the effect runtime.
//!
//! Idea: the original ships this row as a performance escape hatch — its own
//! hover help is "Toggles skill effects to improve performance"
//! (`UIIT_STT_VIDIO_TTDESC_16`, textuisystem :968-984). We already own
//! exactly that switch: [`EffectsEnabled`] pauses the whole effect schedule
//! for zero CPU cost. Without this wiring the row renders but is inert
//! (`Backing::Missing`), i.e. a control with no read site.
//!
//! The one trap this file exists to avoid: `EffectsEnabled(false)` only
//! *pauses* the systems, it does not hide anything, so flipping it alone
//! leaves every live effect frozen on screen instead of gone. The dev
//! inspector solves this by hiding the wrappers as well
//! (`dev::render_debug`, "pause the runtime and hide the (frozen)
//! wrappers"); this system does the same two steps, and restores
//! `Visibility::Inherited` (not `Visible`) on re-enable for the same reason
//! it does — a wrapper must keep following its anchor's visibility.
//!
//! The row is a **three-step** control in the original (`GraphicProfile::
//! quality_step` reads the step index), not the toggle its tooltip suggests.
//! [`EffectsEnabled`] is one boolean, so step 0 is off and steps 1 and 2 are
//! both on: the original's middle step is **not distinguished**, and which
//! effects it keeps is UNKNOWN. Stated rather than approximated.

use bevy::prelude::*;

use crate::plugins::options_video::{GraphicProfileTab, VideoPane};
use crate::plugins::settings::options::GameOptions;

use super::components::EffectInstance;
use super::EffectsEnabled;

/// Id of the "Effect Quality" row within a profile bank (`DETAIL_ROWS`).
pub const EFFECT_QUALITY_ID: u16 = 13;

/// The row's step index in the active graphic profile.
pub fn effect_quality_step(options: &GameOptions, profile: GraphicProfileTab) -> u8 {
    let bank = match profile {
        GraphicProfileTab::One => &options.video.graphic1,
        GraphicProfileTab::Two => &options.video.graphic2,
    };
    bank.quality_step(EFFECT_QUALITY_ID)
}

/// Whether the effect runtime should run at this step. Step 0 is the
/// original's "Turn Off All"; 1 and 2 both mean on here (module doc).
pub fn effect_quality_on(options: &GameOptions, profile: GraphicProfileTab) -> bool {
    effect_quality_step(options, profile) != 0
}

/// Applies the Effect Quality row to the effect runtime.
///
/// Gated on `options.is_changed()` like `apply_bloom_option`, which is what
/// keeps it from fighting the dev inspector: the panel writes
/// `EffectsEnabled` every frame it is open, and an ungated apply here would
/// stamp the option back over it on the next frame.
pub fn apply_effect_quality_option(
    options: Res<GameOptions>,
    panes: Query<&VideoPane>,
    mut enabled: ResMut<EffectsEnabled>,
    mut wrappers: Query<&mut Visibility, With<EffectInstance>>,
) {
    if !options.is_changed() {
        return;
    }
    let profile = panes.iter().next().map(|p| p.profile).unwrap_or_default();
    let on = effect_quality_on(&options, profile);
    if enabled.0 == on {
        return;
    }
    enabled.0 = on;
    for mut visibility in wrappers.iter_mut() {
        *visibility = if on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options_with(profile_two: bool, value: u8) -> GameOptions {
        let mut options = GameOptions::default();
        let bank = if profile_two {
            &mut options.video.graphic2
        } else {
            &mut options.video.graphic1
        };
        bank.set_quality_step(EFFECT_QUALITY_ID, value);
        options
    }

    #[test]
    fn an_unwritten_row_reads_as_on() {
        let options = GameOptions::default();
        assert!(effect_quality_on(&options, GraphicProfileTab::One));
        assert!(effect_quality_on(&options, GraphicProfileTab::Two));
    }

    #[test]
    fn step_zero_is_off_and_the_two_effect_steps_are_both_on() {
        assert!(!effect_quality_on(
            &options_with(false, 0),
            GraphicProfileTab::One
        ));
        assert!(effect_quality_on(
            &options_with(false, 1),
            GraphicProfileTab::One
        ));
        // The original's middle step ("See Hit Effects") is not
        // distinguished here: the runtime is one switch (module doc).
        assert!(effect_quality_on(
            &options_with(false, 2),
            GraphicProfileTab::One
        ));
    }

    #[test]
    fn the_two_profile_banks_are_read_separately() {
        let options = options_with(true, 0);
        assert!(effect_quality_on(&options, GraphicProfileTab::One));
        assert!(!effect_quality_on(&options, GraphicProfileTab::Two));
    }

    /// The wiring as a behaviour test: turning the row
    /// off must both pause the runtime *and* hide the frozen wrappers.
    #[test]
    fn turning_the_row_off_pauses_the_runtime_and_hides_the_wrappers() {
        let mut app = App::new();
        app.init_resource::<GameOptions>()
            .init_resource::<EffectsEnabled>()
            .add_systems(Update, apply_effect_quality_option);
        let wrapper = app
            .world_mut()
            .spawn((
                EffectInstance {
                    handle: Handle::default(),
                },
                Visibility::Inherited,
                Transform::default(),
            ))
            .id();

        app.update();
        assert!(app.world().resource::<EffectsEnabled>().0, "default is on");

        app.world_mut()
            .resource_mut::<GameOptions>()
            .video
            .graphic1
            .set_quality_step(EFFECT_QUALITY_ID, 0);
        app.update();
        assert!(!app.world().resource::<EffectsEnabled>().0);
        assert_eq!(
            app.world().get::<Visibility>(wrapper),
            Some(&Visibility::Hidden),
            "a paused runtime would otherwise leave the effect frozen on screen"
        );

        app.world_mut()
            .resource_mut::<GameOptions>()
            .video
            .graphic1
            .set_quality_step(EFFECT_QUALITY_ID, 1);
        app.update();
        assert!(app.world().resource::<EffectsEnabled>().0);
        assert_eq!(
            app.world().get::<Visibility>(wrapper),
            Some(&Visibility::Inherited),
            "Inherited, not Visible: the wrapper must keep following its anchor"
        );
    }
}
