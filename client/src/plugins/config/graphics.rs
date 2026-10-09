//! Graphics/quality settings that need to be adjustable without recompiling.
//!
//! Currently only bloom. The idea: the renderer already writes genuine HDR
//! values — emissive BMT props at 2.0 (`assets/bmt/material.rs`) and the `+N`
//! enhancement shine at 4.0-8.0 (`assets/bmt/sheen.rs`, `sro_sheen.wgsl`) — but
//! the cameras historically had no `Hdr` marker, so all of it clipped flat at
//! 1.0. Enabling bloom flips the main pass to a float target, which is what
//! actually lets those emitters read as "glowing" instead of "white".

use bevy::post_process::bloom::Bloom;
use bevy::reflect::Reflect;
use serde::Deserialize;

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
    /// Mip levels dropped from world textures — see [`TextureDetail`].
    #[serde(default)]
    pub texture_detail: TextureDetail,
    /// Anisotropic filtering clamp — see [`Anisotropy`].
    #[serde(default)]
    pub anisotropy: Anisotropy,
    /// Particle effect quality — see [`EffectQuality`].
    #[serde(default)]
    pub effect_quality: EffectQuality,
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
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct WaterSettings {
    pub quality: WaterQuality,
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

    /// The usable fraction: non-finite and out-of-range values fall back to
    /// 1.0 with a warning rather than silently producing a broken target, and
    /// exactly 1.0 means "do not retarget".
    pub fn factor(self) -> f32 {
        if !self.0.is_finite() || self.0 <= 0.0 || self.0 > 1.0 {
            bevy::log::warn_once!(
                "graphics.render_scale: {} is outside {}..=1.0; rendering at full resolution",
                self.0,
                Self::MIN
            );
            return 1.0;
        }
        if self.0 < Self::MIN {
            bevy::log::warn_once!(
                "graphics.render_scale: {} is below the {} floor; clamping",
                self.0,
                Self::MIN
            );
        }
        self.0.max(Self::MIN)
    }

    /// Whether this scale asks for an offscreen target at all.
    pub fn is_native(self) -> bool {
        self.factor() >= 1.0
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
    /// Width of the dither/crossfade band before the cull distance. Materials
    /// without the crossfade path hard-cut at the far edge instead.
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
    /// over `fade`; materials without the crossfade path hard-cut at the far
    /// edge.
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
/// exponent)` scaled by the light's color. The original client's fog is
/// fixed-function D3D linear fog with no such term, so **0.0 is the faithful
/// value** and this exists only to dial the halo back in as an enhancement.
///
/// It is off by default because the term reads as *gloss*, not haze: the sun's
/// color arrives premultiplied by its 10 000 lux illuminance
/// (`plugins/map/mod.rs`), it is multiplied by the shadow map (so it lands in
/// patches on geometry rather than uniformly in the air), and it scales with
/// the fog blend, so it is strongest on the most distant objects. With the
/// default exponent it is a ~7-degree-wide hot disc that sweeps across distant
/// terrain as the camera turns — indistinguishable from a specular lobe on
/// objects that should be flat and fogged out.
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct FogGraphicsSettings {
    /// Strength of the sun's in-scattering halo in fog. **0.0 = faithful
    /// 1.188** (no in-scattering at all); 0.5 restores the value this shipped
    /// with before it was found to read as gloss on distant objects.
    pub sun_scattering: f32,
    /// Angular tightness of that halo — the `pow()` exponent on
    /// `dot(view_dir, sun_dir)`. Only has an effect when `sun_scattering > 0`.
    /// Lower is a broader, more atmospheric glow; the previous shipped value
    /// of 100 is half-strength at ~6.7 degrees, which is what made it read as a
    /// specular highlight rather than scattered light.
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

/// Terrain lighting options (`terrain_splat.wgsl`). The lighting *model*
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
/// `assets/bmt/sheen.rs` + `sro_sheen.wgsl`). The defaults adopt the
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
