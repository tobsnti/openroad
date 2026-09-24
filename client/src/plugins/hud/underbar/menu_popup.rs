//! The under-bar MENU popup: one row per major game window, opening upward
//! from the bar's MENU button.
//!
//! Idea: this widget only exists in the **4th-gen** descriptor generation —
//! there is no `resinfo/` counterpart — so unlike the rest of `underbar/` its
//! numbers come from `res_ui/nifundermenubar.2dt` (111 entries,
//! `4 + 111*976 = 108340` B). Three things about that file drive the code
//! below (`docs/re/ui/menu-toggle-bar.md`):
//!
//! 1. **Labels are CHILDREN of their buttons, bound by `ParentId`**, not
//!    siblings ordered by y. The id ladder is deliberately not monotonic in y
//!    (ids 104, 117, 114, 115 are authored after 126), so a y-order binding
//!    would mislabel five rows. [`ROWS`] therefore carries the id with the row.
//! 2. **15 buttons, 14 labelled.** Button id 120 (`ub_new_icon_stallnet`)
//!    duplicates id 117's y=353 and its label child carries an empty `Text`;
//!    it is authoring residue and is not rendered — 14 rows, not 15.
//! 3. **The 20px frame inset is derived, not assumed.** `interface\frame\
//!    ub_new_wnd_` is a prefix, not a file: 8 pieces, no centre, every one
//!    20x20. `rect.deflate(20)` reproduces the authored client rect
//!    `778,64,98,363` byte-for-byte from the frame's `758,44,138,403`.
//!
//! 2DT rects are absolute in one flat design space (children are not re-based
//! on their parent), so everything here is stated frame-local as
//! `authored - (758,44)`, and the frame itself is placed bar-local. Rows
//! overhang the client area by 11px on the left and 10 on the right, and the
//! icons sit further left still, outside the rows: that overhang is the
//! family's idiom, reproduced in `targetmenu.2dt` too, not a defect to fix.
//!
//! Eleven rows reach what they name (System, Action, Alchemy, Collection,
//! Community, Auto Potion, Party, Party Matching, Guild, Stall and Academy are
//! wired below). The rest
//! stay visible and answer in chat on purpose — the popup is the
//! discoverability surface for the unbuilt-window backlog, so hiding them would
//! hide the backlog.

use std::collections::HashSet;

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button};

use crate::assets::FontAssets;
use crate::plugins::hud::action::ActionWindowState;
use crate::plugins::hud::alchemy::model::AlchemyState;
use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::hud::collection::CollectionWindowState;
use crate::plugins::hud::community::model::CommunityPage;
use crate::plugins::hud::game_window::abs_node;
use crate::plugins::hud::stall::owner::OpenStallCommand;
use crate::plugins::system_window::{spawn_system_window, SystemWindow};
use crate::plugins::textdata::ClientUiStrings;
use crate::plugins::ui_v2::style::ImageButtonStyle;

/// The popup frame, bar-local. Authored at `758,44,138,403` in the 2dt's flat
/// design space, whose under-bar root is `81,430,800,68`; our classic bar is
/// the same 800 wide, so `x - 81` and `y - 447` carry a 2dt rect into
/// bar-local space. The 447 is the 2dt bar's own body top: its four cluster
/// buttons sit at 430/412/432/459 against our `-17/-34/-15/+12`, which give
/// 447 three times and 446 once — a 1px authoring jitter between the two
/// generations, also visible in x (`770-81 = 689` vs our menu button's 688).
/// The popup's bottom therefore lands exactly on the bar's top edge:
/// `-403 + 403 = 0`.
pub const FRAME_RECT: (f32, f32, f32, f32) = (677.0, -403.0, 138.0, 403.0);
/// Every `ub_new_wnd_` piece is 20x20, so the client area is the frame
/// deflated by 20 (checked against the authored `778,64,98,363` in the tests).
const PIECE: f32 = 20.0;

const FRAME_DIR: &str = "media://interface/frame/ub_new_wnd_";
/// `com_bg_tile_u.ddj` lives in `ifcommon/bg_tile/`, NOT in `interface/
/// underbar/` — the doc's asset list had all three arts under `underbar/` and
/// two of them are elsewhere (issue #346 comment).
const BG_TILE: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_u.ddj";
const SEPARATOR_DDJ: &str = "media://interface/underbar/ub_new_line.ddj";
const ROW_STEM: &str = "ub_new_menu_button";
const UB_DIR: &str = "media://interface/underbar/";

