// Idea: the original's lens flare (its Lens Flare option) composited from the user's own
// `Map.pk2` art. `sun/lens1`–`lens8` are the flare sprites; `lens3` is the sun disc the
// celestial module already draws, and this module adds the other seven. Each one is an
// unlit, additively blended quad parented to the main camera, two units in front of it,
// so it sits over the scene and follows the camera exactly with no per-frame world math.
// Each frame the sun's screen position S is projected, and the sprites are laid out on the
// line from S through the screen centre and beyond, the classic flare-chain look. The
// chain fades with distance from the centre and fades out when terrain or a building
// blocks the sun. That test is a ray cast against the loaded nav meshes, run at 10 Hz,
// since the cast walks every loaded region and object. Nav meshes cover terrain and
// walkable structures only, so a prop without one (a statue, a tree) does not hide
// the flare.
//
// The original's own positions, sizes and fade live in its code, so the layout table
// below is ours (ADR 0009): a deliberate, stated choice built on the authored art.

use std::time::Duration;

use bevy::camera::RenderTarget;
use bevy::light::NotShadowCaster;
use bevy::math::Ray3d;
use bevy::prelude::*;

use super::celestial::sun_direction;
use super::TimeOfDay;
use crate::plugins::config::ClientConfig;
use crate::plugins::nav::NavMeshRaycast;
use crate::GameState;

/// The flare chain: texture, position on the line from the sun (0) through
/// the screen centre (1) and beyond, and size as a fraction of the screen
/// height. Ours, see the module docs. `lens4` is left out: its background is
/// not black, so additive blending shows it as a pale square.
const ELEMENTS: [(&str, f32, f32); 6] = [
    ("map://sun/lens1.ddj", 0.0, 0.32),
    ("map://sun/lens2.ddj", 0.35, 0.12),
    ("map://sun/lens5.ddj", 0.85, 0.05),
    ("map://sun/lens6.ddj", 1.2, 0.06),
    ("map://sun/lens7.ddj", 1.5, 0.10),
    ("map://sun/lens8.ddj", 1.9, 0.16),
];
/// How far in front of the camera the sprites sit: past the near plane (1),
/// closer than anything in the world, so nothing draws over them.
const FLARE_DISTANCE: f32 = 2.0;
/// Additive brightness at full strength. The sprites are dim greys meant to
/// be added onto a bright sky.
const FLARE_BRIGHTNESS: f32 = 0.8;
/// Time constant of the fade in and out, in seconds.
const FADE_SECONDS: f32 = 0.15;
/// How often the sun's occlusion is re-tested.
const OCCLUSION_INTERVAL: Duration = Duration::from_millis(100);
/// Occluders farther than this are ignored: past the fog nothing is drawn
/// to block the sun.
const OCCLUSION_RANGE: f32 = 6_000.0;

pub struct LensFlarePlugin;

impl Plugin for LensFlarePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FlareState>()
            .add_systems(OnEnter(GameState::Game), spawn_lens_flare)
            .add_systems(Update, update_lens_flare.run_if(in_state(GameState::Game)));
    }
}

/// One sprite of the chain, by index into [`ELEMENTS`].
#[derive(Component)]
struct FlareElement(usize);

#[derive(Resource, Default)]
struct FlareState {
    materials: Vec<Handle<StandardMaterial>>,
    /// Current, faded strength 0..1.
    strength: f32,
    /// Strength last written into the materials.
    applied: f32,
    occluded: bool,
    since_occlusion_test: Duration,
}

