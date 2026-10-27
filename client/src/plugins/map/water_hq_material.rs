use bevy::asset::Asset;
use bevy::color::Srgba;
use bevy::math::{Vec2, Vec4};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::{Handle, Image};
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use crate::plugins::skybox::SKY_COLOR_HEX;

// Idea: high-quality (default/high graphics) water is a `StandardMaterial` extended with a
// custom fragment shader (`water_hq.wesl`), so Bevy's built-in screen-space specular
// transmission keeps doing the heavy lifting of actually rendering what's behind the water,
// refracted by `ior`/`thickness` on `base`. The extension only bolts on the parts that make
// flat transmissive glass read as *moving water*: a scrolling diffuse, a real (procedurally
// generated, see `tools/src/bin/gen_water_normal`) tangent-space ripple normal map sampled at
// two independently scrolling octaves, and a Fresnel blend toward a sky-colored tint at
// grazing angles to fake a reflection. See the shader file itself for the per-step breakdown.
// This mirrors Bevy's own `examples/3d/ssr.rs` water demo (`ExtendedMaterial<StandardMaterial,
// Water>` + a scrolling normal map), minus that example's `ScreenSpaceReflections` + deferred
// rendering — deferred is a renderer-wide switch that would also drop `specular_transmission`
// support, so real reflections are a separate, deliberate follow-up rather than bundled here.
//
// Two of the parts are depth cues rather than surface detail, and they are *fed into* the
// base material rather than blended on top of it: the fragment shader measures the water
// column out of the depth prepass and writes `thickness`-relative `attenuation_distance`,
// `attenuation_color` and `specular_transmission` into its own copy of the material before
// lighting runs. So `attenuation_color` set here would be overwritten every frame and is
// deliberately left at bevy's default; `attenuation_distance` is the one the configuration
// owns, because it is also the on/off switch (see `absorption_distance`).
//
// The low-graphics tier (`graphics.water.quality: low`) deliberately does not use this: it
// keeps the flat, non-transmissive `LowQualityWaterMaterial` in `water_material.rs`, which
// has neither the transmissive pass's per-frame main-texture copy nor the SSR march, and
// which participates in the depth prepass so it gets early-Z.
pub type HighQualityWaterMaterial =
    ExtendedMaterial<bevy::prelude::StandardMaterial, WaterExtension>;

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct WaterExtension {
    #[uniform(100)]
    pub settings: WaterHqSettings,
    /// Procedurally generated tileable ripple normal map (`assets/textures/water_normal.png`,
    /// see `tools/src/bin/gen_water_normal`) — not an SRO game asset, since the original
    /// client's water textures don't include a real normal map to reuse. Must be loaded with
    /// `is_srgb = false` like any normal map.
    #[texture(101)]
    #[sampler(102)]
    pub normal_map: Handle<Image>,
}

/// Water column, in world units, over which the shore fades — the shipped default.
///
/// A look parameter, not a measurement, and the ranking report says so (`§3.2`: "die
/// Absorptionskurve selbst ist ein freier Parameter"). The anchor is this game's own scale:
/// one terrain block is 320 units (`water_patch_mesh`), so 24 units is a fourteenth of a
/// block — wide enough that a beach reads as a gradient from a normal camera distance, narrow
/// enough that a knee-deep ford still shows its water.
pub const DEFAULT_SHORE_FADE: f32 = 24.0;

/// Depth, in world units, at which the transmitted light has fallen to the water texture's
/// own colour once — i.e. what `graphics.water.absorption = 1.0` means.
///
/// Same class of value as [`DEFAULT_SHORE_FADE`] and the same anchor. 64 units is a fifth of
/// a terrain block; with the player's `water101.ddj` (linear ~0.10/0.23/0.22, so red is
/// absorbed about twice as fast as green and blue — real water's own order) that is a gentle
/// gradient over a pond and a dark bottom in a lake.
pub const ABSORPTION_FULL_DEPTH: f32 = 64.0;

