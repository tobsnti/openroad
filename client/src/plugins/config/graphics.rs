//! Graphics/quality settings that need to be adjustable without recompiling.
//!
//! Currently only bloom. The idea: the renderer already writes genuine HDR
//! values — emissive BMT props at 2.0 (`assets/bmt/material.rs`) and the `+N`
//! enhancement shine at 4.0-8.0 (`assets/bmt/sheen.rs`, `sro_sheen.wesl`) — but
//! the cameras historically had no `Hdr` marker, so all of it clipped flat at
//! 1.0. Enabling bloom flips the main pass to a float target, which is what
//! actually lets those emitters read as "glowing" instead of "white".

use bevy::anti_alias::contrast_adaptive_sharpening::ContrastAdaptiveSharpening;
use bevy::post_process::bloom::Bloom;
use bevy::reflect::Reflect;
use serde::Deserialize;

use crate::assets::m::block_splat_material::TerrainSlopeSampling;
use crate::plugins::map::water_hq_material::DEFAULT_SHORE_FADE;

/// The two lighting modes the client can run, mutually exclusive — see
/// [`crate::plugins::environment::EnvironmentSettings`] (the live/hotkey
/// half) and `plugins::environment::apply_render_mode` (what each mode
/// actually switches: the Sun's directional light + shadow maps, the terrain
/// lightmap, the ambient model, and the terrain shader's lighting mode).
/// `Reflect` (unlike this file's other config enums) because it also lives
/// on `EnvironmentSettings`, which the dev inspector reflects.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum RenderMode {
    /// SRO-faithful: baked terrain lightmap + ambient lighting only — no
    /// directional light, no shadow cascades.
    #[default]
    Vanilla,
    /// Dynamic cascaded shadows + PBR directional lighting over the terrain
    /// (lightmap off).
    Pbr,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct GraphicsSettings {
    /// The quality preset whose values fill in every graphics key this file
    /// leaves unset (`plugins/config/preset.rs`). `auto` picks one from the
    /// GPU.
    #[serde(default)]
    pub preset: super::preset::QualityPreset,
    /// What `preset` resolved to and why, set by `ClientConfig::from_file`;
    /// not configured.
    #[serde(skip)]
    pub resolved_preset: Option<super::preset::ResolvedPreset>,
    #[serde(default)]
    pub bloom: BloomSettings,
    #[serde(default)]
    pub sheen: SheenGraphicsSettings,
    #[serde(default)]
    pub rim: RimGraphicsSettings,
    /// Vanilla vs. PBR lighting — see [`RenderMode`].
    #[serde(default)]
    pub render_mode: RenderMode,
    #[serde(default)]
    pub terrain: TerrainGraphicsSettings,
    #[serde(default)]
    pub shadows: ShadowSettings,
    #[serde(default)]
    pub foliage: FoliageSettings,
    #[serde(default)]
    pub dungeon: DungeonGraphicsSettings,
    #[serde(default)]
    pub fog: FogGraphicsSettings,
    /// View, fog and cull distances — see [`ViewSettings`].
    #[serde(default)]
    pub view: ViewSettings,
    /// How long streamed-out assets stay loaded — see [`StreamingSettings`].
    #[serde(default)]
    pub streaming: StreamingSettings,
    #[serde(default)]
    pub objects: ObjectLodSettings,
    #[serde(default)]
    pub msaa: MsaaSamples,
    /// FXAA post pass on the window cameras — see [`FxaaSetting`].
    #[serde(default)]
    pub fxaa: FxaaSetting,
    /// Contrast-adaptive sharpening behind FXAA — see [`Sharpening`].
    #[serde(default)]
    pub sharpening: Sharpening,
    /// Mip levels dropped from world textures — see [`TextureDetail`].
    #[serde(default)]
    pub texture_detail: TextureDetail,
    /// Anisotropic filtering clamp — see [`Anisotropy`].
    #[serde(default)]
    pub anisotropy: Anisotropy,
    /// Particle effect quality — see [`EffectQuality`].
    #[serde(default)]
    pub effect_quality: EffectQuality,
    /// Soft particles: how many world units before an opaque surface an
    /// effect plate starts fading out, so smoke, fire and auras stop ending
    /// in a straight line on the ground (`effects::EffectSoftFade`). 0 = off.
    ///
    /// Needs `depth_prepass`; without it the shader path is compiled out and
    /// this value does nothing. Forced to 0 on `EffectQuality::Low` — effects
    /// carry the frame's heaviest overdraw, and the cost here is one depth
    /// read per effect fragment.
    ///
    /// Not transcribed from the original, which had no such parameter: a
    /// deliberate addition under ADR-0009, so the number is a free choice and
    /// is anchored rather than measured — the presets use 10, about half the
    /// height of an unscaled ~19-unit character
    /// (`scenes/intro_v2/character_select.rs:110`), which is the scale effect
    /// plates are authored at. Whether that is the *best* band needs a
    /// playtest. 0 (the default) is off. Live.
    #[serde(default)]
    pub effect_soft_fade: f32,
    /// The sun's lens flare (the original's Lens Flare option), composited
    /// from your own `Map.pk2` `sun/lens*` sprites
    /// (`environment/lens_flare.rs`). Six small additive quads plus a
    /// 10 Hz occlusion ray, so cheap, but off by default because the client
    /// never drew it before. Live.
    #[serde(default)]
    pub lens_flare: bool,
    /// Bevy's GPU light clustering. Off (the default) assigns the few point
    /// lights there are on the CPU instead: ~0.5 ms per frame cheaper on an
    /// iGPU with nothing visible lost (`plugins/light_clustering.rs`). Live.
    #[serde(default)]
    pub gpu_light_clustering: bool,
    #[serde(default)]
    pub water: WaterSettings,
    #[serde(default)]
    pub render_scale: RenderScale,
    #[serde(default)]
    pub tonemapping: TonemappingConfig,
    /// Depth-only prepass on the window cameras, so the main pass shades each
    /// pixel once: early-Z then rejects every hidden fragment before its
    /// material runs. Facing the Jangan West waterfall the opaque pass shaded
    /// ~10M fragments for a 2M-pixel screen (map objects stacked behind each
    /// other, drawn in no particular depth order) — ~3.5 ms of GPU time.
    #[serde(default)]
    pub depth_prepass: bool,
    /// Bevy's bindless material slabs (`StandardMaterial` and friends binding their textures
    /// from shared arrays indexed per fragment). Bevy turns them on wherever the GPU offers the
    /// features; off here (the default) withholds those features so materials use ordinary
    /// bind groups. Facing the Jangan West waterfall on an AMD iGPU that was ~1.7 ms of GPU time
    /// per frame (57-59 -> 63-65 FPS) with identical output. Restart-only.
    #[serde(default)]
    pub bindless_materials: bool,
    /// The sky environment map on the window cameras (`environment::reflections`): every lit
    /// pixel samples its diffuse and specular cubemaps. Off (the default) skips it entirely:
    /// facing the Jangan West waterfall that was 64 -> 71 FPS (~1.6 ms of GPU time) with no
    /// visible difference in vanilla lighting — the original client had no image-based lighting
    /// either. Sheen materials keep their own sphere maps either way. Restart-only.
    #[serde(default)]
    pub sky_reflections: bool,
    /// Cut-out edges (leaves, grass cards, grates) dither across the MSAA
    /// samples instead of being decided yes/no per pixel
    /// (`assets::bmt::material::cutout_alpha_mode`). Foliage is the largest
    /// alpha-tested area in the frame, so its crawling edges are the most
    /// visible aliasing left after FXAA. A deliberate deviation: the original
    /// tests alpha hard (ref 0x80, `D3DCMP_GREATEREQUAL`).
    ///
    /// Two measured conditions, both of which make it a no-op rather than a
    /// cost: without MSAA bevy falls back to the hard test itself
    /// (`bevy_pbr-0.20.0/src/material.rs:697-700`), and with
    /// [`Self::depth_prepass`] on it is withheld here, because that pass does
    /// not do alpha-to-coverage (`src/prepass/mod.rs:696`) and discards A2C at
    /// 0.05 (`src/render/pbr_prepass_functions.wesl:17,30`) — a fringe texel
    /// would write whole-pixel depth and the samples the main pass leaves
    /// uncovered could no longer be filled by what is behind the leaf.
    /// Restart-only: materials keep the mode they were loaded with.
    #[serde(default)]
    pub foliage_alpha_to_coverage: bool,
}

