import bevy_pbr::render::pbr_types::{
    STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT,
    STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT,
    pbr_input_new,
    PbrInput,
};
import bevy_pbr::prepass::utils as prepass_utils;
import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_normal_local_to_world, mesh_position_local_to_world, mesh_tangent_local_to_world};
import bevy_pbr::render::skinning::{skin_model, skin_normals};
import bevy_pbr::render::morph::morph;
import bevy_pbr::render::view_transformations::position_world_to_clip;
import bevy_pbr::render::forward_io::{Vertex, VertexOutput, FragmentOutput};
import bevy_pbr::render::pbr_functions::{
    apply_normal_mapping,
    apply_pbr_lighting,
    alpha_discard,
    calculate_view,
    main_pass_post_lighting_processing,
    prepare_world_normal,
    calculate_tbn_mikktspace,
};
import bevy_pbr::render::mesh_view_bindings::{view, lights};
import bevy_pbr::render::mesh_bindings::mesh;
import bevy_core_pipeline::tonemapping::tone_mapping;

@vertex
fn vertex(
    @builtin(vertex_index) index: u32,
    vertex_no_morph: Vertex
) -> VertexOutput {
    var out: VertexOutput;
    @if(MORPH_TARGETS)
    var vertex = morph_vertex(vertex_no_morph);
    @if(!MORPH_TARGETS)
    var vertex = vertex_no_morph;

    @if(SKINNED)
    var model = skin_model(
        vertex.joint_indices,
        vertex.joint_weights,
        vertex_no_morph.instance_index,
    );
    // Use vertex_no_morph.instance_index instead of vertex.instance_index to work around a wgpu dx12 bug.
    // See https://github.com/gfx-rs/naga/issues/2416 .
    @if(!SKINNED)
    var model = get_world_from_local(vertex_no_morph.instance_index);

    @if(VERTEX_NORMALS && SKINNED)
    out.world_normal = skin_normals(model, vertex.normal);
    @if(VERTEX_NORMALS && !SKINNED)
    out.world_normal = mesh_normal_local_to_world(
        vertex.normal,
        // Use vertex_no_morph.instance_index instead of vertex.instance_index to work around a wgpu dx12 bug.
        // See https://github.com/gfx-rs/naga/issues/2416
        vertex_no_morph.instance_index
    );

    @if(VERTEX_POSITIONS)
    out.world_position = mesh_position_local_to_world(model, vec4<f32>(vertex.position, 1.0));
    @if(VERTEX_POSITIONS)
    out.position = position_world_to_clip(out.world_position.xyz);

    @if(VERTEX_UVS)
    out.uv = vertex.uv;

    @if(VERTEX_TANGENTS)
    out.world_tangent = mesh_tangent_local_to_world(
        model,
        vertex.tangent,
        // Use vertex_no_morph.instance_index instead of vertex.instance_index to work around a wgpu dx12 bug.
        // See https://github.com/gfx-rs/naga/issues/2416
        vertex_no_morph.instance_index
    );

    @if(VERTEX_COLORS)
    out.color = vertex.color;

    // Use vertex_no_morph.instance_index instead of vertex.instance_index to work around a wgpu dx12 bug.
    // See https://github.com/gfx-rs/naga/issues/2416
    @if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex_no_morph.instance_index;
    return out;
}

