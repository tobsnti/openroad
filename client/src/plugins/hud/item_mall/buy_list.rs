//! `CNIFSilkBuyList` — the `Buying list` sub-window of the Item Mall.
//!
//! Idea: this is the first window in the tree built **entirely from the
//! fourth-generation `res_ui/*.2dt` table** rather than from `resinfo/*.txt`,
//! and the reason it may be is a photograph. The classic tree describes the
//! same pane twice (`ifitemmall.txt:336` `CIFSilkBuyList` with **14** rows and
//! **three** co-anchored period combos, `hud-item-mall.md` §3.6), so "which of
//! the three descriptions does v1.188 render?" was an open question
//! (`hud-item-mall.md` §9-U5). The capture answers it: the photograph shows
//! **one** tab and **11** rows, which is `silksearch.2dt` and not the classic
//! block — see the four template matches below.
//!
//! **Where the numbers come from.** `Media/res_ui/silksearch.2dt`, 63,444 B =
//! `4 + 65*976` (stride check passes), read struct-aware at the fixed offsets
//! of `docs/re/formats/newinterface-2dt.md` §3 (Name@+0, Image@+0x40,
//! Background@+0x140, key@+0x240, type@+0x340, id@+0x344, rect@+0x35C). Entry
//! `[0]` is the root `CNIFSilkBuyList`, id 38, `interface\frame\mall_ms_wnd_`,
//! key `UIIT_STT_SILKMALL_BUY_LIST`, rect `334,140,462,527`. Every rect in this
//! file is that root's rect subtracted from the entry's, so `(0,0)` here is the
//! window's top-left, and each constant names the entry index it came from.
//!
//! **The photograph, and how the layout was checked**
//! (`artifacts/capture/orders/done/260-itemmall/btn-buying.png`, 806x629 with
//! the client area at image offset `(+3,+26)`;
//! `docs/re/ui/original-world-windows-2026-08-23.md` §29). Four independent
//! pieces of art were template-matched over their opaque pixels and **all four
//! land at delta `(0,0)`** against a single window origin of image `(172,63)`,
//! i.e. client `(169,37)`:
//!
//! | art | authored rect | expected image | matched | error |
//! |---|---|---|---|---|
//! | `mall_sub_tab_on.ddj` 96x24 | `[4]` `(22,36)` | `(194,99)` | `(194,99)` | 18.9 |
//! | `mall_sub_box.ddj` 424x116 | `[8]` `(20,66)` | `(192,129)` | `(192,129)` | 14.8 |
//! | `mall_button.ddj` 88x24 | `[12]` `(285,118)` | `(457,181)` | `(457,181)` | 21.2 |
//! | `mall_subj_01.ddj` 68x24 | `[63]` `(22,205)` | `(194,268)` | `(194,268)` | 21.4 |
//! | `com_windowclose.ddj` 16x16 | shared chrome | `(608,73)` | `(608,73)` | **0.32** |
//!
//! The residual errors are the *labels drawn on top* of the art (the close
//! button carries none, which is why it scores 0.32 — that is the run's
//! positive control that the matcher and the decoder are sound). The two other
//! column headers, `mall_subj_02/03`, do **not** match: in this capture they
//! sit behind the `Confirmation window: You have been disconnected` box, which
//! covers image `x 250..555, y 255..395`. An occluded piece that fails to match
//! is not evidence of a wrong rect, so they are transcribed from the table and
//! marked as such rather than counted among the confirmations.
//!
//! **The close button pins the window width.** Its box is
//! `right: 10, top: title_y + 4` off the window
//! (`game_window::close_button_node`), so the exact match at window-relative
//! `(436,10)` yields `462 - 10 - 16 = 436` for `WINDOW_SIZE.0 = 462` and
//! `title_y = 6`. Both are read off the picture, not chosen.
//!
//! **The anchor is measured, and it turns out to be centred.** Window origin
//! client `(169,37)`, and `(800 - 462) / 2 = 169` exactly — the sub-window is
//! horizontally centred over the 800-wide mall page. The authored screen
//! position `334,140` is *not* usable: `140 + 527 = 667` does not fit the
//! 800x600 client, so that pair is authored for a different resolution.
//!
//! **Display only, and the packets say that is right.** Opening this window
//! sends **nothing**: `packet_dump/proxy/c2s/` held 16 files before the click
//! and 16 after (§29). So the window is spawned client-side and no button in it
//! has an observer — `Silk Charge History` would be the first wire path
//! (`UIIT_CTL_SILK_INQUIRY_INQUIRY`), and this lane has no measured silk
//! opcode. The 11 row bars are drawn empty for the same reason the mall's
//! article grid is: the rows are server content.

use bevy::prelude::*;

use crate::assets::FontAssets;
use crate::plugins::hud::game_window::{self, ChromeFamily, WindowGeometry};
use crate::plugins::hud::scale::{font_px, hud_scale};
use crate::plugins::hud::widgets::combo_box::{self, ComboRow, ComboRowState, ComboStyle};
use crate::plugins::textdata::ClientUiStrings;

use super::{strip_sml, Rect, ART};

/// `interface\frame\mall_ms_wnd_`, named by `silksearch.2dt[0]`. Piece extents
/// measured off the eight decoded DDS headers: sides 32 wide, `mid_up` 44 tall,
/// `mid_down` 24 — a smaller ring than the mall page's `mall_wnd_` (40/68/48).
///
/// The `vis_*` fields are the full piece extents because every pixel of all
/// eight pieces is painted: minimum alpha is **219** over the six body pieces
/// and 126 over the two upper corners, so this family is a translucent panel
/// like `mall_wnd_`, not a thin ring.
///
/// `title_y = 6` is the close button's exact match (see the module doc);
/// `title_h = 19` puts the band's bottom on the painted rule at row **25** of
/// `mall_ms_wnd_mid_up` (its row luminance profile is `1: 128`, `2..24: 0`,
/// `25: 127` — two rules with a black band between them). `tile_mid_down` is
/// false because `mall_ms_wnd_mid_down` has no horizontal period: its column
/// profile has a peak-to-peak spread of 1.3/255, i.e. it is flat.
const MALL_MS_WND: ChromeFamily = ChromeFamily {
    dir: "media://interface/frame/mall_ms_wnd_",
    side_w: 32.0,
    top_h: 44.0,
    bottom_h: 24.0,
    vis_left: 32.0,
    vis_right: 32.0,
    vis_top: 44.0,
    vis_bottom: 24.0,
    title_y: 6.0,
    title_h: 19.0,
    tile_mid_down: false,
};