/// Which water shader the streamed water planes use.
///
/// `high` is the current look: a transmissive `StandardMaterial` extension with
/// refraction, two scrolling ripple-normal octaves and a screen-space
/// reflection raymarch (`water_hq_material.rs`). It is the most expensive
/// per-pixel path in the renderer, and two of its costs are paid whether or not
/// much water is on screen: transmission puts it in Bevy's Transmissive phase,
/// which copies the entire main texture every frame any water is visible, and
/// the surface is excluded from the depth prepass (it raymarches that prepass),
/// so it gets no early-Z and pays full shading for every overdrawn pixel.
///
/// `low` keeps only the scrolling surface (`water_material.rs`) — no
/// transmission, no reflection march, no ripple samples, and ordinary alpha
/// blending in place of refraction.
///
/// Neither is the faithful setting in the ADR 0009 sense: the original client's
/// water is fixed-function D3D8 with a scrolling texture, which `low` is much
/// closer to, while `high` is a deliberate modern enhancement.
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct WaterSettings {
    pub quality: WaterQuality,
    /// The high tier's reflection blend (the original's Water Reflection
    /// option). `sky_tint.a` in `water_hq.wesl` is the single factor the
    /// shader multiplies its Fresnel term by, so `false` zeroes the whole
    /// reflection contribution — the raymarched one and the flat sky-tint
    /// fallback alike — without rebuilding anything
    /// (`map::water_options`). Live, high tier only. On by default: that is
    /// the look the client has always shipped.
    pub reflection: bool,
    /// Water column, in world units, over which the high tier's surface fades
    /// into whatever is behind it instead of meeting the terrain on a hard
    /// line; `0.0` switches the fade off.
    ///
    /// A *column* width, not a screen width: the shore softens over the first
    /// `shore_fade` units of water the view ray passes through, so a shallow
    /// beach fades over more ground than a quay wall does. The default is the
    /// material's (`DEFAULT_SHORE_FADE`) — the number belongs to the look, the
    /// same split `reflection` uses.
    ///
    /// Like `absorption` it needs the camera's depth prepass to measure a
    /// column at all, so it is visible where `depth_prepass` is on — `ultra`
    /// only today. Without one the water keeps the image it had before these
    /// cues existed instead of showing half of one, and moving that switch is
    /// its own measurement (`GFX-MODERN-RANGLISTE.md` §3.2).
    pub shore_fade: f32,
    /// How strongly the high tier's water absorbs the light coming through it,
    /// `0.0..=1.0`, where `0.0` is off and `1.0` means the transmitted light
    /// has fallen to the water texture's own colour after
    /// `ABSORPTION_FULL_DEPTH` world units.
    ///
    /// A strength here, a distance on the wire — `absorption_distance` inverts
    /// it, because that distance is also bevy's on/off flag for the whole
    /// Beer-Lambert branch. The *colour* absorbed toward is not configurable:
    /// it is the player's own water texture (`water_hq.wesl`).
    pub absorption: f32,
    /// Multiplier on how fast the high tier's surface flows, `1.0` being the
    /// speed the original's own shader author wrote down
    /// (`WaterHqSettings::scroll_speed_a`/`_b`, sourced from
    /// `Data/shader/oceanwater.vsh:21`).
    ///
    /// The two vectors themselves are deliberately *not* configurable: they
    /// have an origin, and a config key per component would invite guessing
    /// them again. What a player (or a comparison shot) may want is the one
    /// thing the origin cannot decide — whether that speed reads well on this
    /// surface — so the knob is a scale, and `0` freezes the water. High tier
    /// only, live, like `reflection` and `shore_fade`.
    pub flow_speed: f32,
    /// Tint and sight distance while the camera itself is below a drawn water
    /// plane (`environment::underwater`).
    ///
    /// The camera carries no collision and a shore walks right down into the
    /// lake bed, so it does go under; without this the view is a flat sheet
    /// with a hard edge. Neither the colour nor the reach is configurable,
    /// because neither is invented: the tint is the live ENVI water colour of
    /// the region and time of day, the reach is the water surface's own
    /// extinction length (`ABSORPTION_FULL_DEPTH`).
    ///
    /// The v1.188 client has no underwater view at all, so `false` is the
    /// faithful image and `true` is the stated improvement (ADR 0009). Off in
    /// `low`: while submerged it swaps the linear distance band for an
    /// exponential one, and that preset is where every per-pixel term counts.
    /// Live.
    pub underwater_view: bool,
}

impl Default for WaterSettings {
    fn default() -> Self {
        Self {
            quality: WaterQuality::default(),
            reflection: true,
            shore_fade: DEFAULT_SHORE_FADE,
            absorption: 1.0,
            flow_speed: 1.0,
            underwater_view: true,
        }
    }
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum WaterQuality {
    Low,
    #[default]
    High,
}

/// Resolution the 3D view renders at, as a fraction of the window. The HUD is
/// always composited at the window's native size.
///
/// The idea: at 3440x1440 the frame is fill-bound — the measured cost tracks
/// pixels, not draw calls or geometry (`docs/perf-baselines.md`). Pixels are
/// therefore the largest lever left, but lowering the *window* resolution
/// softens the text and icons, which is where players actually notice. Scaling
/// only the 3D pass keeps the HUD razor-sharp and spends the blur where an
/// existing FXAA post pass already softens edges.
///
/// The mechanism is a scale-factor trick rather than a coordinate rewrite
/// (`camera::render_scale_target`): the offscreen image is sized
/// `window_physical * scale` and its `ImageRenderTarget::scale_factor` is set
/// to `window.scale_factor() * scale`, so the camera's *logical* viewport still
/// equals the window's. Every `world_to_viewport` / `viewport_to_world` caller —
/// the picking ray, nameplates, hit counts, world anchors — keeps working in
/// window coordinates with no per-site conversion.
///
/// Above 1.0 the same mechanism is supersampling: the 3D view renders larger
/// than the window and the composite blit resolves it down, which is the one
/// anti-aliasing lever that needs no vendor, no feature flag and no SDK — it
/// works on Metal, on Vulkan/DX12 and in the browser alike. It is never picked
/// by a preset (`auto` least of all): the cost is quadratic in the scale, so
/// 2.0 asks the GPU for four times the fragments.
///
/// 1.0 (the default) retargets nothing at all and is the original behaviour;
/// this is the ADR 0009 "deliberate, stated deviation" case, so it is opt-in.
/// It doubles as the measurement instrument: mirrored onto
/// `RenderDebugSettings`, a resolution ladder is five BRP calls in one session
/// rather than five relaunches, which is what separates the frame's fixed cost
/// from its per-pixel cost.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(transparent)]
pub struct RenderScale(pub f32);

impl Default for RenderScale {
    fn default() -> Self {
        Self(1.0)
    }
}

impl RenderScale {
    /// Below this the 3D view is unreadably coarse, and the image would stop
    /// being a rendering of the scene in any useful sense.
    pub const MIN: f32 = 0.25;

    /// Above this, supersampling stops buying visible quality and only costs
    /// fragments: 2.0 already resolves four samples per output pixel. The
    /// target size has its own, separate ceiling — see
    /// `camera::MAX_TARGET_DIMENSION`.
    pub const MAX: f32 = 2.0;

    /// The usable scale: values that cannot describe a target at all
    /// (non-finite, zero, negative) fall back to 1.0 with a warning rather
    /// than silently producing a broken image; values merely outside
    /// [`MIN`](Self::MIN)..=[`MAX`](Self::MAX) clamp into it, so "as low as it
    /// goes" and "as high as it goes" both land somewhere sane. Exactly 1.0
    /// means "do not retarget".
    pub fn factor(self) -> f32 {
        if !self.0.is_finite() || self.0 <= 0.0 {
            bevy::log::warn_once!(
                "graphics.render_scale: {} is not a usable scale; rendering at full resolution",
                self.0
            );
            return 1.0;
        }
        if !(Self::MIN..=Self::MAX).contains(&self.0) {
            bevy::log::warn_once!(
                "graphics.render_scale: {} is outside {}..={}; clamping",
                self.0,
                Self::MIN,
                Self::MAX
            );
        }
        self.0.clamp(Self::MIN, Self::MAX)
    }

