//! The Video pane's "Effect Quality" row (`SROptionSet` id 13 /
//! `UIIT_STT_EFFECT_QUALITY`), wired to the effect runtime.
//!
//! Idea: the original ships this row as a performance escape hatch — its own
//! hover help is "Toggles skill effects to improve performance"
//! (`UIIT_STT_VIDIO_TTDESC_16`, textuisystem :968-984). We already own
//! exactly that switch: [`EffectsEnabled`] pauses the whole effect schedule
//! for zero CPU cost. The row was rendered but inert (`Backing::Missing`),
//! i.e. a dead wire — a control with no read site.
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
//! Deliberately NOT invented (ADR-0009): the row's *value space*. The
//! `SROptionSet` cell is a `u16` and the name says "Quality", so a graded
//! 0/1/2 scale is conceivable, but nothing states what the steps would mean —
//! and the original's own tooltip says "toggles". So this
//! reads `!= 0` as on, exactly like the Bloom row
//! (`options_video::apply_bloom_option`), and a graded scale stays an
//! UNKNOWN rather than a guessed ramp.

use bevy::prelude::*;

use crate::plugins::options_video::{GraphicProfileTab, VideoPane};
use crate::plugins::settings::options::GameOptions;

use super::components::EffectInstance;
use super::EffectsEnabled;

/// Id of the "Effect Quality" row within a profile bank (`DETAIL_ROWS`).
pub const EFFECT_QUALITY_ID: u16 = 13;

/// Reads the row out of the active graphic profile. `None` (row never
/// written) means on: the original ships the detail rows enabled.
pub fn effect_quality_on(options: &GameOptions, profile: GraphicProfileTab) -> bool {
    let bank = match profile {
        GraphicProfileTab::One => &options.video.graphic1,
        GraphicProfileTab::Two => &options.video.graphic2,
    };
    bank.quality.get(&EFFECT_QUALITY_ID).copied().unwrap_or(1) != 0
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

    fn options_with(profile_two: bool, value: u16) -> GameOptions {
        let mut options = GameOptions::default();
        let bank = if profile_two {
            &mut options.video.graphic2
        } else {
            &mut options.video.graphic1
        };
        bank.quality.insert(EFFECT_QUALITY_ID, value);
        options
    }

    #[test]
    fn an_unwritten_row_reads_as_on() {
        let options = GameOptions::default();
        assert!(effect_quality_on(&options, GraphicProfileTab::One));
        assert!(effect_quality_on(&options, GraphicProfileTab::Two));
    }

    #[test]
    fn zero_is_off_and_every_other_value_is_on() {
        assert!(!effect_quality_on(
            &options_with(false, 0),
            GraphicProfileTab::One
        ));
        assert!(effect_quality_on(
            &options_with(false, 1),
            GraphicProfileTab::One
        ));
        // "Quality" is a u16 cell; while the value space is unknown, any
        // non-zero step means on (see the module doc's ADR-0009 note).
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

    /// The dead wire this module closes, as a behaviour test: turning the row
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
            .quality
            .insert(EFFECT_QUALITY_ID, 0);
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
            .quality
            .insert(EFFECT_QUALITY_ID, 1);
        app.update();
        assert!(app.world().resource::<EffectsEnabled>().0);
        assert_eq!(
            app.world().get::<Visibility>(wrapper),
            Some(&Visibility::Inherited),
            "Inherited, not Visible: the wrapper must keep following its anchor"
        );
    }
}