/// `silksearch.2dt[0]` `334,140,462,527`.
pub(super) const WINDOW_SIZE: (f32, f32) = (462.0, 527.0);
/// See the module doc: measured at client `(169,37)`, which is `(800-462)/2`.
pub(super) const ANCHOR_RIGHT_TOP: (f32, f32) = (169.0, 37.0);
/// `[0]` key, textuisystem L1159.
const TITLE_KEY: (&str, &str) = ("UIIT_STT_SILKMALL_BUY_LIST", "Buying list");

/// `[3]` `interface\frame\mall_sub_wnd04_`, rect `(11,57,440,458)`. The pieces
/// are 32x32 corners.
const INNER_FRAME_AT: Rect = (11.0, 57.0, 440.0, 458.0);
const INNER_FRAME_CORNER: f32 = 32.0;
/// `[2]` `com_bg_tile_p.ddj`, the interior tile over the frame's client area.
const INNER_TILE_AT: Rect = (33.0, 89.0, 397.0, 414.0);

/// `[4]`, Type 16 `TabButton`, `mall_sub_tab_on.ddj` (96x24 — art and rect
/// agree). **One** tab: `silkrecievelist.2dt` has two, which is how the
/// photograph's single `Inquire` tab identifies this file as the one v1.188
/// renders. Drawn in the `_on` state because it is the only tab there is.
const TAB_AT: Rect = (22.0, 36.0, 96.0, 24.0);
/// L3487.
const TAB_KEY: (&str, &str) = ("UIIT_CTL_SILK_INQUIRY", "Inquire");

/// `[8]` `mall_sub_box.ddj` (424x116 — art and rect agree), the plate the
/// inquiry row sits on.
const PANEL_AT: Rect = (20.0, 66.0, 424.0, 116.0);

/// `[15]`, Type 18 `CIFPMLEx` (the rich-text control), rect `(33,88,395,15)`
/// and **no string key of its own** — the key is fed in code. The photograph
/// prints `You can check your Silk charge history for the most recent 3
/// months.` there, which is `UIIT_STT_SILK_INQUIRY_MONTH_SILK` (L3493) *and no
/// other line in the file*: the three neighbouring `_MONTH_*` variants read
/// `present`, `received` and `point`. So the binding is the photograph's, not a
/// guess. The shipped value is wrapped in `<sml2>…</sml2>`, which is why it
/// goes through [`super::strip_sml`].
const INTRO_AT: Rect = (33.0, 88.0, 395.0, 15.0);
const INTRO_KEY: (&str, &str) = (
    "UIIT_STT_SILK_INQUIRY_MONTH_SILK",
    "<sml2>You can check your Silk charge history for the most recent 3 months.</sml2>",
);

/// `[9]` `(34,124,129,16)`, L3492.
const PERIOD_LABEL_AT: Rect = (34.0, 124.0, 129.0, 16.0);
const PERIOD_LABEL_KEY: (&str, &str) = ("UIIT_STT_SILK_INQUIRY_PERIOD", "Set Inquiry Period");

/// `[10]`, Type 12 `ComboBox`, `(170,122,102,20)` — the 20 px height the shared
/// widget asserts for 26 of 26 classic sites holds here too.
///
/// **This rect is the whole control, not the painted field**, and the capture
/// says so to the pixel. In `263-itemmall-combo/04-combo-open.png` (window
/// origin image `(172,63)`, module doc above) the painted field runs image
/// x 342..423 and the drop arrow x 425..442, i.e. window-relative x 170..251 —
/// `170 + 102 = 272` … `443 - 172 = 271`, the same last column. Decomposed:
/// `82 (field) + 1 (gap) + 18 (arrow) + 1 (gap) = 102`, exactly the authored
/// width. The arrow lives *inside* the authored rect here; see
/// [`PERIOD_FIELD_W`].
const PERIOD_COMBO_AT: Rect = (170.0, 122.0, 102.0, 20.0);

/// The painted field, `102 - 1 - 18 - 1 = 82` px — the measured extent of the
/// field art in the capture (image x 342..423), derived from the authored rect
/// and the widget's own arrow metrics rather than restated as a literal.
///
/// Note for the widget: `combo_box::PAINTED_EXTRA_W` reads this same 82 as
/// "4 px wider than authored" because it attributes the field to
/// `resinfo/ifitemmallshop.txt:1209` (`165,85,78,20`). The photographed pane is
/// the fourth-generation `silksearch.2dt` one (module doc: one tab, 11 rows),
/// whose entry `[10]` authors 102 — and 102 closes exactly on field + arrow.
/// So there is no unexplained overdraw at this site; the field is 82 because
/// the arrow takes the remaining 20.
const PERIOD_FIELD_W: f32 =
    PERIOD_COMBO_AT.2 - combo_box::DROP_ARROW_SIZE - 2.0 * combo_box::DROP_ARROW_GAP;

/// The arrow tile, inside the control, drawn **at its own 20x20 size** so it is
/// not resampled: `com_qst_downarrow_button.ddj` carries a 1 px transparent
/// margin around an 18x18 button (`combo_box::DROP_ARROW_SIZE` /
/// `DROP_ARROW_GAP`), so a tile placed at `(82,0)` puts its opaque pixels at
/// `(83,1)` — the capture's arrow at image `(425,186)` against a field origin
/// of `(342,185)`. `82 + 20 = 102` closes the authored rect exactly.
const PERIOD_ARROW_AT: Rect = (
    PERIOD_FIELD_W,
    0.0,
    combo_box::DROP_ARROW_SIZE + 2.0 * combo_box::DROP_ARROW_GAP,
    combo_box::DROP_ARROW_SIZE + 2.0 * combo_box::DROP_ARROW_GAP,
);