fn spawn_lens_flare(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    mut state: ResMut<FlareState>,
    existing: Query<(), With<FlareElement>>,
) {
    if !existing.is_empty() {
        return;
    }
    let quad = meshes.add(Mesh::from(Rectangle::from_size(Vec2::ONE)));
    state.materials.clear();
    for (index, (path, _, _)) in ELEMENTS.iter().enumerate() {
        let material = materials.add(StandardMaterial {
            base_color: Color::BLACK,
            base_color_texture: Some(asset_server.load(*path)),
            unlit: true,
            alpha_mode: AlphaMode::Add,
            cull_mode: None,
            fog_enabled: false,
            ..default()
        });
        state.materials.push(material.clone());
        commands.spawn((
            Mesh3d(quad.clone()),
            MeshMaterial3d(material),
            Transform::default(),
            Visibility::Hidden,
            NotShadowCaster,
            FlareElement(index),
            Name::from(format!("Lens flare {}", index + 1)),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn update_lens_flare(
    mut commands: Commands,
    time: Res<Time>,
    tod: Res<TimeOfDay>,
    config: Res<ClientConfig>,
    cameras: Query<
        (
            Entity,
            &Camera,
            &RenderTarget,
            &GlobalTransform,
            &Projection,
        ),
        With<Camera3d>,
    >,
    mut elements: Query<(
        Entity,
        &FlareElement,
        &mut Transform,
        &mut Visibility,
        Option<&ChildOf>,
    )>,
    mut state: ResMut<FlareState>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    nav: NavMeshRaycast,
) {
    let Some((camera_entity, camera, camera_global, projection)) = cameras
        .iter()
        .find(|(_, camera, target, _, _)| {
            camera.is_active && matches!(target, RenderTarget::Window(_))
        })
        .map(|(entity, camera, _, global, projection)| (entity, camera, global, projection))
    else {
        return;
    };
    let Projection::Perspective(perspective) = projection else {
        return;
    };

    // Where the sun is on screen, if it is up and in view at all.
    let sun_dir = sun_direction(tod.t);
    let camera_pos = camera_global.translation();
    let sun_ndc = (config.graphics.lens_flare && sun_dir.y > 0.0)
        .then(|| camera.world_to_ndc(camera_global, camera_pos + sun_dir * 10_000.0))
        .flatten()
        .filter(|ndc| ndc.z > 0.0 && ndc.x.abs() < 1.2 && ndc.y.abs() < 1.2)
        .map(|ndc| ndc.truncate());

    state.since_occlusion_test += time.delta();
    if sun_ndc.is_some() && state.since_occlusion_test >= OCCLUSION_INTERVAL {
        state.since_occlusion_test = Duration::ZERO;
        let ray = Ray3d::new(camera_pos, Dir3::new(sun_dir).unwrap_or(Dir3::Y));
        state.occluded = nav
            .cast(&ray)
            .is_some_and(|hit| hit.distance < OCCLUSION_RANGE);
    }

    // Strongest with the sun at the screen centre, gone at the edges.
    let target = match sun_ndc {
        Some(ndc) if !state.occluded => (1.0 - ndc.length() / 1.4).clamp(0.0, 1.0),
        _ => 0.0,
    };
    let blend = 1.0 - (-time.delta_secs() / FADE_SECONDS).exp();
    state.strength += (target - state.strength) * blend;
    let visible = state.strength > 0.01;

    if (state.strength - state.applied).abs() > 0.02 || (!visible && state.applied != 0.0) {
        let strength = if visible { state.strength } else { 0.0 };
        state.applied = strength;
        for handle in &state.materials {
            if let Some(mut material) = materials.get_mut(handle) {
                material.base_color =
                    Color::LinearRgba(LinearRgba::WHITE * (FLARE_BRIGHTNESS * strength));
            }
        }
    }

    // Lay the chain out in the camera's own space: half the view's height
    // and width at FLARE_DISTANCE, from the projection.
    let half_height = FLARE_DISTANCE * (perspective.fov * 0.5).tan();
    let half_width = half_height * perspective.aspect_ratio;
    let sun = sun_ndc.unwrap_or(Vec2::ZERO);
    for (entity, element, mut transform, mut visibility, parent) in &mut elements {
        if parent.map(ChildOf::parent) != Some(camera_entity) {
            commands.entity(entity).insert(ChildOf(camera_entity));
        }
        visibility.set_if_neq(if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });
        if !visible {
            continue;
        }
        let (_, along, size) = ELEMENTS[element.0];
        let point = sun * (1.0 - along);
        transform.translation =
            Vec3::new(point.x * half_width, point.y * half_height, -FLARE_DISTANCE);
        transform.scale = Vec3::splat(size * 2.0 * half_height);
    }
}
