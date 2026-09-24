//! `ifguildnotifywrite.txt` — the guild notice **write** modal, the one
//! control that turns the notice strip from a display into an editable field.
//!
//! Idea: the guild page's notice section (`guild.rs` §NotifySubBox) shows the
//! record's `notice` / `message` and its `GDR_GUILD_NOTIFY_EDIT_BTN` had
//! nowhere to lead, because the writing half is not part of `ifguild.txt` at
//! all: `ginterface.txt:911` declares `GDR_GUILD_NOTIFY_WRITE:CIFGuildNotifyWrite`
//! as its own `0,0,439,307` host on the shared `msgbox2_window_` plate, and its
//! contents live in `resinfo/ifguildnotifywrite.txt` (14 controls, transcribed
//! verbatim below). That is exactly the shape `letter_sub.rs` already has for
//! mail, so this module is that module's twin and borrows its plate ring
//! rather than growing a second one.
//!
//! Unlike mail's Send, this one **is** wired: `0x70F9 GuildNoticeEditRequest`
//! carries `{title, message}` with both fields named — no unknown slot — and its
//! ack `0xB0F9` is already read by `net::guild` (it is the one guild ack whose
//! *success* the original reports). So the chain is complete here: strip →
//! modal → `GuildAction::EditNotice` → 0x70F9 → 0xB0F9 → chat line.
//!
//! Two deliberate gaps, stated rather than invented:
//!
//! * The contents caption is `UIIT_MSG_GUILD_COMMON_KNOW_REMIND`, whose
//!   original text ends in `(%d/Max.%d)` — a typed/maximum counter. The
//!   maximum is **not** in the resinfo tree and nothing else names it, so the
//!   counter is dropped instead of printed with a guessed limit, and the
//!   fields carry no `max_characters`.
//! * The permission gate is client-side only and advisory: the server decides.
//!   It exists because `GuildPermissions::NOTICE` is already parsed per member
//!   and refusing locally is cheaper than a round trip that comes back as
//!   `UIIT_MSG_GUILDERR_PERMISSION_DENIED` anyway.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, TextCursorStyle};
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::{Activate, Button};

use crate::assets::FontAssets;
use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::hud::community::guild::{blacksquare_ring, BLACKSQUARE_DIR, NOTICE_OK_COLOR};
use crate::plugins::hud::community::letter_sub::plate_ring;
use crate::plugins::hud::community::model::CommunityState;
use crate::plugins::hud::game_window::abs_node;
use crate::plugins::hud::modal_dialog::MODAL_SCRIM;
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::net::character_info::CharacterInfo;
use crate::plugins::net::guild::{GuildAction, GuildRoster};
use crate::plugins::player::Player;
use crate::plugins::textdata::ClientUiStrings;

// --- Layout (ginterface.txt §GuildNotifyWrite + ifguildnotifywrite.txt) ------

/// `GDR_GUILD_NOTIFY_WRITE:CIFGuildNotifyWrite` (`ginterface.txt:913`,
/// `Rect="0,0,439,307"`, `DDJ="interface\messagebox\msgbox2_window_"`) — the
/// same modal plate the mail sub-windows use, 36 units taller.
const PLATE_SIZE: (f32, f32) = (439.0, 307.0);
const PLATE_DIR: &str = "media://interface/messagebox/msgbox2_window_";

