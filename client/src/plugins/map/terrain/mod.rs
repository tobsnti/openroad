use std::collections::HashMap;
use std::fmt::{Formatter, LowerHex, UpperHex};

use bevy::asset::RenderAssetUsages;
use bevy::asset::{Assets, Handle, LoadState};
use bevy::camera::Camera3d;
use bevy::log::warn;
use bevy::math::Vec3;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::Name;
use bevy::prelude::*;

use crate::assets::m::block_mesh::{merge_block_meshes, merge_block_meshes_lod};
use crate::assets::m::block_splat_material::TerrainLightmapFallback;
use crate::assets::m::block_splat_material::{TerrainBlockSplatMaterial, TerrainGroundTextures};
use crate::assets::m::{TerrainBlock, WaterType, JMXVMAPM};
use crate::assets::mfo::JMXVMFO;
use crate::assets::nvm::JMXVNVM;
use crate::assets::o2::JMXVMAPO2;
use crate::assets::t::JMXVMAPT;
use crate::plugins::asset_residency::AssetResidency;
use crate::plugins::config::graphics::TerrainPipeline;
use crate::plugins::cursor::interactions::GameCursorTarget;
use crate::plugins::dev::aabb_lines::DebugAabb;
use crate::plugins::map::assets::MapsAssets;
use crate::plugins::map::view_range::TerrainLodLevel;
use crate::plugins::map::water_hq_material::HighQualityWaterMaterial;
use crate::plugins::world_origin::WorldOrigin;
use crate::util::mesh::needs_winding_reversal;
use crate::util::region::RegionIdExt;

pub mod render;
pub mod rendering;

/// World-space size of one terrain region/tile (matches the .m/.o2/.nvm grid step).
pub const REGION_SIZE: f32 = 1920.0;
/// Extra rings of regions kept loaded (but already fully hidden by fog) beyond
/// the view distance (`graphics.view`, `ViewRange::load_ring`) before actually being
/// despawned — and, read the other way around, how far ahead of the fully-fogged
/// point a region is spawned.
///
/// This needs to be more than one region: fog opacity is a smooth per-pixel distance
/// from the camera's exact position, so it can reach full opacity partway *through*
/// a region rather than exactly on its boundary (e.g. observed to bottom out around
/// the middle of a region rather than at its far edge). If region `n` is where fog
/// hits full opacity, region `n + 1` must already be loaded by then — a single ring
/// of margin only guarantees `n`. On top of that, map objects (props, mountains,
/// buildings) go through their own multi-step async load chain once their region
/// spawns (see `load_terrain_objects_system`/`load_compound_system`/
/// `load_resources_system` in objects.rs), which takes additional time beyond the
/// region's own mesh load. Extra rings buy that pipeline more real time to finish
/// before the fog clears enough to reveal it.
///
/// Lowered from 3 to 1 (the documented minimum above) as a stopgap while the real fix —
/// merging terrain blocks into far fewer draw calls, see `load_terrain_system` — lands.
/// Watch for object/prop pop-in near the fog boundary if this needs to go back up.
pub const UNLOAD_BUFFER: i32 = 1;

/// Streaming hysteresis: a region is only despawned once it is this many rings
/// *beyond* the ring regions are loaded out to. With the two rings equal (as
/// they were), a camera hovering on a region line despawned a whole row and
/// rebuilt it — re-decoding its `.m/.t/.o2/.nvm`, re-merging its meshes and
/// respawning its map objects — every time it crossed back; a trace showed
/// lightmaps decoded twice within 200 ms. One ring is enough to absorb that
/// back-and-forth, and costs no draws: the lingering ring is past the fog and
/// already hidden by `region_visibility`. It does keep up to one extra row of
/// regions resident behind a moving camera.
pub const UNLOAD_HYSTERESIS: i32 = 1;

/// Whether region `(t_x, t_z)` lies outside the inclusive ring
/// `[min_x, max_x] x [min_z, max_z]` grown by `pad` regions on every side.
fn outside_ring(
    (t_x, t_z): (i32, i32),
    (min_x, min_z): (i32, i32),
    (max_x, max_z): (i32, i32),
    pad: i32,
) -> bool {
    t_x < min_x - pad || t_x > max_x + pad || t_z < min_z - pad || t_z > max_z + pad
}

/// Extra slack past the fog end before a region root is hidden outright:
/// covers map-object meshes that overhang their region's 1920x1920 footprint
/// toward the camera (a pixel at fog_end - m is only m/1920 visible through
/// the fog; half a block of margin makes that ~0).
const REGION_HIDE_MARGIN: f32 = 160.0;

