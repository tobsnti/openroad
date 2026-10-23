//! The Item Mall's "My information" pane (`resinfo/ifitemmallmyinfo.txt`).
//!
//! Idea: `0x3153` pushes the account's three silk counters, and the one surface
//! the original shows them on is this pane — three statics whose `Text` is
//! empty in the descriptor and filled at runtime, one per counter. The pane
//! rides on the shared `game_window` chrome: its own inner frame
//! `GDR_ITEM_MALL_MYINFO_FRAME2` (`:367`, `Rect="6,31,244,149"`) is the shell
//! rect, the title is the enclosing sub-frame's own key
//! `UIIT_STT_SILKMALL_MY_INFO` (`GDR_ITEM_MALL_MYINFO_FRAME1`, `:405`), and the
//! interior tile letter is the one the pane declares, `com_bg_tile_b.ddj`
//! (`GDR_ITEM_MALL_MYINFO_BG`, `:386`) — not the shell's default `_d`.
//!
//! Element rects are **pane**-relative (every block is a sibling of `FRAME1`),
//! so they are rebased twice: by the inner frame's own origin `(6,31)` and then
//! by the chrome's content origin `(FRAME_VIS_SIDE + CHROME_PAD, CONTENT_TOP)`
//! = `(12,36)`, the way `community` and `character_info` do it (#310: subtract
//! the shell's own constants, never re-derive the origin from the art).
//!
//! **Deliberately left out, and why:**
//! * `GDR_ITEM_MALL_MYINFO_BUY_SILK_BTN` (`:255`) and
//!   `..._COUPON_EXCHANGE_BTN` (`:233`). Both would need a purchase/redeem
//!   opcode, and there is none: no opcode catalog, wiki page or xBot handler
//!   covers "buy from the mall", `packet_dump/` holds no frame of it, and
//!   reference servers run the shop on the web (`SK_SilkChange_BY_Web`). A
//!   button that cannot send is a dead wire, so the pane ships without them
//!   rather than with two that lie.
//! * name, level and the portrait (`..._NAME` `:177`, `..._LEVEL` `:196`,
//!   `..._FACE` `:291`). They carry no label of their own and the portrait
//!   needs the face art pipeline; the silk rows are what this slice is for.
//!   Next slice, with the frame art around them.
//!
//! The labels are the pane's own keys through the player's `textuisystem.txt`,
//! never our words, and the font size is the descriptor's own `FontIndex`
//! through [`font_index_px`] — all nine blocks of this pane carry index 0.

use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use crate::assets::resinfo::interface_text::font_index_px;
use crate::assets::FontAssets;
use crate::plugins::hud::game_window::{self, abs_node, GameWindowStyle};
use crate::plugins::hud::item_mall::ItemMallState;
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::net::silk::SilkBalance;
use crate::plugins::textdata::ClientUiStrings;

// --- Layout (resinfo, pane units) -------------------------------------------

/// `GDR_ITEM_MALL_MYINFO_FRAME2` (`ifitemmallmyinfo.txt:367`): the My-Info box
/// inside the mall's left column. Its size is this window's shell rect.
const FRAME_RECT: (f32, f32, f32, f32) = (6.0, 31.0, 244.0, 149.0);

/// `GDR_ITEM_MALL_MYINFO_BG` (`:386`) declares `com_bg_tile_b.ddj` — the
/// interior letter this pane asks for, where the shell defaults to `_d`.
const BG_TILE: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_b.ddj";

/// One counter's row: the label's rect and text key, and the value's rect.
/// Verbatim from `ifitemmallmyinfo.txt`, pane units, in the order the wire
/// carries them (`SilkUpdate { own, gift, point }`).
struct Row {
    label: (f32, f32, f32, f32),
    key: &'static str,
    fallback: &'static str,
    value: (f32, f32, f32, f32),
}

