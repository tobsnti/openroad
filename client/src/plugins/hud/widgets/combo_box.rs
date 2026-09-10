//! `CIFComboBox` — the drop-down field.
//!
//! # The idea, and the honest limit of it
//!
//! A combo box is the one widget in the vanilla vocabulary whose **appearance
//! is not in the data at all**, and this module says so rather than inventing
//! around it:
//!
//! * There is no prototype file. `Media/resinfo/` ships ten widget prototypes
//!   (`ifcheckbox`, `ifsliderctrl`, `ifpagemanager`, …) and none of them is a
//!   combo box.
//! * All 26 classic instances carry `DDJ=""`, `Text=""` and `Style=0`.
//! * All 22 fourth-generation entries (`.2dt` type 12) carry `Image=""` **and**
//!   `Background=""`.
//! * Positive control on the same two read paths: the same struct-aware resinfo
//!   parse finds 161 non-empty `DDJ` values in those same 12 files (e.g.
//!   `ifautopotion.txt:6` `GDR_AUTO_POTION_CANCEL_BTN` =
//!   `interface\ifcommon\com_button.ddj`), and the same `.2dt` sweep finds 298
//!   entries with art in the five files that hold the type-12 rows. The empty
//!   art is the widget's shape, not a parser that missed a field.
//!
//! So the field's chrome is code-side in the original. What *is* authored, and
//! what this module therefore owns, is the geometry and the text treatment:
//! the 20 px field height, the per-site alignment, the two sites that tint
//! their field, and the 2 px client inset that comes with them.
//!
//! The closed field's fill is still ours (see [`FIELD_BG`]): it is drawn in the
//! house inset the store, storage and auto-potion modals already use — the same
//! two colours, so this module *replaces* three private copies instead of
//! adding a fourth style.
//!
//! # The open state is no longer unknown — it was photographed
//!
//! Measured on a capture of the original's item mall with the
//! `Set Inquiry Period` combo open
//! (806x629, client area at image offset `(+3,+26)`, 1:1 inside the window):
//! the item mall's `Set Inquiry Period` combo — `ifitemmallshop.txt:1209`
//! `GDR_ITEM_MALL_SILK_COMBOBOX`, `Rect="165,85,78,20"` — caught open. Every
//! number in the "open list" block below is a pixel coordinate out of that
//! image, and the drop arrow is a template match against a shipped `.ddj`
//! (see [`DROP_ARROW_DDJ`]). The art the *trees* never name does exist in the
//! archive; it is only invisible to a resinfo reader.
//!
//! # The list is assembled per window, in code — never generated from the text
//!
//! **This is a standing rule for every consumer of this widget.** The same §31
//! measured what the open list *shows*, and it shows less than the shipped text
//! offers: `textuisystem.txt` L3498-L3503 holds six consecutive lines
//! (`1 day`, `7 days`, **`28 days`**, **`Permanence`**, `1 month`, `3 months`),
//! and the original lists exactly **four** of them — `1 day`, `7 days`,
//! `1 month`, `3 months`. `28 days` and `Permanence` are shipped, plausible,
//! adjacent — and not in the box.
//!
//! So a combo's rows are **not** a contiguous text block. Whoever fills one by
//! slicing `textuisystem.txt` builds two entries the original does not have,
//! and would never notice, because both read like the others. Each window
//! spells out its own row list, next to the `file:line` of the combo it
//! belongs to.
//!
//! **Display only.** The widget renders the caption it is handed and, when
//! open, the rows it is handed. It never enumerates anything: what a given
//! combo lists is the window's business — see the rule above.

use bevy::prelude::*;

use crate::assets::FontAssets;
use crate::plugins::hud::game_window::abs_node;

// --- Geometry ----------------------------------------------------------------

/// **20 px, in 26 of 26 classic instances** — the single most solid fact about
/// this widget. Widths vary from 57 to 231 (14 distinct values), the height
/// never does. The fourth generation agrees 20 of 22; the two exceptions are
/// `optionwnd.2dt` idx 150/155 at 18, both the same redrawn control.
pub const FIELD_H: f32 = 20.0;

/// `ClientRect="0,2,0,0"` on the two tinted sites (`ifoption_video.txt:44`,
/// `ifvideooptionslot.txt:7`); the other 24 carry `0,0,0,0`. It is the caption's
/// own top inset inside the field, which is why it travels with the tint: those
/// two are the option rows that print a value in a coloured field.
pub const TINTED_TEXT_INSET_Y: f32 = 2.0;

