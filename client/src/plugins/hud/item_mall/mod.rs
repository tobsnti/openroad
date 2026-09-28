//! `ifnewitemmall` — the Item Mall (Silk shop), display only.
//!
//! Idea: this window is the first one in the tree that was **measured against a
//! photograph of the original before a line of it was written**
//! (`docs/re/ui/original-world-windows-2026-08-23.md` §7, screenshots
//! `artifacts/capture/orders/done/241-world-stands/{itemmall-open,probe-0}.png`,
//! 806x629 with the client area at image offset `(+3, +26)`). Every rect below
//! is the authored one from `Media/resinfo/ifnewitemmall.txt` /
//! `ifnewitemmallmyinfo.txt`, and each one was then **template-matched against
//! that photograph**: `mall_mainpage.ddj` lands at image `(21,71)` with a mean
//! error of `0.00` over its opaque pixels, and the seven category tabs, the two
//! list buttons, the account plate and the silk coin all land at delta `(0,0)`.
//! The data and the picture agree pixel for pixel, so nothing here is a guess.
//!
//! **Which generation.** `Media/config/define.txt:19` defines `NEW_ITEMMALL`,
//! so `ginterface.txt:957-978` selects `GDR_RENEWAL_ITEM_MALL:CIFNewItemMall`
//! (`Rect="0,0,800,600"`, `DDJ="interface\frame\mall_wnd_"`,
//! `Text="UIIT_STT_SILKMALL"`) and **not** the `#else` branch's `GDR_ITEM_MALL`
//! on `mframe_wnd_` (`:981-999`). `define.txt:4` also defines
//! `ITEM_MALL_ADD_TABS` ("alchemy + premium tab"), which is exactly why the
//! photographed left column carries `Alchemy` and `Premium`.
//!
//! **The strings are shipped, not blanked.** The first question this lane had
//! to answer was whether the labels are emptied out like MOTP's were. They are
//! not: `Media/server_dep/silkroad/textdata/textuisystem.txt` (UTF-16LE, 5377
//! lines — the size is quoted so a future absence claim can check it read the
//! same file) carries the whole block at lines **1102-1240** plus **4317-4335**,
//! all with English text. Every label below cites its line.
//!
//! **Both generations were checked**, as the guild-tab lesson demands
//! (`original-world-windows-2026-08-23.md` §9): the main window lives in the
//! resinfo tree, and the `.2dt` art carries the two *sub* windows behind the
//! right-hand buttons — `res_ui/silksearch.2dt` (`CNIFSilkBuyList`, title
//! `UIIT_STT_SILKMALL_BUY_LIST`) and `res_ui/silkrecievelist.2dt`
//! (`CNIFSilkRecieveList`, `UIIT_STT_SILKMALL_RECEIVE_LIST`, two sub-tabs
//! `UIIT_CTL_SILK_INQUIRY_GIFT_SILK` / `_GET_SILK`). Neither sub window is
//! built here — they list *purchases*, which need a wire path we do not have.
//!
//! **Measured button behaviour** (§29/§30 of the same document, capture
//! `260-itemmall/btn-*.png`, each click checked against `packet_dump/proxy/c2s/`
//! which stayed at **16 files** throughout): `Buying list` opens a sub-window
//! and sends nothing — so it is the one control here that is drawn *enabled*
//! and wired, to [`buy_list`]. `Obtained list` does nothing at all, which is
//! why the original draws it with the disabled art. `Buy silk` leaves the game
//! for the operating system's browser, so it has no opcode to look for and
//! stays pathless on purpose. `Coupon` is still unmeasured (a foreign window
//! had the focus, §30).
//!
//! **No button moves silk, and none is a dead wire.** The original itself
//! answers how to draw an unreachable button: in the photograph `Obtained list`
//! is drawn with `mall_avata_button_disable.ddj` and `Coupon` with
//! `mall_big_button_disable.ddj` (border-only match error 4.35 / 4.24 against
//! 20.56 / 21.92 for the enabled art), while `Buying list` and `Buy silk` use
//! the enabled art. So the shipped art *has* a disabled state and the original
//! uses it. We draw every button that cannot act in that disabled state and
//! attach no observer: two of them match the original exactly, and for the
//! others it is a deliberate deviation (ADR-0009) — we have no `0x34xx` silk
//! opcode measured, so an enabled-looking `Buy silk` would be a lie about what
//! it can do. `Buying list` is the exception the measurement earned: it needs
//! no opcode, so it keeps the original's enabled art.
//!
//! **What is deliberately left empty**, because the source does not exist yet:
//! the account block's four values (name, level, silk, activity points) and the
//! middle column's item list. The item rows are server content (the mall
//! package list); the account values would need a wire path this lane may not
//! touch. The *labels* are shipped text and are drawn; the numbers stay blank
//! rather than showing an invented `0`.
//!
//! **Corrected by the unoccluded capture** (§24 of the same document, image
//! `artifacts/capture/orders/done/260-itemmall/03-mall-page.png`): the article
//! list is **two columns**, not one, and the right-hand column carries a
//! character preview with two buttons, `Buy` and `Original`, above the two
//! list buttons. The grid lives in [`list`]; the two buttons are
//! [`PREVIEW_BUY_AT`] / [`PREVIEW_ORIGINAL_AT`] below. The same capture also
//! removed a guess: §7 drew `UIIT_STT_SILKMALL_MAIN_INTRO` ("Welcome to
//! Silkroad's special quality Item Mall") across the top of the middle column
//! because nothing better was visible there. The unoccluded photograph shows
//! the list starting at window `y = 75` with a `Best item` bar and no welcome
//! line at all, so that label is gone.
//!
//! Two stated deviations beyond that:
//! * the shared `(X)` close button is drawn although the capture shows no close
//!   control in the mall's top strip (the strip's centre is occluded by the
//!   inventory window in the only photograph we have). A window that can only
//!   be closed by the key that opened it is worse than one extra button.
//! * `GDR_ITEM_MALL_MYINFO_POINT` is authored at `x = 6000`, i.e. parked off
//!   screen by the artists, and its label `_STA_POINT_STR` carries `Text=""`.
//!   The photograph confirms the gap at `y = 50` between `Silk` and
//!   `Activity Points`. We reproduce the omission by not drawing the row at all
//!   instead of drawing it 6000 px to the right of a window that is 800 wide.

