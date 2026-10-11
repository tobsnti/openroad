//! Idea: an offline preview of the character-selection selection UI (info
//! box and Start/Delete/Cancel row) with dummy lobby data, so the pixel
//! layout can be iterated without logging into a live server. Run with
//! `SCENE=ui_testing`; combine with `OPENROAD_SCREENSHOT` for headless
//! capture.

use bevy::prelude::*;

use packets::agent::prelude::LobbyCharacter;

use crate::assets::FontAssets;
use crate::plugins::camera::{spawn_player_camera, DebugCamera, PlayerCamera};
use crate::plugins::textdata::{ClientCharacterData, ClientLevelData, ClientUiStrings};
use crate::scenes::intro_v2::assets::IntroV2Assets;
use crate::scenes::intro_v2::character_select::spawn_lobby_character;
use crate::scenes::intro_v2::{character_select, chrome, fade};
use crate::scenes::SceneState;

pub struct CharSelectUiPreviewPlugin;

impl Plugin for CharSelectUiPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(SceneState::UiTesting),
            (
                spawn_preview,
                spawn_player_camera,
                spawn_preview_figure,
                frame_figure,
            )
                .chain(),
        );
    }
}

fn dummy_character() -> LobbyCharacter {
    LobbyCharacter {
        ref_obj_id: 1907,
        name: "Testcharacter".to_string(),
        scale: 34,
        level: 42,
        exp_offset: 123_456,
        str: 87,
        int: 65,
        stat_points: 12,
        hp: 5230,
        mp: 4180,
        is_deleting: false,
        deletion_time: None,
        guild_member_class: 0,
        is_guild_rename_required: false,
        current_guild_name: None,
        academy_member_class: 0,
        char_items: vec![],
        avatar_items: vec![],
    }
}

fn spawn_preview(
    assets: Res<IntroV2Assets>,
    fonts: Res<FontAssets>,
    level_data: Res<ClientLevelData>,
    ui_strings: Res<ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut commands: Commands,
) {
    let Some(camera) = cam_query.iter().next() else {
        warn!("no 2d camera found");
        return;
    };

    // the intro chrome is included so occlusion/z-order problems between it
    // and the selection UI reproduce here too
    commands
        .spawn_scene(fade::fade_screen())
        .insert(UiTargetCamera(camera));
    commands
        .spawn_scene(chrome::header(&assets))
        .insert(UiTargetCamera(camera));
    commands
        .spawn_scene(chrome::footer(&assets))
        .insert(UiTargetCamera(camera));
    commands
        .spawn_scene(chrome::info_text())
        .insert(UiTargetCamera(camera));
    // `PREVIEW_DELETING=1` previews the deletion-pending variant, where the
    // original swaps Start/Delete for Restore + Cancel (#202). That state is
    // otherwise only reachable by actually deleting a character against a live
    // server, so the row would never get eyeballed offline.
    let deleting = std::env::var("PREVIEW_DELETING").is_ok();
    let mut character = dummy_character();
    character.is_deleting = deleting;

    commands
        .spawn_scene(character_select::info_box(
            &character,
            &level_data,
            &assets,
            &fonts,
        ))
        .insert(UiTargetCamera(camera));
    if deleting {
        commands
            .spawn_scene(character_select::deleting_control_buttons(
                &assets,
                &fonts,
                &ui_strings,
            ))
            .insert(UiTargetCamera(camera));
    } else {
        commands
            .spawn_scene(character_select::selected_control_buttons(&assets, &fonts))
            .insert(UiTargetCamera(camera));
    }
}

/// The figure itself, through the **same** call the live character list uses —
/// so a regression in that spawn shows up in the window gallery instead of
/// only in a session against a server.
fn spawn_preview_figure(
    asset_server: Res<AssetServer>,
    char_data: Res<ClientCharacterData>,
    mut commands: Commands,
) {
    let character = dummy_character();
    // 1907 is the first playable row in `characterdata_5000.txt`
    // (`char\china\chinaman_adventurer.bsr`); every shipped corpus has it.
    let path = char_data
        .get(&(character.ref_obj_id as i32))
        .and_then(|row| row.resource_path());
    if path.is_none() {
        warn!(
            "char-select preview: no characterdata row for ref {} — the figure will not render",
            character.ref_obj_id
        );
    }
    spawn_lobby_character(&mut commands, &asset_server, &character, path, Vec3::ZERO);
}

/// Frame the single figure the way the live scene frames a clicked one.
///
/// The eye sits on **+Z**: `spawn_lobby_character` turns the character by `PI`
/// about Y, so a camera on -Z photographs its back. The distance is the live
/// scene's own zoom (`CAMERA_ZOOM_DISTANCE`), pulled back by the figure's own
/// height so the whole character stands in the plate rather than its torso
/// filling it — the selection screen shows the figure, the zoom shows the face.
fn frame_figure(mut cameras: Query<&mut Transform, Or<(With<PlayerCamera>, With<DebugCamera>)>>) {
    let look_at = Vec3::new(0.0, 11.0, 0.0);
    let eye = Vec3::new(0.0, 13.0, 34.0);
    for mut transform in cameras.iter_mut() {
        *transform = Transform::from_translation(eye).looking_at(look_at, Vec3::Y);
    }
}
