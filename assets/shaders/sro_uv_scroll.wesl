// Continuous UV scroll for SRO TexAni resources (waterfalls, canal water).
//
// The original client applies the TexAni ModData's D3D texture-transform
// matrix scaled by time each frame; observed data is always a pure UV
// translation, so this is the stock PBR fragment with the UVs offset by
// `uv_speed * globals.time` before sampling. Shader-clock driven: the
// material uniform never changes, so instances batch and there is no
// per-frame CPU/bind-group churn (docs/perf-future-levers.md).
import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};
import bevy_pbr::render::pbr_fragment::pbr_input_from_standard_material;
import bevy_pbr::render::pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing};
import bevy_pbr::render::mesh_view_bindings::globals;

@if(VISIBILITY_RANGE_DITHER)
import bevy_pbr::render::pbr_functions::visibility_range_dither;

struct UvScrollSettings {
    // uv/sec, from the TexAni matrix translation slots
    uv_speed: vec2<f32>,
    _padding: vec2<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> uv_scroll: UvScrollSettings;

@fragment
fn fragment(vertex_output: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var in = vertex_output;
    // LOD crossfade, the same call bevy's own fragment entry makes first
    // (`bevy_pbr-0.20.0/src/render/pbr.wesl:53`): without it a mesh part whose
    // `VisibilityRange` is in its fade band (`commands::mesh_visibility_range`
    // gives every part one) hard-cuts at the far edge instead of dithering out.
    @if(VISIBILITY_RANGE_DITHER)
    visibility_range_dither(in.position, in.visibility_range_dither);
    // fract() keeps the offset small (the sampler repeats anyway) so f32
    // UV precision survives globals.time growing over long sessions
    in.uv += fract(uv_scroll.uv_speed * globals.time);

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color,
    );

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
