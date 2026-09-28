//! Idea: an offline preview of the COS (companion) window on its **Setting**
//! page — `GDR_COS_SETUP` (`resinfo/ifcos.txt` id 123, `ifcossetup.txt`). Run
//! with `SCENE=ui_testing`.
//!
//! Why the Setting page and not the default Info page: the three pages share
//! one rect (`12,66,331,314`), so "the tab really selects the page" is only
//! visible on a page that is *not* the default. This preview therefore sets
//! `CosWindowState::page = CosPage::Setup` before the window spawns, which is
//! the same state the tab button writes.
//!
//! The flag values are the pick-pet settings bitfield's own defaults as the
//! window stages them (`hud/cos/setup.rs`: `CosSetupState::edited`); nothing is
//! invented here, and no pet data is faked — `CosState` stays empty, so the
//! Info page's statics would read as the "no pet" state, which is what an
//! offline client honestly has.

use bevy::prelude::*;

use crate::plugins::hud::cos::model::{CosPage, CosWindowState};
use crate::plugins::hud::cos::ui;
use crate::scenes::SceneState;

pub struct CosUiPreviewPlugin;

impl Plugin for CosUiPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(SceneState::UiTesting),
            (open_cos_window_on_the_setup_page, ui::spawn_cos_window).chain(),
        )
        .add_systems(OnExit(SceneState::UiTesting), ui::cleanup_cos_window);
    }
}

fn open_cos_window_on_the_setup_page(mut state: ResMut<CosWindowState>) {
    state.open = true;
    // `OPENROAD_PREVIEW_COS_PAGE=info|inventory|setup` picks the page. Added
    // 2026-08-24 for the cargo page specifically: `ui.rs` states that while it
    // shows, *neither* tab is lit (vanilla names only two tab entries), and a
    // unit test pins that — but "no tab is lit" is a claim about a picture, so
    // the picture has to exist. Default stays Setup, see the module doc.
    state.page = match std::env::var("OPENROAD_PREVIEW_COS_PAGE").as_deref() {
        Ok("info") => CosPage::Info,
        Ok("inventory") => CosPage::Inventory,
        _ => CosPage::Setup,
    };
}
