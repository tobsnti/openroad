use bevy::asset::{Assets, RenderAssetUsages};
use bevy::prelude::Component;
use bevy::prelude::{
    error, info, not, resource_exists, warn, App, AssetServer, Commands, DetectChanges, Handle,
    Image, IntoScheduleConfigs, Plugin, PreUpdate, Res, ResMut, Resource, Startup, Update,
};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::{
    Buffer, BufferInitDescriptor, BufferSize, BufferUsages, Extent3d, TextureDimension,
    TextureFormat,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use super::tile_arrays::TerrainTileArrays;
use crate::assets::ifo::IFOAsset;
use crate::assets::m::TerrainBlock;
use crate::plugins::map::assets::TileAssets;

// The Material-based draw path (`graphics.terrain.pipeline: material`, the default) uses bevy's
// Material/AsBindGroup machinery; the hand-rolled one (`plugins::map::terrain::render`) builds its
// bind groups by hand from `RenderDevice` directly. Both are always compiled; the config decides.
use bevy::asset::Asset;
use bevy::ecs::system::SystemParam;
use bevy::log::warn_once;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::{default, AlphaMode, Material};
use bevy::reflect::TypePath;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    AddressMode, AsBindGroup, AsBindGroupError, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType,
    BufferBindingType, Face, FilterMode, MipmapFilterMode, PipelineCache, PreparedBindGroup,
    RenderPipelineDescriptor, SamplerBindingType, SamplerDescriptor, ShaderStages,
    SpecializedMeshPipelineError, TextureSampleType, TextureViewDimension, UnpreparedBindGroup,
};
use bevy::render::texture::GpuImage;
use bevy::shader::ShaderRef;

/// Size of [`TerrainTileAtlas`]'s CPU-side registry, mapping every tile id the map format can
/// express to its texture handle. `JMXVMAPM` packs a vertex's tile id into 10 bits
/// (`assets/m/mod.rs`: `flags & 0b0000_0011_1111_1111`), so 1024 covers the entire id space by
/// construction. The shipped `tile2d.ifo` defines 719 ids (dense 0..718), of which 685 are
/// referenced by some region (Map.pk2 census 2026-08-08).
///
/// The GPU side is `TerrainTileArrays` (`tile_arrays.rs`): four `texture_2d_array`s of 256
/// layers covering exactly this id space, built once from every tile's normalized layer and
/// shared by every region and both draw paths. Earlier designs bound texture *binding arrays*
/// (1024 slots globally, then 64 region-local slots with a per-region id remap); those need
/// bindless sampling, which pre-2016 GPUs and WebGPU/WebGL2-class devices lack, so the client
/// could not start on them (see `assets::tile_layers`).
pub const TILE_SLOT_COUNT: u32 = 1024;

/// Vertices per block edge, and blocks per region edge — the `tile_map` packing below.
pub(crate) const BLOCK_VERTS: usize = 17;
pub(crate) const REGION_BLOCKS_PER_SIDE: usize = 6;
/// `tile_map` is one 102x102 texture per region: 6 blocks of 17 vertices per axis, with each
/// block keeping its own duplicated edge vertices so a fragment never gathers across a block
/// boundary (see the header comment in `terrain_splat.wgsl`).
pub(crate) const TILE_MAP_SIZE: usize = REGION_BLOCKS_PER_SIDE * BLOCK_VERTS;

/// Every ground tile texture in the game, indexed by the map's 10-bit tile id — the CPU-side
/// registry `TerrainTileArrays` (`tile_arrays.rs`) is built from, and that resolves a tile id to
/// its texture for the foliage tints. Not bound to the GPU directly.
///
/// The handles are the same assets `TileAssets` already loads (the whole `map://tile2d` folder),
/// so populating this costs no extra memory or load time — it only gives them an id-ordered home.
#[derive(Resource, Clone, Default)]
pub struct TerrainTileAtlas {
    /// `TILE_SLOT_COUNT` entries; `None` where the id is undefined (those array layers are
    /// white, so an unknown id renders white rather than breaking anything).
    pub slots: Vec<Option<Handle<Image>>>,
}

impl TerrainTileAtlas {
    pub fn new() -> Self {
        Self {
            slots: vec![None; TILE_SLOT_COUNT as usize],
        }
    }
}

#[derive(TypePath, Asset, Default, Debug, Clone)]
pub struct TerrainBlockSplatMaterial {
    /// Per-vertex tile choice for one region, as a 102x102 `Rg16Uint` image: `r` = the map's
    /// raw 10-bit tile id (layer `r % 256` of tile array `r / 256`, see `TerrainTileArrays`),
    /// `g` = splat scale code. Texel `(block_col * 17 + i, block_row * 17 + j)` is that block's
    /// vertex `(i, j)`.
    ///
    /// The shader gathers the 4 texels around a fragment and blends them by their bilinear
    /// weights, which is algebraically identical to the one-hot weight texture this replaces
    /// but costs 4 texel reads instead of one sample per tile in the group.
    pub tile_map: Handle<Image>,