// Ground splat rendering merges a region's whole 6x6 grid of 320-unit terrain blocks into one
// mesh + one material draw call (see `load_terrain_system` / `block_mesh::merge_block_meshes`).
//
// Each map vertex names exactly ONE ground tile (`JMXVMAPM`'s 10-bit texture_id) plus a tiling
// scale; the blend between neighbouring tiles is the bilinear interpolation of those per-vertex
// choices across the quad. This shader evaluates that directly: it reads the tile id at the 4
// vertices surrounding the fragment and weights each by its bilinear factor.
//
// The previous one-hot-weight-texture design (see git history) cost one weight sample per tile
// per fragment and capped the group at however many one-hot channels the material could bind.
// The 4-corner gather has neither property: 4 texel reads regardless of tile count, one
// `tile_map` layer per region.
//
// `tile_map` packs the region's 6x6 blocks of 17x17 vertices into a single 102x102 texture:
// texel (block_col*17 + i, block_row*17 + j) is that block's vertex (i, j). Blocks keep their
// own duplicated edge vertices, so a fragment never gathers across a block boundary and the
// per-block behaviour of the old layout is reproduced exactly. `.g` = splat scale code always.
// `.r` is the map's raw 10-bit tile id on both draw paths: tile `id` is layer `id % 256` of
// `tile_array_<id / 256>` (see `sample` below and `TerrainTileArrays` in tile_arrays.rs).
const REGION_SIZE: f32 = 1920.0;      // 6 blocks * 320 units
const BLOCK_SIZE: f32 = 320.0;
const VERTEX_SPACING: f32 = 20.0;     // 17 vertices per block edge => 16 intervals of 20
const BLOCK_VERTS: u32 = 17u;

// One bind group, the same on both draw paths: the stock `Material` path binds it as the material
// group, the hand-rolled pipeline (`TerrainPipeline::specialize`) defines `MATERIAL_BIND_GROUP`
// for its own group 3 — four groups in all, the baseline `max_bind_groups`.
//
// The ground tiles are four `texture_2d_array`s of 256 layers (the baseline array-layer limit)
// covering the 10-bit id space, not a texture *binding array*: per-fragment indexing of a binding
// array is bindless sampling, which pre-2016 GPUs and WebGPU/WebGL2-class devices lack (see
// client/src/assets/tile_layers.rs). The shared params are uniform buffers for the same reason.
@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var tile_map: texture_2d<u32>;
// Clamp+filtering sampler, used for the lightmap (`tile_map` is only ever `textureLoad`ed).
@group(constants::MATERIAL_BIND_GROUP) @binding(1)
var clamp_sampler: sampler;
// Ground tiles with ids 0..255; 256.. are bindings 7, 8, 9.
@group(constants::MATERIAL_BIND_GROUP) @binding(2)
var tile_array_0: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3)
var tile_sampler: sampler;
// Per-channel ratio of SRO's terrain ambient to the global (object) ambient — one GPU
// buffer shared by ALL splat materials, updated in place by `TerrainAmbientRatioPlugin`
// (see `TerrainAmbientRatio` in block_splat_material.rs). w unused.
@group(constants::MATERIAL_BIND_GROUP) @binding(4)
var<uniform> ambient_ratio: vec4<f32>;
// Baked per-region terrain lightmap (JMXVMAPT, see assets/t.rs): SRO's pre-baked static sun and
// cast shadows on the ground. Sampled at region-local UV with `clamp_sampler` (binding 1).
// A white 1x1 texture stands in for regions with no `.t`, making the multiply a no-op.
@group(constants::MATERIAL_BIND_GROUP) @binding(5)
var lightmap_tex: texture_2d<f32>;
// Global terrain render params, one shared buffer like `ambient_ratio` (see
// `TerrainRenderParams` in block_splat_material.rs):
//   [0].xy = lightmap UV scale, [0].zw = lightmap UV offset — identity (1,1,0,0) or the
//            V-flip calibration (1,-1,0,1), a data change instead of a shader edit;
//   [1].x  = lighting mode: 0 = dynamic PBR (current), 2 = fully baked albedo × lightmap
//            (mobile port / original). 1 used to be a third "flat baked" mode and is gone
//            (see TerrainLightingMode); any non-zero value reads as fully baked;
//   [1].yzw, [2].xy = ground-tile repeat factors for splat-scale codes 0/8/16/24/32
//            (live calibration sliders in the render-debug panel);
//   [2].z  = lightmap strength: 1 = vanilla baked shadows, 0 = PBR mode (the
//            environment plugin's hotkey-N switch swaps them for cascaded shadow
//            maps). Note: only the dynamic lighting mode substitutes real shadows —
//            baked bypasses apply_pbr_lighting, so strength 0 there just
//            leaves the ground unshaded;
//   [2].w  = slope-aware albedo sampling: 0 = off (XZ only, as before),
//            1 = one extra projection on steep fragments, 2 = both.
//            Was an extra sun-shadow darkening that had been dead (always 0)
//            since vanilla stopped casting the Sun's shadow maps; the slot was
//            kept for the next parameter rather than reworking this layout.
@group(constants::MATERIAL_BIND_GROUP) @binding(6)
var<uniform> terrain_params: array<vec4<f32>, 3>;
@group(constants::MATERIAL_BIND_GROUP) @binding(7)
var tile_array_1: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(8)
var tile_array_2: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(9)
var tile_array_3: texture_2d_array<f32>;

