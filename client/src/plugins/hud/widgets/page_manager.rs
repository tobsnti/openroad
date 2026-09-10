//! `CIFPageManager` — the Prev/Next strip under a paged list.
//!
//! # The idea
//!
//! A `CIFPageManager` instance in a window tree carries **no art at all**
//! (`DDJ=""` in 2 of 2 classic instances, `Image=""` *and* `Background=""` in
//! 4 of 4 fourth-generation entries). Its parts come from the global prototype
//! `Media/resinfo/ifpagemanager.txt`, which is the file this module
//! transcribes: two `CIFButton`s and two art-less `CIFStatic` decorations.
//!
//! The two buttons are **not** arrow glyphs — the art has the words baked in.
//! `interface/mall/mall_page_prev.ddj` is a 36x12 A1R5G5B5 image reading
//! `<| Prev`, `mall_page_next.ddj` reads `Next |>` (decoded from the DDS
//! payload at file offset 148, `w@36`/`h@32`, 36*12*2 + 148 == 1012 == the file
//! size, so the 16 bpp reading is closed by the file length itself).
//!
//! So the strip is: prev button at local `(0,0)`, next button at local
//! `(64,0)`, both 36x12 — leaving a 28 px gap between them at local x 36..64
//! that no control in the prototype occupies. That gap is where the page
//! readout goes; see [`LABEL_RECT`].
//!
//! **Display only.** Nothing here advances a page, loads a row or knows what is
//! being paged. [`PageState`] is a value the window hands in.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::Button;

use crate::assets::FontAssets;
use crate::plugins::hud::game_window::abs_node;
use crate::plugins::ui_v2::style::ImageButtonStyle;

// --- Art ---------------------------------------------------------------------
//
// `ifpagemanager.txt:44` (`GDR_PAGE_MGR_RIGHT_BTN:CIFButton`, ID 2) and `:63`
// (`GDR_PAGE_MGR_LEFT_BTN:CIFButton`, ID 1) name the two base images.
// (Line numbers re-counted 2026-08-23 against
// `<your Media.pk2>/resinfo/ifpagemanager.txt`; they cite the
// control's declaration line, the same convention [`SITES`] uses.) The
// `_focus` / `_press` siblings exist in `Media/interface/mall/` and are the
// standard state-suffix grammar (`shared-input-widgets.md` §5 grammar 1).

/// `ifpagemanager.txt:67` `DDJ="interface\mall\mall_page_prev.ddj"`.
pub const PREV_DDJ: &str = "media://interface/mall/mall_page_prev.ddj";
/// Directory listing of `Media/interface/mall/`.
pub const PREV_FOCUS_DDJ: &str = "media://interface/mall/mall_page_prev_focus.ddj";
/// Directory listing of `Media/interface/mall/`.
pub const PREV_PRESS_DDJ: &str = "media://interface/mall/mall_page_prev_press.ddj";
/// `ifpagemanager.txt:48` `DDJ="interface\mall\mall_page_next.ddj"`.
pub const NEXT_DDJ: &str = "media://interface/mall/mall_page_next.ddj";
/// Directory listing of `Media/interface/mall/`.
pub const NEXT_FOCUS_DDJ: &str = "media://interface/mall/mall_page_next_focus.ddj";
/// Directory listing of `Media/interface/mall/`.
pub const NEXT_PRESS_DDJ: &str = "media://interface/mall/mall_page_next_press.ddj";

/// There is **no** `_disable` frame: `Media/interface/mall/` holds exactly six
/// `mall_page_*` files — `{prev,next}` x `{"",_focus,_press}`. Positive control
/// on the same listing: the neighbouring `mall_mainpage` triple is present with
/// the same three suffixes and likewise no `_disable`, and
/// `interface/ifcommon/com_button_disable.ddj` *does* exist, so a missing
/// `_disable` here is the asset set's shape and not a lookup that failed.
///
/// Ours, therefore, and stated: an unavailable direction is drawn with its
/// normal art tinted to [`DISABLED_TINT`] rather than swapped or hidden.
/// Hiding it would make the strip's width jump between pages; leaving it
/// unchanged would claim a click that does nothing. Which of the three the
/// original does is capture-gated (order `270-shared-widgets-…`, question (d)).
pub const DISABLED_TINT: Color = Color::srgba(1.0, 1.0, 1.0, 0.4);

// --- Geometry ----------------------------------------------------------------

