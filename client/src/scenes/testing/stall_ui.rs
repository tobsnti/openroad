//! Idea: an offline preview of the player stall window. Run with
//! `SCENE=ui_testing`. The window's own Update systems already run in
//! `SceneState::UiTesting`, so this only opens it and fills the model with
//! placeholder rows — there is no wire in #779, and the two strings it seeds
//! are the original's own defaults (`UIIT_STT_STALL_DEFAULT_TITLE` /
//! `_OWNERMSG`, with their `[%s]` owner slot filled in) rather than invented
//! copy.

use bevy::prelude::*;

use crate::plugins::hud::stall::model::{StallRow, StallState, StallTradingState};
use crate::scenes::SceneState;

pub struct StallUiPreviewPlugin;

impl Plugin for StallUiPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(SceneState::UiTesting), open_stall_window);
    }
}

fn open_stall_window(mut state: ResMut<StallState>) {
    state.open = true;
    state.title = "Tobsnti's stall.".to_string();
    state.greeting = "Welcome to Tobsnti's stall.".to_string();
    state.trading = StallTradingState::Open;
    // Three of the ten cells filled, so the preview shows both an occupied and
    // an empty plate in both columns.
    // Real ref ids with the names the player's own data gives them, so the
    // rows resolve an icon (the take-down drag needs one for its ghost) and
    // the label is not an invented item: 3632 = `ITEM_CH_SWORD_01_A_DEF`
    // "Copper Sword" (`itemdata_5000.txt`, name via
    // `textdata_equip&skill.txt`), 4 = `ITEM_ETC_HP_POTION_01` "HP Recovery
    // Herb" and 3681 = `ITEM_ETC_ARCHEMY_REINFORCE_RECIPE_ARMOR_B`
    // "Intensifing Elixir(protector)" (both `textdata_object.txt`). Only the
    // three prices are ours.
    for (index, (ref_id, name, quantity, price)) in [
        (0usize, (3632u32, "Copper Sword", 1u16, 12_500u64)),
        (1, (4, "HP Recovery Herb", 20, 400)),
        (4, (3681, "Intensifing Elixir(protector)", 3, 98_000)),
    ] {
        state.slots[index] = Some(StallRow {
            ref_id,
            name: name.to_string(),
            quantity,
            price,
        });
    }
}