    pub backface_culling: bool,

    /// Baked terrain lightmap for this group's region, decoded from the region's `.t`
    /// (JMXVMAPT, see `assets/t.rs`). Sampled at region-local UV and multiplied into the ground
    /// albedo in `terrain_splat.wgsl`, adding SRO's static baked sun/shadow on top of the dynamic
    /// lighting. Regions without a `.t` get a shared 1×1 white handle, making the multiply a no-op.
    pub lightmap: Handle<Image>,
}

/// Per-region ground textures for the hand-rolled render pipeline
/// (`client/src/plugins/map/terrain/render/`, `graphics.terrain.pipeline: hand_rolled`) — the
/// counterpart to [`TerrainBlockSplatMaterial`] for that path.
///
/// A plain `Component`, not an `Asset`: unlike a normal material, these textures are never
/// shared/deduplicated across users (each region's `tile_map`/`lightmap` are already unique), so
/// the `Handle<Self>`/`Assets<T>` indirection `TerrainBlockSplatMaterial` needs would be pure
/// overhead here. Extracted into the render world by
/// `plugins::map::terrain::render::extract_terrain_ground_textures`.
#[derive(Component, Clone, Default)]
pub struct TerrainGroundTextures {
    /// Per-vertex tile choice for one region — see [`TerrainBlockSplatMaterial::tile_map`].
    pub tile_map: Handle<Image>,
    pub backface_culling: bool,
    /// Baked terrain lightmap — see [`TerrainBlockSplatMaterial::lightmap`].
    pub lightmap: Handle<Image>,
}

impl TerrainGroundTextures {
    /// Builds the per-region textures for one region's merged 6x6 block grid. Same role as
    /// `TerrainBlockSplatMaterial::from`.
    pub(crate) fn from(
        blocks: &[(&TerrainBlock, f32, f32)],
        lightmap: Handle<Image>,
        image_assets: &mut ResMut<Assets<Image>>,
    ) -> Self {
        Self {
            tile_map: tile_map_image(blocks, image_assets),
            backface_culling: true,
            lightmap,
        }
    }
}

