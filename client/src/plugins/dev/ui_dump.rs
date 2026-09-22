//! Idea: a UI screen that is missing from a screenshot has exactly four
//! possible causes — never spawned, laid out off-screen, fully transparent, or
//! covered by a node that draws later. Guessing between them from a PNG is what
//! this dump exists to stop. With `OPENROAD_UI_DUMP=<seconds>` the client walks
//! `UiStack` (the very list the renderer draws back-to-front) once at that time
//! and prints one line per node: draw order, name, screen rect, and colour.
//! Inert unless the variable is set.

use bevy::prelude::*;
use bevy::ui::{ComputedNode, UiGlobalTransform, UiStack};

pub struct UiDumpPlugin;

#[derive(Resource)]
struct UiDump {
    at: f32,
    done: bool,
}

impl Plugin for UiDumpPlugin {
    fn build(&self, app: &mut App) {
        let Ok(raw) = std::env::var("OPENROAD_UI_DUMP") else {
            return;
        };
        let Some(at) = parse_dump_time(raw.trim()) else {
            warn!("OPENROAD_UI_DUMP={raw:?} is not a number of seconds");
            return;
        };
        app.insert_resource(UiDump { at, done: false });
        app.add_systems(Update, dump_ui_stack);
    }
}

/// The dump time, or `None` for an input that cannot be one.
///
/// `"nan"` and `"inf"` parse as `f32`, and the schedule compares with
/// `elapsed_secs() < at` — which is false for NaN, so a typo would dump at the
/// first frame, before any UI exists, and claim to have dumped the screen the
/// caller asked about. A negative value does the same. Both are refused here.
fn parse_dump_time(raw: &str) -> Option<f32> {
    raw.parse::<f32>()
        .ok()
        .filter(|at| at.is_finite() && *at >= 0.0)
}

fn dump_ui_stack(
    time: Res<Time>,
    mut state: ResMut<UiDump>,
    stack: Res<UiStack>,
    nodes: Query<(
        &ComputedNode,
        &UiGlobalTransform,
        Option<&Name>,
        Option<&ImageNode>,
        Option<&TextColor>,
        Option<&Text>,
        &InheritedVisibility,
    )>,
) {
    if state.done || time.elapsed_secs() < state.at {
        return;
    }
    state.done = true;
    info!("ui dump: {} nodes in draw order", stack.uinodes.len());
    for (i, entity) in stack.uinodes.iter().enumerate() {
        let Ok((node, transform, name, image, text_color, text, visible)) = nodes.get(*entity)
        else {
            continue;
        };
        let size = node.size();
        let center = transform.translation;
        info!(
            "ui[{i:>3}] {name} center=({:.0},{:.0}) size=({:.0}x{:.0}) vis={} img={:?} text={:?} {:?}",
            center.x,
            center.y,
            size.x,
            size.y,
            visible.get(),
            image.map(|i| i.color.to_srgba().to_f32_array()),
            text_color.map(|c| c.0.to_srgba().to_f32_array()),
            text.map(|t| t.0.clone()),
            name = name.map(|n| n.as_str().to_string()).unwrap_or_default(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dump time that is not a real instant must be refused, not turned into
    /// a dump at frame 0: `elapsed_secs() < NaN` is false, so the guard that is
    /// supposed to wait lets the very first frame through.
    #[test]
    fn a_nan_or_negative_dump_time_is_refused() {
        assert_eq!(parse_dump_time("2.5"), Some(2.5));
        assert_eq!(parse_dump_time("0"), Some(0.0));
        assert_eq!(parse_dump_time("nan"), None);
        assert_eq!(parse_dump_time("inf"), None);
        assert_eq!(parse_dump_time("-1"), None);
        assert_eq!(parse_dump_time("later"), None);
    }
}