/// Both button images are 36x12 (DDS header `w@36`, `h@32`; all six
/// `mall_page_*` files agree). The prototype's `Rect`s carry `w=h=0`, i.e. the
/// button takes the art's own extent — the same convention `ifspincontrol.txt`
/// breaks by spelling `16,16` out.
pub const PART_W: f32 = 36.0;
/// See [`PART_W`].
pub const PART_H: f32 = 12.0;

/// `ifpagemanager.txt:72` `Rect="0,0,0,0"` on `GDR_PAGE_MGR_LEFT_BTN`.
pub const PREV_X: f32 = 0.0;
/// `ifpagemanager.txt:53` `Rect="64,0,0,0"` on `GDR_PAGE_MGR_RIGHT_BTN`.
pub const NEXT_X: f32 = 64.0;

/// How wide the widget's own parts are: `NEXT_X + PART_W`. The *instance* rects
/// are far wider (385 in both classic sites, see [`SITES`]), so the parts do not
/// fill their instance and the strip is laid out from the instance's left edge
/// as the prototype spells it.
pub const INTRINSIC_W: f32 = NEXT_X + PART_W;

/// The 28 px hole between the two buttons, `(x, y, w, h)` widget-local.
///
/// The prototype puts `GDR_PAGE_MGR_LEFT_DECO` (ID 10) at `0,0,0,0` and
/// `GDR_PAGE_MGR_RIGHT_DECO` (ID 11) at `64,0,0,0` — the buttons' own
/// positions — with `DDJ=""` and `Text=""` on both, so the decorations
/// contribute nothing drawable and no control claims this gap. That the page
/// readout lives here is **ours** (`[S]`): it is the only free space inside the
/// widget's own extent, and both consumers are lists that must show which page
/// they are on. Whether the original prints `3/12` here or a clickable run of
/// page numbers is capture-gated (order `270-shared-widgets-…`, questions
/// (a)/(b)).
pub const LABEL_RECT: (f32, f32, f32, f32) = (PART_W, 0.0, NEXT_X - PART_W, PART_H);

/// The two authored instances in the classic trees, and the fourth-generation
/// rect they all agree on. `(file:line, x, y, w, h)`.
///
/// Kept as data rather than prose because it is the whole argument for this
/// module existing: two windows, one widget. The fourth generation's four
/// entries are all `91x17` (`open_market_main.2dt` idx 28/137/268,
/// `stallnetwork.2dt` idx 8) — a redraw of the same ~100 px strip, which is the
/// cross-check that [`INTRINSIC_W`] is the widget's real width and 385 is just
/// the band it sits in.
pub const SITES: [(&str, f32, f32, f32, f32); 2] = [
    ("resinfo/ifitemmallshop.txt:1251", 15.0, 522.0, 385.0, 26.0),
    ("resinfo/ifstallnetwork.txt:6", 80.0, 582.0, 385.0, 26.0),
];

// --- State -------------------------------------------------------------------

/// Which page of how many — a **value the window hands in**, one-based like the
/// numbers a player reads.
///
/// `total == 0` is a legitimate input (an empty result set) and is not an
/// error: both directions are then unavailable and the readout says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageState {
    current: u32,
    total: u32,
}

impl PageState {
    /// `current` is clamped into `1..=total` (and to `0` when `total == 0`), so
    /// a caller that decrements past the start cannot produce a strip that
    /// shows page 0 of 12.
    pub fn new(current: u32, total: u32) -> Self {
        let current = if total == 0 {
            0
        } else {
            current.clamp(1, total)
        };
        Self { current, total }
    }

    pub fn current(&self) -> u32 {
        self.current
    }

    pub fn total(&self) -> u32 {
        self.total
    }

    /// Is the strip drawn at all?
    ///
    /// **No, while there is at most one page.** Measured on the original
    /// (capture of the item mall): the item mall showing a
    /// single-page section leaves the authored band `15,522,385,26` — image
    /// rect `(18,548)-(403,574)` — at mean brightness **11**, max **61**,
    /// against 52/255 for an equally sized patch of the article list above it.
    /// The button art is white/orange lettering, so a drawn strip would have
    /// peaked near 255 there.
    ///
    /// That also closes the `_disable` question: the original does not need a
    /// disabled frame for the whole strip because it omits the strip. It lives
    /// here rather than in each window because it is a property of the widget,
    /// and a window that forgot it would be the only one showing a `1/1`.
    pub fn visible(&self) -> bool {
        self.total > 1
    }