/// `:44`/`:158` own, `:25`/`:139` gift, `:6`/`:120` point. The fallbacks are
/// the English values the player's own table carries (L1166 / L3488 / L3490),
/// for scenes that load no textdata.
const ROWS: [Row; 3] = [
    Row {
        label: (82.0, 73.0, 33.0, 14.0),
        key: "UIIT_STT_SILKMALL_SILK",
        fallback: "Silk",
        value: (136.0, 74.0, 79.0, 14.0),
    },
    Row {
        label: (60.0, 98.0, 56.0, 14.0),
        key: "UIIT_CTL_SILK_INQUIRY_GIFT_SILK",
        fallback: "Bonus Silk",
        value: (136.0, 96.0, 79.0, 14.0),
    },
    Row {
        label: (26.0, 122.0, 90.0, 14.0),
        key: "UIIT_CTL_SILK_INQUIRY_POINT",
        fallback: "Point",
        value: (136.0, 120.0, 79.0, 14.0),
    },
];

/// Every block of this pane carries `FontIndex=0` (nine of nine, checked in
/// the file), which the ladder reads as 12 px.
const FONT_INDEX: usize = 0;

/// Spawn anchor (right/top, physical px). **Ours**: the mall window is not in
/// `wndpos.dat`'s persisted set, so the original's own position for it is not
/// in the data.
const WINDOW_RIGHT: f32 = 300.0;
const WINDOW_TOP: f32 = 120.0;

const CONTENT_W: f32 = FRAME_RECT.2 - 2.0 * (game_window::FRAME_VIS_SIDE + game_window::CHROME_PAD);
const CONTENT_H: f32 = FRAME_RECT.3
    - game_window::CONTENT_TOP
    - game_window::CHROME_PAD
    - game_window::FRAME_VIS_BOTTOM;

/// A pane rect rebased into the shell's content space: by the inner frame's own
/// origin, then by the chrome's content origin.
fn content_rect(rect: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = rect;
    (
        x - FRAME_RECT.0 - (game_window::FRAME_VIS_SIDE + game_window::CHROME_PAD),
        y - FRAME_RECT.1 - game_window::CONTENT_TOP,
        w,
        h,
    )
}

// --- Markers ----------------------------------------------------------------

#[derive(Component)]
pub struct MyInfoWindowRoot;

/// One of the three value statics, in wire order.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct MyInfoValue(pub usize);

// --- Spawning ---------------------------------------------------------------

/// Spawn the (initially hidden) My-Info pane.
pub fn spawn_my_info_window(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
) {
    let Ok(camera) = cam_query.single() else {
        warn!("item mall: no 2d camera to attach to");
        return;
    };
    let s = hud_scale();

    let window = game_window::spawn_game_window_styled(
        &mut commands,
        &asset_server,
        &fonts,
        camera,
        ui_strings.get_or("UIIT_STT_SILKMALL_MY_INFO", "My information"),
        (CONTENT_W, CONTENT_H),
        (WINDOW_RIGHT, WINDOW_TOP),
        s,
        GameWindowStyle {
            bg_tile: BG_TILE,
            ..GameWindowStyle::default()
        },
    );
    commands
        .entity(window.root)
        .insert((MyInfoWindowRoot, GlobalZIndex(55)))
        .entry::<Node>()
        .and_modify(|mut node| node.display = Display::None);
    commands
        .entity(window.expect_close_button())
        .observe(on_close_button);

    let font_px = font_index_px(FONT_INDEX);
    commands.entity(window.content).with_children(|content| {
        for (index, row) in ROWS.iter().enumerate() {
            // both the label and the number carry `HAlign=2` in the
            // descriptor, i.e. flush right in their own box
            content.spawn((
                Text::new(ui_strings.get_or(row.key, row.fallback)),
                text_font(&fonts, font_px, s),
                TextColor(Color::WHITE),
                TextLayout::justify(Justify::Right),
                abs_node(content_rect(row.label), s),
                Pickable::IGNORE,
            ));
            content.spawn((
                MyInfoValue(index),
                // empty until the server pushes: the descriptor's own value
                // statics carry `Text=""`, so nothing is invented here either
                Text::default(),
                text_font(&fonts, font_px, s),
                TextColor(Color::WHITE),
                TextLayout::justify(Justify::Right),
                abs_node(content_rect(row.value), s),
                Pickable::IGNORE,
            ));
        }
    });
}