/// Row geometry, frame-local, from the authored entries: buttons `789,y,97,20`
/// (x 789-758 = 31), icons `766,y,20,20` (x 8), labels `793,y+7,82,10` (x 35).
const ROW_X: f32 = 31.0;
const ROW_W: f32 = 97.0;
const ROW_H: f32 = 20.0;
const ICON_X: f32 = 8.0;
const ICON_SIZE: f32 = 20.0;
const LABEL_X: f32 = 35.0;
const LABEL_DY: f32 = 7.0;
const LABEL_W: f32 = 82.0;
const LABEL_H: f32 = 10.0;
/// `ub_new_line.ddj` is 120x4, authored at x 767 (frame-local 9).
const SEP_X: f32 = 9.0;
const SEP_W: f32 = 120.0;
const SEP_H: f32 = 4.0;
/// Separators at authored y 186 / 345 / 404.
const SEPARATOR_YS: [f32; 3] = [142.0, 301.0, 360.0];

/// Label colour and size are not in the 2dt's usable fields; the row labels
/// follow the bar's own text convention (`underbar/ui.rs`).
const LABEL_COLOR: Color = Color::WHITE;
const LABEL_FONT_SIZE: f32 = 9.0;

/// One authored row: `(button id, frame-local y, icon stem, text key, English
/// fallback)`. Order is the authored y order; the ids are the `ParentId`s the
/// labels bind through, which is why they are not monotonic here.
/// Fallbacks are the real `textuisystem.txt` English column, used only when the
/// table has not loaded (offline preview scenes).
pub const ROWS: [(u32, f32, &str, &str, &str); 14] = [
    (
        83,
        16.0,
        "ub_new_icon_pt",
        "UIIT_STT_TOGGLE_PARTY",
        "Party ( P )",
    ),
    (
        93,
        41.0,
        "ub_new_icon_ptm",
        "UIIT_STT_TOGGLE_PARTYMATCH",
        "Party Matching(E)",
    ),
    (
        96,
        66.0,
        "ub_new_icon_guild",
        "UIIT_STT_TOGGLE_GUILD",
        "Guild ( U )",
    ),
    (
        99,
        91.0,
        "ub_new_icon_apprenticeship",
        "UIIT_CTL_TC_SHORTKEY_L",
        "Academy ( L )",
    ),
    (
        102,
        116.0,
        "ub_new_icon_apprenticeship_m",
        "UIIT_STT_TC_MACHING_TITLE",
        "Guardian Matching",
    ),
    (
        105,
        151.0,
        "ub_new_icon_action",
        "UIIT_STT_TOGGLE_ACTION",
        "Action ( A )",
    ),
    (
        108,
        176.0,
        "ub_new_icon_commu",
        "UIIT_STT_TOGGLE_COMMUNITY",
        "Community ( U )",
    ),
    (
        111,
        201.0,
        "ub_new_icon_quest",
        "UIIT_STT_TOGGLE_QUEST",
        "Quest ( Q )",
    ),
    (
        114,
        226.0,
        "ub_new_icon_alchemy",
        "UIIT_STT_TOGGLE_ENCHANT",
        "Alchemy ( Y )",
    ),
    (115, 251.0, "ub_new_icon_making", "UIIT_STT_MK", "Craft"),
    (
        104,
        276.0,
        "ub_new_icon_collection",
        "UIIT_PAG_COLLECTION_WINDOW",
        "Collection book",
    ),
    (
        117,
        309.0,
        "ub_new_icon_stall",
        "UIIT_CTL_OPEN_STORE",
        "Stall",
    ),
    (
        123,
        334.0,
        "ub_new_icon_recovery",
        "UIIT_STT_TOGGLE_AUTOPOTION",
        "Auto Potion (T)",
    ),
    (
        126,
        369.0,
        "ub_new_icon_system",
        "UIIT_STT_TOGGLE_SYSTEM",
        "System ( Esc )",
    ),
];

