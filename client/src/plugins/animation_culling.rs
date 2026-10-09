//! Distance-gates skeletal animation, the sibling of the effects module's
//! `cull_effect_simulation`: animated map props (torches, banners) exist out
//! to the terrain unload boundary, far past the fog, and Bevy's
//! `animate_targets` samples and rewrites every bone `Transform` of every
//! `AnimationPlayer` each frame with no visibility filter — pausing the
//! player does NOT stop that (paused only freezes the clock in
//! `advance_animations`). What does stop it: `animate_targets` gives up on a
//! target whose root has no `AnimationGraphHandle`, before any graph or
//! curve work. So the gate simply removes the handle from far roots
//! (stashing it to keep the graph asset alive) and re-inserts it on
//! approach; `AnimationPlayer` state is untouched, so playback resumes
//! exactly where it froze.
//!
//! The render-debug panel's `play_animations` switch rides the same gate:
//! off stashes every root's handle regardless of distance (exempt ones
//! included), on hands control back to the distance logic.
//!
//! Distance is measured from the *active* window camera (`main_world_camera`,
//! shared with the effect culling), never from an arbitrary `Camera3d`: the
//! intro keeps a disabled cinematic camera alive far outside the shifted
//! world origin, and the HUD portrait / paper-doll cameras render offscreen.

use bevy::animation::graph::AnimationGraphHandle;
use bevy::animation::AnimationPlayer;
use bevy::app::{App, Plugin, Update};
use bevy::camera::visibility::ViewVisibility;
use bevy::camera::{Camera, Camera3d, RenderTarget};
use bevy::ecs::entity::EntityHashMap;
use bevy::light::DirectionalLight;
use bevy::prelude::{
    ChildOf, Commands, Component, Entity, GlobalTransform, Has, Local, Query, Res, Time, With,
};

use crate::assets::o2::MapObject;
use crate::commands::{AnimationLibrary, MeshIndexMap};
use crate::plugins::dev::render_debug::RenderDebugSettings;
use crate::plugins::effects::systems::main_world_camera;
use crate::plugins::map::terrain::REGION_SIZE;
use crate::plugins::map::view_range::ViewRange;

/// Stashed graph handle of a distance-gated animated root. Holding the
/// strong `Handle<AnimationGraph>` keeps the graph asset alive while gated.
#[derive(Component)]
pub struct PausedAnimationGraph(pub AnimationGraphHandle);

/// Never distance-gate this animated root. The player's body wrapper needs
/// it: `update_player_animation` holds `&mut AnimationGraphHandle` on it and
/// would silently stop matching if a free-flying dev camera gated it. The
/// panel's `play_animations` switch still gates it — deliberately, unlike
/// the accidental distance case; the walk/stand state re-syncs on resume.
#[derive(Component)]
pub struct AnimationCullExempt;

