//! The Friend page — page 13 of the Community window (`GDR_COMMUNITY_FRIEND`).
//!
//! Idea: the page is transcribed from `resinfo/iffriend.txt`
//! (three sections: §Create, §FriendList, §CommandButton — 12 controls) and its
//! row prototype `resinfo/iffriendslot.txt` (3 controls). Every rect const
//! carries the `iffriend.txt:NNN` line it came from, so no number here was
//! chosen; where a `Rect` has `w,h = 0,0` the original takes the extent from
//! the art, and the size below is that `.ddj`'s own DDS extent.
//!
//! Rects are page-local: the six community pages share `ifcommunity.txt`'s
//! `13,61,451,320`, which `community/ui.rs` already spawns — the same frame
//! `letter.rs` and `guild.rs` draw into.
//!
//! **What the page shows is the wire, and the wire is thin.** The only friend
//! opcode with a documented layout is the join-time roster push 0x3305, kept by
//! `net::friend::FriendRoster`; the entry's four fields come from the original's
//! own parser, and every 0x3305 seen so far is an *empty* roster. So:
//!
//! * the row renders name + presence dot, and nothing else. `char_model` is the
//!   trailing `u32` whose meaning is open — a ref-object id is the obvious
//!   reading, but the ref-id → race mapping does not exist client-side either,
//!   so the race cell keeps `iffriendslot.txt`'s own placeholder art instead of
//!   being derived from a field we have not established.
//! * **Whisper is live** (slot 1): it is pure client behaviour — the row's name
//!   into the chat input, through the one shared helper the target menu and the
//!   whisper panel already use.
//! * **Add friend / Delete friend are not sent** (slots 2/3). Their opcodes are
//!   known by name only — 0x7302 / 0x7304, from third-party opcode maps — and
//!   **no body of either is established**: neither opcode is even in the
//!   `packets!` table. Guessing "u16 length + name" would be exactly the
//!   unsourced magic number ADR-0009 forbids. So the buttons answer the player
//!   in chat, naming what is missing.
//!
//! No scrolling: `GDR_FRIEND_LIST_SCROLLMGR`'s pooled rows and its vertical
//! scrollbar are the shared widget of #56, so the page draws the rows the
//! manager's own height implies and a longer roster shows its first eleven —
//! stated rather than silently truncated, like `guild.rs` does with six.

use bevy::input_focus::InputFocus;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui_widgets::{Activate, Button};

use crate::assets::FontAssets;
use crate::plugins::hud::chat::input::prefill_whisper_input;
use crate::plugins::hud::chat::model::{ChatHistory, ChatLine, ChatState};
use crate::plugins::hud::chat::ui::ChatInputBox;
use crate::plugins::hud::game_window::abs_node;
use crate::plugins::net::friend::FriendRoster;
use crate::plugins::textdata::ClientUiStrings;

// --- §Create — page chrome, `iffriend.txt:4-102` (5 controls) ---------------

/// `GDR_FRIEND_FRAME:CIFFrame` (`:83`, Rect `6,6,440,308`) — the same
/// `frameg01_wnd_` skeleton the letter and guild pages use, 16 px pieces.
const FRAME_RECT: (f32, f32, f32, f32) = (6.0, 6.0, 440.0, 308.0);
const FRAME_PIECE: f32 = 16.0;
const FRAME_DIR: &str = "media://interface/frame/frameg01_wnd_";
/// `GDR_FRIEND_BLACKSQUARE:CIFStretchWnd` (`:44`, Rect `14,18,333,282`) — the
/// list plate; `com_blacksquare_` is six 4 px trim pieces around flat black.
const LIST_PLATE_RECT: (f32, f32, f32, f32) = (14.0, 18.0, 333.0, 282.0);
const BLACKSQUARE_PIECE: f32 = 4.0;
const BLACKSQUARE_DIR: &str = "media://interface/ifcommon/com_blacksquare_";
/// `GDR_FRIEND_BG_01:CIFNormalTile` (`:64`, Rect `328,22,102,276`) — the tile
/// behind the command column.
const COMMAND_BG_RECT: (f32, f32, f32, f32) = (328.0, 22.0, 102.0, 276.0);
const BG_TILE_DDJ: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_b.ddj";
/// `GDR_FRIEND_NUMBER_STA` (`:25`, Rect `353,27,33,15`, Text `UIIT_CTL_FRIEND`,
/// FontColor `255,239,218,164`) and `GDR_FRIEND_NUMBER` (`:6`, Rect
/// `380,27,40,15`, FontColor white) — the "Friend  n" readout.
const COUNT_LABEL_RECT: (f32, f32, f32, f32) = (353.0, 27.0, 33.0, 15.0);
const COUNT_VALUE_RECT: (f32, f32, f32, f32) = (380.0, 27.0, 40.0, 15.0);