// Slope-aware albedo sampling: on a steep fragment the ground texture is also sampled from
// the side, so a cliff stops being a vertically smeared top-down texture.
//
// # The idea
//
// The ground UV is a top-down XZ projection (`sample_splat`'s `uv`), which stretches a tile
// by exactly `1 / |n.y|` along the slope — a 60-degree wall shows it at double length, a
// vertical one at infinity. A side projection has the opposite problem, so the fix is the
// standard one: sample the same tile a second (and third) time with the ZY and XY planes as
// UV and cross-fade by the normal.
//
// **What stays authentic:** this touches the albedo *UV only*. Which tile a fragment uses and
// how the four corner tiles are weighted remain the original's XZ lookup in `tile_map`
// (`sample_splat`), splat scale codes included, and the baked lightmap keeps its own XZ UV —
// both are authored data, while a projection is a drawing choice (ADR-0009).
//
// The vertical axis is the fragment's **world** Y, not a block-local one: the height field is
// absolute (`JMXVMAPM` stores a float height per vertex, no per-block origin), so there is no
// authored Y origin to restart the pattern at. That makes the side pattern continuous across
// block borders, and its phase relative to the XZ pattern is UNKNOWN rather than chosen.
struct SlopeProjection {
    // World Y and its screen-space derivatives — the vertical UV axis of both side planes.
    h: f32,
    dh: vec2<f32>,
    // Blend weights: `.x` = ZY plane, `.y` = the authentic XZ plane, `.z` = XY. Sums to 1,
    // and is exactly `(0, 1, 0)` wherever the feature is off or the ground is not steep —
    // which is what makes the flat case bit-identical to the single-sample path.
    w: vec3<f32>,
}

// Where "steep" begins, as `|n.y|`: 0.5 is 60 degrees from flat.
//
// This is a choice of how much stretch to tolerate, and it names its number: the XZ
// projection stretches the tile by `1 / |n.y|`, so 0.5 is "two times". Measured over the
// player's own `Map.pk2` (4644 `.m` files, 41.9M region-grid vertices, central differences on
// the same 20-unit grid `region_normal` builds its vertex normals from): **9.4%** of the map's
// footprint and 24.8% of its true surface area lean past 60 degrees, while the next step down
// (45 degrees, 1.41x stretch) would take 19.6% / 37.4% — twice the extra samples for a stretch
// that is barely visible. Everything flatter keeps exactly today's single sample.
const SLOPE_START: f32 = 0.5;