/// Region-root visibility for region `(t_x, t_z)` given the camera's
/// SRO-space position: `Hidden` when the nearest point of the region's XZ
/// footprint lies past `cull` — the cull distance, by default where the fog
/// is fully opaque (`ViewRange::live_cull`; env profiles only ever pull fog
/// closer, never past it — see `envi_fog_range`). Regions stay *loaded* out
/// to the unload boundary so streaming keeps its fog-covered head start, but
/// everything fully behind the fog stops being rendered instead of being
/// drawn and then erased by per-pixel fog. The per-pixel fog distance is
/// 3D >= this XZ distance, so the test is conservative. `None` = the camera
/// carries no `DistanceFog` (the render-debug panel can remove it) — without
/// fog there is nothing to hide behind.
fn region_visibility(cam_sro: Vec3, t_x: i32, t_z: i32, cull: Option<f32>) -> Visibility {
    let Some(cull) = cull else {
        return Visibility::default();
    };
    let hide_dist = cull + REGION_HIDE_MARGIN;
    // SRO-space footprint: region x covers world x in [-(t_x+1), -t_x] * REGION_SIZE
    // (world X is mirrored), region z covers [t_z, t_z+1] * REGION_SIZE.
    let x_max = t_x as f32 * -REGION_SIZE;
    let x_min = x_max - REGION_SIZE;
    let z_min = t_z as f32 * REGION_SIZE;
    let z_max = z_min + REGION_SIZE;
    let dx = cam_sro.x - cam_sro.x.clamp(x_min, x_max);
    let dz = cam_sro.z - cam_sro.z.clamp(z_min, z_max);
    if dx * dx + dz * dz > hide_dist * hide_dist {
        Visibility::Hidden
    } else {
        Visibility::default()
    }
}

/// Fixed block-grid size of a region (see `JMXVMAPM`'s parser: always `for z_block in 0..6
/// { for x_block in 0..6 { ... } }`). Every region merges its whole grid into one mesh and one
/// material: the ground-tile atlas is indexed by tile id globally, so there is no per-group
/// texture budget to blow and no reason to split a region into smaller patches (which is what
/// the old `region_group_size` fallback existed for — see `TerrainTileAtlas`).
const REGION_BLOCKS_PER_SIDE: i32 = 6;

#[derive(Resource)]
pub struct TerrainMesh(pub Handle<Mesh>);

/// Marks the process-level terrain benchmark.  It keeps the exact terrain
/// mesh and splat-material construction path, but skips child entities used
/// only by the full client (picking/debug metadata and water surfaces).
#[derive(Resource, Default)]
pub struct TerrainOnlyBenchmark;

/// Default/high graphics tier — see `water_hq_material.rs` for what it adds over plain
/// transmissive glass. Plain water/ice surfaces otherwise use a flat `StandardMaterial`
/// on the shared `water_patch_mesh()` plane (see `setup_terrain_mesh`).
#[derive(Resource)]
pub struct WaterNormalMaterial(pub Handle<HighQualityWaterMaterial>);
/// Low graphics tier (`graphics.water.quality: low`). Exactly one of this and
/// [`WaterNormalMaterial`] is inserted by `setup_terrain_mesh`, and
/// `load_terrain_system` spawns whichever it finds — so only one water draw
/// path is ever live.
#[derive(Resource)]
pub struct WaterLowMaterial(
    pub Handle<crate::plugins::map::water_material::LowQualityWaterMaterial>,
);
#[derive(Resource)]
pub struct WaterIceMaterial(pub Handle<StandardMaterial>);

/// Any streamed water or ice surface, whichever material tier it carries.
///
/// The three spawn sites below differ only in material type — HQ water, low water and ice
/// — so anything that wants to address "the water" as a category (the `world_counts/water`
/// diagnostic, the `render_water` debug toggle) would otherwise have to name all three and
/// be re-taught every time a tier is added. The marker is the category; the material is an
/// implementation detail of the tier.
#[derive(Component)]
pub struct WaterPlane;

#[derive(Bundle)]
pub struct TerrainBundle {
    terrain: Terrain,
    terrain_load_state: TerrainLoadState,
    map_data: TerrainMeshData,
    lightmap_data: TerrainLightmapData,
    object_data: TerrainObjectData,
    nav_mesh: TerrainNavMeshData,
    transform: Transform,
    visibility: Visibility,
}

#[derive(Component)]
pub struct TerrainMeshData(pub(crate) Handle<JMXVMAPM>);

/// Baked terrain lightmap for this region (JMXVMAPT `.t`, see `assets/t.rs`). Loaded alongside the
/// `.m` heightmap; its decoded DDS texture modulates terrain color in the splat shader. Regions
/// without a `.t` file (or that fail to decode) simply carry no lightmap and render unmodulated.
#[derive(Component)]
pub struct TerrainLightmapData(pub(crate) Handle<JMXVMAPT>);

#[derive(Component)]
pub struct TerrainObjectData(pub(crate) Handle<JMXVMAPO2>);

#[derive(Component)]
pub struct TerrainNavMeshData(pub(crate) Handle<JMXVNVM>);

#[derive(Component, Reflect, Default)]
pub enum TerrainLoadState {
    #[default]
    None,
    /// Ground merge groups built so far. Group builds are budgeted per frame
    /// (see `load_terrain_system`), so a region under construction parks its
    /// progress here between frames. `group_size` is the merge size in blocks
    /// per side — always `REGION_BLOCKS_PER_SIDE` now, kept in the state so the
    /// resume arithmetic stays explicit rather than assuming a constant.
    BuildingMeshes {
        next_group: i32,
        group_size: i32,
    },
    LoadedMeshes,
    Completed,
}

#[derive(Debug, Reflect)]
pub struct TerrainId(pub u16);

