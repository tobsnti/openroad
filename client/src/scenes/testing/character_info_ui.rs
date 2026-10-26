//! Idea: an offline preview of the character info window (C) with a mock
//! stat sheet — a mid-level character with spendable stat points so the +
//! buttons show, believable attack/defense ranges and partial HP/MP fills.
//! Run with `SCENE=ui_testing`. The window's own Update systems already run
//! in `SceneState::UiTesting`, so this only spawns the window, seeds the
//! mock `PlayerStats`/`PlayerVitals`/`PlayerProgress` resources and opens it.

use bevy::prelude::*;

use packets::agent::character_data::JobInfo;
use packets::agent::prelude::CharacterStatsUpdate;

use crate::plugins::hud::character_info::model::{CharacterInfoState, PlayerStats};
use crate::plugins::hud::character_info::ui::spawn_character_info_window;
use crate::plugins::hud::player_mini_info::PlayerVitals;
use crate::plugins::hud::underbar::model::PlayerProgress;
use crate::plugins::net::character_info::CharacterInfo;
use crate::plugins::player::Player;
use crate::scenes::SceneState;

pub struct CharacterInfoUiPreviewPlugin;

impl Plugin for CharacterInfoUiPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(SceneState::UiTesting),
            (seed_mock_stats, spawn_character_info_window).chain(),
        )
        // The job strip reads the player's `CharacterInfo`, and the player is
        // spawned through `Commands` by `PreviewPlayerPlugin`, so it is not
        // queryable in the `OnEnter` run above.
        .add_systems(
            Update,
            seed_captured_job.run_if(in_state(SceneState::UiTesting)),
        );
    }
}

fn seed_mock_stats(
    mut stats: ResMut<PlayerStats>,
    mut vitals: ResMut<PlayerVitals>,
    mut progress: ResMut<PlayerProgress>,
    mut state: ResMut<CharacterInfoState>,
) {
    stats.sheet = Some(CharacterStatsUpdate {
        phys_attack_min: 312,
        phys_attack_max: 388,
        mag_attack_min: 501,
        mag_attack_max: 622,
        phys_defense: 148,
        mag_defense: 201,
        hit_rate: 96,
        parry_rate: 87,
        max_hp: 2210,
        max_mp: 2890,
        strength: 41,
        intelligence: 58,
    });
    stats.stat_points = 9;

    vitals.name = "Testcharacter".into();
    vitals.level = 32;
    vitals.hp = 1656;
    vitals.mp = 2890;
    vitals.max_hp = Some(2210);
    vitals.max_mp = Some(2890);

    progress.level = 32;
    progress.exp_offset = 1_284_550;

    state.open = true;
}

/// The one real job record in our captures, byte for byte:
/// `packet_dump/proxy/0x3013.log` frame 2 (a recording of the original
/// client) — `job_name="jobname"`, type 1, level 2, exp 221737, contribution
/// 73336, reward 0. Every other `0x3013` frame we hold carries type 0 with
/// level 1, which is what a jobless character sends.
fn seed_captured_job(mut commands: Commands, fresh: Query<Entity, Added<Player>>) {
    for player in fresh.iter() {
        commands.entity(player).insert(CharacterInfo {
            job: Some(JobInfo {
                job_name: "jobname".into(),
                job_type: 1,
                job_level: 2,
                job_exp: 221_737,
                job_contribution: 73_336,
                job_reward: 0,
            }),
            ..default()
        });
    }
}
