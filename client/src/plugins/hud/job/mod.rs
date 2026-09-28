//! The job-league windows (`docs/re/ui/hud-job-system.md`, #102 / EP-20).
//!
//! First tenant: `ifprevjobinfo`, which is also the first window in this tree
//! drawn on the measured `msgbox2_window_` chrome (#664,
//! `docs/re/ui/msgbox2-chrome.md`). Since Etappe 3 point 8 the two ranking
//! windows (`ifjobrank`, `ifjobcontributionrank`) live here too. The alias, the
//! trade calculator and the export details are still missing.

pub mod prev_info;
pub mod ranking;

pub use prev_info::PrevJobInfoPlugin;
pub use ranking::JobRankingPlugin;

use bevy::app::{PluginGroup, PluginGroupBuilder};

/// The tree's windows as one registration, so the shared HUD plugin registry
/// keeps a single line per subsystem — its tuples top out at 15 entries and it
/// is a three-lane hotspot (`hud/mod.rs`).
pub struct JobWindowsPlugin;

impl PluginGroup for JobWindowsPlugin {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(PrevJobInfoPlugin)
            .add(JobRankingPlugin)
    }
}
