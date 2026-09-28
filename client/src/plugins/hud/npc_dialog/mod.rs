pub mod job_menu;
pub mod model;
pub mod teleport;
pub mod ui;

use bevy::prelude::*;

/// Self-registration for the NPC dialog + teleport windows (#558). The HUD registry holds one line per
/// window, so two windows landing in the same lap no longer collide on it.
pub struct NpcDialogPlugin;

impl Plugin for NpcDialogPlugin {
    fn build(&self, app: &mut App) {
        use crate::scenes::SceneState;

        app.init_resource::<model::NpcDialogState>()
            .init_resource::<teleport::TeleportWindowState>()
            // The job join/leave answer arrives one frame after the click, so
            // the NPC and the action wait here — `ui::on_dialog_line` demands
            // it, which makes registering it a hard requirement (`hud/mod.rs`
            // pins that invariant).
            .init_resource::<job_menu::PendingJobRequest>()
            .add_message::<model::OpenStore>()
            .add_message::<model::OpenTeleport>()
            .add_message::<model::OpenStorage>()
            .add_message::<model::OpenGuildStorage>()
            .add_systems(
                OnExit(SceneState::GameWorld),
                (ui::cleanup_npc_dialog, teleport::cleanup_teleport),
            )
            .add_systems(PostUpdate, ui::despawn_closing_dialogs)
            .add_systems(
                Update,
                (
                    model::on_talk_started,
                    model::on_talk_response,
                    model::close_on_deselect,
                    model::close_on_walk_away,
                    ui::sync_dialog_window,
                    ui::hide_dialog_while_store_open,
                    ui::tint_dialog_lines,
                    ui::update_npc_dialog_scroll_thumb,
                    teleport::on_open_teleport,
                    teleport::close_teleport_with_dialog,
                    teleport::sync_teleport_window,
                    teleport::tint_teleport_lines,
                    job_menu::send_confirmed_job_request,
                )
                    .run_if(super::hud_scenes),
            );
    }
}
