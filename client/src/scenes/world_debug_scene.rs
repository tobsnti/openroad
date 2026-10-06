//! `SCENE=world_debug`: terrain and a fly camera, nothing else.
//!
//! Idea: `RenderDebugSettings` toggles (`render_terrain`, `render_objects`,
//! `render_water`, `play_animations`, `render_effects`, ...) only hide
//! already-spawned entities — the streaming/culling systems that own them
//! keep running regardless (see `dev/render_debug.rs::on_settings_changed`,
//! which is a `Visibility` flip and nothing more). So a reading taken with
//! all of them off still pays nearly the full cost of everything it claims
//! to have turned off, which is no good as a rendering-floor baseline. This
//! scene is the real floor: it reuses `world_scene`'s terrain preload and
//! origin-anchoring, skips the player/HUD entirely, opts terrain streaming
//! out of map-object spawning (see the `SceneState::WorldDebug` check in
//! `plugins::map::mod.rs`), turns the whole `EnvironmentPlugin` day/night
//! tint cycle off (see the `SceneState::WorldDebug` check in
//! `plugins::environment::mod.rs`), and strips the skybox/celestials/fog
//! that spawn regardless of scene (`skybox.rs`/`environment::celestial`'s
//! setup both run on `GameState::Game`, and `dev/render_debug.rs` inserts fog
//! on every camera on `RenderDebugSettings` init) since none of the three
//! have a scene-scoped opt-out today.

use bevy::camera_controller::free_camera::FreeCameraPlugin;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImagePlugin, ImageSamplerDescriptor};
use bevy::light::light_consts;
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use bevy::time::common_conditions::on_timer;
use bevy::window::{Window, WindowPlugin};
use bevy_asset_loader::prelude::{
    ConfigureLoadingState, LoadingState, LoadingStateAppExt, LoadingStateConfig,
};
use iyes_progress::ProgressPlugin;
use std::path::PathBuf;
use std::time::Duration;

use crate::assets::m::block_splat_material::TerrainAmbientRatioPlugin;
use crate::assets::m::block_splat_material::TerrainBlockSplatMaterial;
use crate::assets::SroAssetStructsPlugin;
use crate::plugins::assets::sro::SroAssetPlugin;
use crate::plugins::camera::{
    despawn_cinematic_camera, spawn_fly_camera, spawn_terrain_benchmark_camera, DebugCamera,
};
use crate::plugins::config::graphics::TerrainPipeline;
use crate::plugins::config::ClientConfig;
use crate::plugins::environment::celestial::CelestialBody;
use crate::plugins::map::assets::{MapsAssets, TileAssets};
use crate::plugins::map::terrain::{
    load_terrain_dynamically, load_terrain_system, TerrainLoadState, TerrainOnlyBenchmark,
};
use crate::plugins::skybox::Skybox;
use crate::plugins::world_origin::WorldOrigin;
use crate::scenes::world_scene::{preload_starting_area, set_origin_to_spawn_point};
use crate::scenes::SceneState;
use crate::GameState;

pub struct WorldDebugScenePlugin;

/// Run `SCENE=world_debug` as a process-level terrain benchmark rather than as
/// one state inside the normal client.  The normal scene manager registers
/// every client subsystem even when a scene never spawns its entities; this
/// entry point intentionally does not register those systems at all.
pub fn run_terrain_benchmark(config: ClientConfig, assets_dir: PathBuf) {
    let material_defaults = config.graphics.to_material_defaults();
    let present_mode = config.window_settings.present_mode.to_present_mode();
    let desired_maximum_frame_latency = config.window_settings.frame_latency();
    // the same draw-path choice the full client makes in MapPlugin
    let terrain_pipeline = config.graphics.terrain.pipeline;

    App::new()
        .insert_resource(config)
        .insert_resource(material_defaults)
        .add_plugins((
            SroAssetPlugin,
            DefaultPlugins
                .build()
                .set(bevy::asset::AssetPlugin {
                    file_path: assets_dir.to_string_lossy().into_owned(),
                    watch_for_changes_override: Some(cfg!(debug_assertions)),
                    ..default()
                })
                .set(ImagePlugin {
                    default_sampler: ImageSamplerDescriptor {
                        min_filter: ImageFilterMode::Linear,
                        mag_filter: ImageFilterMode::Linear,
                        mipmap_filter: ImageFilterMode::Linear,
                        address_mode_u: ImageAddressMode::Repeat,
                        address_mode_v: ImageAddressMode::Repeat,
                        address_mode_w: ImageAddressMode::Repeat,
                        anisotropy_clamp: 4,
                        ..default()
                    },
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        present_mode,
                        desired_maximum_frame_latency,
                        ..default()
                    }),
                    ..default()
                }),
        ))
        .init_state::<GameState>()
        .add_plugins((
            SroAssetStructsPlugin,
            TerrainBenchmarkPlugin { terrain_pipeline },
            FreeCameraPlugin,
            FrameTimeDiagnosticsPlugin::default(),
        ))
        .run();
}

/// The smallest OpenRoad application which still uses its real terrain
/// loading, mesh-merging and splat-material path.  It is deliberately separate
/// from `MapPlugin`: that plugin also installs foliage, water, sky, environment
/// and map-object systems needed by a playable world but not by this benchmark.
struct TerrainBenchmarkPlugin {
    terrain_pipeline: TerrainPipeline,
}