// --- §FriendList — the list, `iffriend.txt:104-182` (4 controls) ------------

/// `GDR_FRIEND_LIST_SCROLLMGR:CIFScrollManager` (`:106`, Rect `17,43,328,256`).
const LIST_RECT: (f32, f32, f32, f32) = (17.0, 43.0, 328.0, 256.0);
/// `GDR_FRIEND_LIST_BTN_SHAPE_BEGIN:CIFStatic` (`:163`, Rect `17,21,0,0`) —
/// art-sized `gil_subj_button04.ddj`, 52x24, the header's left cap.
const HEADER_CAP_LEFT: (f32, f32, f32, f32) = (17.0, 21.0, 52.0, 24.0);
const HEADER_CAP_LEFT_DDJ: &str = "media://interface/guild/gil_subj_button04.ddj";
/// `GDR_FRIEND_LIST_SORT_NAME_BTN:CIFButton` (`:144`, Rect `67,21,0,0`,
/// HAlign 1, Text `UIIT_STT_FRIENDLIST`) — art-sized `gil_subj_button09.ddj`,
/// 264x24. Presentational: sorting the roster is not on the wire (a
/// 0x3305 push is the server's own order and there is no re-sort request).
const HEADER_SORT: (f32, f32, f32, f32) = (67.0, 21.0, 264.0, 24.0);
const HEADER_SORT_DDJ: &str = "media://interface/guild/gil_subj_button09.ddj";
/// `GDR_FRIEND_LIST_BTN_SHAPE_END:CIFStatic` (`:125`, Rect `329,21,0,0`) —
/// art-sized `gil_shape.ddj`, 16x24, the right cap.
///
/// The strip's x-run overlaps on purpose, exactly as `ifguild.txt`'s does:
/// `17+52 = 69` vs the sort plate's 67 (−2), `67+264 = 331` vs the cap's 329
/// (−2). Those seams are how vanilla butts header plates together, so they are
/// reproduced rather than "corrected" to a clean tiling.
const HEADER_CAP_RIGHT: (f32, f32, f32, f32) = (329.0, 21.0, 16.0, 24.0);
const HEADER_CAP_RIGHT_DDJ: &str = "media://interface/guild/gil_shape.ddj";

/// Row pitch and height. The height is the row art's own extent
/// (`com_bar01_left/mid/right.ddj`, 4x24 / 24x24 / 4x24), and the pitch-23 /
/// height-24 law is the one `guild.rs` sources from this same file family.
const ROW_PITCH: f32 = 23.0;
const ROW_HEIGHT: f32 = 24.0;
/// `256 = 11·23 + 3` ⇒ eleven visible rows fit the manager's height.
const VISIBLE_ROWS: usize = 11;
/// The row plate: `com_bar01_` normally, `com_bar01select_` for the selected
/// row. The pair is the original's own selected/unselected art (same 4/24/4
/// piece widths), which is why selection is drawn by swapping it rather than by
/// an invented tint overlay.
const ROW_BAR_DIR: &str = "media://interface/ifcommon/com_bar01_";
const ROW_BAR_SELECTED_DIR: &str = "media://interface/ifcommon/com_bar01select_";
const ROW_BAR_CAP: f32 = 4.0;