// Weights for the three planes. `terrain_params[2].w` selects: 0 = off, 1 = one extra
// projection, 2 = both.
fn slope_projection(world_normal: vec3<f32>, h: f32, dh: vec2<f32>) -> SlopeProjection {
    var out: SlopeProjection;
    out.h = h;
    out.dh = dh;
    out.w = vec3(0.0, 1.0, 0.0);
    let mode = terrain_params[2].w;
    if mode < 0.5 {
        return out;
    }
    // `abs` because only the axis matters, not the facing: terrain normals point up, but the
    // region's mesh is X-mirrored by its own transform and `prepare_world_normal` may flip a
    // back face.
    let n = abs(normalize(world_normal));
    // 0 while the surface is flatter than the threshold, 1 on a vertical wall.
    let lean = saturate((SLOPE_START - n.y) / SLOPE_START);
    if lean <= 0.0 {
        return out;
    }
    // Which side plane: a wall whose normal leans along X is textured with (Z, Y) and vice
    // versa. `n.x + n.z` is zero only for a vertical normal, which `lean` has already excluded.
    let zy_share = n.x / max(n.x + n.z, 1e-5);
    if mode < 1.5 {
        // One extra sample instead of two: the dominant side plane takes the whole slope
        // share. On a wall facing a cardinal direction that is the same picture as the full
        // mode; only a 45-degree-facing corner shows one plane instead of the mix.
        if zy_share >= 0.5 {
            out.w = vec3(lean, 1.0 - lean, 0.0);
        } else {
            out.w = vec3(0.0, 1.0 - lean, lean);
        }
    } else {
        out.w = vec3(lean * zy_share, 1.0 - lean, lean * (1.0 - zy_share));
    }
    return out;
}

@fragment
fn fragment(
    @builtin(front_facing) is_front: bool,
    in: VertexOutput,
) -> FragmentOutput {
    // Recover the fragment's position local to the region purely via modulo, the same trick the
    // original single-block shader used with `% 320.0` — it works regardless of *any* ancestor
    // translation because the mesh spans exactly one region. An earlier version tried to recover
    // this by subtracting the group's own Transform.translation, which forgot about the *region*
    // entity's own world-space translation stacked on top — that made the block lookup come out
    // constant almost everywhere (every block showing the same tile data). Modulo sidesteps
    // needing to know either offset at all. World X is negated because region entities are
    // X-mirrored.
    let region_x = (((-in.world_position.x) % REGION_SIZE) + REGION_SIZE) % REGION_SIZE;
    let region_z = ((in.world_position.z % REGION_SIZE) + REGION_SIZE) % REGION_SIZE;

    // Screen-space derivatives of the ground-plane coordinate, taken from the CONTINUOUS world
    // position instead of the wrapped region-local coordinate: the two differ only by a
    // per-region-constant offset, so in the interior the derivatives are identical — but at
    // region borders the modulo wrap makes dpdx jump by ±REGION_SIZE within a straddling 2x2
    // pixel quad, driving every derivative-based sample to the lowest mip and drawing a thin
    // dashed line along the border. Computed here, in uniform control flow, because dpdx/dpdy
    // are derivative ops.
    let ground = vec2(-in.world_position.x, in.world_position.z);
    let dground_dx = dpdx(ground);
    let dground_dy = dpdy(ground);
    // Vertical axis of the two side projections (see `slope_projection`), with its
    // derivatives taken here for the same reason as the ground plane's: dpdx/dpdy are
    // derivative ops and belong in uniform control flow.
    let height = in.world_position.y;
    let dheight = vec2(dpdx(height), dpdy(height));
    let slope = slope_projection(in.world_normal, height, dheight);

    var color = sample_splat(region_x, region_z, dground_dx, dground_dy, slope);
    // Modulate the ground albedo by the baked lightmap before lighting: shadowed ground stays dark
    // under any dynamic sun/ambient, adding SRO's static baked shadows as an albedo/occlusion layer.
    // [2].z is lightmap_enabled as a hard 0.0/1.0 (never fractional — see
    // `TerrainRenderParams::to_render_params`), off whenever `render_mode: pbr` is active. A real
    // branch is safe here despite `sample_lightmap` calling `textureSampleGrad`: unlike an
    // implicit-derivative `textureSample`, Grad takes its derivatives as explicit arguments
    // (computed once above, in uniform control flow), so it carries none of WGSL's
    // non-uniform-control-flow restriction — the same property `sample_splat`'s own fast path
    // already relies on (see its comment). Skipping the call when disabled avoids a lightmap
    // texture fetch on every terrain fragment whose result would otherwise just be discarded.
    var lightmap = vec3(1.0);
    if terrain_params[2].z > 0.0 {
        lightmap = sample_lightmap(region_x, region_z, dground_dx, dground_dy);
    }
    color = vec4(color.rgb * lightmap, color.a);

    // prepare pbr input
    var pbr_input = prepare_pbr(in, is_front, color);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color,
    );
    // Lighting model A/B (`terrain_params[1].x`, uniform across the draw). Both the mobile
    // port and (probably) the original render ground as albedo × baked lightmap with no
    // dynamic response at all, so both variants stay selectable for the playtest
    // (docs/rendering-mobile-shader-comparison.md, gap #6). A third one existed here
    // ("flat baked": base_color × ambient tint × exposure, no N·L) and was removed — it
    // scaled the whole ground by the ambient term alone, which in `render_mode: pbr` is
    // 30x smaller than in vanilla (`pbr_ambient_brightness` 100 vs `ambient_brightness`
    // 3000) and additionally arrives without the lightmap, so the ground went black
    // (ferdoran/openroad#5).
    // Fog is unaffected: it lives in main_pass_post_lighting_processing below.
    let lighting_mode = terrain_params[1].x;
    var out: FragmentOutput;
    if lighting_mode < 0.5 {
        // dynamic (current): full PBR sun + ambient over the baked lightmap
        out.color = apply_pbr_lighting(pbr_input);
        // SRO gives terrain its own ambient color, separate from the object ambient Bevy just
        // applied as `lights.ambient_color * diffuse`. Re-weight that term by the ratio of the
        // two; a ratio of 1 (the default, and whenever the environment system is off) is a
        // no-op. `view.exposure` is required: apply_pbr_lighting multiplies all lighting by it
        // at the end, and this correction is added after — without it the term is orders of
        // magnitude too bright.
        let ambient_correction = (ambient_ratio.rgb - vec3(1.0)) * lights.ambient_color.rgb * pbr_input.material.base_color.rgb * view.exposure;
        out.color = vec4(out.color.rgb + ambient_correction, out.color.a);
    } else {
        // fully baked: the lightmap (already multiplied into base_color above) is the
        // entire lighting, like the original fixed-function ground
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}