    /// Is there a page before this one? Drives [`DISABLED_TINT`] on the Prev
    /// button — it does **not** page anything.
    pub fn has_prev(&self) -> bool {
        self.current > 1
    }

    /// Is there a page after this one?
    pub fn has_next(&self) -> bool {
        self.total > 0 && self.current < self.total
    }

    /// The readout for [`LABEL_RECT`], e.g. `3/12`. Ours: the original's format
    /// is capture-gated (see [`LABEL_RECT`]); `n/m` is picked because it fits
    /// the 28 px gap at the HUD's text size for realistic page counts and needs
    /// no string table entry — no `UIIT_*` key in `textuisystem` names a page
    /// counter.
    pub fn label(&self) -> String {
        if self.total == 0 {
            "-/-".to_string()
        } else {
            format!("{}/{}", self.current, self.total)
        }
    }

    /// Tint for the Prev/Next art given whether that direction exists.
    pub fn tint(enabled: bool) -> Color {
        if enabled {
            Color::WHITE
        } else {
            DISABLED_TINT
        }
    }
}

// --- Markers -----------------------------------------------------------------

/// The strip's container. The window puts its own marker alongside if it runs
/// more than one.
#[derive(Component, Debug)]
pub struct PageManager;

/// The Prev button. The window attaches its own `Activate` observer — this
/// module deliberately ships none, because "what a page is" is the window's.
#[derive(Component, Debug)]
pub struct PagePrevButton;

/// The Next button.
#[derive(Component, Debug)]
pub struct PageNextButton;

/// The readout in [`LABEL_RECT`].
#[derive(Component, Debug)]
pub struct PageLabel;

// --- Spawning ----------------------------------------------------------------

/// The strip's container node, positioned at `rect` in the parent's space.
///
/// `rect` is the *instance* rect straight out of the window tree (see
/// [`SITES`]); the parts inside are laid out at the prototype's own local
/// coordinates, so a consumer never restates 36, 12 or 64.
pub fn page_manager_root(rect: (f32, f32, f32, f32), s: f32) -> impl Bundle {
    (PageManager, abs_node(rect, s), Pickable::IGNORE)
}

/// Spawn the two buttons and the readout into an already-spawned
/// [`page_manager_root`].
///
/// Spawns **nothing** while [`PageState::visible`] is false — the single-page
/// case the original leaves blank (§28, see that method). The root stays, so a
/// window can hand in a new state without rebuilding its own tree.
///
/// Returns the `(prev, next)` button entities so the window can hang its own
/// `Activate` observers on them — `None` in the hidden case, which is also the
/// caller's signal that there is nothing to observe.
pub fn spawn_parts(
    parent: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    state: PageState,
    s: f32,
) -> Option<(Entity, Entity)> {
    if !state.visible() {
        return None;
    }
    let prev = parent
        .spawn(arrow(
            PagePrevButton,
            asset_server,
            (PREV_DDJ, PREV_FOCUS_DDJ, PREV_PRESS_DDJ),
            PREV_X,
            PageState::tint(state.has_prev()),
            s,
        ))
        .id();
    let next = parent
        .spawn(arrow(
            PageNextButton,
            asset_server,
            (NEXT_DDJ, NEXT_FOCUS_DDJ, NEXT_PRESS_DDJ),
            NEXT_X,
            PageState::tint(state.has_next()),
            s,
        ))
        .id();
    parent.spawn((
        PageLabel,
        Text::new(state.label()),
        TextFont {
            font: fonts.two.clone().into(),
            font_size: FontSize::Px(PART_H * 0.75 * s),
            ..default()
        },
        TextColor(Color::WHITE),
        TextLayout::justify(Justify::Center),
        abs_node(LABEL_RECT, s),
        Pickable::IGNORE,
    ));
    Some((prev, next))
}