/// The rows this combo lists — **four**, spelled out here rather than sliced
/// out of the text file, which is the standing rule the shared widget states in
/// its module doc and checks in
/// `combo_box::tests::the_shipped_text_block_is_not_the_row_list`.
///
/// `textuisystem.txt` L3498-L3503 is a *contiguous* `UIIT_CTL_SILK_INQUIRY_*`
/// block of six lines — `1 day`, `7 days`, **`28 days`**, **`Permanence`**,
/// `1 month`, `3 months` — and the original's open box shows only four of them
/// (`original-world-windows-2026-08-23.md` §31, image
/// `263-itemmall-combo/04-combo-open.png`, four entries read off the picture).
/// So this list is L3498, L3499, **L3502, L3503**: the block minus the two
/// lines the client does not offer. Taking the block whole would add two rows
/// the original never shows, and both read plausibly enough that nobody would
/// notice.
///
/// The photograph has the closed field on the first of them, `1 day`, which is
/// therefore [`PeriodCombo::selected`]'s initial value.
///
/// **[V] The binary says the same thing, independently of the photograph**
/// (`sro_client.exe`, SHA-256 `d77406cc…aa92d5` — the build this fact is
/// checkable against; VA =
/// file offset + `0x400000`). The keys are UTF-16LE literals in `.rdata`, and
/// the fill site is an unrolled run of four identical blocks `0x28` bytes
/// apart, each one looking its key up in the string table and appending the
/// result to the same control. Only the addresses are recorded here — the
/// instruction transcript behind them is the evidence class AGENTS.md keeps
/// out of the repository:
///
/// | VA of the `push` | key at | key |
/// |---|---|---|
/// | `0x7d4c12` | `0xdcb994` | `UIIT_CTL_SILK_INQUIRY_ONEDAY` |
/// | `0x7d4c3a` | `0xdcb95c` | `UIIT_CTL_SILK_INQUIRY_WEEK` |
/// | `0x7d4c62` | `0xdcb924` | `UIIT_CTL_SILK_INQUIRY_MONTH` |
/// | `0x7d4c8a` | `0xdcb8e0` | `UIIT_CTL_SILK_INQUIRY_THREE_MONTH` |
///
/// Four `add`s on one control, in exactly this order — so the order below is
/// the original's, not a reading of the picture's row order. The same run
/// repeats at `0x7d63de`, `0x7ed00b` and `0x7ed30d` (the sibling silk panes).
/// And the two shipped lines this list drops, `UIIT_CTL_SILK_INQUIRY_MONTH_DAY`
/// (`28 days`) and `_PERMANENCE`, occur **nowhere in the image** — positive
/// control on the same read path: all four keys above do occur, as does the
/// neighbouring `UIIT_CTL_SILK_INQUIRY` at `0xdcb8b4`. A v1.188 client cannot
/// show a row whose key it never mentions.
const PERIOD_OPTIONS: [(&str, &str); 4] = [
    ("UIIT_CTL_SILK_INQUIRY_ONEDAY", "1 day"),
    ("UIIT_CTL_SILK_INQUIRY_WEEK", "7 days"),
    ("UIIT_CTL_SILK_INQUIRY_MONTH", "1 month"),
    ("UIIT_CTL_SILK_INQUIRY_THREE_MONTH", "3 months"),
];

/// `[12]`, Type 4 `Button`, `mall_button.ddj` (88x24 — art and rect agree),
/// L3491.
///
/// **No observer, and this is the measurement that says so:** §29's packet
/// counter-check shows the *window* sends nothing, and names this button as the
/// place where a wire path would begin. We have no silk opcode measured, so the
/// button is drawn and inert rather than wired to a guess.
const HISTORY_BUTTON_AT: Rect = (285.0, 118.0, 88.0, 24.0);
const HISTORY_BUTTON_KEY: (&str, &str) = ("UIIT_CTL_SILK_INQUIRY_INQUIRY", "Silk Charge History");

/// `[14]` `com_blacksquare_` `(33,156,395,20)` with `[13]`'s `com_bg_tile_e`
/// bed at `(37,160,387,12)` — the empty display strip under the inquiry row.
/// (`[61]` repeats `[14]`'s rect exactly; the shared-rect `if`-arm idiom.)
const RESULT_WELL_AT: Rect = (33.0, 156.0, 395.0, 20.0);
const RESULT_BED_AT: Rect = (37.0, 160.0, 387.0, 12.0);

/// `[6]` `com_blacksquare_` `(22,202,422,282)`, the table's bezel.
const TABLE_WELL_AT: Rect = (22.0, 202.0, 422.0, 282.0);
/// `com_blacksquare_` ships **six** 4x4 pieces (`left/right` x `up/side/down`)
/// and no `mid_*` — there is no strip art to tile, because the interior is the
/// bg tile. One stretched corner piece is therefore the whole plate, which is
/// the treatment `hud/stall_network.rs:205` already established in this tree.
const BLACKSQUARE: &str = "media://interface/ifcommon/com_blacksquare_left_up.ddj";

/// `[63]`/`[62]`/`[64]`, Type 5 statics on purpose-made header art. `[63]` is
/// template-matched at delta `(0,0)`; the other two are occluded in the only
/// capture and are transcribed from the table (see the module doc).
/// L3504 / L3506 / L3507. Note `mall_subj_03.ddj` is 180 px of art in a 162 px
/// rect — the authored rect wins, as everywhere else here.
const COLUMNS: [(Rect, &str, &str, &str); 3] = [
    (
        (22.0, 205.0, 68.0, 24.0),
        "mall_subj_01.ddj",
        "UIIT_STT_SILK_INQUIRY_NUMBER",
        "Number",
    ),
    (
        (88.0, 205.0, 176.0, 24.0),
        "mall_subj_02.ddj",
        "UIIT_STT_SILK_INQUIRY_SILK_POINT",
        "Silk (Point)",
    ),
    (
        (264.0, 205.0, 162.0, 24.0),
        "mall_subj_03.ddj",
        "UIIT_STT_SILK_INQUIRY_DATE",
        "Date",
    ),
];

/// `[17]`, `[21]`, … — **11** `CNIFSilkRecieveListSlot` rows, `402x24` at
/// `x = 22`, `y = 227, 250, … 477`, pitch **23**. The classic block has 14; the
/// photograph has 11, which is this file.
const ROW_COUNT: usize = 11;
const ROW_AT: Rect = (22.0, 227.0, 402.0, 24.0);
const ROW_PITCH: f32 = 23.0;
/// The `Background` the table names is the `_left` piece directly (4x24), not
/// the 9-slice prefix — `hud-item-mall.md` §3.6 records the same for both
/// fourth-generation twins.
const ROW_DDJ: &str = "media://interface/ifcommon/com_bar01_left.ddj";

