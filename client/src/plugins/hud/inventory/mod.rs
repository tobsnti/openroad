//! In-game inventory + equipment window (hotkey I): item grid with bag pages,
//! gold row, equipment slots around a live paper-doll, item tooltips, and
//! drag-and-drop moves confirmed by the server (0x7034/0xB034). Layout
//! hand-transcribed from resinfo/ifinventory.txt and resinfo/ifequipment.txt.

pub mod avatar;
pub mod drop_item;
pub mod model;
pub mod paperdoll;
pub mod split;
pub mod tooltip;
pub mod ui;
pub mod use_on_item;

use bevy::prelude::*;

/// Self-registration for the inventory window (#558). The HUD registry holds one line per
/// window, so two windows landing in the same lap no longer collide on it.
pub struct InventoryPlugin;

impl Plugin for InventoryPlugin {
    fn build(&self, app: &mut App) {
        use crate::scenes::SceneState;

        app.init_resource::<model::InventoryState>()
            .init_resource::<split::SplitPrompt>()
            .init_resource::<split::SplitAmount>()
            // the tooltip renders it, so the tooltip's plugin guarantees it
            // exists — every publishing window only writes to it
            .init_resource::<crate::plugins::hud::item_cell::HoveredItem>()
            .init_resource::<paperdoll::PaperDollYaw>()
            .init_resource::<use_on_item::PendingItemUse>()
            .init_resource::<use_on_item::UseOnItemConfirm>()
            .init_resource::<drop_item::DropConfirm>()
            .add_message::<drop_item::DroppedOnNothing>()
            .add_systems(OnEnter(SceneState::GameWorld), ui::spawn_inventory_window)
            .add_systems(OnExit(SceneState::GameWorld), ui::cleanup_inventory_window)
            // The inventory also runs in the World dev sandbox (its player has
            // no CharacterInfo, so the grid stays empty there, but the window,
            // paper-doll and hotkey are exercised).
            .add_systems(
                OnEnter(SceneState::WorldSandbox),
                ui::spawn_inventory_window,
            )
            .add_systems(
                OnExit(SceneState::WorldSandbox),
                ui::cleanup_inventory_window,
            )
            // reaps only entities that already carry the marker, so it needs
            // no scene gate of its own (same pattern as the store/storage
            // modals)
            .add_systems(PostUpdate, split::despawn_closing_split)
            .add_systems(
                Update,
                // Grouped only because Bevy implements the system-tuple traits
                // up to 20 elements and this window is past that; the groups
                // carry no ordering between them (the schedule was already
                // unordered here). A 21st entry in either goes into a new
                // group rather than growing one.
                (
                    (
                        model::toggle_inventory
                            .run_if(not(crate::plugins::settings::keymap::text_field_focused)),
                        use_on_item::cancel_armed_item,
                        use_on_item::sync_use_on_item_confirm,
                        model::on_gold_update,
                        model::on_inventory_operation_response,
                        model::on_inventory_deltas,
                        model::on_item_consumed,
                        model::on_entity_equip,
                        model::on_entity_unequip,
                        ui::apply_inventory_visibility,
                        ui::apply_avatar_view,
                        ui::update_drag_ghost,
                        ui::refresh_inventory.run_if(ui::inventory_needs_refresh),
                        ui::animate_slot_icon_effects,
                        tooltip::refresh_tooltip,
                        tooltip::position_tooltip,
                        paperdoll::maintain_paperdoll_clone,
                        paperdoll::tag_paperdoll_meshes,
                        paperdoll::aim_paperdoll_camera,
                        paperdoll::update_paperdoll_activity,
                    ),
                    // The drop-to-ground flow: detect, raise, draw, dismiss.
                    //
                    // `detect_drop_on_nothing` runs AFTER every window that can
                    // accept a carried item, because clearing `InventoryState
                    // .drag` is the only signal any of them gives that the drop
                    // was consumed. Without this order, depositing into storage
                    // would also ask to destroy the item.
                    (
                        drop_item::detect_drop_on_nothing
                            .after(crate::plugins::hud::storage::ui::deposit_drop_on_storage)
                            .after(crate::plugins::hud::store::ui::sell_drop_on_store)
                            .after(crate::plugins::hud::alchemy::ui::place_drop_on_alchemy)
                            .after(crate::plugins::hud::alchemy::grant::place_drop_on_grant)
                            .after(crate::plugins::hud::exchange::ui::stage_drop_on_exchange)
                            .after(crate::plugins::hud::stall::stock::stock_drop_on_stall),
                        drop_item::on_ground_drop_release.after(drop_item::detect_drop_on_nothing),
                        drop_item::sync_drop_confirm,
                        drop_item::cancel_drop_confirm,
                    ),
                    // Its own group: the outer tuple is at Bevy's 20-entry
                    // limit, and the drop-to-ground flow above is unrelated to
                    // either of these.
                    (
                        model::predict_ammo_consumption,
                        // the MsgBoxDivideCount stack-split prompt (#139b)
                        split::sync_split_modal,
                        split::sync_split_amount,
                        split::close_split_with_window,
                    ),
                )
                    .run_if(super::hud_scenes.or_else(in_state(SceneState::WorldSandbox))),
            );
    }
}
