//! Idea: Bevy frees an asset the moment its last strong handle drops, and the
//! client's dedup caches (`SroMeshes`, `SroAnimationClips`, the material
//! variants, …) are weak by design, so nothing outlives the entities using it.
//! That is right for memory, but an MMO constantly drops and re-requests the
//! same things within seconds: a monster dies and the same type respawns, a
//! player steps back over a region line, a crowd walks out of range and back.
//! Every such round trip re-read the files from the PK2 and re-decoded them —
//! a trace showed terrain lightmaps decoded twice within 200 ms, at 40–65 ms
//! each, and character textures several times inside one login scene.
//!
//! This keeps a *released* asset alive for a short grace period instead: when a
//! spawned resource or a streamed region goes away, its root handles are parked
//! here, and a sweep drops whatever has not been reclaimed within
//! [`RESIDENCY_GRACE`]. A re-request in that window finds the asset still
//! loaded (the asset server dedups by path), so nothing is decoded again. The
//! parked handles hold their whole dependency tree (a `.bsr` keeps its meshes,
//! material sets and textures), which is the point — and also why the window
//! is short and the set is counted (`cache_counts/resident_assets`).

use std::collections::HashMap;
use std::time::Duration;

use bevy::asset::{UntypedAssetId, UntypedHandle};
use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;

use crate::commands::SpawnedFromResource;

/// How long a released asset stays loaded. Long enough to cover the common
/// round trips (a respawn timer's worth of a dead monster, stepping back over
/// a region line, a crowd briefly leaving view range); short enough that
/// walking steadily away from an area frees it well before the next area's
/// worth of assets has streamed in. A starting value, not a measurement:
/// watch `cache_counts/resident_assets` against `process/mem_usage`.
pub const RESIDENCY_GRACE: Duration = Duration::from_secs(30);

/// How often expired entries are dropped. Only bounds how far past
/// [`RESIDENCY_GRACE`] an entry can linger; the sweep itself is a map scan.
const SWEEP_INTERVAL: Duration = Duration::from_secs(5);

/// Released handles, keyed by asset id, with the time they were parked.
#[derive(Resource)]
pub struct AssetResidency {
    parked: HashMap<UntypedAssetId, (UntypedHandle, Duration)>,
    /// How long a parked asset stays: [`RESIDENCY_GRACE`] unless
    /// `graphics.streaming.residency_grace_secs` says otherwise.
    grace: Duration,
}

impl Default for AssetResidency {
    fn default() -> Self {
        Self {
            parked: HashMap::default(),
            grace: RESIDENCY_GRACE,
        }
    }
}

impl AssetResidency {
    /// Keep `handle`'s asset loaded for the grace period from `now`
    /// (parking an already-parked asset restarts its window).
    pub fn park(&mut self, handle: UntypedHandle, now: Duration) {
        self.parked.insert(handle.id(), (handle, now));
    }

    /// Drop every entry parked longer than the grace period ago.
    fn sweep(&mut self, now: Duration) {
        let grace = self.grace;
        self.parked
            .retain(|_, (_, parked_at)| now.saturating_sub(*parked_at) < grace);
    }

    pub fn len(&self) -> usize {
        self.parked.len()
    }
}

pub struct AssetResidencyPlugin;

impl Plugin for AssetResidencyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AssetResidency>()
            .add_observer(park_released_resource)
            .add_systems(Update, sweep_residency.run_if(on_timer(SWEEP_INTERVAL)))
            .add_systems(
                PreUpdate,
                apply_residency_grace
                    .run_if(resource_exists_and_changed::<crate::plugins::config::ClientConfig>),
            );
    }
}

/// Follows `graphics.streaming.residency_grace_secs`. A shorter grace frees
/// a departed area's meshes and textures sooner, which matters on a 4 GB
/// machine, at the cost of re-decoding them on a quick return. Negative or
/// non-finite values keep the default.
fn apply_residency_grace(
    config: Res<crate::plugins::config::ClientConfig>,
    mut residency: ResMut<AssetResidency>,
) {
    let secs = config.graphics.streaming.residency_grace_secs;
    let grace = if secs.is_finite() && secs >= 0.0 {
        Duration::from_secs_f32(secs)
    } else {
        RESIDENCY_GRACE
    };
    if residency.grace != grace {
        residency.grace = grace;
    }
}

/// A spawned resource wrapper going away (despawn, or a detach) parks its
/// `.bsr` handle, and with it the meshes, material sets and textures it owns.
fn park_released_resource(
    remove: On<Remove, SpawnedFromResource>,
    from: Query<&SpawnedFromResource>,
    residency: Option<ResMut<AssetResidency>>,
    time: Res<Time>,
) {
    let (Ok(from), Some(mut residency)) = (from.get(remove.entity), residency) else {
        return;
    };
    residency.park(from.0.clone().untyped(), time.elapsed());
}

fn sweep_residency(mut residency: ResMut<AssetResidency>, time: Res<Time>) {
    residency.sweep(time.elapsed());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle() -> UntypedHandle {
        Handle::<Image>::default().untyped()
    }

    #[test]
    fn a_parked_asset_lives_for_the_grace_period_then_goes() {
        let mut residency = AssetResidency::default();
        residency.park(handle(), Duration::from_secs(100));

        residency.sweep(Duration::from_secs(100) + RESIDENCY_GRACE - Duration::from_millis(1));
        assert_eq!(residency.len(), 1);

        residency.sweep(Duration::from_secs(100) + RESIDENCY_GRACE);
        assert_eq!(residency.len(), 0);
    }

    #[test]
    fn parking_again_restarts_the_window() {
        let mut residency = AssetResidency::default();
        residency.park(handle(), Duration::from_secs(0));
        residency.park(handle(), Duration::from_secs(20));
        assert_eq!(residency.len(), 1, "one entry per asset");

        residency.sweep(RESIDENCY_GRACE + Duration::from_secs(1));
        assert_eq!(residency.len(), 1, "the second park extended it");
    }
}