/// `[7]`, Type 13 `CNIFVirticalScroll` `(425,220,16,230)`. Drawn as its bed
/// only: the scroll thumb's travel is a function of a row count we do not have.
const SCROLL_AT: Rect = (425.0, 220.0, 16.0, 230.0);

/// `[5]` `(27,493,48,15)` L3508 `Total:` and `[16]` `(60,491,180,18)`, the
/// value beside it. The photograph reads `Total: 0Silk`; the `0` is a *result*
/// and this window has no query, so the value stays empty rather than claiming
/// a balance of zero — the same rule the mall page applies to its account
/// block.
const TOTAL_LABEL_AT: Rect = (27.0, 493.0, 48.0, 15.0);
const TOTAL_LABEL_KEY: (&str, &str) = ("UIIT_STT_SILK_INQUIRY_SUM", "Total:");
const TOTAL_VALUE_AT: Rect = (60.0, 491.0, 180.0, 18.0);

/// The sub-window in front of the mall page.
#[derive(Component)]
pub struct BuyListRoot;

/// Builds the `Buying list` pane. Split out of the observer so a test can call
/// it, like the page and the confirmation box.
pub fn spawn_buy_list(
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
            // `silksearch.2dt` rects are window-absolute once the root's own
            // origin is subtracted, so the content container sits on it.
            content_at: (0.0, 0.0),
        },
        Some(game_window::InnerFrame {
            dir: "media://interface/frame/mall_sub_wnd04_",
            rect: INNER_FRAME_AT,
            corner: INNER_FRAME_CORNER,
        }),
        ANCHOR_RIGHT_TOP,
        s,
        game_window::GameWindowStyle {
            chrome: &MALL_MS_WND,
            ..default()
        },
    );
    commands
        .entity(window.root)
        // above the mall page's 50, below the confirmation box's 60
        .insert((BuyListRoot, GlobalZIndex(55)));
    commands
        .entity(window.expect_close_button())
        .observe(on_close_button);
    // A press anywhere else in the sub-window closes an open drop-down —
    // measured, see [`on_window_press`]. It sits on the root because that is
    // the entity every press inside the window bubbles to.
    commands.entity(window.root).observe(on_window_press);

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
    let plate = |ddj: String, rect: Rect| {
        (
            game_window::abs_node(rect, s),
            ImageNode::new(asset_server.load(ddj)),
            Pickable::IGNORE,
        )
    };

    commands.entity(window.content).with_children(|content| {
        content.spawn(plate(
            "media://interface/ifcommon/bg_tile/com_bg_tile_p.ddj".to_string(),
            INNER_TILE_AT,
        ));

        // The single tab, drawn `_on`: it is the only one this file declares.
        content.spawn(plate(format!("{ART}mall_sub_tab_on.ddj"), TAB_AT));
        content.spawn(label(
            ui_strings.get_or(TAB_KEY.0, TAB_KEY.1).to_string(),
            (TAB_AT.0, TAB_AT.1 + 5.0, TAB_AT.2, 15.0),
            Justify::Center,
        ));

        // The inquiry row: plate, explanation, period combo, history button.
        content.spawn(plate(format!("{ART}mall_sub_box.ddj"), PANEL_AT));
        content.spawn(label(
            strip_sml(ui_strings.get_or(INTRO_KEY.0, INTRO_KEY.1)),
            INTRO_AT,
            Justify::Center,
        ));
        content.spawn(label(
            ui_strings
                .get_or(PERIOD_LABEL_KEY.0, PERIOD_LABEL_KEY.1)
                .to_string(),
            PERIOD_LABEL_AT,
            Justify::Left,
        ));
        // The control: the authored 102x20 rect is the hit area, and it carries
        // the painted 82 px field, the 20x20 arrow tile and — while open — the
        // drop-down panel as children, so every rect below is in the field's
        // own space and `combo_box::open_panel_rect` applies unshifted.
        let caption = ui_strings
            .get_or(PERIOD_OPTIONS[0].0, PERIOD_OPTIONS[0].1)
            .to_string();
        let field_at: Rect = (0.0, 0.0, PERIOD_FIELD_W, PERIOD_COMBO_AT.3);
        content
            .spawn((
                PeriodCombo::default(),
                game_window::abs_node(PERIOD_COMBO_AT, s),
                Pickable::default(),
            ))
            .with_children(|control| {
                control
                    .spawn(combo_box::combo_field(field_at, s))
                    .with_children(|field| {
                        field.spawn((
                            combo_box::combo_caption(
                                fonts,
                                field_at,
                                caption,
                                ComboStyle::default(),
                                s,
                            ),
                            PeriodCaption,
                        ));
                    });
                control.spawn(plate(
                    format!("media://{}", combo_box::DROP_ARROW_DDJ),
                    PERIOD_ARROW_AT,
                ));
            })
            .observe(on_period_press);
        content.spawn(plate(format!("{ART}mall_button.ddj"), HISTORY_BUTTON_AT));
        content.spawn(label(
            ui_strings
                .get_or(HISTORY_BUTTON_KEY.0, HISTORY_BUTTON_KEY.1)
                .to_string(),
            (
                HISTORY_BUTTON_AT.0,
                HISTORY_BUTTON_AT.1 + 5.0,
                HISTORY_BUTTON_AT.2,
                15.0,
            ),
            Justify::Center,
        ));
        content.spawn(plate(BLACKSQUARE.to_string(), RESULT_WELL_AT));
        content.spawn(plate(
            "media://interface/ifcommon/bg_tile/com_bg_tile_e.ddj".to_string(),
            RESULT_BED_AT,
        ));

        // The table: bezel, three column headers, 11 empty row bars, the bed
        // of the vertical scroll.
        content.spawn(plate(BLACKSQUARE.to_string(), TABLE_WELL_AT));
        for (rect, ddj, key, english) in COLUMNS {
            content.spawn(plate(format!("{ART}{ddj}"), rect));
            content.spawn(label(
                ui_strings.get_or(key, english).to_string(),
                (rect.0, rect.1 + 5.0, rect.2, 15.0),
                Justify::Center,
            ));
        }
        for i in 0..ROW_COUNT {
            content.spawn(plate(
                ROW_DDJ.to_string(),
                (
                    ROW_AT.0,
                    ROW_AT.1 + ROW_PITCH * i as f32,
                    ROW_AT.2,
                    ROW_AT.3,
                ),
            ));
        }
        content.spawn(plate(BLACKSQUARE.to_string(), SCROLL_AT));

        content.spawn(label(
            ui_strings
                .get_or(TOTAL_LABEL_KEY.0, TOTAL_LABEL_KEY.1)
                .to_string(),
            TOTAL_LABEL_AT,
            Justify::Left,
        ));
        content.spawn(label(String::new(), TOTAL_VALUE_AT, Justify::Left));
    });
    window.root
}

