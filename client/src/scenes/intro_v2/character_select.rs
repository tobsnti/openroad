use std::f32::consts::PI;
use std::time::Duration;

use bevy::camera::primitives::Aabb;
use bevy::ecs::system::IntoObserverSystem;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::LineBreak;
use bevy::ui::{InteractionDisabled, Overflow, Pressed};
use bevy::ui_widgets::Activate;
use bevy_tweening::TweenAnim;

use packets::agent::prelude::*;
use packets::Packet;

use crate::assets::bmt::rim::SroRimMaterial;
use crate::assets::bmt::sheen::{ShineColor, SroSheenMaterial};
use crate::assets::textdata::leveldata::max_hp_or_mp;
use crate::assets::FontAssets;
use crate::net::connection::SilkroadConnection;
use crate::plugins::camera::CinematicCamera2;
use crate::plugins::dynamic_resource_loader::{
    AttachmentShine, MirroredResource, PendingItemAttachment, PreferredAnimationGroup,
    UnloadedResource,
};
use crate::plugins::hud::modal_dialog::{modal_plate, modal_scrim};
use crate::plugins::map::terrain::Terrain;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::settings::options::GameOptions;
use crate::plugins::textdata::{
    ClientCharacterData, ClientItemData, ClientLevelData, ClientUiStrings,
};
use crate::plugins::ui_v2::style::{ButtonSound, ImageButtonStyle};
use crate::plugins::ui_v2::widgets::{image_button, label};
use crate::plugins::world_origin::{set_world_origin, WorldOrigin};
use crate::scenes::SceneState;
use crate::util::mesh::needs_winding_reversal;
use crate::util::tweening_ext::keyframe_tween;

use super::assets::IntroV2Assets;
use super::chrome::InfoTextV2Update;
use super::fade::{FadeToBlack, FadeToBlackTimer};
use super::login_form::main_button_style;
use super::scene_data::ActiveCharSelectSceneV2;
use super::{
    fill_placeholders, intro_font_px, play_error_sound, unescape_newlines, IntroV2State, IntroV2Ui,
};
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::hud::world_anchor::project_world_anchor;

/// A text size scaled like a HUD rect and rounded to a whole pixel: a
/// fractional em rasterises off the pixel grid, which is the blurry half of
/// "font and spacing do not fit".
fn round_text_px(design_px: f32, scale: f32) -> f32 {
    (design_px * scale).round()
}

fn text_px(design_px: f32) -> f32 {
    round_text_px(design_px, hud_scale())
}

/// The intro font ladder entry for a resinfo `FontIndex`, at `hud_scale()`.
fn font_px(index: usize) -> f32 {
    text_px(intro_font_px(index))
}

/// How many characters an account may hold — the gate in front of the create
/// screen.
///
/// With four characters the original's `Create` click is inert and the lower
/// band prints "A maximum of 4 characters can be created."
/// (`UIO_MSG_ERROR_CHARACTER_OVER_3`, row 181 of `textdata/textuisystem.txt`,
/// verbatim including the `%d`). The number is a **server** policy in
/// principle — the string carries it as `%d` and `0xB007` has an error arm
/// `0x0405` for it — but nothing on the wire tells the client what the limit
/// is, so the original client hardcodes it and so do we, with the arm kept as
/// the backstop for a server that allows more or fewer.
const MAX_CHARACTER_SLOTS: u32 = 4;

/// Root marker of the Create/Cancel button row.
#[derive(Component, Default, Clone)]
pub struct CharSelectControls;

/// Root marker of the Start/Delete/Cancel button row shown while a character
/// is selected.
#[derive(Component, Default, Clone)]
pub struct SelectedCharControls;

/// Marker on the Start button, used to re-enable it when a join fails.
#[derive(Component, Default, Clone)]
pub struct StartButton;

/// Root marker of the character info box.
#[derive(Component, Default, Clone)]
pub struct CharSelectInfoBox;

/// Root marker of the delete confirmation modal.
#[derive(Component, Default, Clone)]
pub struct DeleteConfirmModal;

/// Root marker of the restore confirmation modal — the delete modal's twin for
/// a character whose deletion is still reserved.
#[derive(Component, Default, Clone)]
pub struct RestoreConfirmModal;

/// Root marker of the deletion countdown window (`GDR_STA_REMAINTIME`), shown
/// for a character whose deletion is reserved.
#[derive(Component, Default, Clone)]
pub struct DeletionCountdownWindow;

/// Root marker of the fullscreen loading overlay shown while joining the
/// world.
#[derive(Component, Default, Clone)]
pub struct JoinLoadingOverlay;

/// A 3d character preview spawned from the character list response.
#[derive(Component, Default, Copy, Clone)]
pub struct SelectableCharacterV2;

/// The lobby data of a character preview, kept on the root entity for the
/// info box and the delete/join requests.
#[derive(Component, Clone)]
pub struct CharacterInfoV2(pub LobbyCharacter);

/// The clicked character preview. Its presence drives the whole selection
/// UI (info box, Start/Delete/Cancel row) via run conditions, mirroring the
/// captcha's resource-presence pattern.
#[derive(Resource)]
pub struct SelectedCharacterV2(pub Entity);

/// The shared material a hover-highlighted mesh had before it was swapped
/// for a tinted clone; restored when the pointer leaves.
#[derive(Component)]
pub struct OriginalMaterial(pub Handle<StandardMaterial>);

/// The pre-hover sheen material of one mesh (weapons, metal armour).
#[derive(Component)]
pub struct OriginalSheenMaterial(pub Handle<SroSheenMaterial>);

/// The pre-hover rim material of one mesh — with `graphics.rim` enabled this
/// is *every* mesh of the character.
#[derive(Component)]
pub struct OriginalRimMaterial(pub Handle<SroRimMaterial>);

/// Set while a world join is in flight. Doubles as the signal for the
/// CharacterList exit systems to keep the agent connection alive, because the
/// world scene keeps talking to the same agent server.
#[derive(Resource)]
pub struct PendingWorldJoin {
    #[allow(dead_code)]
    pub character_name: String,
}

/// The full lobby data of the character being joined, carried across the scene
/// transition so the game scene can assemble the *actual* selected character
/// (model + equipment, named after it) instead of a hardcoded default. Consumed
/// and removed by the game scene once the player is spawned.
#[derive(Resource, Clone)]
pub struct JoiningCharacter(pub LobbyCharacter);

/// Camera flight time when zooming onto or away from a character.
const CAMERA_ZOOM_DURATION: Duration = Duration::from_millis(700);

// Zoom framing, in world units of an (unscaled) ~19-unit-tall character
// model: the camera stops this far in front of the clicked character at
// roughly head height, looking at the upper body — the vanilla close-up
// (torso + head filling the frame at fov 1.0).
const CAMERA_ZOOM_DISTANCE: f32 = 14.0;
const CAMERA_ZOOM_HEIGHT: f32 = 15.0;
const CHAR_FOCUS_HEIGHT: f32 = 14.0;

/// Additive emissive tint of the hover highlight.
///
/// **Not the original's.** The original shows no hover highlight on the lobby
/// figures at all — what it does on *click* is the camera dolly-in. We keep it
/// as a deliberate deviation (ADR-0009): the figures are the only click
/// targets on the screen and, unlike the original's, our pointer never gets a
/// cursor change or a name plate, so a faint additive tint is the only
/// affordance telling the player a figure is clickable. The value itself is
/// picked to be just visible on the dark harbour set; if the original's real
/// hover treatment ever turns up, this is replaced, not tuned.
const HOVER_EMISSIVE: LinearRgba = LinearRgba::rgb(0.15, 0.12, 0.05);

// Main button row placement — the FORMULAS the original computes, not fixed
// margins, with `W` = the back-buffer width:
//
//   x(CREATE | DELETE | RESTORE) = W - 0xD1        (= W-209)
//   x(BACK | BACK2 | CANCEL)     = W - 0xD1 + 0x68 (= W-105)
//   x(START)                     = W - 0xD1 - 0x68 (= W-313)
//   y(all seven)                 = barDown.top + 0x1B (= +27)
//
// All four `GDR_BTN_*` carry `Rect=RECT,"0,0,92,41"`
// (`resinfo/pscharacterselect_europe.txt:715,753,772,791,829`); a zero origin
// means code-placed at native texture size, so the row does **not** scale with
// the 1600x1200 authoring canvas — but its *anchor* does, because
// `GDR_STA_SCREENDOWN` is rect-bearing and canvas-scaled
// (`chrome::BAR_H_PCT`). Two corrections to what stood here:
//
//  * **Right margin 13, not 12.** From the formula the Cancel column ends at
//    `W-105+92 = W-13`.
//  * **The bottom margin is not a constant.** `y = barDown.top + 27` with a
//    band of `172/1200` of the window height gives a bottom margin of
//    `H·0.14333 - 27 - 41`: 18 px at 800x600, but **87 px at 1920x1080**. A
//    fixed 16 px therefore pushed the row out of the band on every larger
//    window. [`lower_band_anchor`] reproduces the original's anchor instead.
//
// The three values are deliberately *not* multiplied by `hud_scale()`: the
// buttons are drawn at the native `MAIN_BUTTON_W/H` (92x41, unscaled), and the
// gap only reproduces the pitch of 104 px while it lives in the same unit as
// the art. If the button art ever honours `hud_scale()`, these must scale too.
//
// Decision (stated, per ADR-0009): we keep the **authored** 92/12/13 from
// `Rect=RECT,"0,0,92,41"` and the placement formula, because pitch and total row
// width agree with how the original draws the row, and the remaining pixel is
// what a transparent DDJ border edge costs.
/// Right margin of the button row: `W - (W - 0xD1 + 0x68 + 92)`.
const BUTTON_ROW_RIGHT: f32 = 13.0;
/// `y = barDown.top + 0x1B`: the row starts 27 px *below* the lower band's top
/// edge, i.e. it is drawn on the band, not above it.
const BUTTON_ROW_TOP_IN_BAND: f32 = 27.0;
/// Gap between two buttons: pitch 104 (= `0x68`) − art width 92.
const BUTTON_ROW_GAP: f32 = 12.0;

/// The chrome bands' height as a share of the window — the anchor both
/// [`lower_band_anchor`] and [`upper_band_anchor`] hang on. Re-exported here
/// only so the `bsn!` node literals below name a local item instead of a path.
const BAND_H_PCT: f32 = super::chrome::BAR_H_PCT;

/// Info box right margin: `x = W - 0x10F` with a 228 px wide art → `271-228`
/// (as the original places it).
const INFO_BOX_RIGHT: f32 = 43.0;
/// Info box top: `y = barUp.bottom + 0x29`.
const INFO_BOX_TOP_BELOW_BAND: f32 = 41.0;

/// A zero-height, full-width root node whose bottom edge sits exactly on the
/// **top edge of the lower chrome band** — the `barDown.top` the original's
/// placement code reads. Children position themselves with plain `top:
/// px(...)` from there, which is the one thing a percent-plus-pixel margin
/// cannot express in flexbox: the band's height is a percentage of the window
/// (`chrome::BAR_H_PCT`, from the canvas-scaled `Rect="0,1030,1600,172"`),
/// while the offsets into it are native pixels.
fn lower_band_anchor() -> impl Scene {
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            bottom: percent(BAND_H_PCT),
            height: px(0),
        }
        // a zero-height strip across the whole screen must never eat clicks
        Pickable::IGNORE
    }
}

/// The mirror of [`lower_band_anchor`] for the upper band: its top edge sits on
/// `barUp.bottom`, the anchor the info box is placed from (`GDR_STA_CHARINFO`
/// at `y = barUp.bottom + 0x29`).
fn upper_band_anchor() -> impl Scene {
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: percent(BAND_H_PCT),
            height: px(0),
        }
        Pickable::IGNORE
    }
}

/// Caption size of the main buttons: the `FontIndex=2` of all four
/// `GDR_BTN_*` on this screen (`pscharacterselect_europe.txt:706,744,782,801`),
/// which the ladder resolves to **16 px** unscaled — the buttons are drawn at
/// their native 92x41, so their text is not scaled either
/// ([`super::intro_font_px`]).
///
/// **Correction of a previous correction.** This constant stood at 15.0, on the
/// argument that `FiraSans-Medium`'s OS/2 `sCapHeight` 691/1000 turns 15 px
/// into the ~10.4 px cap the original's captions show. That calibrates against
/// the *fallback* face: with the PK2 present the UI face is `영문서체.ttf`
/// (`assets/mod.rs`), cap height 740/1000, so 15 px already renders an 11.1 px
/// cap — the value the argument rejected for 16. The sourced
/// `FontIndex -> pt -> px` ladder (`event/event_interface.txt:2`) wins over a
/// cap-height proxy taken through the wrong face.
const BUTTON_LABEL_SIZE: f32 = intro_font_px(2);

pub fn control_buttons(
    assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
) -> impl Scene {
    let font = fonts.nine.clone();
    let cancel_font = fonts.nine.clone();
    let button_sound = assets.sound_button_sound_a.clone();
    let cancel_sound = button_sound.clone();
    // Captions come from the PK2, keyed exactly as the resinfo
    // buttons are: `GDR_BTN_CREATE.Text = UIO_COMMON_CTL_CREATE`
    // (`pscharacterselect_europe.txt:832`) and `GDR_BTN_CANCEL.Text =
    // UIO_COMMON_CTL_CANCEL` (`:718`), resolved in
    // `textdata/textuisystem.txt:159,160`. The English fallbacks are what
    // those two lines say, so a tree without PK2 text looks unchanged.
    let create_label = ui_strings
        .get_or("UIO_COMMON_CTL_CREATE", "Create")
        .to_string();
    let cancel_label = ui_strings
        .get_or("UIO_COMMON_CTL_CANCEL", "Cancel")
        .to_string();

    bsn! {
        lower_band_anchor()
        CharSelectControls
        Name("Character Select Controls V2")
        Children [
            (
                Name("Character Select Controls Row")
                Node {
                    position_type: PositionType::Absolute,
                    flex_direction: FlexDirection::Row,
                    top: px(BUTTON_ROW_TOP_IN_BAND),
                    right: px(BUTTON_ROW_RIGHT),
                    column_gap: px(BUTTON_ROW_GAP),
                }
                Children [
                    (
                        image_button(main_button_style(assets), MAIN_BUTTON_W, MAIN_BUTTON_H)
                        ImageNode { color: Color::NONE }
                        ButtonSound({button_sound})
                        Children [ (label(&create_label, font, BUTTON_LABEL_SIZE)) ]
                        on(|_activate: On<Activate>,
                            fade_timer: Option<Res<FadeToBlackTimer>>,
                            characters: Query<(), With<CharacterInfoV2>>,
                            ui_strings: Res<ClientUiStrings>,
                            mut info_text_writer: MessageWriter<InfoTextV2Update>,
                            mut next_state: ResMut<NextState<IntroV2State>>,
                            mut commands: Commands| {
                            // Ignore the click while the Cancel fade is in flight: the
                            // fade overlay doesn't block picking, and the queued
                            // LoginForm switch would then fire from inside
                            // CharacterCreate, skipping the CharacterList exit chain
                            // that restores camera/origin/connection.
                            if fade_timer.is_some() {
                                return;
                            }
                            // The slot limit is enforced *here*, before the screen
                            // switch, because that is where the original enforces it:
                            // with four characters in the account the click is inert,
                            // the lower band says "A maximum of 4 characters can be
                            // created.", and the frame delta holds nothing but `0x2002`
                            // keepalives — the create screen never opens. The
                            // button is *not* greyed out either, so this is a click-time
                            // check and not a disabled state.
                            if characters.iter().count() as u32 >= MAX_CHARACTER_SLOTS {
                                let template = ui_strings.get_plain_or(
                                    "UIO_MSG_ERROR_CHARACTER_OVER_3",
                                    "A maximum of %d characters can be created.",
                                );
                                info_text_writer.write(InfoTextV2Update(fill_placeholders(
                                    &template,
                                    &[MAX_CHARACTER_SLOTS],
                                )));
                                return;
                            }
                            // Keep the agent connection across the sub-state switch so
                            // Create/CheckName can use it (see disconnect_from_agent_server).
                            commands.insert_resource(super::character_create::EnteringCharacterCreate);
                            next_state.set(IntroV2State::RegionSelect);
                        })
                    ),
                    (
                        image_button(main_button_style(assets), MAIN_BUTTON_W, MAIN_BUTTON_H)
                        ImageNode { color: Color::NONE }
                        ButtonSound({cancel_sound})
                        Children [ (label(&cancel_label, cancel_font, BUTTON_LABEL_SIZE)) ]
                        on(|_activate: On<Activate>,
                            mut fade_writer: MessageWriter<FadeToBlack>,
                            mut commands: Commands| {
                            fade_writer.write(FadeToBlack);
                            commands.insert_resource(FadeToBlackTimer::to(IntroV2State::LoginForm));
                        })
                    ),
                ]
            ),
        ]
    }
}