/// The rows whose target window exists in our tree today.
const SYSTEM_ROW_ID: u32 = 126;
/// `UIIT_STT_TOGGLE_ENCHANT` — "Alchemy ( Y )", the alchemy box (#333).
const ALCHEMY_ROW_ID: u32 = 114;
/// The Action row (`ub_new_icon_action`) — a 4th-gen menu entry pointing at the
/// classic-generation `ifaction.txt` panel (#475).
const ACTION_ROW_ID: u32 = 105;
/// The Collection book row (`ub_new_icon_collection`, #537).
const COLLECTION_ROW_ID: u32 = 104;
/// `UIIT_STT_TOGGLE_COMMUNITY` — "Community ( U )" (`ub_new_icon_commu`).
const COMMUNITY_ROW_ID: u32 = 108;
/// `UIIT_STT_TOGGLE_AUTOPOTION` — "Auto Potion (T)" (`ub_new_icon_recovery`).
const AUTOPOTION_ROW_ID: u32 = 123;
/// `UIIT_CTL_OPEN_STORE` — "Stall" (`ub_new_icon_stall`, `menu-toggle-bar.md`
/// row id 117; the unlabelled id 120 beside it is the removed stall-*network*
/// residue). What this row promises exists since #781 as the `/Stall`
/// chat command, so the row asks for the same thing the command does:
/// [`OpenStallCommand`] → `0x70B1` (`hud/stall/owner.rs:79`).
///
/// Deliberately the **request**, not `StallState.open`: `hud/stall/mod.rs:23-26`
/// records that this window is server-driven and "opens on the server's enter
/// broadcast … never on a click", so flipping the flag here would put an empty,
/// unowned stall shell on screen — a second dead end instead of the one being
/// removed. A stall that is already open is refused by the command's own guard
/// (`owner.rs:88-93`, toast "Stall: already open"), so the row stays dumb.
const STALL_ROW_ID: u32 = 117;
/// `UIIT_STT_TOGGLE_PARTY` — "Party ( P )" (`ub_new_icon_pt`), the window that
/// has existed since #32 (`hud/party/**`).
const PARTY_ROW_ID: u32 = 83;
/// `UIIT_STT_TOGGLE_GUILD` — "Guild ( U )" (`ub_new_icon_guild`). The guild
/// page is page 10 *inside* the community shell, and this row is how the
/// original reaches it: the menu carries **separate** toggles for Guild (96)
/// and Community (108), while the only
/// declared Community tab strip is the 4th-gen social trio Friend/Cut/Note
/// — no tab of any generation
/// selects the guild page.
const GUILD_ROW_ID: u32 = 96;
/// `UIIT_STT_TOGGLE_PARTYMATCH` — "Party Matching(E)" (`ub_new_icon_ptm`),
/// the LFG board (`hud/party_matching/**`).
const PARTY_MATCH_ROW_ID: u32 = 93;
/// `UIIT_CTL_TC_SHORTKEY_L` — "Academy ( L )" (`ub_new_icon_apprenticeship`),
/// the academy member panel (`hud/academy/**`). Its sibling row 102
/// ("Guardian Matching", `ub_new_icon_apprenticeship_m`) stays in the backlog:
/// the 785x480 `GDR_MENTOR_MATCH` board has no rows to draw until the
/// `0xB47D` record fields are known (`plugins/net/academy.rs`).
const ACADEMY_ROW_ID: u32 = 99;

#[derive(Component)]
pub struct MenuPopupRoot;

/// The button id this row was built from, so a row's action is keyed on the
/// authored id rather than on its position.
#[derive(Component)]
pub struct MenuRow(pub u32);

/// Toggle the popup: it is a child of the bar body, so it dies with the bar.
pub fn toggle_menu_popup(
    bar: Entity,
    open: Option<Entity>,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
    scale: f32,
    commands: &mut Commands,
) {
    if let Some(open) = open {
        commands.entity(open).despawn();
        return;
    }
    commands.entity(bar).with_children(|bar| {
        spawn_menu_popup(bar, asset_server, fonts, ui_strings, scale);
    });
}

