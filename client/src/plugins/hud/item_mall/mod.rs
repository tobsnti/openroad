pub mod ui;

use bevy::prelude::*;

/// Self-registration for the Item Mall's My-Info pane (stage (b) of
/// `SILK-ANZEIGE.md`): the one surface the original shows the silk balance on.
pub struct ItemMallPlugin;

/// Whether the My-Info pane is open. Toggled by the underbar's mall button,
/// which until now only logged "not implemented yet".
#[derive(Resource, Default)]
pub struct ItemMallState {
    pub open: bool,
}

impl Plugin for ItemMallPlugin {
    fn build(&self, app: &mut App) {
        use crate::scenes::SceneState;

        app.init_resource::<ItemMallState>()
            .add_systems(OnEnter(SceneState::GameWorld), ui::spawn_my_info_window)
            .add_systems(OnExit(SceneState::GameWorld), ui::cleanup_my_info_window)
            .add_systems(
                Update,
                (ui::apply_my_info_visibility, ui::update_my_info_values).run_if(
                    in_state(SceneState::GameWorld).or_else(in_state(SceneState::UiTesting)),
                ),
            );
    }
}