    /// Whether this scale asks for an offscreen target at all. Only exactly
    /// 1.0 does not: both a smaller and a larger target need one.
    pub fn is_native(self) -> bool {
        self.factor() == 1.0
    }
}

/// Percentage-closer filtering kernel for shadow edges.
///
/// The idea: this is a **per-lit-pixel** cost, not a shadow-map cost, so it
/// scales with screen resolution rather than with `map_size` or `cascades`.
/// That is what the measurements show — the `enable_shadows` A/B costs 1.51 ms
/// at 1920x1080 and 2.37 ms at 3440x1440, tracking the 2.39x pixel ratio far
/// more closely than the fixed 2x2048² of shadow map being rendered
/// (`docs/perf-baselines.md`).
///
/// `gaussian` is Bevy's default and what this shipped with: Castaño's 9-sample
/// filter approximating a 5x5 kernel. `hardware_2x2` is a single hardware
/// comparison sample — 1 tap instead of 9.
///
/// Neither is "the faithful setting" in the ADR 0009 sense, because the
/// original client has no PCF at all. But the trade is smallest exactly where
/// the game spends its time: in the default *vanilla* shadow mode only the
/// player casts, into a tight cascade close to the camera, and a hard-edged
/// shadow there is arguably nearer the original than a soft one.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ShadowFiltering {
    /// One hardware comparison sample. Fastest; hard edges.
    Hardware2x2,
    /// Bevy's default: 9 samples, ~5x5 kernel.
    #[default]
    Gaussian,
}

impl ShadowFiltering {
    pub fn to_method(self) -> bevy::light::ShadowFilteringMethod {
        match self {
            ShadowFiltering::Hardware2x2 => bevy::light::ShadowFilteringMethod::Hardware2x2,
            ShadowFiltering::Gaussian => bevy::light::ShadowFilteringMethod::Gaussian,
        }
    }
}

/// Tonemapping curve applied to the main view.
///
/// Where it runs depends on the camera. With bloom on, the camera has an `Hdr`
/// target and the curve is a full-screen post pass. With `bloom.enabled:
/// false` there is no HDR target, and Bevy instead compiles the curve *into
/// every material's fragment shader* (`TONEMAP_IN_SHADER`, with the deband
/// dither riding along): a LUT sample per shaded fragment, so it costs per
/// fragment times overdraw, not per screen pixel. Facing dense vegetation,
/// where the opaque pass shades ~5x the screen's pixels (`docs/perf-remote.md`),
/// that is more than one full-screen pass would be, spent on values already
/// clamped to 0..1.
///
/// `none` removes it from both places (and the `DebandDither` with it).
/// It is **not** pixel-identical — it is a deliberate, stated deviation — but it
/// is the *more* faithful one under ADR 0009: the original client is
/// fixed-function D3D8 and applies no tonemap curve at all.
///
/// Keep `tony_mc_mapface` if bloom is on; that is the case the curve is for.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TonemappingConfig {
    /// No curve, no full-screen pass.
    None,
    /// Bevy's default, and what this shipped with.
    #[default]
    TonyMcMapface,
    /// Cheap curve, noticeable hue shifting. Kept as the middle option for
    /// anyone who wants highlight rolloff without the LUT.
    ReinhardLuminance,
}

impl TonemappingConfig {
    pub fn to_tonemapping(self) -> bevy::core_pipeline::tonemapping::Tonemapping {
        use bevy::core_pipeline::tonemapping::Tonemapping;
        match self {
            TonemappingConfig::None => Tonemapping::None,
            TonemappingConfig::TonyMcMapface => Tonemapping::TonyMcMapface,
            TonemappingConfig::ReinhardLuminance => Tonemapping::ReinhardLuminance,
        }
    }
}

/// Multisample count for every camera that draws to the window.
///
/// The idea: MSAA is the renderer's single largest fixed cost — it multiplies
/// rasterization *and* the depth prepass by the sample count, which is what a
/// bandwidth-limited integrated GPU is worst at. The main cameras also run an
/// `Fxaa` post pass, so 4x MSAA is largely paying twice for edge quality.
///
/// **This value must be identical on every camera targeting the window.** Bevy
/// keys the shared main texture on `(target, usage, format, msaa)`, so a camera
/// that disagrees gets its *own* texture; the UI `Camera2d` has
/// `clear_color: None`, so it would then composite into an empty texture and
/// blit that over the window — erasing the 3D view and leaving only whatever
/// drew last. That is the mechanism behind the long-standing "forcing MSAA off
/// makes terrain and objects vanish, only the skybox draws" note, and it is the
/// same trap `camera::attach_bloom` documents for the *format* axis of that key.
/// `camera::apply_window_camera_msaa` is what keeps them in agreement, so this
/// never has to be remembered at a spawn site again.
///
/// Valid: 1 (off), 2, 4, 8. Anything else falls back to 4 with a warning.
#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(transparent)]
pub struct MsaaSamples(pub u32);

impl Default for MsaaSamples {
    fn default() -> Self {
        // Bevy's own default, and what this shipped with.
        Self(4)
    }
}

impl MsaaSamples {
    pub fn to_msaa(self) -> bevy::render::view::Msaa {
        match self.0 {
            1 => bevy::render::view::Msaa::Off,
            2 => bevy::render::view::Msaa::Sample2,
            4 => bevy::render::view::Msaa::Sample4,
            8 => bevy::render::view::Msaa::Sample8,
            other => {
                bevy::log::warn_once!(
                    "graphics.msaa: {other} is not a valid sample count (1, 2, 4 or 8); using 4"
                );
                bevy::render::view::Msaa::Sample4
            }
        }
    }
}

/// How much of each world texture's mip chain is uploaded
/// (`graphics.texture_detail`; `assets::texture_detail`).
///
/// `full` uploads every authored level. `half` starts one level down: model
/// textures and ground tiles at half their width and height, about a quarter
/// of the VRAM. `quarter` goes one further, to about a sixteenth. The HUD's
/// textures always stay whole. Applies to textures loaded afterwards, so
/// restart to apply it everywhere.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TextureDetail {
    #[default]
    Full,
    Half,
    Quarter,
}

impl TextureDetail {
    /// Mip levels dropped from the top of each chain.
    pub fn skipped_mips(self) -> u32 {
        match self {
            Self::Full => 0,
            Self::Half => 1,
            Self::Quarter => 2,
        }
    }
}

/// Particle effect quality (`graphics.effect_quality`; the original's Effect
/// Quality option): the share of each emitter's authored particle cap that
/// may be alive at once (`effects::EffectQualityScale`). Live.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum EffectQuality {
    Low,
    Medium,
    /// Every authored particle, as the client always drew them.
    #[default]
    High,
}

impl EffectQuality {
    /// Fraction of the authored particle caps. Low keeps enough of a spray
    /// to read as the same effect; medium thins it visibly but mildly.
    pub fn particle_scale(self) -> f32 {
        match self {
            Self::Low => 0.4,
            Self::Medium => 0.7,
            Self::High => 1.0,
        }
    }
}

/// Maximum anisotropic filtering of world textures (`graphics.anisotropy`):
/// 1 (off, plain trilinear), 2, 4, 8 or 16.
///
/// Anisotropic filtering takes extra texture samples on surfaces seen at a
/// grazing angle, which in this game is most of the ground. 4 is what the
/// client shipped with. An old card is short of texture bandwidth, so 1 is
/// the low preset's choice; a strong one can afford 16. Read once when the
/// renderer starts (the default image sampler and the ground-tile sampler),
/// so restart-only.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(transparent)]
pub struct Anisotropy(pub u16);

impl Default for Anisotropy {
    fn default() -> Self {
        Self(4)
    }
}

impl Anisotropy {
    /// The clamp to give a sampler: one of the powers of two wgpu accepts,
    /// anything else falling back to 4 with a warning.
    pub fn clamp(self) -> u16 {
        match self.0 {
            1 | 2 | 4 | 8 | 16 => self.0,
            other => {
                bevy::log::warn_once!(
                    "graphics.anisotropy: {other} is not 1, 2, 4, 8 or 16; using 4"
                );
                4
            }
        }
    }
}

