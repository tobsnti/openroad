//! Frame-rate caps (`window_settings.fps_limit`, `unfocused_fps_limit`).
//!
//! An uncapped client renders as fast as the GPU allows, which on an old
//! laptop means a GPU and CPU pinned at 100% until thermal throttling halves
//! the clock and the frame rate with it. A cap above the rate the hardware
//! can sustain avoids that, and costs nothing visible once it is at or above
//! the display's refresh.
//!
//! The focused cap is a sleep at the end of the frame, in `Last`, until the
//! frame has taken the target time. No vsync is involved, so it works under
//! every present mode, and Rust's `sleep` uses high-resolution timers on
//! Windows 10 1803+, accurate to about a millisecond. The unfocused cap uses
//! Bevy's own `WinitSettings` reactive mode, so a backgrounded client wakes
//! for window events or at the capped rate. The networking keeps running at
//! that rate, which only matters below ~10 FPS.
//!
//! Both are off by default, so the throughput numbers the perf workflow reads
//! (`docs/perf-remote.md`) stay uncapped unless asked for.

use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy::winit::{UpdateMode, WinitSettings};

use crate::plugins::config::ClientConfig;

pub struct FramePacingPlugin;

impl Plugin for FramePacingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FrameLimit>()
            .add_systems(
                PreUpdate,
                apply_frame_pacing.run_if(crate::plugins::settings::live::config_changed),
            )
            .add_systems(Last, limit_frame_rate);
    }
}

/// The minimum focused frame time, if capped.
#[derive(Resource, Default)]
struct FrameLimit(Option<Duration>);

/// The frame period for a cap of `fps`: `None` for 0 (off), negative or
/// non-finite values.
fn period(fps: f32) -> Option<Duration> {
    (fps.is_finite() && fps > 0.0).then(|| Duration::from_secs_f32(1.0 / fps))
}

fn apply_frame_pacing(
    config: Res<ClientConfig>,
    mut limit: ResMut<FrameLimit>,
    winit: Option<ResMut<WinitSettings>>,
) {
    limit.0 = period(config.window_settings.fps_limit);
    if let Some(mut winit) = winit {
        // 0 keeps Bevy's own background mode (`WinitSettings::game`: about
        // 60 Hz), which is what the client always ran with
        winit.unfocused_mode = match period(config.window_settings.unfocused_fps_limit) {
            Some(wait) => UpdateMode::reactive_low_power(wait),
            None => WinitSettings::game().unfocused_mode,
        };
    }
}

/// Sleeps out the rest of the frame when it finished faster than the cap.
fn limit_frame_rate(limit: Res<FrameLimit>, mut frame_start: Local<Option<Instant>>) {
    if let (Some(min), Some(start)) = (limit.0, *frame_start) {
        let elapsed = start.elapsed();
        if elapsed < min {
            std::thread::sleep(min - elapsed);
        }
    }
    *frame_start = Some(Instant::now());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_and_nonsense_mean_uncapped() {
        assert_eq!(period(0.0), None);
        assert_eq!(period(-30.0), None);
        assert_eq!(period(f32::NAN), None);
        assert_eq!(period(60.0), Some(Duration::from_secs_f32(1.0 / 60.0)));
    }
}