/// The Start/Delete/Cancel row replacing [`control_buttons`] while a
/// character is selected. Spawned outside the `show_screen` fade flow, so
/// buttons and labels start fully visible.
pub(crate) fn selected_control_buttons(
    assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
) -> impl Scene {
    let start_font = fonts.nine.clone();
    let delete_font = fonts.nine.clone();
    let cancel_font = fonts.nine.clone();
    let start_sound = assets.sound_button_sound_a.clone();
    let delete_sound = start_sound.clone();
    let cancel_sound = start_sound.clone();
    // Same keying as the resinfo buttons: `GDR_BTN_START.Text =
    // UIO_SELCHAR_CTL_START` (`pscharacterselect_europe.txt:794`),
    // `GDR_BTN_DELETE.Text = UIO_SELCHAR_CTL_DELETE` (`:756`),
    // `GDR_BTN_CANCEL.Text = UIO_COMMON_CTL_CANCEL` (`:718`);
    // `textdata/textuisystem.txt:418,419,160`.
    let start_label = ui_strings
        .get_or("UIO_SELCHAR_CTL_START", "Start")
        .to_string();
    let delete_label = ui_strings
        .get_or("UIO_SELCHAR_CTL_DELETE", "Delete")
        .to_string();
    let cancel_label = ui_strings
        .get_or("UIO_COMMON_CTL_CANCEL", "Cancel")
        .to_string();

    bsn! {
        lower_band_anchor()
        SelectedCharControls
        Name("Selected Character Controls V2")
        Children [
            (
                Name("Selected Character Controls Row")
                Node {
                    position_type: PositionType::Absolute,
                    flex_direction: FlexDirection::Row,
                    top: px(BUTTON_ROW_TOP_IN_BAND),
                    right: px(BUTTON_ROW_RIGHT),
                    column_gap: px(BUTTON_ROW_GAP),
                }
                Children [
                    (
                        image_button(main_button_style(assets), MAIN_BUTTON_W, MAIN_BUTTON_H)
                        StartButton
                        ButtonSound({start_sound})
                        Children [ (label(&start_label, start_font, BUTTON_LABEL_SIZE) TextColor(Color::WHITE)) ]
                        on(on_start_activate)
                    ),
                    (
                        image_button(main_button_style(assets), MAIN_BUTTON_W, MAIN_BUTTON_H)
                        ButtonSound({delete_sound})
                        Children [ (label(&delete_label, delete_font, BUTTON_LABEL_SIZE) TextColor(Color::WHITE)) ]
                        on(on_delete_activate)
                    ),
                    (
                        image_button(main_button_style(assets), MAIN_BUTTON_W, MAIN_BUTTON_H)
                        ButtonSound({cancel_sound})
                        Children [ (label(&cancel_label, cancel_font, BUTTON_LABEL_SIZE) TextColor(Color::WHITE)) ]
                        on(on_selection_cancel_activate)
                    ),
                ]
            ),
        ]
    }
}

/// Control row for a character inside the deletion-pending window: the
/// original swaps Start/Delete for **Restore + Cancel**
/// (`UIO_CTL_CHARACTER_DELETE_CONDITION`, `resinfo/pscharacterselect.txt`), so
/// a pending character can only be recovered or deselected — not started, and
/// not deleted again.
pub(crate) fn deleting_control_buttons(
    assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
) -> impl Scene {
    let restore_font = fonts.nine.clone();
    let cancel_font = fonts.nine.clone();
    let restore_sound = assets.sound_button_sound_a.clone();
    let cancel_sound = restore_sound.clone();
    // Captions come from the PK2 (`UIO_STT_CHAR_RECOVERY` =
    // "Restore"), not from invented wording.
    let restore_label = ui_strings
        .get_or("UIO_STT_CHAR_RECOVERY", "Restore")
        .to_string();
    // `GDR_BTN_CANCEL.Text = UIO_COMMON_CTL_CANCEL`
    // (`pscharacterselect_europe.txt:718`, `textuisystem.txt:160`).
    let cancel_label = ui_strings
        .get_or("UIO_COMMON_CTL_CANCEL", "Cancel")
        .to_string();

    bsn! {
        lower_band_anchor()
        SelectedCharControls
        Name("Deleting Character Controls V2")
        Children [
            (
                Name("Deleting Character Controls Row")
                Node {
                    position_type: PositionType::Absolute,
                    flex_direction: FlexDirection::Row,
                    top: px(BUTTON_ROW_TOP_IN_BAND),
                    right: px(BUTTON_ROW_RIGHT),
                    column_gap: px(BUTTON_ROW_GAP),
                }
                Children [
                    (
                        // `GDR_BTN_RESTORE` is `Rect 0,0,92,41` like every other
                        // button on this screen (`pscharacterselect_europe.txt:772`);
                        // these two were the last 91 px leftovers.
                        image_button(main_button_style(assets), MAIN_BUTTON_W, MAIN_BUTTON_H)
                        ButtonSound({restore_sound})
                        Children [ (label(&restore_label, restore_font, BUTTON_LABEL_SIZE) TextColor(Color::WHITE)) ]
                        on(on_restore_activate)
                    ),
                    (
                        image_button(main_button_style(assets), MAIN_BUTTON_W, MAIN_BUTTON_H)
                        ButtonSound({cancel_sound})
                        Children [ (label(&cancel_label, cancel_font, BUTTON_LABEL_SIZE) TextColor(Color::WHITE)) ]
                        on(on_selection_cancel_activate)
                    ),
                ]
            ),
        ]
    }
}

fn spawn_default_controls(
    commands: &mut Commands,
    assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
    camera: Entity,
) {
    commands
        .spawn_scene(control_buttons(assets, fonts, ui_strings))
        .insert((UiTargetCamera(camera), IntroV2Ui));
}

pub fn spawn_control_buttons(
    mut commands: Commands,
    assets: Res<IntroV2Assets>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
) {
    let Some(camera) = cam_query.iter().next() else {
        warn!("no 2d camera found");
        return;
    };

    spawn_default_controls(&mut commands, &assets, &fonts, &ui_strings, camera);
}

pub fn request_character_list(mut query: Query<&SilkroadConnection, With<AgentConnection>>) {
    let Ok(conn) = query.single_mut() else {
        return;
    };
    send_character_list_request(conn);
}

fn send_character_list_request(conn: &SilkroadConnection) {
    let frame = Packet::from(CharacterSelectionActionRequest {
        action: CharacterSelectionAction::List,
        name: None,
        create: None,
    })
    .into();

    if let Err(e) = conn.get_sender().send(frame) {
        error!("failed to send frame: {}", e.0);
    }
}

/// A fresh entry into character selection never has a join in flight; a
/// stale [`PendingWorldJoin`] (e.g. from a previous session that never made
/// it into the world) would block the Start button forever.
pub fn reset_join_state(mut commands: Commands) {
    commands.remove_resource::<PendingWorldJoin>();
}

/// The mesh raycast backend runs with `require_markers`, so the char-select
/// camera must opt in to mesh picking explicitly.
pub fn enable_mesh_picking_camera(
    cam_query: Query<Entity, (With<CinematicCamera2>, Without<MeshPickingCamera>)>,
    mut commands: Commands,
) {
    for entity in cam_query.iter() {
        commands.entity(entity).insert(MeshPickingCamera);
    }
}

/// Anchor the floating world origin on the char-select location so the camera,
/// the lined-up characters and the surrounding terrain live at small
/// render-space coordinates (see `world_origin`). First system of the
/// `OnEnter(CharacterList)` chain — everything below places entities.
pub fn set_origin_to_char_select(
    char_select_scene: Res<ActiveCharSelectSceneV2>,
    mut origin: ResMut<WorldOrigin>,
    mut terrain: Query<&mut Transform, With<Terrain>>,
) {
    let anchor = char_select_scene.0.cam_base() * Vec3::new(-1.0, 1.0, 1.0);
    set_world_origin(anchor, &mut origin, &mut terrain);
}

pub fn start_camera_animation(
    mut cam_query: Query<(Entity, &mut Transform), With<CinematicCamera2>>,
    mut commands: Commands,
    char_select_scene: Res<ActiveCharSelectSceneV2>,
    origin: Res<WorldOrigin>,
    homed: Option<Res<super::race_stage::CharSelectCameraHomed>>,
) {
    let Ok((entity, mut transform)) = cam_query.single_mut() else {
        return;
    };

    // Coming back from the race board the camera has just *flown* to the
    // lineup's end pose (`race_stage::start_return_flight`).
    // Replaying the fly-in on top of that arrival would teleport back to the
    // start of the fly-in and repeat it, which is the jump the return flight
    // exists to remove — so the re-spawned camera is placed at the pose the
    // player is already looking through, and the marker is consumed.
    if homed.is_some() {
        commands.remove_resource::<super::race_stage::CharSelectCameraHomed>();
        if let Some((pos, rot)) = char_select_scene.0.init_camera_end_pose(origin.0) {
            transform.translation = pos;
            transform.rotation = rot;
            return;
        }
    }

    let base = (char_select_scene.0.cam_base() + char_select_scene.0.cam_offset())
        * Vec3::new(-1.0, 1.0, 1.0);

    transform.translation = origin.to_render(base);
    commands.entity(entity).insert(TweenAnim::new(
        char_select_scene.0.get_init_camera_anim(origin.0),
    ));
}

/// Which `0xB007` refusals *this* pump is the one to speak, and with which
/// sentence.
///
/// Idea: every `0xB007` answer passes through [`on_char_selection_action_response`]
/// while the list is on screen, but two of the actions already have their own
/// speaker — [`on_character_delete_response`] renders Delete and Restore from
/// the very same message. Reporting them here as well would put the identical
/// line up twice for one refusal (and the server is known to repeat a failure
/// frame, see `CharacterSelectionActionResponse`). Everything else that can
/// reach this state — a refused List above all — had no speaker at all: the
/// stage stayed empty and silent while only the log knew why. Create and
/// CheckName cannot arrive here, they are answered in the create screen's own
/// state; they are covered by the catch-all rather than named, because being
/// silent about an unexpected action is the defect being fixed.
fn selection_action_error_line(
    action: &CharacterSelectionAction,
    code: u16,
    ui_strings: &ClientUiStrings,
) -> Option<String> {
    match action {
        CharacterSelectionAction::Delete | CharacterSelectionAction::Restore => None,
        _ => super::lobby_error_line(code, ui_strings),
    }
}

/// Spawns a 3d preview per character from the list response. Port of the
/// old `on_char_selection_action_response`.
pub fn on_char_selection_action_response(
    char_select_scene: Res<ActiveCharSelectSceneV2>,
    char_data: Res<ClientCharacterData>,
    item_data: Res<ClientItemData>,
    asset_server: Res<AssetServer>,
    standing: Query<Entity, With<SelectableCharacterV2>>,
    mut reader: MessageReader<CharacterSelectionActionResponse>,
    mut commands: Commands,
    origin: Res<WorldOrigin>,
    ui_strings: Res<ClientUiStrings>,
    mut info_text_writer: MessageWriter<InfoTextV2Update>,
) {
    for res in reader.read() {
        // The lineup this response describes REPLACES whatever stands on the
        // stage. That used to be guaranteed by `OnExit(CharacterList)` wiping
        // the figures, but since the race board keeps the stage alive (the
        // figures must stay visible while the camera flies) the
        // returning list would otherwise spawn a second lineup into the first
        // one. Rebuilding from the response is idempotent; the earlier form was
        // only accidentally so.
        if res.characters.is_some() {
            for entity in standing.iter() {
                commands.entity(entity).despawn();
            }
        }
        if let Some(err) = res.error_code {
            // The log line stays: it is what made the swallowed refusal
            // visible in the first place. What was missing is the half the
            // player can see — the code is now resolved against the one lobby
            // error table (`packets::agent::lobby::lobby_error_text` via
            // [`super::lobby_error_line`]) and put on the same status line
            // every other pregame refusal uses, instead of only into the log.
            error!("[CharacterSelectionAction] Error : {}", err);
            if let Some(line) = selection_action_error_line(&res.action, err, &ui_strings) {
                info_text_writer.write(InfoTextV2Update(line));
            }
        }

        // Characters are lined up between the start and end offsets on the
        // X axis (see the old character_scene.rs for the reference values).
        let begin = char_select_scene.0.char_start_offset() * Vec3::new(-1.0, 1.0, 1.0);
        let end = char_select_scene.0.char_end_offset() * Vec3::new(-1.0, 1.0, 1.0);
        let dir = end - begin;

        let start =
            origin.to_render(char_select_scene.0.cam_base() * Vec3::new(-1.0, 1.0, 1.0)) + begin;
        if let Some(characters) = &res.characters {
            for (i, char) in characters.characters.iter().enumerate() {
                // Log the server-sent scale byte and the stats: how the byte
                // packs height and volume is still unknown, so the hex form
                // matters; the decimal is for quick reading.
                info!(
                    "lobby char '{}': ref {}, scale 0x{:02X} ({}), level {}, str {}, int {}, hp {}, mp {}",
                    char.name,
                    char.ref_obj_id,
                    char.scale,
                    char.scale,
                    char.level,
                    char.str,
                    char.int,
                    char.hp,
                    char.mp,
                );
                let ref_char_id = char.ref_obj_id as i32;
                let Some(char_data) = char_data.get(&ref_char_id) else {
                    warn!(
                        "lobby char '{}': no characterdata row for ref {ref_char_id} (missing/skipped shard?); not rendering this character",
                        char.name
                    );
                    continue;
                };
                let offset = (dir / (characters.count as f32 + 1.0)) * (i as f32 + 1.0);
                let path = char_data.resource_path();
                // Characters are mirrored on X (scale.x = -1) like every other
                // SRO resource so they render with the correct handedness (weapon
                // in the right hand). The mirror makes the placement determinant
                // negative, so the shared winding rule reverses their meshes.
                let scale = 1.0 + (char.scale as f32 / 255.0);
                let transform = Transform::from_translation(start + offset)
                    .with_scale(Vec3::new(-scale, scale, scale))
                    .with_rotation(Quat::from_rotation_y(PI));
                let mut char_commands = commands.spawn((
                    transform,
                    Visibility::default(),
                    SelectableCharacterV2,
                    Hovered::default(),
                    CharacterInfoV2(char.clone()),
                    Name::from(char.name.as_str()),
                ));
                if let Some(path) = path {
                    char_commands.insert(UnloadedResource(asset_server.load(path)));
                }
                if needs_winding_reversal(&transform.to_matrix()) {
                    char_commands.insert(MirroredResource);
                }
                // picking events on the (asynchronously spawned) leaf meshes
                // bubble up the hierarchy to this root
                char_commands.observe(on_character_clicked);
                let char_entity = char_commands.id();

                attach_equipment(
                    &mut commands,
                    &asset_server,
                    &item_data,
                    char_entity,
                    char.char_items
                        .iter()
                        .chain(char.avatar_items.iter())
                        .map(|item| (item.id, Some(item.plus))),
                );
            }
        }
    }
}

/// Attaches each equipped item's 3d resource under `char_entity`, mirroring the
/// vanilla equip layer: the weapon selects the character's animation stance,
/// items with no 3d resource are skipped, and a `+N` item carries its shine
/// tier. Shared by the char-list previews and the creation screen; `plus` is
/// `None` for freshly-picked starter gear (no enhancement yet).
pub(super) fn attach_equipment(
    commands: &mut Commands,
    asset_server: &AssetServer,
    item_data: &ClientItemData,
    char_entity: Entity,
    items: impl Iterator<Item = (u32, Option<u8>)>,
) {
    for (id, plus) in items {
        let Some(item_row) = item_data.get(&(id as i32)) else {
            warn!("no itemdata entry for equipped item: {}", id);
            continue;
        };
        // the equipped weapon decides which animation set the character plays
        // (e.g. the spear or bow stance)
        if let Some(group) = item_row.animation_group() {
            commands
                .entity(char_entity)
                .insert(PreferredAnimationGroup(group.to_string()));
        }
        // items without a 3d resource (e.g. pure stat items) cannot be rendered
        let Some(item_path) = item_row.resource_path() else {
            continue;
        };
        let mut attachment = commands.spawn((
            PendingItemAttachment(asset_server.load(item_path)),
            ChildOf(char_entity),
            Name::from(format!("item {}", item_row.code_name())),
        ));
        if let Some(color) = plus.and_then(ShineColor::for_opt_level) {
            attachment.insert(AttachmentShine::Tier(color));
        }
    }
}

/// The preview meshes spawn asynchronously once their resources load, so
/// each frame any untagged mesh below a character root is made pickable
/// (the raycast backend only tests `Pickable` entities).
pub fn tag_pickable_meshes(
    roots: Query<Entity, With<SelectableCharacterV2>>,
    children: Query<&Children>,
    untagged: Query<(), (With<Mesh3d>, Without<Pickable>)>,
    mut commands: Commands,
) {
    for root in roots.iter() {
        for entity in children.iter_descendants(root) {
            if untagged.contains(entity) {
                // RayCastBackfaces: the previews are mirrored (scale.x = -1)
                // with pre-reversed winding, so face orientation in mesh
                // space is unreliable for the raycast's backface culling —
                // accept hits from both sides.
                commands
                    .entity(entity)
                    .insert((Pickable::default(), RayCastBackfaces));
            }
        }
    }
}

