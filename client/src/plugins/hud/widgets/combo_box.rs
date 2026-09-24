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
//! # The open state of the combo box
//!
//! The open list is described down to the pixel — panel insets, row
//! pitch, the bevelled frame, the 50 % fill and the hover pair. None of it is
//! code in this module: no window in this tree opens a combo yet, and a widget
//! nobody spawns is a transcription nobody can catch being wrong. The
//! numbers live in `docs/ui/combo-box-open-list.md`, together with the
//! standing rule for whoever implements it — **a combo's rows are assembled by
//! the window, never sliced out of `textuisystem.txt`** (the original's box
//! shows four of six consecutive shipped lines).
//!
//! The drop arrow is a template match against a shipped `.ddj`
//! (see [`DROP_ARROW_DDJ`]): the art the *trees* never name does exist in the
//! archive, it is only invisible to a resinfo reader.
//!
//! **Display only.** The widget renders the caption it is handed. It never
//! enumerates anything: what a given combo lists is the window's business.

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
pub const DROP_ARROW_DDJ: &str = "media://interface/ifcommon/com_qst_downarrow_button.ddj";

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
/// the 4th-generation layout must **not** add this — it derives its width from
/// `102 - arrow - gaps` instead and tests that sum.
pub const PAINTED_EXTRA_W: f32 = 4.0;

// --- Markers -----------------------------------------------------------------

/// The field's container node.
#[derive(Component, Debug)]
pub struct ComboBox;

/// The caption inside the field.
#[derive(Component, Debug)]
pub struct ComboCaption;

// --- Spawning ----------------------------------------------------------------

/// How a site wants its field drawn. Constructed from the tree's own values, so
/// a consumer states the align/tint pair next to the `file:line` it transcribed
/// rather than restating colours.
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

    /// The drop arrow is a real shipped tile, and it lives beside the field
    /// rather than inside it.
    #[test]
    fn the_drop_arrow_sits_outside_the_field() {
        // The path has to be loadable, not merely spelled: every asset in this
        // tree is read through the `media://` source (the PK2 reader), so the
        // prefix is checked against a constant that is actually spawned today
        // (`game_window`'s tile) rather than against a copy of this literal.
        let (source, path) = DROP_ARROW_DDJ
            .split_once("://")
            .expect("an asset path carries its source");
        assert_eq!(
            source,
            crate::plugins::hud::game_window::BG_TILE_DDJ
                .split_once("://")
                .expect("an asset path carries its source")
                .0
        );
        assert_eq!(path, "interface/ifcommon/com_qst_downarrow_button.ddj");
        // The painted field is 4 px wider than the classic authored rect
        // (`ifitemmallshop.txt:1209` = `165,85,78,20`), which is what
        // `PAINTED_EXTRA_W` records and what puts the arrow at x 425.
        assert_eq!(
            SITES[13],
            ("resinfo/ifitemmallshop.txt:1209", 165.0, 85.0, 78.0, 20.0)
        );
        assert_eq!(78.0 + PAINTED_EXTRA_W, 82.0);
        // Field x 342..423 painted 82 wide, arrow x 425..442.
        let field_right = 342.0 + 82.0;
        assert_eq!(field_right + DROP_ARROW_GAP, 425.0);
        assert_eq!(425.0 + DROP_ARROW_SIZE - 1.0, 442.0);
        assert!(DROP_ARROW_SIZE < FIELD_H, "18 in a 20 px field row");
    }
}
