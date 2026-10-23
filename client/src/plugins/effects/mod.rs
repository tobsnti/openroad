//! Runtime for JMXVEFF particle/visual effects: a CPU interpreter of the
//! effect node tree. Every live node is an ordinary Bevy entity, rendered
//! with the unlit [`material::SroEffectMaterial`].
//!
//! bevy_hanabi was evaluated and deliberately not used: SRO effects are many
//! tiny emitters (dozens of particles) with per-node materials, windowed
//! one-shot commands, and bone-anchored geometry — the inverse of hanabi's
//! few-effects/many-particles GPU-compute sweet spot. If dense ambient
//! effects (weather etc.) are ever needed, hanabi can be added alongside
//! this module without touching the JMXVEFF asset or spawn API.

pub mod components;
pub mod instanced;
pub mod material;
pub mod rare;
pub mod spawn;
pub mod systems;
pub mod trail;

use std::time::Duration;

use bevy::app::{App, Plugin, PostUpdate, PreUpdate, Update};
use bevy::asset::Assets;
use bevy::ecs::schedule::{IntoScheduleConfigs, SystemSet};
use bevy::pbr::MaterialPlugin;
use bevy::prelude::{Res, ResMut, Resource};
use bevy::time::common_conditions::on_timer;
use bevy::transform::TransformSystems;

use material::SroEffectMaterial;
use spawn::{EffectMaterials, EffectMeshes, EffectQuad};

pub use components::*;
pub use spawn::{ActiveAnimationEffects, EffectCommandsExt};

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct EffectSystems;

/// Master switch for the effect runtime (render-debug panel `render_effects`
/// toggle): when false every effect system is paused wholesale so the
/// runtime costs nothing — useful to A/B the FPS impact of particles. The
/// dev panel hides the (frozen) visuals by flipping wrapper `Visibility`;
/// `sync_animation_effects` compares state rather than events, so
/// everything self-corrects on re-enable.
#[derive(Resource)]
pub struct EffectsEnabled(pub bool);

impl Default for EffectsEnabled {
    fn default() -> Self {
        Self(true)
    }
}

fn effects_enabled(enabled: Res<EffectsEnabled>) -> bool {
    enabled.0
}

/// Live calibration factor for ADDITIVE effect brightness (dst blend ONE):
/// the original saturated an LDR backbuffer where our HDR pipeline (plus
/// bloom) keeps accumulating, so dense additive stacks (Seal auras, glows)
/// can read hotter than the original. Driven by the render-debug panel
/// (`effect_additive_intensity`); applied to every effect material's color
/// multiplier on change and at material creation. 1.0 = authored.
#[derive(Resource)]
pub struct EffectAdditiveIntensity(pub f32);

impl Default for EffectAdditiveIntensity {
    fn default() -> Self {
        Self(1.0)
    }
}

/// Live toggle for the LDR-additive emulation (saturating blend + 8-bit
/// quantization on `dst = ONE` effect materials, see
/// [`material::SroEffectMaterial::ldr_additive`]). Default on — it
/// reproduces the original client's 8-bit backbuffer clamp; the
/// render-debug panel (`effect_ldr_additive`) flips it live for A/B
/// against unbounded HDR accumulation.
#[derive(Resource)]
pub struct EffectLdrAdditive(pub bool);

impl Default for EffectLdrAdditive {
    fn default() -> Self {
        Self(true)
    }
}

/// Leaf self-emission policy: with `global` on, EVERY effect's leaf
/// StaticEmit nodes self-emit their authored particle
/// counts, thinned by `density`. Default ON — the original engine's
/// StaticEmit is an unconditional effect-level source (exe RE), and
/// treating leaf emitters as single plates makes their whole effect replay
/// its envelope once per loop (the talisman "starts and dies after ~1s"
/// report: its steady glow IS 5+5+20 staggered wisp copies). The
/// render-debug toggle (`leaf_emit_global`) remains the live A/B off
/// switch; emission counts stay bounded by the authored `max_alive` caps
/// plus particle pooling.
#[derive(Resource)]
pub struct LeafEmitPolicy {
    pub global: bool,
    pub density: f32,
}

impl Default for LeafEmitPolicy {
    fn default() -> Self {
        Self {
            global: true,
            density: 1.0,
        }
    }
}

/// Share of each emitter's authored particle cap that may be alive at once
/// (`graphics.effect_quality`; the original's Effect Quality option). Fewer
/// live particles means less simulation on the CPU and less blended
/// overdraw on the GPU, which is what a crowded skill fight costs an old
/// machine. Every emitter keeps at least one particle, so no effect
/// disappears outright.
#[derive(Resource)]
pub struct EffectQualityScale(pub f32);