/// Brightens a hovered character by swapping every mesh's material for an
/// emissive-tinted clone (and restoring the original on pointer-out). The
/// shared handles must never be mutated in place: they are loaded by asset
/// path, so a tint would bleed into every other user of the material.
/// Meshes that finish loading mid-hover stay untinted until the next hover
/// change, which is acceptable.
/// **All three material families, not just `StandardMaterial`.** A character's
/// body is a `StandardMaterial`, but its weapon and metal armour are
/// `SroSheenMaterial`, and with `graphics.rim` enabled — the default —
/// *everything* on the character is `SroRimMaterial`. Tinting only the first
/// one meant the highlight skipped the equipment, and with the rim on it
/// vanished entirely, so character select had no hover state at all. The
/// world's own highlight
/// (`cursor::interactions::entity_select`) has handled all three since it was
/// written; this is the same rule, applied here.
pub fn update_hover_highlight(
    hovered_roots: Query<(Entity, &Hovered), (With<SelectableCharacterV2>, Changed<Hovered>)>,
    children: Query<&Children>,
    mesh_materials: Query<&MeshMaterial3d<StandardMaterial>>,
    sheen_meshes: Query<&MeshMaterial3d<SroSheenMaterial>>,
    rim_meshes: Query<&MeshMaterial3d<SroRimMaterial>>,
    originals: Query<&OriginalMaterial>,
    sheen_originals: Query<&OriginalSheenMaterial>,
    rim_originals: Query<&OriginalRimMaterial>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut sheen_assets: ResMut<Assets<SroSheenMaterial>>,
    mut rim_assets: ResMut<Assets<SroRimMaterial>>,
    mut commands: Commands,
) {
    for (root, hovered) in hovered_roots.iter() {
        if hovered.0 {
            for entity in children.iter_descendants(root) {
                if originals.contains(entity)
                    || sheen_originals.contains(entity)
                    || rim_originals.contains(entity)
                {
                    continue;
                }
                if let Ok(material) = rim_meshes.get(entity) {
                    let Some(mut tinted) = rim_assets.get(&material.0).cloned() else {
                        continue;
                    };
                    tinted.base.emissive += HOVER_EMISSIVE;
                    let handle = rim_assets.add(tinted);
                    commands.entity(entity).insert((
                        MeshMaterial3d(handle),
                        OriginalRimMaterial(material.0.clone()),
                    ));
                    continue;
                }
                if let Ok(material) = sheen_meshes.get(entity) {
                    let Some(mut tinted) = sheen_assets.get(&material.0).cloned() else {
                        continue;
                    };
                    tinted.base.emissive += HOVER_EMISSIVE;
                    let handle = sheen_assets.add(tinted);
                    commands.entity(entity).insert((
                        MeshMaterial3d(handle),
                        OriginalSheenMaterial(material.0.clone()),
                    ));
                    continue;
                }
                let Ok(material) = mesh_materials.get(entity) else {
                    continue;
                };
                let Some(mut tinted) = materials.get(&material.0).cloned() else {
                    continue;
                };
                tinted.emissive += HOVER_EMISSIVE;
                let tinted_handle = materials.add(tinted);
                commands.entity(entity).insert((
                    MeshMaterial3d(tinted_handle),
                    OriginalMaterial(material.0.clone()),
                ));
            }
        } else {
            restore_materials(
                root,
                &children,
                &originals,
                &sheen_originals,
                &rim_originals,
                &mut commands,
            );
        }
    }
}

fn restore_materials(
    root: Entity,
    children: &Query<&Children>,
    originals: &Query<&OriginalMaterial>,
    sheen_originals: &Query<&OriginalSheenMaterial>,
    rim_originals: &Query<&OriginalRimMaterial>,
    commands: &mut Commands,
) {
    for entity in children.iter_descendants(root) {
        if let Ok(original) = originals.get(entity) {
            commands
                .entity(entity)
                .insert(MeshMaterial3d(original.0.clone()))
                .remove::<OriginalMaterial>();
        }
        if let Ok(original) = sheen_originals.get(entity) {
            commands
                .entity(entity)
                .insert(MeshMaterial3d(original.0.clone()))
                .remove::<OriginalSheenMaterial>();
        }
        if let Ok(original) = rim_originals.get(entity) {
            commands
                .entity(entity)
                .insert(MeshMaterial3d(original.0.clone()))
                .remove::<OriginalRimMaterial>();
        }
    }
}

/// Selecting a character freezes the hover state (the pointer sits still, so
/// no `Changed<Hovered>` fires), so any active highlight is cleared here.
pub fn clear_highlights(
    tinted: Query<(Entity, &OriginalMaterial)>,
    sheen_tinted: Query<(Entity, &OriginalSheenMaterial)>,
    rim_tinted: Query<(Entity, &OriginalRimMaterial)>,
    mut commands: Commands,
) {
    for (entity, original) in tinted.iter() {
        commands
            .entity(entity)
            .insert(MeshMaterial3d(original.0.clone()))
            .remove::<OriginalMaterial>();
    }
    // The other two families are cleared the same way — a highlight left on a
    // weapon or on a rim-lit body would otherwise survive the selection.
    for (entity, original) in sheen_tinted.iter() {
        commands
            .entity(entity)
            .insert(MeshMaterial3d(original.0.clone()))
            .remove::<OriginalSheenMaterial>();
    }
    for (entity, original) in rim_tinted.iter() {
        commands
            .entity(entity)
            .insert(MeshMaterial3d(original.0.clone()))
            .remove::<OriginalRimMaterial>();
    }
}

/// `Pointer<Click>` observer attached to each character root: marks the
/// character selected and flies the camera onto it.
pub fn on_character_clicked(
    click: On<Pointer<Click>>,
    selected: Option<Res<SelectedCharacterV2>>,
    transforms: Query<&Transform, With<SelectableCharacterV2>>,
    cam_query: Query<
        (Entity, &Transform),
        (With<CinematicCamera2>, Without<SelectableCharacterV2>),
    >,
    mut commands: Commands,
) {
    if selected.is_some() || click.event.button != PointerButton::Primary {
        return;
    }

    // the event bubbled up from a leaf mesh; `entity` is the propagation
    // target the observer sits on, i.e. the character root
    let root = click.entity;
    let Ok(char_transform) = transforms.get(root) else {
        return;
    };
    let Ok((cam_entity, cam_transform)) = cam_query.single() else {
        return;
    };

    // Like the vanilla client, the zoom targets the clicked character
    // itself, not an authored scene pose (the YAML select keyframes are
    // untuned and frame nothing): the camera stops in front of the
    // character — approaching from wherever the camera currently is — at
    // head height, looking at the upper body.
    let char_pos = char_transform.translation;
    let char_scale = char_transform.scale.y;
    let mut toward_cam = cam_transform.translation - char_pos;
    toward_cam.y = 0.0;
    let toward_cam = toward_cam.normalize_or(Vec3::Z);

    let target_pos =
        char_pos + toward_cam * CAMERA_ZOOM_DISTANCE + Vec3::Y * (CAMERA_ZOOM_HEIGHT * char_scale);
    let focus = char_pos + Vec3::Y * (CHAR_FOCUS_HEIGHT * char_scale);
    let target_rot = Transform::from_translation(target_pos)
        .looking_at(focus, Vec3::Y)
        .rotation;

    commands
        .entity(cam_entity)
        .insert(TweenAnim::new(keyframe_tween(
            cam_transform.translation,
            target_pos,
            cam_transform.rotation,
            target_rot,
            CAMERA_ZOOM_DURATION,
        )));
    commands.insert_resource(SelectedCharacterV2(root));
}

fn zoom_out_camera(
    cam_query: &Query<(Entity, &Transform), With<CinematicCamera2>>,
    char_select_scene: &ActiveCharSelectSceneV2,
    origin: &WorldOrigin,
    commands: &mut Commands,
) {
    let Ok((cam_entity, cam_transform)) = cam_query.single() else {
        return;
    };
    // Flying to the init animation's final keyframe (instead of remembering
    // the pre-zoom transform) keeps the target deterministic even when the
    // character was clicked mid-animation.
    let Some((end_pos, end_rot)) = char_select_scene.0.init_camera_end_pose(origin.0) else {
        return;
    };

    commands
        .entity(cam_entity)
        .insert(TweenAnim::new(keyframe_tween(
            cam_transform.translation,
            end_pos,
            cam_transform.rotation,
            end_rot,
            CAMERA_ZOOM_DURATION,
        )));
}

// Info box layout: transplanted from the vanilla UI definition
// Media.pk2/resinfo/pscharacterselect.txt (Section "Info"). Its
// GDR_STA_CHARINFO window is info.ddj at the native 228x140, and every child
// element carries a Rect=RECT,"x,y,w,h" entry in window space — those
// coordinates are used verbatim below, uniformly scaled by hud_scale().

/// Fill fraction of a `current / max` gauge pair, clamped to `[0, 1]`.
///
/// `max == 0` means the maximum could not be derived (an unknown level/stat
/// combination), and a full bar is the honest fallback there: the lobby never
/// sends a maximum, so a 0-width bar would be a claim we cannot back.
fn gauge_fill(current: u32, max: u32) -> f32 {
    if max == 0 {
        return 1.0;
    }
    (current as f32 / max as f32).clamp(0.0, 1.0)
}

pub(crate) fn info_box(
    info: &LobbyCharacter,
    level_data: &ClientLevelData,
    assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
) -> impl Scene {
    let window = assets.info_window.clone();
    let hp_image = assets.hp_bar.clone();
    let mp_image = assets.mp_bar.clone();
    let name_font = fonts.nine.clone();
    let level_cap_font = fonts.nine.clone();
    let level_font = fonts.nine.clone();
    let exp_cap_font = fonts.nine.clone();
    let exp_font = fonts.nine.clone();
    let sp_cap_font = fonts.nine.clone();
    let sp_font = fonts.nine.clone();

    let name = info.name.clone();
    let level_text = format!("{}", info.level);
    // exp is shown as the percentage of the current level's requirement
    // (leveldata.txt column 1), like the original client
    let exp_text = match level_data.max_exp(info.level) {
        Some(max) if max > 0 => {
            format!("{:.2}%", info.exp_offset as f64 / max as f64 * 100.0)
        }
        _ => "-".to_string(),
    };
    let sp_text = format!("{}", info.stat_points);
    // The three captions are authored `Text=` keys of `GDR_STATIC1/2/3`
    // (`pscharacterselect_europe.txt:185,147,109` -> `UIO_CHARINFO_STT_EXP`,
    // `_SP`, `_LEVEL`, `textuisystem.txt:422-424`). The English column reads
    // "EXP" / "Stat" / "Level", so nothing changes visually — but a Korean or
    // German PK2 now gets its own words instead of ours.
    let exp_caption = ui_strings.get_or("UIO_CHARINFO_STT_EXP", "EXP").to_string();
    let sp_caption = ui_strings.get_or("UIO_CHARINFO_STT_SP", "Stat").to_string();
    let level_caption = ui_strings
        .get_or("UIO_CHARINFO_STT_LEVEL", "Level")
        .to_string();

    // The lobby packet carries current HP/MP but no maximum, so the bars used
    // to be pinned full. The maximum is reproducible offline from the same
    // `1.02^(level-1) * stat * 10` curve the server uses, and the lobby's
    // `str`/`int` are the base primaries with no equip or buff modifiers,
    // which is exactly what that curve wants.
    //
    // APPROX: only the `1.02^(lvl-1)` shape of the curve is certain, the rest
    // is community-derived, and whether v1.188's char-select ever draws a
    // partial bar at all is still open. If it turns out the original always
    // shows a full bar, delete the two fractions, not the formula.
    let hp_fill = gauge_fill(info.hp, max_hp_or_mp(info.level, info.str));
    let mp_fill = gauge_fill(info.mp, max_hp_or_mp(info.level, info.int));

    // gold of the vanilla caption texts (FontColor "255,255,208,81", ARGB)
    let caption_color = Color::srgb_u8(255, 208, 81);

    let s = hud_scale();
    // Every text-bearing control in this box is `FontIndex=2` -> 16 px:
    // `GDR_STA_NAME` (`pscharacterselect_europe.txt:242`), the three captions
    // `GDR_STATIC1/2/3` (`:185`, `:147`, `:109`) and the three values
    // `GDR_STA_EXP`/`_SP`/`_LEVEL` (`:166`, `:128`, `:90`). This box is the one
    // intro surface that is built at `hud_scale()` (see the rects below), so its
    // text carries the same factor — via `font_px`, which also rounds the result
    // to a whole pixel. The `11.0 * s` that stood here was a guess ("11px fits
    // the 13px-high cells"), and it was wrong twice over: 16.5 px is a
    // fractional em, and it is a third short of the ladder, so the labels filled
    // ~0.55 of a box the original fills ~0.85 of.
    let font_size = font_px(2);
    let row_h = 13.0 * s;

    let window_w = 228.0 * s;
    let window_h = 140.0 * s;
    // Screen placement of the box: `GDR_STA_CHARINFO` is
    // `Rect=RECT,"0,0,228,140"` (`resinfo/pscharacterselect_europe.txt:677`) —
    // a zero origin, so the control is code-placed at native size and does NOT
    // scale with the 1600x1200 canvas. The original places it at
    // `x = W - 0x10F` (= W-271 → right margin 271-228 = 43) and
    // `y = barUp.bottom + 0x29` (= +41).
    //
    // On an 800x600 frame those two formulas produce the origin (529, 128)
    // (800-271 = 529; 86+41 = 127) — the y is not a
    // constant: the upper band is rect-bearing and therefore canvas-scaled, so
    // at 1920x1080 the box belongs at y=196, not at y=128.
    // [`upper_band_anchor`] carries that anchor.
    //
    // Deviation with a reason (ADR-0009): both offsets carry the same
    // `hud_scale()` factor as the box itself, because the box is deliberately
    // drawn at `228x140 * s` rather than at native size (see
    // `plugins::hud::scale`). Scaling the art but not its margins would slide
    // the box off its relationship to the screen edge.
    let window_right = INFO_BOX_RIGHT * s;
    let window_top = INFO_BOX_TOP_BELOW_BAND * s;
    // GDR_STA_NAME "32,29,177,13"
    let name_l = 32.0 * s;
    let name_t = 29.0 * s;
    let name_w = 177.0 * s;
    // GDR_GAU_HP "46,50,136,8" / GDR_GAU_MP "46,62,136,8"
    let bar_l = 46.0 * s;
    let bar_w = 136.0 * s;
    let bar_h = 8.0 * s;
    let hp_t = 50.0 * s;
    let mp_t = 62.0 * s;
    // EXP row "38,80,28,13" + "76,80,43,13", SP "126,80,20,13" + "155,80,49,13".
    //
    // **The Stat pair is moved right on purpose (ADR 0009), and here is the
    // arithmetic:**
    //
    // * the authored value cell is 43 px wide and its content is centred
    //   (`HAlign=1`), but the string the original itself prints there — a
    //   `{:.2}%` percentage such as `50.40%` — is **56.7 px** wide at the
    //   ladder's 16 px on `영문서체.ttf` (Arial and `기본서체` land within 2 px
    //   of that), and the worst case `100.00%` is **66.2 px**. Centred in
    //   76..119 that is ink 64..131.
    // * the original therefore collides with itself: it prints
    //   `EXP 50.40%Stat  0` with **no gap at all** between the `%` and the `S`.
    //   That is how the original behaves, not a defect of ours.
    // * what *was* ours is that we had made it worse: the caption sat at 122
    //   instead of the authored 126, widened leftwards into the number.
    //
    // So: keep the value where the data puts it and give the caption the room
    // the number needs — caption cell 136..165, value cell 168..208, both still
    // inside the plate's inner area (the name cell ends at 209). Worst-case ink
    // 131 vs caption ink 137: ~6 px of air where the original has none. This is
    // a deliberate deviation from the original, stated rather than silent.
    let stats_t = 80.0 * s;
    let exp_cap_l = 38.0 * s;
    let exp_cap_w = 28.0 * s;
    let exp_val_l = 76.0 * s;
    let exp_val_w = 43.0 * s;
    let sp_cap_l = 136.0 * s;
    let sp_cap_w = 29.0 * s;
    let sp_val_l = 168.0 * s;
    let sp_val_w = 40.0 * s;
    // LEVEL row "79,102,36,13" + "126,102,25,13"
    let level_t = 102.0 * s;
    let level_cap_l = 79.0 * s;
    let level_cap_w = 36.0 * s;
    let level_val_l = 126.0 * s;
    let level_val_w = 25.0 * s;

    bsn! {
        upper_band_anchor()
        CharSelectInfoBox
        Name("Character Info Box V2")
        Children [
            (
                Name("Character Info Box Frame")
                ImageNode { image: {window}, image_mode: NodeImageMode::Stretch }
                Node {
                    position_type: PositionType::Absolute,
                    right: px(window_right),
                    top: px(window_top),
                    width: px(window_w),
                    height: px(window_h),
                }
                Children [
                    (
                        // the character name goes into the window's sunken slot
                        label(&name, name_font, font_size)
                        TextColor(Color::WHITE)
                        Node { position_type: PositionType::Absolute, left: px(name_l), top: px(name_t), width: px(name_w), height: px(row_h) }
                    ),
                    // HP/MP gauges. A vanilla `CIFGauge` with `Style=0` CROPS its art
                    // along X at 1:1 texel scale, it does not stretch it to the fill
                    // width, so the percentage
                    // goes on a clip wrapper and the image keeps its full authored
                    // width. Same three-node recipe as `hud/character_info/ui.rs`,
                    // which is the one site in the tree that already had it right.
                    // Here the two models happen to render identically — `hp.ddj` and
                    // `mp.ddj` are 136x8 with no X structure — so this is for
                    // consistency, not for the pixels.
                    (
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(bar_l),
                            top: px(hp_t),
                            width: px(bar_w),
                            height: px(bar_h),
                            overflow: {Overflow::clip()},
                        }
                        Children [
                            (
                                Node {
                                    width: percent(hp_fill * 100.0),
                                    height: percent(100),
                                    overflow: {Overflow::clip()},
                                }
                                Pickable::IGNORE
                                Children [
                                    (
                                        ImageNode { image: {hp_image}, image_mode: NodeImageMode::Stretch }
                                        Node {
                                            position_type: PositionType::Absolute,
                                            left: px(0.0),
                                            top: px(0.0),
                                            width: px(bar_w),
                                            height: px(bar_h),
                                        }
                                        Pickable::IGNORE
                                    ),
                                ]
                            ),
                        ]
                    ),
                    (
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(bar_l),
                            top: px(mp_t),
                            width: px(bar_w),
                            height: px(bar_h),
                            overflow: {Overflow::clip()},
                        }
                        Children [
                            (
                                Node {
                                    width: percent(mp_fill * 100.0),
                                    height: percent(100),
                                    overflow: {Overflow::clip()},
                                }
                                Pickable::IGNORE
                                Children [
                                    (
                                        ImageNode { image: {mp_image}, image_mode: NodeImageMode::Stretch }
                                        Node {
                                            position_type: PositionType::Absolute,
                                            left: px(0.0),
                                            top: px(0.0),
                                            width: px(bar_w),
                                            height: px(bar_h),
                                        }
                                        Pickable::IGNORE
                                    ),
                                ]
                            ),
                        ]
                    ),
                    // EXP and SP share one row, gold caption + white value each
                    (
                        label(&exp_caption, exp_cap_font, font_size)
                        TextColor({caption_color})
                        Node { position_type: PositionType::Absolute, left: px(exp_cap_l), top: px(stats_t), width: px(exp_cap_w), height: px(row_h) }
                    ),
                    (
                        label(&exp_text, exp_font, font_size)
                        TextColor(Color::WHITE)
                        Node { position_type: PositionType::Absolute, left: px(exp_val_l), top: px(stats_t), width: px(exp_val_w), height: px(row_h) }
                    ),
                    (
                        label(&sp_caption, sp_cap_font, font_size)
                        TextColor({caption_color})
                        Node { position_type: PositionType::Absolute, left: px(sp_cap_l), top: px(stats_t), width: px(sp_cap_w), height: px(row_h) }
                    ),
                    (
                        label(&sp_text, sp_font, font_size)
                        TextColor(Color::WHITE)
                        Node { position_type: PositionType::Absolute, left: px(sp_val_l), top: px(stats_t), width: px(sp_val_w), height: px(row_h) }
                    ),
                    // LEVEL sits centered in the row below
                    (
                        label(&level_caption, level_cap_font, font_size)
                        TextColor({caption_color})
                        Node { position_type: PositionType::Absolute, left: px(level_cap_l), top: px(level_t), width: px(level_cap_w), height: px(row_h) }
                    ),
                    (
                        label(&level_text, level_font, font_size)
                        TextColor(Color::WHITE)
                        Node { position_type: PositionType::Absolute, left: px(level_val_l), top: px(level_t), width: px(level_val_w), height: px(row_h) }
                    ),
                ]
            ),
        ]
    }
}

