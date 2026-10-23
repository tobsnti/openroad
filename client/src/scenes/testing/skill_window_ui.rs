//! Idea: an offline preview of the skill window. Run with `SCENE=ui_testing`.
//! The window is spawned by the `S` key in a live session, so a UI-testing run
//! has to spawn it here.
//!
//! **Mock:** only the two things the **network** layer writes — the character's
//! level and skill points (`PlayerProgress`, from `0x3013` and its updates).
//! Everything the window actually lists — masteries, branches, skill names,
//! icons, the level ladders — is read from the player's own textdata
//! (`ClientMasteryData`, `ClientSkillGroups`, `ClientSkillData`), which the
//! UI-testing scene loads like any other scene. Nothing here invents a skill:
//! what is learnable with 7 points at level 40 is what the player's own data
//! says it is.

use bevy::prelude::*;

use crate::plugins::hud::skill_window::model::SkillWindowState;
use crate::plugins::hud::skill_window::ui::spawn_skill_window;
use crate::plugins::hud::underbar::model::PlayerProgress;
use crate::scenes::SceneState;

pub struct SkillWindowUiPreviewPlugin;

impl Plugin for SkillWindowUiPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(SceneState::UiTesting),
            (seed_mock_progress, spawn_skill_window, open_skill_window).chain(),
        );
    }
}

/// A character far enough in to have a tree worth looking at, with points left
/// so the window draws both the learned and the learnable state.
fn seed_mock_progress(mut progress: ResMut<PlayerProgress>) {
    progress.level = 40;
    progress.exp_offset = 1_200_000;
    progress.skill_exp = 150;
    progress.skill_points = 7;
}

/// `spawn_skill_window` resets the state it is given, so opening has to come
/// after it rather than with the seed.
fn open_skill_window(mut state: ResMut<SkillWindowState>) {
    state.open = true;
}
