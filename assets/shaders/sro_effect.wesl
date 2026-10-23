// Unlit shader for SRO effect nodes (JMXVEFF). Texture * animated tint *
// optional vertex color (trails fade per-vertex). The tint arrives per
// instance as a packed 0xAARRGGBB MeshTag (sRGB bytes, converted to linear
// here) so one immutable material can be shared by every node with the same
// texture/blend — animating a material uniform instead would re-prepare its
// bind group every frame and break batching. Blending, depth-write and
// culling are configured per material via pipeline specialization in
// `plugins/effects/material.rs`.

import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, get_tag};
import bevy_pbr::render::view_transformations::position_world_to_clip;
@if(DEPTH_PREPASS)
import bevy_pbr::prepass::utils::prepass_depth;
@if(DEPTH_PREPASS)
import bevy_pbr::render::view_transformations::depth_ndc_to_view_z;

struct SroEffectUniform {
    // xy = UV offset, zw = UV scale
    uv_offset_scale: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> material: SroEffectUniform;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var effect_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var effect_sampler: sampler;
// x = color multiplier, y = alpha multiplier — the fixed-function texture-stage
// color/alpha op scale (MODULATE2X = 2, MODULATE4X = 4) times the additive HDR
// intensity; zw unused
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var<uniform> effect_params: vec4<f32>;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @if(VERTEX_UVS_A)
    @location(2) uv: vec2<f32>,
    @if(VERTEX_COLORS)
    @location(5) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

// Matches bevy's Srgba::gamma_function_inverse (alpha stays linear).
fn srgb_to_linear(srgb: vec3<f32>) -> vec3<f32> {
    let lower = srgb / 12.92;
    let higher = pow((srgb + 0.055) / 1.055, vec3(2.4));
    return select(higher, lower, srgb <= vec3(0.04045));
}

fn tag_tint(instance_index: u32) -> vec4<f32> {
    let tag = get_tag(instance_index);
    let argb = vec4(
        f32((tag >> 24u) & 0xffu),
        f32((tag >> 16u) & 0xffu),
        f32((tag >> 8u) & 0xffu),
        f32(tag & 0xffu),
    ) / 255.0;
    return vec4(srgb_to_linear(argb.yzw), argb.x);
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = get_world_from_local(vertex.instance_index);
    let world_position = mesh_position_local_to_world(world_from_local, vec4(vertex.position, 1.0));
    out.clip_position = position_world_to_clip(world_position.xyz);
    @if(VERTEX_UVS_A)
    out.uv = vertex.uv * material.uv_offset_scale.zw + material.uv_offset_scale.xy;
    @if(!VERTEX_UVS_A)
    out.uv = vec2(0.5, 0.5);
    @if(VERTEX_COLORS)
    out.color = vertex.color * tag_tint(vertex.instance_index);
    @if(!VERTEX_COLORS)
    out.color = tag_tint(vertex.instance_index);
    return out;
}

// Inverse of srgb_to_linear, quantized to 1/255 steps — the original's 8-bit
// backbuffer precision. Quantizing must happen in sRGB space, where those
// steps are perceptually uniform; doing it in linear space posterizes large
// soft gradients (the aura's scale-15 shine plates) into hard-edged rings.
fn quantize_srgb_255(linear: vec3<f32>) -> vec3<f32> {
    let lower = linear * 12.92;
    let higher = 1.055 * pow(linear, vec3(1.0 / 2.4)) - 0.055;
    let srgb = select(higher, lower, linear <= vec3(0.0031308));
    return srgb_to_linear(floor(srgb * 255.0 + 0.5) / 255.0);
}

// Soft particles: scale a fragment's contribution by how far it still is from
// the opaque surface behind it, so a smoke/fire/aura plate that intersects the
// ground stops ending in a straight line. `effect_params.w` is the fade
// distance in world units and doubles as the switch (<= 0 = off, the shipped
// `low` preset and any camera the player left the prepass off on).
//
// Needs the camera's depth prepass: `DEPTH_PREPASS` is bevy's own view-key def
// (`bevy_pbr-0.20.0/src/render/mesh.rs:3582-3583`), so without a prepass the
// whole path is compiled out and the fallback is exactly today's hard edge —
// not a half-applied fade. `prepass_depth` handles the multisampled texture
// variant, which matters because `high`/`ultra` run MSAA 4.
@if(DEPTH_PREPASS)
fn soft_particle_fade(frag_coord: vec4<f32>, fade_distance: f32) -> f32 {
    if fade_distance <= 0.0 {
        return 1.0;
    }
    let scene_z = depth_ndc_to_view_z(prepass_depth(frag_coord, 0u));
    let own_z = depth_ndc_to_view_z(frag_coord.z);
    // View-space z grows negative away from the camera, so the surface behind
    // this fragment is the *more* negative value: a plate hanging free in air
    // gets a large positive difference and stays untouched, one lying on the
    // ground gets ~0 and disappears.
    return saturate((own_z - scene_z) / fade_distance);
}
@if(!DEPTH_PREPASS)
fn soft_particle_fade(frag_coord: vec4<f32>, fade_distance: f32) -> f32 {
    return 1.0;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(effect_texture, effect_sampler, in.uv) * in.color;
    // D3D9 fixed-function texture stages saturate their outputs to [0, 1]:
    // the MODULATE2X/4X scales clamp per stage, BEFORE the blend factors see
    // them (e.g. the Sun aura's circle runs an alpha MODULATE2X whose result
    // must not double past 1).
    var rgb = min(color.rgb * effect_params.x, vec3(1.0));
    // The soft fade rides in on the alpha, which is where the blend reads a
    // fragment's contribution for every combo whose source factor is SrcAlpha.
    // It lands BEFORE the quantization below on purpose: the 1/255 step is the
    // original's 8-bit backbuffer, so a faded wisp has to fall *under* that
    // threshold and vanish. Quantizing first and fading after would scale the
    // step itself and move a threshold material.rs reproduces deliberately.
    let fade = soft_particle_fade(in.clip_position, effect_params.w);
    let a = min(color.a * effect_params.y, 1.0) * fade;
    // LDR-additive emulation (effect_params.z, see material.rs): the blend
    // factors are (OneMinusDst, One), so the D3D source factor moves in
    // here — mode 1 premultiplies SrcAlpha, mode 2 keeps ONE. Quantization
    // reproduces the 8-bit backbuffer: sub-threshold wisps vanish instead
    // of lingering in the HDR target and blooming.
    if effect_params.z >= 0.5 {
        if effect_params.z < 1.5 {
            // Mode 1 (D3D SrcAlpha): the premultiply already carries the fade.
            rgb *= a;
        } else {
            // Mode 2 (D3D ONE): this blend never reads alpha, so the fade has
            // to reach the color or it would be a no-op on One/One nodes.
            rgb *= fade;
        }
        rgb = quantize_srgb_255(rgb);
    }
    return vec4(rgb, a);
}
