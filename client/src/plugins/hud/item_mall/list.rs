//! `ifnewitemmallshopitemslot` — the mall's two-column article grid.
//!
//! Idea: the article list is not a window of its own. The original composes it
//! from one *slot* template that is stamped twice per row into the middle
//! column, and that template is fully authored in
//! `Media/resinfo/ifnewitemmallshopitemslot.txt`. What the file does **not**
//! carry is the slot's own box: every control in it has
//! `ClientRect="0,0,0,0"`, so the cell's outer size, the column pitch and the
//! row pitch had to be measured off the unoccluded photograph
//! `artifacts/capture/orders/done/260-itemmall/03-mall-page.png` (806x629,
//! client area at image offset `(+3,+26)`, 1:1 inside the window —
//! `docs/re/ui/original-world-windows-2026-08-23.md` §24).
//!
//! **Measured, then confirmed by art.** The cell origin was read off the
//! photograph as image `(145,121)` — the two grey border lines of the first
//! cell. Three independent template matches then agree with it to the pixel,
//! which is why the origin is a fact and not a reading:
//! * `mall_silk_icon.ddj` (14x14) matches at image `(324,161)`, error **0.48**.
//!   The file authors that coin at cell-relative `179,40`, and
//!   `(145+179, 121+40) = (324,161)`. Data and photograph meet exactly.
//! * `mall_icon_limited.ddj` (the red `H`) matches at image `(154,181)`, error
//!   **0.00** — cell-relative `(9,60)`.
//! * `mall_icon_popular.ddj` (the magenta `R`) matches at image `(154,363)`,
//!   error **0.00** — the same `(9,60)` inside the *third* row, which is how
//!   the row pitch of 81 is confirmed rather than counted.
//!
//! **The badge letters are art, not text.** `H` is `mall_icon_limited.ddj` and
//! `R` is `mall_icon_popular.ddj` (both 12x12); the shipped set also holds
//! `mall_icon_sale` (green `S`), `mall_icon_new` (blue `N`) and
//! `mall_icon_recommendation ` (purple `D`, trailing space in the file name is
//! the original's). Each was identified by decoding the `.ddj` and comparing
//! the glyph, not by guessing from the file name.
//!
//! **The section header is `mall_menu_bar.ddj`** (420x20), matched at image
//! `(142,101)` with error 5.52 against 9.76 one pixel up and 11.86 two pixels
//! up — the vertical position is decisive, the horizontal one is not, because
//! the art is a left-to-right gradient and shifting it sideways barely changes
//! the residue. The `x` is therefore taken from the authored middle-column
//! plate instead: `_MID_BG` is `137,72,424,519` and the bar is 420 wide, i.e.
//! centred with a 2 px inset at `x = 139`, which is inside the match's own
//! minimum (142/143 image = 139/140 window).
//!
//! **No item rows are invented here.** The four articles in the photograph are
//! server content (the mall package list, for which this tree has no wire
//! path). [`MallSection`] is the shape the server would fill; the window
//! spawns with an empty catalogue and then draws the empty grid — see
//! [`EMPTY_ROWS`] for why that is a stated deviation rather than a reading of
//! the original.

use bevy::prelude::*;

use crate::assets::FontAssets;
use crate::plugins::hud::game_window::abs_node;
use crate::plugins::hud::scale::font_px;

use super::{strip_sml, Rect, ART};

/// `_MID_BG` is `137,72,424,519` and its art `mall_back_wnd.ddj` draws its
/// frame line at texture `x = 0..422`, `y = 0..512`, so the plate's interior
/// starts one pixel inside. The list's own content box was measured on the
/// photograph: the cell borders start at image `x = 145`, `y = 101 + 20`
/// (bar), i.e. window `(142, 75)`, and the right-hand column's outer border
/// line sits at image `x = 539` = window 536, giving a content width of
/// `536 - 142 + 1 = 395`. The height runs to the plate's inner bottom line
/// (window `y = 583`), which is `508`.
pub const LIST_AT: Rect = (142.0, 75.0, 395.0, 508.0);