/// Every authored `CIFComboBox` in the classic corpus: `(file:line, x, y, w, h)`.
///
/// Present in full because the height invariant above is only worth anything if
/// the reader can see the sample it was drawn from, and because this table *is*
/// the consumer list that justifies a shared widget: 12 windows.
pub const SITES: [(&str, f32, f32, f32, f32); 26] = [
    ("resinfo/ifautopotion.txt:164", 306.0, 375.0, 57.0, 20.0),
    ("resinfo/ifautopotion.txt:202", 188.0, 375.0, 57.0, 20.0),
    ("resinfo/ifautopotionslot.txt:82", 282.0, 6.0, 57.0, 20.0),
    ("resinfo/ifautopotionslot.txt:120", 165.0, 6.0, 57.0, 20.0),
    ("resinfo/ifcasrequesthelp.txt:103", 82.0, 114.0, 231.0, 20.0),
    ("resinfo/ifghachaselectwnd.txt:6", 90.0, 179.0, 166.0, 20.0),
    ("resinfo/ifghachaselectwnd.txt:25", 90.0, 128.0, 166.0, 20.0),
    ("resinfo/ifghachaselectwnd.txt:44", 90.0, 77.0, 166.0, 20.0),
    (
        "resinfo/ifguildwarrequest.txt:224",
        240.0,
        140.0,
        77.0,
        20.0,
    ),
    (
        "resinfo/ifguildwarrequest.txt:244",
        163.0,
        140.0,
        77.0,
        20.0,
    ),
    ("resinfo/ifguildwarrequest.txt:264", 86.0, 140.0, 77.0, 20.0),
    ("resinfo/ifguildwarrequest.txt:284", 86.0, 85.0, 97.0, 20.0),
    ("resinfo/ifitemmallshop.txt:1186", 165.0, 85.0, 78.0, 20.0),
    ("resinfo/ifitemmallshop.txt:1209", 165.0, 85.0, 78.0, 20.0),
    ("resinfo/ifitemmallshop.txt:1228", 165.0, 85.0, 78.0, 20.0),
    ("resinfo/ifmentormatch.txt:315", 400.0, 75.0, 106.0, 20.0),
    ("resinfo/ifmentormatch.txt:334", 253.0, 75.0, 74.0, 20.0),
    ("resinfo/ifoption_video.txt:44", 171.0, 48.0, 176.0, 20.0),
    ("resinfo/ifoption_video.txt:82", 171.0, 18.0, 176.0, 20.0),
    ("resinfo/ifoption_video.txt:257", 171.0, 18.0, 176.0, 20.0),
    ("resinfo/ifpartymatch.txt:372", 253.0, 75.0, 74.0, 20.0),
    ("resinfo/ifspecialtydeal.txt:25", 85.0, 211.0, 104.0, 20.0),
    ("resinfo/ifstallnetwork.txt:392", 419.0, 106.0, 90.0, 20.0),
    ("resinfo/ifstallnetwork.txt:411", 259.0, 106.0, 109.0, 20.0),
    ("resinfo/ifstallnetwork.txt:430", 79.0, 106.0, 121.0, 20.0),
    ("resinfo/ifvideooptionslot.txt:7", 151.0, 5.0, 156.0, 20.0),
];

// --- Text treatment ----------------------------------------------------------

/// `HAlign` on the instance: `0` in 19 sites, `2` in 5
/// (`ifguildwarrequest.txt` x4 — the numeric war-period fields — and
/// `ifspecialtydeal.txt:25`), `1` in 2 (the two tinted option rows, which also
/// carry the only `VAlign=1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComboAlign {
    /// `HAlign=0`.
    #[default]
    Left,
    /// `HAlign=1`.
    Center,
    /// `HAlign=2`.
    Right,
}

impl ComboAlign {
    /// Map the tree's raw `HAlign` integer. Any other value is `None` rather
    /// than a silent default: this is user data, and an unknown alignment is
    /// something a caller should be able to report.
    pub fn from_halign(halign: u32) -> Option<Self> {
        match halign {
            0 => Some(Self::Left),
            1 => Some(Self::Center),
            2 => Some(Self::Right),
            _ => None,
        }
    }

    fn justify(self) -> Justify {
        match self {
            Self::Left => Justify::Left,
            Self::Center => Justify::Center,
            Self::Right => Justify::Right,
        }
    }
}

/// `Color="255,42,154,67"` — `ifoption_video.txt:44` `GDR_OPT_VIDEO_SP_BR`.
/// The grammar's `COLOR` order is A,R,G,B (same order `Jmxv2dtEntry::color`
/// documents for the binary form), so this is an opaque green.
pub const TINT_OPT_VIDEO_BR: Color = Color::srgb_u8(42, 154, 67);
/// `Color="255,246,153,78"` — `ifvideooptionslot.txt:7` `GDR_OPT_VOS_CB`, an
/// opaque orange. The remaining 24 sites are `255,255,255,255`.
pub const TINT_OPT_VIDEO_SLOT: Color = Color::srgb_u8(246, 153, 78);

// --- House chrome (ours, stated) ---------------------------------------------

/// Field fill. Taken from the inset the store/storage/auto-potion modals
/// already draw (`hud/storage/ui.rs:837-841`, restated at
/// `hud/autopotion/ui.rs:638`) so that adopting this widget changes no pixel in
/// the windows that have a stand-in today. **Not** an original value: the
/// original's field art is UNKNOWN (see the module doc).
pub const FIELD_BG: Color = Color::srgb(0.09, 0.08, 0.06);
/// Field border, same source as [`FIELD_BG`].
pub const FIELD_BORDER: Color = Color::srgb(0.55, 0.45, 0.25);