impl LowerHex for TerrainId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:x}", self.0)
    }
}
impl UpperHex for TerrainId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:X}", self.0)
    }
}

impl TerrainId {
    pub fn from_x_z(x: u8, z: u8) -> Self {
        let mut region_id = 0_u16;
        region_id += x as u16;
        region_id += (z as u16) << 8;
        TerrainId(region_id)
    }
    pub fn to_x_z(&self) -> (u8, u8) {
        self.0.to_x_z()
    }
}

#[derive(Component, Reflect)]
pub struct Terrain(pub(crate) TerrainId);

impl Terrain {
    pub fn to_x_z(&self) -> (u8, u8) {
        self.0.to_x_z()
    }
}

/// Marks a terrain region as explicitly preloaded (see `preload_terrain_region`) — exempt
/// from the distance-based despawn in `load_terrain_dynamically`, so it stays resident
/// permanently instead of being torn down and rebuilt from scratch as the camera moves
/// away and back. A prototype for keeping a known area (a starting zone, a boss room)
/// always loaded rather than streaming it like the rest of the world.
#[derive(Component)]
pub struct PreloadedTerrain;

/// Eagerly spawns terrain for every valid region in `[min_x..=max_x] x [min_z..=max_z]`
/// and marks it `PreloadedTerrain`. Skips regions the `.mfo` bitmap marks as not present,
/// same check `load_terrain_dynamically` uses.
pub fn preload_terrain_region(
    commands: &mut Commands,
    asset_server: &AssetServer,
    map_info: &JMXVMFO,
    origin: &WorldOrigin,
    min_x: u8,
    max_x: u8,
    min_z: u8,
    max_z: u8,
) {
    for x in min_x..=max_x {
        for z in min_z..=max_z {
            if !map_info.region_data[x as usize + z as usize * map_info.map_width] {
                continue;
            }
            let terrain_id = TerrainId::from_x_z(x, z);
            let hex_id = format!("{:x}", terrain_id);
            let p = origin.to_render(Vec3::new(
                x as f32 * -REGION_SIZE,
                0.0,
                z as f32 * REGION_SIZE,
            ));
            trace!("Preload TerrainId {:?} ({:?})", terrain_id, hex_id);
            commands
                .spawn(TerrainBundle {
                    terrain: Terrain(terrain_id),
                    map_data: TerrainMeshData(asset_server.load(format!("map://{z}/{x}.m"))),
                    lightmap_data: TerrainLightmapData(
                        asset_server.load(format!("map://{z}/{x}.t")),
                    ),
                    object_data: TerrainObjectData(asset_server.load(format!("map://{z}/{x}.o2"))),
                    terrain_load_state: TerrainLoadState::None,
                    nav_mesh: TerrainNavMeshData(
                        asset_server.load(format!("data://navmesh/nv_{hex_id}.nvm")),
                    ),
                    transform: Transform::from_translation(p),
                    visibility: Visibility::default(),
                })
                .insert(Name::from(format!("Region {}/{} (preloaded)", x, z)))
                .insert(PreloadedTerrain);
        }
    }
}

/// Region despawns budgeted per frame. Crossing a region boundary can flag a
/// whole ring/row of regions out-of-range at once, and `despawn()` recursively
/// tears down each one's entire mesh/object/compound subtree — unthrottled,
/// this was half of the ~208ms region-crossing hitch measured live (the
/// other half was the mirrored unbudgeted spawn burst, see
/// `OBJECT_SPAWNS_PER_FRAME` in `objects.rs`), documented in
/// `docs/perf-remote.md`. `out_of_range` is recomputed fresh from the
/// camera's position every time this system runs, so a region that misses
/// its budget this frame is simply re-evaluated (and despawned once budget
/// allows) the next one — no extra state needed.
pub(crate) const REGION_UNLOADS_PER_FRAME: u32 = 2;