use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use crate::assets::FontAssets;
use crate::plugins::hud::chat::model::ChatState;
use crate::plugins::hud::game_window::{self, ChromeFamily, WindowGeometry};
use crate::plugins::hud::scale::{font_px, hud_scale};
use crate::plugins::textdata::ClientUiStrings;
use crate::scenes::SceneState;

pub mod buy_list;
pub mod list;

use list::{MallSection, UNIT_STRING_KEY};

/// `interface\frame\mall_wnd_`, the family `ginterface.txt:964` names for this
/// window. Measured off the eight decoded pieces: corners and sides are 40
/// wide, `mid_up` is 68 tall, `mid_down` 48 — the same extents as
/// [`game_window::MFRAME_WND`], which is why the piece sizes are not a guess
/// even though the family is a different one.
///
/// The `vis_*` fields are the piece extents rather than a narrower painted
/// trim: unlike `mframe_wnd_`, every pixel of these pieces carries alpha (min
/// alpha > 32 across all four profiled pieces, 30 % of `left_up` above 200), so
/// the family is a translucent panel, not a thin ring. The photograph shows
/// exactly that — the HUD behind the mall's top strip is dimmed, not hidden.
const MALL_WND: ChromeFamily = ChromeFamily {
    dir: "media://interface/frame/mall_wnd_",
    side_w: 40.0,
    top_h: 68.0,
    bottom_h: 48.0,
    vis_left: 40.0,
    vis_right: 40.0,
    vis_top: 68.0,
    vis_bottom: 48.0,
    // No caption is legible in the capture (the centre of the top strip is
    // behind the inventory window), so the band is the default family's, which
    // is the only measured one for a 68 px strip.
    title_y: 6.0,
    title_h: 22.0,
    tile_mid_down: false,
};

/// `ginterface.txt:967` — `Rect="0,0,800,600"`. The window *is* the page.
const WINDOW_SIZE: (f32, f32) = (800.0, 600.0);
/// The page has no measurable anchor (it fills the original's 800x600 client
/// area), so it sits on the top edge like the original does.
const ANCHOR_RIGHT_TOP: (f32, f32) = (0.0, 0.0);
/// `GDR_RENEWAL_ITEM_MALL_BG`, `ifnewitemmall.txt` — `com_bg_tile_m`.
const BG_TILE: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_m.ddj";
/// `ginterface.txt:970`, textuisystem L1102.
const TITLE_KEY: (&str, &str) = ("UIIT_STT_SILKMALL", "Item Mall");

const ART: &str = "media://interface/mall/";

/// The page spawns with no articles. The mall package list is server content
/// and this tree has no opcode for it, so the grid is drawn empty rather than
/// filled with the four articles the photograph happens to show.
const NO_CATALOGUE: &[MallSection] = &[];

/// `(left, top, width, height)` in window units — the shape of every authored
/// rect here.
type Rect = (f32, f32, f32, f32);

/// `GDR_RENEWAL_ITEM_MALL_MAIN_BTN`, `Rect="18,45,0,0"` — art-sized, and
/// `mall_mainpage.ddj` is 120x28 (DDS header). Template-matched at image
/// `(21,71)` with error `0.00`: the authored origin and the photograph are the
/// same point.
const MAIN_PAGE_AT: Rect = (18.0, 45.0, 120.0, 28.0);

/// The seven category tabs. The tab art is `mall_item_tab_off.ddj` (116x32),
/// identified by matching all `mall/*.ddj` of tab size against the photograph:
/// its border pixels match at `x = 18`, `y = 75 + 32k` for **exactly** `k =
/// 0..=6` with error 0.67, and fail (35.3) at `k = 7`. So both the pitch (32,
/// the art's own height) and the count (7) are measured, not counted off a
/// screenshot by eye.
const TAB_X: f32 = 18.0;
const TAB_TOP: f32 = 75.0;
const TAB_PITCH: f32 = 32.0;
const TAB_SIZE: (f32, f32) = (116.0, 32.0);
/// The icon inside a tab: 20x20 art at `(25, 81 + 32k)`. Each of the seven was
/// identified by template match with an error below 0.5 — including `Reserved`,
/// whose icon is `mall_choicelist_icon.ddj` and would otherwise have been a
/// guess between three plausible files.
const TAB_ICON_AT: (f32, f32) = (25.0, 81.0);
const TAB_ICON_SIZE: (f32, f32) = (20.0, 20.0);
/// Label inset from the tab's left edge, from the photograph: the text starts
/// at image `x = 56`, i.e. window `x = 53`, which is 35 past the tab origin.
const TAB_LABEL_INSET: f32 = 35.0;

/// One category: its shipped label and its measured icon.
struct Category {
    key: &'static str,
    fallback: &'static str,
    icon: &'static str,
}

/// Top to bottom as photographed. The keys are the shipped ones; the line
/// numbers are `textuisystem.txt`.
const CATEGORIES: [Category; 7] = [
    Category {
        // L1105
        key: "UIIT_STT_SILKMALL_CONSUMPTION",
        fallback: "Expendables",
        icon: "mall_consum_icon.ddj",
    },
    Category {
        // L1107
        key: "UIIT_STT_SILKMALL_AVATA",
        fallback: "Avatar",
        icon: "mall_avatar_icon.ddj",
    },
    Category {
        // L1108
        key: "UIIT_STT_SILKMALL_PET",
        fallback: "Pet",
        icon: "mall_pet_icon.ddj",
    },
    Category {
        // L1120
        key: "UIIT_STT_SILKMALL_PREMIUM",
        fallback: "Premium",
        icon: "mall_premium_icon.ddj",
    },
    Category {
        // L1121
        key: "UIIT_STT_SILKMALL_ARCHEMY",
        fallback: "Alchemy",
        icon: "mall_alchemy_icon.ddj",
    },
    Category {
        // L1104
        key: "UIIT_STT_SILKMALL_HOT_STORE",
        fallback: "HOT shop",
        icon: "mall_hot_icon.ddj",
    },
    Category {
        // L1110
        key: "UIIT_STT_SILKMALL_SHOPPING_BASKET_LIST",
        fallback: "Reserved",
        icon: "mall_choicelist_icon.ddj",
    },
];