// ---- Per-figure name plate (`GDR_STA_PLAYER1/2/3`) ------------------------
//
// Idea, and the honest label on it: **the original does not draw this.** It
// shows the title art, both bands, the buttons and the info box, and **no name
// over either figure**, and nothing inside its char-select module ever touches
// the three controls' ids 6/7/8 — in v1.188 EU they are dead code. The older
// "the name floats above the figure" claim is wrong.
//
// So this is a **deliberate improvement under ADR 0009** (the hovered
// character's name belongs on screen), not a reproduction — and
// it is built by *reviving the authored control* rather than inventing a
// position: `GDR_STA_PLAYER1/2/3` (`pscharacterselect_europe.txt:934/915/896`)
// are `Rect="0,0,200,40"`, `FontIndex=0` and white, i.e. code-placed 200x40
// plates with no art of their own. Everything below except the *anchor* comes
// from those three lines; the anchor is the figure's own head point, because a
// code-placed control has no authored screen position at all.
// The alternative considered and rejected: a free position at the top of the
// screen. That would be an invented number where this is authored geometry.
//
// Scaled by `hud_scale()` like this screen's other code-placed surfaces (the
// info box and the countdown window), so the three do not disagree with each
// other.

/// The one plate node; it follows whichever figure is hovered, or the selected
/// one when nothing is hovered.
#[derive(Component, Default, Clone)]
pub struct CharacterNamePlate;

/// `GDR_STA_PLAYER1/2/3` `Rect="0,0,200,40"` (`:934`, `:915`, `:896`).
const NAME_PLATE_W: f32 = 200.0;
const NAME_PLATE_H: f32 = 40.0;
/// Head anchor while the figure's meshes are still streaming in: the world
/// nameplates' `HEAD_OFFSET` (`plugins/hud/nameplates.rs`).
///
/// It is only the fallback: the lobby bodies are **18.1 units tall** —
/// `chinaman_adventurer.bsr`'s own bbox is `(-9.13, -0.005, -1.97) ..
/// (9.13, 18.12, 1.20)` (JMXVRES bbox block) — so a 23-unit anchor sits ~5
/// units *above* the head, and the lineup camera stands only ~45 units away
/// (`constantinople.selection`), where one unit is ~39 px at 1440p. That is
/// ~210 px of empty air plus the plate's own height, which lands the plate in
/// the top chrome band instead of over the figure, where it belongs. The world
/// nameplates get away with the same number because the
/// world camera is much farther away — which is why one shared *absolute*
/// offset was the defect, and the figure's own extent is the fix.
const NAME_PLATE_HEAD_OFFSET: f32 = 23.0;

/// Projects the hovered (else selected) figure's head to the viewport and puts
/// its name there; hides the plate when nothing is hovered or selected.
///
/// One pooled node, spawned on first use: a plate per figure would be three
/// nodes of which at most one is ever visible.
pub fn update_character_name_plate(
    camera3d: Query<(&Camera, &GlobalTransform), With<CinematicCamera2>>,
    ui_camera: Query<Entity, With<Camera2d>>,
    figures: Query<(Entity, &Hovered, &Transform, &CharacterInfoV2), With<SelectableCharacterV2>>,
    // The figure's own extent decides where its head is. Same
    // shape as `plugins/cos/riding.rs::resolve_saddle_seats`, which reads a
    // mount's top the same way — meshes (and their Aabbs) stream in
    // asynchronously, so the caller falls back until they exist.
    children: Query<&Children>,
    aabbs: Query<&Aabb>,
    selection: Option<Res<SelectedCharacterV2>>,
    fonts: Res<FontAssets>,
    mut plate: Query<(Entity, &mut Node, &mut Visibility), With<CharacterNamePlate>>,
    mut plate_text: Query<&mut Text, With<CharacterNamePlate>>,
    mut commands: Commands,
) {
    let Some(camera) = ui_camera.iter().next() else {
        return;
    };
    let s = hud_scale();
    if plate.is_empty() {
        commands.spawn((
            CharacterNamePlate,
            IntroV2Ui,
            UiTargetCamera(camera),
            Name::new("Character Name Plate V2"),
            Text::new(String::new()),
            TextFont {
                font: fonts.nine.clone().into(),
                // `FontIndex=0` on all three plates -> ladder 12 px, scaled
                font_size: FontSize::Px(font_px(0)),
                ..default()
            },
            // `FontColor="255,255,255,255"` on all three (`:934`/`:915`/`:896`)
            TextColor(Color::WHITE),
            TextLayout::justify(Justify::Center),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(NAME_PLATE_W * s),
                height: Val::Px(NAME_PLATE_H * s),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(4),
            Visibility::Hidden,
            Pickable::IGNORE,
        ));
        return;
    }
    let Ok((_entity, mut node, mut visibility)) = plate.single_mut() else {
        return;
    };
    let Ok(mut text) = plate_text.single_mut() else {
        return;
    };

    // Hover wins over selection: while the pointer is on a figure the plate
    // answers "who is this one".
    let hovered = figures
        .iter()
        .find(|(_, hovered, _, _)| hovered.0)
        .map(|(entity, _, transform, info)| (entity, transform, info));
    let target = hovered.or_else(|| {
        let selected = selection.as_ref()?.0;
        figures
            .get(selected)
            .ok()
            .map(|(entity, _, transform, info)| (entity, transform, info))
    });
    let Some((entity, transform, info)) = target else {
        *visibility = Visibility::Hidden;
        return;
    };
    let Some((camera3d, cam_gt)) = camera3d.iter().find(|(c, _)| c.is_active) else {
        *visibility = Visibility::Hidden;
        return;
    };
    // Model scale is negative on X (the SRO mirror), so the *Y* scale is the
    // one that says how tall this figure is. The height itself comes from the
    // figure's own meshes — a borrowed absolute offset put the plate in the
    // chrome band — and falls back to the world nameplates' number only while
    // the `.bsr` is still streaming.
    let local_top = figure_top(entity, &children, &aabbs).unwrap_or(NAME_PLATE_HEAD_OFFSET);
    let head = transform.translation + Vec3::Y * local_top * transform.scale.y.abs();
    let Some(px) = project_world_anchor(camera3d, cam_gt, head) else {
        *visibility = Visibility::Hidden;
        return;
    };

    if text.0 != info.0.name {
        text.0 = info.0.name.clone();
    }
    // The projected point is the head; the 200x40 plate is centred on it
    // horizontally and sits with its bottom edge there.
    node.left = Val::Px(px.x - NAME_PLATE_W * s / 2.0);
    node.top = Val::Px(px.y - NAME_PLATE_H * s);
    if *visibility != Visibility::Visible {
        *visibility = Visibility::Visible;
    }
}

/// Highest point of a figure's loaded meshes, in the figure's own local space
/// (i.e. before its `Transform` scale) — `None` until at least one mesh of the
/// asynchronously streamed `.bsr` exists.
///
/// Same read as `plugins/cos/riding.rs::resolve_saddle_seats` does for a
/// mount's saddle: the union of the descendants' `Aabb` tops. Reason to prefer
/// it over a constant: `chinaman_adventurer.bsr` is 18.12 units tall
/// and `chinawoman_*` differs, so any single number is wrong for some body —
/// and at the lineup camera's ~45 units even 5 units of error is ~200 px.
fn figure_top(entity: Entity, children: &Query<&Children>, aabbs: &Query<&Aabb>) -> Option<f32> {
    let mut top: Option<f32> = None;
    for child in children.iter_descendants(entity) {
        if let Ok(aabb) = aabbs.get(child) {
            let child_top = aabb.center.y + aabb.half_extents.y;
            top = Some(top.map_or(child_top, |t: f32| t.max(child_top)));
        }
    }
    top
}

// ---- Deletion countdown (`GDR_STA_REMAINTIME`, resinfo section `Remain`) ----
//
// Idea: a delete-ordered character is not gone, it is *reserved for 7 days*, and
// the original says how much of that reservation is left in a small 328x92
// window above the lower band. Without it the Restore/Cancel row we already had
// states a choice whose deadline is invisible — the mechanic the delete copy
// (`UIO_STT_CHAR_DEL_CONFIRM`) promises was dead-wire here even though
// `is_deleting`/`deletion_time` have been parsed for months
// (`packets/src/agent/lobby.rs:102-103`).
//
// Geometry comes from the data: `GDR_STA_REMAINTIME` (id 26) at
// `x = (W-328)/2`, `y = barDown.top - 0x66`, with
// `GDR_STA_WREMINTIME1 0,14,328,12`, `GDR_PML_WREMINTIME2 0,36,328,12` and
// `GDR_REMAINGBOX`/`GDR_REMAING 24,63,280,8`; the strings are keys 265+266.
//
// Art: the three ddjs the data names live in `interface/outer/` —
// `delete_time_window.ddj` (`GDR_STA_REMAINTIME`, resinfo `:691`),
// `delete_time_gauge.ddj` (`GDR_REMAING`, `:1094`) and
// `delete_time_gauge_up.ddj` (`GDR_REMAINGBOX`, `:1075`) — and are loaded in
// `IntroV2Assets`. The flat-rect substitute gauge and the plate-less window that
// stood here were placeholders for exactly these three handles.
//
// One open decision remains: both gauge controls share the rect, so the arts
// are layered and only their *order* is a judgement call. `_up` is read as the
// upper layer (its name, and a fill that is hidden behind its frame would make
// the `CIFGauge` pointless), so the fill is drawn first and `_up` over it.
/// `GDR_STA_REMAINTIME` `Rect="0,0,328,92"` (resinfo id 687).
const REMAIN_W: f32 = 328.0;
const REMAIN_H: f32 = 92.0;
/// `y = base - 0x66` in the original: the window's **top** edge sits
/// 102 px above the lower band's top edge, so with its own 92 px height its
/// bottom edge clears the band by 10 px — which is how it is expressed here,
/// because the anchor node's edge *is* `barDown.top`.
const REMAIN_TOP_ABOVE_BAND: f32 = 102.0;
/// `GDR_STA_WREMINTIME1` `0,14,328,12` / `GDR_PML_WREMINTIME2` `0,36,328,12`.
const REMAIN_LINE1_TOP: f32 = 14.0;
const REMAIN_LINE2_TOP: f32 = 36.0;
const REMAIN_LINE_H: f32 = 12.0;
/// `GDR_REMAINGBOX` (frame) and `GDR_REMAING` (fill) share `24,63,280,8`.
const REMAIN_GAUGE: (f32, f32, f32, f32) = (24.0, 63.0, 280.0, 8.0);
/// The gold of `GDR_STA_WREMINTIME1` and of the countdown's own
/// `<font color="255,255,208,81">` (resinfo COLOR is ARGB -> RGB 255,208,81),
/// the same gold the info-box captions use.
const REMAIN_GOLD: Color = Color::srgb_u8(255, 208, 81);
/// Backdrop behind the gauge's fill art: the neutral dark the HUD uses for an
/// empty bar. **Ours, not the data's** — the resinfo names no empty-track art,
/// and `delete_time_gauge_up.ddj` is drawn over the fill, so without a backdrop
/// the consumed part of the bar would show the plate through it. Rationale
/// rather than origin (ADR-0009).
const REMAIN_GAUGE_TRACK: Color = Color::srgba(0.0, 0.0, 0.0, 0.6);
/// The reservation the delete warning states in words: **7 days**
/// (`UIO_STT_CHAR_DEL_CONFIRM`). Used as the countdown gauge's full
/// scale, because the wire sends only the remainder.
const DELETE_RESERVATION_MINUTES: u32 = 7 * 24 * 60;

// --- Making the countdown line fit its control ------------------------------
//
// The idea: `GDR_PML_WREMINTIME2` is 328 px wide and its sentence is roughly
// half again that long, so *something* has to give. The original's own string
// says which: it is wrapped in `<sml2>`, a PML **size class**, so the original
// shrinks this line rather than drawing it at the control's `FontIndex`. We
// drop that tag (`resolve_pml`), so the size is fixed here instead — once, for
// the longest sentence the line can ever hold.

/// The longest the countdown line can ever be, in **em widths of whatever face
/// draws it**.
///
/// # Why an em width and not a pixel count
///
/// Four faces can draw this line at runtime — the machine's Arial
/// (`config.fonts.system_ui_face`, the face the original's own
/// `textuisystem.txt:1` names), the PK2's `영문서체` and `기본서체`, and the
/// bundled Fira fallback — and a fit that holds for one of them is not a fit.
/// Advance sums scale linearly with the em, so one number in em covers all
/// sizes, and the *widest* face fixes the bound.
///
/// # The bound
///
/// Summed `hmtx` advances / `unitsPerEm` over the longest string the line can
/// produce, for both templates (the PK2's `UIO_MSG_CHAR_DEL_TIME` and the
/// English fallback below) and all four faces. The worst case is the
/// fallback string in `기본서체`:
///
/// | face | PK2 string | fallback string |
/// |---|---|---|
/// | Arial (system) | 34.19 em | 35.68 em |
/// | FiraSans-Medium (bundled) | 35.25 em | 36.98 em |
/// | `영문서체` (Arial Rounded MT Bold) | 37.48 em | 39.33 em |
/// | `기본서체` (TaeUtum) | 37.74 em | **40.20 em** |
///
/// "Longest possible" is bounded, not guessed: the template carries three
/// `%d` and no `%s` — no character name — and `deletion_remaining` clamps to
/// [`DELETE_RESERVATION_MINUTES`], so days is one digit (0..=7), hours and
/// minutes two: `6days 23hours 59minuites`.
///
/// `cfg(test)` with its neighbour below: the bound is a property of the
/// *template*, so it is checked once at build time rather than per frame.
#[cfg(test)]
const REMAIN_LINE2_LONGEST_EMS: f32 = 40.21;

/// Design-space em size of the countdown line: **8 px, where the control's own
/// `FontIndex` 0 is 12** (`font_px(0)`).
///
/// **The original shrinks this line too, and its own string says so.**
/// `UIO_MSG_CHAR_DEL_TIME` is wrapped in `<sml2>`, which is a PML *size class*,
/// not a colour or a paragraph tag: the original's tag classifier lists
/// `sml2` next to `layer`/`table`/`strong`/`font`/`left`/`center`/`right`.
/// `resolve_pml` drops that tag (see its doc), so drawing this
/// line at `FontIndex` 0 is *our* omission, not the original's design.
///
/// So the **shrink is the original's**; only the **number is ours**: the PML
/// size table the class resolves to is unknown, so 8 is fitted against the
/// control instead — `countdown_line_fits` is that check, and 8 is the largest
/// whole design pixel that fits at every `hud_scale` the tests below check. One
/// fixed size rather than a per-frame fitting loop: the bound above is a
/// property of the *template*, so it can be settled once at build time.
///
/// Fixing this also un-crowds the row vertically: `REMAIN_LINE_H` is 12 px, so
/// `FontIndex` 0 asked for an em exactly as tall as the clipped row.
const REMAIN_LINE2_FONT_PX: f32 = 8.0;

/// Does the longest possible countdown line still fit `GDR_PML_WREMINTIME2` at
/// `design_px`, once `scale` has been applied to both?
///
/// Takes the scale rather than reading [`hud_scale`] so the tests can walk
/// several scales without writing the process global. The rounding matters and
/// is therefore the *same* rounding the renderer does ([`round_text_px`], which
/// `text_px` is): at `hud_scale` 1.5 a design 9 px becomes 14 device px, not
/// 13.5, and those half pixels are what decides the fit.
#[cfg(test)]
fn countdown_line_fits(design_px: f32, scale: f32) -> bool {
    round_text_px(design_px, scale) * REMAIN_LINE2_LONGEST_EMS <= REMAIN_W * scale
}

/// The countdown line's em size in device pixels — the one place both the
/// spawn path and `tick_deletion_countdown` read it from, so a re-render
/// cannot disagree with the spawn about the size.
fn countdown_font_px() -> f32 {
    text_px(REMAIN_LINE2_FONT_PX)
}

