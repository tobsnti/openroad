//! Idea: an offline preview of the in-game player mini-info HUD with mock
//! vitals, so the pixel layout, bar fills and the portrait render-to-texture
//! can be iterated without logging into a live server. Run with
//! `SCENE=ui_testing`; combine with `OPENROAD_SCREENSHOT` for headless
//! capture. The HUD's own Update systems (refresh, portrait rig/tagging) are
//! already gated to also run in `SceneState::UiTesting`, so this only spawns
//! the panel and the fake numbers. The character to portray comes from
//! `preview_player`, which owns the single preview player.

use bevy::prelude::*;

use crate::plugins::hud::player_mini_info::{spawn_mini_info, PlayerVitals};
use crate::scenes::SceneState;

pub struct MiniInfoUiPreviewPlugin;

impl Plugin for MiniInfoUiPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(SceneState::UiTesting),
            (spawn_mini_info, spawn_mock_data).chain(),
        );
    }
}

fn spawn_mock_data(mut vitals: ResMut<PlayerVitals>) {
    *vitals = PlayerVitals {
        name: "Testchar".to_string(),
        level: 42,
        hp: 3450,
        mp: 2210,
        max_hp: Some(5230),
        max_mp: Some(4180),
        berserk_pips: 3,
        hwan_level: 0,
    };
}