/// `GDR_RENEWAL_ITEM_MALL_SEARCH_BG`, `Rect="141,43,0,0"`, art `mall_find_wnd.ddj`
/// (332x24), and `_SEARCH` above it, `Rect="141,43,332,24"` — the edit box is
/// exactly the plate.
const SEARCH_AT: Rect = (141.0, 43.0, 332.0, 24.0);
/// L4317 `UIIT_STT_SILKMALL_SEARCH_DEFAULT`.
const SEARCH_KEY: (&str, &str) = ("UIIT_STT_SILKMALL_SEARCH_DEFAULT", "Write Item name");
/// `GDR_RENEWAL_ITEM_MALL_MID_BG`, `Rect="137,72,424,519"`, art `mall_back_wnd.ddj`.
/// The art is 424x520 and paints its frame line at texture `0..422 / 0..512`,
/// which is where [`list::LIST_AT`]'s bottom bound comes from.
const MID_BG_AT: Rect = (137.0, 72.0, 424.0, 519.0);

/// `GDR_RENEWAL_ITEM_MALL_LEFT_FRAME` `15,42,124,542` (art `mall_ava_wnd_`) and
/// `_LEFT_BG` `25,52,83,510`; `_RIGHT_BG` `564,42,224,372` (`mall_charac_frame.ddj`)
/// and `_RIGHT_BG_WND` `564,414,224,124` (`mall_charac_wnd.ddj`).
const RIGHT_FRAME_AT: Rect = (564.0, 42.0, 224.0, 372.0);
const RIGHT_WND_AT: Rect = (564.0, 414.0, 224.0, 124.0);

/// The two list buttons, `mall_avata_button*.ddj` (104x32), art-sized:
/// `_RIGHT_SILKBUYLIST_BTN03` `571,416` (L1159) and `_RIGHT_SILKRECIEVELIST_BTN04`
/// `676,416` (L1160). Both matched at delta `(0,0)` in the photograph.
const LIST_BUTTON_SIZE: (f32, f32) = (104.0, 32.0);
const BUY_LIST_AT: (f32, f32) = (571.0, 416.0);
const BUY_LIST_KEY: (&str, &str) = ("UIIT_STT_SILKMALL_BUY_LIST", "Buying list");
/// **Measured inert** (`original-world-windows-2026-08-23.md` §30,
/// `260-itemmall/btn-obtained2.png`): clicking `Obtained list` in the original
/// leaves the window unchanged and sends no c2s packet. That is the *reason*
/// for the disabled art the photograph already showed it drawn with, so this
/// button now has a measurement behind it rather than only a match score, and
/// it stays exactly as it was drawn.
const RECEIVE_LIST_AT: (f32, f32) = (676.0, 416.0);
const RECEIVE_LIST_KEY: (&str, &str) = ("UIIT_STT_SILKMALL_RECEIVE_LIST", "Obtained list");
/// `mall_avata_button_disable.ddj` — see the module doc on why the unwired
/// buttons use the shipped disabled art.
const LIST_BUTTON_DDJ: &str = "mall_avata_button_disable.ddj";
/// `mall_avata_button.ddj`, the enabled state of the same 104x32 art. Used by
/// exactly one button, `Buying list`: §29 measured that opening its sub-window
/// sends **nothing** (`packet_dump/proxy/c2s/` held 16 files before and after
/// the click), so it is the one control here that can be honestly reachable
/// without a wire path. The photograph draws it enabled too.
const LIST_BUTTON_ENABLED_DDJ: &str = "mall_avata_button.ddj";

/// The two buttons under the character preview, which §7's occluded capture
/// could not see (`original-world-windows-2026-08-23.md` §24). Both are
/// `mall_avata_button.ddj` (104x32) and both were template-matched in
/// `03-mall-page.png`: `Buy` at image `(574,403)` with error **1.61** and
/// `Original` at `(679,403)` with error **2.11**, i.e. window `(571,377)` and
/// `(676,377)` — the same two `x` as `Buying list` / `Obtained list`, 39 px
/// higher. The positive control for the match is `Buying list` itself, which
/// scores 2.33 at its known place on the same art.
///
/// **Stated deviation (ADR-0009):** the original draws *both* of them with the
/// enabled art (1.61 / 2.11 against 17.78 / 18.25 for
/// `mall_avata_button_disable.ddj`). We draw them disabled and attach no
/// observer, for the same reason as `Buy silk`: `Buy` moves silk and
/// `Original` resets a preview render we do not have, so an enabled-looking
/// button would be a lie about what it can do.
const PREVIEW_BUY_AT: (f32, f32) = (571.0, 377.0);
/// textuisystem L1156. `UIIT_CTL_SILKMALL_ITEM_BUY` (L4330) reads `Buy avatar`
/// and is therefore *not* this button: the photograph shows exactly `Buy`.
const PREVIEW_BUY_KEY: (&str, &str) = ("UIIT_STT_SILKMALL_BUY", "Buy");
const PREVIEW_ORIGINAL_AT: (f32, f32) = (676.0, 377.0);
/// textuisystem L4321, one line below `UIIT_STT_SILKMALL_BESTSELLER_ITEM`.
const PREVIEW_ORIGINAL_KEY: (&str, &str) = ("UIIT_CTL_SILKMALL_INIT_ITEM", "Original");