/// Strength of the contrast-adaptive sharpening pass on the window cameras
/// (`graphics.sharpening`), 0.0 (off) to 1.0.
///
/// The pass exists because the two cheapest quality levers this client has
/// both pay in sharpness: [`FxaaSetting`] smooths edges by blurring them, and
/// [`RenderScale`] below 1.0 renders fewer pixels than the window shows. CAS
/// raises local contrast where there is detail and leaves flat areas alone,
/// which is what the pairing was built for — see
/// `camera::apply_window_camera_msaa` for the wiring and the one limit.
///
/// The original client has no post-processing, so any value above 0.0 is a
/// deliberate deviation (ADR 0009) whose purpose is readability. 0.6 is bevy's
/// own default (`contrast_adaptive_sharpening/mod.rs:60`) and is kept as ours;
/// the preset ladder moves it the one way the measurement justifies: hardest
/// where the image is upscaled (`low: 0.8`, `render_scale: 0.75`), softest
/// where it is rendered at native resolution with every other quality lever on
/// (`ultra: 0.5`). Read when a camera is spawned, so a change takes effect on
/// the next scene.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(transparent)]
pub struct Sharpening(pub f32);

impl Default for Sharpening {
    fn default() -> Self {
        Self(0.6)
    }
}

impl Sharpening {
    /// The component to hang on a main-view camera.
    ///
    /// Out-of-range and non-finite values clamp into 0.0..=1.0 rather than
    /// reaching the shader: above 1.0 bevy's own note is "extreme artifacts
    /// and fireflies" (`contrast_adaptive_sharpening/mod.rs:94`), and it
    /// clamps there too — doing it here as well is what lets the component say
    /// what is actually in effect. At 0.0 bevy drops the pass entirely in
    /// extract (`:87-90`), so "off" costs nothing.
    pub fn to_cas(self) -> ContrastAdaptiveSharpening {
        let strength = if self.0.is_finite() {
            self.0.clamp(0.0, 1.0)
        } else {
            bevy::log::warn_once!(
                "graphics.sharpening: {} is not a usable strength; using 0",
                self.0
            );
            0.0
        };
        ContrastAdaptiveSharpening {
            enabled: strength > 0.0,
            sharpening_strength: strength,
            ..Default::default()
        }
    }
}

/// Whether the window cameras run Bevy's FXAA post pass (`graphics.fxaa`).
///
/// It is a full-screen pass, about 1 ms at 1920x1080 on an integrated GPU
/// (`docs/perf-remote.md`), and the only edge smoothing left once MSAA is
/// off. On by default, as it always was. The low preset turns it off: on a
/// DX10-class card a full-screen pass costs proportionally more, and that
/// preset also renders below window resolution, where the upscale softens
/// edges anyway. A newtype so the derived `GraphicsSettings::default()`
/// agrees with the serde default.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(transparent)]
pub struct FxaaSetting(pub bool);

impl Default for FxaaSetting {
    fn default() -> Self {
        Self(true)
    }
}

/// Per-part distance LOD for map objects (`commands::mesh_visibility_range`).
///
/// The idea: an object's mesh parts are culled on a "constant screen size"
/// heuristic — a part whose largest dimension is `S` is only a few pixels wide
/// past roughly `S * factor`, so that is where it stops being worth a draw
/// item. The fog cull (`map::objects::cull_fogged_objects`) already hides whole
/// objects at the fully-fogged distance; this trims the cheaper-to-lose *parts*
/// inside that range, which is what the GPU-preprocessing and instance-buffer
/// systems pay per frame.
///
/// These are exposed because the right values are hardware-dependent, not
/// data-derived: the original client has no equivalent (its draw distance is a
/// fixed fog range), so there is no reference value to match. The defaults are
/// the tuning this shipped with; lowering `factor` or `min_distance` culls more
/// aggressively and is the cheapest draw-item reduction available on a
/// fill-limited GPU.
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct ObjectLodSettings {
    /// Maps a part's largest dimension to its cull distance in world units.
    pub factor: f32,
    /// Floor on that distance: nothing closer than this is ever part-culled,
    /// so small props stay visible in the immediate area regardless of size.
    pub min_distance: f32,
    /// Width of the dither/crossfade band before the cull distance. Every
    /// material a mesh part can carry honours it: bevy's own fragment entry
    /// for [`StandardMaterial`], and `sro_rim`/`sro_sheen`/`sro_uv_scroll`
    /// through the same `visibility_range_dither` call.
    pub fade: f32,
    /// View distance (world units) for `res/nature/` resources — trees, grass,
    /// flowers. 0 = unlimited (they reach the fog like everything else). Their
    /// alpha-tested cards are the heaviest overdraw in dense views: facing the
    /// Jangan West waterfall, 2000 took 12.6M shaded fragments to 10.1M
    /// (68 -> 79 FPS) and 1200 to 7.3M (86 FPS), with the fog already hiding
    /// most of what is cut.
    pub nature_view_distance: f32,
    /// Share of `res/nature/` placements (trees, grass, flowers) that spawn
    /// at all, 0..=1. 1 (the default) is every placement, as authored. Lower
    /// thins vegetation evenly by a stable hash per placement, cutting the
    /// alpha-tested overdraw within `nature_view_distance` too, which the
    /// distance cap cannot. It applies to regions loaded afterwards.
    pub nature_density: f32,
    /// Animate map props (swaying trees, flags, lamps, wheels; the original's
    /// Dynamic Animation option). Off holds them still, which saves the bone
    /// animation of every prop in view: at the Jangan West waterfall that was
    /// ~1,550 of 1,668 animated bones (`docs/perf-remote.md`). Characters
    /// always animate. Live.
    pub animate: bool,
}

impl Default for ObjectLodSettings {
    fn default() -> Self {
        Self {
            factor: 150.0,
            min_distance: 1200.0,
            fade: 600.0,
            nature_view_distance: 0.0,
            nature_density: 1.0,
            animate: true,
        }
    }
}

impl ObjectLodSettings {
    /// The cull range of a mesh part whose largest dimension is `extent`.
    /// It is fully visible up to the size-scaled distance, then dithered out
    /// over `fade` — in every material a part can carry (see [`fade`]).
    ///
    /// [`fade`]: ObjectLodSettings::fade
    ///
    /// `ceiling` is the cull distance (`ViewRange::static_cull`), which no part
    /// outlives. For a `nature` part, `nature_view_distance` lowers that
    /// ceiling when set. `use_aabb` is false, so the distance is measured to
    /// the part's origin. That origin equals the object's world anchor, since
    /// every level in between carries a default transform, which matches how
    /// the whole-object and animation culls measure.
    pub fn part_range(
        &self,
        extent: f32,
        nature: bool,
        ceiling: f32,
    ) -> bevy::camera::visibility::VisibilityRange {
        let max = if nature && self.nature_view_distance > 0.0 {
            self.nature_view_distance.min(ceiling)
        } else {
            ceiling
        };
        let cull = (extent.max(0.0) * self.factor).clamp(self.min_distance.min(max), max);
        bevy::camera::visibility::VisibilityRange {
            start_margin: 0.0..0.0,
            end_margin: (cull - self.fade).max(0.0)..cull,
            use_aabb: false,
        }
    }
}

/// Trees, grass, flowers and other vegetation (`res/nature/...`).
pub fn is_nature_path(path: &bevy::asset::AssetPath) -> bool {
    path.path()
        .to_string_lossy()
        .to_ascii_lowercase()
        .replace('\\', "/")
        .contains("res/nature/")
}

