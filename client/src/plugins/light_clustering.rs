//! GPU light clustering on or off (`graphics.gpu_light_clustering`).
//!
//! Bevy assigns point lights to view-space clusters with a compute and raster
//! pass, run every frame for every 3D view whenever the GPU has compute shaders
//! and storage buffers, whether or not any point light exists. The overworld
//! has none in vanilla lighting (the sun is a directional light, which is never
//! clustered), and a dungeon has at most `graphics.dungeon.max_lights`. With
//! GPU clustering off, Bevy assigns those few lights on the CPU instead, which
//! costs next to nothing at these counts.
//!
//! Measured in the `SCENE=world` sandbox (2026-10-07, AMD integrated GPU),
//! on / off / on: 9.84 / 9.32 / 9.93 ms per frame (102.8 / 108.5 / 102.7 FPS).
//! The pass itself read ~0.23 ms of GPU time. Off is therefore the default,
//! following ADR 0011's rule for optional capabilities that measure slower.
//!
//! Bevy re-extracts `GlobalClusterSettings` into the render world whenever it
//! changes, so the switch applies live.

use bevy::light::cluster::{GlobalClusterGpuSettings, GlobalClusterSettings};
use bevy::prelude::*;

use crate::plugins::config::ClientConfig;

pub struct LightClusteringPlugin;

impl Plugin for LightClusteringPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            apply_light_clustering.run_if(crate::plugins::settings::live::config_changed),
        );
    }
}

/// Follows the config into Bevy's cluster settings. Bevy's own GPU settings
/// are kept on the first run, so turning it back on restores exactly what
/// Bevy chose. If Bevy chose none because the GPU cannot do it, it stays off.
fn apply_light_clustering(
    config: Res<ClientConfig>,
    settings: Option<ResMut<GlobalClusterSettings>>,
    mut bevy_choice: Local<Option<Option<GlobalClusterGpuSettings>>>,
) {
    let Some(mut settings) = settings else {
        return;
    };
    let original = *bevy_choice.get_or_insert(settings.gpu_clustering);
    let wanted = if config.graphics.gpu_light_clustering {
        original
    } else {
        None
    };
    if settings.gpu_clustering.is_some() != wanted.is_some() {
        settings.gpu_clustering = wanted;
    }
}
