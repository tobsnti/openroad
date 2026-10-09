//! Graphics quality presets (`graphics.preset`), layered under `config.yaml`.
//!
//! A preset is a partial YAML file (`presets/<tier>.yaml`, compiled in) that
//! [`super::ClientConfig::from_file`] adds as a config source *before* the
//! user's own file. config-rs merges sources key by key, so any key the user
//! writes wins and every key they leave out comes from the preset. A
//! `config.yaml` that sets everything explicitly therefore behaves exactly as
//! before. A fresh one copied from `config.example.yaml`, where the
//! preset-controlled keys are commented out, follows the preset.
//!
//! `auto`, the default, resolves the tier from the GPU before the App exists
//! ([`super::gpu_probe`]), because the preset decides restart-only settings
//! such as the requested wgpu features.

use serde::Deserialize;

/// The tiers, from what a 2008-era card can carry to what a current one can.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum QualityPreset {
    /// Picked from the GPU at startup; see [`tier_for`].
    #[default]
    Auto,
    Low,
    Medium,
    High,
    Ultra,
}

impl QualityPreset {
    /// The preset's YAML layer. `Auto` has none of its own: it is resolved
    /// to one of the others before the config is built.
    pub fn layer(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Low => Some(include_str!("presets/low.yaml")),
            Self::Medium => Some(include_str!("presets/medium.yaml")),
            Self::High => Some(include_str!("presets/high.yaml")),
            Self::Ultra => Some(include_str!("presets/ultra.yaml")),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Ultra => "ultra",
        }
    }
}

/// The preset a config was actually built with, and why. Logged at startup.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPreset {
    pub tier: QualityPreset,
    pub reason: String,
}

/// What the `auto` preset decides from, read off the adapter the renderer
/// will use (`gpu_probe::probe`). Plain data, so the decision is testable
/// without a GPU.
#[derive(Debug, Clone, PartialEq)]
pub struct GpuSummary {
    pub name: String,
    pub backend: String,
    pub kind: GpuKind,
    /// Compute shaders and storage buffers, which Bevy's GPU-driven paths
    /// (preprocessing, clustering) need. DX10-class GPUs on wgpu's GL 3.3
    /// backend have neither.
    pub compute: bool,
    /// The adapter runs on wgpu's GL backend.
    pub gl: bool,
}

/// `wgpu::DeviceType`, reduced to what the decision needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuKind {
    Discrete,
    Integrated,
    /// A software rasterizer or a virtualized device.
    Software,
    Unknown,
}

/// The `auto` decision. Deliberately coarse: the adapter's type and its
/// capability class are the only signals every backend reports reliably, and
/// a wrong guess is one config line to override.
/// - no compute/storage, the GL backend, or a software device → `low`
/// - an integrated GPU → `medium`
/// - a discrete GPU → `high`. `ultra` is never automatic; it streams 11x11
///   regions and is for someone who asks for it.
/// - no adapter found at all → `medium`, a safe middle. The renderer will
///   report the real failure itself.
pub fn tier_for(gpu: Option<&GpuSummary>) -> ResolvedPreset {
    let Some(gpu) = gpu else {
        return ResolvedPreset {
            tier: QualityPreset::Medium,
            reason: "auto: no GPU adapter found by the probe".into(),
        };
    };
    let (tier, why) = if !gpu.compute || gpu.gl {
        (QualityPreset::Low, "no compute shaders / GL backend")
    } else {
        match gpu.kind {
            GpuKind::Software => (QualityPreset::Low, "software rasterizer"),
            GpuKind::Integrated => (QualityPreset::Medium, "integrated GPU"),
            GpuKind::Discrete => (QualityPreset::High, "discrete GPU"),
            GpuKind::Unknown => (QualityPreset::Medium, "unknown GPU type"),
        }
    };
    ResolvedPreset {
        tier,
        reason: format!("auto: {why} '{}' ({})", gpu.name, gpu.backend),
    }
}

/// The GPU unit tests resolve `auto` against (`ClientConfig::from_file`).
#[cfg(test)]
pub fn test_discrete_gpu() -> Option<GpuSummary> {
    Some(GpuSummary {
        name: "unit-test GPU".into(),
        backend: "Vulkan".into(),
        kind: GpuKind::Discrete,
        compute: true,
        gl: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::config::graphics::GraphicsSettings;

    fn gpu(kind: GpuKind, compute: bool, gl: bool) -> GpuSummary {
        GpuSummary {
            name: "test".into(),
            backend: "Vulkan".into(),
            kind,
            compute,
            gl,
        }
    }

    #[test]
    fn auto_follows_the_gpu_class() {
        let tier = |g: Option<&GpuSummary>| tier_for(g).tier;
        assert_eq!(
            tier(Some(&gpu(GpuKind::Discrete, true, false))),
            QualityPreset::High
        );
        assert_eq!(
            tier(Some(&gpu(GpuKind::Integrated, true, false))),
            QualityPreset::Medium
        );
        assert_eq!(
            tier(Some(&gpu(GpuKind::Software, true, false))),
            QualityPreset::Low
        );
        // a DX10-class card on GL: low whatever wgpu calls it
        assert_eq!(
            tier(Some(&gpu(GpuKind::Discrete, false, true))),
            QualityPreset::Low
        );
        assert_eq!(
            tier(Some(&gpu(GpuKind::Discrete, false, false))),
            QualityPreset::Low
        );
        assert_eq!(tier(None), QualityPreset::Medium);
    }

    /// Every preset layer is valid on its own: it parses, deserializes into
    /// the graphics section, and sets nothing outside `graphics`.
    fn layer(preset: QualityPreset) -> GraphicsSettings {
        let yaml = preset.layer().expect("concrete tiers have a layer");
        let value: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        let top = value.as_mapping().unwrap();
        assert_eq!(top.len(), 1, "{} sets more than `graphics`", preset.name());
        serde_yaml::from_value(top["graphics"].clone()).unwrap()
    }

    #[test]
    fn every_preset_layer_deserializes() {
        for preset in [
            QualityPreset::Low,
            QualityPreset::Medium,
            QualityPreset::High,
            QualityPreset::Ultra,
        ] {
            layer(preset);
        }
        assert!(QualityPreset::Auto.layer().is_none());
    }

    /// Higher tiers never see or draw less than lower ones.
    #[test]
    fn the_tiers_are_ordered() {
        let tiers = [
            layer(QualityPreset::Low),
            layer(QualityPreset::Medium),
            layer(QualityPreset::High),
            layer(QualityPreset::Ultra),
        ];
        for pair in tiers.windows(2) {
            let (lo, hi) = (&pair[0], &pair[1]);
            assert!(lo.view.view_distance <= hi.view.view_distance);
            assert!(lo.view.fog_end <= hi.view.fog_end);
            assert!(lo.render_scale.factor() <= hi.render_scale.factor());
            let nature = |d: f32| if d == 0.0 { f32::INFINITY } else { d };
            assert!(
                nature(lo.objects.nature_view_distance) <= nature(hi.objects.nature_view_distance)
            );
        }
    }

    /// `high` is what the client looked like before presets existed: its
    /// layer must not move anything away from the built-in defaults.
    #[test]
    fn high_is_the_unpreset_look() {
        let high = layer(QualityPreset::High);
        let defaults = GraphicsSettings::default();
        assert_eq!(high.view, defaults.view);
        assert_eq!(high.msaa.0, 4);
        assert_eq!(high.render_scale.factor(), 1.0);
        assert_eq!(high.objects.nature_view_distance, 0.0);
        assert!(!high.depth_prepass);
    }
}
