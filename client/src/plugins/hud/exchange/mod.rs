pub mod model;
pub mod ui;

use bevy::prelude::*;

/// Self-registration for the player-exchange (trade) window (#558). The HUD registry holds one line per
/// window, so two windows landing in the same lap no longer collide on it.
pub struct ExchangePlugin;

impl Plugin for ExchangePlugin {
    fn build(&self, app: &mut App) {
        use crate::scenes::SceneState;

        app.init_resource::<model::ExchangeState>()
            .init_resource::<model::ExchangeCarry>()
            .init_resource::<ui::ExchangeGoldModal>()
            .init_resource::<ui::ExchangeGoldAmount>()
            .add_systems(OnExit(SceneState::GameWorld), ui::cleanup_exchange)
            .add_systems(PostUpdate, ui::despawn_closing_exchange)
            // server-driven throughout: it opens on 0x3085 and closes only on
            // an ack/cancel, never on a click
            .add_systems(
                Update,
                (
                    model::on_invite_response,
                    model::on_exchange_started,
                    model::on_partner_items,
                    model::on_partner_gold,
                    model::on_partner_confirmed,
                    model::on_confirm_response,
                    model::on_approve_response,
                    model::on_exchange_completed,
                    model::on_exchange_canceled,
                    model::on_exit_response,
                    ui::sync_exchange_window,
                )
                    .run_if(super::hud_scenes),
            )
            // staging our own side (0x7034 sub-ops 4/5/13): the drag polls and
            // the gold popup. Split into a second tuple so neither group
            // approaches Bevy's system-tuple arity.
            .add_systems(
                Update,
                (
                    model::on_staging_response,
                    ui::stage_drop_on_exchange,
                    ui::withdraw_drop_off_exchange,
                    ui::sync_exchange_gold_modal,
                    ui::sync_exchange_gold_amount,
                    ui::clear_exchange_extras,
                )
                    .run_if(super::hud_scenes),
            );
    }
}