/// A region's packed tile map as an image (see [`TerrainBlockSplatMaterial::tile_map`]).
fn tile_map_image(
    blocks: &[(&TerrainBlock, f32, f32)],
    image_assets: &mut ResMut<Assets<Image>>,
) -> Handle<Image> {
    image_assets.add(Image::new(
        Extent3d {
            width: TILE_MAP_SIZE as u32,
            height: TILE_MAP_SIZE as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pack_tile_map(blocks),
        TextureFormat::Rg16Uint,
        RenderAssetUsages::RENDER_WORLD,
    ))
}

/// Packs a region's per-vertex tile choices into the `Rg16Uint` 102x102 layout described on
/// [`TerrainBlockSplatMaterial::tile_map`]: `r` = the vertex's raw map tile id, `g` = splat
/// scale code. Blocks are placed by their own `(x, z)` grid position, so this does not depend
/// on the order `blocks` arrives in or on the mesh's vertex layout.
fn pack_tile_map(blocks: &[(&TerrainBlock, f32, f32)]) -> Vec<u8> {
    let mut buf = vec![0u8; TILE_MAP_SIZE * TILE_MAP_SIZE * 4];
    for (block, _, _) in blocks {
        let (bx, bz) = (block.x as usize, block.z as usize);
        debug_assert!(bx < REGION_BLOCKS_PER_SIDE && bz < REGION_BLOCKS_PER_SIDE);
        for z in 0..BLOCK_VERTS {
            for x in 0..BLOCK_VERTS {
                let v = &block.vertices[z * BLOCK_VERTS + x];
                let texel = (bz * BLOCK_VERTS + z) * TILE_MAP_SIZE + (bx * BLOCK_VERTS + x);
                let o = texel * 4;
                buf[o..o + 2].copy_from_slice(&v.texture_id.to_le_bytes());
                buf[o + 2..o + 4].copy_from_slice(&(v.splat_scale as u16).to_le_bytes());
            }
        }
    }
    buf
}

/// Exercises `pack_tile_map`'s raw-id packing.
#[cfg(test)]
mod raw_pack_tests {
    use super::*;
    use crate::assets::m::{MapVertex, WaterType};
    use bevy::camera::primitives::Aabb;
    use bevy::math::Vec3;

    fn block(bx: i32, bz: i32, tile_at: impl Fn(usize, usize) -> (u16, u8)) -> TerrainBlock {
        let mut vertices = Vec::with_capacity(BLOCK_VERTS * BLOCK_VERTS);
        for z in 0..BLOCK_VERTS {
            for x in 0..BLOCK_VERTS {
                let (texture_id, splat_scale) = tile_at(x, z);
                vertices.push(MapVertex {
                    x: x as i32,
                    z: z as i32,
                    height: 0.0,
                    texture_id,
                    brightness: 0,
                    splat_scale,
                    splat_offset: 0,
                });
            }
        }
        TerrainBlock {
            x: bx,
            z: bz,
            flag: 0,
            environment_id: 0,
            water_type: WaterType::None,
            vertices,
            tiles: Vec::new(),
            aabb: Aabb::from_min_max(Vec3::ZERO, Vec3::ONE),
        }
    }

    fn texel(buf: &[u8], u: usize, v: usize) -> (u16, u16) {
        let o = (v * TILE_MAP_SIZE + u) * 4;
        (
            u16::from_le_bytes([buf[o], buf[o + 1]]),
            u16::from_le_bytes([buf[o + 2], buf[o + 3]]),
        )
    }

    /// A block lands at its own grid position, and vertex (x, z) at texel
    /// (bx * 17 + x, bz * 17 + z) — same addressing as `pack_tile_map`, just with the raw map
    /// tile id instead of a remapped local index.
    #[test]
    fn packs_blocks_at_their_grid_position_with_raw_ids() {
        let b = block(2, 3, |x, z| ((z * BLOCK_VERTS + x) as u16, 8));
        let buf = pack_tile_map(&[(&b, 0.0, 0.0)]);

        assert_eq!(texel(&buf, 2 * 17, 3 * 17), (0, 8), "vertex (0,0)");
        assert_eq!(texel(&buf, 2 * 17 + 5, 3 * 17), (5, 8), "vertex (5,0)");
        assert_eq!(texel(&buf, 2 * 17, 3 * 17 + 5), (5 * 17, 8), "vertex (0,5)");
        assert_eq!(
            texel(&buf, 2 * 17 + 16, 3 * 17 + 16),
            ((16 * 17 + 16) as u16, 8),
            "vertex (16,16)"
        );
        assert_eq!(
            texel(&buf, 0, 0),
            (0, 0),
            "a different block's area is untouched"
        );
    }

    /// Neighbouring blocks keep their own duplicated edge vertices, same as `pack_tile_map`.
    #[test]
    fn adjacent_blocks_keep_separate_edge_vertices() {
        let left = block(0, 0, |_, _| (11, 16));
        let right = block(1, 0, |_, _| (22, 32));
        let buf = pack_tile_map(&[(&left, 0.0, 0.0), (&right, 320.0, 0.0)]);

        assert_eq!(texel(&buf, 16, 0), (11, 16), "left block's last vertex");
        assert_eq!(texel(&buf, 17, 0), (22, 32), "right block's first vertex");
    }
}

/// Per-channel ratio of SRO's terrain ambient to the global (object) ambient light,
/// `1.0` = no difference. The shader re-weights the ambient term Bevy already applied
/// by this factor, giving terrain its own ambient color (see `plugins/environment`)
/// without needing brightness/exposure bookkeeping. `w` unused.
///
/// Deliberately NOT a material field: every `TerrainBlockSplatMaterial` binds one
/// shared GPU buffer (binding 4) that [`TerrainAmbientRatioPlugin`] updates in place
/// via `queue.write_buffer`. Mutating the materials instead would flag them all
/// `Modified` on every day/night tick — and bevy_pbr 0.19's `CreateBindGroupDirectly`
/// re-prepare path (which this material takes for its texture binding array) never
/// frees the previous prepared bind group, permanently leaking its buffers, samplers,
/// and pinned texture views each time (bevy_pbr material.rs `prepare_asset`, the
/// direct-path arm misses the `bind_group_allocator.free` the unprepared arm has).
#[derive(Resource, Clone, ExtractResource)]
pub struct TerrainAmbientRatio(pub [f32; 4]);

impl Default for TerrainAmbientRatio {
    fn default() -> Self {
        Self([1.0; 4])
    }
}

/// The shared render-world buffer behind [`TerrainAmbientRatio`], bound by every
/// splat material's bind group.
#[derive(Resource)]
pub struct TerrainAmbientRatioBuffer(pub Buffer);

/// Global terrain render parameters, shared by every splat material through
/// one GPU buffer (binding 6) — the same zero-leak `write_buffer` route as
/// [`TerrainAmbientRatio`], so all values are runtime-togglable without
/// dirtying a single material:
///
/// - `lightmap_flip_v`: mirrors the baked-lightmap V axis. The `.t` DDS row
///   order vs. world Z was never GPU-calibrated (`docs/formats/mapt-jmxvmapt.md`);
///   this makes the pending flip a data change instead of a shader edit.
/// - `lighting_mode`: the terrain dynamic-lighting A/B
///   (`docs/rendering-mobile-shader-comparison.md` gap #6). `Dynamic` is the
///   current full PBR sun + ambient over the baked lightmap; `Baked` is
///   `albedo × lightmap`, full stop — the mobile port's (and probably the
///   original's) fully-baked ground. A third mode `FlatBaked` (GPU value 1.0)
///   was removed in ferdoran/openroad#5: it scaled the ground by the ambient
///   term alone, which in `render_mode: pbr` is 30x smaller than in vanilla
///   and arrives without the lightmap, so the ground rendered black.
///
/// Kept in step with `graphics.terrain` by `apply_terrain_render_params`
/// (see [`crate::plugins::settings::live`]); the render-debug panel and the
/// dungeon atmosphere override cycle it live on top of that.
#[derive(Resource, Clone, PartialEq, ExtractResource)]
pub struct TerrainRenderParams {
    pub lightmap_flip_v: bool,
    pub lighting_mode: TerrainLightingMode,
    /// Baked terrain lightmap on/off — follows `graphics.render_mode`
    /// (`Vanilla` = on) as a baseline (`apply_terrain_render_params`), with
    /// the environment plugin's live hotkey-N override
    /// (`EnvironmentSettings.mode`, `apply_render_mode`) able to flip it for
    /// the current session without touching config. NOT owned by the
    /// render-debug panel; `on_settings_changed` carries the current value
    /// through its rebuild instead of resetting it.
    pub lightmap_enabled: bool,
    /// Extra sun-shadow darkening on the lit ground. Always 0 now — it stood
    /// in for a vanilla-mode player shadow that vanilla no longer casts at
    /// all (vanilla fully disables the Sun's shadow maps). Kept as a no-op
    /// GPU-layout slot rather than reworking `terrain_splat.wgsl`'s uniform
    /// layout for its removal.
    pub shadow_strength: f32,
    /// Tiling repeat factors for the five splat-scale codes `8·i`
    /// (`docs/formats/mapm-jmxvmapm.md`), index `i` = code/8, all
    /// live-tunable from the render-debug panel. Playtest verdict
    /// 2026-08-10: **the vertex "Scale" field does not drive tiling at
    /// all** — every code matches vanilla at a constant 0.25 (one repeat
    /// per 80 world units). The constant also removes the hard tiling
    /// seams a varying per-texel factor produced. The field's real
    /// meaning is UNKNOWN; the per-code plumbing stays for future
    /// re-calibration.
    pub splat_factors: [f32; 5],
}

impl Default for TerrainRenderParams {
    fn default() -> Self {
        Self {
            lightmap_flip_v: false,
            lighting_mode: TerrainLightingMode::Dynamic,
            lightmap_enabled: true,
            shadow_strength: 0.0,
            splat_factors: [0.25; 5],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainLightingMode {
    /// Full PBR sun + ambient over the baked lightmap (current behavior).
    Dynamic,
    /// `albedo × lightmap` — fully baked ground (mobile port / original).
    /// Encoded as 2.0, not 1.0: 1.0 was a removed third mode (see the type
    /// doc) and the panel/`perf-capture.ps1` numbering stays as it was.
    Baked,
}

impl TerrainRenderParams {
    /// GPU layout: three vec4s — `[0]` = lightmap UV scale.xy + offset.zw,
    /// `[1].x` = lighting mode as float (the shader branches on `< 0.5`),
    /// `[1].yzw` + `[2].xy` = the splat repeat factors for codes 0..32,
    /// `[2].z` = lightmap strength (1 = multiply baked lightmap into albedo,
    /// 0 = off, PBR mode), `[2].w` = extra sun-shadow darkening — unused,
    /// always 0 (see [`TerrainRenderParams::shadow_strength`]).
    /// Factors are clamped away from 0 — the shader divides by them.
    fn to_gpu(&self) -> [f32; 12] {
        let (scale_v, offset_v) = if self.lightmap_flip_v {
            (-1.0, 1.0)
        } else {
            (1.0, 0.0)
        };
        let mode = match self.lighting_mode {
            TerrainLightingMode::Dynamic => 0.0,
            TerrainLightingMode::Baked => 2.0,
        };
        let f = self.splat_factors.map(|factor| factor.max(0.01));
        let lightmap = if self.lightmap_enabled { 1.0 } else { 0.0 };
        let shadow = self.shadow_strength.clamp(0.0, 1.0);
        [
            1.0, scale_v, 0.0, offset_v, mode, f[0], f[1], f[2], f[3], f[4], lightmap, shadow,
        ]
    }
}

/// The shared render-world buffer behind [`TerrainRenderParams`], bound by
/// every splat material's bind group (binding 6).
#[derive(Resource)]
pub struct TerrainRenderParamsBuffer(pub Buffer);

/// Shared 1×1 white texture used as the lightmap for regions that ship no `.t` file (or whose
/// lightmap failed to decode): white multiplies to a no-op in `terrain_splat.wgsl`, so such terrain
/// renders unmodulated. Created once at startup; every lightmap-less material clones this handle.
#[derive(Resource)]
pub struct TerrainLightmapFallback(pub Handle<Image>);

/// Re-derives the terrain render params from `graphics.terrain`
/// ([`crate::plugins::settings::live`]).
///
/// Previously computed in `Plugin::build`, which meant the lighting mode and
/// the lightmap V-flip were fixed for the whole process; `ExtractResourcePlugin`
/// copies this resource into the render world every frame, so writing it here
/// is all a live change needs. The resource keeps its `Default` for tests and
/// tools that build the plugin without a `ClientConfig`.
fn apply_terrain_render_params(
    config: Res<crate::plugins::config::ClientConfig>,
    mut params: ResMut<TerrainRenderParams>,
) {
    *params = config
        .graphics
        .terrain
        .to_render_params(config.graphics.render_mode);
}

pub struct TerrainAmbientRatioPlugin;

impl Plugin for TerrainAmbientRatioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TerrainAmbientRatio>()
            .init_resource::<TerrainRenderParams>()
            .init_resource::<crate::assets::tile_layers::TerrainTileLayers>()
            .add_plugins((
                ExtractResourcePlugin::<TerrainAmbientRatio>::default(),
                ExtractResourcePlugin::<TerrainRenderParams>::default(),
                ExtractResourcePlugin::<TerrainTileArrays>::default(),
            ))
            .add_systems(
                PreUpdate,
                apply_terrain_render_params.run_if(crate::plugins::settings::live::config_changed),
            )
            .add_systems(Startup, init_terrain_lightmap_fallback)
            .add_systems(
                Update,
                (
                    build_tile_atlas.run_if(not(resource_exists::<TerrainTileAtlas>)),
                    super::tile_arrays::build_tile_arrays
                        .run_if(not(resource_exists::<TerrainTileArrays>)),
                )
                    .chain(),
            );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(
                RenderStartup,
                (
                    init_terrain_ambient_ratio_buffer,
                    init_terrain_params_buffer,
                    check_tile_atlas_support,
                ),
            )
            .add_systems(
                Render,
                (write_terrain_ambient_ratio, write_terrain_params)
                    .in_set(RenderSystems::PrepareResources),
            );
    }
}

/// Populates [`TerrainTileAtlas`] from `tile2d.ifo`, mapping each tile id to its texture handle.
///
/// Runs until the index asset is available, then inserts the resource (which drops it out via its
/// run condition). The handles resolve to assets `TileAssets` has already loaded — bevy_asset_loader
/// gates `GameState::Loading` on that whole collection — so by the time terrain builds, every
/// texture behind these handles is resident.
fn build_tile_atlas(
    mut commands: Commands,
    tile_assets: Option<Res<TileAssets>>,
    ifo_assets: Res<Assets<IFOAsset>>,
    asset_server: Res<AssetServer>,
) {
    let Some(tile_assets) = tile_assets else {
        return;
    };
    let Some(index) = ifo_assets
        .get(&tile_assets.tile_index)
        .and_then(|ifo| ifo.tile_info_index.as_ref())
    else {
        return;
    };

    let mut atlas = TerrainTileAtlas::new();
    let mut out_of_range = 0usize;
    for (id, info) in &index.tiles {
        let slot = *id as usize;
        // The map format can only express ids < TILE_SLOT_COUNT, so an id beyond it could never
        // be referenced by a vertex anyway — skip rather than grow the array.
        if slot >= TILE_SLOT_COUNT as usize {
            out_of_range += 1;
            continue;
        }
        atlas.slots[slot] =
            Some(asset_server.load(format!("map://tile2d/{}", info.texture.display())));
    }
    let defined = atlas.slots.iter().filter(|s| s.is_some()).count();
    if out_of_range > 0 {
        warn!("{out_of_range} tile ids in tile2d.ifo exceed the {TILE_SLOT_COUNT}-slot atlas");
    }
    info!("ground tile atlas: {defined} tile ids of {TILE_SLOT_COUNT} slots");
    commands.insert_resource(atlas);
}

fn init_terrain_lightmap_fallback(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let white = Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[255, 255, 255, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    commands.insert_resource(TerrainLightmapFallback(images.add(white)));
}

/// The splat shader samples `TerrainTileArrays`: four `texture_2d_array`s of up to
/// `TILE_ARRAY_LAYERS` (256) layers, indexed per fragment. 256 layers is the baseline WebGPU /
/// WebGL2 guarantee, so this should never fire; say so loudly if a device reports less rather
/// than leaving mis-rendered ground to debug.
fn check_tile_atlas_support(render_device: Res<RenderDevice>) {
    let layers = render_device.limits().max_texture_array_layers;
    if layers < crate::assets::tile_layers::TILE_ARRAY_LAYERS {
        error!(
            "GPU allows {layers} texture array layers; the ground-tile arrays need {}",
            crate::assets::tile_layers::TILE_ARRAY_LAYERS
        );
    }
}

fn init_terrain_ambient_ratio_buffer(mut commands: Commands, render_device: Res<RenderDevice>) {
    let buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: "terrain_ambient_ratio_shared_buffer".into(),
        contents: bytemuck::cast_slice(&[1.0f32; 4]),
        usage: BufferUsages::COPY_DST | BufferUsages::UNIFORM,
    });
    commands.insert_resource(TerrainAmbientRatioBuffer(buffer));
}

fn init_terrain_params_buffer(mut commands: Commands, render_device: Res<RenderDevice>) {
    let buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: "terrain_render_params_shared_buffer".into(),
        contents: bytemuck::cast_slice(&TerrainRenderParams::default().to_gpu()),
        usage: BufferUsages::COPY_DST | BufferUsages::UNIFORM,
    });
    commands.insert_resource(TerrainRenderParamsBuffer(buffer));
}