/// Distance fog (`plugins/map/terrain/rendering.rs`, driven per-frame by
/// `plugins/environment/`).
///
/// The idea: Bevy's `DistanceFog` can add a *directional in-scattering* term —
/// a sun glow blended into the fog color, `pow(dot(view_dir, sun_dir),
/// exponent)` scaled by the light's color. Our fog itself is
/// `FogFalloff::Linear` between the two planes the environment profile
/// carries, which is the shape of the data: `EnvironmentProfile` holds
/// `fog_near_plane`, `fog_far_plane` and a `fog_color` graph and **no**
/// scattering coefficients (`assets/ifo/environment.rs`).
///
/// What the original does on the terrain path is a different equation, and the
/// player's own shaders spell it out — `Data/shader/scattervs10.vsh` and
/// `terrainps14.psh`, both headed "(c) 2002 Nathaniel Hoffman, Kenneth J.
/// Mitchell and Arcot J. Preetham":
///
/// ```text
/// Phase1(theta) = 1 + cos^2(theta)                        ; Rayleigh
/// Phase2(theta) = (1-g^2)/(1+g-2g*cos(theta))^(3/2)       ; Henyey-Greenstein
/// E = exp(-(beta_1+beta_2) * s) * terrain_reflectance
/// I = (beta'_1*Phase1 + beta'_2*Phase2) * (1-E1) / (beta_1+beta_2) * frac_ins
/// Final_color = Surf_color * E + I          ; terrainps14.psh
/// Sky         = I                           ; skyps10.psh — "just inscattering"
/// ```
///
/// So the earlier claim here — that the original has "no such term, so 0.0 is
/// the faithful value" — was wrong twice over: the original's in-scattering is
/// unconditional (the sky is *nothing but* `I`), and its directionality comes
/// from two phase functions rather than a `pow()` lobe. 0.0 is a **deliberate
/// deviation**, not fidelity.
///
/// It is a deviation we keep, because the term reads as *gloss*, not haze: the
/// sun's color arrives premultiplied by its 10 000 lux illuminance
/// (`plugins/map/mod.rs`), it is multiplied by the shadow map (so it lands in
/// patches on geometry rather than uniformly in the air), and it scales with
/// the fog blend, so it is strongest on the most distant objects. With the
/// default exponent it is a ~7-degree-wide hot disc that sweeps across distant
/// terrain as the camera turns — indistinguishable from a specular lobe on
/// objects that should be flat and fogged out. A faithful `I` would need the
/// phase functions and the `beta` pair, and **those values are not in the
/// player's files** — the shader declares them as constant registers the client
/// fills at runtime (`CV_BETA_1`, `CV_BETA_2`, `CV_HG`). Inventing them is what
/// ADR 0009 forbids; reproducing the equation without them is not possible from
/// data we have.
///
/// Version note: the profile format grows by appended graphs
/// (`terrain_shadow_color` at fileVersion 1003, `sky_bottom_color` and
/// `water_color` at 1002, `graph12` at 1001), but the three fog fields predate
/// all of them. So fog reads the same across every profile version we parse,
/// while a client older than 1002 has no `sky_bottom_color` to gradient with.

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct FogGraphicsSettings {
    /// Strength of the sun's in-scattering halo in fog. **0.0 is our choice,
    /// not the original's** — see the type's note: the original in-scatters
    /// unconditionally. 0.5 restores the value this shipped with before the
    /// halo was found to read as gloss on distant objects.
    pub sun_scattering: f32,
    /// Angular tightness of that halo — the `pow()` exponent on
    /// `dot(view_dir, sun_dir)`. Only has an effect when `sun_scattering > 0`.
    /// Lower is a broader, more atmospheric glow; the previous shipped value
    /// of 100 is half-strength at ~6.7 degrees, which is what made it read as a
    /// specular highlight rather than scattered light.
    ///
    /// This number is **ours with a reason, not a transcription**: the original
    /// shapes its lobe with `Phase2`'s asymmetry `g`, which lives in the
    /// runtime-filled `CV_HG` register and in none of the player's files, so
    /// there is nothing here to copy. What it *can* be measured against is the
    /// shape, and 30.0 turns out to be the best `cos^n` match to a
    /// Henyey-Greenstein lobe of **g = 0.78** — mid-band for atmospheric Mie
    /// scattering (0.75-0.9), so the guess was a good one for the lobe's
    /// *width*.
    ///
    /// The bound, both lobes normalised to the same peak over 0-90 degrees:
    ///
    /// ```text
    /// level 0.9 .. 0.25 (the bright core)   within 1.25 degrees
    /// level 0.10                            5.75 degrees off
    /// level 0.05                           12.05 degrees off
    /// worst amplitude error   0.077 of peak at 26.3 degrees
    /// share of the contribution beyond 20 deg   HG 0.50  vs  cos^30 0.14
    /// beyond 30 deg                             HG 0.32  vs  cos^30 0.01
    /// ```
    ///
    /// So `pow` reproduces the core and loses the halo: half of the
    /// Henyey-Greenstein contribution sits past 20 degrees where `cos^30` puts
    /// a seventh of it. **The error floors at ~6.5% of peak and does not fall
    /// as `g` rises** — fitting the amplitude as well (which is what
    /// `sun_scattering` does) buys about 1.5 points and moves the floor by
    /// nothing. That is the useful part: no exponent and no gain make a `pow`
    /// lobe an HG profile, so **a real halo needs its own term**, not a better
    /// exponent. It is also why this term reads as gloss — a `pow` lobe is a
    /// disc, HG is a disc *with* a halo, and a disc on distant geometry is what
    /// a specular highlight looks like.
    ///
    /// Computed from the equation in `Data/shader/scattervs10.vsh`; `g` itself
    /// is still unmeasured, so the numbers hold "if `g` is in the Mie band".
    /// The register comment reads `1-g^2, 1+g, 2g` where textbook HG wants
    /// `(1-g)^2, 1+g^2, 2g` — one character apart and not separable from data
    /// the shader only consumes. Under the register reading 30.0 means g = 0.93
    /// and the floor is 0.08; the conclusion is the same either way.
    pub sun_scattering_exponent: f32,
}

impl Default for FogGraphicsSettings {
    fn default() -> Self {
        FogGraphicsSettings {
            sun_scattering: 0.0,
            sun_scattering_exponent: 30.0,
        }
    }
}

/// How far the overworld is streamed, fogged and drawn (`graphics.view`),
/// all in world units. Derived and clamped into
/// [`crate::plugins::map::view_range::ViewRange`], which is what the
/// streaming, fog and cull systems read.
///
/// The defaults are the distances the client has always used: the fog
/// starts two regions out (3840) and is opaque at three (5760), terrain is
/// streamed one region beyond that, and the camera's far plane sits one
/// region past the fog. The original client's own draw distance is likewise
/// fog-bound (its `config.ifo` far plane is 5500). The fields are
/// configurable because the right values depend on the hardware rather than
/// the game data. Every streamed region costs RAM, draw items and per-frame
/// visibility work, so a weak machine wants less and a strong one can afford
/// more.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct ViewSettings {
    /// How far terrain and map objects are streamed in and can be drawn.
    /// The load ring is `ceil(view_distance / 1920)` regions around the
    /// camera's region, plus one more ring for streaming head start.
    /// Valid range 1920..=9600.
    pub view_distance: f32,
    /// Where the fog begins when `envi_fog` is off. With it on, the
    /// environment profile's near plane decides instead.
    pub fog_start: f32,
    /// Where the fog is fully opaque, and the ceiling the environment
    /// profiles' fog may reach (they can only pull it closer). Clamped to
    /// `view_distance`, so nothing is ever drawn unfogged at the edge of what
    /// is streamed.
    pub fog_end: f32,
    /// Follow the fog planes of the region's environment profile
    /// (`Map.pk2` `environment.ifo`): long, clear noons and short, heavily
    /// fogged nights, as in the original. Off = a fixed `fog_start..fog_end`.
    pub envi_fog: bool,
    /// Multiplier on the profile-mapped fog distances. The raw mapping reads
    /// short (noon median ~3530 of the 5760 ceiling across the 1.188
    /// profiles), so the client has always scaled it by 1.5.
    pub envi_fog_scale: f32,
    /// Distance past which whole map objects, terrain regions, effects and
    /// animations stop being drawn or simulated. 0 = at `fog_end`, where the
    /// fog has already hidden them. A value below `fog_end` culls things the
    /// fog has not fully hidden yet, which trades visible pop-out for speed.
    pub fog_cull_distance: f32,
    /// Cull at the *current* fog end (night fog can end at ~1200) instead of
    /// at the configured ceiling. Fully fogged geometry still shows as a
    /// fog-coloured silhouette against the sky gradient, so this changes the
    /// look of the horizon at night; off (the default) keeps it as it was.
    pub cull_follows_envi_fog: bool,
    /// Distance LOD for the terrain mesh — see [`TerrainLod`].
    pub terrain_lod: TerrainLod,
    /// How far other players, NPCs and monsters are drawn (the original's
    /// Character Sight Range option); 0 = as far as the server sends them.
    /// Each one is a skinned, animated model with equipment, and a crowded
    /// town is the heaviest view in the game, so a short range is a large
    /// saving on a weak machine. It is client-side only (the server still
    /// sends them) and never shorter than 1000: clicking reaches 1000 and
    /// nameplates 600, so nothing hidden can be clicked or labelled. Live.
    pub character_distance: f32,
}

/// Terrain geometry LOD (`graphics.view.terrain_lod`).
///
/// Off, every region draws its full 20-unit grid out to the fog: 18,432
/// triangles a region, ~1.5M for a 9x9 ring. With LOD, regions farther away
/// draw every 2nd vertex (a quarter of the triangles), then every 4th (a
/// sixteenth). Textures are unaffected, since the splat shader samples by
/// world position, and only distant relief loses detail, deep in the fog.
/// The original client has no terrain LOD, so `off` is the faithful setting.
/// Switch distances are measured to each region's centre.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TerrainLod {
    #[default]
    Off,
    /// Full grid within 2880 of a region's centre, half to 4800, quarter
    /// beyond.
    Normal,
    /// Full grid within 1920, half to 3840, quarter beyond.
    Aggressive,
}