/// `GDR_GUILD_NOTIFY_WRITE_TITLE:CIFStatic` (`:198`, `125,11,188,13`) — the
/// caption inside the plate's own 40px top strip.
const TITLE_RECT: (f32, f32, f32, f32) = (125.0, 11.0, 188.0, 13.0);
/// `_BG1/_BG2/_BG3:CIFNormalTile`, `com_bg_tile_b.ddj` (`:236,:255,:217`).
const BG1_RECT: (f32, f32, f32, f32) = (16.0, 40.0, 407.0, 28.0);
const BG2_RECT: (f32, f32, f32, f32) = (16.0, 93.0, 407.0, 42.0);
const BG3_RECT: (f32, f32, f32, f32) = (16.0, 260.0, 407.0, 31.0);
const BG_TILE_B: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_b.ddj";
const BG_TILE_E: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_e.ddj";
/// `_STA_SUBJECT:CIFStatic` (`:179`, `125,49,188,13`).
const STA_SUBJECT_RECT: (f32, f32, f32, f32) = (125.0, 49.0, 188.0, 13.0);
/// `_SUBJECT_BG:CIFNormalTile` (`:102`, `16,72,407,17`, `com_bg_tile_e.ddj`).
const SUBJECT_BG_RECT: (f32, f32, f32, f32) = (16.0, 72.0, 407.0, 17.0);
/// `_SUBJECT:CIFEdit` (`:64`, `19,70,400,20`) — the **title** half of 0x70F9.
const SUBJECT_RECT: (f32, f32, f32, f32) = (19.0, 70.0, 400.0, 20.0);
/// `_STA_CONTENTS:CIFStatic` (`:160`, `55,115,329,13`).
const STA_CONTENTS_RECT: (f32, f32, f32, f32) = (55.0, 115.0, 329.0, 13.0);
/// `_CONTENTS_BLACKSQUARE:CIFStretchWnd` (`:121`, `15,135,410,125`).
const CONTENTS_PLATE_RECT: (f32, f32, f32, f32) = (15.0, 135.0, 410.0, 125.0);
/// `_CONTENTS_BG:CIFNormalTile` (`:83`, `19,139,402,117`) and `_CONTENTS:CIFEdit`
/// (`:44`) — byte-identical rects, fill and field.
const CONTENTS_RECT: (f32, f32, f32, f32) = (19.0, 139.0, 402.0, 117.0);
/// `_OK_BTN` (`:25`, `140,270,0,0`) and `_CANCEL_BTN` (`:6`, `228,270,0,0`) —
/// `Rect` w,h = `0,0`, so the size is the art's: `com_button.ddj` is 76x24,
/// the same size `letter_sub.rs` states. The 88-unit pitch is the
/// authored one.
const BUTTON_Y: f32 = 270.0;
const BUTTON_SIZE: (f32, f32) = (76.0, 24.0);
const OK_X: f32 = 140.0;
const CANCEL_X: f32 = 228.0;
const BUTTON_DDJ: &str = "media://interface/ifcommon/com_button.ddj";

/// `FontColor=255,239,218,164` on both statics (`:164`, `:183`).
const CAPTION_COLOR: Color = Color::srgb(239.0 / 255.0, 218.0 / 255.0, 164.0 / 255.0);

/// "You are not authorized." — the guild family's own refusal string, the same
/// one guild storage uses for its client-side gate
/// (`hud/guild_storage/model.rs`), and the message the server's own
/// `0x4C1E`/`0x4C52` codes map to.
const DENIED_KEY: &str = "UIIT_MSG_GUILDERR_PERMISSION_DENIED";
const DENIED_FALLBACK: &str = "You are not authorized.";

// --- State ------------------------------------------------------------------

/// Is the write modal up? A resource rather than a field on `CommunityState`
/// because the modal outlives no page state and the community shell already
/// owns one such flag per sub-window (`CommunityState::letter_sub`) — this one
/// belongs to the guild page alone.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub struct GuildNoticeWrite(pub bool);

/// Root of the modal (scrim + plate).
#[derive(Component)]
pub struct GuildNoticeWriteRoot;

/// The two editable fields, in the order 0x70F9 writes them.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum GuildNoticeWriteField {
    /// `GDR_GUILD_NOTIFY_WRITE_SUBJECT` → `GuildNoticeEditRequest::title`.
    Subject,
    /// `GDR_GUILD_NOTIFY_WRITE_CONTENTS` → `GuildNoticeEditRequest::message`.
    Contents,
}

/// The modal's two buttons.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum GuildNoticeWriteButton {
    /// `_OK_BTN`, `UIIT_CTL_GUILD_WRITE_END` ("Completed") — sends 0x70F9.
    Ok,
    /// `_CANCEL_BTN`, `UIIT_CTL_CANCEL`.
    Cancel,
}

/// The edit button on the notice strip: open the modal if this member may edit
/// the notice, and say so in the chat if not.
///
/// Name-matched against the roster because the guild record carries no "this
/// is you" marker — the same lookup, for the same reason, that guild storage
/// makes before it asks the server to open (`GuildRoster::permissions_for`).
pub fn on_notice_edit(
    _: On<Activate>,
    roster: Res<GuildRoster>,
    players: Query<&CharacterInfo, With<Player>>,
    ui_strings: Res<ClientUiStrings>,
    mut history: ResMut<ChatHistory>,
    mut write: ResMut<GuildNoticeWrite>,
) {
    let name = players.single().ok().and_then(|info| info.name.clone());
    let permitted = name
        .as_deref()
        .and_then(|n| roster.permissions_for(n))
        .is_some_and(|p| p.can_edit_notice());
    if !permitted {
        history.push(ChatLine::system(
            ui_strings.get_or(DENIED_KEY, DENIED_FALLBACK),
        ));
        return;
    }
    write.0 = true;
}