/// Swaps `AnimationGraphHandle` out of/into animated roots by camera
/// distance, with the same fully-fogged threshold and hysteresis as
/// `cull_effect_simulation`. Scoped to `AnimationLibrary` roots (resources
/// spawned by `SpawnResource` — map props, characters); hand-built rigs in
/// test scenes are untouched.
///
/// Map props are also gated while off screen: none of their meshes visible
/// for [`OFFSCREEN_GRACE_SECS`], resumed the frame one is in view again. At
/// the Jangan West waterfall 271 of 286 animating roots — 1,554 of 1,668
/// animated bones — were swaying trees, flowers and lamps outside the view
/// (BRP count, 2026-10-05). Characters stay distance-gated only: their
/// clock freezing behind the camera would replay a one-shot (an attack, a
/// death) late once turned to. Not while a directional light renders
/// shadow maps either — an off-screen prop's shadow can be in view.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn cull_distant_animations(
    mut commands: Commands,
    time: Res<Time>,
    settings: Res<RenderDebugSettings>,
    view: Res<ViewRange>,
    config: Option<Res<crate::plugins::config::ClientConfig>>,
    cameras: Query<(&Camera, &RenderTarget, &GlobalTransform), With<Camera3d>>,
    roots: Query<
        (
            Entity,
            &GlobalTransform,
            Option<&AnimationGraphHandle>,
            Option<&PausedAnimationGraph>,
            Has<AnimationCullExempt>,
            Option<&MeshIndexMap>,
        ),
        (With<AnimationPlayer>, With<AnimationLibrary>),
    >,
    parents: Query<&ChildOf>,
    placements: Query<(), With<MapObject>>,
    views: Query<&ViewVisibility>,
    lights: Query<&DirectionalLight>,
    mut last_seen: Local<EntityHashMap<f32>>,
) {
    let pause_dist = view.live_cull;
    let resume_dist = pause_dist - REGION_SIZE * 0.25;

    // The gate must measure from the camera the player is actually looking
    // through. `iter().next()` returned an arbitrary `Camera3d`, and several
    // live at once: the intro keeps its cinematic camera alive (disabled)
    // across state changes while the world origin moves to the char-select
    // anchor, leaving that camera ~200k units from the stage, and the HUD
    // portrait / inventory paper-doll cameras render into offscreen images.
    // Any of those is far past `pause_dist`, so every animated root — the
    // char-select lobby and the character-create preview included — had its
    // graph handle stashed and froze (#641). `main_world_camera` is the same
    // filter the sibling effect culling already uses.
    let Some(camera) = main_world_camera(&cameras) else {
        return;
    };
    let camera_pos = camera.translation();

    let now = time.elapsed_secs();
    let animate_props = config.is_none_or(|config| config.graphics.objects.animate);
    let shadows = lights.iter().any(|light| light.shadow_maps_enabled);
    last_seen.retain(|root, _| roots.contains(*root));

    for (root, global, graph, stashed, exempt, meshes) in &roots {
        let (pause, resume) = if !settings.play_animations {
            (true, false)
        } else if exempt {
            (false, true)
        } else if !animate_props && is_map_prop(root, &parents, &placements) {
            // `graphics.objects.animate` (Dynamic Animation) off: map props
            // hold still wherever they are; characters are unaffected
            (true, false)
        } else {
            let dist_sq = global.translation().distance_squared(camera_pos);
            let (far, near) = (
                dist_sq > pause_dist * pause_dist,
                dist_sq < resume_dist * resume_dist,
            );
            match meshes.filter(|_| !shadows && is_map_prop(root, &parents, &placements)) {
                Some(meshes) => {
                    let visible = meshes
                        .0
                        .values()
                        .any(|&mesh| views.get(mesh).is_ok_and(|v| v.get()));
                    let seen = last_seen.entry(root).or_insert(now);
                    if visible {
                        *seen = now;
                    }
                    let hidden = now - *seen > OFFSCREEN_GRACE_SECS;
                    (far || hidden, near && visible)
                }
                None => (far, near),
            }
        };
        match (graph, stashed) {
            (Some(graph), None) if pause => {
                commands
                    .entity(root)
                    .insert(PausedAnimationGraph(graph.clone()))
                    .remove::<AnimationGraphHandle>();
            }
            (None, Some(stashed)) if resume => {
                commands
                    .entity(root)
                    .insert(stashed.0.clone())
                    .remove::<PausedAnimationGraph>();
            }
            _ => {}
        }
    }
}

/// How long a map prop must stay entirely off screen before it is paused:
/// long enough that sweeping the camera past props does not toggle them
/// every few frames, short enough to catch everything behind the camera
/// within a moment. Starting value, not a measurement.
pub const OFFSCREEN_GRACE_SECS: f32 = 0.5;

/// A resource spawned for an `.o2` placement: its anchor is the placement
/// itself, or a compound part under it.
fn is_map_prop(
    root: Entity,
    parents: &Query<&ChildOf>,
    placements: &Query<(), With<MapObject>>,
) -> bool {
    parents
        .iter_ancestors(root)
        .take(2)
        .any(|ancestor| placements.contains(ancestor))
}

pub struct AnimationCullingPlugin;