fn spawn_menu_popup(
    bar: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
    s: f32,
) {
    let (_, _, fw, fh) = FRAME_RECT;
    bar.spawn((
        MenuPopupRoot,
        Name::from("Under-bar Menu Popup"),
        abs_node(FRAME_RECT, s),
    ))
    .with_children(|popup| {
        // the 8-piece ring: all pieces 20x20, no centre piece exists. Mids and
        // sides stretch and overlap their neighbours by 1 unit, like the
        // mframe_wnd_ shell, so scaled positions cannot open a seam.
        let p = PIECE;
        let edges = [
            ((0.0, 0.0, p, p), "left_up"),
            ((p - 1.0, 0.0, fw - 2.0 * p + 2.0, p), "mid_up"),
            ((fw - p, 0.0, p, p), "right_up"),
            ((0.0, p - 1.0, p, fh - 2.0 * p + 2.0), "left_side"),
            ((fw - p, p - 1.0, p, fh - 2.0 * p + 2.0), "right_side"),
            ((0.0, fh - p, p, p), "left_down"),
            ((p - 1.0, fh - p, fw - 2.0 * p + 2.0, p), "mid_down"),
            ((fw - p, fh - p, p, p), "right_down"),
        ];
        for (rect, piece) in edges {
            popup.spawn((
                abs_node(rect, s),
                ImageNode {
                    image: asset_server.load(format!("{FRAME_DIR}{piece}.ddj")),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }

        // client area = frame deflated by one piece: the authored 778,64,98,363
        popup.spawn((
            abs_node((p, p, fw - 2.0 * p, fh - 2.0 * p), s),
            ImageNode {
                image: asset_server.load(BG_TILE),
                image_mode: NodeImageMode::Tiled {
                    tile_x: true,
                    tile_y: true,
                    stretch_value: s,
                },
                ..default()
            },
            Pickable::IGNORE,
        ));

        for y in SEPARATOR_YS {
            popup.spawn((
                abs_node((SEP_X, y, SEP_W, SEP_H), s),
                ImageNode {
                    image: asset_server.load(SEPARATOR_DDJ),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }

        for (id, y, icon, key, fallback) in ROWS {
            popup.spawn((
                abs_node((ICON_X, y, ICON_SIZE, ICON_SIZE), s),
                ImageNode {
                    image: asset_server.load(format!("{UB_DIR}{icon}.ddj")),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                Pickable::IGNORE,
            ));
            // the whole row is the hit area (WCAG: the 20px rows are under the
            // 24px target minimum, so at least make all 97px of them clickable)
            let style = ImageButtonStyle {
                normal: asset_server.load(format!("{UB_DIR}{ROW_STEM}.ddj")),
                hover: asset_server.load(format!("{UB_DIR}{ROW_STEM}_focus.ddj")),
                press: asset_server.load(format!("{UB_DIR}{ROW_STEM}_press.ddj")),
                ..Default::default()
            };
            popup
                .spawn((
                    MenuRow(id),
                    Button,
                    Hovered::default(),
                    abs_node((ROW_X, y, ROW_W, ROW_H), s),
                    ImageNode {
                        image: style.normal.clone(),
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                    style,
                ))
                .observe(on_row_activate);
            popup.spawn((
                Text::new(ui_strings.get_or(key, fallback).to_string()),
                TextFont {
                    font: fonts.nine.clone().into(),
                    font_size: FontSize::Px(LABEL_FONT_SIZE * s),
                    ..default()
                },
                TextColor(LABEL_COLOR),
                TextLayout::justify(Justify::Left),
                abs_node((LABEL_X, y + LABEL_DY, LABEL_W, LABEL_H), s),
                Pickable::IGNORE,
            ));
        }
    });
}

/// Activate a row. Nine of the fourteen rows now reach the window (or, for
/// Stall, the request) their label names; the rest answer in chat and stay put,
/// which is the honest rendering of the backlog this popup indexes.
#[allow(clippy::too_many_arguments)]
fn on_row_activate(
    activate: On<Activate>,
    rows: Query<&MenuRow>,
    popups: Query<Entity, With<MenuPopupRoot>>,
    window: Query<Entity, With<SystemWindow>>,
    cameras: Query<Entity, With<Camera2d>>,
    asset_server: Res<AssetServer>,
    ui_strings: Res<ClientUiStrings>,
    mut alchemy: ResMut<AlchemyState>,
    mut action_state: ResMut<ActionWindowState>,
    mut collection_state: ResMut<CollectionWindowState>,
    mut community_state: ResMut<crate::plugins::hud::community::model::CommunityState>,
    mut party_state: ResMut<crate::plugins::hud::party::model::PartyWindowState>,
    mut autopotion_state: ResMut<crate::plugins::hud::autopotion::model::AutoPotionState>,
    mut open_stall: MessageWriter<OpenStallCommand>,
    mut history: Option<ResMut<ChatHistory>>,
    mut announced: Local<HashSet<u32>>,
    mut commands: Commands,
) {
    let Ok(MenuRow(id)) = rows.get(activate.entity) else {
        return;
    };
    if *id == COLLECTION_ROW_ID {
        for popup in popups.iter() {
            commands.entity(popup).despawn();
        }
        collection_state.open = !collection_state.open;
        return;
    }
    if *id == ACTION_ROW_ID {
        for popup in popups.iter() {
            commands.entity(popup).despawn();
        }
        action_state.open = !action_state.open;
        return;
    }
    // Community and Auto Potion have had their windows for a while; the popup
    // was never told and kept dropping both rows into the "no window yet"
    // debug line, which is invisible at INFO — so the row simply did nothing.
    if *id == COMMUNITY_ROW_ID {
        for popup in popups.iter() {
            commands.entity(popup).despawn();
        }
        community_state.open = !community_state.open;
        return;
    }
    if *id == AUTOPOTION_ROW_ID {
        for popup in popups.iter() {
            commands.entity(popup).despawn();
        }
        autopotion_state.open = !autopotion_state.open;
        return;
    }
    // The same lag, two rows further: the party window has existed since #32
    // and the guild page since #486, and both rows still answered
    // "not available yet".
    if *id == PARTY_ROW_ID {
        for popup in popups.iter() {
            commands.entity(popup).despawn();
        }
        party_state.open = !party_state.open;
        return;
    }
    if *id == PARTY_MATCH_ROW_ID {
        for popup in popups.iter() {
            commands.entity(popup).despawn();
        }
        // Toggled through the command queue rather than through a 18th
        // `ResMut`: Bevy's `SystemParamFunction` impls stop at 16 parameters,
        // and this observer was already at the limit — one more turned into an
        // `cannot be used as an entity observer` error that names no type.
        commands.queue(|world: &mut World| {
            let mut state =
                world.resource_mut::<crate::plugins::hud::party_matching::model::PartyMatchState>();
            state.open = !state.open;
        });
        return;
    }
    if *id == ACADEMY_ROW_ID {
        for popup in popups.iter() {
            commands.entity(popup).despawn();
        }
        // Queued for the same reason party-matching is: this observer is at
        // Bevy's 16-parameter limit, and a 17th `ResMut` fails with an error
        // that names no type.
        commands.queue(|world: &mut World| {
            let mut state = world.resource_mut::<crate::plugins::hud::academy::AcademyState>();
            state.open = !state.open;
        });
        return;
    }
    if *id == GUILD_ROW_ID {
        for popup in popups.iter() {
            commands.entity(popup).despawn();
        }
        // Toggling on the *page*, not just on `open`: the shell may already be
        // up on mail, and then "Guild" must switch to guild rather than close
        // the window the player is looking at.
        if community_state.open && community_state.page == CommunityPage::Guild {
            community_state.open = false;
        } else {
            community_state.open = true;
            community_state.page = CommunityPage::Guild;
        }
        return;
    }
    if *id == STALL_ROW_ID {
        for popup in popups.iter() {
            commands.entity(popup).despawn();
        }
        open_stall.write(OpenStallCommand { title: None });
        return;
    }
    if !matches!(*id, SYSTEM_ROW_ID | ALCHEMY_ROW_ID) {
        // These rows used to end in a `debug!`, which is invisible at the
        // default filter: the click looked like a dead button and nothing —
        // log or screen — said why (REGRESSION-AUDIT.md finding 5). Now the
        // row names itself **once** in the log, and every click answers the
        // player in chat, the same way `hud/petition.rs` names a petition arm
        // it decodes but cannot host. Chat is `Option` because the popup's own
        // plugin does not own `ChatHistory` (`hud/chat/mod.rs:27` does), so a
        // scene that builds the underbar alone must degrade, not panic.
        let label = ROWS
            .iter()
            .find(|(row_id, ..)| row_id == id)
            .map(|(_, _, _, key, fallback)| ui_strings.get_or(key, fallback).to_string())
            .unwrap_or_else(|| format!("row {id}"));
        if announced.insert(*id) {
            warn!("underbar menu: \"{label}\" (row id {id}) has no window in openroad yet");
        }
        if let Some(history) = history.as_mut() {
            history.push(ChatLine::system(format!("{label}: not available yet.")));
        }
        return;
    }
    // the popup closes behind the row it opened
    for popup in popups.iter() {
        commands.entity(popup).despawn();
    }
    if *id == ALCHEMY_ROW_ID {
        alchemy.open = !alchemy.open;
        if !alchemy.open {
            alchemy.clear();
        }
        return;
    }
    if let Ok(open) = window.single() {
        commands.entity(open).despawn();
    } else if let Some(camera) = cameras.iter().next() {
        spawn_system_window(&mut commands, &asset_server, &ui_strings, camera);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The client rect is DERIVED from the piece size, not transcribed: all
    /// eight `ub_new_wnd_` pieces are 20x20, and deflating the authored frame
    /// by 20 must reproduce the authored `778,64,98,363` exactly. If it ever
    /// stops doing so, one of the two numbers was copied wrong.
    #[test]
    fn the_client_area_is_the_frame_deflated_by_one_piece() {
        // authored, 2dt design space
        let frame = (758.0, 44.0, 138.0, 403.0);
        let client = (
            frame.0 + PIECE,
            frame.1 + PIECE,
            frame.2 - 2.0 * PIECE,
            frame.3 - 2.0 * PIECE,
        );
        assert_eq!(client, (778.0, 64.0, 98.0, 363.0));
        // and the frame we place is that same frame carried into bar-local
        // space by (x-81, y-447)
        assert_eq!(
            FRAME_RECT,
            (frame.0 - 81.0, frame.1 - 447.0, frame.2, frame.3)
        );
        // the popup's bottom edge sits exactly on the bar's top edge
        assert_eq!(FRAME_RECT.1 + FRAME_RECT.3, 0.0);
    }

    /// 15 buttons, 14 labelled: id 120 duplicates id 117's y and has an empty
    /// label, so it is not a row. Rendering 15 would stack two rows at y 353.
    #[test]
    fn the_unlabelled_leftover_is_not_a_row() {
        assert_eq!(ROWS.len(), 14);
        assert!(!ROWS.iter().any(|(id, ..)| *id == 120));
        // id 117 owns the authored y=353 slot alone (frame-local 309)
        let stall = ROWS.iter().find(|(id, ..)| *id == 117).expect("stall row");
        assert_eq!(stall.1, 309.0);
        assert_eq!(ROWS.iter().filter(|(_, y, ..)| *y == 309.0).count(), 1);
    }

    /// The id ladder is NOT monotonic in y — ids 104, 117, 114, 115 are
    /// authored after 126 — which is exactly why labels bind by `ParentId`
    /// and this table carries the id explicitly. A y-ordered id list would
    /// mislabel five rows.
    #[test]
    fn rows_are_y_ordered_but_ids_are_not() {
        let ys: Vec<f32> = ROWS.iter().map(|(_, y, ..)| *y).collect();
        assert!(ys.windows(2).all(|w| w[0] < w[1]), "rows ascend in y");
        let ids: Vec<u32> = ROWS.iter().map(|(id, ..)| *id).collect();
        assert!(
            !ids.windows(2).all(|w| w[0] < w[1]),
            "the authored id ladder is deliberately out of order"
        );
        // every id is distinct, and every one is an authored CNIFMenuButton id
        let authored = [
            83, 93, 96, 99, 102, 105, 108, 111, 120, 123, 126, 104, 117, 114, 115,
        ];
        assert!(ids.iter().all(|id| authored.contains(id)));
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ROWS.len());
    }

    /// Row/icon/label geometry, frame-local, against the authored absolutes.
    #[test]
    fn row_geometry_matches_the_authored_entries() {
        // buttons 789,y,97,20 · icons 766,y,20,20 · labels 793,y+7,82,10
        assert_eq!((ROW_X + 758.0, ROW_W, ROW_H), (789.0, 97.0, 20.0));
        assert_eq!((ICON_X + 758.0, ICON_SIZE), (766.0, 20.0));
        assert_eq!(
            (LABEL_X + 758.0, LABEL_DY, LABEL_W, LABEL_H),
            (793.0, 7.0, 82.0, 10.0)
        );
        // separators ub_new_line.ddj 120x4 at 767, y 186/345/404
        assert_eq!((SEP_X + 758.0, SEP_W, SEP_H), (767.0, 120.0, 4.0));
        assert_eq!(
            SEPARATOR_YS.map(|y| y + 44.0),
            [186.0, 345.0, 404.0],
            "authored separator ys"
        );
        // the icons sit left of the rows and both overhang the client area —
        // the family's idiom (same in targetmenu.2dt), not a defect
        assert!(ICON_X < PIECE, "icons overhang the 20px client inset");
        assert!(ROW_X + ROW_W > FRAME_RECT.2 - PIECE);
    }

    /// The wired rows point at the window their label promises; the remaining
    /// rows stay visible and inert on purpose (the popup is the backlog's
    /// discoverability surface). The ids are the authored ones, so a row
    /// moving in the list cannot silently repoint a target.
    ///
    /// Community and Auto Potion were inert while their windows existed — the
    /// row fell into a `debug!` that no default log level shows, so the click
    /// looked broken rather than unimplemented. Pinning label *and* id here is
    /// what makes that mismatch a test failure.
    #[test]
    fn every_wired_row_points_at_the_window_its_label_promises() {
        assert_eq!(
            (
                SYSTEM_ROW_ID,
                ACTION_ROW_ID,
                ALCHEMY_ROW_ID,
                ACADEMY_ROW_ID,
                COMMUNITY_ROW_ID,
                AUTOPOTION_ROW_ID,
                PARTY_ROW_ID,
                PARTY_MATCH_ROW_ID,
                GUILD_ROW_ID,
                STALL_ROW_ID
            ),
            (126, 105, 114, 99, 108, 123, 83, 93, 96, 117)
        );
        for (id, key) in [
            (SYSTEM_ROW_ID, "UIIT_STT_TOGGLE_SYSTEM"),
            (ACTION_ROW_ID, "UIIT_STT_TOGGLE_ACTION"),
            (ALCHEMY_ROW_ID, "UIIT_STT_TOGGLE_ENCHANT"),
            (ACADEMY_ROW_ID, "UIIT_CTL_TC_SHORTKEY_L"),
            (COMMUNITY_ROW_ID, "UIIT_STT_TOGGLE_COMMUNITY"),
            (AUTOPOTION_ROW_ID, "UIIT_STT_TOGGLE_AUTOPOTION"),
            (PARTY_ROW_ID, "UIIT_STT_TOGGLE_PARTY"),
            (PARTY_MATCH_ROW_ID, "UIIT_STT_TOGGLE_PARTYMATCH"),
            (GUILD_ROW_ID, "UIIT_STT_TOGGLE_GUILD"),
            (STALL_ROW_ID, "UIIT_CTL_OPEN_STORE"),
        ] {
            let row = ROWS.iter().find(|(row, ..)| *row == id).expect("wired row");
            assert_eq!(row.3, key);
        }

        // The rows that stay inert, named. This is what the old
        // `wired.len() + 5 == ROWS.len()` count asserted, but it moves with the
        // table instead of needing a hand-kept number: a new wired row must be
        // taken out of this list, and a new inert row must be added to it.
        let wired = [
            SYSTEM_ROW_ID,
            ACTION_ROW_ID,
            ALCHEMY_ROW_ID,
            ACADEMY_ROW_ID,
            COMMUNITY_ROW_ID,
            AUTOPOTION_ROW_ID,
            PARTY_ROW_ID,
            PARTY_MATCH_ROW_ID,
            GUILD_ROW_ID,
            STALL_ROW_ID,
            COLLECTION_ROW_ID,
        ];
        let inert: Vec<u32> = ROWS
            .iter()
            .map(|(id, ..)| *id)
            .filter(|id| !wired.contains(id))
            .collect();
        assert_eq!(
            inert,
            vec![102, 111, 115],
            "the inert rows are Guardian Matching, Quest and Craft"
        );
        assert_eq!(wired.len() + inert.len(), ROWS.len());
    }

    /// Every row that names a window must *open* that window — driven off the
    /// row table itself, one `Activate` per row, so the check is the click a
    /// player makes and not a re-reading of the `if` chain.
    ///
    /// This is the test the Party and Guild rows needed: both windows existed
    /// (`hud/party/**` since #32, the guild page since #486) while their rows
    /// fell through into the "not available yet" chat line, which no unit test
    /// noticed because the fallback branch is a perfectly healthy code path
    /// for the *other* nine rows.
    #[test]
    fn every_row_that_names_a_window_opens_it() {
        for (id, open) in EXPECTED_OPENERS {
            let mut app = row_app();
            let row = app
                .world_mut()
                .spawn(MenuRow(id))
                .observe(on_row_activate)
                .id();
            assert!(!open(&app), "row {id}: the window is up before the click");
            app.world_mut().trigger(Activate { entity: row });
            app.update();
            assert!(
                open(&app),
                "menu row {id} ({}) names a window that exists and did not open it",
                ROWS.iter()
                    .find(|(row_id, ..)| *row_id == id)
                    .map(|r| r.4)
                    .unwrap_or("?")
            );
        }
    }

    /// `(row id, "is the window this row promises now up?")`. The guild probe
    /// is deliberately page-aware: `open` alone would pass while the shell
    /// still showed mail, which is the exact defect being fixed.
    #[allow(clippy::type_complexity)]
    const EXPECTED_OPENERS: [(u32, fn(&App) -> bool); 8] = [
        (ACADEMY_ROW_ID, |app| {
            app.world()
                .resource::<crate::plugins::hud::academy::AcademyState>()
                .open
        }),
        (PARTY_MATCH_ROW_ID, |app| {
            app.world()
                .resource::<crate::plugins::hud::party_matching::model::PartyMatchState>()
                .open
        }),
        (PARTY_ROW_ID, |app| {
            app.world()
                .resource::<crate::plugins::hud::party::model::PartyWindowState>()
                .open
        }),
        (GUILD_ROW_ID, |app| {
            let state = app
                .world()
                .resource::<crate::plugins::hud::community::model::CommunityState>();
            state.open && state.page == CommunityPage::Guild
        }),
        (COMMUNITY_ROW_ID, |app| {
            app.world()
                .resource::<crate::plugins::hud::community::model::CommunityState>()
                .open
        }),
        (ACTION_ROW_ID, |app| {
            app.world().resource::<ActionWindowState>().open
        }),
        (COLLECTION_ROW_ID, |app| {
            app.world().resource::<CollectionWindowState>().open
        }),
        (AUTOPOTION_ROW_ID, |app| {
            app.world()
                .resource::<crate::plugins::hud::autopotion::model::AutoPotionState>()
                .open
        }),
    ];

    /// The Stall row asks for a stall the way `/Stall` does, and says nothing
    /// in chat.
    ///
    /// Two assertions because the row has two ways to lie: sending nothing (the
    /// state before this change: the row fell through to
    /// "Stall: not available yet." while `/Stall` had worked since #781), and
    /// sending *and* apologising in the same click. Deliberately checks the
    /// message, not `StallState.open` — this window is server-driven
    /// (`hud/stall/mod.rs:23-26`), so a click that opened it locally would be
    /// the defect, not the fix.
    #[test]
    fn the_stall_row_asks_for_a_stall_instead_of_apologising() {
        let mut app = row_app();
        let row = app
            .world_mut()
            .spawn(MenuRow(STALL_ROW_ID))
            .observe(on_row_activate)
            .id();
        app.world_mut().trigger(Activate { entity: row });
        app.update();

        let messages = app.world().resource::<Messages<OpenStallCommand>>();
        let mut cursor = messages.get_cursor();
        let sent: Vec<&OpenStallCommand> = cursor.read(messages).collect();
        assert_eq!(
            sent,
            vec![&OpenStallCommand { title: None }],
            "the Stall row must send the same request /Stall sends"
        );
        let lines: Vec<String> = app
            .world()
            .resource::<ChatHistory>()
            .iter()
            .map(|line| line.text.clone())
            .collect();
        assert!(
            lines.is_empty(),
            "a row that acts must not also say it cannot: {lines:?}"
        );
    }

    /// Just enough world for `on_row_activate`: it takes an `AssetServer` and
    /// a camera query for the System row, and every window state it toggles.
    fn row_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ))
        .init_asset::<Image>()
        .init_resource::<ClientUiStrings>()
        .init_resource::<ChatHistory>()
        .init_resource::<AlchemyState>()
        .init_resource::<ActionWindowState>()
        .init_resource::<CollectionWindowState>()
        .init_resource::<crate::plugins::hud::community::model::CommunityState>()
        .init_resource::<crate::plugins::hud::party::model::PartyWindowState>()
        .init_resource::<crate::plugins::hud::party_matching::model::PartyMatchState>()
        .init_resource::<crate::plugins::hud::autopotion::model::AutoPotionState>()
        .init_resource::<crate::plugins::hud::academy::AcademyState>()
        .add_message::<OpenStallCommand>();
        app
    }
}