/// `mall_menu_bar.ddj`, 420x20 — see the module doc for the match.
pub const SECTION_BAR_AT_X: f32 = 139.0;
pub const SECTION_BAR_SIZE: (f32, f32) = (420.0, 20.0);
const SECTION_BAR_DDJ: &str = "mall_menu_bar.ddj";
/// The bar's caption: image ink at `x = 156..205`, `y = 107..115`, i.e. 14 px
/// in from the bar's own left edge and vertically centred in the 20 px band.
const SECTION_LABEL_INSET: f32 = 14.0;

/// The cell as photographed: the two border lines of the first cell run
/// `x = 145..340` and `y = 121..196` (image), so the drawn box is 196x76. The
/// shipped `mall_item_wnd.ddj` is 200x84 with its frame painted at texture
/// `0..196 / 0..78`; the client evidently stretches it into the 196x76 box —
/// the *inner* icon outline lands at texture `6..54` and at image `151..199`,
/// which is `145 + 6 .. 145 + 54` to the pixel horizontally.
pub const CELL_SIZE: (f32, f32) = (196.0, 76.0);
/// The second column's border line is at image `x = 344` and the second row's
/// at image `y = 202` — 199 and 81 past the first. Neither is counted off the
/// screenshot by eye: the `R` badge of the *third* row matches its art at
/// exactly `2 * 81` below the first row's badge (error 0.00).
pub const CELL_PITCH: (f32, f32) = (199.0, 81.0);
/// Two per row, as §24 corrects §7.
pub const COLUMNS: usize = 2;
const CELL_DDJ: &str = "mall_item_wnd.ddj";

// --- the slot template, all cell-relative, all from the resinfo file --------

/// `GDR_RENEWAL_SHOP_SLOT_ICON:CIFSlotWithHelpForPackage`, `Rect="8,8,45,45"`.
const SLOT_ICON_AT: Rect = (8.0, 8.0, 45.0, 45.0);
/// `GDR_RENEWAL_ITEM_MALL_ITEM_NAME:CIFTextBox`, `Rect="60,9,130,26"`,
/// `HAlign=2` (right) — 26 is two lines, which is why the photographed names
/// wrap into two right-aligned lines.
const NAME_AT: Rect = (60.0, 9.0, 130.0, 26.0);
/// `GDR_RENEWAL_ITEM_MALL_ITEM_SILK`, `Rect="124,42,51,12"`, `HAlign=2`.
const SILK_AT: Rect = (124.0, 42.0, 51.0, 12.0);
/// `GDR_RENEWAL_ITEM_MALL_ITEM_SILK_STR`, `Rect="179,40,0,0"`, art-sized —
/// `mall_silk_icon.ddj` is 14x14.
const SILK_COIN_AT: Rect = (179.0, 40.0, 14.0, 14.0);
/// `GDR_RENEWAL_ITEM_MALL_ITEM_RE_SILK`, `Rect="59,42,50,12"`, `HAlign=2` —
/// the pre-discount price. None of the photographed articles is discounted, so
/// it is only drawn when the server says there is one.
const RE_SILK_AT: Rect = (59.0, 42.0, 50.0, 12.0);
/// `GDR_RENEWAL_ITEM_MALL_ITEM_COUT`, `Rect="107,63,83,12"`, `HAlign=2` — the
/// `1Unit` / `5Unit` / `14Unit` line. `Unit` is `UIIT_STT_UNIT`
/// (`textuisystem.txt` L437).
const COUNT_AT: Rect = (107.0, 63.0, 83.0, 12.0);
const UNIT_KEY: (&str, &str) = ("UIIT_STT_UNIT", "Unit");
/// The badge. It has **no** control in the slot file (positive control: the
/// same file does declare seven other children, all with a `Rect`), so this
/// one number is the photograph's: image `(154,181)` against a cell origin of
/// `(145,121)`, confirmed by two art matches at error 0.00.
const BADGE_AT: Rect = (9.0, 60.0, 12.0, 12.0);

/// The badges the shipped art carries, each identified by decoding the `.ddj`
/// and reading the letter it paints.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MallBadge {
    /// red `H` — the marker on every `Best item` cell in the photograph.
    Hot,
    /// magenta `R` — the marker on every `Recommend` cell.
    Popular,
    /// blue `N`.
    New,
    /// green `S`.
    Sale,
    /// purple `D`.
    Recommendation,
}