/// Spawn / despawn the modal when [`GuildNoticeWrite`] changes — the shape
/// `apply_letter_sub_window` established, for the same reason: one reactive
/// system beats a hidden tree that has to be kept in sync with the record.
pub fn apply_guild_notice_write(
    write: Res<GuildNoticeWrite>,
    state: Res<CommunityState>,
    roster: Res<GuildRoster>,
    existing: Query<Entity, With<GuildNoticeWriteRoot>>,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut commands: Commands,
) {
    if !write.is_changed() && !state.is_changed() {
        return;
    }
    for entity in existing.iter() {
        commands.entity(entity).despawn();
    }
    // The modal belongs to the guild page: closing the community window has to
    // take it with it rather than leave a dialog floating over the game.
    if !write.0 || !state.open {
        return;
    }
    let Ok(camera) = cam_query.single() else {
        warn!("guild notice write: no 2d camera to attach to");
        return;
    };
    let s = hud_scale();
    let text_font = |size: f32| TextFont {
        font: fonts.two.clone().into(),
        font_size: FontSize::Px(size * s),
        ..default()
    };
    let image = |rect: (f32, f32, f32, f32), path: String| {
        (
            abs_node(rect, s),
            ImageNode {
                image: asset_server.load(path),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Pickable::IGNORE,
        )
    };
    let tile = |rect: (f32, f32, f32, f32), path: &'static str| {
        (
            abs_node(rect, s),
            ImageNode {
                image: asset_server.load(path),
                image_mode: NodeImageMode::Tiled {
                    tile_x: true,
                    tile_y: true,
                    stretch_value: s,
                },
                ..default()
            },
            Pickable::IGNORE,
        )
    };
    let (title, message) = match roster.data.as_ref() {
        Some(data) => (data.notice.clone(), data.message.clone()),
        None => (String::new(), String::new()),
    };

    commands
        .spawn((
            GuildNoticeWriteRoot,
            Name::from("Guild Notice Write"),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(MODAL_SCRIM),
            // Above the community shell's own GlobalZIndex(55), like the mail
            // sub-windows this modal is a sibling of.
            GlobalZIndex(60),
            UiTargetCamera(camera),
        ))
        .with_children(|scrim| {
            scrim
                .spawn((
                    Node {
                        width: Val::Px(PLATE_SIZE.0 * s),
                        height: Val::Px(PLATE_SIZE.1 * s),
                        ..default()
                    },
                    Pickable::default(),
                ))
                .with_children(|plate| {
                    for ((x, y, w, h), piece) in plate_ring(PLATE_SIZE.0, PLATE_SIZE.1) {
                        plate.spawn(image((x, y, w, h), format!("{PLATE_DIR}{piece}.ddj")));
                    }
                    for rect in [BG1_RECT, BG2_RECT, BG3_RECT] {
                        plate.spawn(tile(rect, BG_TILE_B));
                    }
                    for rect in [SUBJECT_BG_RECT, CONTENTS_RECT] {
                        plate.spawn(tile(rect, BG_TILE_E));
                    }
                    // `_CONTENTS_BLACKSQUARE` — the body field's trim.
                    let (px, py, pw, ph) = CONTENTS_PLATE_RECT;
                    for ((x, y, w, h), piece) in blacksquare_ring(pw, ph) {
                        plate.spawn(image(
                            (px + x, py + y, w, h),
                            format!("{BLACKSQUARE_DIR}{piece}.ddj"),
                        ));
                    }

                    for (rect, key, fallback) in [
                        (
                            TITLE_RECT,
                            "UIIT_STT_GUILD_COMMON_KNOW_WRITE",
                            "Writing guild notice",
                        ),
                        (
                            STA_SUBJECT_RECT,
                            "UIIT_MSG_GUILD_COMMON_KNOW_TITLE_REMIND",
                            "Please enter the title of the note.",
                        ),
                        (
                            STA_CONTENTS_RECT,
                            "UIIT_MSG_GUILD_COMMON_KNOW_REMIND",
                            "Please enter the contents of the note.",
                        ),
                    ] {
                        plate.spawn((
                            Text::new(caption(ui_strings.get_or(key, fallback))),
                            text_font(7.5),
                            TextColor(CAPTION_COLOR),
                            TextLayout::justify(Justify::Center),
                            abs_node(rect, s),
                            Pickable::IGNORE,
                        ));
                    }

                    spawn_field(
                        plate,
                        GuildNoticeWriteField::Subject,
                        SUBJECT_RECT,
                        title,
                        true,
                        s,
                        text_font(7.5),
                    );
                    spawn_field(
                        plate,
                        GuildNoticeWriteField::Contents,
                        CONTENTS_RECT,
                        message,
                        false,
                        s,
                        text_font(7.5),
                    );

                    for (button, x, key, fallback) in [
                        (
                            GuildNoticeWriteButton::Ok,
                            OK_X,
                            "UIIT_CTL_GUILD_WRITE_END",
                            "Completed",
                        ),
                        (
                            GuildNoticeWriteButton::Cancel,
                            CANCEL_X,
                            "UIIT_CTL_CANCEL",
                            "Cancel",
                        ),
                    ] {
                        plate
                            .spawn((
                                button,
                                Button,
                                Hovered::default(),
                                abs_node((x, BUTTON_Y, BUTTON_SIZE.0, BUTTON_SIZE.1), s),
                                ImageNode {
                                    image: asset_server.load(BUTTON_DDJ),
                                    image_mode: NodeImageMode::Stretch,
                                    ..default()
                                },
                            ))
                            .observe(on_notice_write_button)
                            .with_children(|b| {
                                b.spawn((
                                    Text::new(ui_strings.get_or(key, fallback).to_string()),
                                    text_font(7.5),
                                    TextColor(NOTICE_OK_COLOR),
                                    TextLayout::justify(Justify::Center),
                                    Node {
                                        position_type: PositionType::Absolute,
                                        top: Val::Px(6.0 * s),
                                        width: Val::Percent(100.0),
                                        ..default()
                                    },
                                    Pickable::IGNORE,
                                ));
                            });
                    }
                });
        });
}

