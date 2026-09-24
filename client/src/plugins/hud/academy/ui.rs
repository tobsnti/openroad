//! Geometry and spawn for the Academy member panel.
//!
//! Every rect below is transcribed from the user's own PK2 with its line
//! number; where the data declares `DDJ=""` or `Text=""` the value is
//! server-supplied and is left blank rather than filled with a guess.
//!
//! Coordinate note: the original hosts this page inside `GDR_MAINPOPUP`'s tab
//! strip (`ifmainpopup.txt:16`, page rect `13,38,365,355`, and that whole
//! block is behind `#ifdef APPLY_MENTOR_SYSTEM`). We have no MainPopup, so —
//! exactly as `hud/party/ui.rs` and `hud/character_info/ui.rs` did before us —
//! this is a standalone `game_window`, and content space is page space minus
//! `(0, ORIGIN_Y)` with `ORIGIN_Y` the topmost authored control. All internal
//! distances (the 33 px row pitch, the per-control offsets) are unchanged.

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::{Activate, Button};

use crate::assets::FontAssets;
use crate::plugins::cursor::interactions::entity_select::SelectedEntity;
use crate::plugins::hud::academy::AcademyState;
use crate::plugins::hud::game_window::{self, abs_node};
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::net::academy::AcademyAction;
use crate::plugins::net::entities::NetworkId;
use crate::plugins::textdata::ClientUiStrings;
use crate::plugins::ui_v2::style::ImageButtonStyle;

// --- Layout ----------------------------------------------------------------

/// `GDR_APPRENTICESHIP_LEADER`'s y (`ifapprenticeship.txt:34`) — the topmost
/// control on the page, and so the row the content box starts on.
const ORIGIN_Y: f32 = 27.0;
/// `GDR_APPRENTICESHIP_FRAMEL` is `0,0,364,303` (`ifapprenticeship.txt:433`),
/// so the page is 364 wide.
const CONTENT_W: f32 = 364.0;
/// The MainPopup page is 355 tall, but the two footer buttons are authored at
/// y 339 on 24-tall art (`:338`, `:357`), i.e. they end at 363 — 8 px past the
/// page. Hosted in the original's tab strip that overhang is the strip's
/// problem; in a standalone shell it would clip the buttons off, so the
/// content box is sized to the content's own extent. Deviation, stated
/// (ADR-0009): 363 instead of 355, for the 8 px the original itself authored
/// outside its page.
const PAGE_BOTTOM: f32 = 363.0;
const CONTENT_H: f32 = PAGE_BOTTOM - ORIGIN_Y;
/// `GDR_MAINPOPUP` sits at `595,262` and is 388 wide (`ginterface.txt:664`) in
/// the original's 1024-wide design space, so its right margin is
/// `1024 - (595 + 388) = 41` — the same anchor `hud/party/ui.rs` derived for
/// the sibling page of the same shell.
const WINDOW_RIGHT: f32 = 41.0;
const WINDOW_TOP: f32 = 262.0;

const ART: &str = "media://interface/party/";
const IFCOMMON: &str = "media://interface/ifcommon/";

// Chrome: `_BGTILE_0` `16,36,332,33` on `com_bg_tile_b` (:414), `_BGTILE_1`
// `27,300,308,4` on `com_bg_tile_a` (:395), `_SLOT_MSGBOARD` `0,304` on
// `pt_msg.ddj`, whose art is 364x36 (:186).
const BGTILE_0_RECT: (f32, f32, f32, f32) = (16.0, 36.0 - ORIGIN_Y, 332.0, 33.0);
const DIVIDER_RECT: (f32, f32, f32, f32) = (27.0, 300.0 - ORIGIN_Y, 308.0, 4.0);
const MSGBOARD_RECT: (f32, f32, f32, f32) = (0.0, 304.0 - ORIGIN_Y, 364.0, 36.0);