/// The (X) closes the sub-window and leaves the mall page standing, which is
/// what the original does — §29 had to close it this way between clicks.
fn on_close_button(
    _activate: On<bevy::ui_widgets::Activate>,
    mut commands: Commands,
    open: Query<Entity, With<BuyListRoot>>,
) {
    for entity in open.iter() {
        commands.entity(entity).despawn();
    }
}

// --- The drop-down, wired ----------------------------------------------------
//
// Idea: `widgets::combo_box` draws and nothing else — it has no idea where the
// pointer is, and its rows are spawned `Pickable::IGNORE`. So the hit logic
// lives here, in the window, and it is *geometry*, not picking: a pointer
// position is turned into a row index with the widget's own `list_row_rect`,
// which is the same arithmetic that placed the rows. Two consequences worth
// stating, because both are measurements rather than taste:
//
// * **Press does not select, release does** (§33: the pressed row wears
//   exactly the hover's two colours, panel difference against the hover state
//   `0`). So `Press` only opens/closes, and `Release` over a row takes the
//   value — which is also what §32 photographed (`04-after-select.png`: the
//   field reads `7 days` afterwards and the list is gone).
// * **Closing without selecting is measured too** (§34, order
//   `264-combo-close`): a second press on the field *and* a press elsewhere in
//   the sub-window both erase the panel completely (identical panel-area
//   difference, mean 15.66 / maximum 255) and both leave the value at
//   `1 day`. `ESC` was not measured and is therefore not wired — see
//   [`on_window_press`].
// * **The highlight follows the pointer and never sticks to the selection**
//   (§32: the whole open panel holds 844 highlight pixels and every one of them
//   is in the row under the cursor, with the *selected* row unmarked). So the
//   hover walk sets exactly one row `Hovered` and all others `Idle`; there is
//   no third "selected" paint to apply.
//
// **No wire.** Choosing a period changes the caption and nothing else: a query
// would be `UIIT_CTL_SILK_INQUIRY_INQUIRY`'s job, and §29 measured that this
// window sends nothing at all (`packet_dump/proxy/c2s/`: 16 files before, 16
// after). The first silk opcode is not going to be guessed here.

/// The drop-down's state, on the control node (the authored 102x20 rect).
#[derive(Component, Default)]
pub struct PeriodCombo {
    /// Index into [`PERIOD_OPTIONS`]. `0` = `1 day`, which is what the
    /// photograph shows in the closed field.
    selected: usize,
}

/// The caption inside the field, so a selection can rewrite it.
#[derive(Component)]
struct PeriodCaption;

/// The open list's panel. Also the "is it open?" flag: the panel exists exactly
/// while the list is open, so there is no second copy of that state to keep in
/// step with the entity.
#[derive(Component)]
struct PeriodList;

/// Which row of the open list a point lands on, in the panel's **padding box**
/// coordinates and in widget units (i.e. before the HUD scale). Pure, so the
/// row arithmetic can be tested without a pointer.
///
/// The rects come from the widget rather than from a local copy: a row is
/// `combo_box::list_row_rect`, which is exactly what
/// [`combo_box::combo_list_row`] used to place it. The gaps between rows
/// (the 5/4 px insets and the 4 px side ring) are deliberately *not* a hit —
/// pressing the panel's frame does nothing, which is the same "press does not
/// select" the capture shows.
fn row_at(local: Vec2, rows: usize) -> Option<usize> {
    (0..rows).find(|&index| {
        let (x, y, w, h) = combo_box::list_row_rect(PERIOD_FIELD_W, index);
        local.x >= x && local.x < x + w && local.y >= y && local.y < y + h
    })
}

/// [`row_at`] for a live pointer: converts the pointer's logical viewport
/// position into the panel's padding box.
///
/// `UiGlobalTransform` holds the node's **centre in physical pixels** and
/// `ComputedNode::size`/`border` are physical too, so both are taken back to
/// logical with the node's own `inverse_scale_factor` before the pointer is
/// subtracted; the remaining division by the HUD scale undoes `abs_node`. The
/// border is subtracted because Bevy lays absolutely positioned children out
/// against the parent's padding box, and `combo_list_panel` draws a 1 px frame
/// on three sides — so the rows sit one pixel below the panel's own top edge.
fn row_under_pointer(
    pointer: Vec2,
    node: &ComputedNode,
    transform: &UiGlobalTransform,
    rows: usize,
    s: f32,
) -> Option<usize> {
    let inv = node.inverse_scale_factor();
    let top_left = transform.translation * inv - node.size() * inv * 0.5;
    let border = node.border.min_inset * inv;
    row_at((pointer - top_left - border) / s, rows)
}