/// The original's contents caption carries a `(%d/Max.%d)` counter whose
/// maximum is in neither the resinfo tree nor the wire. Printing the raw
/// format string would show `%d` to the player and inventing a limit would be
/// a magic number, so the counter is cut and the sentence kept.
fn caption(text: &str) -> String {
    match text.find("(%d") {
        Some(cut) => text[..cut].trim_end().to_string(),
        None => text.to_string(),
    }
}

/// One `CIFEdit` of the modal, pre-filled with what the record currently says —
/// the original opens the writer on the standing notice, and an empty box would
/// make every edit a rewrite.
fn spawn_field(
    plate: &mut ChildSpawnerCommands,
    marker: GuildNoticeWriteField,
    rect: (f32, f32, f32, f32),
    initial: String,
    single_line: bool,
    s: f32,
    font: TextFont,
) {
    let mut node = abs_node(rect, s);
    node.padding = UiRect::all(Val::Px(2.0 * s));
    let mut editable = EditableText::new(initial);
    editable.visible_lines = if single_line { Some(1.0) } else { None };
    editable.allow_newlines = !single_line;
    plate.spawn((
        marker,
        editable,
        node,
        font,
        TextColor(Color::WHITE),
        TextLayout::justify(Justify::Left),
        TextCursorStyle {
            color: Color::WHITE,
            ..default()
        },
    ));
}

/// Completed sends 0x70F9 with what is in the two fields; Cancel drops it.
fn on_notice_write_button(
    activate: On<Activate>,
    buttons: Query<&GuildNoticeWriteButton>,
    fields: Query<(&GuildNoticeWriteField, &EditableText)>,
    mut write: ResMut<GuildNoticeWrite>,
    mut actions: MessageWriter<GuildAction>,
) {
    let Ok(button) = buttons.get(activate.entity) else {
        return;
    };
    if *button == GuildNoticeWriteButton::Ok {
        actions.write(notice_edit_action(&fields));
    }
    write.0 = false;
}

/// The two fields, read in the request's own order. Split out so the mapping
/// is testable without a window: a swap here would put the body in the strip
/// on every other client in the guild.
fn notice_edit_action(fields: &Query<(&GuildNoticeWriteField, &EditableText)>) -> GuildAction {
    let mut title = String::new();
    let mut message = String::new();
    for (field, editable) in fields.iter() {
        match field {
            GuildNoticeWriteField::Subject => title = editable.value().to_string(),
            GuildNoticeWriteField::Contents => message = editable.value().to_string(),
        }
    }
    GuildAction::EditNotice { title, message }
}