/// `GDR_ITEM_MALL_MYINFO`, `ifnewitemmall.txt` section `MyInfo`:
/// `Rect="562,449,223,135"`. Its children are authored **relative to it**, and
/// the photograph confirms the composition: `mall_charac_textbox.ddj` at
/// `562+3, 449+0` matched at delta `(0,0)`.
const MYINFO_AT: (f32, f32) = (562.0, 449.0);
const MYINFO_BG_AT: Rect = (3.0, 0.0, 220.0, 88.0);
/// `_NAME` `7,9,106,15`, `_LEVEL` `115,9,98,15` (HAlign=2, right).
const MYINFO_NAME_AT: Rect = (7.0, 9.0, 106.0, 15.0);
const MYINFO_LEVEL_AT: Rect = (115.0, 9.0, 98.0, 15.0);
/// `_STA_SILK_STR` `7,29,106,15` (L4333 `Silk`), value `_SILK` `115,29,82,15`,
/// coin `_STA_SILK` `201,27` (`mall_silk_icon.ddj`, 14x14 — matched at delta
/// `(0,0)`).
const MYINFO_SILK_LABEL_AT: Rect = (7.0, 29.0, 106.0, 15.0);
const MYINFO_SILK_VALUE_AT: Rect = (115.0, 29.0, 82.0, 15.0);
const MYINFO_SILK_COIN_AT: (f32, f32, f32, f32) = (201.0, 27.0, 14.0, 14.0);
const MYINFO_SILK_KEY: (&str, &str) = ("UIIT_STT_SILKMALL_ITEM_SILK", "Silk");
/// `_STA_GIFT_SILK_STR` `7,69,106,15` (L4334 `Activity Points`), value
/// `_GIFT_SILK` `115,69,82,15`, unit `_STA_GIFT_SILK` `216,69`
/// (`UIIT_STT_SILKMALL_P_POINT`, the `AP` in the photograph).
const MYINFO_AP_LABEL_AT: Rect = (7.0, 69.0, 106.0, 15.0);
const MYINFO_AP_VALUE_AT: Rect = (115.0, 69.0, 82.0, 15.0);
const MYINFO_AP_UNIT_AT: Rect = (216.0, 69.0, 24.0, 15.0);
const MYINFO_AP_KEY: (&str, &str) = ("UIIT_STT_SILKMALL_ITEM_GIFTSILK", "Activity Points");
const MYINFO_AP_UNIT_KEY: (&str, &str) = ("UIIT_STT_SILKMALL_P_POINT", "AP");
/// `_BUY_SILK_BTN` `3,92` (L1167) and `_COUPON_EXCHANGE_BTN` `114,92` (L1169),
/// art `mall_big_button*.ddj` (108x44), both matched at delta `(0,0)`.
const MYINFO_BUTTON_SIZE: (f32, f32) = (108.0, 44.0);
///
/// **`Buy silk` is measured, and it is measured as leaving the game**
/// (`original-world-windows-2026-08-23.md` §30,
/// `260-itemmall/btn-buysilk2.png`): the click brings the *operating system's
/// browser* to the front, on a shop URL of the server's operator, and
/// `packet_dump/proxy/c2s/` stays at 16 files. There is therefore no opcode to
/// find for this button and nothing here to wire: a clone has no business
/// opening a third party's payment page. It stays drawn and pathless **on
/// purpose** — this comment is the reason, so that a later reader does not
/// mistake it for an unfinished one.
const BUY_SILK_AT: (f32, f32) = (3.0, 92.0);
const BUY_SILK_KEY: (&str, &str) = ("UIIT_STT_SILKMALL_BUY_SILK", "Buy silk");
const COUPON_AT: (f32, f32) = (114.0, 92.0);
const COUPON_KEY: (&str, &str) = ("UIIT_STT_SILKMALL_COUPON_EXCHANGE", "Coupon");
const MYINFO_BUTTON_DDJ: &str = "mall_big_button_disable.ddj";

// --- the confirmation the original asks before it opens the mall ------------

/// `probe-0.png`. Nothing in the resinfo tree references
/// `UIIT_STT_SILKMALL_{ENTER,WARNING_1..3}` (grep over all 231 `resinfo/*.txt`:
/// zero hits, while `SILKMALL` itself hits 19 files — that is the positive
/// control), so the dialog is built in code in the original and its plate art
/// could not be identified by template match either (the plate is alpha-blended
/// over the scene, so no candidate reaches a decisive error). What **is**
/// measured from the photograph: the plate is 400x208, the logo
/// `mall_pic.ddj` (148x52) sits at plate-local `(126,40)` (matched at image
/// `(397,315)`, error 0.37), the two buttons are `com_button.ddj` (76x24) at
/// `(122,163)` and `(202,163)` — matched at image `(392,440)` / `(472,440)`,
/// error 3.9 / 3.5, the residue being the label drawn on top — and the three
/// warning lines sit at `y = 105, 122, 139`, pitch 17.
///
/// The chrome is therefore ours: the shared `msgbox2_window_` shell with the
/// window's own shipped caption. Stated as a deviation, per ADR-0009.
const CONFIRM_SIZE: (f32, f32) = (400.0, 208.0);
const CONFIRM_LOGO_AT: Rect = (126.0, 40.0, 148.0, 52.0);
const CONFIRM_LOGO_DDJ: &str = "mall_pic.ddj";
const CONFIRM_LINE_X: f32 = 20.0;
const CONFIRM_LINE_W: f32 = 360.0;
const CONFIRM_LINE_TOP: f32 = 105.0;
const CONFIRM_LINE_PITCH: f32 = 17.0;
/// L1181, L1184-1186.
const CONFIRM_TITLE_KEY: (&str, &str) = ("UIIT_STT_SILKMALL_ENTER", "Enter Item Mall");
const CONFIRM_LINES: [(&str, &str); 3] = [
    (
        "UIIT_STT_SILKMALL_WARNING_1",
        "You can be attacked by monsters or",
    ),
    (
        "UIIT_STT_SILKMALL_WARNING_2",
        "other players while using Item Mall.",
    ),
    (
        "UIIT_STT_SILKMALL_WARNING_3",
        "It is recommended to use Item Mall in safe zones.",
    ),
];
const CONFIRM_BUTTON_SIZE: (f32, f32) = (76.0, 24.0);
const CONFIRM_YES_AT: (f32, f32) = (122.0, 163.0);
const CONFIRM_NO_AT: (f32, f32) = (202.0, 163.0);
/// L434 / L435.
const CONFIRM_YES_KEY: (&str, &str) = ("UIIT_CTL_YES", "Yes");
const CONFIRM_NO_KEY: (&str, &str) = ("UIIT_CTL_NO", "No");
const CONFIRM_BUTTON_DDJ: &str = "media://interface/ifcommon/com_button.ddj";

/// The mall page.
#[derive(Component)]
pub struct ItemMallRoot;

/// The `Enter Item Mall` confirmation in front of it.
#[derive(Component)]
pub struct ItemMallConfirmRoot;

/// Marks the `Yes` button so one observer can tell the two apart.
#[derive(Component)]
struct ConfirmYes;