pub fn load_terrain_dynamically(
    // Only the main 3D view cameras (the `Main` render layer) stream and
    // fog-cull the world. The UI camera has no fog, so it would show every
    // region, and the HUD portrait and paper-doll rigs sit somewhere else
    // entirely, so they would stream around the wrong place. With a
    // `Changed<Transform>` filter that never came up, because those cameras
    // do not move. It does now that a view-range change also re-runs this.
    camera_query: Query<
        (
            Ref<Transform>,
            &Camera,
            Has<DistanceFog>,
            // absent = Bevy's default, layer 0, which is `Main` (the
            // terrain benchmark's camera)
            Option<&bevy::camera::visibility::RenderLayers>,
        ),
        With<Camera3d>,
    >,
    view: Res<crate::plugins::map::view_range::ViewRange>,
    config: Option<Res<crate::plugins::config::ClientConfig>>,
    mut terrain_query: Query<
        (Entity, &Terrain, &mut Visibility, Option<&PreloadedTerrain>),
        With<Terrain>,
    >,
    region_files: Query<(
        Option<&TerrainMeshData>,
        &TerrainLightmapData,
        &TerrainObjectData,
        &TerrainNavMeshData,
    )>,
    mut residency: Option<ResMut<AssetResidency>>,
    time: Res<Time>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    map_infos: Res<Assets<JMXVMFO>>,
    maps_assets: Res<MapsAssets>,
    origin: Res<WorldOrigin>,
) {
    // Regions streamed in on each side of the camera's region: the view
    // distance plus `UNLOAD_BUFFER` rings of head start (`ViewRange::load_ring`).
    let load_ring = view.load_ring;

    // Nothing to do
    if camera_query.is_empty() {
        return;
    }

    let main_layer = bevy::camera::visibility::RenderLayers::layer(
        crate::plugins::camera::CameraLayers::Main.into(),
    );
    for (transform, camera, fog_active, layers) in camera_query.iter() {
        if !camera.is_active || !layers.is_none_or(|layers| layers.intersects(&main_layer)) {
            continue;
        }
        // Only a moved camera or a changed view range can change the ring or
        // the region visibility.
        if !transform.is_changed() && !view.is_changed() {
            continue;
        }
        let cull = fog_active.then_some(view.live_cull);

        // The camera lives in render space; region ids are derived from SRO
        // space, so the world origin has to be added back first.
        let position = origin.to_sro(transform.translation);
        let (region_x, region_z) = position.to_x_z();

        let mut existing_terrains = HashMap::new();

        let map_info = match map_infos.get(&maps_assets.map_info) {
            Some(map_info) => map_info,
            None => {
                warn!("no map info");
                return;
            }
        };

        if map_info.map_width == 0 || map_info.map_height == 0 {
            warn!("map info has zero size");
            return;
        }

        let unload_min_x = region_x as i32 - load_ring;
        let unload_max_x = region_x as i32 + load_ring;
        let unload_min_z = region_z as i32 - load_ring;
        let unload_max_z = region_z as i32 + load_ring;

        // Load chunks out to the full margin ring (they are despawned only
        // `UNLOAD_HYSTERESIS` rings beyond it), not just
        // out to the fully-visible (no-fog) range. That gives mesh/texture streaming a
        // head start while the chunk is still hidden behind fog, so by the time the camera
        // gets close enough for the fog to clear, the chunk is already fully loaded instead
        // of popping in.
        let load_min_x = unload_min_x.max(0);
        let load_max_x = unload_max_x.min(map_info.map_width as i32 - 1);
        let load_min_z = unload_min_z.max(0);
        let load_max_z = unload_max_z.min(map_info.map_height as i32 - 1);

        // Regions stay *loaded* out to the unload boundary (streaming head start under
        // fog cover), but only regions not yet fully behind the opaque fog are rendered —
        // see `region_visibility`. Hiding the region root hides its whole subtree
        // (ground groups, map objects, water planes).
        // `graphics.streaming.region_unloads_per_frame` (this constant by default)
        let mut unload_budget = config.as_ref().map_or(REGION_UNLOADS_PER_FRAME, |config| {
            crate::plugins::config::graphics::StreamingSettings::budget(
                config.graphics.streaming.region_unloads_per_frame,
            )
        });
        terrain_query
            .iter_mut()
            .for_each(|(entity, terrain, mut visibility, preloaded)| {
                let (t_x, t_z) = terrain.to_x_z();
                let t_x = t_x as i32;
                let t_z = t_z as i32;
                let ring_min = (unload_min_x, unload_min_z);
                let ring_max = (unload_max_x, unload_max_z);
                let in_load_ring = !outside_ring((t_x, t_z), ring_min, ring_max, 0);
                let out_of_range = outside_ring((t_x, t_z), ring_min, ring_max, UNLOAD_HYSTERESIS);
                if out_of_range && preloaded.is_none() && unload_budget > 0 {
                    unload_budget -= 1;
                    trace!(
                        "Unload TerrainId {:?}",
                        TerrainId::from_x_z(t_x as u8, t_z as u8)
                    );
                    // Keep the region's files loaded for a grace period, so
                    // coming back soon finds them decoded (the lightmap alone
                    // is tens of ms); see `asset_residency`.
                    if let (Some(residency), Ok((mesh, lightmap, objects, nav))) =
                        (residency.as_deref_mut(), region_files.get(entity))
                    {
                        let now = time.elapsed();
                        if let Some(mesh) = mesh {
                            residency.park(mesh.0.clone().untyped(), now);
                        }
                        residency.park(lightmap.0.clone().untyped(), now);
                        residency.park(objects.0.clone().untyped(), now);
                        residency.park(nav.0.clone().untyped(), now);
                    }
                    commands.entity(entity).despawn();
                } else {
                    // The preload marker is a *head start*, not a pin: it exists so the
                    // eagerly-spawned starting area survives until the camera actually
                    // reaches it. Retiring it on first arrival is what bounds the resident
                    // set — left on, those regions (up to 7x7 around Jangan) were exempt
                    // from despawn for the whole session, so walking away kept their
                    // meshes, materials, textures and full map-object subtrees resident
                    // and still paying extract/prepare cost every frame.
                    if in_load_ring && preloaded.is_some() {
                        commands.entity(entity).remove::<PreloadedTerrain>();
                    }
                    // set_if_neq: an unconditional write would change-flag all
                    // ~81 region subtrees for visibility re-propagation on
                    // every camera move.
                    visibility.set_if_neq(region_visibility(position, t_x, t_z, cull));
                    existing_terrains.insert((t_x, t_z), true);
                }
            });

        for x in load_min_x..=load_max_x {
            for z in load_min_z..=load_max_z {
                let x_u8 = x as u8;
                let z_u8 = z as u8;
                if !existing_terrains.contains_key(&(x_u8 as i32, z_u8 as i32))
                    && map_info.region_data[x as usize + z as usize * map_info.map_width]
                {
                    let p = origin.to_render(Vec3::new(
                        x as f32 * -REGION_SIZE,
                        0.0,
                        z as f32 * REGION_SIZE,
                    ));
                    let terrain_id = TerrainId::from_x_z(x_u8, z_u8);
                    let hex_id = format!("{:x}", terrain_id);
                    trace!("Load TerrainId {:?} ({:?})", terrain_id, hex_id);
                    commands
                        .spawn(TerrainBundle {
                            terrain: Terrain(terrain_id),
                            map_data: TerrainMeshData(
                                asset_server.load(format!("map://{z}/{x}.m")),
                            ),
                            lightmap_data: TerrainLightmapData(
                                asset_server.load(format!("map://{z}/{x}.t")),
                            ),
                            object_data: TerrainObjectData(
                                asset_server.load(format!("map://{z}/{x}.o2")),
                            ),
                            terrain_load_state: TerrainLoadState::None,
                            nav_mesh: TerrainNavMeshData(
                                asset_server.load(format!("data://navmesh/nv_{hex_id}.nvm")),
                            ),
                            transform: Transform::from_translation(p),
                            // outer-ring regions spawn already hidden — they
                            // must not render a stray frame before the next
                            // camera move re-evaluates them
                            visibility: region_visibility(position, x, z, cull),
                        })
                        .insert(Name::from(format!("Region {}/{}", x, z)));
                }
            }
        }
    }
}