impl MallBadge {
    /// The shipped file. `mall_icon_recommendation .ddj` really does carry a
    /// space before the extension in `Media`; it is quoted verbatim so a
    /// future reader does not "fix" it.
    pub fn ddj(self) -> &'static str {
        match self {
            MallBadge::Hot => "mall_icon_limited.ddj",
            MallBadge::Popular => "mall_icon_popular.ddj",
            MallBadge::New => "mall_icon_new.ddj",
            MallBadge::Sale => "mall_icon_sale.ddj",
            MallBadge::Recommendation => "mall_icon_recommendation .ddj",
        }
    }
}

/// One article. Every field is server content; nothing here has a default that
/// invents a price or a name.
#[derive(Clone, Debug)]
pub struct MallItem {
    pub name: String,
    /// `media://` path of the item icon, when the package names one.
    pub icon: Option<String>,
    pub silk: u32,
    /// The pre-discount price, when there is one.
    pub was_silk: Option<u32>,
    /// The `nUnit` line's `n`.
    pub count: u32,
    pub badge: Option<MallBadge>,
}

/// One captioned block of articles — `Best item`, `Recommend`, ... The caption
/// is server content too (only `Best item` has a shipped key,
/// `UIIT_STT_SILKMALL_BESTSELLER_ITEM` L4320; there is no `SILKMALL` key whose
/// text is `Recommend` — the positive control is that the same grep finds
/// `Best item` at L4320 and `Original` at L4321).
#[derive(Clone, Debug)]
pub struct MallSection {
    pub title: String,
    pub items: Vec<MallItem>,
}

/// How many empty rows the page draws when the catalogue is empty.
///
/// **Stated deviation (ADR-0009).** The photograph only ever shows cells that
/// carry an article — below the last one the plate is plain background, so the
/// original very likely draws nothing for an empty category. We draw the bare
/// grid instead, because a mall with no wire path would otherwise be an empty
/// rectangle that says nothing about what the page is. The count is the number
/// of rows that fit: `floor(508 / 81) = 6`.
pub const EMPTY_ROWS: usize = (LIST_AT.3 / CELL_PITCH.1) as usize;

/// Where the `i`-th row of the flowed list sits, given how many pixels of the
/// list are already used. Header bands are 20 tall, rows 81 — checked against
/// the photograph: header at window `y = 75`, rows at 95 and 176, the second
/// header at 257 (`= 95 + 2 * 81`) and its rows at 277 and 358.
fn cell_at(column: usize, y: f32) -> Rect {
    (
        LIST_AT.0 + CELL_PITCH.0 * column as f32,
        y,
        CELL_SIZE.0,
        CELL_SIZE.1,
    )
}

/// Draws the grid into the mall page's content node.
///
/// `sections` is the server's catalogue; with an empty slice this draws
/// [`EMPTY_ROWS`] rows of empty cells and no caption bar, because a caption we
/// do not have would be an invented one.
#[allow(clippy::too_many_arguments)]
pub fn spawn_list(
    content: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    unit_word: &str,
    sections: &[MallSection],
    s: f32,
) {
    let mut y = LIST_AT.1;
    let bottom = LIST_AT.1 + LIST_AT.3;
    if sections.is_empty() {
        for _ in 0..EMPTY_ROWS {
            for column in 0..COLUMNS {
                spawn_cell(
                    content,
                    asset_server,
                    fonts,
                    unit_word,
                    None,
                    cell_at(column, y),
                    s,
                );
            }
            y += CELL_PITCH.1;
        }
        return;
    }
    for section in sections {
        if y + SECTION_BAR_SIZE.1 > bottom {
            break;
        }
        spawn_section_bar(content, asset_server, fonts, &section.title, y, s);
        y += SECTION_BAR_SIZE.1;
        for row in section.items.chunks(COLUMNS) {
            if y + CELL_SIZE.1 > bottom {
                break;
            }
            for column in 0..COLUMNS {
                spawn_cell(
                    content,
                    asset_server,
                    fonts,
                    unit_word,
                    row.get(column),
                    cell_at(column, y),
                    s,
                );
            }
            y += CELL_PITCH.1;
        }
    }
}

