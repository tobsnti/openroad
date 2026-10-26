//! Idea: an offline preview of the quest list. Run with `SCENE=ui_testing`
//! (`UI_ONLY=quest`). The window itself is built by its own `OnEnter` system,
//! so this only fills what the **network** layer would otherwise write: the
//! player's `CharacterInfo` with active and completed quests, and the window
//! opened.
//!
//! Everything here is **mock**, and the ref ids are chosen to exercise the
//! three readings a row can take (`hud::quest::row_title`): 7, 8, 9 and 10 are
//! ordinary `questdata.txt` rows, and 220 is one of the two ids our own server
//! sends that have **no** row in the 706-row table at all
//! (`assets::textdata::quest`'s own measurement). Without this seed the list
//! shows "No active quest." — which is also what a login against the stub
//! shows today, because the stub sends an empty quest section.

use bevy::prelude::*;

use packets::agent::character_data::ActiveQuest;

use crate::plugins::hud::quest::QuestWindowState;
use crate::plugins::net::character_info::CharacterInfo;
use crate::plugins::player::Player;
use crate::scenes::SceneState;

pub struct QuestUiPreviewPlugin;

impl Plugin for QuestUiPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(SceneState::UiTesting), open_quest_preview)
            // The record lands on the player the frame it appears: the player
            // is spawned by `preview_player` in the same enter run, so it is
            // only queryable after that schedule's sync point. `Added<Player>`
            // is also what keeps this from re-seeding every frame.
            .add_systems(
                Update,
                seed_quest_record.run_if(in_state(SceneState::UiTesting)),
            );
    }
}

/// Ref ids from the player's own `questdata.txt`, plus the one id that is not
/// in it (see the module note).
const QUEST_IDS: [u32; 5] = [7, 8, 9, 220, 10];

fn open_quest_preview(mut state: ResMut<QuestWindowState>) {
    // The window reads `Query<&CharacterInfo, With<Player>>`; that player is
    // spawned by `preview_player`, which `add_ui_previews` registers for this
    // preview.
    state.open = true;
}

fn seed_quest_record(mut commands: Commands, fresh: Query<Entity, Added<Player>>) {
    let active = QUEST_IDS
        .iter()
        .map(|&ref_id| ActiveQuest {
            ref_id,
            achievements: 0,
            auto_share_required: 0,
            quest_type: 0,
            time_remain: None,
            state: 1,
            objectives: vec![],
            target_npcs: vec![],
        })
        .collect();
    let info = CharacterInfo {
        active_quests: active,
        // a plausible completed set, so the count readout has something to say
        completed_quests: (1..=37).collect(),
        ..CharacterInfo::default()
    };
    for player in fresh.iter() {
        commands.entity(player).insert(info.clone());
    }
}