/// Marks a merged terrain ground-group entity, whichever draw path built it
/// (`graphics.terrain.pipeline`): the two paths carry different components
/// (`MeshMaterial3d<TerrainBlockSplatMaterial>` vs. `TerrainGroundTextures`),
/// and diagnostics / the render-debug panel just need "is this terrain".
#[derive(Component)]
pub struct TerrainGround;

/// The per-region ground component of whichever draw path is active.
enum GroundMaterial {
    Material(MeshMaterial3d<TerrainBlockSplatMaterial>),
    HandRolled(TerrainGroundTextures),
}

/// How many merged terrain groups may be built in one frame, across all
/// regions. One build merges the group's block meshes and copies tile pixel
/// data into the splat texture array — several milliseconds of main-thread
/// work. Crossing a region boundary queues several regions × 4 groups at
/// once; building them all in one frame was the terrain-streaming hitch, so
/// the work is spread over frames instead, nearest region first (the far
/// ones sit behind fog while they wait).
pub(crate) const GROUP_BUILDS_PER_FRAME: u32 = 2;

/// How far the terrain LOD skirts hang below a region's edge
/// (`merge_block_meshes_lod`). It only has to exceed the height a coarser
/// edge can miss between two of its vertices. That is bounded by the relief
/// across 80 units (a quarter-grid step), and 100 covers the steep slopes of
/// the shipped regions. A deeper skirt costs nothing visible, since it stays
/// under the neighbour's surface.
const TERRAIN_SKIRT_DEPTH: f32 = 100.0;