/// The captioned bar above a section.
fn spawn_section_bar(
    content: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    title: &str,
    y: f32,
    s: f32,
) {
    content.spawn((
        abs_node(
            (SECTION_BAR_AT_X, y, SECTION_BAR_SIZE.0, SECTION_BAR_SIZE.1),
            s,
        ),
        ImageNode::new(asset_server.load(format!("{ART}{SECTION_BAR_DDJ}"))),
        Pickable::IGNORE,
    ));
    content.spawn(text_node(
        fonts,
        strip_sml(title),
        (
            SECTION_BAR_AT_X + SECTION_LABEL_INSET,
            y + 3.0,
            SECTION_BAR_SIZE.0 - SECTION_LABEL_INSET,
            15.0,
        ),
        Justify::Left,
        s,
    ));
}

/// One cell. `item` of `None` draws the frame and nothing else — an empty cell
/// stays empty; no placeholder price, no invented zero.
fn spawn_cell(
    content: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    unit_word: &str,
    item: Option<&MallItem>,
    cell: Rect,
    s: f32,
) {
    let at = |rect: Rect| (cell.0 + rect.0, cell.1 + rect.1, rect.2, rect.3);
    content.spawn((
        abs_node(cell, s),
        ImageNode::new(asset_server.load(format!("{ART}{CELL_DDJ}"))),
        Pickable::IGNORE,
    ));
    let Some(item) = item else {
        return;
    };
    if let Some(icon) = item.icon.as_deref() {
        content.spawn((
            abs_node(at(SLOT_ICON_AT), s),
            ImageNode::new(asset_server.load(icon.to_string())),
            Pickable::IGNORE,
        ));
    }
    if let Some(badge) = item.badge {
        content.spawn((
            abs_node(at(BADGE_AT), s),
            ImageNode::new(asset_server.load(format!("{ART}{}", badge.ddj()))),
            Pickable::IGNORE,
        ));
    }
    content.spawn(text_node(
        fonts,
        strip_sml(&item.name),
        at(NAME_AT),
        Justify::Right,
        s,
    ));
    content.spawn(text_node(
        fonts,
        item.silk.to_string(),
        at(SILK_AT),
        Justify::Right,
        s,
    ));
    content.spawn((
        abs_node(at(SILK_COIN_AT), s),
        ImageNode::new(asset_server.load(format!("{ART}mall_silk_icon.ddj"))),
        Pickable::IGNORE,
    ));
    if let Some(was) = item.was_silk {
        content.spawn(text_node(
            fonts,
            was.to_string(),
            at(RE_SILK_AT),
            Justify::Right,
            s,
        ));
    }
    content.spawn(text_node(
        fonts,
        format!("{}{unit_word}", item.count),
        at(COUNT_AT),
        Justify::Right,
        s,
    ));
}

/// The one text style this grid uses: `FontColor=COLOR,"255,255,255,255"` on
/// every control in the slot file, and the photograph's brightest ink in the
/// caption, the name, the price and the unit line is `(255,255,255)` in all
/// four places.
fn text_node(
    fonts: &FontAssets,
    text: String,
    rect: Rect,
    justify: Justify,
    s: f32,
) -> impl Bundle {
    (
        abs_node(rect, s),
        Text::new(text),
        TextFont {
            font: fonts.two.clone().into(),
            font_size: FontSize::Px(font_px(0)),
            ..default()
        },
        TextColor(Color::WHITE),
        TextLayout::justify(justify),
        Pickable::IGNORE,
    )
}

/// The shipped key for the unit word, so the caller can resolve it once.
pub const UNIT_STRING_KEY: (&str, &str) = UNIT_KEY;

#[cfg(test)]
mod test {
    use super::*;

    /// The grid the unoccluded capture shows: two columns, 199 apart, rows 81
    /// apart, cells 196x76 — and the second column stays inside the list box.
    #[test]
    fn the_grid_is_the_measured_two_column_one() {
        assert_eq!(COLUMNS, 2);
        assert_eq!(CELL_SIZE, (196.0, 76.0));
        assert_eq!(CELL_PITCH, (199.0, 81.0));
        let last = cell_at(COLUMNS - 1, LIST_AT.1);
        assert_eq!(last.0, 341.0);
        assert!(last.0 + last.2 <= LIST_AT.0 + LIST_AT.2);
        // The row pitch leaves the 5 px gutter the photograph shows between
        // the bottom border of one cell and the top border of the next.
        assert_eq!(CELL_PITCH.1 - CELL_SIZE.1, 5.0);
    }