/// `UIIT_STT_SILKMALL_SHORT_KEY` (L1224) is literally `Item Mall(F10)`, so F10
/// is the shipped shortcut, not a choice of ours. It is not routed through
/// `GameOptions::key_for` because `OptionSet.csv` carries no rebindable action
/// for the mall — the same reason `Option ( ESC )` has none
/// (`original-world-windows-2026-08-23.md` §20).
const SHORTCUT: KeyCode = KeyCode::F10;

/// F10 opens the confirmation, and closes whatever the mall has open.
pub fn toggle_item_mall(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    chat: Res<ChatState>,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
    open: Query<
        Entity,
        Or<(
            With<ItemMallRoot>,
            With<ItemMallConfirmRoot>,
            With<buy_list::BuyListRoot>,
        )>,
    >,
) {
    if !keys.just_pressed(SHORTCUT) || chat.input_open {
        return;
    }
    if !open.is_empty() {
        for entity in open.iter() {
            commands.entity(entity).despawn();
        }
        return;
    }
    let Ok(camera) = cam_query.single() else {
        warn!("item mall: no 2d camera to attach to");
        return;
    };
    spawn_confirm(&mut commands, &asset_server, &fonts, &ui_strings, camera);
}

/// Builds the `Enter Item Mall` box. Split out of the system so a test can call
/// it.
pub fn spawn_confirm(
    commands: &mut Commands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
    camera: Entity,
) -> Entity {
    let s = hud_scale();
    let title = ui_strings
        .get_or(CONFIRM_TITLE_KEY.0, CONFIRM_TITLE_KEY.1)
        .to_string();
    let window = game_window::spawn_game_window_with(
        commands,
        asset_server,
        fonts,
        camera,
        &title,
        WindowGeometry {
            outer: CONFIRM_SIZE,
            content_at: (0.0, 0.0),
        },
        None,
        (200.0, 180.0),
        s,
        game_window::GameWindowStyle {
            chrome: &game_window::MSGBOX2_WINDOW,
            close_button: false,
            ..default()
        },
    );
    commands
        .entity(window.root)
        .insert((ItemMallConfirmRoot, GlobalZIndex(60)));

    let font = fonts.two.clone();
    let text_px = font_px(0);
    commands.entity(window.content).with_children(|content| {
        content.spawn((
            game_window::abs_node(CONFIRM_LOGO_AT, s),
            ImageNode::new(asset_server.load(format!("{ART}{CONFIRM_LOGO_DDJ}"))),
            Pickable::IGNORE,
        ));
        for (i, (key, fallback)) in CONFIRM_LINES.iter().enumerate() {
            content.spawn((
                game_window::abs_node(
                    (
                        CONFIRM_LINE_X,
                        CONFIRM_LINE_TOP + CONFIRM_LINE_PITCH * i as f32,
                        CONFIRM_LINE_W,
                        CONFIRM_LINE_PITCH,
                    ),
                    s,
                ),
                Text::new(ui_strings.get_or(key, fallback).to_string()),
                TextFont {
                    font: font.clone().into(),
                    font_size: FontSize::Px(text_px),
                    ..default()
                },
                TextColor(Color::WHITE),
                TextLayout::justify(Justify::Center),
                Pickable::IGNORE,
            ));
        }
        for (at, (key, fallback), yes) in [
            (CONFIRM_YES_AT, CONFIRM_YES_KEY, true),
            (CONFIRM_NO_AT, CONFIRM_NO_KEY, false),
        ] {
            let mut button = content.spawn((
                game_window::abs_node(
                    (at.0, at.1, CONFIRM_BUTTON_SIZE.0, CONFIRM_BUTTON_SIZE.1),
                    s,
                ),
                bevy::ui_widgets::Button,
                bevy::picking::hover::Hovered::default(),
                ImageNode::new(asset_server.load(CONFIRM_BUTTON_DDJ)),
                children![(
                    Text::new(ui_strings.get_or(key, fallback).to_string()),
                    TextFont {
                        font: font.clone().into(),
                        font_size: FontSize::Px(text_px),
                        ..default()
                    },
                    TextColor(Color::srgb_u8(255, 247, 202)),
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(100.0),
                        top: Val::Px(4.0 * s),
                        ..default()
                    },
                    TextLayout::justify(Justify::Center),
                    Pickable::IGNORE,
                )],
            ));
            if yes {
                button.insert(ConfirmYes);
            }
            button.observe(on_confirm_button);
        }
    });
    window.root
}

/// `Yes` replaces the box with the page; `No` just closes it.
fn on_confirm_button(
    activate: On<Activate>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
    yes: Query<&ConfirmYes>,
    boxes: Query<Entity, With<ItemMallConfirmRoot>>,
) {
    let accepted = yes.get(activate.entity).is_ok();
    for entity in boxes.iter() {
        commands.entity(entity).despawn();
    }
    if !accepted {
        return;
    }
    let Ok(camera) = cam_query.single() else {
        warn!("item mall: no 2d camera to attach to");
        return;
    };
    spawn_item_mall(&mut commands, &asset_server, &fonts, &ui_strings, camera);
}