impl TerrainLod {
    /// The distances at which the half and the quarter grid take over.
    pub fn switch_distances(self) -> Option<(f32, f32)> {
        match self {
            Self::Off => None,
            Self::Normal => Some((2880.0, 4800.0)),
            Self::Aggressive => Some((1920.0, 3840.0)),
        }
    }
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            view_distance: 5760.0,
            fog_start: 3840.0,
            fog_end: 5760.0,
            envi_fog: true,
            envi_fog_scale: 1.5,
            fog_cull_distance: 0.0,
            cull_follows_envi_fog: false,
            terrain_lod: TerrainLod::Off,
            character_distance: 0.0,
        }
    }
}

/// Streaming memory (`graphics.streaming`, `plugins/asset_residency.rs`).
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct StreamingSettings {
    /// Seconds a departed area's meshes, materials and textures stay loaded,
    /// so a quick return finds them decoded. Shorter frees memory sooner on
    /// a machine with little RAM, and costs re-decoding on a return. 30 is
    /// a starting value, not a measurement.
    pub residency_grace_secs: f32,
    /// Per-frame streaming budgets. Work past a budget waits for the next
    /// frame, nearest to the camera first, so a region crossing spreads over
    /// several frames instead of stalling one. Smaller budgets mean smaller
    /// hitches but slower pop-in; that suits a 2-core CPU, while a fast one
    /// can stream more per frame. The defaults are the values that took the
    /// measured worst region-crossing frame from 208 ms to 58 ms
    /// (`docs/perf-remote.md`). Each is at least 1. All apply live.
    ///
    /// Region ground meshes merged and spawned per frame
    /// (`terrain::load_terrain_system`).
    pub region_builds_per_frame: u32,
    /// Regions torn down per frame (`terrain::load_terrain_dynamically`).
    pub region_unloads_per_frame: u32,
    /// Map-object placements spawned per frame
    /// (`map::objects::load_terrain_objects_system`).
    pub object_spawns_per_frame: u32,
    /// Loaded resources (models with skeletons, meshes and materials) turned
    /// into entities per frame (`dynamic_resource_loader`).
    pub resource_spawns_per_frame: u32,
}

impl Default for StreamingSettings {
    fn default() -> Self {
        Self {
            residency_grace_secs: 30.0,
            // the measured defaults, documented where they are used
            region_builds_per_frame: crate::plugins::map::terrain::GROUP_BUILDS_PER_FRAME,
            region_unloads_per_frame: crate::plugins::map::terrain::REGION_UNLOADS_PER_FRAME,
            object_spawns_per_frame: crate::plugins::map::objects::OBJECT_SPAWNS_PER_FRAME,
            resource_spawns_per_frame:
                crate::plugins::dynamic_resource_loader::RESOURCE_SPAWNS_PER_FRAME,
        }
    }
}

impl StreamingSettings {
    /// A budget as the consumers use it: never 0, which would stall
    /// streaming outright.
    pub fn budget(value: u32) -> u32 {
        value.max(1)
    }
}

/// Dungeon-runtime visuals (`plugins/dungeon/`).
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct DungeonGraphicsSettings {
    /// Draw a flat placeholder circle on dungeon gate trigger areas. The
    /// original client marks these with a portal glow effect (not yet
    /// reproduced), so this is a non-original stand-in — without it the
    /// offline gates are invisible.
    pub gate_circles: bool,
    /// Cap on simultaneously active dungeon point lights (nearest to the
    /// player win); 0 = unlimited. Dense dungeons author hundreds of lights
    /// (Donwhang cave), far past what clustered forward rendering enjoys —
    /// a calibration knob in the spirit of the D3D8-era hardware light
    /// limit, not PK2 data.
    pub max_lights: usize,
}

impl Default for DungeonGraphicsSettings {
    fn default() -> Self {
        DungeonGraphicsSettings {
            gate_circles: true,
            max_lights: 16,
        }
    }
}

/// Ground foliage (`plugins/map/foliage/`). `native` renders the tile-driven
/// 3D-grass *authored* in the user's own Data.pk2 — tile2d.ifo
/// `{model,count}` pairs with the referenced grass `.bsr` models. Note: the
/// original v1.188 client parses those pairs but never renders them
/// (RE-verified, `docs/formats/2dti-jmxv2dti.md`) — so **`off` = faithful
/// 1.188**, and every mode here is an enhancement built on authored-but-
/// unused game data. `pack` scatters bundled CC0 cards on Grass/LongGrass
/// tiles instead (not game data at all).
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct FoliageSettings {
    /// off | native | pack | both. **off = faithful 1.188** (the original
    /// renders no 3D grass); native is the authored-data enhancement.
    pub mode: FoliageMode,
    /// 0.0 = every authored tuft is always rendered (max fidelity; the
    /// original culls grass near the camera, radius unknown). > 0.0 =
    /// per-block visibility-range end distance in world units. Defaults to a
    /// cull: with ~1.8k blocks in a region, "never cull" means every tuft on
    /// the map is submitted every frame. 300 covers the camera's working
    /// range (`camera.rs`: 40..400, 150 default) and fades out before max
    /// zoom-out.
    pub view_distance: f32,
    /// Multiplier on the authored native tuft counts, which are read as
    /// tufts per fully covered 320×320 terrain **block** (scaled by tile
    /// coverage). There is no original reference value — the v1.188 exe
    /// never reads the count field at all (see `FoliageSettings` docs) — so
    /// this is a pure taste knob on our per-block baseline.
    pub density: f32,
    /// Grass casts sun shadows. Costs one extra alpha-masked draw of every
    /// grass mesh per shadow cascade — keep `graphics.shadows` scoped (the
    /// designed 2 cascades × ~120 units) or grass pays it across the whole
    /// map. Off = grass still *receives* shadows, just doesn't cast them.
    pub cast_shadows: bool,
    pub pack: PackFoliageSettings,
    pub tint: FoliageTintSettings,
}

/// Environment blending of the grass: tufts take on the average color of the
/// terrain tile they stand on plus the region's baked per-cell light, so
/// grass reads as part of the ground instead of a foreign object.
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct FoliageTintSettings {
    /// 0..1: how much of the underlying tile's average color tints pack
    /// tufts. The photoreal pack cards are neutral-colored scans; 1.0
    /// grounds them fully in the local terrain palette.
    pub tile_blend_pack: f32,
    /// Same for native grass — subtle by default, its textures are already
    /// SRO-authored to match the tiles they were placed on. 0.0 = faithful.
    pub tile_blend_native: f32,
    /// Strength of the per-cell baked-light darkening from the region's `.t`
    /// tile grid (`JMXVMAPT::tile_light`, the same baked shading the ground
    /// carries). 0 = off. The byte's exact scale is unverified in the
    /// original client — calibrate visually in baked-shadow areas.
    pub baked_light: f32,
}

impl Default for FoliageTintSettings {
    fn default() -> Self {
        Self {
            tile_blend_pack: 1.0,
            tile_blend_native: 0.25,
            baked_light: 0.5,
        }
    }
}

