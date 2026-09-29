//! The Video pane's "Shadow Detail" row (`SROptionSet` id 1 /
//! `UIIT_STT_SHADOW_DETAIL`), wired to the Sun's shadow maps.
//!
//! Idea: the row already steps `graphics.shadows.enabled`
//! (`options_video::config_row`), and that key is read exactly once — when
//! `map::setup_lighting` spawns the Sun (`map/mod.rs`:
//! `shadow_maps_enabled: is_pbr && shadows.enabled`). So the row was saved
//! and honoured on the next launch while doing nothing in the running client,
//! which is why it carried a "restart" note. This follows the key into the
//! live light instead, the same shape `effects::apply_effect_quality` uses to
//! follow `graphics.effect_quality`. The pane's own data path stays the single
//! source of truth: nothing here re-reads the option bank.
//!
//! Scope, stated rather than hidden: shadow maps only exist in
//! [`RenderMode::Pbr`]. In the default `vanilla` mode the Sun carries no
//! [`PbrModeActive`] marker, its illuminance is 0 and
//! `environment::apply_render_mode` keeps `shadow_maps_enabled` false — the
//! original has no dynamic shadows either. The query below is filtered on
//! that marker, so in vanilla this row changes nothing, and the pane says so
//! through `Requirement::PbrLighting` rather than letting the row claim an
//! effect.
//!
//! The write happens only while the config is changed, which is what keeps it
//! off the render-debug panel's toes: that panel owns the same field while it
//! is open (`dev::render_debug`, `enable_shadows`), and an ungated apply would
//! stamp the option back over it every frame.
//!
//! [`RenderMode::Pbr`]: crate::plugins::config::graphics::RenderMode::Pbr

use bevy::prelude::*;

use crate::plugins::config::ClientConfig;

use super::{PbrModeActive, Sun};

/// Follows `graphics.shadows.enabled` into the Sun's shadow maps.
pub fn apply_shadow_detail_option(
    config: Res<ClientConfig>,
    mut sun: Query<&mut DirectionalLight, (With<Sun>, With<PbrModeActive>)>,
) {
    let wanted = config.graphics.shadows.enabled;
    for mut light in sun.iter_mut() {
        // Guarded write: an unconditional one dirties the light on every
        // config change, including the ones for unrelated keys.
        if light.shadow_maps_enabled != wanted {
            light.shadow_maps_enabled = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// `pbr: false` is the vanilla Sun — spawned without the
    /// [`PbrModeActive`] marker, which is the filter that keeps this row from
    /// claiming an effect in a mode that draws no shadows at all.
    fn app_with_sun(pbr: bool) -> (App, Entity) {
        let mut app = App::new();
        app.insert_resource(test_config())
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
            .expect("the Sun keeps its light")
            .shadow_maps_enabled
    }

    fn set_enabled(app: &mut App, enabled: bool) {
        app.world_mut()
            .resource_mut::<ClientConfig>()
            .graphics
            .shadows
            .enabled = enabled;
    }

    /// The wiring as a behaviour test: the config key must reach the live
    /// light, both ways, without a restart.
    #[test]
    fn the_key_reaches_the_live_sun_both_ways() {
        let (mut app, sun) = app_with_sun(true);
        set_enabled(&mut app, false);
        app.update();
        assert!(!shadows(&app, sun));

        set_enabled(&mut app, true);
        app.update();
        assert!(shadows(&app, sun));
    }

    /// The control that keeps the claim honest: in vanilla lighting the Sun
    /// has no `PbrModeActive` marker, so the row must leave it alone instead
    /// of pretending to switch shadows that are not drawn.
    #[test]
    fn the_vanilla_sun_is_left_alone() {
        let (mut app, sun) = app_with_sun(false);
        set_enabled(&mut app, false);
        app.update();
        assert!(
            shadows(&app, sun),
            "vanilla has no shadow pass; this row must not touch its light"
        );
    }
}
