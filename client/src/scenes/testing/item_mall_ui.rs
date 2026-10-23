//! Idea: an offline preview of the Item Mall's "My information" pane. Run with
//! `SCENE=ui_testing`. The pane is spawned `OnEnter(SceneState::GameWorld)` in
//! the live client, so a UI-testing run has to spawn it here, and the three
//! silk counters come from `0x3153` — a push no offline scene ever receives.
//!
//! **Mock:** the three counters and the name/level line. The values are
//! deliberately **different from each other** (and the point counter non-zero,
//! which no captured frame of the user's own account has), so each of the
//! three rows shows a different number instead of three zeros. Everything
//! else — the rects, the labels, the title, the font size — is the player's
//! own data through `ifitemmallmyinfo.txt` and `textuisystem.txt`.

use bevy::prelude::*;

use crate::plugins::hud::item_mall::{ui::spawn_my_info_window, ItemMallState};
use crate::plugins::hud::player_mini_info::PlayerVitals;
use crate::plugins::net::silk::{SilkBalance, SilkCounters};
use crate::scenes::SceneState;

pub struct ItemMallUiPreviewPlugin;

impl Plugin for ItemMallUiPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(SceneState::UiTesting),
            (seed_mock_silk, spawn_my_info_window).chain(),
        );
    }
}

fn seed_mock_silk(
    mut balance: ResMut<SilkBalance>,
    mut vitals: ResMut<PlayerVitals>,
    mut state: ResMut<ItemMallState>,
) {
    balance.0 = Some(SilkCounters {
        own: 1_000_000_500,
        gift: 50_000,
        point: 1_250,
    });
    // the identity row reads the same aggregate the mini-info panel does
    vitals.name = "Tobsnti".to_string();
    vitals.level = 40;
    state.open = true;
}