/// Builds the page. Split out of the observer so a test can call it.
pub fn spawn_item_mall(
    commands: &mut Commands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
    camera: Entity,
) -> Entity {
    let s = hud_scale();
    let title = ui_strings.get_or(TITLE_KEY.0, TITLE_KEY.1).to_string();
    let window = game_window::spawn_game_window_with(
        commands,
        asset_server,
        fonts,
        camera,
        &title,
        WindowGeometry {
            outer: WINDOW_SIZE,
            // The tree's rects are window-absolute (`_MAIN_BTN` at `18,45`),
            // so the content container sits on the window origin.
            content_at: (0.0, 0.0),
        },
        None,
        ANCHOR_RIGHT_TOP,
        s,
        game_window::GameWindowStyle {
            chrome: &MALL_WND,
            bg_tile: BG_TILE,
            ..default()
        },
    );
    commands
        .entity(window.root)
        .insert((ItemMallRoot, GlobalZIndex(50)));
    commands
        .entity(window.expect_close_button())
        .observe(on_close_button);

    let font = fonts.two.clone();
    let text_px = font_px(0);
    let label = |text: String, rect: Rect, justify: Justify| {
        (
            game_window::abs_node(rect, s),
            Text::new(text),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(text_px),
                ..default()
            },
            TextColor(Color::WHITE),
            TextLayout::justify(justify),
            Pickable::IGNORE,
        )
    };
    commands.entity(window.content).with_children(|content| {
        // Left column: the head button and the seven categories.
        content.spawn((
            game_window::abs_node(MAIN_PAGE_AT, s),
            ImageNode::new(asset_server.load(format!("{ART}mall_mainpage.ddj"))),
            Pickable::IGNORE,
        ));
        for (i, category) in CATEGORIES.iter().enumerate() {
            let top = TAB_TOP + TAB_PITCH * i as f32;
            content.spawn((
                game_window::abs_node((TAB_X, top, TAB_SIZE.0, TAB_SIZE.1), s),
                ImageNode::new(asset_server.load(format!("{ART}mall_item_tab_off.ddj"))),
                Pickable::IGNORE,
            ));
            content.spawn((
                game_window::abs_node(
                    (
                        TAB_ICON_AT.0,
                        TAB_ICON_AT.1 + TAB_PITCH * i as f32,
                        TAB_ICON_SIZE.0,
                        TAB_ICON_SIZE.1,
                    ),
                    s,
                ),
                ImageNode::new(asset_server.load(format!("{ART}{}", category.icon))),
                Pickable::IGNORE,
            ));
            content.spawn(label(
                ui_strings
                    .get_or(category.key, category.fallback)
                    .to_string(),
                (
                    TAB_X + TAB_LABEL_INSET,
                    top + 9.0,
                    TAB_SIZE.0 - TAB_LABEL_INSET,
                    15.0,
                ),
                Justify::Left,
            ));
        }

        // Middle column: the search plate and the empty page behind it.
        content.spawn((
            game_window::abs_node(MID_BG_AT, s),
            ImageNode::new(asset_server.load(format!("{ART}mall_back_wnd.ddj"))),
            Pickable::IGNORE,
        ));
        content.spawn((
            game_window::abs_node(SEARCH_AT, s),
            ImageNode::new(asset_server.load(format!("{ART}mall_find_wnd.ddj"))),
            Pickable::IGNORE,
        ));
        content.spawn(label(
            ui_strings.get_or(SEARCH_KEY.0, SEARCH_KEY.1).to_string(),
            (SEARCH_AT.0, SEARCH_AT.1 + 5.0, SEARCH_AT.2, 15.0),
            Justify::Center,
        ));
        // The article grid. The catalogue is empty because the package list is
        // server content and this tree has no wire path for it, so what gets
        // drawn is the bare grid — see `list::EMPTY_ROWS`.
        list::spawn_list(
            content,
            asset_server,
            fonts,
            ui_strings.get_or(UNIT_STRING_KEY.0, UNIT_STRING_KEY.1),
            NO_CATALOGUE,
            s,
        );

        // Right column: the two plates, the two list buttons, the account block.
        content.spawn((
            game_window::abs_node(RIGHT_FRAME_AT, s),
            ImageNode::new(asset_server.load(format!("{ART}mall_charac_frame.ddj"))),
            Pickable::IGNORE,
        ));
        content.spawn((
            game_window::abs_node(RIGHT_WND_AT, s),
            ImageNode::new(asset_server.load(format!("{ART}mall_charac_wnd.ddj"))),
            Pickable::IGNORE,
        ));
        // The preview area itself — the top of `mall_charac_frame` above the
        // two buttons — stays **empty on purpose**. What the original renders
        // there is the player's own character (a lit 3D figure with its
        // equipment, as `03-mall-page.png` shows), i.e. game content on the
        // avatar render path, not a UI element. Drawing a placeholder portrait
        // would be an invented asset; the plate is drawn and the figure is
        // left to whichever lane wires the avatar preview.
        for (at, (key, fallback), reachable) in [
            (PREVIEW_BUY_AT, PREVIEW_BUY_KEY, false),
            (PREVIEW_ORIGINAL_AT, PREVIEW_ORIGINAL_KEY, false),
            (BUY_LIST_AT, BUY_LIST_KEY, true),
            (RECEIVE_LIST_AT, RECEIVE_LIST_KEY, false),
        ] {
            let art = if reachable {
                LIST_BUTTON_ENABLED_DDJ
            } else {
                LIST_BUTTON_DDJ
            };
            let mut button = content.spawn((
                game_window::abs_node((at.0, at.1, LIST_BUTTON_SIZE.0, LIST_BUTTON_SIZE.1), s),
                ImageNode::new(asset_server.load(format!("{ART}{art}"))),
            ));
            if reachable {
                button.insert((
                    bevy::ui_widgets::Button,
                    bevy::picking::hover::Hovered::default(),
                ));
                button.observe(on_buy_list_button);
            } else {
                button.insert(Pickable::IGNORE);
            }
            content.spawn(label(
                ui_strings.get_or(key, fallback).to_string(),
                (at.0, at.1 + 9.0, LIST_BUTTON_SIZE.0, 15.0),
                Justify::Center,
            ));
        }

        // The account block, authored relative to `GDR_ITEM_MALL_MYINFO`.
        let at = |rect: Rect| (MYINFO_AT.0 + rect.0, MYINFO_AT.1 + rect.1, rect.2, rect.3);
        content.spawn((
            game_window::abs_node(at(MYINFO_BG_AT), s),
            ImageNode::new(asset_server.load(format!("{ART}mall_charac_textbox.ddj"))),
            Pickable::IGNORE,
        ));
        // Name and level: labels exist, values do not — see the module doc.
        content.spawn(label(String::new(), at(MYINFO_NAME_AT), Justify::Left));
        content.spawn(label(String::new(), at(MYINFO_LEVEL_AT), Justify::Right));
        content.spawn(label(
            ui_strings
                .get_or(MYINFO_SILK_KEY.0, MYINFO_SILK_KEY.1)
                .to_string(),
            at(MYINFO_SILK_LABEL_AT),
            Justify::Left,
        ));
        content.spawn(label(
            String::new(),
            at(MYINFO_SILK_VALUE_AT),
            Justify::Right,
        ));
        content.spawn((
            game_window::abs_node(at(MYINFO_SILK_COIN_AT), s),
            ImageNode::new(asset_server.load(format!("{ART}mall_silk_icon.ddj"))),
            Pickable::IGNORE,
        ));
        content.spawn(label(
            ui_strings
                .get_or(MYINFO_AP_KEY.0, MYINFO_AP_KEY.1)
                .to_string(),
            at(MYINFO_AP_LABEL_AT),
            Justify::Left,
        ));
        content.spawn(label(String::new(), at(MYINFO_AP_VALUE_AT), Justify::Right));
        content.spawn(label(
            ui_strings
                .get_or(MYINFO_AP_UNIT_KEY.0, MYINFO_AP_UNIT_KEY.1)
                .to_string(),
            at(MYINFO_AP_UNIT_AT),
            Justify::Right,
        ));
        for (button_at, (key, fallback)) in [(BUY_SILK_AT, BUY_SILK_KEY), (COUPON_AT, COUPON_KEY)] {
            let rect = at((
                button_at.0,
                button_at.1,
                MYINFO_BUTTON_SIZE.0,
                MYINFO_BUTTON_SIZE.1,
            ));
            content.spawn((
                game_window::abs_node(rect, s),
                ImageNode::new(asset_server.load(format!("{ART}{MYINFO_BUTTON_DDJ}"))),
                Pickable::IGNORE,
            ));
            content.spawn(label(
                ui_strings.get_or(key, fallback).to_string(),
                (rect.0, rect.1 + 15.0, rect.2, 15.0),
                Justify::Center,
            ));
        }
    });
    window.root
}