fn write_terrain_params(
    params: Option<Res<TerrainRenderParams>>,
    buffer: Option<Res<TerrainRenderParamsBuffer>>,
    queue: Res<RenderQueue>,
) {
    let (Some(params), Some(buffer)) = (params, buffer) else {
        return;
    };
    if params.is_changed() {
        queue.write_buffer(&buffer.0, 0, bytemuck::cast_slice(&params.to_gpu()));
    }
}

fn write_terrain_ambient_ratio(
    ratio: Option<Res<TerrainAmbientRatio>>,
    buffer: Option<Res<TerrainAmbientRatioBuffer>>,
    queue: Res<RenderQueue>,
) {
    // ratio is absent until the first extraction, the buffer until RenderStartup ran
    let (Some(ratio), Some(buffer)) = (ratio, buffer) else {
        return;
    };
    if ratio.is_changed() {
        queue.write_buffer(&buffer.0, 0, bytemuck::cast_slice(&ratio.0));
    }
}

impl Material for TerrainBlockSplatMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/terrain_splat.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/terrain_splat.wgsl".into()
    }

    #[inline]
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Opaque
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = if key.bind_group_data {
            Some(Face::Back)
        } else {
            None
        };
        Ok(())
    }
}

impl AsBindGroup for TerrainBlockSplatMaterial {
    type Data = bool;
    type Param = (
        Res<'static, RenderAssets<GpuImage>>,
        Option<Res<'static, TerrainAmbientRatioBuffer>>,
        Option<Res<'static, TerrainRenderParamsBuffer>>,
        Option<Res<'static, TerrainTileArrays>>,
    );