// The pinned leader band, hand-laid in `ifapprenticeship.txt` and NOT the slot
// template offset by a constant: matched control for control the x deltas are
// 7, 9, 8, 11, 11, 2 and the name is 3 px wider — the descriptor's own
// numbers, kept because the plate art is authored against them.
// Lines: crown :34, icon frame :376, name :129, "Lv" :110, level :91,
// rank name :72, dissolve button :53.
const HEADER_CROWN_RECT: (f32, f32, f32, f32) = (33.0, 27.0 - ORIGIN_Y, 12.0, 12.0);
const HEADER_FRAME_RECT: (f32, f32, f32, f32) = (6.0, 30.0 - ORIGIN_Y, 36.0, 36.0);
const HEADER_NAME_RECT: (f32, f32, f32, f32) = (71.0, 46.0 - ORIGIN_Y, 83.0, 17.0);
const HEADER_LV_RECT: (f32, f32, f32, f32) = (156.0, 46.0 - ORIGIN_Y, 14.0, 17.0);
const HEADER_LEVEL_RECT: (f32, f32, f32, f32) = (172.0, 46.0 - ORIGIN_Y, 19.0, 17.0);
const HEADER_RANK_NAME_RECT: (f32, f32, f32, f32) = (201.0, 46.0 - ORIGIN_Y, 104.0, 17.0);
/// `pt_button.ddj` measures 44x20 and the block authors `w=h=0`, i.e. art-sized:
/// the 44x20 here is the art's extent, not a chosen number.
const HEADER_DISSOLVE_RECT: (f32, f32, f32, f32) = (316.0, 38.0 - ORIGIN_Y, 44.0, 20.0);

// The seven `CIFApprenticeShipSlot` instances: `2,{69,102,...,267},360,36`
// (`ifapprenticeship.txt:319,300,281,262,243,224,205`). Pitch 33 on a 36-tall
// rect, because all three slot arts have their bottom three rows fully
// transparent. **The art split is the capacity split**: slots 0-1 are
// `pt_slot_co.ddj` and slots 2-6 are `pt_slot_re.ddj`, and the binary's own
// assertion strings say why — `SubMentor is Over than 2` and
// `ApprenticeShip is Over than 5`, both referenced by
// the same window function. 1 leader + 2 + 5 = the 8 of `TraningCampMember is Over than 8`.
const SLOT_X: f32 = 2.0;
const SLOT_Y0: f32 = 69.0 - ORIGIN_Y;
const SLOT_PITCH: f32 = 33.0;
const SLOT_W: f32 = 360.0;
const SLOT_H: f32 = 36.0;
const SLOT_COUNT: usize = 7;
/// How many of the seven rows are assistant-guardian rows (`pt_slot_co.ddj`).
const ASSISTANT_ROWS: usize = 2;

// Row-local rects from `ifapprenticeshipslot.txt` (360x36 space): rank name
// :34, level :53, "Lv" :72, name :91, rank mark :110, status/portrait :129
// and :148 (byte-identical rects, the shared-rect idiom).
const SLOT_NAME_RECT: (f32, f32, f32, f32) = (63.0, 14.0, 80.0, 17.0);
const SLOT_LV_RECT: (f32, f32, f32, f32) = (145.0, 14.0, 14.0, 17.0);
const SLOT_LEVEL_RECT: (f32, f32, f32, f32) = (161.0, 14.0, 19.0, 17.0);
const SLOT_RANK_NAME_RECT: (f32, f32, f32, f32) = (199.0, 14.0, 104.0, 17.0);

// Footer: Invite `56,339` (`:357`, `Text="UIIT_CTL_TC_INVITE"`) and Matching
// `230,339` (`:338`, `Text="UIIT_CTL_TC_MACHING"`), both `w=h=0` on
// `com_button.ddj`, which measures 76x24.
const INVITE_RECT: (f32, f32, f32, f32) = (56.0, 339.0 - ORIGIN_Y, 76.0, 24.0);
const MATCH_RECT: (f32, f32, f32, f32) = (230.0, 339.0 - ORIGIN_Y, 76.0, 24.0);

/// The empty-state sentence, drawn on the message board while the roster is
/// empty. `resinfo` COLOR is A,R,G,B, and the board's own `FontColor` is
/// `255,254,251,216` (`ifapprenticeship.txt:182`).
const MSGBOARD_TEXT_RECT: (f32, f32, f32, f32) = (12.0, 313.0 - ORIGIN_Y, 340.0, 17.0);

/// `GDR_APPRENTICESHIP_STATIC_NAME`'s `FontColor="255,255,255,148"`
/// (`ifapprenticeship.txt:125`), A,R,G,B — the leader band's pale yellow,
/// where the slot rows are plain white (`ifapprenticeshipslot.txt:88`).
const HEADER_TEXT_COLOR: Color = Color::srgb_u8(255, 255, 148);
const SLOT_TEXT_COLOR: Color = Color::srgb_u8(255, 255, 255);
/// `GDR_APPRENTICESHIP_DISSOLVE_APPRENTICESHIP_BTN`'s `FontColor`
/// `255,254,245,218` (`:49`); the message board's is `255,254,251,216` (`:182`).
const BUTTON_TEXT_COLOR: Color = Color::srgb_u8(254, 245, 218);
const MSGBOARD_TEXT_COLOR: Color = Color::srgb_u8(254, 251, 216);