/// `Buying list` opens the sub-window in front of the page — the one path in
/// this window that a measurement clears (§29: it sends nothing). A second
/// click is idempotent: the pane is spawned only if none is open, because the
/// original's button does not toggle.
fn on_buy_list_button(
    _activate: On<Activate>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
    open: Query<Entity, With<buy_list::BuyListRoot>>,
) {
    if !open.is_empty() {
        return;
    }
    let Ok(camera) = cam_query.single() else {
        warn!("item mall: no 2d camera to attach to");
        return;
    };
    buy_list::spawn_buy_list(&mut commands, &asset_server, &fonts, &ui_strings, camera);
}

/// `<sml2>...</sml2>` is the original's rich-text wrapper. We draw plain text,
/// so the wrapper is stripped rather than printed — and stripped only when it
/// is actually there, so a translation without it is unaffected.
fn strip_sml(text: &str) -> String {
    text.trim()
        .strip_prefix("<sml2>")
        .and_then(|rest| rest.strip_suffix("</sml2>"))
        .unwrap_or(text)
        .to_string()
}

/// The (X) closes the page — and with it the `Buying list` pane, which is a
/// child of the page in the original's sense (it cannot outlive it) but a
/// sibling entity here.
fn on_close_button(
    activate: On<Activate>,
    mut commands: Commands,
    parents: Query<&ChildOf>,
    roots: Query<Entity, With<ItemMallRoot>>,
    sub: Query<Entity, With<buy_list::BuyListRoot>>,
) {
    for entity in sub.iter() {
        commands.entity(entity).despawn();
    }
    let mut node = activate.entity;
    loop {
        if roots.get(node).is_ok() {
            commands.entity(node).despawn();
            return;
        }
        match parents.get(node) {
            Ok(parent) => node = parent.parent(),
            Err(_) => return,
        }
    }
}

/// Leaving the world takes the page and its confirmation with it.
fn cleanup_item_mall(
    mut commands: Commands,
    windows: Query<
        Entity,
        Or<(
            With<ItemMallRoot>,
            With<ItemMallConfirmRoot>,
            With<buy_list::BuyListRoot>,
        )>,
    >,
) {
    for entity in windows.iter() {
        commands.entity(entity).despawn();
    }
}

/// Registers the window.
pub struct ItemMallPlugin;

impl Plugin for ItemMallPlugin {
    fn build(&self, app: &mut App) {
        // Scene-gated like every other HUD window: `FontAssets` and
        // `ClientUiStrings` exist only there, and Bevy panics the schedule on a
        // missing `Res` rather than skipping the system.
        app.add_systems(Update, toggle_item_mall.run_if(super::hud_scenes))
            .add_systems(OnExit(SceneState::GameWorld), cleanup_item_mall);
    }
}

#[cfg(test)]
mod test {
    use super::list::LIST_AT;
    use super::*;

    /// The seven tabs sit where the art matched the photograph: `x = 18`,
    /// `y = 75 + 32k`, and the eighth row is *not* part of the window.
    #[test]
    fn the_seven_category_tabs_are_the_measured_ones() {
        assert_eq!(CATEGORIES.len(), 7);
        let tops: Vec<f32> = (0..CATEGORIES.len())
            .map(|i| TAB_TOP + TAB_PITCH * i as f32)
            .collect();
        assert_eq!(tops, vec![75.0, 107.0, 139.0, 171.0, 203.0, 235.0, 267.0]);
        // The pitch is the art's own height, which is why the rows abut.
        assert_eq!(TAB_PITCH, TAB_SIZE.1);
        // The last tab stays inside the left frame (`15,42,124,542`).
        let last_bottom = TAB_TOP + TAB_PITCH * (CATEGORIES.len() as f32 - 1.0) + TAB_SIZE.1;
        assert!(last_bottom < 42.0 + 542.0);
        // And the head button sits directly above the first tab.
        assert!(MAIN_PAGE_AT.1 + MAIN_PAGE_AT.3 <= TAB_TOP);
    }

    /// Every category label is a shipped `UIIT_STT_SILKMALL_*` key, in the
    /// photographed order.
    #[test]
    fn the_categories_are_the_photographed_order() {
        let names: Vec<&str> = CATEGORIES.iter().map(|c| c.fallback).collect();
        assert_eq!(
            names,
            vec![
                "Expendables",
                "Avatar",
                "Pet",
                "Premium",
                "Alchemy",
                "HOT shop",
                "Reserved"
            ]
        );
        for category in CATEGORIES.iter() {
            assert!(
                category.key.starts_with("UIIT_STT_SILKMALL_"),
                "{} is not a shipped mall key",
                category.key
            );
            assert!(category.icon.ends_with("_icon.ddj"));
        }
    }