/// A combo the window has switched off. Ours: no `_disable` art exists to swap
/// to, for the same reason no art exists at all.
pub const DISABLED_TEXT: Color = Color::srgba(1.0, 1.0, 1.0, 0.4);

/// Open state. Measured, not guessed: the list opens **downwards**, directly
/// under the field, at the field's own width (§31).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComboState {
    #[default]
    Closed,
    Open,
}

// --- The open list, measured (§31) -------------------------------------------
//
// One frame of the original, read pixel by pixel. Reference frame: the field is
// painted at image x 342..423, y 185..204; the list panel at x 342..423,
// y 204..264 — the two share the y=204 line, which is why the panel's local y
// below is `FIELD_H - 1` and not `FIELD_H`.

/// **13 px.** The four rows' glyph tops sit at image y 212 / 225 / 238 / 251 —
/// three intervals, all 13. Not [`FIELD_H`]: an open row is markedly tighter
/// than the field that carries the caption.
pub const LIST_ROW_H: f32 = 13.0;

/// Panel top edge (y 204) to the first row's top (y 209): the 1 px frame line
/// plus the 4 px of opaque black behind it.
pub const LIST_INSET_TOP: f32 = 5.0;

/// Last row's bottom (y 260, exclusive) to the panel's bottom edge (y 264):
/// 3 px of opaque black plus the 1 px frame line.
pub const LIST_INSET_BOTTOM: f32 = 4.0;

/// Left and right inset of a row inside the panel. Left: x 342 (the panel's
/// own dark edge) plus 343..345 opaque black, so the fill starts at x 346 =
/// panel + 4. Right: the fill ends at x 419, then 420..422 opaque black and the
/// frame line at 423 — panel + 82 - 4. Symmetric, 4 px.
pub const LIST_INSET_X: f32 = 4.0;

/// The measured glyph top inside a row (row 1 spans y 209..221, its glyphs
/// start at 212). Our font's own ascent is not the original's, so this is the
/// text node's inset rather than a promise about the baseline.
pub const LIST_TEXT_TOP: f32 = 3.0;

/// `rgb(123,121,123)` — sampled at image (346,204), (423,220) and (346,264),
/// identical on all three. It is a 1 px line on **top, right and bottom only**:
/// the panel's left edge (x 342) is dark (`rgb(8,12,8)` where the frame line
/// would be, against `rgb(37,35,32)` of the dialog one pixel further left), so
/// the frame reads as a bevel, not as a box. Reproduced as measured.
pub const LIST_BORDER: Color = Color::srgb_u8(123, 121, 123);

/// The panel is **translucent black at 50 %**, which is why it is worth a
/// constant rather than a flat fill: four different backgrounds shine through
/// it at exactly half strength — the table header bar `rgb(123,121,123)` reads
/// `rgb(61,60,61)` at y 238, the divider `rgb(73,69,63)` reads `rgb(36,34,31)`
/// at y 244, the dialog `rgb(37,35,32)` reads `rgb(18,17,16)` at y 209-218, and
/// `rgb(8,12,8)` reads `rgb(4,6,4)` at y 219. Four ratios, 0.486-0.500.
pub const LIST_FILL: Color = Color::srgba(0.0, 0.0, 0.0, 0.5);

/// The opaque black ring between [`LIST_BORDER`] and [`LIST_FILL`]. Same probe
/// row that proved the 50 % fill proves this is *not* 50 %: over the bright
/// header bar at y 238, x 343..345 and x 420..422 stay `rgb(0,0,0)`.
pub const LIST_INSET_FILL: Color = Color::BLACK;

/// Row text: pure `rgb(255,255,255)`, and **centred** — "1 day" spans
/// x 369..396 (centre 382.5) in a panel spanning x 342..423 (centre 382.5); the
/// other three rows centre within a pixel. Note this is the row's own
/// treatment, not the field's: the same combo is authored `HAlign=0`
/// (`ifitemmallshop.txt:1209`), and its *caption* is centred too, so the open
/// list does not inherit the site's alignment.
pub const LIST_TEXT: Color = Color::WHITE;

/// **Hover fill, `rgb(128,128,255)`** — from a capture of the original with a
/// hovered row (806x629, same frame as the header): the row under the pointer (`3 months`,
/// row 3) is filled over image x 346..419, y 248..260 — 844 of the row's
/// 962 px, the remaining 118 being the glyphs. That rectangle is exactly
/// [`list_row_rect`] for row 3 (74x13), so the highlight is the row, not a
/// band around the text.
///
/// The value is an exact half/full triple (128/128/255), i.e. *set* by the
/// client, not a blend of the 50 % fill with something behind it — a blend
/// would land off the halves the way every other pixel in this panel does.
///
/// One caveat, recorded rather than smoothed: the row-1 hover in the same
/// series (`05-hover-row2.png`) measures y 222..235, **14** px — one pixel
/// into the next row's top — while the last row measures 13. We draw 13
/// ([`LIST_ROW_H`]): the 14 px variant would run into the opaque inset ring
/// on the last row, and the capture cannot say which of the two the client
/// intends. [V] on both extents, [S] on the choice.
pub const LIST_ROW_HOVER: Color = Color::srgb_u8(128, 128, 255);

