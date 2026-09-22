//! Idea: headless-friendly visual debugging. When `OPENROAD_SCREENSHOT=/path/prefix` is set,
//! this plugin saves periodic screenshots of the primary window via bevy's built-in
//! `Screenshot` entity and exits the app after the last one. This lets a script or CI
//! launch the real client, record what actually renders, and inspect the PNGs — no human
//! at the window, no compositor-specific tools. Inert unless the env var is set.

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

const SHOT_TIMES_SECS: [f32; 3] = [2.0, 2.8, 3.6];
const EXIT_TIME_SECS: f32 = 4.1;
/// Grace between the last shot and the exit, kept from the default ladder
/// (`4.1 - 3.6`): the screenshot is written by an observer one frame later, so
/// exiting on the same instant loses the last PNG.
const EXIT_GRACE_SECS: f32 = 0.5;

pub struct AutoScreenshotPlugin;

impl Plugin for AutoScreenshotPlugin {
    fn build(&self, app: &mut App) {
        let Ok(prefix) = std::env::var("OPENROAD_SCREENSHOT") else {
            return;
        };
        // Announced at startup, not only at the exit: `make run` sources `.env`, so a
        // forgotten screenshot variable there ends someone else's session after a few
        // seconds. Seeing it in the first log line beats diagnosing it afterwards.
        warn!("OPENROAD_SCREENSHOT is set — this run exits itself after the last shot");
        app.insert_resource(AutoScreenshot {
            prefix,
            taken: 0,
            times: shot_times(),
        });
        app.add_systems(Update, take_screenshots);
    }
}

/// Shot ladder, `OPENROAD_SCREENSHOT_AT="8,10,12"` overriding the default.
/// The default ends at 3.6 s and only ever reaches the first screen of a scene;
/// anything behind a login comes seconds later.
fn shot_times() -> Vec<f32> {
    match std::env::var("OPENROAD_SCREENSHOT_AT") {
        Ok(raw) => parse_shot_times(&raw),
        Err(_) => SHOT_TIMES_SECS.to_vec(),
    }
}

/// The parsing half of [`shot_times`], kept apart from the environment so it
/// can be tested without mutating process state.
///
/// An entry is dropped with a warning rather than silently changing the ladder,
/// and a ladder that ends up empty falls back to the default. Three kinds of
/// entry are rejected, all of which produce a run that cannot do its job:
/// unparseable text; a non-finite value (`"nan"` and `"inf"` *do* parse as
/// `f32`, and `elapsed >= NaN` is false forever — the run would never shoot and
/// never exit); and a negative value, which fires at the first frame and exits
/// the client before anything has rendered.
fn parse_shot_times(raw: &str) -> Vec<f32> {
    let mut times: Vec<f32> = raw
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            match part.parse::<f32>() {
                Ok(secs) if secs.is_finite() && secs >= 0.0 => Some(secs),
                Ok(secs) => {
                    warn!("OPENROAD_SCREENSHOT_AT: ignoring out-of-range entry '{secs}'");
                    None
                }
                Err(_) => {
                    warn!("OPENROAD_SCREENSHOT_AT: ignoring unparseable entry '{part}'");
                    None
                }
            }
        })
        .collect();
    if times.is_empty() {
        return SHOT_TIMES_SECS.to_vec();
    }
    times.sort_by(f32::total_cmp);
    times
}

#[derive(Resource)]
struct AutoScreenshot {
    prefix: String,
    taken: usize,
    times: Vec<f32>,
}

fn take_screenshots(
    mut commands: Commands,
    time: Res<Time>,
    mut state: ResMut<AutoScreenshot>,
    mut exit: MessageWriter<AppExit>,
) {
    let elapsed = time.elapsed_secs();
    if state.taken < state.times.len() && elapsed >= state.times[state.taken] {
        let path = format!("{}_{}.png", state.prefix, state.taken);
        info!("auto-screenshot -> {path}");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        state.taken += 1;
    }
    let exit_at = state
        .times
        .last()
        .map(|last| last + EXIT_GRACE_SECS)
        .unwrap_or(EXIT_TIME_SECS);
    if elapsed >= exit_at {
        // A silent exit reads as a crash; name the culprit before leaving.
        info!(
            "auto-screenshot: {} shot(s) taken, exiting (OPENROAD_SCREENSHOT is set)",
            state.taken
        );
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ladder parser, including its negative case: an override that parses
    /// to nothing must fall back to the default rather than produce a run that
    /// takes no picture and never exits.
    ///
    /// Driven through [`parse_shot_times`], not through the environment: a test
    /// that sets `OPENROAD_SCREENSHOT_AT` would race every other test in the
    /// process, because cargo runs a test binary on several threads by default.
    #[test]
    fn the_default_ladder_survives_a_broken_override() {
        assert_eq!(parse_shot_times("nonsense,"), SHOT_TIMES_SECS.to_vec());
        assert_eq!(parse_shot_times(""), SHOT_TIMES_SECS.to_vec());
        assert_eq!(parse_shot_times("12, 8,10"), vec![8.0, 10.0, 12.0]);
    }

    /// `"nan"`, `"inf"` and `"-5"` all parse as `f32`, so the parser has to
    /// reject them itself. A NaN entry makes `elapsed >= times[taken]` false
    /// for good: the run saves nothing and never reaches its own exit.
    /// A negative entry is the mirror image — it is already due at frame 0.
    #[test]
    fn a_nan_or_negative_entry_never_reaches_the_ladder() {
        assert_eq!(parse_shot_times("nan"), SHOT_TIMES_SECS.to_vec());
        assert_eq!(parse_shot_times("inf"), SHOT_TIMES_SECS.to_vec());
        assert_eq!(parse_shot_times("-5"), SHOT_TIMES_SECS.to_vec());
        assert_eq!(parse_shot_times("nan,8,-5,inf"), vec![8.0]);
        assert!(
            parse_shot_times("nan,8").iter().all(|t| t.is_finite()),
            "no non-finite value survives into the ladder"
        );
    }
}
