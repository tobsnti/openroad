//! The Friend page — page 13 of the Community window (`GDR_COMMUNITY_FRIEND`).
//!
//! Idea: the page is transcribed from the user's own `resinfo/iffriend.txt`
//! (three sections, 12 controls) and its row prototype
//! `resinfo/iffriendslot.txt` (3 controls). Every rect below carries the
//! `iffriend.txt:NNN` line it came from, so no number here was chosen; where a
//! `Rect` has `w,h = 0,0` the original takes the extent from the art, and the
//! size is that `.ddj`'s DDS extent, measured in the player's own archive.
//!
//! Rects are page-local: the six community pages share `ifcommunity.txt`'s
//! `13,61,451,320`, which `community/ui.rs` already spawns — the same frame
//! `letter.rs` and `guild.rs` draw into.
//!
//! **What the page shows is the wire, and the wire is thin.** The only friend
//! opcode this tree models is the join-time roster push 0x3305, kept by
//! [`FriendRoster`]; its entry has four fields and the only capture documented
//! here is an *empty* roster (`packets/src/agent/ingame.rs:896`). So a row
//! renders name + presence dot and nothing else: `char_model` is a trailing
//! `u32` whose meaning is `[U]`, and no ref-id -> race mapping exists
//! client-side either, so the race cell keeps `iffriendslot.txt`'s own
//! placeholder art rather than a guess. Clicking a row selects it
//! ([`FriendSelection`]) and its plate swaps to the selected art; the command
//! column that acts on the selection is the next slice, because its three verbs
//! are a separate question — one is pure client behaviour, two have no captured
//! request body.
//!
//! No scrolling: `GDR_FRIEND_LIST_SCROLLMGR`'s pooled rows and its scrollbar
//! are a shared widget this tree does not have, so the page draws the rows the
//! manager's own height implies ([`VISIBLE_ROWS`]) and a longer roster shows its
//! first eleven — stated rather than silently truncated, like `guild.rs` does
//! with six.
//!
//! **No field here needs a pinned line box**, and that is measured: all 12
//! controls carry `FontIndex=0` = 12 px ([`font_index_px`]) while every authored
//! text box is taller than that line (count cells 15, name cell 16, header plate
//! 24), so Bevy's 1.2 line box fits and `font_index_px`'s "One-line boxes" rule
//! has no subject on this page.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;

use crate::assets::resinfo::interface_text::font_index_px;
use crate::assets::FontAssets;
use crate::plugins::hud::game_window::abs_node;
use crate::plugins::hud::scale::text_px;
use crate::plugins::net::friend::FriendRoster;
use crate::plugins::textdata::ClientUiStrings;

// --- §Create — page chrome, `iffriend.txt:4-102` (5 controls) ---------------

/// `GDR_FRIEND_FRAME:CIFFrame` (`:83`, Rect `6,6,440,308`) — the same
/// `frameg01_wnd_` skeleton the letter and guild pages use, 16 px pieces.
const FRAME_RECT: (f32, f32, f32, f32) = (6.0, 6.0, 440.0, 308.0);
const FRAME_PIECE: f32 = 16.0;
const FRAME_DIR: &str = "media://interface/frame/frameg01_wnd_";

/// `GDR_FRIEND_BLACKSQUARE:CIFStretchWnd` (`:44`, Rect `14,18,333,282`) — the
/// list plate: flat black with six 4 px `com_blacksquare_` trim pieces.
const LIST_PLATE_RECT: (f32, f32, f32, f32) = (14.0, 18.0, 333.0, 282.0);
const BLACKSQUARE_PIECE: f32 = 4.0;
const BLACKSQUARE_DIR: &str = "media://interface/ifcommon/com_blacksquare_";

/// `GDR_FRIEND_BG_01:CIFNormalTile` (`:64`, Rect `328,22,102,276`) — the
/// command column's tiled fill, `com_bg_tile_b.ddj`.
const COMMAND_BG_RECT: (f32, f32, f32, f32) = (328.0, 22.0, 102.0, 276.0);
const BG_TILE_DDJ: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_b.ddj";