/// **Hover text, `rgb(255,255,128)`** — same frame, 118 px at image
/// x 359..408, y 251..259, i.e. the glyph band of the hovered row. Again an
/// exact half/full triple. It replaces [`LIST_TEXT`] wholesale: the hovered
/// row has no white glyph pixels left, only this colour plus 74 px of black
/// glyph edging.
///
/// Fill and text are **one state**, not two independent ones — see
/// [`ComboRowState::paint`].
pub const LIST_ROW_HOVER_TEXT: Color = Color::srgb_u8(255, 255, 128);

/// Which of the two paints a row wears. **Display only**: the widget draws
/// the state it is handed, the window decides which row the pointer is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComboRowState {
    /// Every row the pointer is not on, *including the selected one* — see
    /// the absence note on [`ComboRowState::paint`].
    #[default]
    Idle,
    /// The single row under the pointer.
    Hovered,
}

impl ComboRowState {
    /// `(fill, text)`, resolved together because the capture shows them
    /// switching together: in `06-hover-row4.png` the hovered row has
    /// [`LIST_ROW_HOVER`] behind it *and* not one white glyph pixel left.
    /// A caller that flips only one of them draws a state the original never
    /// shows.
    ///
    /// # There is no selection marker, and that is measured
    ///
    /// `06-hover-row4.png` was taken **after** a click had set the field to
    /// `7 days` (`04-after-select.png`), i.e. row 1 is the current value —
    /// and row 1 carries nothing: the whole image holds exactly 844
    /// `rgb(128,128,255)` pixels and every one of them is in row 3, under the
    /// pointer. Positive control on the same read path: that same probe *does*
    /// find the fill (844 px) in this image and finds it 13 rows higher in
    /// `05-hover-row2.png`, so a marker on the selected row would have been
    /// found by the identical scan. The highlight follows the pointer; it does
    /// not stick to the selection. This is an absence in the original, not a
    /// piece of this widget that is still missing.
    pub fn paint(self) -> (Color, Color) {
        match self {
            Self::Idle => (Color::NONE, LIST_TEXT),
            Self::Hovered => (LIST_ROW_HOVER, LIST_ROW_HOVER_TEXT),
        }
    }
}

/// The drop arrow, and the answer to "the data carry no art for this widget":
/// the art is in the archive, it is just never named by a tree. The 18x18
/// arrow the original draws template-matches
/// `interface/ifcommon/com_qst_downarrow_button.ddj` (20x20, A1R5G5B5, the
/// drawn button at texture offset (1,1)) with a mean channel error of **0.44 /
/// 255**; the runners-up are the same button pointing elsewhere
/// (`com_qst_rightarrow_button` 4.98, `com_qst_lefttarrow_button` 5.14), so the
/// match is decided by the triangle, not by the frame.
///
/// Positive control for "no tree names it": the same grep over all of
/// `Media/resinfo/*.txt` finds `com_blacksquare_` (e.g. `ifitemmallshop.txt`
/// `GDR_ITEM_MALL_SILK_SQUARE_1`) and returns nothing for `com_qst_downarrow`.
/// `_focus` and `_press` variants ship beside it, so the button has the usual
/// three states.
pub const DROP_ARROW_DDJ: &str = "interface/ifcommon/com_qst_downarrow_button.ddj";

/// The drop arrow is drawn 18x18 (image x 425..442, y 186..203) — the 20x20
/// texture's 1 px transparent margin is not part of it.
pub const DROP_ARROW_SIZE: f32 = 18.0;

/// The arrow sits one transparent pixel to the right of the painted field
/// (field ends x 423, arrow starts x 425) and one below its top (field y 185,
/// arrow y 186).
///
/// Whether it is "outside the control" depends on
/// which generation authored it: the drawn control is the
/// 4th-generation one, `res_ui/silksearch.2dt[10]` = `170,122,102,20`, and the
/// picture adds up inside those 102: field 82 + gap 1 + arrow 18 + gap 1. There
/// the arrow is *inside* the authored rect, which simply budgets for both parts.
/// Against the classic `ifitemmallshop.txt:1209` (`165,85,78,20`) the same
/// layout looks like a 4 px overdraw — an earlier note read it that way,
/// against a rect this window never renders.
pub const DROP_ARROW_GAP: f32 = 1.0;

/// Difference between the painted field (82 px) and the **classic** authored
/// width (78 px, `ifitemmallshop.txt:1209`).
///
/// Kept for the classic generation only, and no longer described as an
/// overdraw: in the 4th-generation rect the same 82 px are simply the field's
/// share of a 102 px control (see [`DROP_ARROW_GAP`]). A consumer that renders
/// the 4th-generation layout must **not** add this — `item_mall::buy_list`
/// derives its width from `102 - arrow - gaps` instead and tests that sum.
pub const PAINTED_EXTRA_W: f32 = 4.0;