fn text_font(fonts: &FontAssets, px: f32, s: f32) -> TextFont {
    TextFont {
        font: fonts.two.clone().into(),
        font_size: FontSize::Px(px * s),
        ..default()
    }
}

pub fn cleanup_my_info_window(
    mut commands: Commands,
    windows: Query<Entity, With<MyInfoWindowRoot>>,
) {
    for entity in windows.iter() {
        commands.entity(entity).despawn();
    }
}

// --- Behavior ---------------------------------------------------------------

fn on_close_button(_: On<Activate>, mut state: ResMut<ItemMallState>) {
    state.open = false;
}

/// Mirror [`ItemMallState`] onto the shell.
pub fn apply_my_info_visibility(
    state: Res<ItemMallState>,
    mut roots: Query<&mut Node, With<MyInfoWindowRoot>>,
) {
    if !state.is_changed() {
        return;
    }
    for mut node in roots.iter_mut() {
        node.display = if state.open {
            Display::Flex
        } else {
            Display::None
        };
    }
}

/// Write the three counters into their statics. `SilkBalance` is a **net**
/// resource and may be absent in a HUD-only scene, and `None` until the first
/// `0x3153` — both leave the statics as the descriptor has them: empty.
pub fn update_my_info_values(
    balance: Option<Res<SilkBalance>>,
    mut values: Query<(&MyInfoValue, &mut Text)>,
) {
    let Some(balance) = balance else {
        return;
    };
    if !balance.is_changed() {
        return;
    }
    for (slot, mut text) in values.iter_mut() {
        let next = match (balance.0, slot.0) {
            (Some(counters), 0) => counters.own.to_string(),
            (Some(counters), 1) => counters.gift.to_string(),
            (Some(counters), 2) => counters.point.to_string(),
            _ => String::new(),
        };
        if text.0 != next {
            text.0 = next;
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The shell's outer box must come back out as the pane's own inner frame
    /// (`ifitemmallmyinfo.txt:367`, `6,31,244,149`) — the window is that box,
    /// not a size we picked.
    #[test]
    fn the_shell_is_the_panes_own_inner_frame() {
        assert_eq!(
            game_window::outer_size((CONTENT_W, CONTENT_H)),
            (FRAME_RECT.2, FRAME_RECT.3)
        );
    }

    /// Rebasing is a pure translation by `(6+12, 31+36)`, and every row has to
    /// land inside the content box — the check that catches a mis-copied rect.
    #[test]
    fn every_row_lands_inside_the_content_box() {
        assert_eq!(
            content_rect((136.0, 74.0, 79.0, 14.0)),
            (118.0, 7.0, 79.0, 14.0)
        );
        for row in &ROWS {
            for rect in [row.label, row.value] {
                let (x, y, w, h) = content_rect(rect);
                assert!(x >= 0.0 && y >= 0.0, "{rect:?} rebases above the content");
                assert!(
                    x + w <= CONTENT_W && y + h <= CONTENT_H,
                    "{rect:?} leaves the content box ({CONTENT_W}x{CONTENT_H})"
                );
            }
        }
    }

    /// The rows are in wire order, so the slot index of a value static is the
    /// field of `SilkUpdate` it shows. A swapped pair here would show gift
    /// silk as own silk, which no later test could see.
    #[test]
    fn the_rows_follow_the_wire_order_and_carry_the_panes_keys() {
        assert_eq!(
            [ROWS[0].key, ROWS[1].key, ROWS[2].key],
            [
                "UIIT_STT_SILKMALL_SILK",
                "UIIT_CTL_SILK_INQUIRY_GIFT_SILK",
                "UIIT_CTL_SILK_INQUIRY_POINT",
            ]
        );
        // the labels descend in the order the pane lists them
        assert!(ROWS[0].value.1 < ROWS[1].value.1 && ROWS[1].value.1 < ROWS[2].value.1);
    }

    /// The pane declares `FontIndex=0` in all nine blocks, and the ladder reads
    /// that as 12 px. Pinned because the ladder is **not** ordered: a wrong
    /// index would silently pick a different size, not an obviously wrong one.
    #[test]
    fn the_font_size_is_the_descriptors_own_index() {
        assert_eq!(font_index_px(FONT_INDEX), 12.0);
    }
}
