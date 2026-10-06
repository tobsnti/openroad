//! Runs Bevy's directional-light shadow visibility only while a directional
//! light actually renders shadow maps.
//!
//! Idea: `bevy_light::check_dir_light_mesh_visibility` finds the meshes each
//! directional light's shadow cascades can see. Even with every light's shadow
//! maps off — always in vanilla mode — it still queues a command that builds a
//! fresh `world.query::<&mut ViewVisibility>()` every frame, and building a
//! query walks every archetype in the world (~2,500 in game). That was
//! ~0.37 ms of main-thread time per frame for no output (chrome trace of
//! 2026-10-05). The sun, the paper-doll and the portrait lights all exist in
//! both modes, so "no directional light" is not the condition — "no light
//! with shadow maps" is.
//!
//! So the system is taken out of `PostUpdate` and put back with its original
//! sets and ordering plus a run condition. A light whose shadow maps were just
//! switched off still gets one run (the change is visible to the condition),
//! which clears its cascades' visible lists the way the system does for
//! shadowless lights.

use bevy::camera::visibility::VisibilitySystems;
use bevy::ecs::schedule::ScheduleCleanupPolicy;
use bevy::light::{check_dir_light_mesh_visibility, SimulationLightSystems};
use bevy::prelude::*;
use bevy::transform::TransformSystems;

pub struct ShadowGatedLightVisibilityPlugin;

impl Plugin for ShadowGatedLightVisibilityPlugin {
    fn build(&self, _app: &mut App) {}

    // `finish`, not `build`: every plugin's `build` has run by now, so the
    // light plugin's registration is there to replace whatever the order.
    fn finish(&self, app: &mut App) {
        match app.remove_systems_in_set(
            PostUpdate,
            check_dir_light_mesh_visibility,
            ScheduleCleanupPolicy::RemoveSystemsOnly,
        ) {
            Ok(removed) if removed > 0 => {
                info!("directional-light shadow visibility runs only while shadow maps are on");
                // the same configuration as `bevy_light::LightPlugin`
                app.add_systems(
                    PostUpdate,
                    check_dir_light_mesh_visibility
                        .run_if(dir_light_shadows_in_use)
                        .in_set(SimulationLightSystems::CheckLightVisibility)
                        .after(VisibilitySystems::CalculateBounds)
                        .after(TransformSystems::Propagate)
                        .after(SimulationLightSystems::UpdateLightFrusta)
                        .after(VisibilitySystems::CheckVisibility)
                        .before(VisibilitySystems::MarkNewlyHiddenEntitiesInvisible),
                );
            }
            // headless/test apps without the light plugin: nothing to gate
            Ok(_) | Err(_) => {}
        }
    }
}

/// True while any directional light renders shadow maps, and for the frame a
/// light's settings change (so a light switching shadows off gets its
/// cascade lists cleared).
pub fn dir_light_shadows_in_use(lights: Query<Ref<DirectionalLight>>) -> bool {
    lights
        .iter()
        .any(|light| light.shadow_maps_enabled || light.is_changed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn shadowless_lights_do_not_need_the_visibility_pass() {
        let mut world = World::new();
        let light = world
            .spawn(DirectionalLight {
                shadow_maps_enabled: false,
                ..default()
            })
            .id();
        // a fresh condition sees the spawn as a change: one run to settle
        assert!(world.run_system_once(dir_light_shadows_in_use).unwrap());

        let mut condition = IntoSystem::into_system(dir_light_shadows_in_use);
        condition.initialize(&mut world);
        condition.run((), &mut world).unwrap();
        assert!(
            !condition.run((), &mut world).unwrap(),
            "settled, shadowless: skip"
        );

        world
            .get_mut::<DirectionalLight>(light)
            .unwrap()
            .shadow_maps_enabled = true;
        assert!(condition.run((), &mut world).unwrap(), "shadows on: run");
        assert!(condition.run((), &mut world).unwrap(), "and keep running");
    }
}