/// Height of the panel an open combo needs for `rows` entries. Four rows give
/// `5 + 4*13 + 4 = 61`, which is exactly the measured y 204..264.
pub fn open_panel_height(rows: usize) -> f32 {
    if rows == 0 {
        return 0.0;
    }
    LIST_INSET_TOP + rows as f32 * LIST_ROW_H + LIST_INSET_BOTTOM
}

/// The open list panel's rect, widget-local, directly under the field. `y` is
/// `FIELD_H - 1`: the field's last row and the panel's frame line are the same
/// pixel in the capture (y 204).
pub fn open_panel_rect(field_w: f32, rows: usize) -> (f32, f32, f32, f32) {
    (0.0, FIELD_H - 1.0, field_w, open_panel_height(rows))
}

/// Row `index`'s rect inside a panel of `panel_w`, panel-local.
pub fn list_row_rect(panel_w: f32, index: usize) -> (f32, f32, f32, f32) {
    (
        LIST_INSET_X,
        LIST_INSET_TOP + index as f32 * LIST_ROW_H,
        panel_w - 2.0 * LIST_INSET_X,
        LIST_ROW_H,
    )
}

// --- Markers -----------------------------------------------------------------

/// The field's container node.
#[derive(Component, Debug)]
pub struct ComboBox;

/// The caption inside the field.
#[derive(Component, Debug)]
pub struct ComboCaption;

/// One row of an open list.
#[derive(Component, Debug)]
pub struct ComboRow(pub usize);

/// The open list's panel node.
#[derive(Component, Debug)]
pub struct ComboList;

/// The opaque black ring drawn inside [`ComboList`], between the frame line and
/// the translucent fill.
#[derive(Component, Debug)]
pub struct ComboListInset;

// --- Spawning ----------------------------------------------------------------

/// How a site wants its field drawn. Constructed from the tree's own values, so
/// a consumer writes `ComboStyle::from_site(0, None)` next to the `file:line` it
/// transcribed rather than restating colours.
#[derive(Debug, Clone, Copy)]
pub struct ComboStyle {
    pub align: ComboAlign,
    /// `None` for the 24 sites authored `255,255,255,255`.
    pub tint: Option<Color>,
    pub enabled: bool,
}

impl Default for ComboStyle {
    fn default() -> Self {
        Self {
            align: ComboAlign::Left,
            tint: None,
            enabled: true,
        }
    }
}

impl ComboStyle {
    /// The caption's colour and its top inset, resolved together — the two
    /// tinted sites are exactly the two that carry `ClientRect="0,2,0,0"`.
    pub fn caption(&self) -> (Color, f32) {
        let colour = match (self.enabled, self.tint) {
            (false, _) => DISABLED_TEXT,
            (true, Some(tint)) => tint,
            (true, None) => Color::WHITE,
        };
        let inset = if self.tint.is_some() {
            TINTED_TEXT_INSET_Y
        } else {
            0.0
        };
        (colour, inset)
    }
}

/// The closed field at `rect` in the parent's space. `rect.3` should be
/// [`FIELD_H`]; it is taken from the caller so a site can stay literally equal
/// to its `file:line`.
pub fn combo_field(rect: (f32, f32, f32, f32), s: f32) -> impl Bundle {
    (
        ComboBox,
        abs_node(rect, s),
        BackgroundColor(FIELD_BG),
        Outline {
            width: Val::Px(1.0),
            color: FIELD_BORDER,
            ..default()
        },
    )
}

/// The caption child of a [`combo_field`]. `field` is the field's own rect, so
/// the caption can be inset without the caller doing the arithmetic.
pub fn combo_caption(
    fonts: &FontAssets,
    field: (f32, f32, f32, f32),
    text: impl Into<String>,
    style: ComboStyle,
    s: f32,
) -> impl Bundle {
    let (colour, inset) = style.caption();
    (
        ComboCaption,
        Text::new(text.into()),
        TextFont {
            font: fonts.two.clone().into(),
            font_size: FontSize::Px(FIELD_H * 0.5 * s),
            ..default()
        },
        TextColor(colour),
        TextLayout::justify(style.align.justify()),
        abs_node((2.0, inset, field.2 - 4.0, field.3 - inset), s),
        Pickable::IGNORE,
    )
}

/// The open list's panel. Spawn with [`open_panel_rect`] in the field's parent
/// space; give it [`combo_list_inset`] and one [`combo_list_row`] per entry as
/// children.
///
/// The frame is asymmetric on purpose — top, right and bottom carry
/// [`LIST_BORDER`], the left edge carries nothing, because that is what the
/// capture shows (see [`LIST_BORDER`]).
pub fn combo_list_panel(rect: (f32, f32, f32, f32), s: f32) -> impl Bundle {
    let mut node = abs_node(rect, s);
    node.border = UiRect {
        left: Val::Px(0.0),
        top: Val::Px(s),
        right: Val::Px(s),
        bottom: Val::Px(s),
    };
    (
        ComboList,
        node,
        BackgroundColor(LIST_FILL),
        BorderColor {
            top: LIST_BORDER,
            right: LIST_BORDER,
            bottom: LIST_BORDER,
            left: Color::NONE,
        },
    )
}

