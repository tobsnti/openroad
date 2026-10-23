// Instanced variant of sro_effect.wesl for effect particles
// (`plugins/effects/instanced.rs`). Same unlit look — texture * tint, the
// fixed-function texture-op clamps and the LDR-additive emulation — but every
// per-particle value arrives through an instance-rate vertex buffer instead of
// bevy's per-mesh data: the world matrix, the packed 0xAARRGGBB tint (what the
// MeshTag carries on the per-entity path) and the TextureSlide UV offset/scale
// (what a private material uniform carried there). One draw renders a whole
// effect's worth of particles that share a mesh and a material, so the
// particles never become individual render-world objects.
//
// Must stay in step with sro_effect.wesl's fragment stage; blending, depth
// write and culling come from `material::apply_effect_render_state`.

import bevy_pbr::render::view_transformations::position_world_to_clip;
@if(DEPTH_PREPASS)
import bevy_pbr::prepass::utils::prepass_depth;
@if(DEPTH_PREPASS)
import bevy_pbr::render::view_transformations::depth_ndc_to_view_z;

@group(2) @binding(0) var effect_texture: texture_2d<f32>;
@group(2) @binding(1) var effect_sampler: sampler;
// x = color multiplier, y = alpha multiplier, z = LDR mode, w = soft-fade
// distance in world units — see sro_effect.wesl
@group(2) @binding(2) var<uniform> effect_params: vec4<f32>;

struct Vertex {
    @location(0) position: vec3<f32>,
    @if(VERTEX_UVS_A)
    @location(2) uv: vec2<f32>,
    @if(VERTEX_COLORS)
    @location(5) color: vec4<f32>,
    // per instance (see `EffectInstance` in instanced.rs)
    @location(8) world_from_local_0: vec4<f32>,
    @location(9) world_from_local_1: vec4<f32>,
    @location(10) world_from_local_2: vec4<f32>,
    @location(11) world_from_local_3: vec4<f32>,
    @location(12) uv_offset_scale: vec4<f32>,
    @location(13) tint: u32,
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

fn unpack_tint(tag: u32) -> vec4<f32> {
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
    let world_from_local = mat4x4<f32>(
        vertex.world_from_local_0,
        vertex.world_from_local_1,
        vertex.world_from_local_2,
        vertex.world_from_local_3,
    );
    let world_position = world_from_local * vec4(vertex.position, 1.0);
    out.clip_position = position_world_to_clip(world_position.xyz);
    @if(VERTEX_UVS_A)
    out.uv = vertex.uv * vertex.uv_offset_scale.zw + vertex.uv_offset_scale.xy;
    @if(!VERTEX_UVS_A)
    out.uv = vec2(0.5, 0.5);
    @if(VERTEX_COLORS)
    out.color = vertex.color * unpack_tint(vertex.tint);
    @if(!VERTEX_COLORS)
    out.color = unpack_tint(vertex.tint);
    return out;
}

// Soft-particle fade — see sro_effect.wesl for the reasoning. Reachable here
// for the same reason the quantization is: this pipeline specializes bevy's
// mesh pipeline (`instanced.rs` `specialize`) and keeps its view groups, so it
// inherits both the `DEPTH_PREPASS` shader def and the group-0 prepass texture.
@if(DEPTH_PREPASS)
fn soft_particle_fade(frag_coord: vec4<f32>, fade_distance: f32) -> f32 {
    if fade_distance <= 0.0 {
        return 1.0;
    }
    let scene_z = depth_ndc_to_view_z(prepass_depth(frag_coord, 0u));
    let own_z = depth_ndc_to_view_z(frag_coord.z);
    return saturate((own_z - scene_z) / fade_distance);
}
@if(!DEPTH_PREPASS)
fn soft_particle_fade(frag_coord: vec4<f32>, fade_distance: f32) -> f32 {
    return 1.0;
}

// Inverse of srgb_to_linear, quantized to 1/255 steps — see sro_effect.wesl.
fn quantize_srgb_255(linear: vec3<f32>) -> vec3<f32> {
    let lower = linear * 12.92;
    let higher = 1.055 * pow(linear, vec3(1.0 / 2.4)) - 0.055;
    let srgb = select(higher, lower, linear <= vec3(0.0031308));
    return srgb_to_linear(floor(srgb * 255.0 + 0.5) / 255.0);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(effect_texture, effect_sampler, in.uv) * in.color;
    // D3D9 texture stages saturate their outputs before blending (see
    // sro_effect.wesl).
    var rgb = min(color.rgb * effect_params.x, vec3(1.0));
    // Fade before the quantization, and into the color on mode 2 — the same
    // two rules as sro_effect.wesl, which this file must stay in step with.
    let fade = soft_particle_fade(in.clip_position, effect_params.w);
    let a = min(color.a * effect_params.y, 1.0) * fade;
    if effect_params.z >= 0.5 {
        if effect_params.z < 1.5 {
            rgb *= a;
        } else {
            rgb *= fade;
        }
        rgb = quantize_srgb_255(rgb);
    }
    return vec4(rgb, a);
}