/// Live state of the countdown line (`GDR_PML_WREMINTIME2`).
///
/// Idea: the wire sends the remainder **once**, in `0xB007`, and never again
/// while the lobby is open. So the only honest way to keep the sentence true is
/// to count the client's own elapsed time down from that value — the same thing
/// the original does (its window ticks while you sit in the lobby). The line is
/// re-rendered only when the displayed *minute* changes, because that is the
/// finest unit `UIO_MSG_CHAR_DEL_TIME` prints; a per-frame rebuild of the
/// coloured runs would be work nobody can see.
#[derive(Component)]
pub struct DeletionCountdown {
    /// Minutes of the reservation still left, ticked by `Time` (fractional, so
    /// the accumulated seconds are not lost between whole minutes).
    minutes_left: f32,
    /// The PML template of `UIO_MSG_CHAR_DEL_TIME`, kept so a re-render needs
    /// no second string lookup.
    template: String,
    /// The whole-minute value the runs currently spell out.
    shown: u32,
}

/// The percentage-wide wrapper that clips the gauge's fill art.
#[derive(Component)]
pub struct DeletionCountdownGaugeFill;

/// Share of the 7-day reservation already **elapsed** — the gauge's width.
///
/// Direction follows the original: while its window reads `7days 0hour`
/// (nothing elapsed yet) the bar is **empty**, and a minute later at
/// `6days 23hour` it still is. So the window is a progress bar of the deletion
/// ("Character deletion is being processed …"), not a remaining-time bar; a
/// remaining-time reading would have shown it full in both states. That is also
/// the direction the art argues for: the 7-band ramp runs gold -> red left to
/// right, so a growing left-anchored crop reaches the red band exactly as the
/// reservation expires.
///
/// The wire sends no total, so the reservation the delete copy states in words
/// is the scale (see `DELETE_RESERVATION_MINUTES`).
fn gauge_elapsed(minutes_left: f32) -> f32 {
    let left = (minutes_left / DELETE_RESERVATION_MINUTES as f32).clamp(0.0, 1.0);
    1.0 - left
}

/// Re-renders the countdown line and grows its gauge as time passes.
///
/// Runs while the lobby is up; does nothing at all until a whole displayed
/// minute has elapsed.
pub fn tick_deletion_countdown(
    mut commands: Commands,
    time: Res<Time>,
    fonts: Res<FontAssets>,
    mut lines: Query<(Entity, &mut DeletionCountdown)>,
    mut fills: Query<&mut Node, With<DeletionCountdownGaugeFill>>,
) {
    let elapsed_minutes = time.delta_secs() / 60.0;
    for (entity, mut state) in &mut lines {
        state.minutes_left = (state.minutes_left - elapsed_minutes).max(0.0);
        let whole = state.minutes_left as u32;
        if whole == state.shown {
            continue;
        }
        state.shown = whole;

        let (days, hours, minutes) = (whole / (24 * 60), (whole / 60) % 24, whole % 60);
        let text = fill_placeholders(&state.template, &[days, hours, minutes]);
        let runs = resolve_pml(&text, Color::WHITE);
        // Same size the spawn path uses: the fitted size through
        // `countdown_font_px`, i.e. scaled by `hud_scale` like the window
        // around it. A re-render at the unscaled `intro_font_px` — or at the
        // control's own `font_px(0)` — would change the line's size the first
        // time a minute ticked over.
        let line_font = countdown_font_px();
        let font = fonts.nine.clone();
        commands
            .entity(entity)
            .despawn_related::<Children>()
            .with_children(|line| {
                for (run, color) in &runs {
                    line.spawn((
                        Text::new(run.clone()),
                        TextFont {
                            font: font.clone().into(),
                            font_size: FontSize::Px(line_font),
                            ..default()
                        },
                        // same clip-don't-wrap rule as the spawn path (B5)
                        TextLayout {
                            linebreak: LineBreak::NoWrap,
                            ..default()
                        },
                        TextColor(*color),
                        Pickable::IGNORE,
                    ));
                }
            });

        let fill = gauge_elapsed(state.minutes_left);
        for mut node in &mut fills {
            node.width = Val::Percent(fill * 100.0);
        }
    }
}

/// Splits `deletion_time` into days / hours / minutes.
///
/// The unit is **minutes**: a freshly deleted character carries `0x2760` =
/// 10080 = 7*24*60, i.e. the 7-day reservation the delete copy states in words.
/// An earlier "seconds" reading was wrong.
///
/// The clamp to the reservation stays, but as a *display* guard, not as a unit
/// heuristic: another server may run a different (longer) policy, and the gauge
/// below has only the 7-day scale to draw on, so a longer remainder reads as
/// "the full bar" instead of overflowing it.
fn deletion_remaining(deletion_time: u32) -> (u32, u32, u32) {
    let minutes = deletion_time.min(DELETE_RESERVATION_MINUTES);
    (minutes / (24 * 60), (minutes / 60) % 24, minutes % 60)
}

/// Resolves the PML subset the lobby strings use into coloured runs.
///
/// `UIO_MSG_CHAR_DEL_TIME` is a **markup** string, not plain text
/// (`GDR_PML_WREMINTIME2` is a `CIFPML`): `<sml2>Complete deletion … in <font
/// color="255,255,208,81">%d</font>days …`. A plain-text renderer prints the
/// tags, which is exactly what we
/// would have done here. Two tags matter: `<font color="a,r,g,b">…</font>`,
/// which becomes a run colour (ARGB, alpha ignored — the original's own colours
/// are all `255,…`), and `<sml2>`, a *size class* we cannot honour without the
/// PML size table; it is dropped, and the line already renders at the control's
/// `FontIndex`. Unknown tags are dropped rather than printed, because a stray
/// `<br>` is closer to "nothing" than to literal text.
fn resolve_pml(markup: &str, default: Color) -> Vec<(String, Color)> {
    let mut runs: Vec<(String, Color)> = Vec::new();
    let mut colors: Vec<Color> = Vec::new();
    let mut text = String::new();
    let mut rest = markup;

    let flush = |text: &mut String, runs: &mut Vec<(String, Color)>, color: Color| {
        if !text.is_empty() {
            runs.push((std::mem::take(text), color));
        }
    };

    while let Some(open) = rest.find('<') {
        let Some(close) = rest[open..].find('>') else {
            break;
        };
        let tag = &rest[open + 1..open + close];
        text.push_str(&rest[..open]);
        rest = &rest[open + close + 1..];

        let lower = tag.trim().to_ascii_lowercase();
        if lower.starts_with("font") {
            flush(&mut text, &mut runs, *colors.last().unwrap_or(&default));
            colors.push(parse_pml_color(tag).unwrap_or(default));
        } else if lower.starts_with("/font") {
            flush(&mut text, &mut runs, *colors.last().unwrap_or(&default));
            colors.pop();
        }
    }
    text.push_str(rest);
    flush(&mut text, &mut runs, *colors.last().unwrap_or(&default));
    runs
}

/// `color="a,r,g,b"` out of a PML `font` tag; ARGB like every resinfo COLOR.
fn parse_pml_color(tag: &str) -> Option<Color> {
    let value = tag.split("color").nth(1)?;
    let value = value.trim_start().strip_prefix('=')?.trim();
    let value = value.trim_matches('"').trim_matches('\'');
    let mut parts = value.split(',').map(|p| p.trim().parse::<u8>());
    let _a = parts.next()?.ok()?;
    let r = parts.next()?.ok()?;
    let g = parts.next()?.ok()?;
    let b = parts.next()?.ok()?;
    Some(Color::srgb_u8(r, g, b))
}