// --- Markers ---------------------------------------------------------------

#[derive(Component)]
pub struct AcademyWindowRoot;

// --- Spawning --------------------------------------------------------------

/// Spawn the (initially hidden) academy member panel.
pub fn spawn_academy_window(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
) {
    let Ok(camera) = cam_query.single() else {
        warn!("academy window: no 2d camera to attach to");
        return;
    };
    let s = hud_scale();
    // `GDR_APPRENTICESHIP_FRAMEL`'s own caption (`ifapprenticeship.txt:436`).
    let title = ui_strings
        .get_or("UIIT_CTL_TC_MEMBER", "Academy Member")
        .to_string();

    let window = game_window::spawn_game_window(
        &mut commands,
        &asset_server,
        &fonts,
        camera,
        &title,
        (CONTENT_W, CONTENT_H),
        (WINDOW_RIGHT, WINDOW_TOP),
        s,
    );
    commands
        .entity(window.root)
        .insert((AcademyWindowRoot, GlobalZIndex(55)))
        .entry::<Node>()
        .and_modify(|mut node| node.display = Display::None);
    commands
        .entity(window.expect_close_button())
        .observe(on_close_button);

    let text_font = |size: f32| TextFont {
        font: fonts.two.clone().into(),
        font_size: FontSize::Px(size * s),
        ..default()
    };
    let lv = ui_strings.get_or("UIIT_STT_LEVEL_LV", "Lv").to_string();

    commands.entity(window.content).with_children(|content| {
        let img = |rect: (f32, f32, f32, f32), path: String| {
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
        let tile = |rect: (f32, f32, f32, f32), letter: &str| {
            (
                abs_node(rect, s),
                ImageNode {
                    image: asset_server.load(format!("{IFCOMMON}bg_tile/com_bg_tile_{letter}.ddj")),
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
        let text = |rect, value: String, color: Color, size: f32| {
            (
                Text::new(value),
                text_font(size),
                TextColor(color),
                TextLayout::justify(Justify::Left),
                abs_node(rect, s),
                Pickable::IGNORE,
            )
        };

        // --- chrome ------------------------------------------------------
        content.spawn(tile(BGTILE_0_RECT, "b"));
        content.spawn(tile(DIVIDER_RECT, "a"));
        content.spawn(img(MSGBOARD_RECT, format!("{ART}pt_msg.ddj")));
        // The board is the panel's own status line in the original; what it
        // prints there is unknown. While the roster is empty this prints the
        // client's own shipped sentence rather than leaving a blank plate — a
        // deviation with a reason (ADR-0009), and the string is data, not
        // invention.
        content.spawn(text(
            MSGBOARD_TEXT_RECT,
            ui_strings
                .get_or(
                    "UIIT_MSG_TC_ERROR_TRAININGCAMP_NOTHING",
                    "Academy does not exist.",
                )
                .to_string(),
            MSGBOARD_TEXT_COLOR,
            8.0,
        ));

        // --- pinned leader band ------------------------------------------
        //
        // Not drawn, on purpose rather than by omission: `_LEADER_RANK_MARK`
        // (`53,46,16,16`) and the `_PICTURE`/`_LEADER_STATUS` pair (both
        // `9,33,28,28`) all declare `DDJ=""` — their art is chosen at runtime
        // from member data this client has no source for.
        content.spawn(img(HEADER_FRAME_RECT, format!("{ART}pt_icon_frame.ddj")));
        content.spawn(img(
            HEADER_CROWN_RECT,
            format!("{IFCOMMON}com_pt_leader.ddj"),
        ));
        content.spawn(text(
            HEADER_NAME_RECT,
            String::new(),
            HEADER_TEXT_COLOR,
            8.0,
        ));
        content.spawn(text(HEADER_LV_RECT, lv.clone(), HEADER_TEXT_COLOR, 8.0));
        content.spawn(text(
            HEADER_LEVEL_RECT,
            String::new(),
            HEADER_TEXT_COLOR,
            8.0,
        ));
        content.spawn(text(
            HEADER_RANK_NAME_RECT,
            String::new(),
            HEADER_TEXT_COLOR,
            8.0,
        ));
        spawn_dissolve_button(content, &asset_server, &ui_strings, s, &text_font);

        // --- the seven member rows ---------------------------------------
        for row in 0..SLOT_COUNT {
            let y = SLOT_Y0 + SLOT_PITCH * row as f32;
            let art = if row < ASSISTANT_ROWS {
                "pt_slot_co.ddj"
            } else {
                "pt_slot_re.ddj"
            };
            content.spawn(img((SLOT_X, y, SLOT_W, SLOT_H), format!("{ART}{art}")));
            // Row content is server-supplied and has no source here (0x3C81
            // reads zero bytes in the original), so the four statics are
            // spawned empty at their authored rects — a real empty slot, not a
            // placeholder shape.
            for rect in [SLOT_NAME_RECT, SLOT_LEVEL_RECT, SLOT_RANK_NAME_RECT] {
                content.spawn(text(
                    (SLOT_X + rect.0, y + rect.1, rect.2, rect.3),
                    String::new(),
                    SLOT_TEXT_COLOR,
                    8.0,
                ));
            }
            content.spawn(text(
                (
                    SLOT_X + SLOT_LV_RECT.0,
                    y + SLOT_LV_RECT.1,
                    SLOT_LV_RECT.2,
                    SLOT_LV_RECT.3,
                ),
                lv.clone(),
                SLOT_TEXT_COLOR,
                8.0,
            ));
        }

        // --- footer -------------------------------------------------------
        spawn_footer_button(
            content,
            &asset_server,
            s,
            INVITE_RECT,
            ui_strings.get_or("UIIT_CTL_TC_INVITE", "Invite"),
            &text_font,
            true,
        );
        spawn_footer_button(
            content,
            &asset_server,
            s,
            MATCH_RECT,
            ui_strings.get_or("UIIT_CTL_TC_MACHING", "Matching"),
            &text_font,
            false,
        );
    });
}

/// The two 76x24 `com_button.ddj` footer buttons. `is_invite` picks the
/// observer, not the look: both are the same art in the data.
fn spawn_footer_button(
    parent: &mut RelatedSpawnerCommands<ChildOf>,
    asset_server: &AssetServer,
    s: f32,
    rect: (f32, f32, f32, f32),
    label: &str,
    text_font: &impl Fn(f32) -> TextFont,
    is_invite: bool,
) {
    let mut node = abs_node(rect, s);
    node.justify_content = JustifyContent::Center;
    node.align_items = AlignItems::Center;
    let mut button = parent.spawn((
        Button,
        Hovered::default(),
        node,
        ImageNode {
            image: asset_server.load(format!("{IFCOMMON}com_button.ddj")),
            image_mode: NodeImageMode::Stretch,
            ..default()
        },
    ));
    button.with_children(|b| {
        b.spawn((
            Text::new(label.to_string()),
            text_font(7.0),
            TextColor(BUTTON_TEXT_COLOR),
            TextLayout::justify(Justify::Center),
            Node {
                width: Val::Percent(100.0),
                align_self: AlignSelf::Center,
                ..default()
            },
            Pickable::IGNORE,
        ));
    });
    if is_invite {
        button.observe(on_academy_invite);
    } else {
        button.observe(on_academy_matching);
    }
}

/// The leader band's `pt_button.ddj` — "Disband" (`UIIT_CLT_TC_DISSOLUTION_BUTTON`,
/// the key's own typo included; the block itself declares `Text=""`, so the
/// word is code-fed, the same construction the party window's leader button has).
///
/// Drawn **disabled**: disbanding is `0x7471` and this tree has no request
/// type for it, because that body is unknown. Hiding the button would hide a
/// control the original shows; wiring it would mean inventing a body. So it is
/// visible and greyed, which is also the state the original itself shows
/// outside an academy (same precedent as the party window's greyed
/// "Dismiss").
fn spawn_dissolve_button(
    parent: &mut RelatedSpawnerCommands<ChildOf>,
    asset_server: &AssetServer,
    ui_strings: &ClientUiStrings,
    s: f32,
    text_font: &impl Fn(f32) -> TextFont,
) {
    let mut node = abs_node(HEADER_DISSOLVE_RECT, s);
    node.justify_content = JustifyContent::Center;
    node.align_items = AlignItems::Center;
    parent
        .spawn((
            Button,
            Hovered::default(),
            InteractionDisabled,
            node,
            ImageNode {
                image: asset_server.load(format!("{ART}pt_button_disable.ddj")),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            ImageButtonStyle {
                normal: asset_server.load(format!("{ART}pt_button.ddj")),
                hover: asset_server.load(format!("{ART}pt_button_focus.ddj")),
                press: asset_server.load(format!("{ART}pt_button_press.ddj")),
                disable: asset_server.load(format!("{ART}pt_button_disable.ddj")),
            },
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(
                    ui_strings
                        .get_or("UIIT_CLT_TC_DISSOLUTION_BUTTON", "Disband")
                        .to_string(),
                ),
                text_font(7.0),
                TextColor(BUTTON_TEXT_COLOR),
                TextLayout::justify(Justify::Center),
                Node {
                    width: Val::Percent(100.0),
                    align_self: AlignSelf::Center,
                    ..default()
                },
                Pickable::IGNORE,
            ));
        });
}

// --- Behaviour -------------------------------------------------------------

fn on_close_button(_: On<Activate>, mut state: ResMut<AcademyState>) {
    state.open = false;
}

/// Invite (0x7472): the body is a bare `u32` and the verdict against the
/// original's builder is `matches`,
/// so this is the one footer command with a fully sourced request. Like the
/// guild window's invite it addresses the click-selected world entity — the
/// panel has no target picker of its own, and neither does the original's.
fn on_academy_invite(
    _: On<Activate>,
    selected: Res<SelectedEntity>,
    ids: Query<&NetworkId>,
    mut actions: MessageWriter<AcademyAction>,
) {
    let Some(entity) = selected.0 else {
        info!("academy: invite clicked with nothing selected");
        return;
    };
    match ids.get(entity) {
        Ok(id) => {
            actions.write(AcademyAction::Invite(id.0));
        }
        // A selected world entity always carries a NetworkId; sending a zero
        // uid instead would be a silent, server-visible mistake.
        Err(_) => warn!("academy: invite on {entity:?}, which has no NetworkId"),
    }
}

/// Matching (0x747D): asks for the first page of the matching board. The reply
/// lands in [`crate::plugins::net::academy::AcademyMatchBoard`], whose record
/// block stays undecoded until its fields are known — so this button sends and
/// logs today, and gets its own window (the 785x480 `GDR_MENTOR_MATCH` tree)
/// when there is something sourced to draw in it.
fn on_academy_matching(_: On<Activate>, mut actions: MessageWriter<AcademyAction>) {
    actions.write(AcademyAction::MatchListPage(0));
}

/// Show/hide the panel to match [`AcademyState`].
pub fn apply_academy_window_visibility(
    state: Res<AcademyState>,
    mut roots: Query<&mut Node, With<AcademyWindowRoot>>,
) {
    if !state.is_changed() {
        return;
    }
    for mut node in &mut roots {
        node.display = if state.open {
            Display::Flex
        } else {
            Display::None
        };
    }
}

/// Drop the window when the world scene ends (rule 7's other half).
pub fn cleanup_academy_window(
    mut commands: Commands,
    roots: Query<Entity, With<AcademyWindowRoot>>,
    mut state: ResMut<AcademyState>,
) {
    for entity in roots.iter() {
        commands.entity(entity).despawn();
    }
    *state = AcademyState::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seven rows are the authored ladder, and the 2/5 art split is the
    /// capacity split the binary's assertion strings state.
    #[test]
    fn the_seven_rows_are_the_authored_ladder_with_the_two_five_art_split() {
        let ys: Vec<f32> = (0..SLOT_COUNT)
            .map(|row| SLOT_Y0 + SLOT_PITCH * row as f32 + ORIGIN_Y)
            .collect();
        assert_eq!(ys, vec![69.0, 102.0, 135.0, 168.0, 201.0, 234.0, 267.0]);
        // 2 assistant guardians + 5 apprentices + the pinned leader = 8, the
        // bound of `TraningCampMember is Over than 8`.
        assert_eq!(ASSISTANT_ROWS + (SLOT_COUNT - ASSISTANT_ROWS) + 1, 8);
    }

    /// The leader band is NOT the slot template offset by a constant: six
    /// controls, six different x deltas. A future refactor that "tidies" the
    /// header into `slot + delta` would silently move five of them.
    #[test]
    fn the_leader_band_is_not_the_slot_template_offset() {
        let deltas = [
            HEADER_NAME_RECT.0 - SLOT_NAME_RECT.0,
            HEADER_LV_RECT.0 - SLOT_LV_RECT.0,
            HEADER_LEVEL_RECT.0 - SLOT_LEVEL_RECT.0,
            HEADER_RANK_NAME_RECT.0 - SLOT_RANK_NAME_RECT.0,
        ];
        assert_eq!(deltas, [8.0, 11.0, 11.0, 2.0]);
        // and the name field is 3 px wider on the leader row
        assert_eq!(HEADER_NAME_RECT.2 - SLOT_NAME_RECT.2, 3.0);
    }

    /// The content box covers the footer buttons the original authors past its
    /// own page bottom (the stated deviation above).
    #[test]
    fn the_content_box_covers_the_authored_footer() {
        assert_eq!(INVITE_RECT.1 + INVITE_RECT.3 + ORIGIN_Y, PAGE_BOTTOM);
        assert_eq!(MATCH_RECT.1 + MATCH_RECT.3, CONTENT_H);
    }
}