fn prepare_pbr(in: VertexOutput, is_front: bool, color: vec4<f32>) -> PbrInput {
    // prepare pbr input
    var pbr_input = pbr_input_new();
    // pbr_input_new() zeroes the MESH flags; copy the instance flags like
    // bevy's pbr_input_from_vertex_output does, or apply_pbr_lighting never
    // sees MESH_FLAGS_SHADOW_RECEIVER_BIT and terrain silently ignores every
    // shadow map (characters cast onto objects but not the ground). Only the
    // dynamic lighting mode consumes this; baked bypasses PBR lighting.
    pbr_input.flags = mesh[in.instance_index].flags;
    pbr_input.material.base_color = color;
//
//    // alpha discard
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color,
    );
//
    let double_sided = (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT) != 0u;
    pbr_input.frag_coord = in.position;
    pbr_input.world_position = in.world_position;
    pbr_input.world_normal = prepare_world_normal(
        in.world_normal,
        double_sided,
        is_front,
    );
    pbr_input.material.perceptual_roughness = 0.9;
    pbr_input.material.reflectance = vec3(0.1);
    pbr_input.material.metallic = 0.0;
    // pbr_input_new() leaves fog off by default; main_pass_post_lighting_processing()
    // only calls apply_fog() when this bit is set.
    pbr_input.material.flags |= STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
