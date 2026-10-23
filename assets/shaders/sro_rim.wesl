// Fresnel rim highlight for hovered / click-selected world entities.
//
// Extends the standard PBR fragment: after normal lighting, add a soft edge
// glow that is bright where the surface faces away from the camera (the
// silhouette) and fades toward the center, reading as a subtle outline. The
// base material also carries a small emissive lighten (set CPU-side when the
// highlight clone is built), so the whole model brightens slightly while the
// rim traces its edge. Everything else (skinning, shadows, prepass, light
// probes) stays the stock PBR pipeline via `pbr_input_from_standard_material`.
import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};
import bevy_pbr::render::pbr_fragment::pbr_input_from_standard_material;
import bevy_pbr::render::pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing};

@if(VISIBILITY_RANGE_DITHER)
import bevy_pbr::render::pbr_functions::visibility_range_dither;

struct RimSettings {
    // rgb = rim tint (scene-linear), a = strength
    color: vec4<f32>,
    // fresnel falloff exponent: higher = thinner edge
    power: f32,
    // 0 = absolute add (a scene-linear constant — vanishes against HDR
    // daylight), 1 = relative (lit x (1 + rim·fresnel)), which keeps the
    // rim-to-lit ratio at any exposure/time of day
    mode: f32,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> rim_settings: RimSettings;

@fragment
fn fragment(vertex_output: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var in = vertex_output;
    // LOD crossfade, the same call bevy's own fragment entry makes first
    // (`bevy_pbr-0.20.0/src/render/pbr.wesl:53`): without it a mesh part whose
    // `VisibilityRange` is in its fade band (`commands::mesh_visibility_range`
    // gives every part one) hard-cuts at the far edge instead of dithering out.
    @if(VISIBILITY_RANGE_DITHER)
    visibility_range_dither(in.position, in.visibility_range_dither);
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    // `pbr_input_from_standard_material` samples the texture but runs no
    // alpha handling, so a fragment shader that replaces the stock one has to
    // do it — exactly as `sro_uv_scroll.wesl`, `terrain_splat.wesl` and
    // `water_hq.wesl` already do. Two things happen here, and both matter for
    // character-class meshes, which are the only always-on users of this
    // material: `AlphaMode::Mask` texels below the cutoff are discarded in the
    // main pass (the depth prepass already discards them, so without this the
    // two passes disagree), and for every non-blend alpha mode the output alpha
    // is forced to 1. Skipping it leaks the raw sampled texture alpha into the
    // render target, which is how an opaque skin material can come out looking
    // see-through.
    //
    // `sro_sheen.wesl` is the deliberate exception: it consumes the sampled
    // alpha as its sheen mask and then sets `base_color.a = 1.0` itself.
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color,
    );

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);

    // fresnel: 0 facing the camera, 1 at grazing angles (the silhouette)
    let facing = saturate(dot(normalize(pbr_input.N), normalize(pbr_input.V)));
    let fresnel = pow(1.0 - facing, rim_settings.power);
    let rim = rim_settings.color.rgb * rim_settings.color.a * fresnel;
    let rim_absolute = out.color.rgb + rim;
    let rim_relative = out.color.rgb * (vec3<f32>(1.0) + rim);
    out.color = vec4<f32>(mix(rim_absolute, rim_relative, rim_settings.mode), out.color.a);

    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