/// The opaque black ring, a child of [`combo_list_panel`]. It is a *border*
/// with no fill, so the translucent middle survives: only the ring is painted.
///
/// It spans the panel minus the frame line on top and bottom, and its own edges
/// are the 4/4/3/3 px measured at x 342..345, x 420..422, y 205..208 and
/// y 261..263.
pub fn combo_list_inset(panel: (f32, f32, f32, f32), s: f32) -> impl Bundle {
    let (_, _, w, h) = panel;
    let mut node = abs_node((0.0, 1.0, w - 1.0, h - 2.0), s);
    node.border = UiRect {
        left: Val::Px(LIST_INSET_X * s),
        top: Val::Px((LIST_INSET_TOP - 1.0) * s),
        right: Val::Px((LIST_INSET_X - 1.0) * s),
        bottom: Val::Px((LIST_INSET_BOTTOM - 1.0) * s),
    };
    (
        ComboListInset,
        node,
        BorderColor::all(LIST_INSET_FILL),
        Pickable::IGNORE,
    )
}

/// One row of the open list, a child of [`combo_list_panel`].
///
/// **No selection highlight**, and that is a measurement, not an omission: the
/// panel's fill is uniform 50 % black over all four rows, including the row
/// whose value the field is showing. §31 saw it with `1 day` selected and
/// row 0 unmarked; §32 saw it again after a click had moved the value to
/// `7 days`, with row 1 unmarked while the pointer's row 3 carried the full
/// [`LIST_ROW_HOVER`] fill. Positive control on the same read path — the two
/// bands that *look* lighter in the §31 screenshot, y 209-218 and y 238-244,
/// are exactly the y ranges where bright content sits **outside** the panel at
/// the same height (x 338..341 reads `rgb(37,35,32)` and `rgb(123,121,123)`
/// there), i.e. they are the translucency doing its job.
///
/// The row node spans the whole [`list_row_rect`] and pushes its glyphs down
/// with padding instead of an offset, because the hover fill was measured over
/// the full 13 px row, not over the glyph band.
///
/// The pressed state was not in either frame and stays unknown.
pub fn combo_list_row(
    fonts: &FontAssets,
    panel_w: f32,
    index: usize,
    text: impl Into<String>,
    state: ComboRowState,
    s: f32,
) -> impl Bundle {
    let (x, y, w, h) = list_row_rect(panel_w, index);
    let (fill, colour) = state.paint();
    let mut node = abs_node((x, y, w, h), s);
    node.padding = UiRect::top(Val::Px(LIST_TEXT_TOP * s));
    (
        ComboRow(index),
        Text::new(text.into()),
        TextFont {
            font: fonts.two.clone().into(),
            font_size: FontSize::Px(FIELD_H * 0.5 * s),
            ..default()
        },
        TextColor(colour),
        TextLayout::justify(Justify::Center),
        node,
        BackgroundColor(fill),
        Pickable::IGNORE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one invariant the data actually states: 26 of 26 sites are 20 px
    /// high while their widths spread over 14 distinct values.
    #[test]
    fn every_authored_site_is_twenty_high() {
        assert_eq!(SITES.len(), 26);
        for site in SITES {
            assert_eq!(site.4, FIELD_H, "{} is not {FIELD_H} high", site.0);
        }
        let mut widths: Vec<u32> = SITES.iter().map(|s| s.3 as u32).collect();
        widths.sort_unstable();
        widths.dedup();
        assert_eq!(widths.len(), 14);
        assert_eq!(widths.first(), Some(&57));
        assert_eq!(widths.last(), Some(&231));
    }

    /// Twelve windows, which is the argument for the widget being shared.
    #[test]
    fn the_sites_span_twelve_windows() {
        let mut files: Vec<&str> = SITES
            .iter()
            .map(|s| s.0.split(':').next().unwrap())
            .collect();
        files.sort_unstable();
        files.dedup();
        assert_eq!(files.len(), 12);
        // The four consumers the backlog names.
        for expected in [
            "resinfo/ifpartymatch.txt",
            "resinfo/ifitemmallshop.txt",
            "resinfo/ifstallnetwork.txt",
            "resinfo/ifoption_video.txt",
        ] {
            assert!(files.contains(&expected), "{expected} missing");
        }
    }

    #[test]
    fn halign_maps_to_the_three_authored_values() {
        assert_eq!(ComboAlign::from_halign(0), Some(ComboAlign::Left));
        assert_eq!(ComboAlign::from_halign(1), Some(ComboAlign::Center));
        assert_eq!(ComboAlign::from_halign(2), Some(ComboAlign::Right));
        assert_eq!(ComboAlign::from_halign(3), None);
    }

    /// The tint and the 2 px caption inset are one fact, not two: only the two
    /// sites with a non-white `Color` carry `ClientRect="0,2,0,0"`.
    #[test]
    fn tint_and_inset_travel_together() {
        let plain = ComboStyle::default();
        assert_eq!(plain.caption(), (Color::WHITE, 0.0));

        let tinted = ComboStyle {
            tint: Some(TINT_OPT_VIDEO_BR),
            ..Default::default()
        };
        assert_eq!(tinted.caption(), (TINT_OPT_VIDEO_BR, TINTED_TEXT_INSET_Y));

        // Disabled wins over the tint, and keeps the inset it was authored with.
        let off = ComboStyle {
            tint: Some(TINT_OPT_VIDEO_SLOT),
            enabled: false,
            ..Default::default()
        };
        assert_eq!(off.caption(), (DISABLED_TEXT, TINTED_TEXT_INSET_Y));
        assert!(DISABLED_TEXT.alpha() < 1.0);
    }

    /// An open list is a stack of rows directly under the field — and an empty
    /// list must not produce a negative or floating panel.
    #[test]
    fn open_panel_stacks_under_the_field() {
        assert_eq!(open_panel_height(0), 0.0);
        assert_eq!(
            open_panel_height(5),
            LIST_INSET_TOP + 5.0 * LIST_ROW_H + LIST_INSET_BOTTOM
        );
        let (x, y, w, h) = open_panel_rect(176.0, 3);
        assert_eq!((x, y, w), (0.0, FIELD_H - 1.0, 176.0));
        assert_eq!(h, LIST_INSET_TOP + 3.0 * LIST_ROW_H + LIST_INSET_BOTTOM);
        assert_eq!(ComboState::default(), ComboState::Closed);

        // A row is not as tall as the field that carries the caption. This is
        // the one thing a "rows of FIELD_H" guess gets wrong, so state it.
        assert!(LIST_ROW_H < FIELD_H);
    }

    /// The capture, replayed in absolute image coordinates: the item mall's
    /// `Set Inquiry Period` combo open with four rows
    /// (the capture named in the module header). Field top-left (342,185),
    /// painted 82x20; panel y 204..264; row glyph tops y 212/225/238/251.
    ///
    /// Every assertion here is a pixel someone can go and re-read.
    #[test]
    fn the_open_list_reproduces_the_measured_capture() {
        const FIELD_X: f32 = 342.0;
        const FIELD_Y: f32 = 185.0;
        const PAINTED_W: f32 = 82.0;

        // The authored rect is `165,85,78,20` (ifitemmallshop.txt:1209): the
        // height is honoured exactly, the width is painted 4 px wider.
        assert_eq!(PAINTED_W, 78.0 + PAINTED_EXTRA_W);
        assert_eq!(
            SITES[13],
            ("resinfo/ifitemmallshop.txt:1209", 165.0, 85.0, 78.0, 20.0)
        );

        // Panel: shares the field's last row (y 204) and its width, ends 264.
        let panel = open_panel_rect(PAINTED_W, 4);
        assert_eq!(FIELD_Y + panel.1, 204.0);
        assert_eq!(panel.2, PAINTED_W, "the list holds the field's width");
        assert_eq!(FIELD_Y + panel.1 + panel.3 - 1.0, 264.0);

        // Rows: the translucent fill starts at x 346 and y 209 and is exactly
        // four rows tall (52 px, y 209..260).
        let first = list_row_rect(panel.2, 0);
        assert_eq!(FIELD_X + first.0, 346.0);
        assert_eq!(FIELD_Y + panel.1 + first.1, 209.0);
        assert_eq!(first.2, 74.0, "fill spans x 346..419");
        let last = list_row_rect(panel.2, 3);
        assert_eq!(FIELD_Y + panel.1 + last.1 + last.3, 261.0);

        // The four measured glyph tops, in order.
        for (index, glyph_top) in [212.0, 225.0, 238.0, 251.0].into_iter().enumerate() {
            let row = list_row_rect(panel.2, index);
            assert_eq!(
                FIELD_Y + panel.1 + row.1 + LIST_TEXT_TOP,
                glyph_top,
                "row {index}"
            );
        }
    }

    /// The frame the capture shows is a bevel, not a box: three light edges and
    /// a dark left one. Worth a test because the obvious implementation —
    /// `Outline`/`BorderColor::all` — silently draws the fourth.
    #[test]
    fn the_panel_frame_is_light_on_three_edges_only() {
        assert_eq!(LIST_BORDER, Color::srgb_u8(123, 121, 123));
        assert_eq!(LIST_FILL.alpha(), 0.5, "the panel is translucent");
        assert_eq!(LIST_INSET_FILL.alpha(), 1.0, "its inset ring is not");
        assert_ne!(LIST_FILL, LIST_INSET_FILL);
    }

    /// §31's rule, as an executable reminder: the shipped text block holds six
    /// consecutive period lines, the original's box lists four. A consumer that
    /// slices the block gets two rows that do not exist.
    ///
    /// The rule is no longer capture-only: the binary confirms it. The four
    /// shown keys are pushed onto one control in this order in the original,
    /// and the two dropped keys (`UIIT_CTL_SILK_INQUIRY_MONTH_DAY`,
    /// `_PERMANENCE`) do not occur in the image at all. A test cannot read the exe,
    /// so this stays a note beside the assertions rather than an assertion.
    #[test]
    fn the_shipped_text_block_is_not_the_row_list() {
        const SHIPPED: [&str; 6] = [
            "1 day",
            "7 days",
            "28 days",
            "Permanence",
            "1 month",
            "3 months",
        ];
        const SHOWN: [&str; 4] = ["1 day", "7 days", "1 month", "3 months"];

        assert_eq!(SHIPPED.len(), 6, "textuisystem.txt L3498-L3503");
        assert_eq!(SHOWN.len(), 4, "capture 04-combo-open.png");
        for missing in ["28 days", "Permanence"] {
            assert!(SHIPPED.contains(&missing));
            assert!(!SHOWN.contains(&missing), "{missing} is not in the box");
        }
        // And the panel is sized for what is shown, not for what ships.
        assert_ne!(
            open_panel_height(SHOWN.len()),
            open_panel_height(SHIPPED.len())
        );
    }

    /// §32, replayed in the same absolute image coordinates: the hovered row
    /// is filled over exactly its own [`list_row_rect`]
    /// (`271-combo-hover/06-hover-row4.png`, x 346..419, y 248..260 — 74x13),
    /// and the two hover colours are the exact half/full triples that were
    /// measured, not blends.
    ///
    /// Fails without the hover state: `LIST_ROW_HOVER` is what it asserts.
    #[test]
    fn the_hovered_row_matches_the_measured_rectangle() {
        const FIELD_X: f32 = 342.0;
        const FIELD_Y: f32 = 185.0;
        const PAINTED_W: f32 = 82.0;

        assert_eq!(LIST_ROW_HOVER, Color::srgb_u8(128, 128, 255));
        assert_eq!(LIST_ROW_HOVER_TEXT, Color::srgb_u8(255, 255, 128));
        assert_eq!(
            LIST_ROW_HOVER.alpha(),
            1.0,
            "the fill is opaque, not the 50 %"
        );

        let panel = open_panel_rect(PAINTED_W, 4);
        let row = list_row_rect(panel.2, 3);
        assert_eq!(FIELD_X + row.0, 346.0);
        assert_eq!(FIELD_X + row.0 + row.2 - 1.0, 419.0);
        assert_eq!(FIELD_Y + panel.1 + row.1, 248.0);
        assert_eq!(FIELD_Y + panel.1 + row.1 + row.3 - 1.0, 260.0);
        // 74x13 = 962 px, of which 844 are fill and 118 are the recoloured
        // glyphs; the remaining 74 are the black glyph edging.
        assert_eq!(row.2 * row.3, 962.0);
        assert_eq!(844.0 + 118.0, 962.0);
    }

    /// The one thing a later refactor can silently break: fill and text are a
    /// pair. The hovered row in the capture has *no* white glyph pixel left,
    /// so a state that keeps [`LIST_TEXT`] on a filled row — or tints the text
    /// without filling — is a state the original never shows.
    ///
    /// Also the absence: [`ComboRowState::default`] is `Idle`, i.e. the
    /// selected row is painted like every other one (§32, row 1 unmarked while
    /// the field read `7 days`).
    #[test]
    fn hover_switches_fill_and_text_together() {
        for state in [ComboRowState::Idle, ComboRowState::Hovered] {
            let (fill, text) = state.paint();
            assert_eq!(
                fill != Color::NONE,
                text != LIST_TEXT,
                "{state:?} changes only one of fill/text"
            );
        }
        assert_eq!(ComboRowState::Idle.paint(), (Color::NONE, LIST_TEXT));
        assert_eq!(
            ComboRowState::Hovered.paint(),
            (LIST_ROW_HOVER, LIST_ROW_HOVER_TEXT)
        );
        assert_eq!(ComboRowState::default(), ComboRowState::Idle);
        // No selection marker: nothing but the pointer's row can be painted,
        // because `Hovered` is the only non-idle state there is.
        assert_ne!(LIST_ROW_HOVER, LIST_FILL);
    }

    /// The drop arrow is a real shipped tile, and it lives beside the field
    /// rather than inside it.
    #[test]
    fn the_drop_arrow_sits_outside_the_field() {
        assert_eq!(
            DROP_ARROW_DDJ,
            "interface/ifcommon/com_qst_downarrow_button.ddj"
        );
        // Field x 342..423 painted 82 wide, arrow x 425..442.
        let field_right = 342.0 + 82.0;
        assert_eq!(field_right + DROP_ARROW_GAP, 425.0);
        assert_eq!(425.0 + DROP_ARROW_SIZE - 1.0, 442.0);
        assert!(DROP_ARROW_SIZE < FIELD_H, "18 in a 20 px field row");
    }
}