impl Plugin for AnimationCullingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewRange>()
            .add_systems(Update, cull_distant_animations);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::animation::graph::AnimationGraph;
    use bevy::asset::Assets;
    use bevy::camera::RenderTarget;
    use bevy::ecs::system::{RunSystemOnce, System};
    use bevy::math::Vec3;
    use bevy::prelude::IntoSystem;
    use bevy::prelude::World;
    use bevy::window::WindowRef;

    use crate::commands::AnimationLibrary;

    /// One animated root at the render origin plus the two cameras of the
    /// intro flow: the *stale* one (spawned first, like the cinematic camera
    /// the intro keeps alive) and the active one framing the stage.
    fn world_with(active_pos: Vec3) -> (World, Entity) {
        let mut world = World::new();
        world.init_resource::<RenderDebugSettings>();
        world.init_resource::<ViewRange>();
        world.init_resource::<Assets<AnimationGraph>>();
        world.init_resource::<Time>();
        let graph = world
            .resource_mut::<Assets<AnimationGraph>>()
            .add(AnimationGraph::new());

        // Spawned FIRST, so an unfiltered `iter().next()` picks it: the
        // intro's disabled cinematic camera, left ~200k units away by the
        // world-origin shift to the char-select anchor.
        world.spawn((
            Camera3d::default(),
            Camera {
                is_active: false,
                ..Default::default()
            },
            RenderTarget::Window(WindowRef::Primary),
            GlobalTransform::from_translation(Vec3::new(-205_440.0, 0.0, 19_200.0)),
        ));
        world.spawn((
            Camera3d::default(),
            Camera {
                is_active: true,
                ..Default::default()
            },
            RenderTarget::Window(WindowRef::Primary),
            GlobalTransform::from_translation(active_pos),
        ));

        let root = world
            .spawn((
                GlobalTransform::from_translation(Vec3::ZERO),
                AnimationPlayer::default(),
                AnimationLibrary {
                    entries: Vec::new(),
                },
                AnimationGraphHandle(graph),
            ))
            .id();
        (world, root)
    }

    /// #641: the character-create preview (and the char-select lobby) stood
    /// frozen because the gate measured from an arbitrary `Camera3d`. With a
    /// disabled far camera present, the active one decides.
    #[test]
    fn the_disabled_far_camera_no_longer_freezes_the_animated_root() {
        let (mut world, root) = world_with(Vec3::new(0.0, 0.0, 40.0));
        world
            .run_system_once(cull_distant_animations)
            .expect("system runs and its commands are applied");
        assert!(
            world.get::<AnimationGraphHandle>(root).is_some(),
            "the root keeps its graph handle: the ACTIVE camera is 40 units away"
        );
        assert!(
            world.get::<PausedAnimationGraph>(root).is_none(),
            "and nothing was stashed"
        );
    }

    /// Control: the gate itself still works — a far *active* camera pauses.
    #[test]
    fn a_far_active_camera_still_pauses_the_animated_root() {
        let (mut world, root) = world_with(Vec3::new(0.0, 0.0, 200_000.0));
        world
            .run_system_once(cull_distant_animations)
            .expect("system runs and its commands are applied");
        assert!(
            world.get::<AnimationGraphHandle>(root).is_none(),
            "beyond the fully-fogged distance the handle is stashed"
        );
        assert!(world.get::<PausedAnimationGraph>(root).is_some());
    }

    /// A map prop near the camera: its root is anchored on an `.o2`
    /// placement and owns one mesh whose `ViewVisibility` the test controls.
    fn prop_world() -> (World, Entity, Entity) {
        let (mut world, root) = world_with(Vec3::new(0.0, 0.0, 40.0));
        let placement = world
            .spawn(MapObject {
                id: 0,
                position: Vec3::ZERO,
                is_static: true,
                yaw: 0.0,
                uid: 0,
                short_0: 0,
                is_big: false,
                is_struct: false,
                region_id: 0,
            })
            .id();
        let mesh = world.spawn(ViewVisibility::HIDDEN).id();
        world.entity_mut(root).insert((
            ChildOf(placement),
            MeshIndexMap([(0, mesh)].into_iter().collect()),
        ));
        (world, root, mesh)
    }

    fn step(world: &mut World, system: &mut impl System<In = (), Out = ()>, secs: f32) {
        world
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(secs));
        system.run((), world).unwrap();
        system.apply_deferred(world);
    }

    #[test]
    fn an_offscreen_prop_pauses_after_the_grace_and_resumes_in_view() {
        let (mut world, root, mesh) = prop_world();
        let mut system = IntoSystem::into_system(cull_distant_animations);
        system.initialize(&mut world);

        step(&mut world, &mut system, 0.0);
        assert!(
            world.get::<AnimationGraphHandle>(root).is_some(),
            "just out of view: still within the grace"
        );
        step(&mut world, &mut system, OFFSCREEN_GRACE_SECS + 0.1);
        assert!(
            world.get::<PausedAnimationGraph>(root).is_some(),
            "off screen past the grace: paused"
        );

        world.entity_mut(mesh).insert(ViewVisibility::VISIBLE);
        step(&mut world, &mut system, 0.016);
        assert!(
            world.get::<AnimationGraphHandle>(root).is_some(),
            "a mesh in view resumes it the next frame"
        );
    }

    #[test]
    fn characters_and_shadowed_scenes_are_not_offscreen_gated() {
        // not anchored on a placement: a character
        let (mut world, root) = world_with(Vec3::new(0.0, 0.0, 40.0));
        let mesh = world.spawn(ViewVisibility::HIDDEN).id();
        world
            .entity_mut(root)
            .insert(MeshIndexMap([(0, mesh)].into_iter().collect()));
        let mut system = IntoSystem::into_system(cull_distant_animations);
        system.initialize(&mut world);
        step(&mut world, &mut system, 0.0);
        step(&mut world, &mut system, OFFSCREEN_GRACE_SECS + 0.1);
        assert!(world.get::<AnimationGraphHandle>(root).is_some());

        // a prop, but a light renders shadow maps
        let (mut world, root, _) = prop_world();
        world.spawn(DirectionalLight {
            shadow_maps_enabled: true,
            ..Default::default()
        });
        let mut system = IntoSystem::into_system(cull_distant_animations);
        system.initialize(&mut world);
        step(&mut world, &mut system, 0.0);
        step(&mut world, &mut system, OFFSCREEN_GRACE_SECS + 0.1);
        assert!(world.get::<AnimationGraphHandle>(root).is_some());
    }
}