//    pbr_input.material.ior = 1.5;
//    pbr_input.material.thickness = 1.0;
//    pbr_input.material.diffuse_transmission = 0.0;
//    pbr_input.material.specular_transmission = 0.0;
//
    pbr_input.is_orthographic = view.clip_from_view[3].w == 1.0;
    @if(LOAD_PREPASS_NORMALS)
    pbr_input.N = prepass_utils::prepass_normal(in.position, 0u);
    @if(!LOAD_PREPASS_NORMALS)
    pbr_input.N = normalize(pbr_input.world_normal);

    pbr_input.V = calculate_view(in.world_position, pbr_input.is_orthographic);
    return pbr_input;
}

// Each material's lightmap covers exactly one region, so the region-local coordinates the
// fragment already derived map straight to UV. NOTE: V (world_z) orientation vs. the DDS row
// order is calibration-sensitive; flip `v` here if the baked shadows come out mirrored along Z.
fn sample_lightmap(region_x: f32, region_z: f32, duv_dx: vec2<f32>, duv_dy: vec2<f32>) -> vec3<f32> {
    let region = REGION_SIZE;
    // scale/offset from the shared params buffer, so the pending V-flip
    // calibration is a data change (see the binding-6 comment above)
    let uv = vec2(region_x / region, region_z / region) * terrain_params[0].xy + terrain_params[0].zw;
    // textureSampleGrad with the continuous ground-plane derivatives, NOT textureSample: the
    // region-local uv wraps 1→0 exactly at region borders, where implicit derivatives explode and
    // pick the lowest mip (the DDS decodes with a mip chain) — that was the dashed line along every
    // region border. Not textureSampleLevel(0) either: the low-res lightmap stretched over 1920
    // units minifies heavily at distance and would alias without mips. The 1x1 white fallback for
    // regions without a `.t` is unaffected.
    return textureSampleGrad(
        lightmap_tex,
        clamp_sampler,
        uv,
        duv_dx / region * terrain_params[0].xy,
        duv_dy / region * terrain_params[0].xy,
    ).rgb;
}