/// Press on the control (field or arrow): open the list, or close it again.
///
/// **The second press is measured now, it is no longer [S]** (§34, order
/// `artifacts/capture/orders/done/264-combo-close/`): in
/// `02-second-click.png` the list is gone and the field still reads `1 day` —
/// panel-area difference against the open state `01-open.png` is mean 15.66,
/// maximum **255**, i.e. the drop-down vanishes completely and leaves no
/// remainder. Closing this way does **not** take a value; only a release over
/// a row does (§32, [`on_list_release`]).
///
/// The press is kept from bubbling because the window root closes the list too
/// ([`on_window_press`]): this event *is* a press in the window, and letting
/// it reach the root would queue a second `despawn` for the panel this
/// observer already despawned — observer queries see the entity until the
/// command queue is applied, so the duplicate is only caught at flush time.
/// Stopping here also states the intent: the control owns its own toggle.
fn on_period_press(
    mut press: On<Pointer<Press>>,
    mut commands: Commands,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    controls: Query<Entity, With<PeriodCombo>>,
    open: Query<Entity, With<PeriodList>>,
) {
    press.propagate(false);
    if press.event.button != PointerButton::Primary {
        return;
    }
    if let Ok(panel) = open.single() {
        commands.entity(panel).despawn();
        return;
    }
    // One `Buying list` pane exists at a time (`on_buy_list_button` is
    // idempotent), so its one control is the one that was pressed.
    let Ok(control) = controls.single() else {
        return;
    };
    let s = hud_scale();
    let rows = PERIOD_OPTIONS.len();
    let panel = combo_box::open_panel_rect(PERIOD_FIELD_W, rows);
    commands.entity(control).with_children(|control| {
        control
            .spawn((
                combo_box::combo_list_panel(panel, s),
                PeriodList,
                Pickable::default(),
            ))
            .observe(on_list_press)
            .observe(on_list_move)
            .observe(on_list_out)
            .observe(on_list_release)
            .with_children(|list| {
                list.spawn(combo_box::combo_list_inset(panel, s));
                for (index, (key, english)) in PERIOD_OPTIONS.into_iter().enumerate() {
                    list.spawn(combo_box::combo_list_row(
                        &fonts,
                        PERIOD_FIELD_W,
                        index,
                        ui_strings.get_or(key, english).to_string(),
                        ComboRowState::Idle,
                        s,
                    ));
                }
            });
    });
}

/// A press anywhere else in the sub-window closes an open list and **keeps the
/// value** — measured, not assumed (§34, order
/// `artifacts/capture/orders/done/264-combo-close/`): in
/// `03-click-elsewhere.png` the pointer went down on the sub-window beside the
/// control, the list is gone and the field still reads `1 day`. The
/// panel-area difference against the open `01-open.png` is mean 15.66,
/// maximum **255** — *bit for bit the same pair of numbers as the second press
/// on the field*, so both ways erase the drop-down completely and neither of
/// them selects anything.
///
/// **`ESC` is deliberately not wired here.** It was **not** measured: in this
/// client build the key terminates the process (§20, reproduced three times),
/// so the observation order stopped short of it rather than spend the session.
/// An unmeasured key must not be implemented as if it had been seen, so the
/// window has no keyboard path to close the list at all — the gap is honest.
///
/// This observer catches the presses that bubble to the window root: the
/// control itself stops propagation ([`on_period_press`]) and so does a press
/// inside the panel ([`on_list_press`]), which is exactly the "everything
/// except the control and the list" the capture clicked on.
fn on_window_press(
    press: On<Pointer<Press>>,
    mut commands: Commands,
    open: Query<Entity, With<PeriodList>>,
) {
    if press.event.button != PointerButton::Primary {
        return;
    }
    // The caption and `PeriodCombo::selected` are left untouched on purpose:
    // §34 read `1 day` off the field after the click.
    for panel in open.iter() {
        commands.entity(panel).despawn();
    }
}

/// A press inside the open list does **nothing** — §33 measured that the held
/// row wears the hover's colours and not a third paint. It only has to stop
/// bubbling, or [`on_period_press`] would read it as "close".
fn on_list_press(mut press: On<Pointer<Press>>) {
    press.propagate(false);
}

/// The highlight follows the pointer: exactly one row `Hovered`, all others
/// `Idle` (§32).
fn on_list_move(
    move_: On<Pointer<Move>>,
    lists: Query<(&ComputedNode, &UiGlobalTransform, &Children), With<PeriodList>>,
    mut rows: Query<(&ComboRow, &mut BackgroundColor, &mut TextColor)>,
) {
    let Ok((node, transform, children)) = lists.single() else {
        return;
    };
    let hovered = row_under_pointer(
        move_.pointer_location.position,
        node,
        transform,
        PERIOD_OPTIONS.len(),
        hud_scale(),
    );
    for child in children.iter() {
        let Ok((row, mut fill, mut colour)) = rows.get_mut(child) else {
            continue;
        };
        let state = if Some(row.0) == hovered {
            ComboRowState::Hovered
        } else {
            ComboRowState::Idle
        };
        let (background, text) = state.paint();
        fill.0 = background;
        colour.0 = text;
    }
}

/// Leaving the panel takes the highlight with it — there is no row under the
/// pointer any more, and no selected row to fall back on (§32).
fn on_list_out(
    out: On<Pointer<Out>>,
    lists: Query<(Entity, &Children), With<PeriodList>>,
    mut rows: Query<(&mut BackgroundColor, &mut TextColor), With<ComboRow>>,
) {
    let Ok((panel, children)) = lists.single() else {
        return;
    };
    // Only a pointer that left the *panel* clears the highlight. Rows are
    // spawned `Pickable::IGNORE` and should never be a target, but an `Out`
    // that bubbled up from one would blank the row the pointer just moved
    // onto — a one-frame flicker that is invisible in a test and obvious on
    // screen.
    if out.entity != panel {
        return;
    }
    let (background, text) = ComboRowState::Idle.paint();
    for child in children.iter() {
        if let Ok((mut fill, mut colour)) = rows.get_mut(child) {
            fill.0 = background;
            colour.0 = text;
        }
    }
}