/// `GDR_FRIEND_NUMBER_STA:CIFStatic` (`:25`, Rect `353,27,33,15`,
/// `Text="UIIT_CTL_FRIEND"`) and `GDR_FRIEND_NUMBER:CIFStatic` (`:6`, Rect
/// `380,27,40,15`) — the "Friend  n" readout.
const COUNT_LABEL_RECT: (f32, f32, f32, f32) = (353.0, 27.0, 33.0, 15.0);
const COUNT_VALUE_RECT: (f32, f32, f32, f32) = (380.0, 27.0, 40.0, 15.0);

// --- §FriendList — the list, `iffriend.txt:104-182` (4 controls) ------------

/// `GDR_FRIEND_LIST_SCROLLMGR:CIFScrollManager` (`:106`, Rect `17,43,328,256`).
const LIST_RECT: (f32, f32, f32, f32) = (17.0, 43.0, 328.0, 256.0);

/// `GDR_FRIEND_LIST_BTN_SHAPE_BEGIN:CIFStatic` (`:163`, Rect `17,21,0,0`) on
/// `gil_subj_button04.ddj`, whose DDS header measures **52x24**.
const HEADER_CAP_LEFT: (f32, f32, f32, f32) = (17.0, 21.0, 52.0, 24.0);
const HEADER_CAP_LEFT_DDJ: &str = "media://interface/guild/gil_subj_button04.ddj";

/// `GDR_FRIEND_LIST_SORT_NAME_BTN:CIFButton` (`:144`, Rect `67,21,0,0`,
/// `Text="UIIT_STT_FRIENDLIST"`) on `gil_subj_button09.ddj`, **264x24**.
///
/// Drawn, not wired: the original sorts the list by this button and the roster
/// we hold is the wire's own order ([`FriendRoster`] keeps it verbatim).
/// Sorting a list whose only capture is empty would be a feature without a
/// subject, so the plate carries its caption and nothing else.
const HEADER_SORT: (f32, f32, f32, f32) = (67.0, 21.0, 264.0, 24.0);
const HEADER_SORT_DDJ: &str = "media://interface/guild/gil_subj_button09.ddj";

/// `GDR_FRIEND_LIST_BTN_SHAPE_END:CIFStatic` (`:125`, Rect `329,21,0,0`) on
/// `gil_shape.ddj`, **16x24**.
const HEADER_CAP_RIGHT: (f32, f32, f32, f32) = (329.0, 21.0, 16.0, 24.0);
const HEADER_CAP_RIGHT_DDJ: &str = "media://interface/guild/gil_shape.ddj";

/// Row pitch and height. The manager's `256` of height over `iffriendslot`'s
/// 24-tall row gives **eleven** rows at a 23-unit pitch (`11*23 + 1 = 254`),
/// which is the same packing `guild.rs` reads out of its own manager.
const ROW_PITCH: f32 = 23.0;
const ROW_HEIGHT: f32 = 24.0;
/// How many rows the manager's height holds; a longer roster shows its first
/// eleven (see the module note).
const VISIBLE_ROWS: usize = 11;

/// A row's plate: three `com_bar01_` pieces, 4 px caps. The selected row swaps
/// to `com_bar01select_`, which is how the original marks the selection.
const ROW_BAR_DIR: &str = "media://interface/ifcommon/com_bar01_";
const ROW_BAR_SELECTED_DIR: &str = "media://interface/ifcommon/com_bar01select_";
const ROW_BAR_CAP: f32 = 4.0;

// --- The row prototype, `iffriendslot.txt` (3), slot-local ------------------

/// `GDR_FRIEND_SLOT_CONTACT:CIFStatic` (`:44`, Rect `20,6,0,0`) on
/// `gil_contact_off.ddj` / `_on.ddj`, both **12x16** — the presence dot, the
/// only cell whose art follows the record (`is_online`).
const SLOT_CONTACT: (f32, f32, f32, f32) = (20.0, 6.0, 12.0, 16.0);
const SLOT_CONTACT_OFF_DDJ: &str = "media://interface/guild/gil_contact_off.ddj";
const SLOT_CONTACT_ON_DDJ: &str = "media://interface/guild/gil_contact_on.ddj";

/// `GDR_FRIEND_SLOT_RACE_MARK:CIFStatic` (`:25`, Rect `73,4,16,16`) on
/// `com_kindred_china16.ddj`, **16x16**. The prototype names *one* race mark,
/// and the entry carries no race field we have established — so this stays the
/// tree's own placeholder art rather than a guess off `char_model`. `[U]`.
const SLOT_RACE_MARK: (f32, f32, f32, f32) = (73.0, 4.0, 16.0, 16.0);
const SLOT_RACE_MARK_DDJ: &str = "media://interface/ifcommon/com_kindred_china16.ddj";

