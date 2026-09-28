//! Idea: an offline preview of the Academy ("Training Camp") member panel.
//! Run with `SCENE=ui_testing`.
//!
//! Two things this module does that the other previews do not have to:
//!
//! 1. The panel's own `Update` systems already run in the HUD scenes
//!    (`hud/academy/mod.rs`: `.run_if(super::hud_scenes)`), but its **spawn**
//!    hangs off `OnEnter(SceneState::GameWorld)` alone, so nothing exists to
//!    make visible here. Rather than widen the production registration for a
//!    screenshot, this preview registers the very same public spawn/cleanup
//!    systems for `SceneState::UiTesting` — the window that gets photographed
//!    is therefore bit-for-bit the in-game one.
//! 2. There is **no placeholder roster**, on purpose. The member list has no
//!    wire source in this tree (`0x3C81` is deliberately unwired because the
//!    original's own handler reads zero bytes of it — `hud/academy/mod.rs`
//!    module doc, `docs/net-academy-0x3C81.md`), so the seven slots and the
//!    message board render their **empty state**, which is the shipped
//!    sentence from the user's own string table. Seeding invented member names
//!    would be exactly the unsourced value ADR-0009 forbids, and a later
//!    reader would take them for decoded data.

use bevy::prelude::*;

use crate::plugins::hud::academy::{ui, AcademyState};
use crate::scenes::SceneState;

pub struct AcademyUiPreviewPlugin;

impl Plugin for AcademyUiPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(SceneState::UiTesting),
            (open_academy_panel, ui::spawn_academy_window).chain(),
        )
        .add_systems(OnExit(SceneState::UiTesting), ui::cleanup_academy_window);
    }
}

fn open_academy_panel(mut state: ResMut<AcademyState>) {
    state.open = true;
}