// --- The row prototype, `iffriendslot.txt` (3), slot-local ------------------
//
// Slot-local like `ifguildmemberslot.txt`: the slot files of this generation
// carry cells only (no plate, no origin), and the manager places them. Their
// rects are therefore offsets into the row, not page coordinates.

/// `GDR_FRIEND_SLOT_CONTACT:CIFStatic` (`:44`, Rect `20,6,0,0`) — art-sized
/// `gil_contact_off.ddj`, 12x16; `_on` is the same size. The one cell
/// whose art carries state.
const SLOT_CONTACT: (f32, f32, f32, f32) = (20.0, 6.0, 12.0, 16.0);
const SLOT_CONTACT_OFF_DDJ: &str = "media://interface/guild/gil_contact_off.ddj";
const SLOT_CONTACT_ON_DDJ: &str = "media://interface/guild/gil_contact_on.ddj";
/// `GDR_FRIEND_SLOT_RACE_MARK:CIFStatic` (`:25`, Rect `73,4,16,16`) — the file
/// names `com_kindred_china16.ddj`, which is the *placeholder*: the original
/// swaps it for the friend's race at runtime. We draw the same placeholder,
/// because the entry's only candidate field for it (`char_model`) is open and
/// no ref-id → race mapping exists client-side.
const SLOT_RACE_MARK: (f32, f32, f32, f32) = (73.0, 4.0, 16.0, 16.0);
const SLOT_RACE_MARK_DDJ: &str = "media://interface/ifcommon/com_kindred_china16.ddj";
/// `GDR_FRIEND_SLOT_NAME:CIFStatic` (`:6`, Rect `95,7,89,16`, HAlign 0).
const SLOT_NAME: (f32, f32, f32, f32) = (95.0, 7.0, 89.0, 16.0);

// --- §CommandButton — the action column, `iffriend.txt:184-243` (3) ---------

/// `GDR_FRIEND_COMMAND_BUTTON_1..3` (`:224`, `:205`, `:186`), Rect
/// `352,55|84|113,0,0` — art-sized `com_mid_button.ddj`, 88x24, at the
/// authored 29-unit pitch. FontColor `255,255,245,218`, `ClientRect` y-offset 6.
const COMMAND_X: f32 = 352.0;
const COMMAND_YS: [f32; 3] = [55.0, 84.0, 113.0];
const COMMAND_BTN_SIZE: (f32, f32) = (88.0, 24.0);
const COMMAND_BTN_DDJ: &str = "media://interface/ifcommon/com_mid_button.ddj";
const COMMAND_TEXT_COLOR: Color = Color::srgb(1.0, 245.0 / 255.0, 218.0 / 255.0);

/// `FontColor=255,239,218,164` — the count label.
const LABEL_COLOR: Color = Color::srgb(239.0 / 255.0, 218.0 / 255.0, 164.0 / 255.0);
/// `FontColor=255,255,255,255` — the count value and the row names.
const VALUE_COLOR: Color = Color::srgb(1.0, 1.0, 1.0);

/// The three commands of §CommandButton, in button order 1..3.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FriendCommand {
    /// `UIIT_STT_GET_WHISPER` — live, client-side only.
    Whisper,
    /// `UIIT_CTL_FRIENDADD` — 0x7302, body not established.
    Add,
    /// `UIIT_CTL_FRIENDDEL` — 0x7304, body not established.
    Delete,
}

impl FriendCommand {
    /// The `Text` key `iffriend.txt` gives the button, and its shipped English.
    fn label(self) -> (&'static str, &'static str) {
        match self {
            FriendCommand::Whisper => ("UIIT_STT_GET_WHISPER", "Whisper"),
            FriendCommand::Add => ("UIIT_CTL_FRIENDADD", "Add friend"),
            FriendCommand::Delete => ("UIIT_CTL_FRIENDDEL", "Delete friend"),
        }
    }
}