// Bilinear blend of the ground tiles named by the 4 map vertices surrounding this fragment.
//
// `duv_dx`/`duv_dy` are the ground-plane derivatives precomputed in `fragment()` — from the
// CONTINUOUS world position, not the wrapped region-local coordinate, so they don't blow up
// where the modulo wraps at region borders (which forced the lowest mip and drew dashed border
// lines). Passing them in also keeps every tile fetch a `textureSampleGrad`, so the uniform-tile
// fast path below is legal: branching around an *implicit*-derivative `textureSample` would be
// undefined per WGSL's uniformity rules, since derivatives need every fragment in a 2x2 quad to
// run the same fetch.
fn sample_splat(region_x: f32, region_z: f32, duv_dx: vec2<f32>, duv_dy: vec2<f32>, slope: SlopeProjection) -> vec4<f32> {
    // Which block, and where inside it. Vertices sit every 20 units, 17 per block edge, and the
    // fragment is strictly inside the block (region_x < REGION_SIZE => local < BLOCK_SIZE), so
    // `i0` lands in 0..15 and `i0 + 1` never leaves the block's own 17 vertices. That is what
    // makes the packed 102x102 `tile_map` reproduce the old per-block layers exactly.
    let block_col = min(u32(floor(region_x / BLOCK_SIZE)), 5u);
    let block_row = min(u32(floor(region_z / BLOCK_SIZE)), 5u);
    let local_x = region_x - f32(block_col) * BLOCK_SIZE;
    let local_z = region_z - f32(block_row) * BLOCK_SIZE;

    let fx = local_x / VERTEX_SPACING;
    let fz = local_z / VERTEX_SPACING;
    let i0 = min(u32(floor(fx)), 15u);
    let j0 = min(u32(floor(fz)), 15u);
    let tx = fx - f32(i0);
    let tz = fz - f32(j0);

    let base = vec2<u32>(block_col * BLOCK_VERTS + i0, block_row * BLOCK_VERTS + j0);
    let v00 = textureLoad(tile_map, base, 0);
    let v10 = textureLoad(tile_map, base + vec2<u32>(1u, 0u), 0);
    let v01 = textureLoad(tile_map, base + vec2<u32>(0u, 1u), 0);
    let v11 = textureLoad(tile_map, base + vec2<u32>(1u, 1u), 0);

    // BLOCK-local, not region-local: `sample` turns this into the tile's uv, and the authored
    // mapping restarts the tiling pattern at every block edge (`Mesh::from(&JMXVMAPM)` maps each
    // block's 17 vertices over `16 * splat_scale`). For splat scales 0.25/0.5/1 the two choices
    // differ by a whole number of repeats and would look identical, but at scale 2 and 4 they do
    // not — block-local is what the original client does.
    let uv = vec2(local_x, local_z);

    // Overwhelmingly the common case (median 8 distinct tiles per region, and most quads sit
    // well inside one tile's area): all four corners name the same tile, so the blend collapses
    // to a single sample.
    if v00.r == v10.r && v00.r == v01.r && v00.r == v11.r
        && v00.g == v10.g && v00.g == v01.g && v00.g == v11.g {
        return sample(1.0, v00.g, v00.r, uv, duv_dx, duv_dy, slope);
    }

    let w00 = (1.0 - tx) * (1.0 - tz);
    let w10 = tx * (1.0 - tz);
    let w01 = (1.0 - tx) * tz;
    let w11 = tx * tz;

    var color = vec4(0.0, 0.0, 0.0, 0.0);
    // Each corner carries its own tiling scale, so a tile used at different scales across a
    // region still renders correctly — that used to be approximated by a single per-texel scale
    // read from the dominant corner.
    if w00 > 0.0 { color += sample(w00, v00.g, v00.r, uv, duv_dx, duv_dy, slope); }
    if w10 > 0.0 { color += sample(w10, v10.g, v10.r, uv, duv_dx, duv_dy, slope); }
    if w01 > 0.0 { color += sample(w01, v01.g, v01.r, uv, duv_dx, duv_dy, slope); }
    if w11 > 0.0 { color += sample(w11, v11.g, v11.r, uv, duv_dx, duv_dy, slope); }

    return color;
}

// `tile` is the map's raw 10-bit tile id (see the header comment above `tile_map`'s declaration).
fn sample(contrib: f32, splat: u32, tile: u32, uv: vec2<f32>, duv_dx: vec2<f32>, duv_dy: vec2<f32>, slope: SlopeProjection) -> vec4<f32> {
    let splat_scale = get_splat_scale(splat);
    // 16, not 4: one texture repeat spans `320 * splat_scale` world units (a 320-unit block
    // has 16 20-unit tiles), so splat_scale 1.0 => exactly one repeat per block (1x1), 0.25 =>
    // 4x4, etc. This matches the documented reference UV mapping in `JMXVMAPM::from`
    // (`assets/m/mod.rs`, `let max = 16.0 * splat_scale`). The old `4.0` tiled 4x too densely;
    // it was only masked before because secondary textures silently fell back to scale 1.0.
    let max_uv = 16.0 * splat_scale;
    let scale = 1.0 / (20.0 * max_uv);
    let uv2 = uv * scale;

    // The authentic top-down projection, at full weight unless a side plane takes a share.
    var color = sample_tile(tile, uv2, duv_dx * scale, duv_dy * scale) * slope.w.y;
    // Both side branches are skipped on flat ground, which is where the cost bound comes from.
    // Branching around a `textureSampleGrad` is legal in non-uniform control flow because its
    // derivatives are arguments, not implicit — the same property the uniform-tile fast path
    // above and the lightmap branch in `fragment` already rely on.
    if slope.w.x > 0.0 {
        // ZY plane: the wall's horizontal axis is Z, its vertical axis world Y.
        let zy = vec2(uv.y, slope.h) * scale;
        color += sample_tile(
            tile,
            zy,
            vec2(duv_dx.y, slope.dh.x) * scale,
            vec2(duv_dy.y, slope.dh.y) * scale,
        ) * slope.w.x;
    }
    if slope.w.z > 0.0 {
        // XY plane: horizontal axis X.
        let xy = vec2(uv.x, slope.h) * scale;
        color += sample_tile(
            tile,
            xy,
            vec2(duv_dx.x, slope.dh.x) * scale,
            vec2(duv_dy.x, slope.dh.y) * scale,
        ) * slope.w.z;
    }

    return color * contrib;
}