pub fn load_terrain_system(
    mut commands: Commands,
    mut query: Query<(
        Entity,
        &Terrain,
        &Name,
        &Transform,
        &TerrainMeshData,
        &TerrainLightmapData,
        &mut TerrainLoadState,
    )>,
    camera_query: Query<(&Transform, &Camera), (With<Camera3d>, Without<Terrain>)>,
    asset_server: Res<AssetServer>,
    terrain_mesh: Option<Res<TerrainMesh>>,
    map_mesh_assets: Res<Assets<JMXVMAPM>>,
    lightmap_assets: Res<Assets<JMXVMAPT>>,
    lightmap_fallback: Res<TerrainLightmapFallback>,
    mut image_assets: ResMut<Assets<Image>>,
    mut mesh_assets: ResMut<Assets<Mesh>>,
    mut terrain_block_material_assets: ResMut<Assets<TerrainBlockSplatMaterial>>,
    // Fixed at startup by MapPlugin from `graphics.terrain.pipeline` (absent = material):
    // only the pipeline registered then can draw what is built here.
    pipeline: Option<Res<TerrainPipeline>>,
    // Exactly one water tier is inserted by `setup_terrain_mesh` (`graphics.water.quality`),
    // so both are optional and the spawn below picks whichever is present.
    // tupled: this system is at Bevy's 16-parameter ceiling
    (water_material, water_low_material, ice_material): (
        Option<Res<WaterNormalMaterial>>,
        Option<Res<WaterLowMaterial>>,
        Option<Res<WaterIceMaterial>>,
    ),
    // tupled with the view range: at the 16-parameter ceiling
    (view, config): (
        Res<crate::plugins::map::view_range::ViewRange>,
        Option<Res<crate::plugins::config::ClientConfig>>,
    ),
    terrain_only: Option<Res<TerrainOnlyBenchmark>>,
) {
    // No tile-index gate here any more: ground textures are resolved once, globally, by
    // `build_tile_atlas`, and a region's bind group simply retries until that atlas exists.
    let camera_pos = camera_query
        .iter()
        .find(|(_, camera)| camera.is_active)
        .map(|(transform, _)| transform.translation);

    // Regions with groups left to build, closest to the camera first: when a
    // boundary crossing queues several regions at once, the ground the player
    // can actually see finishes before the fog-hidden ring.
    let mut building = Vec::new();
    for (terrain_entity, _terrain, terrain_name, transform, map_data, lightmap_data, load_state) in
        query.iter_mut()
    {
        if !asset_server
            .get_load_state(&map_data.0)
            .unwrap_or(LoadState::NotLoaded)
            .is_loaded()
        {
            continue;
        }
        // Resolve the region's baked lightmap before building its groups so they pick it up.
        // Wait only while the `.t` is still loading; a missing or failed lightmap resolves to
        // `None` and the group builds unmodulated (white-fallback) rather than blocking forever.
        let lightmap = match asset_server.get_load_state(&lightmap_data.0) {
            Some(LoadState::Loading) => continue,
            _ => lightmap_assets
                .get(&lightmap_data.0)
                .and_then(|lm| lm.lightmap.clone()),
        };
        match load_state.as_ref() {
            TerrainLoadState::None | TerrainLoadState::BuildingMeshes { .. } => {
                let dist_sq = camera_pos
                    .map(|p| p.distance_squared(transform.translation))
                    .unwrap_or(0.0);
                building.push((
                    terrain_entity,
                    terrain_name,
                    map_data,
                    lightmap,
                    load_state,
                    dist_sq,
                ));
            }
            TerrainLoadState::LoadedMeshes => {}
            TerrainLoadState::Completed => {
                if let Ok(mut entity) = commands.get_entity(terrain_entity) {
                    entity.remove::<TerrainMeshData>();
                }
            }
        }
    }
    building.sort_by(|a, b| a.5.total_cmp(&b.5));

    // `graphics.streaming.region_builds_per_frame` (this constant by default)
    let mut budget = config.as_ref().map_or(GROUP_BUILDS_PER_FRAME, |config| {
        crate::plugins::config::graphics::StreamingSettings::budget(
            config.graphics.streaming.region_builds_per_frame,
        )
    });
    for (terrain_entity, terrain_name, map_data, lightmap, mut load_state, _dist_sq) in building {
        if budget <= 0 {
            break;
        }
        let map_data = map_mesh_assets
            .get(&map_data.0)
            .expect("failed to get map data");
        // Merge size is decided once (on the first frame this region builds) and
        // carried in the load state so `total_groups` is stable across the
        // multi-frame build.
        let (mut next_group, group_size) = match *load_state {
            TerrainLoadState::BuildingMeshes {
                next_group,
                group_size,
            } => (next_group, group_size),
            _ => (0, REGION_BLOCKS_PER_SIDE),
        };
        let groups_per_side = (REGION_BLOCKS_PER_SIDE + group_size - 1) / group_size;
        let total_groups = groups_per_side * groups_per_side;

        while budget > 0 && next_group < total_groups {
            let gx = next_group % groups_per_side;
            let gz = next_group / groups_per_side;
            next_group += 1;

            let origin_x = gx * group_size;
            let origin_z = gz * group_size;
            let mut group_blocks: Vec<(&TerrainBlock, f32, f32)> = Vec::new();
            for dz in 0..group_size {
                for dx in 0..group_size {
                    let bx = origin_x + dx;
                    let bz = origin_z + dz;
                    if bx >= REGION_BLOCKS_PER_SIDE || bz >= REGION_BLOCKS_PER_SIDE {
                        continue;
                    }
                    let block = &map_data.blocks[(bz * REGION_BLOCKS_PER_SIDE + bx) as usize];
                    group_blocks.push((block, (dx * 320) as f32, (dz * 320) as f32));
                }
            }
            if group_blocks.is_empty() {
                continue;
            }
            budget -= 1;

            let group_origin = (origin_x as f32 * -320.0, origin_z as f32 * 320.0);
            let group_transform = Transform {
                translation: Vec3::new(group_origin.0, 0.0, group_origin.1),
                scale: Vec3::new(-1.0, 1.0, 1.0),
                ..default()
            };
            let hand_rolled = pipeline.as_deref() == Some(&TerrainPipeline::HandRolled);
            let reverse_winding = needs_winding_reversal(&group_transform.to_matrix());
            // `graphics.view.terrain_lod`: the full grid plus a half and a
            // quarter grid, all skirted, switched by distance (`ViewRange::
            // terrain_lod_range`). Material path only: the hand-rolled
            // pipeline draws whatever it extracts and would draw all three.
            let lod = view.terrain_lod.is_some() && !hand_rolled;
            let mut group_aabb = merged_group_aabb(&group_blocks);
            let mesh = if lod {
                group_aabb.center.y -= TERRAIN_SKIRT_DEPTH / 2.0;
                group_aabb.half_extents.y += TERRAIN_SKIRT_DEPTH / 2.0;
                merge_block_meshes_lod(
                    map_data,
                    &group_blocks,
                    reverse_winding,
                    1,
                    Some(TERRAIN_SKIRT_DEPTH),
                )
            } else {
                merge_block_meshes(map_data, &group_blocks, reverse_winding)
            };
            let mesh = mesh_assets.add(mesh);
            let lod_meshes = lod.then(|| {
                [2, 4].map(|step| {
                    mesh_assets.add(merge_block_meshes_lod(
                        map_data,
                        &group_blocks,
                        reverse_winding,
                        step,
                        Some(TERRAIN_SKIRT_DEPTH),
                    ))
                })
            });
            let region_lightmap = lightmap
                .clone()
                .unwrap_or_else(|| lightmap_fallback.0.clone());

            let Ok(mut entity) = commands.get_entity(terrain_entity) else {
                continue;
            };
            let ground_material = if hand_rolled {
                GroundMaterial::HandRolled(TerrainGroundTextures::from(
                    &group_blocks,
                    region_lightmap,
                    &mut image_assets,
                ))
            } else {
                let material = TerrainBlockSplatMaterial::from(
                    &group_blocks,
                    region_lightmap,
                    &mut image_assets,
                );
                GroundMaterial::Material(MeshMaterial3d(
                    terrain_block_material_assets.add(material),
                ))
            };
            let lod_material = match &ground_material {
                GroundMaterial::Material(material) if lod => Some(material.clone()),
                _ => None,
            };
            entity.with_children(|terrain_entity: &mut bevy::ecs::hierarchy::ChildSpawnerCommands| {
                // The half and quarter grids: siblings of the full one, same
                // placement and material, each drawn over its own distances.
                if let (Some(lod_meshes), Some(material)) = (&lod_meshes, &lod_material) {
                    for (level, lod_mesh) in (1u8..).zip(lod_meshes) {
                        terrain_entity.spawn((
                            Mesh3d(lod_mesh.clone()),
                            material.clone(),
                            TerrainGround,
                            group_transform,
                            Visibility::default(),
                            group_aabb,
                            TerrainLodLevel(level),
                            view.terrain_lod_range(level),
                            Name::from(format!("Ground LOD{level} ({})", terrain_name.as_str())),
                        ));
                    }
                }
                let mut group_entity = terrain_entity.spawn((
                    Mesh3d(mesh),
                    TerrainGround,
                    group_transform,
                    Visibility::default(),
                    group_aabb,
                    Name::from(format!("Ground group {}x{} ({})", gx, gz, terrain_name.as_str())),
                ));
                match ground_material {
                    GroundMaterial::Material(material) => group_entity.insert(material),
                    GroundMaterial::HandRolled(textures) => group_entity.insert(textures),
                };
                if lod {
                    group_entity.insert((TerrainLodLevel(0), view.terrain_lod_range(0)));
                }
                if terrain_only.is_some() {
                    return;
                }
                group_entity.with_children(|group_entity: &mut bevy::ecs::hierarchy::ChildSpawnerCommands| {
                    for (block, dx, dz) in &group_blocks {
                        group_entity.spawn((
                            Transform::from_xyz(*dx, 0.0, *dz),
                            Visibility::default(),
                            (*block).clone(),
                            block.aabb.clone(),
                            DebugAabb::with_color(bevy::color::palettes::basic::GREEN),
                            GameCursorTarget::default(),
                            Name::from(format!("Ground {}x{} ({})", block.x, block.z, terrain_name.as_str())),
                        )).with_children(|block_entity: &mut bevy::ecs::hierarchy::ChildSpawnerCommands| match &block.water_type {
                            WaterType::None => {}
                            WaterType::Water(_wave_type, height) => {
                                // plane mesh vertices are located around plane center
                                let transform = Transform::from_xyz(320.0, *height, 320.0)
                                    .with_scale(Vec3::new(-1.0, 1.0, -1.0));
                                let name = Name::from(format!("Water {}x{} ({})", block.x, block.z, terrain_name.as_str()));
                                // The two tiers differ only in which material type the plane
                                // carries, so the mesh/transform/name are shared and only the
                                // `MeshMaterial3d` component type changes.
                                match (&water_material, &water_low_material) {
                                    (Some(hq), _) if terrain_mesh.is_some() => {
                                        block_entity.spawn((
                                            Mesh3d(terrain_mesh.as_ref().unwrap().0.clone()),
                                            MeshMaterial3d(hq.0.clone()),
                                            transform,
                                            Visibility::default(),
                                            WaterPlane,
                                            name,
                                        ));
                                    }
                                    (None, Some(low)) if terrain_mesh.is_some() => {
                                        block_entity.spawn((
                                            Mesh3d(terrain_mesh.as_ref().unwrap().0.clone()),
                                            MeshMaterial3d(low.0.clone()),
                                            transform,
                                            Visibility::default(),
                                            WaterPlane,
                                            name,
                                        ));
                                    }
                                    (None, None) => {}
                                    _ => {}
                                }
                            }
                            WaterType::Ice(height) if terrain_mesh.is_some() && ice_material.is_some() => {
                                block_entity.spawn((
                                    Mesh3d(terrain_mesh.as_ref().unwrap().0.clone()),
                                    MeshMaterial3d(ice_material.as_ref().unwrap().0.clone()),
                                    Transform::from_xyz(320.0, *height, 320.0)
                                        .with_scale(Vec3::new(-1.0, 1.0, -1.0)),
                                    Visibility::default(),
                                    WaterPlane,
                                    Name::from(format!("Ice {}x{} ({})", block.x, block.z, terrain_name.as_str())),
                                ));
                            }
                            WaterType::Ice(_) => {}
                        });
                    }
                });
            });
        }

        *load_state = if next_group >= total_groups {
            TerrainLoadState::LoadedMeshes
        } else {
            TerrainLoadState::BuildingMeshes {
                next_group,
                group_size,
            }
        };
    }
}