    fn label() -> &'static str {
        "terrain_block_splat_material"
    }

    fn bind_group_data(&self) -> Self::Data {
        self.backface_culling
    }

    // The bind group is built by hand below (shared samplers and buffers that outlive every
    // material); `CreateBindGroupDirectly` routes the framework to `as_bind_group()`.
    fn unprepared_bind_group(
        &self,
        _layout: &BindGroupLayout,
        _render_device: &RenderDevice,
        _param: &mut <Self::Param as SystemParam>::Item<'_, '_>,
        _force_no_bindless: bool,
    ) -> Result<UnpreparedBindGroup, AsBindGroupError> {
        Err(AsBindGroupError::CreateBindGroupDirectly)
    }

    fn as_bind_group(
        &self,
        layout_descriptor: &BindGroupLayoutDescriptor,
        render_device: &RenderDevice,
        pipeline_cache: &PipelineCache,
        param: &mut <Self::Param as SystemParam>::Item<'_, '_>,
    ) -> Result<PreparedBindGroup, AsBindGroupError> {
        let layout = &pipeline_cache.get_bind_group_layout(layout_descriptor);
        let (image_assets, ambient_ratio_buffer, params_buffer, tile_arrays) = param;

        // Every guard below returns RetryNextUpdate, which bevy retries WITHOUT logging —
        // a dependency that never materializes therefore renders as "terrain silently
        // missing". The warn_once calls turn a stuck retry loop into a diagnosable log
        // line while staying quiet on the expected first-frames retries (each fires at
        // most once per run, and a healthy startup passes through these within a frame
        // or two of the resources appearing).

        // created in RenderStartup; not there yet on the very first prepares
        let Some(ambient_ratio_buffer) = ambient_ratio_buffer else {
            warn_once!("terrain splat: waiting for TerrainAmbientRatioBuffer (RenderStartup)");
            return Err(AsBindGroupError::RetryNextUpdate);
        };
        let Some(params_buffer) = params_buffer else {
            warn_once!("terrain splat: waiting for TerrainRenderParamsBuffer (RenderStartup)");
            return Err(AsBindGroupError::RetryNextUpdate);
        };
        // built once every ground tile has loaded, then uploaded like any image
        let Some(tile_arrays) = tile_arrays else {
            warn_once!("terrain splat: waiting for the ground-tile arrays");
            return Err(AsBindGroupError::RetryNextUpdate);
        };
        let Some(arrays) = tile_array_views(tile_arrays, image_assets) else {
            warn_once!("terrain splat: waiting for the ground-tile arrays to upload");
            return Err(AsBindGroupError::RetryNextUpdate);
        };

        let Some(tile_map_tex) = image_assets.get(&self.tile_map) else {
            warn_once!("terrain splat: waiting for a region tile map upload");
            return Err(AsBindGroupError::RetryNextUpdate);
        };
        let Some(lightmap_tex) = image_assets.get(&self.lightmap) else {
            warn_once!("terrain splat: waiting for a region lightmap upload");
            return Err(AsBindGroupError::RetryNextUpdate);
        };

        let clamp_sampler = render_device.create_sampler(&SamplerDescriptor {
            min_filter: FilterMode::Linear,
            mag_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Linear,
            ..default()
        });
        let tile_sampler = render_device.create_sampler(&tile_sampler_descriptor());

        let bind_group = render_device.create_bind_group(
            "terrain_block_splat_material_bind_group",
            layout,
            &terrain_bind_group_entries(TerrainBindings {
                tile_map: &tile_map_tex.texture_view,
                lightmap: &lightmap_tex.texture_view,
                clamp_sampler: &clamp_sampler,
                tile_sampler: &tile_sampler,
                tile_arrays: arrays,
                ambient_ratio: &ambient_ratio_buffer.0,
                params: &params_buffer.0,
            }),
        );

        Ok(PreparedBindGroup {
            // Nothing here is owned by the bind group: the tile arrays, the shared buffers and
            // both samplers outlive it, and the per-region textures are plain assets.
            bindings: bevy::render::render_resource::BindingResources(vec![]),
            bind_group,
        })
    }

    fn bind_group_layout_entries(
        _render_device: &RenderDevice,
        _force_no_bindless: bool,
    ) -> Vec<BindGroupLayoutEntry>
    where
        Self: Sized,
    {
        let fragment = |binding, ty| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        vec![
            // 0: per-region tile map (Rg16Uint, textureLoad only)
            fragment(
                0,
                BindingType::Texture {
                    multisampled: false,
                    sample_type: TextureSampleType::Uint,
                    view_dimension: TextureViewDimension::D2,
                },
            ),
            // 1: clamp sampler (lightmap)
            fragment(1, BindingType::Sampler(SamplerBindingType::Filtering)),
            // 2, 7, 8, 9: the ground-tile arrays (tile ids 0..255, 256..511, ...)
            fragment(2, tile_array_binding()),
            // 3: repeat+aniso sampler (ground tiles)
            fragment(3, BindingType::Sampler(SamplerBindingType::Filtering)),
            // 4: shared terrain ambient ratio (vec4)
            fragment(4, uniform_binding(16)),
            // 5: baked region lightmap
            fragment(
                5,
                BindingType::Texture {
                    multisampled: false,
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2,
                },
            ),
            // 6: shared global terrain render params (3 x vec4)
            fragment(6, uniform_binding(48)),
            fragment(7, tile_array_binding()),
            fragment(8, tile_array_binding()),
            fragment(9, tile_array_binding()),
        ]
    }
}