impl Default for FoliageSettings {
    fn default() -> Self {
        Self {
            mode: FoliageMode::default(),
            view_distance: 300.0,
            density: 1.0,
            cast_shadows: false,
            pack: PackFoliageSettings::default(),
            tint: FoliageTintSettings::default(),
        }
    }
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FoliageMode {
    #[default]
    Off,
    Native,
    Pack,
    Both,
}

/// Pack layer only (non-original content; sprites from `assets/foliage/`).
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct PackFoliageSettings {
    /// Tufts per 20x20 terrain cell on Grass/LongGrass tiles.
    pub density: f32,
    /// Uniform sprite scale multiplier.
    pub scale: f32,
}

impl Default for PackFoliageSettings {
    fn default() -> Self {
        Self {
            density: 8.0,
            scale: 1.0,
        }
    }
}

/// Dynamic sun shadows via Bevy's cascaded shadow maps, **PBR mode only**
/// (`RenderMode::Vanilla` disables the Sun's shadow maps entirely — the
/// original has none beyond the baked terrain lightmap — gap #9 in
/// `docs/rendering-mobile-shader-comparison.md`; acceptance per backlog
/// EP-28.10 is a measured `make perf` budget). The render-debug panel's
/// `enable_shadows` toggles them live on top of this master switch, still
/// PBR-mode only.
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct ShadowSettings {
    pub enabled: bool,
    /// Shadow cascade count, **PBR mode only**. Bevy's default 4 over a huge
    /// range re-renders all loaded terrain/objects several times per frame;
    /// few short cascades keep the cost bounded.
    pub cascades: usize,
    /// Maximum shadow distance in world units, **PBR mode only** (~120
    /// covers the immediate combat area). Beware: the shadow map is spread
    /// over this whole range — at 100 000 units a 2048 map has ~0.8-unit
    /// texels (1 unit = 10 cm) and every shadow blurs into a faint blob.
    pub distance: f32,
    /// Shadow map resolution per cascade (power of two; clamped to ≥512 and
    /// rounded up). 2048 is Bevy's default; 4096 halves the texel size for
    /// crisper shadows at 4× the GPU memory per cascade.
    pub map_size: u32,
    /// How shadow edges are filtered. See [`ShadowFiltering`].
    pub filtering: ShadowFiltering,
}

impl Default for ShadowSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            cascades: 2,
            distance: 120.0,
            map_size: 2048,
            filtering: ShadowFiltering::default(),
        }
    }
}

/// Terrain lighting options (`terrain_splat.wesl`). The lighting *model*
/// (dynamic PBR vs. baked) follows [`RenderMode`] — see
/// [`TerrainGraphicsSettings::to_render_params`] — not a field here; the
/// third A/B option (`flat_baked`, no N·L but keeps the time-of-day tint) is
/// reachable only as a live override in the render-debug panel
/// (`docs/rendering-mobile-shader-comparison.md` gaps #5/#6), since it isn't
/// one of the two shipped modes.
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct TerrainGraphicsSettings {
    /// Mirror the baked lightmap's V axis — the pending `.t` row-order
    /// calibration (`docs/formats/mapt-jmxvmapt.md`); flip if baked shadows
    /// come out mirrored along Z.
    pub lightmap_flip_v: bool,
    /// Which draw path renders terrain ground — see [`TerrainPipeline`].
    /// Restart-only: it decides which render pipeline is registered.
    pub pipeline: TerrainPipeline,
    /// Sample cliff albedo from the side as well as from above
    /// (`terrain_splat.wesl`, `slope_projection`): `off` is the original's
    /// top-down projection, which stretches a ground tile by `1 / cos(slope)`
    /// — double at 60 degrees, unbounded on a wall. `blend` adds one
    /// projection on fragments past 60 degrees, `full` adds both and
    /// cross-fades them.
    ///
    /// Only the albedo UV changes; tile choice and splat weights stay the
    /// original's XZ lookup. Costs up to two extra texture samples on the
    /// 9.4% of the map's footprint that is that steep (measured over the
    /// player's own `Map.pk2` — see `SLOPE_START` in the shader), and nothing
    /// at all on flatter ground. `off` in `low`, which is also the setting a
    /// GL-baseline GPU wants.
    pub slope_sampling: TerrainSlopeSampling,
}

/// The two terrain ground draw paths. Both sample the same global ground-tile arrays
/// (`client/src/assets/m/tile_arrays.rs`); they differ in how the per-draw bind groups are built.
///
/// `material` (default) is the stock `Material`/`MaterialPlugin` path: battle-tested, with shadow
/// casting, prepass and deferred support for free, but every region gets its own bind group
/// (holding the shared arrays and buffers again).
///
/// `hand_rolled` is the custom `SpecializedMeshPipeline` in `plugins/map/terrain/render/` that
/// binds the shared arrays and buffers once, globally. Newer: shadow casting is its own
/// reimplementation (directional/Sun only), and the render-debug backface toggle does not reach it
/// yet.
///
/// Was the `terrain_hand_rolled_pipeline` Cargo feature; a config option so the two can be A/B'd
/// on the same build. Read once at startup.
#[derive(Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, bevy::prelude::Resource)]
#[serde(rename_all = "snake_case")]
pub enum TerrainPipeline {
    #[default]
    Material,
    HandRolled,
}

impl TerrainGraphicsSettings {
    /// `render_mode` comes from the sibling [`GraphicsSettings::render_mode`]
    /// field, not `self` — it decides the terrain shader's lighting model:
    /// `Vanilla` → fully baked (`albedo × lightmap`), `Pbr` → dynamic PBR sun
    /// + ambient over the baked lightmap.
    pub fn to_render_params(
        &self,
        render_mode: RenderMode,
    ) -> crate::assets::m::block_splat_material::TerrainRenderParams {
        use crate::assets::m::block_splat_material::{TerrainLightingMode, TerrainRenderParams};
        TerrainRenderParams {
            lightmap_flip_v: self.lightmap_flip_v,
            slope_sampling: self.slope_sampling,
            lighting_mode: match render_mode {
                RenderMode::Vanilla => TerrainLightingMode::Baked,
                RenderMode::Pbr => TerrainLightingMode::Dynamic,
            },
            lightmap_enabled: render_mode == RenderMode::Vanilla,
            // splat 24/32 repeat factors are a calibration in progress
            // (render-debug sliders), not a config concern
            ..TerrainRenderParams::default()
        }
    }
}

/// Always-on rim light on character-class models and equipment (the mobile
/// port applies it to everything, all the time — gap #1 in
/// `docs/rendering-mobile-shader-comparison.md`; the original 1.188 client
/// has no rim at all). The hover/click selection highlight is separate
/// (`selection.highlight`) and overrides this subtle rim with its stronger
/// color while active.
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct RimGraphicsSettings {
    /// Off = faithful 1.188 (no ambient rim; selection highlighting still
    /// works exactly as before).
    pub enabled: bool,
    /// AARRGGBB; the alpha is the rim strength (mobile ships 0.353 gray —
    /// white at alpha 0x5A gives the same term as a *fraction of the lit
    /// color* in `relative` mode).
    pub color: String,
    /// Fresnel falloff exponent — higher makes the edge thinner (mobile
    /// ships 3).
    pub power: f32,
    /// `relative` (default): `lit × (1 + rim·fresnel)` — exposure- and
    /// tonemap-independent, matches the mobile ratio at any time of day.
    /// `absolute`: plain scene-linear add (the first-round behavior, which
    /// disappears against HDR daylight; kept for A/B).
    pub mode: RimMode,
    /// Extra multiplier on the color's alpha strength — headroom beyond
    /// the 1.0 the AARRGGBB alpha byte caps at.
    pub strength: f32,
}

impl Default for RimGraphicsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            color: "5AFFFFFF".into(),
            power: 3.0,
            mode: RimMode::Relative,
            strength: 1.0,
        }
    }
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RimMode {
    #[default]
    Relative,
    Absolute,
}

/// Metallic-sheen rendering options (weapons/metal armor,
/// `assets/bmt/sheen.rs` + `sro_sheen.wesl`). The defaults adopt the
/// mobile port's look (see `docs/rendering-mobile-shader-comparison.md`);
/// each field's faithful-1.188 value is documented so one config line
/// restores the original behavior.
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct SheenGraphicsSettings {
    /// Sharpening exponent on the +N enhancement streak (the mobile
    /// port's `streamerPOW`): higher = a narrower, crisper band sweeping
    /// the blade. **1.0 = faithful** (the original's soft unsharpened
    /// streak).
    ///
    /// It is also an *energy* control, which is why the default is no longer
    /// 4.0: the streak texture is a mid-tone gradient, so `pow(0.5, 4)` keeps
    /// barely 6 % of it. Paired with `intensity` below — raising this without
    /// raising that is what made the +N glow invisible.
    pub shine_pow: f32,
    /// Overall brightness of the +N enhancement glow.
    ///
    /// **Ours, and the fix for a real problem.** The per-tier intensities in
    /// `ShineColor::colors` are scene-linear HDR values chosen for a bloom-lit
    /// pipeline; with `graphics.bloom.enabled: false` the cameras have no HDR
    /// target, tonemapping runs inside the forward pass, and those values are
    /// squashed flat — the glow all but disappears. This multiplies them, so
    /// the effect is tunable independently of a scene-wide bloom setting that
    /// is not really about weapons.
    ///
    /// 1.0 is the old behaviour. The default is higher so the glow is visible
    /// without bloom; drop it back to 1.0 if you run with bloom on and find it
    /// blown out. Note `docs/formats/resinfo-itemoption.md` records the
    /// underlying intensities as calibration-pending, so this knob is tuning a
    /// number that was never measured in the first place.
    pub intensity: f32,
    /// Metallic sheen on weapons and metal armour at all (the original's
    /// Metallic Sheen option). Off draws them with their plain material: no
    /// chrome probe, no +N enhancement glow, and one texture sample less per
    /// pixel of armour. Restart-only, because materials are chosen at spawn
    /// from defaults built once at startup.
    pub enabled: bool,
}