/// Union of a merge group's blocks' AABBs, each offset by its `(dx, dz)` group-local position
/// (see `merge_block_meshes`) — used purely for frustum culling of the merged mesh entity.
fn merged_group_aabb(blocks: &[(&TerrainBlock, f32, f32)]) -> bevy::camera::primitives::Aabb {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for (block, dx, dz) in blocks {
        let offset = Vec3::new(*dx, 0.0, *dz);
        min = min.min(Vec3::from(block.aabb.min()) + offset);
        max = max.max(Vec3::from(block.aabb.max()) + offset);
    }
    bevy::camera::primitives::Aabb::from_min_max(min, max)
}

/// Water and ice animate in the fragment shader, so only the four corners are
/// needed. The 320-unit extent is one terrain block (16 cells of 20 units);
/// preserve its UVs and authored winding under scale=(-1,1,-1) placement.
pub fn water_patch_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [0.0, 0.0, 0.0],
            [320.0, 0.0, 0.0],
            [0.0, 0.0, 320.0],
            [320.0, 0.0, 320.0],
        ],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4]);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
    );
    mesh.insert_indices(Indices::U32(vec![2, 0, 3, 3, 0, 1]));
    mesh
}

#[cfg(test)]
mod streaming_tests {
    use super::*;

    /// A camera at region (10, 10) loads the ring 10 ± the default load ring.
    fn ring() -> ((i32, i32), (i32, i32)) {
        let r = crate::plugins::map::view_range::ViewRange::default().load_ring;
        ((10 - r, 10 - r), (10 + r, 10 + r))
    }

