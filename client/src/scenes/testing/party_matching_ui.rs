//! Idea: an offline preview of the party-match board. Run with
//! `SCENE=ui_testing`. `party_ui.rs` already spawns this window as part of the
//! party family, but it spawns four surfaces at once — with `UI_ONLY` that is
//! one picture of everything rather than one picture of the board, which is
//! what a per-window gallery needs. So this registers the board under its own
//! name and fills only what it draws.
//!
//! **Mock:** the twelve listed entries and the page counter — they come from
//! `0xB06C`, a push no offline scene receives. The row shapes are the wire's
//! (`PartyMatchEntry`), and the values vary per row on purpose: different
//! races, setups, purposes, member counts and level bands, so every column of
//! the board shows something different instead of twelve identical lines.

use bevy::prelude::*;

use packets::agent::party::PartyMatchEntry;

use crate::plugins::hud::party_matching::model::PartyMatchState;
use crate::plugins::hud::party_matching::ui::spawn_match_window;
use crate::scenes::SceneState;

pub struct PartyMatchingUiPreviewPlugin;

impl Plugin for PartyMatchingUiPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(SceneState::UiTesting),
            (seed_mock_listings, spawn_match_window).chain(),
        );
    }
}

fn seed_mock_listings(mut state: ResMut<PartyMatchState>) {
    let entry = |number: u32,
                 master: &str,
                 race_type: u8,
                 member_count: u8,
                 setup: u8,
                 purpose: u8,
                 level_min: u8,
                 level_max: u8,
                 title: &str| PartyMatchEntry {
        number,
        // a unix timestamp, see the field's own doc — stepped per row so the
        // list is not twelve identical instants
        registered_at: 1_750_000_000 + number * 70,
        master_name: master.to_string(),
        race_type,
        member_count,
        setup,
        purpose,
        level_min,
        level_max,
        title: title.to_string(),
    };

    state.entries = vec![
        entry(5, "Tobsnti", 0, 3, 1, 0, 30, 45, "Jangan cave, exp run"),
        entry(6, "Ravenna", 1, 1, 0, 1, 40, 60, "Looking for a cleric"),
        entry(7, "Kazim", 0, 7, 1, 1, 20, 35, "Donwhang trade escort"),
        entry(8, "Mira", 1, 5, 0, 0, 50, 70, "Uruchi, share loot"),
    ];
    state.page_index = 0;
    state.page_count = 3;
    state.selected = Some(6);
    // the row that is ours pins to the top and draws in the configured tint
    state.own_party_master = Some("Tobsnti".to_string());
    state.open = true;
}