/// Leaving the world takes the modal with it — the flag is a resource, so
/// without this the next entry into the world would rebuild a dialog nobody
/// opened.
pub fn cleanup_guild_notice_write(
    existing: Query<Entity, With<GuildNoticeWriteRoot>>,
    mut write: ResMut<GuildNoticeWrite>,
    mut commands: Commands,
) {
    for entity in existing.iter() {
        commands.entity(entity).despawn();
    }
    write.0 = false;
}

#[cfg(test)]
mod test {
    use super::*;

    /// Every transcribed rect stays inside the `0,0,439,307` host
    /// `ginterface.txt` declares — a rect past it means a bad transcription.
    #[test]
    fn every_rect_fits_the_authored_host() {
        for (x, y, w, h) in [
            TITLE_RECT,
            BG1_RECT,
            BG2_RECT,
            BG3_RECT,
            STA_SUBJECT_RECT,
            SUBJECT_BG_RECT,
            SUBJECT_RECT,
            STA_CONTENTS_RECT,
            CONTENTS_PLATE_RECT,
            CONTENTS_RECT,
            (OK_X, BUTTON_Y, BUTTON_SIZE.0, BUTTON_SIZE.1),
            (CANCEL_X, BUTTON_Y, BUTTON_SIZE.0, BUTTON_SIZE.1),
        ] {
            assert!(
                x + w <= PLATE_SIZE.0,
                "rect {x},{y},{w},{h} overflows the host width"
            );
            assert!(
                y + h <= PLATE_SIZE.1,
                "rect {x},{y},{w},{h} overflows the host height"
            );
        }
    }

    /// The buttons keep the family's authored 88-unit pitch and the art's own
    /// 76x24 — the same two numbers `letter_sub.rs` pins for `com_button.ddj`.
    #[test]
    fn the_button_row_keeps_the_authored_pitch() {
        assert_eq!(CANCEL_X - OK_X, 88.0);
        assert_eq!(BUTTON_SIZE, (76.0, 24.0), "com_button.ddj is 76x24");
    }

    /// The `(%d/Max.%d)` tail is dropped rather than printed or filled with a
    /// guessed limit; a caption without one is passed through untouched.
    #[test]
    fn the_unknown_character_limit_is_cut_not_invented() {
        assert_eq!(
            caption("Please enter the contents of the note.(%d/Max.%d)"),
            "Please enter the contents of the note."
        );
        assert_eq!(caption("Writing guild notice"), "Writing guild notice");
    }

    /// The whole chain in one app: two fields with text, a click on Completed,
    /// and a `GuildAction::EditNotice` carrying them in the request's order.
    #[test]
    fn completed_sends_the_two_fields_in_the_requests_order() {
        let mut app = App::new();
        app.add_message::<GuildAction>()
            .init_resource::<GuildNoticeWrite>();
        app.world_mut().spawn((
            GuildNoticeWriteField::Contents,
            EditableText::new("Tonight, 8pm."),
        ));
        app.world_mut()
            .spawn((GuildNoticeWriteField::Subject, EditableText::new("Raid")));
        app.add_systems(
            Update,
            |fields: Query<(&GuildNoticeWriteField, &EditableText)>,
             mut actions: MessageWriter<GuildAction>| {
                actions.write(notice_edit_action(&fields));
            },
        );
        app.update();

        let messages = app.world().resource::<Messages<GuildAction>>();
        let sent: Vec<GuildAction> = messages.iter_current_update_messages().cloned().collect();
        assert_eq!(
            sent,
            vec![GuildAction::EditNotice {
                title: "Raid".into(),
                message: "Tonight, 8pm.".into(),
            }],
            "the strip's subject is the title, the pane's body is the message"
        );
    }

    /// The start trap: this modal asks for
    /// `FontAssets`, which does not exist in the loading screen, so its
    /// registration must carry the world-scene gate and a cleanup on leaving
    /// it. `cargo test` cannot observe that, so it is asserted on the
    /// registration text itself — the way `hud/job/ranking.rs` does.
    #[test]
    fn the_modal_is_gated_on_the_world_scene() {
        let source = include_str!("mod.rs");
        assert!(
            source.contains("apply_guild_notice_write"),
            "the modal is never registered"
        );
        assert!(
            source.contains("in_state(SceneState::GameWorld)"),
            "community/mod.rs registers an Update system without the scene gate"
        );
        assert!(
            source.contains("OnExit(SceneState::GameWorld)"),
            "community/mod.rs leaves its windows up when the world scene ends"
        );
        assert!(
            source.contains("cleanup_guild_notice_write"),
            "the modal survives leaving the world"
        );
    }
}