    #[test]
    fn a_region_just_past_the_load_ring_is_kept() {
        let (min, max) = ring();
        let edge = (max.0 + 1, 10);
        assert!(outside_ring(edge, min, max, 0), "not loaded any more");
        assert!(
            !outside_ring(edge, min, max, UNLOAD_HYSTERESIS),
            "but kept: one step back across the line must not rebuild it"
        );
    }

    #[test]
    fn a_region_past_the_hysteresis_ring_is_unloaded() {
        let (min, max) = ring();
        assert!(outside_ring(
            (min.0 - UNLOAD_HYSTERESIS - 1, 10),
            min,
            max,
            UNLOAD_HYSTERESIS
        ));
        assert!(outside_ring(
            (10, max.1 + UNLOAD_HYSTERESIS + 1),
            min,
            max,
            UNLOAD_HYSTERESIS
        ));
    }

    /// The whole point: oscillating across one region line never despawns a
    /// region that is loaded on either side of it.
    #[test]
    fn oscillating_across_a_region_line_unloads_nothing() {
        let r = crate::plugins::map::view_range::ViewRange::default().load_ring;
        for cam in [10, 11, 10, 11] {
            let (min, max) = ((cam - r, 10 - r), (cam + r, 10 + r));
            for other in [10, 11] {
                for x in (other - r)..=(other + r) {
                    assert!(!outside_ring((x, 10), min, max, UNLOAD_HYSTERESIS));
                }
            }
        }
    }
}

#[cfg(test)]
mod water_tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    #[test]
    fn water_quad_preserves_extent_uvs_and_authored_winding() {
        let mesh = water_patch_mesh();
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions");
        };
        let Some(VertexAttributeValues::Float32x2(uvs)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0)
        else {
            panic!("uvs");
        };
        assert_eq!(positions.len(), 4);
        for (p, uv) in positions.iter().zip(uvs) {
            assert_eq!(*p, [uv[0] * 320.0, 0.0, uv[1] * 320.0]);
        }
        let indices: Vec<_> = mesh.indices().unwrap().iter().collect();
        assert_eq!(indices.len(), 6);
        for tri in indices.chunks_exact(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]]
                .map(|i| Vec3::from_array(positions[i]) * Vec3::new(-1.0, 1.0, -1.0));
            // Compare against the actual old grid, whose authored winding
            // is opposite the supplied +Y normals. Preserve that convention.
            let reference = &crate::assets::m::block_mesh::BLOCK_INDICES[..3];
            let [ra, rb, rc] = [reference[0], reference[1], reference[2]]
                .map(|i| Vec3::new((i % 17) as f32 * -20.0, 0.0, (i / 17) as f32 * -20.0));
            assert!((b - a).cross(c - a).dot((rb - ra).cross(rc - ra)) > 0.0);
        }
    }
}