/// Release over a row takes the value and closes the list (§32/§33). Display
/// only: nothing is sent.
fn on_list_release(
    mut release: On<Pointer<Release>>,
    mut commands: Commands,
    ui_strings: Res<ClientUiStrings>,
    lists: Query<(Entity, &ComputedNode, &UiGlobalTransform), With<PeriodList>>,
    mut controls: Query<&mut PeriodCombo>,
    mut caption: Query<&mut Text, With<PeriodCaption>>,
) {
    release.propagate(false);
    if release.event.button != PointerButton::Primary {
        return;
    }
    let Ok((panel, node, transform)) = lists.single() else {
        return;
    };
    let Some(index) = row_under_pointer(
        release.pointer_location.position,
        node,
        transform,
        PERIOD_OPTIONS.len(),
        hud_scale(),
    ) else {
        return;
    };
    if let Ok(mut combo) = controls.single_mut() {
        combo.selected = index;
    }
    if let Ok(mut text) = caption.single_mut() {
        let (key, english) = PERIOD_OPTIONS[index];
        **text = ui_strings.get_or(key, english).to_string();
    }
    commands.entity(panel).despawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::camera::NormalizedRenderTarget;
    use bevy::picking::backend::HitData;
    use bevy::picking::pointer::{Location, PointerId};
    use bevy::window::WindowRef;

    /// A primary-button press on one entity, propagated like a real one.
    fn press(app: &mut App, entity: Entity) {
        let window = app.world_mut().spawn(Window::default()).id();
        let location = Location {
            target: NormalizedRenderTarget::Window(
                WindowRef::Entity(window).normalize(None).unwrap(),
            ),
            position: Vec2::ZERO,
        };
        app.world_mut().trigger(Pointer::new(
            PointerId::Mouse,
            location,
            Press {
                button: PointerButton::Primary,
                hit: HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
                count: 1,
            },
            entity,
        ));
        app.world_mut().flush();
    }

    /// The two counts that identify `silksearch.2dt` as the rendered
    /// description rather than the classic `CIFSilkBuyList`: **one** tab and
    /// **11** rows, against the classic block's none and 14.
    #[test]
    fn the_photographed_pane_is_the_fourth_generation_one() {
        assert_eq!(ROW_COUNT, 11);
        assert_eq!(ROW_PITCH, 23.0);
        // last row's bottom stays inside the table bezel
        let last_bottom = ROW_AT.1 + ROW_PITCH * (ROW_COUNT - 1) as f32 + ROW_AT.3;
        assert!(
            last_bottom <= TABLE_WELL_AT.1 + TABLE_WELL_AT.3,
            "row {last_bottom} runs out of the bezel"
        );
    }

    /// The close button's exact template match is what fixes the width and the
    /// caption band; restate it as arithmetic so a later edit to either cannot
    /// silently walk away from the photograph.
    #[test]
    fn the_close_button_lands_where_the_photograph_has_it() {
        // `close_button_node`: right 10, top 4, 16x16.
        assert_eq!(WINDOW_SIZE.0 - 10.0 - 16.0, 436.0);
        assert_eq!(MALL_MS_WND.title_y + 4.0, 10.0);
        assert!(MALL_MS_WND.title_y + MALL_MS_WND.title_h <= MALL_MS_WND.top_h);
    }

    /// The anchor is the measured origin, and it is the centred one.
    #[test]
    fn the_pane_is_centred_over_the_mall_page() {
        assert_eq!(
            ANCHOR_RIGHT_TOP.0,
            (super::super::WINDOW_SIZE.0 - WINDOW_SIZE.0) / 2.0
        );
    }

    /// Every authored rect stays inside the window, and the combo obeys the
    /// shared widget's one invariant.
    #[test]
    fn every_rect_is_inside_the_window_and_the_combo_is_twenty_high() {
        let mut rects = vec![
            INNER_FRAME_AT,
            INNER_TILE_AT,
            TAB_AT,
            PANEL_AT,
            INTRO_AT,
            PERIOD_LABEL_AT,
            PERIOD_COMBO_AT,
            HISTORY_BUTTON_AT,
            RESULT_WELL_AT,
            RESULT_BED_AT,
            TABLE_WELL_AT,
            SCROLL_AT,
            TOTAL_LABEL_AT,
            TOTAL_VALUE_AT,
        ];
        rects.extend(COLUMNS.iter().map(|c| c.0));
        for r in rects {
            assert!(r.0 >= 0.0 && r.1 >= 0.0, "{r:?} starts outside");
            assert!(r.0 + r.2 <= WINDOW_SIZE.0, "{r:?} runs past the right edge");
            assert!(r.1 + r.3 <= WINDOW_SIZE.1, "{r:?} runs past the bottom");
        }
        assert_eq!(PERIOD_COMBO_AT.3, combo_box::FIELD_H);
    }

    /// The rule the shared widget states and this window is the first to obey:
    /// the rows are **four**, not the six-line text block they were cut from.
    ///
    /// Fails without the change — the list used to be `PERIOD_OPTIONS.len() ==
    /// 6`, i.e. `28 days` (L3500) and `Permanence` (L3501) were in it, and §31
    /// photographed a box that does not show them.
    #[test]
    fn the_list_is_the_four_photographed_rows_not_the_six_shipped_lines() {
        let shown: Vec<&str> = PERIOD_OPTIONS.iter().map(|o| o.1).collect();
        assert_eq!(shown, vec!["1 day", "7 days", "1 month", "3 months"]);
        for absent in ["28 days", "Permanence"] {
            assert!(
                !shown.contains(&absent),
                "{absent} is shipped (L3500/L3501) but not in the original's box"
            );
        }
        // Every row is a real `UIIT_CTL_SILK_INQUIRY_*` key, and the captions
        // are the shipped English, so a translation swaps both together.
        for (key, _) in PERIOD_OPTIONS {
            assert!(key.starts_with("UIIT_CTL_SILK_INQUIRY_"), "{key}");
        }
        // The closed field shows the first of them (§31).
        assert_eq!(PERIOD_OPTIONS[0].1, "1 day");
        assert_eq!(PeriodCombo::default().selected, 0);
    }

    /// The authored 102 px is field + arrow, exactly — which is why this site
    /// needs no "painted wider than authored" fudge.
    #[test]
    fn the_authored_rect_is_field_plus_arrow() {
        assert_eq!(PERIOD_FIELD_W, 82.0);
        assert_eq!(
            PERIOD_FIELD_W + combo_box::DROP_ARROW_SIZE + 2.0 * combo_box::DROP_ARROW_GAP,
            PERIOD_COMBO_AT.2
        );
        // The tile is drawn unscaled and its opaque 18x18 lands at (83,1),
        // the capture's arrow against a field origin of (342,185).
        assert_eq!(PERIOD_ARROW_AT.2, PERIOD_ARROW_AT.3);
        assert_eq!(PERIOD_ARROW_AT.0 + combo_box::DROP_ARROW_GAP, 83.0);
        assert_eq!(PERIOD_ARROW_AT.1 + combo_box::DROP_ARROW_GAP, 1.0);
        assert_eq!(PERIOD_ARROW_AT.0 + PERIOD_ARROW_AT.2, PERIOD_COMBO_AT.2);
        // Field and arrow both fit the 20 px control row.
        assert_eq!(PERIOD_ARROW_AT.3, PERIOD_COMBO_AT.3);
    }

    /// The hit logic, in the panel's own coordinates: the four rows are hit at
    /// their centres, the frame and the insets are not, and the mapping is the
    /// widget's `list_row_rect` rather than a second copy of it.
    ///
    /// Fails without the change: `row_at` did not exist, and `combo_list_row`
    /// had no caller at all.
    #[test]
    fn row_at_finds_exactly_the_row_under_the_point() {
        let rows = PERIOD_OPTIONS.len();
        for index in 0..rows {
            let (x, y, w, h) = combo_box::list_row_rect(PERIOD_FIELD_W, index);
            assert_eq!(
                row_at(Vec2::new(x + w / 2.0, y + h / 2.0), rows),
                Some(index)
            );
            // The row owns its top edge and not the next row's.
            assert_eq!(row_at(Vec2::new(x, y), rows), Some(index));
            assert_ne!(row_at(Vec2::new(x, y + h), rows), Some(index));
        }
        // The 5 px of frame plus opaque ring above the first row is not a row,
        // and neither is the 4 px side ring.
        let (x, y, w, _) = combo_box::list_row_rect(PERIOD_FIELD_W, 0);
        assert_eq!(row_at(Vec2::new(x + w / 2.0, y - 1.0), rows), None);
        assert_eq!(row_at(Vec2::new(x - 1.0, y + 1.0), rows), None);
        assert_eq!(row_at(Vec2::new(x + w, y + 1.0), rows), None);
        // Below the last row: the bottom inset, then nothing.
        let last = combo_box::list_row_rect(PERIOD_FIELD_W, rows - 1);
        assert_eq!(row_at(Vec2::new(x + w / 2.0, last.1 + last.3), rows), None);
        // And the panel is sized for exactly these four rows (§31: y 204..264).
        assert_eq!(combo_box::open_panel_height(rows), 61.0);
    }

    /// The two paints the window may apply, and the absence between them: there
    /// is no "selected" state to set, so hovering row 3 while row 1 is the
    /// value leaves row 1 exactly like rows 0 and 2 (§32).
    #[test]
    fn only_the_pointer_row_is_painted() {
        let hovered = 3usize;
        let selected = 1usize;
        let paint = |index: usize| {
            if index == hovered {
                ComboRowState::Hovered
            } else {
                ComboRowState::Idle
            }
            .paint()
        };
        assert_eq!(paint(selected), paint(0));
        assert_eq!(paint(selected), paint(2));
        assert_ne!(paint(hovered), paint(selected));
    }

    /// The two ways of closing the list **without** selecting, in one live app
    /// (§34, order `artifacts/capture/orders/done/264-combo-close/`).
    ///
    /// Fails without the change: `on_window_press` did not exist, so a press
    /// beside the control left the panel standing (case (b),
    /// `03-click-elsewhere.png`), and the control did not stop propagation, so
    /// once the root observer is there, opening the list would close it again
    /// in the same event.
    ///
    /// The value is asserted after every close because that is what the
    /// capture read off the field: `1 day` in both pictures. `ESC` is not
    /// exercised — it was never measured (§20: the key kills that build).
    #[test]
    fn a_press_beside_the_control_closes_the_list_and_keeps_the_value() {
        let mut app = App::new();
        // `AssetPlugin` needs the IO task pool, which only `TaskPoolPlugin`
        // creates — the same fixture note as `inventory/ui.rs`.
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ))
        .init_asset::<Image>()
        .init_resource::<ClientUiStrings>()
        .insert_resource(FontAssets {
            one: Handle::default(),
            two: Handle::default(),
            three: Handle::default(),
            nine: Handle::default(),
        });
        app.world_mut().spawn(Camera2d);
        app.world_mut()
            .run_system_cached(
                |mut commands: Commands,
                 asset_server: Res<AssetServer>,
                 fonts: Res<FontAssets>,
                 ui_strings: Res<ClientUiStrings>,
                 cameras: Query<Entity, With<Camera2d>>| {
                    let camera = cameras.single().expect("camera");
                    spawn_buy_list(&mut commands, &asset_server, &fonts, &ui_strings, camera);
                },
            )
            .expect("spawn_buy_list failed");
        app.world_mut().flush();

        let one = |app: &mut App| -> Entity {
            let mut query = app
                .world_mut()
                .query_filtered::<Entity, With<PeriodCombo>>();
            let found: Vec<Entity> = query.iter(app.world()).collect();
            assert_eq!(found.len(), 1, "one period combo");
            found[0]
        };
        let control = one(&mut app);
        let root = {
            let mut query = app
                .world_mut()
                .query_filtered::<Entity, With<BuyListRoot>>();
            let found: Vec<Entity> = query.iter(app.world()).collect();
            assert_eq!(found.len(), 1, "one buy list window");
            found[0]
        };
        let open = |app: &mut App| {
            let mut query = app.world_mut().query_filtered::<Entity, With<PeriodList>>();
            query.iter(app.world()).count()
        };
        let value = |app: &mut App| -> (usize, String) {
            let mut combos = app.world_mut().query::<&PeriodCombo>();
            let selected = combos.single(app.world()).expect("combo").selected;
            let mut captions = app
                .world_mut()
                .query_filtered::<&Text, With<PeriodCaption>>();
            let caption = captions.single(app.world()).expect("caption").0.clone();
            (selected, caption)
        };

        // The list is closed to start with and the field carries the
        // photographed `1 day`.
        assert_eq!(open(&mut app), 0);
        assert_eq!(value(&mut app), (0, "1 day".to_string()));

        // A press on the control opens it — and must survive its own bubbling
        // up to the window root.
        press(&mut app, control);
        assert_eq!(
            open(&mut app),
            1,
            "the press on the field did not open the list"
        );

        // (b) `03-click-elsewhere.png`: a press on the sub-window beside the
        // control closes the list and leaves the value alone.
        press(&mut app, root);
        assert_eq!(
            open(&mut app),
            0,
            "a press elsewhere in the window left the list open (§34 case (b))"
        );
        assert_eq!(
            value(&mut app),
            (0, "1 day".to_string()),
            "closing selected"
        );

        // (a) `02-second-click.png`: open again, second press on the field
        // closes it, value unchanged.
        press(&mut app, control);
        assert_eq!(open(&mut app), 1);
        press(&mut app, control);
        assert_eq!(open(&mut app), 0, "the second press did not close the list");
        assert_eq!(
            value(&mut app),
            (0, "1 day".to_string()),
            "closing selected"
        );
    }
}