/// `graphics.water.absorption` (0..=1) as bevy's `StandardMaterial::attenuation_distance`.
///
/// Inverse, because the wire protocol of Beer-Lambert is a *distance*: twice the strength is
/// half the distance. Zero returns infinity, which is also the switch — bevy only sets
/// `ATTENUATION_ENABLED` for a finite distance (`pbr_material.rs:1206`), so the whole
/// absorption branch disappears rather than being computed and multiplied by one.
pub fn absorption_distance(absorption: f32) -> f32 {
    if absorption <= 0.0 {
        return f32::INFINITY;
    }
    ABSORPTION_FULL_DEPTH / absorption.min(1.0)
}

/// Std140 layout, field order must match the `WaterHqSettings` struct in `water_hq.wesl`.
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct WaterHqSettings {
    /// Reflection tint blended in at grazing angles; alpha is the max blend strength.
    pub sky_tint: Vec4,
    /// UV/sec scroll of the base diffuse texture (the visible water surface).
    ///
    /// The original does the same thing in the same unit:
    /// `Data/shader/oceanwater.vsh:110-113` multiplies the time register by
    /// `c17.xy`, takes `frc`, and adds it to the vertex UV. Its value is the
    /// one in the file's **header comment** (`:21`, "basetexcoord distortion
    /// x0, y0, x1, y1"), not a `def` — the file declares none, so the client
    /// uploads these at runtime and this is the shader author's stated intent
    /// rather than the shipped number.
    pub scroll_speed_a: Vec2,
    /// UV/sec scroll of the second, ripple-only sample.
    ///
    /// Same header comment (`:21`), second pair. **Deliberate deviation**: in
    /// the shipped original the four lines that would apply it are commented
    /// out (`oceanwater.vsh:116-120`) and stage 1 gets an unscrolled second UV
    /// set instead (`mov oT1, v4`). We have no second UV set — our ripples are
    /// a second octave of the one normal map — so the scroll stays live here.
    pub scroll_speed_b: Vec2,
    /// UV scale applied before `scroll_speed_b`, kept non-integer to avoid the second normal
    /// map octave lining up with the first and canceling itself out.
    ///
    /// **Ours, and it has no original counterpart** — the original separates
    /// its two bump samples by giving stage 1 its own UV channel
    /// (`oceanwater.vsh:121`) instead of re-scaling stage 0's. Re-scaling is
    /// what a single normal map needs for the same effect.
    pub ripple_tiling: f32,
    /// Multiplier on the combined tangent-space slope from both normal map octaves before
    /// it's rebuilt into a world-space normal — turn down for calmer water.
    pub distortion_strength: f32,
    /// Fresnel exponent; higher values narrow the reflection tint to shallower angles.
    ///
    /// **Ours by necessity**: the original ships *no* fresnel. Its pixel
    /// shader is two texture loads and `mov r0, r1`
    /// (`Data/shader/oceanwater.psh:4-11`); the environment map, the fresnel
    /// term and the specular highlight are inside a `/* … */` block
    /// (`:12-40`). There is no value to adopt, so this is a modern addition
    /// under ADR-0009, not an unsourced number.
    pub fresnel_power: f32,
    /// Water column, in world units, over which the surface fades into whatever is behind it
    /// (`graphics.water.shore_fade`; 0 switches the fade off). It is a *column* width, not a
    /// screen width: the shore softens over the first `shore_fade` units of water the view ray
    /// passes through, so a shallow slope fades over more ground than a steep one, which is
    /// what makes a beach and a quay wall look different.
    ///
    /// Took the slot the explicit `_padding` float held, so the std140 size is unchanged —
    /// see `the_uniform_layout_still_fits_three_std140_rows`.
    pub shore_fade: f32,
}

impl Default for WaterHqSettings {
    fn default() -> Self {
        let sky = Srgba::hex(SKY_COLOR_HEX).unwrap();
        Self {
            sky_tint: Vec4::new(sky.red, sky.green, sky.blue, 0.55),
            // `oceanwater.vsh:21` header comment, both pairs. Ours were
            // (0.008, 0.004) and (-0.013, 0.02) — invented, and about four
            // times slower than what the original's author wrote down.
            scroll_speed_a: Vec2::new(0.031, 0.04),
            scroll_speed_b: Vec2::new(-0.03, 0.02),
            ripple_tiling: 2.3,
            distortion_strength: 1.2,
            fresnel_power: 4.0,
            shore_fade: DEFAULT_SHORE_FADE,
        }
    }
}