/// Which list row the player last clicked. The command column acts on it — the
/// original's own model (the three buttons sit beside a single-selection list).
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub struct FriendSelection(pub Option<usize>);

/// A clickable row of the list, by index into the roster.
#[derive(Component, Clone, Copy)]
pub struct FriendRow(pub usize);

/// One of a row's three `com_bar01_` plate pieces.
#[derive(Component, Clone, Copy)]
pub struct FriendRowBar {
    pub row: usize,
    pub piece: &'static str,
}

/// A row's name cell.
#[derive(Component, Clone, Copy)]
pub struct FriendRowName(pub usize);

/// A row's presence dot — the only cell whose art changes with the record.
#[derive(Component, Clone, Copy)]
pub struct FriendRowOnline(pub usize);

/// A row's race placeholder; hidden past the roster's end so an empty list
/// does not show eleven marks for nobody.
#[derive(Component, Clone, Copy)]
pub struct FriendRowRace(pub usize);

/// The `GDR_FRIEND_NUMBER` readout.
#[derive(Component)]
pub struct FriendCountText;

// --- Spawning ---------------------------------------------------------------

/// Fill the Friend page container with the list page.
pub fn spawn_friend_page(
    page: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
    s: f32,
) {
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

    // GDR_FRIEND_FRAME
    let (fx, fy, fw, fh) = FRAME_RECT;
    for ((x, y, w, h), piece) in ring(fw, fh, FRAME_PIECE) {
        page.spawn(image(
            (fx + x, fy + y, w, h),
            format!("{FRAME_DIR}{piece}.ddj"),
        ));
    }

    // GDR_FRIEND_BG_01 — the command column's tile.
    page.spawn((
        abs_node(COMMAND_BG_RECT, s),
        ImageNode {
            image: asset_server.load(BG_TILE_DDJ),
            image_mode: NodeImageMode::Tiled {
                tile_x: true,
                tile_y: true,
                stretch_value: s,
            },
            ..default()
        },
        Pickable::IGNORE,
    ));

    // GDR_FRIEND_BLACKSQUARE — flat black plate plus its 4px trim.
    let (bx, by, bw, bh) = LIST_PLATE_RECT;
    page.spawn((
        abs_node(LIST_PLATE_RECT, s),
        BackgroundColor(Color::BLACK),
        Pickable::IGNORE,
    ));
    for ((x, y, w, h), piece) in blacksquare_ring(bw, bh) {
        page.spawn(image(
            (bx + x, by + y, w, h),
            format!("{BLACKSQUARE_DIR}{piece}.ddj"),
        ));
    }

    // The header strip: left cap, the sort plate with its caption, right cap.
    page.spawn(image(HEADER_CAP_LEFT, HEADER_CAP_LEFT_DDJ.to_string()));
    page.spawn(image(HEADER_SORT, HEADER_SORT_DDJ.to_string()))
        .with_children(|header| {
            header.spawn((
                Text::new(
                    ui_strings
                        .get_or("UIIT_STT_FRIENDLIST", "Friends list")
                        .to_string(),
                ),
                text_font(7.5),
                TextColor(VALUE_COLOR),
                TextLayout::justify(Justify::Center),
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(5.0 * s),
                    width: Val::Percent(100.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        });
    page.spawn(image(HEADER_CAP_RIGHT, HEADER_CAP_RIGHT_DDJ.to_string()));

    // The "Friend  n" readout.
    page.spawn((
        Text::new(ui_strings.get_or("UIIT_CTL_FRIEND", "Friend").to_string()),
        text_font(7.5),
        TextColor(LABEL_COLOR),
        TextLayout::justify(Justify::Left),
        abs_node(COUNT_LABEL_RECT, s),
        Pickable::IGNORE,
    ));
    page.spawn((
        FriendCountText,
        Text::new("0".to_string()),
        text_font(7.5),
        TextColor(VALUE_COLOR),
        TextLayout::justify(Justify::Left),
        abs_node(COUNT_VALUE_RECT, s),
        Pickable::IGNORE,
    ));

    // The eleven visible rows: plate, cells, and the pick target over them.
    let (lx, ly, lw, _) = LIST_RECT;
    for row in 0..VISIBLE_ROWS {
        let top = ly + row as f32 * ROW_PITCH;
        for ((x, y, w, h), piece) in bar((lx, top, lw, ROW_HEIGHT), ROW_BAR_CAP) {
            page.spawn((
                FriendRowBar { row, piece },
                {
                    let mut node = abs_node((x, y, w, h), s);
                    node.display = Display::None;
                    node
                },
                ImageNode {
                    image: asset_server.load(format!("{ROW_BAR_DIR}{piece}.ddj")),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }
        page.spawn((
            FriendRowOnline(row),
            hidden_node(
                (
                    lx + SLOT_CONTACT.0,
                    top + SLOT_CONTACT.1,
                    SLOT_CONTACT.2,
                    SLOT_CONTACT.3,
                ),
                s,
            ),
            ImageNode {
                image: asset_server.load(SLOT_CONTACT_OFF_DDJ),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Pickable::IGNORE,
        ));
        page.spawn((
            FriendRowRace(row),
            hidden_node(
                (
                    lx + SLOT_RACE_MARK.0,
                    top + SLOT_RACE_MARK.1,
                    SLOT_RACE_MARK.2,
                    SLOT_RACE_MARK.3,
                ),
                s,
            ),
            ImageNode {
                image: asset_server.load(SLOT_RACE_MARK_DDJ),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Pickable::IGNORE,
        ));
        page.spawn((
            FriendRowName(row),
            Text::new(String::new()),
            text_font(7.5),
            TextColor(VALUE_COLOR),
            TextLayout::justify(Justify::Left),
            abs_node(
                (
                    lx + SLOT_NAME.0,
                    top + SLOT_NAME.1,
                    SLOT_NAME.2,
                    SLOT_NAME.3,
                ),
                s,
            ),
            Pickable::IGNORE,
        ));
        page.spawn((
            FriendRow(row),
            Hovered::default(),
            abs_node((lx, top, lw, ROW_HEIGHT), s),
            Pickable::default(),
        ))
        .observe(on_friend_row_press);
    }

    // §CommandButton — all three are buttons; two of them answer that they
    // cannot send (see the module doc), which is still a reaction the player
    // sees, unlike the silent plate they were before.
    for (command, y) in [
        (FriendCommand::Whisper, COMMAND_YS[0]),
        (FriendCommand::Add, COMMAND_YS[1]),
        (FriendCommand::Delete, COMMAND_YS[2]),
    ] {
        let (key, fallback) = command.label();
        page.spawn((
            command,
            Button,
            Hovered::default(),
            abs_node((COMMAND_X, y, COMMAND_BTN_SIZE.0, COMMAND_BTN_SIZE.1), s),
            ImageNode {
                image: asset_server.load(COMMAND_BTN_DDJ),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
        ))
        .observe(on_friend_command)
        .with_children(|button| {
            button.spawn((
                Text::new(ui_strings.get_or(key, fallback).to_string()),
                text_font(7.5),
                TextColor(COMMAND_TEXT_COLOR),
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
}

/// An `abs_node` that starts hidden — every per-row cell does, because row 0
/// of an empty roster must not show a dot for nobody.
fn hidden_node(rect: (f32, f32, f32, f32), s: f32) -> Node {
    let mut node = abs_node(rect, s);
    node.display = Display::None;
    node
}

// --- Behavior ---------------------------------------------------------------

fn on_friend_row_press(
    press: On<Pointer<Press>>,
    rows: Query<&FriendRow>,
    roster: Res<FriendRoster>,
    mut selection: ResMut<FriendSelection>,
) {
    let Ok(row) = rows.get(press.entity) else {
        return;
    };
    // An empty row is not selectable: the command column acts on a *friend*,
    // and "row 7 of an empty list" is not one.
    selection.0 = (row.0 < roster.len()).then_some(row.0);
}

/// The command column. One verb is sent, two are answered.
fn on_friend_command(
    activate: On<Activate>,
    commands: Query<&FriendCommand>,
    roster: Res<FriendRoster>,
    selection: Res<FriendSelection>,
    mut history: ResMut<ChatHistory>,
    mut chat: ResMut<ChatState>,
    mut focus: ResMut<InputFocus>,
    mut input: Query<(Entity, &mut EditableText), With<ChatInputBox>>,
) {
    let Ok(command) = commands.get(activate.entity) else {
        return;
    };
    let selected = selection.0.and_then(|row| roster.friends.get(row));
    match command {
        FriendCommand::Whisper => match selected {
            Some(friend) => {
                chat.collapsed = false;
                chat.whisper_panel_open = false;
                prefill_whisper_input(&friend.name, &mut focus, &mut chat, &mut input);
            }
            None => history.push(ChatLine::system(NO_SELECTION)),
        },
        // 0x7302. The opcode number comes from third-party opcode maps; the
        // **body** does not, and the opcode is not in `packets!`. Missing
        // field: the request's payload — almost certainly the target's name,
        // but "almost certainly" is the guess ADR-0009 bans.
        FriendCommand::Add => history.push(ChatLine::system(ADD_UNSOURCED)),
        // 0x7304, same story: the opcode is known, the body is not. Missing
        // field: whether
        // the delete request addresses the friend by `char_id` (the entry's
        // `u32`) or by name, and what the ack 0xB304 carries.
        FriendCommand::Delete => match selected {
            Some(_) => history.push(ChatLine::system(DELETE_UNSOURCED)),
            None => history.push(ChatLine::system(NO_SELECTION)),
        },
    }
}

/// The three answers the page gives. Wording is ours — the original's own
/// `UIIT_MSG_FRIENDERR_*` strings describe *server* refusals, and reusing one
/// for "our client cannot send this yet" would put a false server voice on it.
const NO_SELECTION: &str = "Select a name in the friends list first.";
const ADD_UNSOURCED: &str =
    "Adding a friend is not sent yet: the 0x7302 request body is not known.";
const DELETE_UNSOURCED: &str =
    "Deleting a friend is not sent yet: the 0x7304 request body is not known.";

/// Push [`FriendRoster`] onto the eleven visible rows and the count.
///
/// Runs on a change of either the roster or the selection, because the row
/// plate's art is what draws the selection.
pub fn update_friend_list(
    roster: Res<FriendRoster>,
    selection: Res<FriendSelection>,
    asset_server: Res<AssetServer>,
    // All four row queries take `&mut Node`, so each one has to exclude the
    // other three markers — not just some of them. A partial set compiles and
    // then panics at startup with B0001 — before the first frame — because
    // Bevy's access check only needs one pair of queries
    // that could match the same entity.
    mut names: Query<
        (&FriendRowName, &mut Text, &mut Node),
        (
            Without<FriendRowOnline>,
            Without<FriendRowRace>,
            Without<FriendRowBar>,
        ),
    >,
    mut dots: Query<
        (&FriendRowOnline, &mut Node, &mut ImageNode),
        (
            Without<FriendRowName>,
            Without<FriendRowRace>,
            Without<FriendRowBar>,
        ),
    >,
    mut races: Query<
        (&FriendRowRace, &mut Node),
        (
            Without<FriendRowName>,
            Without<FriendRowOnline>,
            Without<FriendRowBar>,
        ),
    >,
    mut bars: Query<
        (&FriendRowBar, &mut Node, &mut ImageNode),
        (
            Without<FriendRowName>,
            Without<FriendRowOnline>,
            Without<FriendRowRace>,
        ),
    >,
    mut count: Query<&mut Text, (With<FriendCountText>, Without<FriendRowName>)>,
) {
    if !roster.is_changed() && !selection.is_changed() {
        return;
    }
    let friends = roster.friends.as_slice();
    for (cell, mut text, mut node) in names.iter_mut() {
        let next = friends
            .get(cell.0)
            .map(|f| f.name.clone())
            .unwrap_or_default();
        if text.0 != next {
            text.0 = next;
        }
        node.display = row_display(friends.len(), cell.0);
    }
    for (dot, mut node, mut image) in dots.iter_mut() {
        match friends.get(dot.0) {
            Some(friend) => {
                node.display = Display::Flex;
                // The entry's trailing `u8`: nonzero = online, per the roster
                // node's status slot (`packets::agent::ingame::FriendEntry`).
                // open in name, and the only art that depends on it.
                image.image = asset_server.load(if friend.is_online == 0 {
                    SLOT_CONTACT_OFF_DDJ
                } else {
                    SLOT_CONTACT_ON_DDJ
                });
            }
            None => node.display = Display::None,
        }
    }
    for (race, mut node) in races.iter_mut() {
        node.display = row_display(friends.len(), race.0);
    }
    for (bar, mut node, mut image) in bars.iter_mut() {
        node.display = row_display(friends.len(), bar.row);
        let dir = if selection.0 == Some(bar.row) {
            ROW_BAR_SELECTED_DIR
        } else {
            ROW_BAR_DIR
        };
        image.image = asset_server.load(format!("{dir}{}.ddj", bar.piece));
    }
    if let Ok(mut text) = count.single_mut() {
        let next = friends.len().to_string();
        if text.0 != next {
            text.0 = next;
        }
    }
}

/// A row is drawn only while a friend occupies it.
fn row_display(len: usize, row: usize) -> Display {
    if row < len {
        Display::Flex
    } else {
        Display::None
    }
}

/// The 3 pieces of a `com_bar0*_` plate over a rect: two caps and the middle.
fn bar(rect: (f32, f32, f32, f32), cap: f32) -> [((f32, f32, f32, f32), &'static str); 3] {
    let (x, y, w, h) = rect;
    [
        ((x, y, cap, h), "left"),
        ((x + cap, y, w - 2.0 * cap, h), "mid"),
        ((x + w - cap, y, cap, h), "right"),
    ]
}

/// The six 4px trim pieces of a `com_blacksquare_` plate over a `w x h` box.
fn blacksquare_ring(w: f32, h: f32) -> [((f32, f32, f32, f32), &'static str); 6] {
    let p = BLACKSQUARE_PIECE;
    [
        ((0.0, 0.0, p, p), "left_up"),
        ((w - p, 0.0, p, p), "right_up"),
        ((0.0, p, p, h - 2.0 * p), "left_side"),
        ((w - p, p, p, h - 2.0 * p), "right_side"),
        ((0.0, h - p, p, p), "left_down"),
        ((w - p, h - p, p, p), "right_down"),
    ]
}

/// The 8 pieces of a `*_wnd_` frame ring over a `w x h` box, at `p` px pieces.
fn ring(w: f32, h: f32, p: f32) -> [((f32, f32, f32, f32), &'static str); 8] {
    [
        ((0.0, 0.0, p, p), "left_up"),
        ((p - 1.0, 0.0, w - 2.0 * p + 2.0, p), "mid_up"),
        ((w - p, 0.0, p, p), "right_up"),
        ((0.0, p - 1.0, p, h - 2.0 * p + 2.0), "left_side"),
        ((w - p, p - 1.0, p, h - 2.0 * p + 2.0), "right_side"),
        ((0.0, h - p, p, p), "left_down"),
        ((p - 1.0, h - p, w - 2.0 * p + 2.0, p), "mid_down"),
        ((w - p, h - p, p, p), "right_down"),
    ]
}
