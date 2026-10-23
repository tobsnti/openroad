pub mod model;
pub mod net;
pub mod owner;
pub mod stock;
pub mod ui;

use bevy::prelude::*;

use crate::scenes::SceneState;

/// Self-registration for the player stall window (#558 registry, #779).
pub struct StallPlugin;

impl Plugin for StallPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<model::StallState>()
            .init_resource::<owner::RequestedStallTitle>()
            .init_resource::<stock::StockModal>()
            .init_resource::<stock::StockAmount>()
            .init_resource::<stock::StockPrice>()
            .add_message::<owner::OpenStallCommand>()
            .add_systems(OnExit(SceneState::GameWorld), ui::cleanup_stall)
            .add_systems(PostUpdate, ui::despawn_closing_stall)
            .add_systems(
                Update,
                (
                    // server-driven throughout: the window opens on the
                    // server's enter broadcast and closes on the leave ack,
                    // never on a click (#780, docs/re/systems/stall.md §3)
                    owner::on_open_stall_command,
                    owner::on_stall_create_response,
                    owner::on_stall_destroy_response,
                    owner::on_stall_update_response,
                    owner::on_entity_stall_title_update,
                    owner::on_entity_stall_destroy,
                    net::on_stall_entity_action,
                    net::on_stall_buy_response,
                    net::on_stall_leave_response,
                    ui::spawn_stall_window.run_if(ui::open_pending_stall),
                    ui::refresh_stall_window,
                    // after the window, so a drop sees this frame's hover and
                    // the box is rebuilt in the same frame it is raised
                    stock::stock_drop_on_stall,
                    stock::clear_stock_with_stall,
                    stock::sync_stock_modal,
                    // after the box exists, so the first frame already has
                    // the prefilled amount in `StockAmount`
                    stock::sync_stock_fields,
                )
                    .chain()
                    .run_if(super::hud_scenes),
            );
    }
}