/// Everything one terrain region's bind group holds. Both draw paths bind exactly this, with the
/// layout `TerrainBlockSplatMaterial::bind_group_layout_entries` describes.
pub(crate) struct TerrainBindings<'a> {
    pub tile_map: &'a bevy::render::render_resource::TextureView,
    pub lightmap: &'a bevy::render::render_resource::TextureView,
    pub clamp_sampler: &'a bevy::render::render_resource::Sampler,
    pub tile_sampler: &'a bevy::render::render_resource::Sampler,
    pub tile_arrays: [&'a bevy::render::render_resource::TextureView; 4],
    pub ambient_ratio: &'a Buffer,
    pub params: &'a Buffer,
}

/// The bind group entries for [`TerrainBindings`]: 0 tile map, 1 clamp sampler (lightmap),
/// 2/7/8/9 the ground-tile arrays, 3 tile sampler, 4 ambient ratio, 5 lightmap, 6 render params.
pub(crate) fn terrain_bind_group_entries<'a>(b: TerrainBindings<'a>) -> [BindGroupEntry<'a>; 10] {
    let texture = |binding, view| BindGroupEntry {
        binding,
        resource: BindingResource::TextureView(view),
    };
    let sampler = |binding, sampler| BindGroupEntry {
        binding,
        resource: BindingResource::Sampler(sampler),
    };
    [
        texture(0, b.tile_map),
        sampler(1, b.clamp_sampler),
        texture(2, b.tile_arrays[0]),
        sampler(3, b.tile_sampler),
        // shared, updated in place, never re-created — so never owned by a bind group
        BindGroupEntry {
            binding: 4,
            resource: BindingResource::Buffer(b.ambient_ratio.as_entire_buffer_binding()),
        },
        texture(5, b.lightmap),
        BindGroupEntry {
            binding: 6,
            resource: BindingResource::Buffer(b.params.as_entire_buffer_binding()),
        },
        texture(7, b.tile_arrays[1]),
        texture(8, b.tile_arrays[2]),
        texture(9, b.tile_arrays[3]),
    ]
}

