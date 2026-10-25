pub mod model;
pub mod ui;

use bevy::prelude::*;

/// Self-registration for the guild warehouse (#558): the session state, the
/// wire round trip, and the window that shows what they fetch.
///
/// The window is `GDR_GUILDSTORAGEROOM` — the same `CIFStorageRoom` class,
/// geometry and art as the personal warehouse, so every interior rect comes
/// from `ifstorageroom.txt` (cited per constant in `ui.rs`) and none of it is
/// a fresh design.
///
/// Carry and drop send ops 29/30/31 and apply their acks; the money button
/// (ops 32/33) is still a later slice, so it is not spawned rather than
/// spawned inert.
pub struct GuildStoragePlugin;

impl Plugin for GuildStoragePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<model::GuildStorageState>()
            .init_resource::<model::GuildStorageDataBuffer>()
            .init_resource::<ui::GuildStorageCarry>()
            .add_systems(
                Update,
                (
                    model::open_guild_storage,
                    model::on_guild_storage_response,
                    model::close_guild_storage_with_dialog,
                    ui::sync_guild_storage_window,
                    model::on_guild_storage_operation,
                    ui::update_guild_storage_ghost,
                    ui::finish_guild_storage_carry,
                    ui::deposit_drop_on_guild_storage,
                    ui::clear_carry_with_guild_storage,
                    ui::track_guild_storage_hover,
                    // chained for the same reason the personal storage push
                    // is: begin/chunk/end usually land in ONE frame, and an
                    // unordered tuple lets `end` parse an empty buffer
                    (
                        model::on_guild_storage_begin,
                        model::on_guild_storage_chunk,
                        model::on_guild_storage_end,
                    )
                        .chain(),
                )
                    .run_if(super::hud_scenes),
            )
            // Same seam as the personal warehouse: leaving the world drops the
            // window and the mirror together.
            .add_systems(
                OnExit(crate::scenes::SceneState::GameWorld),
                ui::cleanup_guild_storage,
            )
            // PostUpdate like the sibling windows: despawning inside the
            // observer that closed the window tears down the entity being read.
            .add_systems(
                PostUpdate,
                ui::despawn_closing_guild_storage.run_if(super::hud_scenes),
            );
    }
}
