//! Idea: an offline preview of the character-selection selection UI (info
//! box and Start/Delete/Cancel row) with dummy lobby data, so the pixel
//! layout can be iterated without logging into a live server. Run with
//! `SCENE=ui_testing`; combine with `OPENROAD_SCREENSHOT` for headless
//! capture.

use bevy::prelude::*;

use packets::agent::prelude::LobbyCharacter;

use crate::assets::FontAssets;
use crate::plugins::textdata::{ClientLevelData, ClientUiStrings};
use crate::scenes::intro_v2::assets::IntroV2Assets;
use crate::scenes::intro_v2::{character_select, chrome, fade};
use crate::scenes::SceneState;

pub struct CharSelectUiPreviewPlugin;

impl Plugin for CharSelectUiPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(SceneState::UiTesting), spawn_preview)
            // The notice line's own pump is registered for `SceneState::IntroV2`
            // only (`intro_v2/mod.rs`), so inside this preview the message this
            // module writes would never reach the entity — the band would stay
            // empty and the multi-line behaviour could not be seen here. Same
            // system, not a second copy.
            .add_systems(
                Update,
                chrome::update_info_text.run_if(in_state(SceneState::UiTesting)),
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
    mut info_text: MessageWriter<chrome::InfoTextV2Update>,
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
        .spawn_scene(chrome::info_text(&fonts))
        .insert(UiTargetCamera(camera));
    // `PREVIEW_DELETING=1` previews the deletion-pending variant, where the
    // original swaps Start/Delete for Restore + Cancel. That state is
    // otherwise only reachable by actually deleting a character against a live
    // server, so the row would never get eyeballed offline.
    let deleting = std::env::var("PREVIEW_DELETING").is_ok();
    let mut character = dummy_character();
    character.is_deleting = deleting;
    if deleting {
        // A remainder inside the 7-day reservation, so `deletion_remaining`
        // reads it as minutes (its `> 10080` branch is the seconds reading):
        // 3 days 23 hours 59 minutes. Not a claim about the wire — but not
        // arbitrary either: two-digit hours *and* minutes make the countdown
        // sentence the **longest it can ever be** (days is a single digit for
        // the whole reservation), which is the case
        // `REMAIN_LINE2_FONT_PX` is sized against, so this preview is where
        // that fit gets eyeballed. Days at 3 keeps the gauge near the middle.
        character.deletion_time = Some(3 * 24 * 60 + 23 * 60 + 59);
    }

    commands
        .spawn_scene(character_select::info_box(
            &character,
            &level_data,
            &assets,
            &fonts,
            &ui_strings,
        ))
        .insert(UiTargetCamera(camera));
    if deleting {
        // The three-line notice is part of this state too — the original shows
        // it in the lower band (`UIO_STT_CHAR_DEL_WAITING`), and it is the one
        // message on this screen that is *not* one line, i.e. the only way to see
        // the notice line's multi-line behaviour here.
        info_text.write(chrome::InfoTextV2Update(
            ui_strings
                .get_plain_or(
                    "UIO_STT_CHAR_DEL_WAITING",
                    "The character's deletion is reserved.\nTo restore it, click 'Restore' \
                     button.\nTo keep the reservation, click 'Cancel' button.",
                )
                .replace("\\n", "\n"),
        ));
        // The countdown window belongs to this state as much as the Restore row
        // does: it states the deadline the row's choice is about. Without it the
        // preview shows half the screen the original shows.
        character_select::spawn_deletion_countdown(
            &mut commands,
            camera,
            &character,
            &assets,
            &fonts,
            &ui_strings,
        );
        commands
            .spawn_scene(character_select::deleting_control_buttons(
                &assets,
                &fonts,
                &ui_strings,
            ))
            .insert(UiTargetCamera(camera));
    } else {
        commands
            .spawn_scene(character_select::selected_control_buttons(
                &assets,
                &fonts,
                &ui_strings,
            ))
            .insert(UiTargetCamera(camera));
    }
}
