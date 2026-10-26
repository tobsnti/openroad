//! Gallery preview for the quest detail pane (`UI_ONLY=quest_detail`).
//!
//! Idea: the pane only ever shows what the wire gave it, so the preview seeds
//! one record of each shape our own captures contain and nothing else — a
//! hunt objective with a counter, a three-objective quest, and a quest with a
//! target NPC. The ids and the key are the ones the proxy recording of the
//! original client really carries
//! (`packet_dump/proxy/0x3013.log`, frame 2).

use bevy::prelude::*;
use packets::agent::character_data::{ActiveQuest, QuestObjective, QUEST_TYPE_NPC_LIST};

use crate::plugins::hud::quest_detail::{spawn_quest_detail, QuestDetailState};
use crate::plugins::net::character_info::CharacterInfo;
use crate::plugins::player::Player;
use crate::scenes::SceneState;

/// `ref_id`, `quest_type`, objective keys and target NPC, all as captured.
/// Quest 3 is the one record of ours that carries a target NPC (ref 2037 ->
/// `NPC_CH_SOLDIER_SO1` -> "Soldier Jingyo [Teleport]"), quest 57 is the one
/// with three objectives, and quest 11 is a plain hunt line.
fn captured_quests() -> Vec<ActiveQuest> {
    let objective = |name: &str, enabled: bool, tasks: Vec<u32>| QuestObjective {
        id: 1,
        enabled,
        name: name.to_string(),
        tasks,
    };
    let record = |ref_id, quest_type, objectives, target_npcs| ActiveQuest {
        ref_id,
        achievements: 16,
        auto_share_required: 0,
        quest_type,
        time_remain: None,
        state: 1,
        objectives,
        target_npcs,
    };
    vec![
        record(
            3,
            QUEST_TYPE_NPC_LIST,
            vec![objective("SN_CON_QTUTORIAL2_CH_1", false, vec![1])],
            vec![2037],
        ),
        record(
            11,
            24,
            vec![objective("SN_CON_QNO_CH_POTION_1", true, vec![0])],
            Vec::new(),
        ),
        record(
            57,
            24,
            vec![
                objective("SN_CON_QNO_CH_GENARAL_1_01", true, vec![0]),
                objective("SN_CON_QNO_CH_GENARAL_1_02", true, vec![0]),
                objective("SN_CON_QNO_CH_GENARAL_1_03", true, vec![0]),
            ],
            Vec::new(),
        ),
    ]
}

pub struct QuestDetailPreviewPlugin;

impl Plugin for QuestDetailPreviewPlugin {
    fn build(&self, app: &mut App) {
        // The pane's own spawn system is GameWorld-only, so the preview pulls
        // it in and opens it afterwards — the shape `inventory_ui` uses.
        app.add_systems(
            OnEnter(SceneState::UiTesting),
            (spawn_quest_detail, open_quest_detail_preview).chain(),
        )
        // The player is spawned through `Commands`, so it does not exist
        // inside the `OnEnter` system; `Added<Player>` is also what keeps
        // this from re-seeding every frame.
        .add_systems(
            Update,
            seed_quest_detail.run_if(in_state(SceneState::UiTesting)),
        );
    }
}

fn open_quest_detail_preview(mut detail: ResMut<QuestDetailState>) {
    // After the spawn, which resets the resource. The player comes from
    // `PreviewPlayerPlugin`, which `add_ui_previews` registers for the names
    // in `PLAYER_PREVIEWS`; clicking a list row is the live way in, so the
    // preview sets the selection itself.
    detail.open = true;
    detail.selected = 1;
}

fn seed_quest_detail(mut commands: Commands, fresh: Query<Entity, Added<Player>>) {
    let info = CharacterInfo {
        active_quests: captured_quests(),
        completed_quests: (1..=37).collect(),
        ..default()
    };
    for player in fresh.iter() {
        commands.entity(player).insert(info.clone());
    }
}