impl MaterialExtension for WaterExtension {
    fn fragment_shader() -> ShaderRef {
        "shaders/water_hq.wesl".into()
    }

    /// The water must stay out of the depth prepass: its fragment shader raymarches that very
    /// prepass for screen-space reflections, and if the water wrote its own depth there, every
    /// reflection ray would immediately hit the water surface it started from (and the lakebed
    /// would vanish from the refraction, since transmission also depends on the water not
    /// occluding what's behind it).
    fn enable_prepass() -> bool {
        false
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// `shore_fade` took the slot the explicit `_padding` float held. The Rust struct and the
    /// WESL one have to agree byte for byte, and a mismatch is a silently wrong binding that
    /// no compile error and no unit test of behaviour would catch — so pin the size: three
    /// std140 rows, exactly as before this field existed.
    #[test]
    fn the_uniform_layout_still_fits_three_std140_rows() {
        assert_eq!(
            <WaterHqSettings as ShaderType>::min_size().get(),
            48,
            "water_hq.wesl's WaterHqSettings must be the same 48 bytes"
        );
    }

    /// Absorption is a strength in the configuration and a distance on the wire, and the two
    /// run opposite ways. Zero must come out as infinity rather than as a huge number: that is
    /// what clears bevy's `ATTENUATION_ENABLED` flag and removes the branch instead of
    /// computing it and multiplying by one.
    #[test]
    fn absorption_strength_inverts_into_a_distance_and_zero_switches_it_off() {
        assert!(absorption_distance(0.0).is_infinite());
        assert!(absorption_distance(-1.0).is_infinite());
        assert_eq!(absorption_distance(1.0), ABSORPTION_FULL_DEPTH);
        assert_eq!(absorption_distance(0.5), ABSORPTION_FULL_DEPTH * 2.0);
        // out of range clamps rather than inverting past full strength
        assert_eq!(absorption_distance(4.0), ABSORPTION_FULL_DEPTH);
        // monotonic: more strength is never more distance
        let mut previous = f32::INFINITY;
        for step in 1..=10 {
            let distance = absorption_distance(step as f32 / 10.0);
            assert!(distance <= previous, "{distance} > {previous}");
            previous = distance;
        }
    }

    /// The shipped water must still be water: the shore fade is on by default (that is the
    /// point of the slice) and the reflection strength the Video pane reads as "On" is
    /// untouched by it.
    #[test]
    fn the_shipped_defaults_carry_a_shore_fade_and_keep_the_reflection_strength() {
        let settings = WaterHqSettings::default();
        assert_eq!(settings.shore_fade, DEFAULT_SHORE_FADE);
        assert!(settings.shore_fade > 0.0, "0 is the off switch");
        assert_eq!(
            settings.sky_tint.w, 0.55,
            "map::water_options reads this as On"
        );
    }
    /// The two scroll speeds are the original's own, and the test exists so a
    /// later "that looks slow, bump it" does not quietly turn them back into
    /// invented numbers. `Data/shader/oceanwater.vsh:21` names
    /// `{basetexcoord distortion x0, y0, x1, y1} (0.031, 0.04, -0.03, 0.02)`,
    /// and `:110-113` is the live code that uses the first pair as UV/sec.
    ///
    /// The two values with no counterpart stay ours on purpose and are pinned
    /// separately, so a change to either reads as a decision rather than a
    /// drift.
    #[test]
    fn the_scroll_speeds_are_the_originals_and_the_rest_is_ours_on_purpose() {
        let settings = WaterHqSettings::default();
        assert_eq!(settings.scroll_speed_a, Vec2::new(0.031, 0.04));
        assert_eq!(settings.scroll_speed_b, Vec2::new(-0.03, 0.02));

        // no counterpart in the original shader — see the field docs
        assert_eq!(settings.ripple_tiling, 2.3);
        assert_eq!(settings.fresnel_power, 4.0);
    }
}