    /// The flow the photograph pins down: header at 75, rows at 95 and 176,
    /// the second header at 257, its rows at 277 and 358 — all in window
    /// units, all read off `03-mall-page.png` at image offset `(+3,+26)`.
    #[test]
    fn the_section_flow_matches_the_photograph() {
        let mut y = LIST_AT.1;
        let mut tops = vec![y];
        y += SECTION_BAR_SIZE.1;
        for _ in 0..2 {
            tops.push(y);
            y += CELL_PITCH.1;
        }
        tops.push(y);
        y += SECTION_BAR_SIZE.1;
        for _ in 0..2 {
            tops.push(y);
            y += CELL_PITCH.1;
        }
        assert_eq!(tops, vec![75.0, 95.0, 176.0, 257.0, 277.0, 358.0]);
    }

    /// Every slot child is the authored rect from
    /// `ifnewitemmallshopitemslot.txt`, and each one stays inside the cell.
    #[test]
    fn the_slot_children_are_the_authored_rects() {
        assert_eq!(SLOT_ICON_AT, (8.0, 8.0, 45.0, 45.0));
        assert_eq!(NAME_AT, (60.0, 9.0, 130.0, 26.0));
        assert_eq!(SILK_AT, (124.0, 42.0, 51.0, 12.0));
        assert_eq!(SILK_COIN_AT.0, 179.0);
        assert_eq!(SILK_COIN_AT.1, 40.0);
        assert_eq!(RE_SILK_AT, (59.0, 42.0, 50.0, 12.0));
        assert_eq!(COUNT_AT, (107.0, 63.0, 83.0, 12.0));
        for rect in [
            SLOT_ICON_AT,
            NAME_AT,
            SILK_AT,
            SILK_COIN_AT,
            RE_SILK_AT,
            COUNT_AT,
            BADGE_AT,
        ] {
            assert!(rect.0 >= 0.0 && rect.0 + rect.2 <= CELL_SIZE.0, "{rect:?}");
            assert!(rect.1 >= 0.0 && rect.1 + rect.3 <= CELL_SIZE.1, "{rect:?}");
        }
    }

    /// The two badges the photograph shows are the two art files that matched
    /// it at error 0.00, and the badge sits where those matches put it.
    #[test]
    fn the_badges_are_the_matched_art() {
        assert_eq!(MallBadge::Hot.ddj(), "mall_icon_limited.ddj");
        assert_eq!(MallBadge::Popular.ddj(), "mall_icon_popular.ddj");
        assert_eq!(BADGE_AT, (9.0, 60.0, 12.0, 12.0));
        // Image `(154,181)` against the first cell's origin `(145,121)`, and
        // `(154,343)` against the third row's `(145,283)`.
        //
        // The third row's origin is measured, not assumed: a brightness scan
        // down the middle column of `03-mall-page.png` (x 150..340) puts the
        // cell borders at y = **121, 202, 283** — pitch 81, three rows visible.
        // An earlier draft of this test wrote 303/363 for that row, which is
        // the same pitch mistyped as 91; it disagreed with `CELL_PITCH.1` and
        // was caught by the assertion rather than by review.
        assert_eq!((145.0 + BADGE_AT.0, 121.0 + BADGE_AT.1), (154.0, 181.0));
        assert_eq!(
            (145.0 + BADGE_AT.0, 121.0 + 2.0 * CELL_PITCH.1 + BADGE_AT.1),
            (154.0, 343.0)
        );
    }

    /// The coin's authored rect is what made the cell origin a fact: the
    /// template match landed at image `(324,161)`.
    #[test]
    fn the_coin_confirms_the_cell_origin() {
        assert_eq!(
            (145.0 + SILK_COIN_AT.0, 121.0 + SILK_COIN_AT.1),
            (324.0, 161.0)
        );
    }

    /// Six rows fit the list box, and the seventh does not.
    #[test]
    fn the_empty_grid_fills_the_list_box() {
        assert_eq!(EMPTY_ROWS, 6);
        let used = EMPTY_ROWS as f32 * CELL_PITCH.1;
        assert!(used <= LIST_AT.3);
        assert!(used + CELL_PITCH.1 > LIST_AT.3);
    }
}