/// Layout of one ground-tile array binding (shared with the hand-rolled pipeline).
pub(crate) fn tile_array_binding() -> BindingType {
    BindingType::Texture {
        multisampled: false,
        sample_type: TextureSampleType::Float { filterable: true },
        view_dimension: TextureViewDimension::D2Array,
    }
}

/// Layout of a shared terrain uniform buffer of `size` bytes.
pub(crate) fn uniform_binding(size: u64) -> BindingType {
    BindingType::Buffer {
        ty: BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: BufferSize::new(size),
    }
}

/// The ground tiles' anisotropic filtering clamp, `graphics.anisotropy`. Set
/// once by `main` before the renderer exists; the sampler is created in the
/// render world, which has no `ClientConfig`.
pub static TILE_ANISOTROPY: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(4);

/// The repeat + anisotropic sampler ground tiles are drawn with.
pub(crate) fn tile_sampler_descriptor() -> SamplerDescriptor<'static> {
    SamplerDescriptor {
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        address_mode_w: AddressMode::Repeat,
        // ground tiles are viewed at grazing angles almost everywhere;
        // aniso (4x unless `graphics.anisotropy` says otherwise) keeps them
        // sharp where trilinear over-blurs
        anisotropy_clamp: TILE_ANISOTROPY.load(std::sync::atomic::Ordering::Relaxed),
        ..default()
    }
}