/// Spawns the deletion countdown window for a pending character.
///
/// Built imperatively rather than as a `bsn!` scene because the countdown line
/// is a *variable* number of coloured runs (whatever the PML resolves to), which
/// a static `Children [...]` list cannot express.
pub(crate) fn spawn_deletion_countdown(
    commands: &mut Commands,
    camera: Entity,
    info: &LobbyCharacter,
    assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
) {
    let (days, hours, minutes) = deletion_remaining(info.deletion_time.unwrap_or(0));
    // Line 1: `UIO_MSG_CHAR_DEL_WND` (textuisystem.txt:265), gold, plain text.
    let processing = ui_strings
        .get_or(
            "UIO_MSG_CHAR_DEL_WND",
            "Character deletion is being processed.",
        )
        .to_string();
    // Line 2: `UIO_MSG_CHAR_DEL_TIME` (:266), PML. The fallback is the English
    // of the sibling key 156 `UIO_CTL_CHARACTER_DELETE` — a real client string
    // with the same three `%d`, so a tree without PK2 text still says the truth.
    let countdown_template = ui_strings
        .get_or(
            "UIO_MSG_CHAR_DEL_TIME",
            "[%d]day(s) [%d]hour(s) [%d]minute(s) is left until the complete deletion of Character.",
        )
        .to_string();
    let countdown = fill_placeholders(&countdown_template, &[days, hours, minutes]);
    let runs = resolve_pml(&countdown, Color::WHITE);
    // Elapsed share of the 7-day reservation; the wire sends no total, and the
    // original's window is empty while the whole reservation is still left.
    let minutes_left = (days * 24 * 60 + hours * 60 + minutes) as f32;
    let fill = gauge_elapsed(minutes_left);
    // **Scaled like its neighbour, on purpose (ADR 0009)** — drawn native the
    // overlay is small and partly unreadable.
    //
    // This window is a *code-placed* control (`GDR_STA_REMAINTIME`
    // `Rect="0,0,328,92"`), and the original draws that class at **native art
    // size, 1:1, unscaled** (330 px wide on an 800x600 client).
    // We already deviate from that for the *other* code-placed surface of this
    // very screen: `info_box` draws its 228x140 plate at `hud_scale()` (1.5 by
    // default) with `font_px` text, because a native-size plate is postage-stamp
    // sized on a 1600x900 window. Drawing this window native while the box
    // beside it is 1.5x is what makes it look "oddly scaled" — the two are the
    // same class of control and were treated differently.
    //
    // So both offsets and both font sizes now carry the same factor as
    // `info_box`. `font_px` (rather than `intro_font_px`) is the same call the
    // box uses: ladder x `hud_scale`, rounded to a whole pixel.
    let s = hud_scale();
    // Line 1 draws at the control's own `FontIndex` (19.81 em in the widest
    // face = 238 px, so it fits 328 px with room to spare); only
    // the countdown line needs the fitted size.
    let line_font = font_px(0);
    let countdown_font = countdown_font_px();
    let plate_art = assets.delete_time_window.clone();
    let gauge_art = assets.delete_time_gauge.clone();
    let gauge_frame_art = assets.delete_time_gauge_up.clone();

    let mut window = commands.spawn((
        DeletionCountdownWindow,
        IntroV2Ui,
        UiTargetCamera(camera),
        Name::new("Deletion Countdown Window V2"),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            bottom: Val::Percent(BAND_H_PCT),
            height: Val::Px(0.0),
            ..default()
        },
        Pickable::IGNORE,
    ));
    window.with_children(|anchor| {
        let mut plate = anchor.spawn((
            Name::new("Deletion Countdown Plate"),
            // `GDR_STA_REMAINTIME` names `interface/outer/delete_time_window.ddj`
            // (`resinfo/pscharacterselect_europe.txt:691`); the art is stretched
            // to the control's own `328x92`, the same way every other plate in
            // this scene is drawn.
            ImageNode {
                image: plate_art,
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Node {
                position_type: PositionType::Absolute,
                // `x = (W - 328) / 2`: centred, expressed as "half the screen
                // minus half the window" because the anchor spans the width.
                left: Val::Percent(50.0),
                margin: UiRect::left(Val::Px(-REMAIN_W * s / 2.0)),
                bottom: Val::Px((REMAIN_TOP_ABOVE_BAND - REMAIN_H) * s),
                width: Val::Px(REMAIN_W * s),
                height: Val::Px(REMAIN_H * s),
                ..default()
            },
            Pickable::IGNORE,
        ));
        plate.with_children(|p| {
            p.spawn((
                Text::new(processing),
                TextFont {
                    font: fonts.nine.clone().into(),
                    font_size: FontSize::Px(line_font),
                    ..default()
                },
                TextColor(REMAIN_GOLD),
                TextLayout::justify(Justify::Center),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(REMAIN_LINE1_TOP * s),
                    width: Val::Px(REMAIN_W * s),
                    height: Val::Px(REMAIN_LINE_H * s),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                Pickable::IGNORE,
            ));
            // The PML line: one text node per colour run, laid out as a
            // centred row so the runs read as one sentence. It carries
            // `DeletionCountdown`, which is what `tick_deletion_countdown`
            // re-renders once a displayed minute has passed.
            p.spawn((
                DeletionCountdown {
                    minutes_left,
                    template: countdown_template.clone(),
                    shown: minutes_left as u32,
                },
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(REMAIN_LINE2_TOP * s),
                    width: Val::Px(REMAIN_W * s),
                    height: Val::Px(REMAIN_LINE_H * s),
                    flex_direction: FlexDirection::Row,
                    // **Left-flush and clipped, like the original**: it prints
                    // `Complete deletion of the selected character in 7days
                    // 0hour` and the rest is **cut off at the window edge** —
                    // the string carries three `%d` and no `<br>`, so the
                    // original clips it rather than wrapping it. Centring the
                    // row would clip it at *both* ends instead of only the
                    // right one.
                    //
                    // Narrowing that: it holds for the *wrapping* question and
                    // for this row's layout, and both stay as they are. It does
                    // **not** hold for the size — the string is wrapped in
                    // `<sml2>`, a PML size class, so the original does ask for a
                    // smaller font on this very line and `resolve_pml` is what
                    // drops it. So the text below draws at
                    // `REMAIN_LINE2_FONT_PX` and this clip is the safety net
                    // behind it, not the fit itself.
                    justify_content: JustifyContent::FlexStart,
                    align_items: AlignItems::Center,
                    overflow: Overflow::clip(),
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|line| {
                for (run, color) in &runs {
                    line.spawn((
                        Text::new(run.clone()),
                        TextFont {
                            font: fonts.nine.clone().into(),
                            font_size: FontSize::Px(countdown_font),
                            ..default()
                        },
                        // The other half of the same decision: without `NoWrap`
                        // Bevy breaks the first run into two lines inside a
                        // one-line-high clipped row, which renders as two
                        // half-cut lines of text on top of each other.
                        TextLayout {
                            linebreak: LineBreak::NoWrap,
                            ..default()
                        },
                        TextColor(*color),
                        Pickable::IGNORE,
                    ));
                }
            });
            // The authored gauge: `GDR_REMAING` (`CIFGauge`, the fill,
            // `delete_time_gauge.ddj`) and `GDR_REMAINGBOX`
            // (`delete_time_gauge_up.ddj`) carry the *same* rect `24,63,280,8`,
            // so the two arts are layered, not placed side by side — `_up` is
            // the overlay drawn on top of the fill. The fill is clipped by a
            // percentage-wide wrapper (the same trick the HP/MP bars above use)
            // instead of being scaled, so the art keeps its proportions while
            // the bar shortens.
            p.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(REMAIN_GAUGE.0 * s),
                    top: Val::Px(REMAIN_GAUGE.1 * s),
                    width: Val::Px(REMAIN_GAUGE.2 * s),
                    height: Val::Px(REMAIN_GAUGE.3 * s),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(REMAIN_GAUGE_TRACK),
                Pickable::IGNORE,
            ))
            .with_children(|g| {
                g.spawn((
                    DeletionCountdownGaugeFill,
                    Node {
                        width: Val::Percent(fill * 100.0),
                        height: Val::Percent(100.0),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|f| {
                    f.spawn((
                        ImageNode {
                            image: gauge_art,
                            image_mode: NodeImageMode::Stretch,
                            ..default()
                        },
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            width: Val::Px(REMAIN_GAUGE.2 * s),
                            height: Val::Px(REMAIN_GAUGE.3 * s),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                });
            });
            p.spawn((
                ImageNode {
                    image: gauge_frame_art,
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(REMAIN_GAUGE.0 * s),
                    top: Val::Px(REMAIN_GAUGE.1 * s),
                    width: Val::Px(REMAIN_GAUGE.2 * s),
                    height: Val::Px(REMAIN_GAUGE.3 * s),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        });
    });
}

/// Swaps the underbar to Start/Delete/Cancel and shows the info box when a
/// character gets selected.
pub fn spawn_selection_ui(
    selected: Res<SelectedCharacterV2>,
    info_query: Query<&CharacterInfoV2>,
    controls: Query<Entity, With<CharSelectControls>>,
    level_data: Res<ClientLevelData>,
    assets: Res<IntroV2Assets>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut info_text_writer: MessageWriter<InfoTextV2Update>,
    mut commands: Commands,
) {
    let Ok(info) = info_query.get(selected.0) else {
        return;
    };
    let Some(camera) = cam_query.iter().next() else {
        warn!("no 2d camera found");
        return;
    };

    for entity in controls.iter() {
        commands.entity(entity).despawn();
    }
    commands
        .spawn_scene(info_box(&info.0, &level_data, &assets, &fonts, &ui_strings))
        .insert((UiTargetCamera(camera), IntroV2Ui));
    // A character awaiting deletion gets Restore + Cancel instead of
    // Start/Delete/Cancel.
    if info.0.is_deleting {
        commands
            .spawn_scene(deleting_control_buttons(&assets, &fonts, &ui_strings))
            .insert((UiTargetCamera(camera), IntroV2Ui));
        // …and the countdown that says how much of the 7-day reservation is
        // left, which the two buttons above are the answer to.
        spawn_deletion_countdown(&mut commands, camera, &info.0, &assets, &fonts, &ui_strings);
        // The original also states the two-button contract as a three-line
        // orange notice in the lower band — `UIO_STT_CHAR_DEL_WAITING`, row 264
        // of `textdata/textuisystem.txt`, with line breaks stored as literal
        // `\n` escapes like its sibling `UIO_STT_CHAR_DEL_CONFIRM`. We had the
        // window and the buttons but no sentence telling the user what they do.
        info_text_writer.write(InfoTextV2Update(
            ui_strings
                .get_plain_or(
                    "UIO_STT_CHAR_DEL_WAITING",
                    "The character's deletion is reserved.\nTo restore it, click 'Restore' \
                     button.\nTo keep the reservation, click 'Cancel' button.",
                )
                .replace("\\n", "\n"),
        ));
    } else {
        commands
            .spawn_scene(selected_control_buttons(&assets, &fonts, &ui_strings))
            .insert((UiTargetCamera(camera), IntroV2Ui));
        // Selecting a healthy character after a pending one must not leave the
        // "deletion is reserved" sentence standing under it: the notice line is
        // one entity for the whole intro scene, so whoever owns the screen has
        // to clear it: the original leaves the band empty for a normal
        // selection.
        info_text_writer.write(InfoTextV2Update(String::new()));
    }
}

/// Tears the selection UI down and brings the Create/Cancel row back once
/// the selection resource disappears (Cancel or successful deletion). The
/// respawned row starts transparent, so `show_screen::<CharSelectControls>`
/// is chained after this system to fade it in.
pub fn on_deselect(
    selection_ui: Query<
        Entity,
        Or<(
            With<CharSelectInfoBox>,
            With<SelectedCharControls>,
            With<DeleteConfirmModal>,
            With<RestoreConfirmModal>,
            With<DeletionCountdownWindow>,
        )>,
    >,
    existing_controls: Query<(), With<CharSelectControls>>,
    assets: Res<IntroV2Assets>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut info_text_writer: MessageWriter<InfoTextV2Update>,
    mut commands: Commands,
) {
    for entity in selection_ui.iter() {
        commands.entity(entity).despawn();
    }
    // The "deletion is reserved" notice belongs to the pending selection, so it
    // goes when the selection does (Cancel, or a successful delete/restore).
    info_text_writer.write(InfoTextV2Update(String::new()));

    // `resource_removed` can fire once spuriously when the state is
    // re-entered (the resource was dropped while this system wasn't
    // running), so never stack a second row onto an existing one.
    if !existing_controls.is_empty() {
        return;
    }
    let Some(camera) = cam_query.iter().next() else {
        warn!("no 2d camera found");
        return;
    };
    spawn_default_controls(&mut commands, &assets, &fonts, &ui_strings, camera);
}

/// `Activate` observer of the Cancel button in the selection underbar: flies
/// the camera back out and drops the selection ([`on_deselect`] restores the
/// UI).
pub fn on_selection_cancel_activate(
    _activate: On<Activate>,
    pending: Option<Res<PendingWorldJoin>>,
    cam_query: Query<(Entity, &Transform), With<CinematicCamera2>>,
    char_select_scene: Res<ActiveCharSelectSceneV2>,
    origin: Res<WorldOrigin>,
    mut commands: Commands,
) {
    if pending.is_some() {
        return;
    }

    zoom_out_camera(&cam_query, &char_select_scene, &origin, &mut commands);
    commands.remove_resource::<SelectedCharacterV2>();
}

/// Size of the shared `warning_button.ddj` art: `76x32`, the same in both
/// modals' authored rects (`GDR_BTN_WACCEPT`/`WCANCEL` `..,145,76,32` here,
/// `GDR_BTN_WCREATE`/`WCANCEL` `..,75,76,32` on char-create). The *positions*
/// genuinely differ per screen and stay local; the art size is one fact and
/// lives next to the style that draws it.
pub(super) const WARNING_BUTTON_SIZE: (f32, f32) = (76.0, 32.0);

/// The shared `warning_button.ddj` three-state style. Both confirm modals in
/// the intro (delete on char-select, create on char-create) sit on the same
/// 76x32 art, so they share one style rather than each minting their own.
pub(super) fn warning_button_style(assets: &IntroV2Assets) -> ImageButtonStyle {
    ImageButtonStyle {
        normal: assets.warning_button.clone(),
        hover: assets.warning_button_focus.clone(),
        press: assets.warning_button_press.clone(),
        ..Default::default()
    }
}

// Native size of the warning_delete.ddj window frame and its
// warning_button.ddj buttons.
const DELETE_MODAL_WIDTH: f32 = 344.0;
const DELETE_MODAL_HEIGHT: f32 = 192.0;
/// `GDR_STA_WNAME`'s `FontColor="255,255,239,153"` (ARGB), the paler gold the
/// info box uses for Level — not the `208,81` gold of the captions.
const DELETE_MODAL_NAME_GOLD: Color = Color::srgb_u8(255, 239, 153);
/// `GDR_BTN_WACCEPT` `90,145,76,32` / `GDR_BTN_WCANCEL` `178,145,76,32`.
const WARNING_BUTTON_Y: f32 = 145.0;
const WARNING_BUTTON_WIDTH: f32 = WARNING_BUTTON_SIZE.0;
const WARNING_BUTTON_HEIGHT: f32 = WARNING_BUTTON_SIZE.1;

/// Deletion confirmation modal, following the captcha modal's template: a
/// dimming fullscreen scrim (which deliberately blocks clicks on the
/// characters and the underbar) with the centered warning window.
/// Button geometry from `resinfo/pscharacterselect.txt`: every `GDR_BTN_*` on
/// this screen is `0,0,92,41` (we had 91 wide).
const MAIN_BUTTON_W: f32 = 92.0;
const MAIN_BUTTON_H: f32 = 41.0;

/// The shared body of the two warning modals on this screen, parameterised by
/// the three strings and the marker component that tell them apart.
///
/// Idea: delete and restore are the *same authored window*. Both draw
/// `warning_delete.ddj` at `344x192`, both put `GDR_STA_WNAME` (`0,24,344,15`,
/// `FontIndex=2`, the paler gold) at the top and `GDR_TB_INFO` (`16,48,316,90`,
/// `FontIndex=0`) below it, and both carry `GDR_BTN_WACCEPT` `90,145,76,32` and
/// `GDR_BTN_WCANCEL` `178,145,76,32` — one resinfo section, so one builder. The
/// two used to be transcribed side by side, which is two places for the same
/// rect to be corrected in.
///
/// What genuinely differs stays a parameter: the title/body/accept strings, the
/// accept observer (delete sends action 2, restore action 5 — deliberately not
/// merged into one dispatching observer), and the marker `M`. `M` is *not* part
/// of the scene: the spawn site inserts it next to `UiTargetCamera`/`IntroV2Ui`,
/// which keeps the generic out of the `bsn!` tree while the Cancel button can
/// still despawn "its own" modal through `With<M>`.
fn warning_modal<M, A, AM>(
    title: &str,
    body: &str,
    accept_label: &str,
    accept: A,
    assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
) -> impl Scene
where
    M: Component,
    A: IntoObserverSystem<Activate, (), AM> + Clone + Send + Sync + 'static,
    AM: Send + Sync + 'static,
{
    let window = assets.warning_delete_window.clone();
    let text_font = fonts.nine.clone();
    let confirm_font = fonts.nine.clone();
    let cancel_font = fonts.nine.clone();
    // The warning body `GDR_TB_INFO` (`:1170`) and both buttons
    // `GDR_BTN_WACCEPT`/`GDR_BTN_WCANCEL` (`:1132`, `:1113`) are `FontIndex=0`
    // -> 12 px.
    let text_px = intro_font_px(0);
    // `GDR_STA_WNAME` is its own control: `0,24,344,15`, `FontIndex=2` (16 px)
    // and the paler gold `FontColor="255,255,239,153"` (ARGB -> RGB
    // 255,239,153). The original uses the character name as the modal's
    // *title*, for delete and for restore alike, and puts the accept button
    // left of Cancel in both.
    let name_font = fonts.nine.clone();
    let name_px = intro_font_px(2);
    let button_sound = assets.sound_button_sound_a.clone();
    let cancel_sound = button_sound.clone();

    let title_line = title.to_string();
    let body_text = body.to_string();
    let accept_text = accept_label.to_string();
    let cancel_label = ui_strings
        .get_or("UIO_COMMON_CTL_CANCEL", "Cancel")
        .to_string();

    bsn! {
        modal_scrim()
        Children [
            (
                modal_plate(window, DELETE_MODAL_WIDTH, DELETE_MODAL_HEIGHT)
                Children [
                    (
                        // `GDR_STA_WNAME` `0,24,344,15`, centred over the full
                        // frame width, gold.
                        label(&title_line, name_font, name_px)
                        TextColor({DELETE_MODAL_NAME_GOLD})
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(0),
                            top: px(24),
                            width: px(DELETE_MODAL_WIDTH),
                            height: px(15),
                        }
                    ),
                    (
                        // `GDR_TB_INFO` `16,48,316,90`.
                        // label() starts transparent for the fade systems,
                        // which never run on this modal, so force it white.
                        label(&body_text, text_font, text_px)
                        TextColor(Color::WHITE)
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(16),
                            top: px(48),
                            width: px(316),
                            height: px(90),
                        }
                    ),
                    (
                        image_button(warning_button_style(assets), WARNING_BUTTON_WIDTH, WARNING_BUTTON_HEIGHT)
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(90),
                            top: px(WARNING_BUTTON_Y),
                            width: px(WARNING_BUTTON_WIDTH),
                            height: px(WARNING_BUTTON_HEIGHT),
                        }
                        ButtonSound({button_sound})
                        Children [ (label(&accept_text, confirm_font, text_px) TextColor(Color::WHITE)) ]
                        on(accept)
                    ),
                    (
                        image_button(warning_button_style(assets), WARNING_BUTTON_WIDTH, WARNING_BUTTON_HEIGHT)
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(178),
                            top: px(WARNING_BUTTON_Y),
                            width: px(WARNING_BUTTON_WIDTH),
                            height: px(WARNING_BUTTON_HEIGHT),
                        }
                        ButtonSound({cancel_sound})
                        Children [ (label(&cancel_label, cancel_font, text_px) TextColor(Color::WHITE)) ]
                        on(|_activate: On<Activate>,
                            modal_query: Query<Entity, With<M>>,
                            mut commands: Commands| {
                            for modal in modal_query.iter() {
                                commands.entity(modal).despawn();
                            }
                        })
                    ),
                ]
            ),
        ]
    }
}

/// Deletion confirmation modal: the original's own 7-day reservation warning
/// (`UIO_STT_CHAR_DEL_CONFIRM`, which our hand-written line used to drop
/// entirely) over the character's name, with **Delete** — not "Confirm" —
/// on the accept button (`UIO_SELCHAR_CTL_DELETE`).
fn delete_modal(
    character_name: &str,
    assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
) -> impl Scene {
    let warning = unescape_newlines(
        ui_strings
            .get("UIO_STT_CHAR_DEL_CONFIRM")
            .unwrap_or("The character will be deleted."),
    );
    let accept_label = ui_strings.get_or("UIO_SELCHAR_CTL_DELETE", "Delete");
    warning_modal::<DeleteConfirmModal, _, _>(
        character_name,
        &warning,
        accept_label,
        on_delete_confirm_activate,
        assets,
        fonts,
        ui_strings,
    )
}

/// `Activate` observer of the Delete button: opens the confirmation modal.
pub fn on_delete_activate(
    _activate: On<Activate>,
    selected: Option<Res<SelectedCharacterV2>>,
    info_query: Query<&CharacterInfoV2>,
    existing_modal: Query<(), With<DeleteConfirmModal>>,
    assets: Res<IntroV2Assets>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut commands: Commands,
) {
    if !existing_modal.is_empty() {
        return;
    }
    let Some(selected) = selected else {
        return;
    };
    let Ok(info) = info_query.get(selected.0) else {
        return;
    };
    let Some(camera) = cam_query.iter().next() else {
        warn!("no 2d camera found");
        return;
    };

    commands
        .spawn_scene(delete_modal(&info.0.name, &assets, &fonts, &ui_strings))
        // The marker rides on the spawn, not in the shared `bsn!` tree — see
        // [`warning_modal`].
        .insert((
            UiTargetCamera(camera),
            IntroV2Ui,
            DeleteConfirmModal,
            Name::new("Delete Confirmation Modal V2"),
        ));
}

/// `Activate` observer of the modal's Confirm button: sends the deletion
/// request and closes the modal; [`on_character_delete_response`] handles
/// the outcome.
pub fn on_delete_confirm_activate(
    _activate: On<Activate>,
    selected: Option<Res<SelectedCharacterV2>>,
    info_query: Query<&CharacterInfoV2>,
    conn_query: Query<&SilkroadConnection, With<AgentConnection>>,
    modal_query: Query<Entity, With<DeleteConfirmModal>>,
    mut commands: Commands,
) {
    for modal in modal_query.iter() {
        commands.entity(modal).despawn();
    }

    let Some(selected) = selected else {
        return;
    };
    let Ok(info) = info_query.get(selected.0) else {
        return;
    };
    let Ok(conn) = conn_query.single() else {
        return;
    };

    let frame = Packet::from(CharacterSelectionActionRequest {
        action: CharacterSelectionAction::Delete,
        name: Some(info.0.name.clone()),
        create: None,
    })
    .into();

    if let Err(e) = conn.get_sender().send(frame) {
        error!("failed to send frame: {}", e.0);
    }
}

/// The restore confirmation modal — the same `warning_delete.ddj` window, the
/// same two-button row, `Restore` where the delete twin says `Delete`.
///
/// The original opens a modal on Restore with the character name as its title,
/// five lines of body copy and **Restore | Cancel** (Restore left), while the
/// countdown window stays visible behind it. Earlier we sent action 5 straight
/// from the button.
///
/// The body text is `UIO_STT_CHAR_RECOVERY_CONFIRM`: row 263 of
/// `textdata/textuisystem.txt` (the sibling of the delete warning at 262) reads
/// *"The character will be restored.\nIf you restore it, the reservation will
/// be canceled.\nSo you can use it normally.\nIf you cancel the restoration, it
/// will be reserved again.\n\nWill you now restore the character?"* — character
/// for character the copy the original shows, including the blank line before
/// the question. So both the key binding and the hardcoded fallback are
/// sourced, not guessed. Line breaks are the same literal two-character `\n`
/// escapes as in the delete warning.
fn restore_modal(
    character_name: &str,
    assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
) -> impl Scene {
    let body = unescape_newlines(&ui_strings.get_plain_or(
        "UIO_STT_CHAR_RECOVERY_CONFIRM",
        "The character will be restored.\nIf you restore it, the reservation will be canceled.\nSo \
         you can use it normally.\nIf you cancel the restoration, it will be reserved \
         again.\n\nWill you now restore the character?",
    ));
    let accept_label = ui_strings.get_or("UIO_STT_CHAR_RECOVERY", "Restore");
    warning_modal::<RestoreConfirmModal, _, _>(
        character_name,
        &body,
        accept_label,
        on_restore_confirm_activate,
        assets,
        fonts,
        ui_strings,
    )
}

/// `Activate` observer of the Restore button shown for a deletion-pending
/// character: opens the confirmation modal. Cancelling it sends nothing — only
/// `0x2002` keepalives go out while either modal is open.
pub fn on_restore_activate(
    _activate: On<Activate>,
    selected: Option<Res<SelectedCharacterV2>>,
    info_query: Query<&CharacterInfoV2>,
    existing_modal: Query<(), With<RestoreConfirmModal>>,
    assets: Res<IntroV2Assets>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut commands: Commands,
) {
    if !existing_modal.is_empty() {
        return;
    }
    let Some(selected) = selected else {
        return;
    };
    let Ok(info) = info_query.get(selected.0) else {
        return;
    };
    let Some(camera) = cam_query.iter().next() else {
        warn!("no 2d camera found");
        return;
    };

    commands
        .spawn_scene(restore_modal(&info.0.name, &assets, &fonts, &ui_strings))
        .insert((
            UiTargetCamera(camera),
            IntroV2Ui,
            RestoreConfirmModal,
            Name::new("Restore Confirmation Modal V2"),
        ));
}

/// `Activate` observer of the restore modal's Restore button: sends action 5
/// with the character's name (same wire shape as Delete, answered by
/// `0xB007`) and closes the modal.
/// [`on_character_delete_response`] handles the outcome.
pub fn on_restore_confirm_activate(
    _activate: On<Activate>,
    selected: Option<Res<SelectedCharacterV2>>,
    info_query: Query<&CharacterInfoV2>,
    conn_query: Query<&SilkroadConnection, With<AgentConnection>>,
    modal_query: Query<Entity, With<RestoreConfirmModal>>,
    mut commands: Commands,
) {
    for modal in modal_query.iter() {
        commands.entity(modal).despawn();
    }

    let Some(selected) = selected else {
        return;
    };
    let Ok(info) = info_query.get(selected.0) else {
        return;
    };
    let Ok(conn) = conn_query.single() else {
        return;
    };

    let frame = Packet::from(CharacterSelectionActionRequest {
        action: CharacterSelectionAction::Restore,
        name: Some(info.0.name.clone()),
        create: None,
    })
    .into();

    if let Err(e) = conn.get_sender().send(frame) {
        error!("failed to send frame: {}", e.0);
    }
}

/// Handles the server's answer to a deletion or restore request: on success the
/// camera flies back out and the lobby is rebuilt from a fresh list request (a
/// deleted character then shows its deletion countdown state; a restored one is
/// back to normal).
pub fn on_character_delete_response(
    mut reader: MessageReader<CharacterSelectionActionResponse>,
    mut info_text_writer: MessageWriter<InfoTextV2Update>,
    characters: Query<Entity, With<SelectableCharacterV2>>,
    conn_query: Query<&SilkroadConnection, With<AgentConnection>>,
    cam_query: Query<(Entity, &Transform), With<CinematicCamera2>>,
    char_select_scene: Res<ActiveCharSelectSceneV2>,
    assets: Res<IntroV2Assets>,
    origin: Res<WorldOrigin>,
    options: Res<GameOptions>,
    ui_strings: Res<ClientUiStrings>,
    mut commands: Commands,
) {
    for res in reader.read() {
        let action = match res.action {
            CharacterSelectionAction::Delete => "deletion",
            CharacterSelectionAction::Restore => "restore",
            _ => continue,
        };

        if res.result == 1 {
            zoom_out_camera(&cam_query, &char_select_scene, &origin, &mut commands);
            commands.remove_resource::<SelectedCharacterV2>();
            for entity in characters.iter() {
                commands.entity(entity).despawn();
            }
            if let Ok(conn) = conn_query.single() {
                send_character_list_request(conn);
            }
        } else {
            // The refusal now speaks the original's own row instead of our
            // prose: the char-select pump answers *any* `0xB007` with
            // `result != 1` by reading the u16 and calling the one lobby error
            // dispatcher, so delete and restore resolve through exactly the
            // same table as CheckName and Create — see
            // [`super::lobby_error_line`]. `0x0419` (restore not scheduled) is
            // a code that table has no arm for; it renders as the original's
            // bare `(S1049)`.
            let code = res.error_code.unwrap_or_default();
            warn!("character {action} rejected (error {:#06x})", code);
            if let Some(line) = super::lobby_error_line(code, &ui_strings) {
                info_text_writer.write(InfoTextV2Update(line));
            }
            play_error_sound(&mut commands, &assets, &options);
        }
    }
}

/// The three chrome rects of this overlay, in percent of the window.
///
/// Idea, and why this is not three constants of its own: the four
/// `Section = Loading` controls of `pscharacterselect_europe.txt` are the
/// **same** authored chrome the loading screen draws — `GDR_LOADINGFRAME` id 23
/// `241,973,1121,64` (`:345`), `GDR_LOADINGG` id 24 `268,985,1064,20` (`:326`),
/// `GDR_LOADING_STA` id 27 `268,1025,252,35` (`:307`), all rect-bearing, i.e.
/// canvas-scaled from 1600x1200. They were transcribed a second time here,
/// together with a second copy of the percentage math, which is exactly the
/// duplicate the loading screen's own doc comment warns about ("every one of
/// them that re-derived the geometry got it wrong"). So the numbers and the
/// conversion now come from [`crate::scenes::loading_screen`].
///
/// What is *not* shared is the spawning: this overlay is a `bsn!` scene, so it
/// cannot call `loading_screen::spawn_loading_chrome`, which takes a
/// `ChildSpawnerCommands`. Deduplicating the data rather than inventing an
/// abstraction that cannot serve this caller is the deliberate choice here.
/// The background (id 22, `0,0,1600,1200`) stays local: it is a full-window
/// stretch and each screen names its own picture.
fn join_chrome_pct() -> [(f32, f32, f32, f32); 3] {
    use crate::scenes::loading_screen::{design_pct, CAPTION_RECT, FRAME_RECT, GAUGE_RECT};

    [
        design_pct(FRAME_RECT),
        design_pct(GAUGE_RECT),
        design_pct(CAPTION_RECT),
    ]
}

/// Fullscreen overlay shown while the world join is in flight, reusing the
/// vanilla loading artwork. Geometry: [`join_chrome_pct`].
fn join_loading_overlay(assets: &IntroV2Assets, fonts: &FontAssets) -> impl Scene {
    use crate::scenes::loading_screen::{DesignFit, LoadingBackdrop};

    // `loading_default.ddj` stood here, and a classified sweep over all 247
    // resinfo files finds **zero** references to it: it is the fallback of the
    // *world* loading screen (the `loading_<region>.ddj` family), a different
    // screen entirely. The four arts below are what this screen authors
    // (`Section = Loading`), so this was the wrong picture, not a near miss.
    let background = assets.join_loading_background.clone();
    let backdrop = background.clone();
    let frame = assets.join_loading_frame.clone();
    let gauge = assets.join_loading_gauge.clone();
    let label_art = assets.join_loading_label.clone();
    let font = fonts.nine.clone();

    let [(frame_l, frame_t, frame_w, frame_h), (gauge_l, gauge_t, gauge_w, gauge_h), (label_l, label_t, label_w, label_h)] =
        join_chrome_pct();

    bsn! {
        JoinLoadingOverlay
        Name("Join Loading Overlay V2")
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            // the 4:3 cover box below overflows on the axis that does not fit
            overflow: {bevy::ui::Overflow::clip()},
        }
        BackgroundColor(Color::BLACK)
        // above the fade screen (100) so nothing of the scene shines through
        GlobalZIndex(200)
        Pickable::IGNORE
        Children [
            (
                // The leftover area around the 4:3 painting: the same picture
                // blown up to cover the window, blurred and dimmed, so a
                // widescreen window gets colour instead of black bars
                // (`loading_screen::DESIGN_ASPECT`). Sizes come
                // from `fit_design_surfaces`, the material from
                // `attach_backdrop_material`.
                LoadingBackdrop({backdrop})
                DesignFit { cover: true }
                Node { position_type: PositionType::Absolute, left: px(0), top: px(0), width: px(0), height: px(0) }
                Pickable::IGNORE
            ),
            (
                // `GDR_LOADING` `0,0,1600,1200`: the background fills the
                // canvas. It used to stretch to the *window*, which squashed
                // the 1024x768 painting on every 16:9 screen, and then to
                // *cover* it, which cropped up to 44% of it away; it now sits
                // whole in the 4:3 contain box.
                ImageNode { image: {background}, image_mode: NodeImageMode::Stretch }
                DesignFit { cover: false }
                Node { position_type: PositionType::Absolute, left: px(0), top: px(0), width: px(0), height: px(0) }
                Pickable::IGNORE
            ),
            (
                // The authored chrome keeps the 4:3 proportions it was drawn
                // for: percentages of this centred box, not of the window.
                DesignFit { cover: false }
                Node { position_type: PositionType::Absolute, left: px(0), top: px(0), width: px(0), height: px(0) }
                Pickable::IGNORE
                Children [
                    (
                        ImageNode { image: {frame}, image_mode: NodeImageMode::Stretch }
                        Node { position_type: PositionType::Absolute, left: percent(frame_l), top: percent(frame_t), width: percent(frame_w), height: percent(frame_h) }
                        Pickable::IGNORE
                    ),
                    (
                        // `GDR_LOADINGG` is a `CIFGauge` whose art is a 4x12 fill tile
                        // stretched over the 1064 px track — stretching is the intended
                        // mode here, not a workaround.
                        //
                        // It is drawn FULL, and that is a deliberate honest fallback
                        // (the same one `gauge_fill` states for an underivable maximum):
                        // the join has no progress signal — `0xB001`/`0x3020` arrive as
                        // single steps, the terrain streaming that follows lives in the
                        // game scene and reports nothing back here. A part-filled bar
                        // would be an invented number; the world scene owns the real
                        // loading progress.
                        ImageNode { image: {gauge}, image_mode: NodeImageMode::Stretch }
                        Node { position_type: PositionType::Absolute, left: percent(gauge_l), top: percent(gauge_t), width: percent(gauge_w), height: percent(gauge_h) }
                        Pickable::IGNORE
                    ),
                    (
                        // `GDR_LOADING_STA` — the "now loading" strip is ART, not text,
                        // so it needs no string key and no font.
                        ImageNode { image: {label_art}, image_mode: NodeImageMode::Stretch }
                        Node { position_type: PositionType::Absolute, left: percent(label_l), top: percent(label_t), width: percent(label_w), height: percent(label_h) }
                        Pickable::IGNORE
                    ),
                ]
            ),
            (
                // OUR OWN LINE, not the original's: `nowloading.ddj` says "now
                // loading" and nothing about *what*, and a frozen screen with no
                // sentence is the state players read as a hang. No textdata key
                // exists for it (checked), so it stays English and is marked as
                // a deviation rather than dressed up as sourced. It sits below
                // the authored frame (frame bottom = 1037/1200 = 86.4%), so it
                // covers none of the four arts.
                label("Joining world...", font, 16.0)
                TextColor(Color::WHITE)
                Node { position_type: PositionType::Absolute, bottom: percent(10), left: percent(0), width: percent(100), justify_content: JustifyContent::Center }
            ),
        ]
    }
}

/// `Activate` observer of the Start button: sends the join request and
/// covers the screen with the loading overlay right away — deliberately no
/// fade animation.
pub fn on_start_activate(
    activate: On<Activate>,
    pending: Option<Res<PendingWorldJoin>>,
    selected: Option<Res<SelectedCharacterV2>>,
    info_query: Query<&CharacterInfoV2>,
    conn_query: Query<&SilkroadConnection, With<AgentConnection>>,
    assets: Res<IntroV2Assets>,
    fonts: Res<FontAssets>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut commands: Commands,
) {
    if pending.is_some() {
        return;
    }
    let Some(selected) = selected else {
        return;
    };
    let Ok(info) = info_query.get(selected.0) else {
        return;
    };
    let Ok(conn) = conn_query.single() else {
        return;
    };
    let Some(camera) = cam_query.iter().next() else {
        warn!("no 2d camera found");
        return;
    };

    // double-activation guard; also clear Pressed because the release
    // observer skips disabled buttons (same pattern as the Connect button)
    commands
        .entity(activate.entity)
        .insert(InteractionDisabled)
        .remove::<Pressed>();

    let frame = Packet::from(CharacterJoinRequest {
        character_name: info.0.name.clone(),
    })
    .into();
    if let Err(e) = conn.get_sender().send(frame) {
        error!("failed to send frame: {}", e.0);
        commands
            .entity(activate.entity)
            .remove::<InteractionDisabled>();
        return;
    }

    commands.insert_resource(PendingWorldJoin {
        character_name: info.0.name.clone(),
    });
    // Carry the picked character into the game scene, which assembles the
    // real model + equipment from it (see game_scene::spawn_selected_player).
    commands.insert_resource(JoiningCharacter(info.0.clone()));
    commands
        .spawn_scene(join_loading_overlay(&assets, &fonts))
        .insert((UiTargetCamera(camera), IntroV2Ui));
}

/// Handles the join response: enters the world scene on success, otherwise
/// tears the loading overlay down again and re-enables the Start button.
pub fn on_character_join_response(
    mut reader: MessageReader<CharacterJoinResponse>,
    mut info_text_writer: MessageWriter<InfoTextV2Update>,
    mut next_scene: ResMut<NextState<SceneState>>,
    overlay_query: Query<Entity, With<JoinLoadingOverlay>>,
    start_buttons: Query<Entity, (With<StartButton>, With<InteractionDisabled>)>,
    assets: Res<IntroV2Assets>,
    options: Res<GameOptions>,
    ui_strings: Res<ClientUiStrings>,
    mut commands: Commands,
) {
    for res in reader.read() {
        if res.result == 1 {
            // Enter the dedicated in-game scene, which assembles the selected
            // character (carried via JoiningCharacter) and shows its own
            // loading screen until terrain + player are ready.
            next_scene.set(SceneState::GameWorld);
        } else {
            // Same table as the `0xB007` failures, and not merely assumed to
            // be: the original's `0xB001` arm reads the result byte, and on
            // anything but 1 it reads the u16 and calls the identical
            // dispatcher. So a join
            // refusal is rendered by [`super::lobby_error_line`] too, instead
            // of the invented "Failed to join the world (error 1049)." line
            // that stood here. A frame with `result` neither 1 nor 2 carries no
            // code at all (`when = "result == 2"`); it renders as `(S0)`, which
            // says "refused, no code" rather than naming a reason we do not have.
            let code = res.error.unwrap_or_default();
            error!("[CharacterJoin] failed with error {:#06x}", code);
            if let Some(line) = super::lobby_error_line(code, &ui_strings) {
                info_text_writer.write(InfoTextV2Update(line));
            }
            for entity in overlay_query.iter() {
                commands.entity(entity).despawn();
            }
            for entity in start_buttons.iter() {
                commands.entity(entity).remove::<InteractionDisabled>();
            }
            commands.remove_resource::<PendingWorldJoin>();
            play_error_sound(&mut commands, &assets, &options);
        }
    }
}

pub fn despawn_controls(query: Query<Entity, With<CharSelectControls>>, mut commands: Commands) {
    for entity in query.iter() {
        commands.entity(entity).despawn();
    }
}

/// Despawns the selection UI (info box, Start/Delete/Cancel row, modal) and
/// drops the selection when the character list state is left.
pub fn despawn_selection_ui(
    query: Query<
        Entity,
        Or<(
            With<CharSelectInfoBox>,
            With<SelectedCharControls>,
            With<DeleteConfirmModal>,
            With<RestoreConfirmModal>,
            With<DeletionCountdownWindow>,
            // The join overlay was despawned only by the join *response*
            // (`on_character_join_response`), so leaving the list any other way
            // — a failed join followed by Cancel, or the scene tearing down
            // mid-flight — left a fullscreen loading image on top of the next
            // screen. It belongs to the selection, so it leaves with it.
            With<JoinLoadingOverlay>,
            // The pooled name plate belongs to the stage it
            // labels; without this it survives into the next screen as a
            // floating name.
            With<CharacterNamePlate>,
        )>,
    >,
    mut commands: Commands,
) {
    for entity in query.iter() {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<SelectedCharacterV2>();
}

pub fn despawn_characters(
    query: Query<Entity, With<SelectableCharacterV2>>,
    mut commands: Commands,
) {
    for entity in query.iter() {
        commands.entity(entity).despawn();
    }
}

pub fn disconnect_from_agent_server(
    query: Query<Entity, With<AgentConnection>>,
    joining: Option<Res<PendingWorldJoin>>,
    entering_create: Option<Res<super::character_create::EnteringCharacterCreate>>,
    mut commands: Commands,
) {
    // A join in flight means this exit leads into the world scene, which keeps
    // talking to the same agent server. Entering the creation sub-state likewise
    // keeps the connection alive (Create/CheckName go over it).
    if joining.is_some() || entering_create.is_some() {
        return;
    }

    let Ok(entity) = query.single() else {
        return;
    };

    commands.entity(entity).despawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lower band's top edge at window height `h` — `barDown.top`, the
    /// anchor [`lower_band_anchor`] expresses as `bottom: percent(BAND_H_PCT)`.
    /// One derivation for both placement tests, so the band model cannot be
    /// "fixed" in one of them only.
    fn band_top(h: f32) -> f32 {
        h - h * BAND_H_PCT / 100.0
    }

    /// The upper band's bottom edge — `barUp.bottom`, the anchor
    /// [`upper_band_anchor`] expresses as `top: percent(BAND_H_PCT)`.
    fn band_bottom(h: f32) -> f32 {
        h * BAND_H_PCT / 100.0
    }

    /// Every `GDR_BTN_*` on this screen is `0,0,92,41` in
    /// `resinfo/pscharacterselect.txt` — we rendered them 91 wide.
    #[test]
    fn main_buttons_match_the_resinfo_rect() {
        assert_eq!((MAIN_BUTTON_W, MAIN_BUTTON_H), (92.0, 41.0));
    }

    /// The info box's gauges used to be pinned at 100%. The fill is
    /// `current / (1.02^(level-1) * stat * 10)`, clamped, with a full bar when
    /// the maximum cannot be derived.
    #[test]
    fn info_box_gauges_fill_from_the_derived_maximum() {
        // the canonical starting character: level 1, STR/INT 20 -> max 200
        assert_eq!(max_hp_or_mp(1, 20), 200);
        assert_eq!(gauge_fill(200, max_hp_or_mp(1, 20)), 1.0);
        assert_eq!(gauge_fill(100, max_hp_or_mp(1, 20)), 0.5);
        // a level-17 character: STR 36 -> 494, INT 84 -> 1153
        assert_eq!(max_hp_or_mp(17, 36), 494);
        assert_eq!(max_hp_or_mp(17, 84), 1153);
        assert_eq!(gauge_fill(247, 494), 0.5);
    }

    /// No panic and no empty bar when the maximum is unknown, and a server
    /// value above the derived maximum cannot overflow the track.
    #[test]
    fn gauge_fill_is_clamped_and_falls_back_to_full() {
        assert_eq!(gauge_fill(0, 0), 1.0, "unknown max -> full bar");
        assert_eq!(gauge_fill(500, 0), 1.0);
        assert_eq!(gauge_fill(0, 200), 0.0, "a dead character reads empty");
        assert_eq!(gauge_fill(999, 200), 1.0, "over-max clamps to full");
        assert!((0.0..=1.0).contains(&gauge_fill(u32::MAX, 1)));
        // level 0 must not produce a negative exponent
        assert_eq!(max_hp_or_mp(0, 20), 200);
    }

    /// The button row is placed by the original's **formula**, not by a
    /// constant bottom margin: `x = W-0xD1 {-,+} 0x68` at the native 92x41 and
    /// `y = barDown.top + 0x1B`. The x side is checked against the three boxes
    /// of an 800x600 frame, the y side against that frame's row *and* against
    /// 1920x1080, where the fixed 16 px bottom margin that stood here missed
    /// the band by 71 px.
    #[test]
    fn the_button_row_follows_the_originals_placement_formula() {
        let pitch = MAIN_BUTTON_W + BUTTON_ROW_GAP;
        assert_eq!(pitch, 104.0, "0x68 — the pitch the binary adds/subtracts");
        // x(CANCEL) = W - 0xD1 + 0x68 = W - 105
        let cancel_left = |w: f32| w - BUTTON_ROW_RIGHT - MAIN_BUTTON_W;
        assert_eq!(cancel_left(800.0), 695.0, "W - 105");
        assert_eq!(cancel_left(800.0) - pitch, 591.0, "W - 209: Create/Delete");
        assert_eq!(cancel_left(800.0) - 2.0 * pitch, 487.0, "W - 313: Start");
        // y = barDown.top + 27, and barDown.top scales with the window because
        // the band is rect-bearing (`Rect="0,1030,1600,172"`)
        let row_top = |h: f32| band_top(h) + BUTTON_ROW_TOP_IN_BAND;
        assert_eq!(band_top(600.0), 514.0, "515 on the 800x600 frame");
        assert!(
            (row_top(600.0) - 543.0).abs() <= 2.0,
            "row top 543, got {}",
            row_top(600.0)
        );
        // the same formula at a modern resolution — and the old constant
        // margin's answer, 71 px lower, for contrast
        assert!((row_top(1080.0) - 952.2).abs() <= 0.1);
        assert!(row_top(1080.0) < 1080.0 - 16.0 - MAIN_BUTTON_H);
    }

    /// The info box is code-placed too (`GDR_STA_CHARINFO` `Rect 0,0,228,140`)
    /// and sits at origin (529, 128) at 800x600 — but its y is
    /// `barUp.bottom + 0x29`, so the 128 is a *result* at that one window size,
    /// not a constant.
    #[test]
    fn the_info_box_is_placed_from_the_upper_band() {
        let origin_x = 800.0 - INFO_BOX_RIGHT - 228.0;
        assert_eq!(origin_x, 529.0);
        assert_eq!(band_bottom(600.0), 86.0);
        assert_eq!(band_bottom(600.0) + INFO_BOX_TOP_BELOW_BAND, 127.0);
        // 128 in the original -> one pixel of slack
        assert!((band_bottom(600.0) + INFO_BOX_TOP_BELOW_BAND - 128.0).abs() <= 1.0);
    }

    /// The button captions are pinned to the original's font ladder, not to a
    /// cap height read off a picture: all four `GDR_BTN_*` on this screen are
    /// `FontIndex=2`, which is 12 pt = 16 px. The buttons are drawn at their
    /// native 92x41, so the size is the *unscaled* ladder entry — asserted
    /// against `intro_font_px` so a future `hud_scale()` creeping into the
    /// button art shows up here as a mismatch, and against the two independent
    /// sources' arithmetic so "tidying" the ladder cannot move it silently.
    #[test]
    fn the_button_caption_size_is_the_ladder_entry_for_font_index_2() {
        assert_eq!(BUTTON_LABEL_SIZE, 16.0);
        assert_eq!(BUTTON_LABEL_SIZE, super::super::intro_font_px(2));
        // 12 pt at 96 dpi -> 16 px, the same conversion the original does
        assert_eq!(BUTTON_LABEL_SIZE, (12.0f32 * 96.0 / 72.0).round());
    }

    /// The info box is the one intro surface built at `hud_scale()`, so its text
    /// must carry the same factor — and land on a whole pixel. This is the
    /// regression the `11.0 * hud_scale()` here was: 16.5 px, a third under the
    /// `FontIndex=2` the box's seven text controls all declare.
    #[test]
    fn the_info_box_text_scales_with_the_box_and_stays_a_whole_pixel() {
        // `hud_scale()` is a process global other tests in this crate write, so
        // the assertions below hold for *any* valid scale rather than for 1.5.
        let size = font_px(2);
        assert_eq!(size.fract(), 0.0, "a fractional em rasterises off-grid");
        assert!(size >= intro_font_px(2), "text must not shrink below 16 px");
        // the old guess was 11 px of ladder the box has no control for
        assert_ne!(intro_font_px(2), 11.0);
        assert!(hud_scale() > 0.0);
    }

    /// The delete-warning modal's two buttons are `GDR_BTN_WACCEPT`
    /// `90,145,76,32` and `GDR_BTN_WCANCEL` `178,145,76,32`. We had y=136.
    #[test]
    fn delete_modal_buttons_match_the_resinfo_rects() {
        assert_eq!(WARNING_BUTTON_Y, 145.0);
        assert_eq!((WARNING_BUTTON_WIDTH, WARNING_BUTTON_HEIGHT), (76.0, 32.0));
    }

    /// `UIO_STT_CHAR_DEL_CONFIRM` stores its line breaks as the two characters
    /// `\` `n`, so the raw string has to be unescaped or the whole 7-day
    /// warning renders as one run-on line.
    #[test]
    fn the_deletion_warning_unescapes_its_line_breaks() {
        let raw = "The character will be deleted.\\nIf you delete the character, \
                   it will be reserved for 7 days.";
        let shown = raw.replace("\\n", "\n");
        assert!(shown.contains('\n'), "literal \\n must become a real break");
        assert!(!shown.contains("\\n"), "no escapes may survive");
        assert_eq!(shown.lines().count(), 2);
    }

    /// The join overlay showed `loading_default.ddj`, which **no** resinfo file
    /// references — it belongs to the world loading screen. This screen authors
    /// four rect-bearing controls, so they are percentages of the 1600x1200
    /// canvas, and the frame must contain the gauge and the label strip.
    ///
    /// Second thing pinned here: the overlay reads the *same* rects as
    /// `loading_screen`, not a second transcription of them. The three
    /// constants were duplicated bit for bit in this file; if a future edit
    /// re-localises them, this test fails.
    #[test]
    fn the_join_overlay_uses_the_authored_loading_rects() {
        use crate::plugins::ui_v2::RESINFO_CANVAS as CANVAS;
        use crate::scenes::loading_screen::{design_pct, CAPTION_RECT, FRAME_RECT, GAUGE_RECT};

        assert_eq!(FRAME_RECT, (241.0, 973.0, 1121.0, 64.0));
        assert_eq!(GAUGE_RECT, (268.0, 985.0, 1064.0, 20.0));
        assert_eq!(CAPTION_RECT, (268.0, 1025.0, 252.0, 35.0));
        // gauge and label sit inside the frame's x span
        let frame_right = FRAME_RECT.0 + FRAME_RECT.2;
        assert!(GAUGE_RECT.0 > FRAME_RECT.0);
        assert!(GAUGE_RECT.0 + GAUGE_RECT.2 < frame_right);
        assert!(CAPTION_RECT.0 + CAPTION_RECT.2 < frame_right);
        // and the whole group lives in the lower sixth of the canvas
        assert!(FRAME_RECT.1 / CANVAS.1 > 0.8);
        assert_eq!(CANVAS, (1600.0, 1200.0));

        // the overlay's own three rows are those rects, through the shared
        // conversion — one source for both renderers
        assert_eq!(
            join_chrome_pct(),
            [
                design_pct(FRAME_RECT),
                design_pct(GAUGE_RECT),
                design_pct(CAPTION_RECT)
            ]
        );
        // and the conversion is the percentage math the overlay used to inline
        assert_eq!(
            design_pct(FRAME_RECT),
            (
                100.0 * 241.0 / 1600.0,
                100.0 * 973.0 / 1200.0,
                100.0 * 1121.0 / 1600.0,
                100.0 * 64.0 / 1200.0
            )
        );
    }

    /// `UIO_MSG_CHAR_DEL_TIME` is markup: a plain-text renderer prints
    /// `<sml2>` and the `<font>` tags on screen. The resolver must drop the
    /// size class, keep the words, and turn the font tag into a run colour.
    #[test]
    fn the_countdown_line_resolves_its_pml_markup() {
        let raw = "<sml2>Complete deletion in <font color=\"255,255,208,81\">3</font>days";
        let runs = resolve_pml(raw, Color::WHITE);
        let text: String = runs.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(text, "Complete deletion in 3days");
        assert!(!text.contains('<'), "no tag may reach the screen");
        // three runs: white lead-in, gold number, white tail
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[1].0, "3");
        assert_eq!(runs[1].1, REMAIN_GOLD);
        assert_eq!(runs[2].1, Color::WHITE, "the colour ends with </font>");
    }

    /// The longest sentence the countdown line can produce has to *fit*
    /// `GDR_PML_WREMINTIME2` (328 px), at every scale a user can set.
    ///
    /// The red control is inside the test on purpose: the first assertion is
    /// that the size this line used to draw at — the control's own `FontIndex`
    /// 0, i.e. the ladder's 12 px — does **not** fit. Without it the second
    /// assertion would pass for any small number and would prove nothing.
    #[test]
    fn the_countdown_line_fits_its_control_at_every_hud_scale() {
        for scale in [1.0f32, 1.5, 2.0, 2.5, 3.0] {
            assert!(
                !countdown_line_fits(intro_font_px(0), scale),
                "the control's own FontIndex 0 fits at scale {scale}? then the \
                 clipping this size exists for is gone and \
                 REMAIN_LINE2_LONGEST_EMS is wrong"
            );
            assert!(
                countdown_line_fits(REMAIN_LINE2_FONT_PX, scale),
                "the longest countdown line is clipped at scale {scale}: \
                 {} px of text in a {} px control",
                round_text_px(REMAIN_LINE2_FONT_PX, scale) * REMAIN_LINE2_LONGEST_EMS,
                REMAIN_W * scale
            );
        }
    }

    /// 8 px is not "small enough", it is the *largest* whole design pixel that
    /// fits — so the line stays as readable as it can be. A future edit that
    /// drops it further has to explain why; one that raises it breaks the test
    /// above.
    #[test]
    fn the_fitted_size_is_the_largest_whole_design_pixel_that_fits() {
        for scale in [1.0f32, 1.5, 2.0, 2.5, 3.0] {
            assert!(
                !countdown_line_fits(REMAIN_LINE2_FONT_PX + 1.0, scale),
                "one pixel larger also fits at scale {scale}, so this size is \
                 needlessly small"
            );
        }
        // And every ladder entry is larger than that, which is why no
        // `FontIndex` could have been used here instead of a fitted value.
        for index in 0..5 {
            assert!(intro_font_px(index) > REMAIN_LINE2_FONT_PX);
        }
    }

    /// ARGB like every resinfo COLOR: the leading 255 is alpha, not red.
    #[test]
    fn a_pml_font_colour_is_read_as_argb() {
        assert_eq!(
            parse_pml_color("font color=\"255,255,208,81\""),
            Some(Color::srgb_u8(255, 208, 81))
        );
        assert_eq!(parse_pml_color("font"), None);
    }

    /// The three `%d` of the countdown template are filled in order, and a
    /// template with more placeholders than values keeps the extras visible
    /// instead of eating them.
    #[test]
    fn the_countdown_fills_its_placeholders_in_order() {
        assert_eq!(
            fill_placeholders("[%d]day(s) [%d]hour(s) [%d]minute(s)", &[6, 23, 5]),
            "[6]day(s) [23]hour(s) [5]minute(s)"
        );
        assert_eq!(fill_placeholders("%d/%d", &[1]), "1/%d");
        assert_eq!(fill_placeholders("no fields", &[1]), "no fields");
    }

    /// `deletion_time` is in **minutes**: a just-deleted character carries
    /// `0x2760` = 10080 = 7*24*60.
    /// The old double reading — "divide by 60 if it looks too big" —
    /// is gone; what stays is the display clamp to the reservation the copy
    /// states, so a server with a longer policy cannot overflow the line or the
    /// gauge.
    #[test]
    fn the_countdown_reads_deletion_time_as_minutes_and_clamps() {
        // a fresh reservation
        assert_eq!(deletion_remaining(10080), (7, 0, 0));
        assert_eq!(deletion_remaining(90), (0, 1, 30));
        // a *seconds*-sized number is no longer silently re-scaled: 6 days in
        // seconds is above the reservation, so it clamps instead of reading 6d
        assert_eq!(deletion_remaining(6 * 24 * 3600), (7, 0, 0));
        assert_eq!(deletion_remaining(u32::MAX), (7, 0, 0));
        assert_eq!(deletion_remaining(0), (0, 0, 0));
        // one minute after the delete the line ticks to 6days 23hour 59min,
        // which the original prints as "6days 23hour"
        assert_eq!(deletion_remaining(10080 - 1), (6, 23, 59));
    }

    /// The gauge shows the reservation **already elapsed**, because the
    /// original's window is empty at `7days 0hour` and still empty at
    /// `6days 23hour`. The ticking countdown must keep it inside `[0, 1]` at both ends: a
    /// remainder above the reservation (a foreign server's policy) is an empty
    /// bar, an expired one a full bar — never a negative width.
    #[test]
    fn the_countdown_gauge_grows_as_the_reservation_elapses() {
        let full = DELETE_RESERVATION_MINUTES as f32;
        assert_eq!(gauge_elapsed(full), 0.0, "nothing elapsed -> empty bar");
        assert_eq!(gauge_elapsed(full * 2.0), 0.0);
        assert_eq!(gauge_elapsed(full / 2.0), 0.5);
        assert_eq!(gauge_elapsed(0.0), 1.0, "expired -> full bar");
        assert_eq!(gauge_elapsed(-1.0), 1.0);
    }

    /// The block look of the bar is the overlay art, not a drawn grid: the six
    /// opaque tick groups of `delete_time_gauge_up.ddj` sit at texels
    /// 38/78/118/158/198/238, and the original's bar shows bright columns at
    /// exactly those positions of the 280-px rect — seven blocks, one per
    /// reservation day. That is only
    /// possible at 1:1 texel scale, which is why both arts are placed at the
    /// authored 280x8 rect and the fill is *clipped* by a percentage wrapper
    /// instead of being stretched.
    #[test]
    fn the_gauge_rect_is_the_authored_one_so_the_ticks_land_on_the_bands() {
        assert_eq!(REMAIN_GAUGE, (24.0, 63.0, 280.0, 8.0));
        // seven 40-texel bands, six boundaries between them
        assert_eq!(REMAIN_GAUGE.2 / 40.0, 7.0);
        assert_eq!(DELETE_RESERVATION_MINUTES / (24 * 60), 7);
    }

    /// The defect this closes, in one sentence: the char-select pump used to
    /// answer a refusal with `error!` **only**, so the player saw an empty
    /// stage and nothing else. `0x0402` (1026) is an appending arm, so the
    /// rendered line must
    /// carry the original's `(S1026)` — text *and* suffix are asserted, because
    /// either half alone would let a regression through.
    #[test]
    fn a_refused_list_speaks_the_lobby_table_and_appends_the_original_code() {
        // A row transcribed in the shipped file's own wording, so the test
        // pins the *lookup* and not the hardcoded fallback literal.
        let strings = ClientUiStrings::from_tsv(
            "1\tUIO_MSG_ERROR_SEVER_CONNECT\t\t\t\t\t\t\tFailed to connect to server.\r\n",
        );
        assert_eq!(
            selection_action_error_line(&CharacterSelectionAction::List, 0x0402, &strings)
                .as_deref(),
            Some("Failed to connect to server.(S1026)")
        );
    }

    /// `0x0401` has its own arm that shows nothing at all. A pump that starts
    /// speaking must not start speaking *here* — otherwise the free-name answer
    /// of the original's own table turns into a false error line.
    #[test]
    fn the_silent_lobby_code_stays_silent_on_the_list_pump() {
        assert_eq!(
            selection_action_error_line(
                &CharacterSelectionAction::List,
                0x0401,
                &ClientUiStrings::default()
            ),
            None
        );
    }

    /// Seven of the fourteen arms do not append the code. `0x0410` is one of
    /// them, so no `(S…)` may appear — the suffix is table-driven, not global.
    #[test]
    fn a_plain_arm_reaches_the_list_pump_without_the_code_suffix() {
        let line = selection_action_error_line(
            &CharacterSelectionAction::List,
            0x0410,
            &ClientUiStrings::default(),
        )
        .expect("a plain arm still has to reach the player");
        assert_eq!(line, "This ID already exists.");
        assert!(!line.contains("(S"), "plain arm appended a code: {line}");
    }

    /// A code outside the table's range must neither panic nor vanish. The
    /// number stays visible and searchable (the original's default arm looks up
    /// the empty string and appends the code, so this *is* its wording).
    #[test]
    fn a_code_the_table_has_no_arm_for_still_reaches_the_player() {
        for code in [0x0419u16, 0x0400, 0xffff] {
            let line = selection_action_error_line(
                &CharacterSelectionAction::List,
                code,
                &ClientUiStrings::default(),
            )
            .unwrap_or_else(|| panic!("code {code:#06x} was swallowed"));
            assert!(
                line.contains(&code.to_string()),
                "code {code:#06x} rendered without its number: {line}"
            );
        }
    }

    /// A table that has *not* got the key (unloaded data, foreign client, a
    /// renamed row) falls back to the shipped English instead of showing a bare
    /// key or nothing — the universal-client half of this path.
    #[test]
    fn a_key_the_loaded_table_lacks_falls_back_to_the_shipped_english() {
        // Loaded, non-empty, and deliberately without UIO_MSG_ERROR_ID.
        let strings = ClientUiStrings::from_rows(&[("UIO_MSG_ERROR_SEVER_CONNECT", "irrelevant")]);
        assert_eq!(
            selection_action_error_line(&CharacterSelectionAction::List, 0x0410, &strings)
                .as_deref(),
            Some("This ID already exists.")
        );
    }

    /// Delete and Restore reach this pump too, but
    /// [`on_character_delete_response`] already renders them from the same
    /// message. Without this exclusion one refusal would post the identical
    /// line twice.
    #[test]
    fn delete_and_restore_refusals_are_not_spoken_twice() {
        let strings = ClientUiStrings::default();
        for action in [
            CharacterSelectionAction::Delete,
            CharacterSelectionAction::Restore,
        ] {
            assert_eq!(
                selection_action_error_line(&action, 0x0402, &strings),
                None,
                "{action:?} is spoken by on_character_delete_response"
            );
        }
        // ...while the actions that have no other speaker still are.
        assert!(
            selection_action_error_line(&CharacterSelectionAction::List, 0x0402, &strings)
                .is_some()
        );
    }
}