impl Default for SheenGraphicsSettings {
    fn default() -> Self {
        Self {
            // Down from 4.0: see the field docs — at 4.0 the streak lost ~94%
            // of its energy and no intensity compensated for it.
            shine_pow: 2.0,
            intensity: 3.0,
            enabled: true,
        }
    }
}

impl GraphicsSettings {
    /// The config-derived settings the rim/sheen material variants are built
    /// from (inserted as [`BmtMaterialDefaults`] at startup and read by the
    /// spawn path's `SroMaterialVariants` — the asset code itself can't see
    /// config types; the parser-only lib target has no `plugins` module).
    ///
    /// [`BmtMaterialDefaults`]: crate::assets::bmt::material::BmtMaterialDefaults
    pub fn to_material_defaults(&self) -> crate::assets::bmt::material::BmtMaterialDefaults {
        use crate::assets::bmt::material::BmtMaterialDefaults;
        use crate::assets::bmt::rim::rim_settings;
        use crate::assets::bmt::sheen::SheenSettings;
        use bevy::log::warn;

        let rim = self.rim.enabled.then(|| {
            let color =
                crate::plugins::config::chat::parse_argb(&self.rim.color).unwrap_or_else(|| {
                    let default = RimGraphicsSettings::default().color;
                    warn!(
                        "config: invalid graphics.rim.color {:?}, using {default}",
                        self.rim.color
                    );
                    crate::plugins::config::chat::parse_argb(&default)
                        .expect("default rim color is valid")
                });
            rim_settings(
                color,
                self.rim.strength.max(0.0),
                self.rim.power,
                self.rim.mode == RimMode::Relative,
            )
        });
        let mut sheen = SheenSettings {
            shine_pow: self.sheen.shine_pow.max(0.0),
            ..SheenSettings::default()
        };
        // weapons/metal armor carry the ambient rim inside the sheen shader
        if let Some(rim) = &rim {
            sheen.rim_color = rim.color;
            sheen.rim_power = rim.power;
            sheen.rim_mode = rim.mode;
        }
        BmtMaterialDefaults {
            sheen,
            rim,
            sheen_enabled: self.sheen.enabled,
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct BloomSettings {
    /// Off means the cameras stay LDR: no `Bloom` **and** no `Hdr` component,
    /// i.e. byte-identical to the pre-bloom rendering.
    pub enabled: bool,
    /// Scatter strength in energy-conserving terms, 0.0..=1.0.
    /// [`Bloom::NATURAL`]'s default is 0.15.
    pub intensity: f32,
}

impl Default for BloomSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            intensity: 0.15,
        }
    }
}

impl BloomSettings {
    /// Built on [`Bloom::NATURAL`]: prefilter threshold 0 + energy-conserving
    /// compositing. Deliberately *not* thresholded — bevy documents a non-zero
    /// prefilter as physically inaccurate and only valid together with
    /// `BloomCompositeMode::Additive`. Energy conservation already keeps the
    /// sub-1.0 terrain/sky almost untouched while the 2.0-8.0 emitters bloom,
    /// which is the "only real emitters glow" look we want. If it ever reads
    /// too soft, `Bloom::OLD_SCHOOL` (threshold 0.6 + additive) is the
    /// stylised fallback.
    pub fn to_bloom(&self) -> Bloom {
        Bloom {
            intensity: self.intensity,
            ..Bloom::NATURAL
        }
    }
}

#[cfg(test)]
mod gpu_option_tests {
    use super::*;

    /// The tiers have to stay ordered in work, and `low` has to stay off:
    /// it is the preset `auto` picks for a GL-baseline GPU, where two extra
    /// texture samples per steep fragment are the last thing wanted.
    #[test]
    fn slope_sampling_is_off_by_default_and_rises_with_the_tier() {
        use crate::plugins::config::preset::QualityPreset;
        let empty: GraphicsSettings = serde_yaml::from_str("{}").unwrap();
        assert_eq!(empty.terrain.slope_sampling, TerrainSlopeSampling::Off);

        let tier = |preset: QualityPreset| -> TerrainSlopeSampling {
            let yaml = preset.layer().expect("concrete tiers have a layer");
            let value: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
            let graphics: GraphicsSettings =
                serde_yaml::from_value(value.as_mapping().unwrap()["graphics"].clone()).unwrap();
            graphics.terrain.slope_sampling
        };
        assert_eq!(tier(QualityPreset::Low), TerrainSlopeSampling::Off);
        assert_eq!(tier(QualityPreset::Medium), TerrainSlopeSampling::Blend);
        assert_eq!(tier(QualityPreset::High), TerrainSlopeSampling::Full);
        assert_eq!(tier(QualityPreset::Ultra), TerrainSlopeSampling::Full);
    }

    /// The key reaches the GPU: `to_render_params` is the only path from this
    /// config to `terrain_params[2].w`.
    #[test]
    fn the_key_is_carried_into_the_render_params() {
        let settings: TerrainGraphicsSettings =
            serde_yaml::from_str("slope_sampling: full").unwrap();
        let params = settings.to_render_params(RenderMode::Vanilla);
        assert_eq!(params.slope_sampling, TerrainSlopeSampling::Full);
    }

    #[test]
    fn bindless_and_sky_reflections_default_off() {
        let empty: GraphicsSettings = serde_yaml::from_str("{}").unwrap();
        assert!(!empty.bindless_materials);
        assert!(!empty.sky_reflections);
        assert!(!GraphicsSettings::default().sky_reflections);

        let set: GraphicsSettings =
            serde_yaml::from_str("bindless_materials: true\nsky_reflections: true").unwrap();
        assert!(set.bindless_materials);
        assert!(set.sky_reflections);
    }
}

#[cfg(test)]
mod sharpening_tests {
    use super::*;

    /// The serde default and the derived `GraphicsSettings::default()` have to
    /// agree, or a `config.yaml` without the key renders differently from one
    /// with the documented value written out.
    #[test]
    fn an_absent_key_is_bevys_own_default_strength() {
        let empty: GraphicsSettings = serde_yaml::from_str("{}").unwrap();
        assert_eq!(empty.sharpening, Sharpening(0.6));
        assert_eq!(GraphicsSettings::default().sharpening, Sharpening(0.6));
        let set: GraphicsSettings = serde_yaml::from_str("sharpening: 0.8").unwrap();
        assert_eq!(set.sharpening, Sharpening(0.8));
    }

    /// 0.0 is the documented "off", and it has to reach the component as a
    /// *disabled* pass: bevy skips extracting it then, so off costs nothing.
    #[test]
    fn zero_is_off_and_every_other_strength_is_enabled() {
        let off = Sharpening(0.0).to_cas();
        assert!(!off.enabled);
        assert_eq!(off.sharpening_strength, 0.0);

        let on = Sharpening(0.8).to_cas();
        assert!(on.enabled);
        assert_eq!(on.sharpening_strength, 0.8);
    }

    /// A hand-edited file must not reach the shader with a strength bevy calls
    /// "extreme artifacts and fireflies", and a non-finite value must not
    /// reach it at all.
    #[test]
    fn an_out_of_range_strength_clamps_instead_of_reaching_the_shader() {
        assert_eq!(Sharpening(5.0).to_cas().sharpening_strength, 1.0);
        assert_eq!(Sharpening(-1.0).to_cas().sharpening_strength, 0.0);
        assert!(!Sharpening(-1.0).to_cas().enabled);
        assert_eq!(Sharpening(f32::NAN).to_cas().sharpening_strength, 0.0);
        assert!(!Sharpening(f32::INFINITY).to_cas().enabled);
    }

    /// The denoise flag stays off: bevy's own guidance is "you probably
    /// shouldn't use this" (`contrast_adaptive_sharpening/mod.rs:50`), and
    /// nothing in this client adds film grain for it to protect.
    #[test]
    fn denoising_stays_off() {
        assert!(!Sharpening(0.6).to_cas().denoise);
    }
}