/// The four tile arrays' GPU views, once all of them are uploaded.
pub(crate) fn tile_array_views<'a>(
    arrays: &TerrainTileArrays,
    images: &'a RenderAssets<GpuImage>,
) -> Option<[&'a bevy::render::render_resource::TextureView; 4]> {
    let [a, b, c, d] = &arrays.0;
    Some([
        &images.get(a)?.texture_view,
        &images.get(b)?.texture_view,
        &images.get(c)?.texture_view,
        &images.get(d)?.texture_view,
    ])
}

impl TerrainBlockSplatMaterial {
    /// Builds the material for one region's merged 6x6 block grid. `blocks` is `(block, dx, dz)`
    /// in the same order and offset convention as `block_mesh::merge_block_meshes`; the tile map
    /// is addressed by each block's own `(x, z)` grid position rather than by `dx`/`dz`, so the
    /// packing is independent of the mesh's vertex ordering.
    pub(crate) fn from(
        blocks: &[(&TerrainBlock, f32, f32)],
        lightmap: Handle<Image>,
        image_assets: &mut ResMut<Assets<Image>>,
    ) -> Self {
        Self {
            tile_map: tile_map_image(blocks, image_assets),
            lightmap,
            backface_culling: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tile ids are 10-bit by construction, so every id the map can express has an array layer.
    #[test]
    fn every_expressible_tile_id_fits_the_atlas() {
        assert!(u16::from(u16::MAX & 0b0000_0011_1111_1111) < TILE_SLOT_COUNT as u16);
        assert_eq!(
            TerrainTileAtlas::new().slots.len(),
            TILE_SLOT_COUNT as usize
        );
        assert_eq!(
            TILE_SLOT_COUNT,
            crate::assets::tile_layers::TILE_ARRAY_COUNT as u32
                * crate::assets::tile_layers::TILE_ARRAY_LAYERS
        );
    }
}