impl Plugin for TerrainBenchmarkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldOrigin>()
            .init_resource::<TerrainOnlyBenchmark>()
            .add_loading_state(LoadingState::new(GameState::Loading))
            .configure_loading_state(
                LoadingStateConfig::new(GameState::Loading)
                    .load_collection::<TileAssets>()
                    .load_collection::<MapsAssets>(),
            )
            .add_plugins(
                ProgressPlugin::<GameState>::new()
                    .with_state_transition(GameState::Loading, GameState::Game),
            )
            .add_plugins(TerrainAmbientRatioPlugin)
            .add_systems(
                OnEnter(GameState::Game),
                (
                    set_origin_to_spawn_point,
                    spawn_terrain_benchmark_camera,
                    preload_starting_area,
                    setup_terrain_benchmark_lighting,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    load_terrain_system.run_if(any_with_component::<TerrainLoadState>),
                    load_terrain_dynamically,
                )
                    .chain()
                    .run_if(in_state(GameState::Game)),
            )
            .add_systems(
                Update,
                log_terrain_benchmark_fps
                    .run_if(in_state(GameState::Game))
                    .run_if(on_timer(Duration::from_secs(1))),
            )
            .add_systems(Update, log_terrain_benchmark_adapter);

        app.insert_resource(self.terrain_pipeline)
            .add_plugins(bevy::pbr::MaterialPlugin::<TerrainBlockSplatMaterial>::default());
        if self.terrain_pipeline == TerrainPipeline::HandRolled {
            app.add_plugins(crate::plugins::map::terrain::render::TerrainRenderPipelinePlugin);
        }
        if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render_app.insert_resource(self.terrain_pipeline);
        }
    }
}

/// Terrain's splat material is a lit PBR material, so retain a single direct
/// light and ambient term.  Shadows, sky/environment updates and every other
/// world visual are deliberately absent.
fn setup_terrain_benchmark_lighting(mut commands: Commands) {
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 100.0,
        ..default()
    });
    commands.insert_resource(ClearColor(Color::srgb(0.5, 0.6, 0.7)));
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: false,
            illuminance: light_consts::lux::AMBIENT_DAYLIGHT,
            ..default()
        },
        Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::PI / 4.0)),
        Name::from("Terrain benchmark light"),
    ));
}

fn log_terrain_benchmark_fps(diagnostics: Res<DiagnosticsStore>) {
    if let Some(fps) = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
    {
        info!("terrain_benchmark fps: {fps:.1}");
    }
}

fn log_terrain_benchmark_adapter(adapter: Option<Res<RenderAdapterInfo>>, mut logged: Local<bool>) {
    if *logged {
        return;
    }
    if let Some(adapter) = adapter {
        info!(
            "terrain_benchmark render backend: {:?} ({})",
            adapter.backend, adapter.name
        );
        *logged = true;
    }
}

impl Plugin for WorldDebugScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(SceneState::WorldDebug),
            (
                set_origin_to_spawn_point,
                spawn_fly_camera,
                preload_starting_area,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (despawn_skybox, despawn_celestials, strip_distance_fog)
                .run_if(in_state(SceneState::WorldDebug)),
        )
        .add_systems(
            Update,
            log_fps
                .run_if(in_state(SceneState::WorldDebug))
                .run_if(on_timer(Duration::from_secs(1))),
        )
        .add_systems(
            OnExit(SceneState::WorldDebug),
            despawn_cinematic_camera::<DebugCamera>,
        );
    }
}

/// This scene has no UI camera by design (see the module doc), so
/// `bevy::dev_tools::fps_overlay::FpsOverlayPlugin`'s text node has nothing to
/// render onto here — it works fine in scenes that do have one. A plain log
/// line needs no camera at all and is grep-able from a scripted capture's
/// output, which is what the rendering-floor comparisons this scene exists
/// for actually need.
fn log_fps(diagnostics: Res<DiagnosticsStore>) {
    if let Some(fps) = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
    {
        info!("world_debug fps: {fps:.1}");
    }
}

/// Skybox spawn is keyed on `GameState::Game` (`skybox.rs`), a state
/// independent of `SceneState` with no ordering guarantee against it, so
/// there is no `OnEnter` hook that can skip it at the source for just this
/// one scene. Reactively despawning it here is correct regardless of which
/// state settles first, and free (empty query) on every frame after the one
/// that catches it.
fn despawn_skybox(mut commands: Commands, skybox: Query<Entity, With<Skybox>>) {
    for entity in &skybox {
        commands.entity(entity).despawn();
    }
}

/// Same reasoning as `despawn_skybox`: the sun/moon discs
/// (`environment::celestial::CelestialPlugin`) also spawn unconditionally on
/// `OnEnter(GameState::Game)`, and their glow is exactly the kind of "tint"
/// this scene wants gone.
fn despawn_celestials(mut commands: Commands, bodies: Query<Entity, With<CelestialBody>>) {
    for entity in &bodies {
        commands.entity(entity).despawn();
    }
}

/// Same reasoning as `despawn_skybox`: `dev/render_debug.rs::on_settings_changed`
/// inserts `DistanceFog` on every camera as soon as `RenderDebugSettings`
/// exists (`enable_fog` defaults to `true`), regardless of scene.
fn strip_distance_fog(
    mut commands: Commands,
    fogged_cameras: Query<Entity, (With<Camera3d>, With<DistanceFog>)>,
) {
    for entity in &fogged_cameras {
        commands.entity(entity).remove::<DistanceFog>();
    }
}
