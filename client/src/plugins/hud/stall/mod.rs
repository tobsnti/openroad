pub mod model;
pub mod net;
pub mod owner;
pub mod stock;
pub mod ui;

use bevy::prelude::*;

use crate::scenes::SceneState;

/// Self-registration for the player stall window (#558 registry, #779).
///
/// **Who may open this window.** Two places in
/// the tree used to contradict each other: this header said "server-driven,
/// never on a click", while the action window's row 1009 (노점) flipped
/// `StallState.open` locally. Both were half right, because the window is one
/// shell for two roles:
///
/// * **Visitor half** — genuinely server-driven. It opens on `0xB0B3`/`0x30B7`
///   action 2 and closes on the `0xB0B5` ack; a local flip would show an empty
///   shell, since only the server knows the listing (`net.rs`).
/// * **Owner half** — *not* server-driven in the original. The 1009 arm
///   calls UI only: it sends no packet at all, where the 1006/1007 arms in the
///   same table send `0x7081`/`0x7060`. The original opens a local *composer*
///   — title edit box, item stocking — and only sends `0x70B1` when the owner
///   confirms.
///
/// We cannot reproduce that composer honestly: the title/price entry box it
/// needs is `MsgBoxStoreMoney`, whose binding to this window is unconfirmed,
/// and `0x70BA` types 1/2/3 (stocking) have no sourced dialog to drive them
/// either (`owner.rs` header). **Stated deviation:** every entry
/// point therefore sends `0x70B1` straight away under the vanilla default
/// title (`UIIT_STT_STALL_DEFAULT_TITLE`) and the window opens on the `0xB0B1`
/// ack — one path, `OpenStallCommand`, shared by `/Stall` (#781) and the
/// action window's 1009 (`hud/action.rs`). Reinstating the composer is what
/// gives 1009 its original behaviour back; until then a local toggle would
/// only show an empty shell.
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
                    // The visitor half is server-driven throughout: the window
                    // opens on the server's snapshot/enter broadcast and closes
                    // on the leave ack, never on a click (#780).
                    // The owner half opens on the
                    // 0xB0B1 ack of the one create command above (see the type
                    // comment for why that is the whole story).
                    owner::on_open_stall_command,
                    owner::on_stall_create_response,
                    owner::on_stall_destroy_response,
                    owner::on_stall_update_response,
                    owner::on_entity_stall_title_update,
                    owner::on_entity_stall_destroy,
                    net::on_stall_talk_response,
                    net::on_stall_entity_action,
                    net::on_stall_buy_response,
                    net::on_stall_leave_response,
                    ui::spawn_stall_window.run_if(ui::open_pending_stall),
                    ui::refresh_stall_window,
                )
                    .chain()
                    .run_if(super::hud_scenes),
            )
            // Stocking our own stall (0x70BA types 2/3). A second tuple, not
            // more entries in the one above: that one is `.chain()`ed for the
            // window's open/refresh order, which these four do not belong to,
            // and Bevy's system-tuple arity ends at 15.
            //
            // `run_if(super::hud_scenes)` is not decoration: `sync_stock_modal`
            // takes `Res<FontAssets>`, and an ungated HUD system with a scene
            // resource kills the schedule in the loading screen (hud/mod.rs:87,
            // the four-times-paid start trap).
            .add_systems(
                Update,
                (
                    stock::stock_drop_on_stall,
                    stock::sync_stock_modal,
                    stock::sync_stock_fields,
                    stock::clear_stock_with_stall,
                )
                    .run_if(super::hud_scenes),
            );
    }
}