impl Default for EffectQualityScale {
    fn default() -> Self {
        Self(1.0)
    }
}

/// Soft-particle fade distance in world units, 0 = off. Follows
/// `graphics.effect_soft_fade`, forced to 0 on `EffectQuality::Low`.
///
/// Why the quality tier overrides the option rather than scaling it: the cost
/// of this effect is one depth `textureLoad` per effect fragment, and effects
/// carry the heaviest overdraw in the frame. A narrower fade distance would
/// look cheaper without *being* cheaper — only skipping the read is cheaper,
/// and `effect_params.w == 0` is what makes the shader skip it.
#[derive(Resource, Default)]
pub struct EffectSoftFade(pub f32);

/// Follows `graphics.effect_quality` into [`EffectQualityScale`], and
/// `graphics.effect_soft_fade` into [`EffectSoftFade`].
fn apply_effect_quality(
    config: Res<crate::plugins::config::ClientConfig>,
    mut scale: ResMut<EffectQualityScale>,
    mut soft_fade: ResMut<EffectSoftFade>,
    mut materials: ResMut<Assets<SroEffectMaterial>>,
) {
    let wanted = config.graphics.effect_quality.particle_scale();
    if scale.0 != wanted {
        scale.0 = wanted;
    }
    let wanted_fade =
        if config.graphics.effect_quality == crate::plugins::config::graphics::EffectQuality::Low {
            0.0
        } else {
            config.graphics.effect_soft_fade.max(0.0)
        };
    if soft_fade.0 != wanted_fade {
        soft_fade.0 = wanted_fade;
        // Same shape as the render-debug additive-intensity dial: only on an
        // actual change, because a material write re-prepares its bind group.
        for (_, mat) in materials.iter_mut() {
            mat.set_soft_fade(wanted_fade);
        }
    }
}

/// Global effect playback-rate multiplier: `< 1.0` slows every effect's
/// simulation uniformly — node ages (graphs, lifespans, loops), emission
/// pacing, program scheduling, velocity/rotation integration, trail aging.
/// The calibration dial against the original client (render-debug field
/// `effect_time_scale`); 1.0 = the exe-derived 20 fps timebase as-is.
#[derive(Resource)]
pub struct EffectPlaybackSpeed(pub f32);

impl Default for EffectPlaybackSpeed {
    fn default() -> Self {
        Self(1.0)
    }
}

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(instanced::InstancedEffectsPlugin);
        app.add_plugins(MaterialPlugin::<SroEffectMaterial>::default())
            .init_resource::<EffectQuad>()
            .init_resource::<EffectMeshes>()
            .init_resource::<EffectMaterials>()
            .init_resource::<EffectsEnabled>()
            .init_resource::<LeafEmitPolicy>()
            .init_resource::<EffectAdditiveIntensity>()
            .init_resource::<EffectLdrAdditive>()
            .init_resource::<EffectPlaybackSpeed>()
            .init_resource::<EffectQualityScale>()
            .init_resource::<EffectSoftFade>()
            .add_systems(
                PreUpdate,
                apply_effect_quality.run_if(
                    bevy::ecs::schedule::common_conditions::resource_exists_and_changed::<
                        crate::plugins::config::ClientConfig,
                    >,
                ),
            )
            // `cull_effect_simulation` pauses at its cull distance
            .init_resource::<crate::plugins::map::view_range::ViewRange>()
            .add_systems(
                Update,
                (
                    spawn::sync_animation_effects,
                    spawn::start_delayed_effects,
                    spawn::instantiate_effects,
                    systems::rebuild_program_caches,
                    systems::cull_effect_simulation,
                    systems::toggle_night_only_effects,
                    systems::tick_effect_nodes,
                    systems::emit_particles,
                    systems::run_programs,
                    systems::sample_graphs,
                    trail::update_effect_trails,
                )
                    .chain()
                    .run_if(effects_enabled)
                    .in_set(EffectSystems),
            )
            .add_systems(
                PostUpdate,
                systems::billboard_effect_nodes
                    .run_if(effects_enabled)
                    .before(TransformSystems::Propagate),
            )
            // outside the effects_enabled gate: entities keep despawning
            // (and their assets keep freeing) while effects are toggled off
            .add_systems(
                Update,
                spawn::prune_effect_caches.run_if(on_timer(Duration::from_secs(10))),
            );
    }
}