/// One of the two buttons: the prototype's `Rect` has `w=h=0`, so the extent is
/// the art's own 36x12.
fn arrow<M: Component>(
    marker: M,
    asset_server: &AssetServer,
    art: (&'static str, &'static str, &'static str),
    x: f32,
    tint: Color,
    s: f32,
) -> impl Bundle {
    (
        marker,
        Button,
        Hovered::default(),
        abs_node((x, 0.0, PART_W, PART_H), s),
        ImageNode {
            image: asset_server.load(art.0),
            color: tint,
            image_mode: NodeImageMode::Stretch,
            ..default()
        },
        ImageButtonStyle {
            normal: asset_server.load(art.0),
            hover: asset_server.load(art.1),
            press: asset_server.load(art.2),
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The prototype's two buttons plus the art's own extent have to close on
    /// one width, and the gap between them has to be the 28 px the readout is
    /// placed in. If any of the four transcribed numbers drifts, this fails.
    #[test]
    fn prototype_geometry_closes() {
        assert_eq!(PREV_X, 0.0);
        assert_eq!(NEXT_X, 64.0);
        assert_eq!((PART_W, PART_H), (36.0, 12.0));
        assert_eq!(INTRINSIC_W, 100.0);
        // The readout sits exactly between the two buttons.
        assert_eq!(LABEL_RECT.0, PREV_X + PART_W);
        assert_eq!(LABEL_RECT.0 + LABEL_RECT.2, NEXT_X);
        assert_eq!(LABEL_RECT.2, 28.0);
    }

    /// Both authored instances are the same 385x26 band, and the widget's own
    /// parts fit inside it with room to spare. A transcription that made the
    /// parts wider than the band would be laying out over the window.
    #[test]
    fn authored_sites_are_one_band() {
        let widths: Vec<f32> = SITES.iter().map(|s| s.3).collect();
        let heights: Vec<f32> = SITES.iter().map(|s| s.4).collect();
        assert_eq!(widths, vec![385.0, 385.0]);
        assert_eq!(heights, vec![26.0, 26.0]);
        for site in SITES {
            assert!(
                INTRINSIC_W <= site.3 && PART_H <= site.4,
                "parts do not fit in {}",
                site.0
            );
        }
    }

    #[test]
    fn page_state_clamps_and_reports() {
        let first = PageState::new(1, 12);
        assert!(!first.has_prev());
        assert!(first.has_next());
        assert_eq!(first.label(), "1/12");

        let last = PageState::new(12, 12);
        assert!(last.has_prev());
        assert!(!last.has_next());

        // Out of range in both directions, and the zero-row case.
        assert_eq!(PageState::new(0, 12).current(), 1);
        assert_eq!(PageState::new(99, 12).current(), 12);
        let empty = PageState::new(1, 0);
        assert_eq!(empty.current(), 0);
        assert!(!empty.has_prev());
        assert!(!empty.has_next());
        assert_eq!(empty.label(), "-/-");
    }

    /// The measured rule from §28: at most one page means the strip is not
    /// drawn at all. Two pages is the first count that shows it, and the
    /// unreachable-in-practice `total == 0` (empty result set) must be silent
    /// too — the readout would otherwise print `-/-` on an empty list.
    #[test]
    fn a_single_page_hides_the_strip() {
        assert!(!PageState::new(1, 0).visible());
        assert!(!PageState::new(1, 1).visible());
        assert!(PageState::new(1, 2).visible());
        assert!(PageState::new(2, 2).visible());
        assert!(PageState::new(7, 12).visible());
        // A one-page strip would otherwise be a pair of buttons with both
        // directions unavailable and a "1/1" between them — exactly the
        // picture the original does not show.
        let one = PageState::new(1, 1);
        assert!(!one.has_prev() && !one.has_next());
        assert_eq!(one.label(), "1/1");
    }

    /// No `_disable` art exists, so the unavailable direction must be a tint
    /// and the available one must be untouched white.
    #[test]
    fn unavailable_direction_is_tinted_not_swapped() {
        assert_eq!(PageState::tint(true), Color::WHITE);
        assert_ne!(PageState::tint(false), Color::WHITE);
        assert!(PageState::tint(false).alpha() < 1.0);
        // Every art path we name is a real `mall_page_*` file, all six of them
        // distinct — a copy-paste that pointed `prev` at `next` art would draw
        // "Next" on both ends.
        let art = [
            PREV_DDJ,
            PREV_FOCUS_DDJ,
            PREV_PRESS_DDJ,
            NEXT_DDJ,
            NEXT_FOCUS_DDJ,
            NEXT_PRESS_DDJ,
        ];
        for path in art {
            assert!(path.starts_with("media://interface/mall/mall_page_"));
            assert!(path.ends_with(".ddj"));
        }
        let mut sorted = art.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 6);
    }
}