// Tile `id` is layer `id % 256` of `tile_array_<id / 256>`. The array varies per fragment, so
// each branch samples with explicit gradients (legal in non-uniform control flow, unlike an
// implicit-derivative `textureSample`).
//
// The layer is clamped against the array's OWN depth, asked for per branch rather than passed
// in: `build_tile_arrays` sizes each array to its highest *defined* tile id, so on a drop whose
// `tile2d.ifo` stops before 1024 the ids above it address past the end, and WGSL leaves an
// out-of-range array layer indeterminate. Asking the bound texture means the clamp cannot drift
// from what was actually built -- and it is why there is no uniform to keep in step here.
fn sample_tile(tile: u32, uv: vec2<f32>, ddx: vec2<f32>, ddy: vec2<f32>) -> vec4<f32> {
    let slot = tile & 255u;
    switch (min(tile, 1023u) >> 8u) {
        case 0u: {
            let layer = existing_layer(slot, textureNumLayers(tile_array_0));
            return textureSampleGrad(tile_array_0, tile_sampler, uv, layer, ddx, ddy);
        }
        case 1u: {
            let layer = existing_layer(slot, textureNumLayers(tile_array_1));
            return textureSampleGrad(tile_array_1, tile_sampler, uv, layer, ddx, ddy);
        }
        case 2u: {
            let layer = existing_layer(slot, textureNumLayers(tile_array_2));
            return textureSampleGrad(tile_array_2, tile_sampler, uv, layer, ddx, ddy);
        }
        default: {
            let layer = existing_layer(slot, textureNumLayers(tile_array_3));
            return textureSampleGrad(tile_array_3, tile_sampler, uv, layer, ddx, ddy);
        }
    }
}

// Turns an id past the end of a trimmed array into the last layer it really has. That is
// defined-but-arbitrary rather than correct: the tile has no texture either way, and the
// `build_tile_arrays` warning is what says so. Landing on the white fallback instead would mean
// building one extra layer per gapped array -- a separate choice, not needed for correctness.
fn existing_layer(slot: u32, layers: u32) -> i32 {
    return i32(min(slot, max(layers, 1u) - 1u));
}

// Code -> repeat factor, all five from the shared params buffer so every
// code is live-calibratable (render-debug sliders). Playtest verdict
// 2026-08-10: the vertex "Scale" field does NOT drive tiling — every code
// matches vanilla at a constant 0.25 (one repeat per 80 world units), so
// all five default to 0.25 and the field's real meaning is UNKNOWN. The
// constant also removes the hard tiling seams a varying per-texel factor
// produced. See docs/formats/mapm-jmxvmapm.md.
fn get_splat_scale(scale: u32) -> f32 {
    var actual = terrain_params[1].w; // code 16 = the common default
    switch scale {
        case 0u: {
            actual = terrain_params[1].y;
        }
        case 8u: {
            actual = terrain_params[1].z;
        }
        case 16u, default {
            actual = terrain_params[1].w;
        }
        case 24u: {
            actual = terrain_params[2].x;
        }
        case 32u: {
            actual = terrain_params[2].y;
        }
    }

    return actual;
}

