//! Idea: an offline preview of the player stall window **and the price/quantity
//! box that stocks it** (`hud/stall/stock.rs`, `0x70BA` type 1). Run with
//! `SCENE=ui_testing`. The window's own Update systems already run in
//! `SceneState::UiTesting`, so this only opens it and fills the model with
//! placeholder rows — there is no wire in #779, and the two strings it seeds
//! are the original's own defaults (`UIIT_STT_STALL_DEFAULT_TITLE` /
//! `_OWNERMSG`, with their `[%s]` owner slot filled in) rather than invented
//! copy.

use bevy::prelude::*;

use crate::plugins::hud::stall::model::{StallRow, StallState, StallTradingState};
use crate::plugins::hud::stall::stock::{reprice_prompt, StockModal};
use crate::plugins::hud::stall::ui::StallWindowRoot;
use crate::scenes::SceneState;

pub struct StallUiPreviewPlugin;

impl Plugin for StallUiPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(SceneState::UiTesting), open_stall_window)
            .add_systems(
                Update,
                open_stock_box.run_if(in_state(SceneState::UiTesting)),
            );
    }
}

/// Open the price/quantity box over the stall on the first filled row, through
/// the window's own entry point (`reprice_prompt`) rather than by hand-building
/// a prompt — so the preview shows what a click on that cell shows, including
/// the quantity/price prefill rules that function owns.
///
/// The icon deliberately resolves against `ref_id = 0`: this preview has no
/// itemdata row behind it, and what the box does with an unknown ref id is
/// exactly the thing a picture can judge and a test cannot.
fn open_stock_box(
    state: Res<StallState>,
    mut modal: ResMut<StockModal>,
    mut armed: Local<bool>,
    windows: Query<Entity, With<StallWindowRoot>>,
) {
    // Armed once, and only after the stall window itself exists: opened in
    // `OnEnter` the box vanished without a trace, because `clear_stock_with_stall`
    // runs unordered against the frame in which the stall window is still being
    // spawned. A one-shot in `Update` is also what a click does, so the preview
    // follows the real order (window first, then the box over it).
    if *armed || windows.is_empty() {
        return;
    }
    modal.prompt = reprice_prompt(&state, 0, |_| String::new());
    *armed = true;
}

fn open_stall_window(mut state: ResMut<StallState>) {
    state.open = true;
    // Our own stall, not one we are visiting: the price/quantity box is the
    // owner path (`0x70BA`), and `stock::clear_stock_with_stall` closes the box
    // for a non-owner — which is exactly what a preview without this line got.
    state.owner = true;
    state.title = "Tobsnti's stall.".to_string();
    state.greeting = "Welcome to Tobsnti's stall.".to_string();
    state.trading = StallTradingState::Open;
    // Three of the ten cells filled, so the preview shows both an occupied and
    // an empty plate in both columns.
    for (index, (name, quantity, price)) in [
        (0usize, ("Steppe Blade", 1u16, 12_500u64)),
        (1, ("HP Potion (S)", 20, 400)),
        (4, ("Devil's Spirit", 3, 98_000)),
    ] {
        state.slots[index] = Some(StallRow {
            name: name.to_string(),
            // The preview has no itemdata behind it; ref id 0 renders from the name.
            ref_id: 0,
            quantity,
            price,
        });
    }
}