/// `GDR_FRIEND_SLOT_NAME:CIFStatic` (`:6`, Rect `95,7,89,16`).
const SLOT_NAME: (f32, f32, f32, f32) = (95.0, 7.0, 89.0, 16.0);

/// Every one of the 12 controls in both trees carries `FontIndex=0` = 12 px.
const TEXT_FONT_INDEX: usize = 0;
/// `FontColor=255,239,218,164` on `GDR_FRIEND_NUMBER_STA` — the count label.
const LABEL_COLOR: Color = Color::srgb(239.0 / 255.0, 218.0 / 255.0, 164.0 / 255.0);
/// `FontColor=255,255,255,255` on the count value, the header caption and the
/// row names.
const VALUE_COLOR: Color = Color::srgb(1.0, 1.0, 1.0);

/// Which list row the player last clicked. The command column will act on it —
/// the original's own model (three buttons beside a single-selection list).
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

/// A row's presence dot.
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
    // One size for the page, from the trees' own `FontIndex`, rounded to a
    // whole pixel by the one rounding rule (`scale::text_px`).
    let size = text_px(font_index_px(TEXT_FONT_INDEX), s);
    let text_font = TextFont {
        font: fonts.two.clone().into(),
        font_size: FontSize::Px(size),
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

    // GDR_FRIEND_BLACKSQUARE — flat black plate plus its 4 px trim.
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
                text_font.clone(),
                TextColor(VALUE_COLOR),
                TextLayout::justify(Justify::Center),
                Node {
                    position_type: PositionType::Absolute,
                    // the plate is 24 tall and the line 12: centred by hand
                    // because the caption is a child of an image node
                    top: Val::Px(text_px(6.0, s)),
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
        text_font.clone(),
        TextColor(LABEL_COLOR),
        TextLayout::justify(Justify::Left),
        abs_node(COUNT_LABEL_RECT, s),
        Pickable::IGNORE,
    ));
    page.spawn((
        FriendCountText,
        Text::new("0".to_string()),
        text_font.clone(),
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
                hidden_node((x, y, w, h), s),
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
            hidden_node(slot_rect(lx, top, SLOT_CONTACT), s),
            ImageNode {
                image: asset_server.load(SLOT_CONTACT_OFF_DDJ),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Pickable::IGNORE,
        ));
        page.spawn((
            FriendRowRace(row),
            hidden_node(slot_rect(lx, top, SLOT_RACE_MARK), s),
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
            text_font.clone(),
            TextColor(VALUE_COLOR),
            TextLayout::justify(Justify::Left),
            hidden_node(slot_rect(lx, top, SLOT_NAME), s),
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
}

/// A slot-local prototype rect placed on the list row at `top`.
fn slot_rect(lx: f32, top: f32, slot: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    (lx + slot.0, top + slot.1, slot.2, slot.3)
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
    mut press: On<PointerPress>,
    rows: Query<&FriendRow>,
    roster: Res<FriendRoster>,
    mut selection: ResMut<FriendSelection>,
) {
    press.propagate(false);
    let Ok(row) = rows.get(press.entity) else {
        return;
    };
    // An empty row is not selectable: the command column acts on a *friend*,
    // and "row 7 of an empty list" is not one.
    selection.0 = (row.0 < roster.len()).then_some(row.0);
}

/// Push [`FriendRoster`] onto the eleven visible rows and the count.
///
/// Runs on a change of either the roster or the selection, because the row
/// plate's art is what draws the selection.
pub fn update_friend_list(
    roster: Res<FriendRoster>,
    selection: Res<FriendSelection>,
    asset_server: Res<AssetServer>,
    // Each row query takes `&mut Node`, so every one has to exclude *all* the
    // other row markers — a partial set compiles and then panics at startup
    // with B0001, because Bevy's access check only needs one pair of queries
    // that could match the same entity.
    mut names: Query<
        (&FriendRowName, &mut Text, &mut Node),
        (Without<FriendRowOnline>, Without<FriendRowRace>),
    >,
    mut dots: Query<
        (&FriendRowOnline, &mut ImageNode, &mut Node),
        (Without<FriendRowName>, Without<FriendRowRace>),
    >,
    mut races: Query<
        (&FriendRowRace, &mut Node),
        (Without<FriendRowName>, Without<FriendRowOnline>),
    >,
    mut bars: Query<(&FriendRowBar, &mut ImageNode, &mut Node), Without<FriendRowOnline>>,
    mut count: Query<&mut Text, With<FriendCountText>>,
) {
    if !roster.is_changed() && !selection.is_changed() {
        return;
    }
    let friends = &roster.friends;
    for (cell, mut text, mut node) in &mut names {
        node.display = row_display(friends.len(), cell.0);
        let next = friends
            .get(cell.0)
            .map(|f| f.name.clone())
            .unwrap_or_default();
        if text.0 != next {
            text.0 = next;
        }
    }
    for (cell, mut image, mut node) in &mut dots {
        node.display = row_display(friends.len(), cell.0);
        // `is_online` is the entry's trailing `u8`; nonzero reads as online per
        // the roster node's status slot, which is the only thing the parser
        // tells us about it (`FriendEntry`, `[U]` on the name).
        let online = friends.get(cell.0).is_some_and(|f| f.is_online != 0);
        let art = if online {
            SLOT_CONTACT_ON_DDJ
        } else {
            SLOT_CONTACT_OFF_DDJ
        };
        image.image = asset_server.load(art);
    }
    for (cell, mut node) in &mut races {
        node.display = row_display(friends.len(), cell.0);
    }
    for (bar, mut image, mut node) in &mut bars {
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

/// The six 4 px trim pieces of a `com_blacksquare_` plate over a `w x h` box.
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
///
/// The fourth copy of this shape in `hud/community/` (`letter.rs:303`,
/// `letter_sub.rs:412`, `guild.rs`) — noted rather than extracted here,
/// because hoisting it edits three ported pages in a commit that adds one.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Eleven rows at the authored 23-unit pitch have to stay inside the
    /// manager's own 256 of height — that is where `VISIBLE_ROWS` comes from,
    /// and a twelfth row would hang out of the plate.
    #[test]
    fn the_visible_rows_fit_the_managers_own_height() {
        let used = (VISIBLE_ROWS - 1) as f32 * ROW_PITCH + ROW_HEIGHT;
        assert!(used <= LIST_RECT.3, "{used} must fit {}", LIST_RECT.3);
        let one_more = VISIBLE_ROWS as f32 * ROW_PITCH + ROW_HEIGHT;
        assert!(one_more > LIST_RECT.3, "a twelfth row would overhang");
    }

    /// Rows are drawn only while a friend occupies them: an empty roster shows
    /// no plate, no dot and no name — the state the only capture we own is in.
    #[test]
    fn an_empty_roster_draws_no_rows() {
        for row in 0..VISIBLE_ROWS {
            assert_eq!(row_display(0, row), Display::None);
        }
        assert_eq!(row_display(1, 0), Display::Flex);
        assert_eq!(row_display(1, 1), Display::None);
    }

    /// The row cells sit at the slot prototype's own offsets inside the list
    /// rect, so a drift in either one is visible here rather than on screen.
    #[test]
    fn a_row_places_the_prototypes_cells() {
        let (lx, ly, ..) = LIST_RECT;
        assert_eq!(slot_rect(lx, ly, SLOT_NAME), (112.0, 50.0, 89.0, 16.0));
        assert_eq!(slot_rect(lx, ly, SLOT_CONTACT), (37.0, 49.0, 12.0, 16.0));
        assert_eq!(slot_rect(lx, ly + ROW_PITCH, SLOT_NAME).1 - 50.0, ROW_PITCH);
    }

    /// Every authored text box on this page is taller than the 12 px line the
    /// trees' `FontIndex=0` asks for, so Bevy's 1.2 line box (14.4) fits and
    /// the one-line pinning rule has no subject here.
    #[test]
    fn no_text_box_on_this_page_is_a_one_line_box() {
        let line = font_index_px(TEXT_FONT_INDEX);
        assert_eq!(line, 12.0);
        for box_h in [COUNT_LABEL_RECT.3, COUNT_VALUE_RECT.3, SLOT_NAME.3] {
            assert!(box_h > line * 1.2, "{box_h} would need a pinned line box");
        }
    }
}