    /// No button that would move silk is drawn in an enabled state — the
    /// module doc's rule, checked on the two art constants that carry it.
    #[test]
    fn every_unwired_button_uses_the_shipped_disabled_art() {
        assert!(LIST_BUTTON_DDJ.ends_with("_disable.ddj"));
        assert!(MYINFO_BUTTON_DDJ.ends_with("_disable.ddj"));
    }

    /// §29/§30: of the four right-column buttons exactly **one** — `Buying
    /// list` — was measured to do something reachable without a wire path, so
    /// exactly one is drawn with the enabled art. The other three keep the
    /// disabled one, and `Buy silk` keeps it *because* its measured action
    /// leaves the game entirely.
    #[test]
    fn only_the_measured_reachable_button_is_drawn_enabled() {
        assert_eq!(LIST_BUTTON_ENABLED_DDJ, "mall_avata_button.ddj");
        assert!(!LIST_BUTTON_ENABLED_DDJ.ends_with("_disable.ddj"));
        assert_eq!(LIST_BUTTON_DDJ, "mall_avata_button_disable.ddj");
    }

    /// The `Buying list` pane sits inside the page it belongs to, horizontally
    /// centred — which is where the photograph put it, and the arithmetic that
    /// derived its anchor from that measurement.
    #[test]
    fn the_buying_list_pane_is_centred_over_this_page() {
        let (w, _) = buy_list::WINDOW_SIZE;
        assert!(w < WINDOW_SIZE.0);
        assert_eq!(buy_list::ANCHOR_RIGHT_TOP.0, (WINDOW_SIZE.0 - w) / 2.0);
    }

    /// The account block's children stay inside its authored plate, and the
    /// off-screen `_POINT` row (`x = 6000`) is not among them.
    #[test]
    fn the_account_block_children_stay_on_the_plate() {
        let plate = MYINFO_BG_AT;
        for rect in [
            MYINFO_NAME_AT,
            MYINFO_LEVEL_AT,
            MYINFO_SILK_LABEL_AT,
            MYINFO_SILK_VALUE_AT,
            MYINFO_AP_LABEL_AT,
            MYINFO_AP_VALUE_AT,
        ] {
            assert!(
                rect.0 >= 0.0 && rect.0 + rect.2 <= plate.0 + plate.2,
                "{rect:?}"
            );
            assert!(rect.1 >= 0.0, "{rect:?}");
        }
        // The two buttons hang below the plate, as authored (`y = 92` against a
        // plate of height 88) — that is the original, not a slip.
        assert!(BUY_SILK_AT.1 > plate.1 + plate.3);
        assert_eq!(COUPON_AT.1, BUY_SILK_AT.1);
    }

    /// The confirmation's measured geometry: logo centred on the plate, the
    /// two buttons centred as a pair with the photographed 4 px gap.
    #[test]
    fn the_confirm_box_geometry_is_the_measured_one() {
        assert_eq!(CONFIRM_SIZE, (400.0, 208.0));
        let gap = CONFIRM_NO_AT.0 - (CONFIRM_YES_AT.0 + CONFIRM_BUTTON_SIZE.0);
        assert_eq!(gap, 4.0);
        let pair = CONFIRM_BUTTON_SIZE.0 * 2.0 + gap;
        let centred = (CONFIRM_SIZE.0 - pair) / 2.0;
        assert!((CONFIRM_YES_AT.0 - centred).abs() <= 1.0);
        assert_eq!(CONFIRM_YES_AT.1, CONFIRM_NO_AT.1);
        // The logo sits on the plate's own top inset.
        assert_eq!(CONFIRM_LOGO_AT.1, game_window::MSGBOX2_WINDOW.top_h);
        assert_eq!(CONFIRM_LINES.len(), 3);
    }

    /// The chrome is the family the window's own registration names, with the
    /// piece extents its art actually has.
    #[test]
    fn the_page_uses_the_declared_mall_chrome() {
        assert_eq!(MALL_WND.dir, "media://interface/frame/mall_wnd_");
        assert_eq!(
            (MALL_WND.side_w, MALL_WND.top_h, MALL_WND.bottom_h),
            (40.0, 68.0, 48.0)
        );
        assert_ne!(MALL_WND.dir, game_window::MFRAME_WND.dir);
        assert_eq!(WINDOW_SIZE, (800.0, 600.0));
    }

    /// The article grid sits inside the middle plate the tree authors for it,
    /// and it starts below the plate's own top line — which is what replaced
    /// §7's welcome-line guess.
    #[test]
    fn the_article_grid_stays_on_the_middle_plate() {
        assert!(LIST_AT.0 > MID_BG_AT.0);
        assert!(LIST_AT.1 > MID_BG_AT.1);
        assert!(LIST_AT.0 + LIST_AT.2 <= MID_BG_AT.0 + MID_BG_AT.2);
        assert!(LIST_AT.1 + LIST_AT.3 <= MID_BG_AT.1 + MID_BG_AT.3);
    }

    /// The preview's two buttons share the list buttons' columns and sit one
    /// button-height-plus-gutter above them, as measured.
    #[test]
    fn the_preview_buttons_sit_above_the_list_buttons() {
        assert_eq!(PREVIEW_BUY_AT.0, BUY_LIST_AT.0);
        assert_eq!(PREVIEW_ORIGINAL_AT.0, RECEIVE_LIST_AT.0);
        assert_eq!(PREVIEW_BUY_AT.1, PREVIEW_ORIGINAL_AT.1);
        assert_eq!(BUY_LIST_AT.1 - PREVIEW_BUY_AT.1, 39.0);
        // Both hang inside the preview frame, which is what makes them part of
        // the preview rather than a third row of list buttons.
        assert!(PREVIEW_BUY_AT.1 + LIST_BUTTON_SIZE.1 <= RIGHT_FRAME_AT.1 + RIGHT_FRAME_AT.3);
    }

    /// The markup wrapper is stripped, and only when present.
    #[test]
    fn the_sml_wrapper_is_stripped() {
        assert_eq!(strip_sml("<sml2>Welcome</sml2>"), "Welcome");
        assert_eq!(strip_sml("Welcome"), "Welcome");
    }
}
