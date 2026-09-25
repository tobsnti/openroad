pub mod model;
pub mod ui;

use bevy::prelude::*;

/// Self-registration for the guild storage window + consumer (#558). The wire
/// half is the 0x7250 open / 0x7252 list / 0x7251 close family; the window is
/// `GDR_GUILDSTORAGEROOM`, which is the personal warehouse's own
/// `CIFStorageRoom` layout with another title and data source (see `ui.rs`).
pub struct GuildStoragePlugin;

impl Plugin for GuildStoragePlugin {
    fn build(&self, app: &mut App) {
        use crate::scenes::SceneState;

        app.init_resource::<model::GuildStorageState>()
            .init_resource::<model::GuildStorageDataBuffer>()
            .init_resource::<ui::GuildStorageCarry>()
            .init_resource::<crate::plugins::hud::item_cell::HoveredItem>()
            .add_systems(OnExit(SceneState::GameWorld), ui::cleanup_guild_storage)
            .add_systems(PostUpdate, ui::despawn_closing_guild_storage)
            .add_systems(
                Update,
                (
                    model::open_guild_storage,
                    model::on_guild_storage_response,
                    model::close_guild_storage_with_dialog,
                    model::on_guild_storage_operation,
                    // chained for the same reason the personal storage push
                    // is: begin/chunk/end usually land in ONE frame, and an
                    // unordered tuple lets `end` parse an empty buffer
                    (
                        model::on_guild_storage_begin,
                        model::on_guild_storage_chunk,
                        model::on_guild_storage_end,
                    )
                        .chain(),
                    ui::sync_guild_storage_window,
                    ui::track_guild_storage_hover,
                    // carry/drag half (ops 29/30/31), unordered like the
                    // personal warehouse's: the two are mutually exclusive by
                    // state (one needs an inventory drag, the other a guild
                    // carry, and the pickup observer refuses to start a carry
                    // while a drag is live), so no order is load-bearing.
                    ui::deposit_drop_on_guild_storage,
                    ui::finish_guild_storage_carry,
                    ui::update_guild_storage_ghost,
                    ui::clear_carry_with_guild_storage,
                )
                    .run_if(super::hud_scenes),
            );
    }
}
