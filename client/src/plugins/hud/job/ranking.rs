//! The two job ranking windows — `ifjobrank` (388x369) and
//! `ifjobcontributionrank` (388x500), the answer to `0xB0E4`.
//!
//! Idea: one builder, two authored descriptors. Both windows are the same
//! object — a title, a `com_blacksquare_` well, a strip of art-sized guild
//! headers and ten `com_bar01_` rows on pitch 23 — so the geometry is data
//! ([`RankLayout`]) transcribed from the user's own `Media/resinfo/` and the
//! code walks it, rather than two near-copies of the same spawn.
//!
//! **`rank_kind` picks a window, not a tab.** `job-trade-system.md` §8 point 8
//! predicted "two tabs, `rank_kind` 0/1"; the data says otherwise and the data
//! wins. `ginterface.txt` registers *two* windows — `GDR_JOB_RANK:CIFJobRank`
//! id 64 `Rect="0,0,388,369"` (`:1238`, section `JobActiveRank`) and
//! `GDR_JOB_CONTRIBUTION_RANK:CIFJobContributionRank` id 65 `Rect="0,0,388,500"`
//! (`:1261`, section `JobContributionRank`) — each with its own tree, its own
//! row prototype (`ifjobrankslot.txt` 5 cells vs `ifjobcontributionrankslot.txt`
//! 4 cells) and its own title static, and neither tree declares a tab control.
//! The contribution window is 131 px taller purely because of the "my donation
//! this week" panel stacked above its ranking half.
//!
//! Both declare `DDJ="interface\messagebox\msgbox2_window_"`, so both are drawn
//! on the measured [`game_window::MSGBOX2_WINDOW`] chrome
//! (`docs/re/ui/msgbox2-chrome.md`), like `job::prev_info`. Their registration
//! carries `Text=""` — unlike `ifprevjobinfo` these windows have **no caption in
//! the title band**; the heading is a static inside the tree
//! (`_STA_TITLE`), and that is where it is drawn here.
//!
//! Every rect below is the authored one, cited by block-header line in
//! `Media/resinfo/`; `w,h = 0,0` means the original takes the extent from the
//! art, and the size given is that `.ddj`'s measured DDS extent.
//!
//! What this window deliberately leaves blank, and why:
//!
//! - **`_GRADENAME`** (activity rows only): `assets::textdata::job::rank_name`
//!   could resolve one, but only after choosing the Chinese or the European key
//!   set, and `0xB0E4` does not say which applies (`MERCHANT_6` vs
//!   `MERCHANT_6_NEW` is `[U]` on top of that). Same blank, same reason, as
//!   `prev_info`'s.
//! - **The contribution window's own-donation panel** (`_INFO_PML`,
//!   `_INFO_ALIAS`, `_INFO_GRADE`, `_INFO_AMOUNT`): no field of `0xB0E4` carries
//!   the *viewer's* contribution. The authored labels are drawn, the value cells
//!   stay empty. The tree's grey `_INFO_NOEXIST` line ("corresponding
//!   information does not exist") is **not** drawn: it states a server fact we
//!   never received.
//! - **`_SPIN_CTRL`** (`ifjobrank.txt:6` `168,334,52,18`,
//!   `ifjobcontributionrank.txt:25` `168,464,52,18`): the page spinner has no
//!   art of its own (prototype `ifspincontrol.txt`) and nothing to drive — the
//!   `0x70E4` request is not sent by this client yet, so paging would be a
//!   control that cannot ask for a page.
//! - **`JobRankEntry::trailing`**, the activity branch's extra byte, is `[U]`
//!   in the packet and has no cell in the descriptor either. Not shown.
//!
//! One measured disagreement, recorded rather than smoothed: the trees author a
//! background tile 1 px larger than the shell's interior — `_BG` `16,39,356,314`
//! against the family's `16,40,356,313` (activity, 1 px *up*), and
//! `16,40,356,445` against `16,40,356,444` (contribution, 1 px *down*). The
//! shell's own tile is used; a 1 px authored overshoot into the chrome is not
//! worth a second tile node.

use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use packets::agent::job::{JobRankEntry, JOB_RANK_KIND_CONTRIBUTION};

use crate::assets::FontAssets;
use crate::plugins::hud::game_window::{self, WindowGeometry};
use crate::plugins::hud::scale::{font_px, hud_scale};
use crate::plugins::net::job::{JobRanking, JobRankingPage};
use crate::plugins::textdata::ClientUiStrings;

/// A resinfo `Rect=`: `(x, y, w, h)` in window units, the way every descriptor
/// writes it.
type Rect = (f32, f32, f32, f32);
/// A `(textdata key, shipped English)` pair — the fallback is the string that
/// key carries in the user's own `textdata/textuisystem.txt`.
type UiKey = (&'static str, &'static str);
/// One piece of a multi-part plate: where it goes and which file suffix it is.
type Piece = (Rect, &'static str);
/// One `(label rect, label key, value rect)` row of the donation panel.
type InfoRow = (Rect, UiKey, Rect);

/// `com_bg_tile_b`, the `_BG` tile letter of both trees.
const BG_TILE: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_b.ddj";
/// `com_blacksquare_` is flat black behind six 4 px trim pieces.
const BLACKSQUARE_DIR: &str = "media://interface/ifcommon/com_blacksquare_";
const BLACKSQUARE_PIECE: f32 = 4.0;
/// The row plate prefix; `com_bar01_left/mid/right.ddj` measure 4x24 / 24x24 /
/// 4x24, so the caps are 4 px and the middle stretches.
const ROW_BAR_DIR: &str = "media://interface/ifcommon/com_bar01_";
const ROW_BAR_CAP: f32 = 4.0;
/// Ten `_STA_SLOT_01..10`, `346,24` at x=20, pitch 23 — in both trees.
const ROWS: usize = 10;
const ROW_RECT: Rect = (20.0, 0.0, 346.0, 24.0);
const ROW_PITCH: f32 = 23.0;
/// `frameg_wnd_` pieces: corners 24x16, `mid_*` 128x16, `*_side` 24x52 — a
/// 24 px side / 16 px top-and-bottom ring (same measurement as `cos::info`).
const FRAME_DIR: &str = "media://interface/frame/frameg_wnd_";
const FRAME_SIDE: f32 = 24.0;
const FRAME_TOP: f32 = 16.0;
/// The label colour both trees give their `_STA` captions
/// (`FontColor=255,239,218,164`); the runtime cells are white
/// (`255,255,255,255`).
const LABEL_COLOR: Color = Color::srgb_u8(239, 218, 164);

/// Which wire field a row cell shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    /// `_RANK` — the server's own 1-based position.
    Rank,
    /// `_ALIAS` — the *job alias*, not the character name.
    Alias,
    /// `_GRADE` — `job_level`.
    Grade,
    /// `_EXP` / `_AMOUNT` — `points`.
    Points,
    /// `_GRADENAME` — declared, left empty (module doc).
    GradeName,
}

/// One row cell: its slot-local rect and its `HAlign`.
struct SlotCell {
    cell: Cell,
    rect: Rect,
    justify: Justify,
}

/// One art-sized column header.
struct Header {
    /// Authored `Rect` x/y plus the `.ddj`'s measured extent.
    rect: Rect,
    ddj: &'static str,
    text: UiKey,
}

/// A whole window, as its descriptor authors it.
struct RankLayout {
    /// `ginterface.txt` `Rect` of the window itself.
    window: (f32, f32),
    /// `_STA_TITLE`: rect and key.
    title_rect: Rect,
    title: UiKey,
    /// `_BLACKSQUARE:CIFStretchWnd` — the list well.
    plate: Rect,
    headers: &'static [Header],
    /// The `CIFBarWnd` header of the contribution window (`com_bar02_`, caps
    /// 12x24), which the activity window does not have.
    bar_header: Option<(Header, f32)>,
    /// y of `_STA_SLOT_01`.
    first_row_y: f32,
    slot: &'static [SlotCell],
    /// The contribution window's "my donation" panel, when the tree has one.
    info: Option<InfoPanel>,
}

/// `ifjobcontributionrank.txt`'s upper half: a `frameg_wnd_` frame, a
/// runtime-fed `CIFPML` and three label/value pairs.
struct InfoPanel {
    frame: Rect,
    /// `_INFO_PML` `31,68,324,15` — fed at runtime from
    /// `textuisystem.txt:3164/3165` (`UIIT_STT_JOBGUILD_MYCONTRIBUTE`/`2`).
    /// Which of the pair applies is not in this packet, so the box stays empty.
    pml: Rect,
    /// `(label rect, label key, value rect)` ×3, top-to-bottom as authored.
    labels: [InfoRow; 3],
}

/// `ifjobrank.txt` — the weekly activity ranking, `ginterface.txt:1247`
/// `0,0,388,369`.
const ACTIVITY: RankLayout = RankLayout {
    window: (388.0, 369.0),
    // `:310` `GDR_JOB_RANK_STA_TITLE` id 8.
    title_rect: (21.0, 50.0, 230.0, 15.0),
    title: (
        "UIIT_STT_JOBGUILD_TITLE",
        "Last week's job activity ranking",
    ),
    // `:329` `_BLACKSQUARE` id 6, `StretchType=0`.
    plate: (17.0, 66.0, 352.0, 260.0),
    // `:291,:272,:253,:234,:215` — five art-sized statics at y=69, all
    // `HAlign 1`, all borrowing guild art. Extents measured from the DDS
    // headers: `gil_subj_button03` 44x24, `07` 116x24, `12` 56x24, `06` 96x24.
    headers: &[
        Header {
            rect: (20.0, 69.0, 44.0, 24.0),
            ddj: "media://interface/guild/gil_subj_button03.ddj",
            text: ("UIIT_STT_JOBGUILD_RANKMENU", "Ranking"),
        },
        Header {
            rect: (60.0, 69.0, 116.0, 24.0),
            ddj: "media://interface/guild/gil_subj_button07.ddj",
            text: ("UIIT_STT_CHAR_ALIAS", "Job alias"),
        },
        Header {
            rect: (176.0, 69.0, 44.0, 24.0),
            ddj: "media://interface/guild/gil_subj_button03.ddj",
            text: ("UIIT_STT_GRADE", "Level"),
        },
        Header {
            rect: (217.0, 69.0, 56.0, 24.0),
            ddj: "media://interface/guild/gil_subj_button12.ddj",
            text: ("UIIT_STT_JOBEXP", "Experience"),
        },
        Header {
            rect: (271.0, 69.0, 96.0, 24.0),
            ddj: "media://interface/guild/gil_subj_button06.ddj",
            text: ("UIIT_STT_GRADENAME", "Title of level"),
        },
    ],
    bar_header: None,
    first_row_y: 91.0,
    // `ifjobrankslot.txt` `:82,:63,:44,:25,:6`, all `HAlign 1`, slot-local.
    slot: &[
        SlotCell {
            cell: Cell::Rank,
            rect: (7.0, 6.0, 24.0, 15.0),
            justify: Justify::Center,
        },
        SlotCell {
            cell: Cell::Alias,
            rect: (46.0, 6.0, 105.0, 15.0),
            justify: Justify::Center,
        },
        SlotCell {
            cell: Cell::Grade,
            rect: (163.0, 6.0, 26.0, 15.0),
            justify: Justify::Center,
        },
        SlotCell {
            cell: Cell::Points,
            rect: (204.0, 6.0, 41.0, 15.0),
            justify: Justify::Center,
        },
        SlotCell {
            cell: Cell::GradeName,
            rect: (256.0, 6.0, 82.0, 15.0),
            justify: Justify::Center,
        },
    ],
    info: None,
};

/// `ifjobcontributionrank.txt` — the weekly donation ranking,
/// `ginterface.txt:1270` `0,0,388,500`.
const CONTRIBUTION: RankLayout = RankLayout {
    window: (388.0, 500.0),
    // `:310` `_STA_TITLE` id 20.
    title_rect: (21.0, 179.0, 188.0, 15.0),
    title: (
        "UIIT_STT_JOBGUILD_CONTRIBUTERANK",
        "Last week's donation ranking",
    ),
    // `:462` `_BLACKSQUARE` id 6.
    plate: (17.0, 195.0, 352.0, 260.0),
    // `:291,:272,:253` — three art-sized statics at y=198; the fourth column
    // header is the `CIFBarWnd` below.
    headers: &[
        Header {
            rect: (20.0, 198.0, 44.0, 24.0),
            ddj: "media://interface/guild/gil_subj_button03.ddj",
            text: ("UIIT_STT_JOBGUILD_RANKMENU", "Ranking"),
        },
        Header {
            rect: (60.0, 198.0, 116.0, 24.0),
            ddj: "media://interface/guild/gil_subj_button07.ddj",
            text: ("UIIT_STT_CHAR_ALIAS", "Job alias"),
        },
        Header {
            rect: (176.0, 198.0, 44.0, 24.0),
            ddj: "media://interface/guild/gil_subj_button03.ddj",
            text: ("UIIT_STT_GRADE", "Level"),
        },
    ],
    // `:234` `_SUBJ_STA_CONTRIBUTION:CIFBarWnd` id 28 `217,198,150,22` on
    // `com_bar02_` (caps 12x24, mid 24x24) — a *sized* rect, not art-sized.
    bar_header: Some((
        Header {
            rect: (217.0, 198.0, 150.0, 22.0),
            ddj: "media://interface/ifcommon/com_bar02_",
            text: ("UIIT_STT_DONATION", "Donation amount"),
        },
        12.0,
    )),
    first_row_y: 220.0,
    // `ifjobcontributionrankslot.txt` `:63,:44,:25,:6` — four cells, no
    // gradename column, and the amount is right-aligned (`HAlign 2`).
    slot: &[
        SlotCell {
            cell: Cell::Rank,
            rect: (9.0, 8.0, 22.0, 15.0),
            justify: Justify::Center,
        },
        SlotCell {
            cell: Cell::Alias,
            rect: (47.0, 8.0, 105.0, 15.0),
            justify: Justify::Center,
        },
        SlotCell {
            cell: Cell::Grade,
            rect: (163.0, 8.0, 23.0, 15.0),
            justify: Justify::Center,
        },
        SlotCell {
            cell: Cell::Points,
            rect: (206.0, 8.0, 132.0, 15.0),
            justify: Justify::Right,
        },
    ],
    info: Some(InfoPanel {
        // `:482` `_INFO_FRAME:CIFFrame` id 5, `frameg_wnd_`.
        frame: (19.0, 47.0, 349.0, 113.0),
        // `:443` `_INFO_PML:CIFPML` id 10.
        pml: (31.0, 68.0, 324.0, 15.0),
        labels: [
            // `:424` `_INFO_ALIAS_STA` id 11 / `:367` `_INFO_ALIAS` id 14 (HA 2).
            (
                (41.0, 104.0, 77.0, 15.0),
                ("UIIT_STT_CHAR_ALIAS", "Job alias"),
                (135.0, 104.0, 94.0, 15.0),
            ),
            // `:405` `_INFO_GRADE_STA` id 12 / `:348` `_INFO_GRADE` id 15.
            (
                (260.0, 104.0, 37.0, 15.0),
                ("UIIT_STT_GRADE", "Level"),
                (307.0, 104.0, 37.0, 15.0),
            ),
            // `:386` `_INFO_AMOUNT_STA` id 13 / `:329` `_INFO_AMOUNT` id 16.
            (
                (41.0, 128.0, 91.0, 15.0),
                ("UIIT_STT_DONATION", "Donation amount"),
                (145.0, 127.0, 199.0, 15.0),
            ),
        ],
    }),
};

/// The layout `rank_kind` selects.
fn layout_for(rank_kind: u8) -> &'static RankLayout {
    if rank_kind == JOB_RANK_KIND_CONTRIBUTION {
        &CONTRIBUTION
    } else {
        &ACTIVITY
    }
}

/// The whole window.
#[derive(Component)]
pub struct JobRankWindowRoot;

/// The page a spawned window was built from, so a repeat of the same page does
/// not respawn it.
#[derive(Component, PartialEq)]
struct JobRankWindowData(JobRankingPage);

/// Opens the matching window when a `0xB0E4` page lands, and rebuilds it when
/// the page changes. The request that produces such a page is the job NPC's
/// ranking line (`hud::npc_dialog::job_menu`, `job-trade-system.md` §11.8);
/// nothing else opens this window.
fn sync_job_rank_window(
    mut commands: Commands,
    ranking: Res<JobRanking>,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
    open: Query<(Entity, &JobRankWindowData), With<JobRankWindowRoot>>,
) {
    if !ranking.is_changed() {
        return;
    }
    let Some(page) = ranking.0.as_ref() else {
        return;
    };
    let Ok(camera) = cam_query.single() else {
        warn!("job ranking window: no 2d camera to attach to");
        return;
    };
    for (entity, data) in open.iter() {
        if data.0 == *page {
            return;
        }
        commands.entity(entity).despawn();
    }
    spawn_job_rank_window(
        &mut commands,
        &asset_server,
        &fonts,
        &ui_strings,
        camera,
        page.clone(),
    );
}

/// Builds the window for a page. Split out of the system so a test can call it.
pub fn spawn_job_rank_window(
    commands: &mut Commands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
    camera: Entity,
    page: JobRankingPage,
) -> Entity {
    let s = hud_scale();
    let layout = layout_for(page.rank_kind);
    let window = game_window::spawn_game_window_with(
        commands,
        asset_server,
        fonts,
        camera,
        // The registration carries `Text=""`: no caption in the title band.
        // The heading is `_STA_TITLE`, drawn below at its authored rect.
        "",
        WindowGeometry {
            outer: layout.window,
            // The trees' rects are window-absolute, so the content container
            // sits on the window origin and every child uses its `Rect=`.
            content_at: (0.0, 0.0),
        },
        None,
        (24.0, 120.0),
        s,
        game_window::GameWindowStyle {
            chrome: &game_window::MSGBOX2_WINDOW,
            bg_tile: BG_TILE,
            ..default()
        },
    );
    commands
        .entity(window.root)
        .insert((JobRankWindowRoot, JobRankWindowData(page.clone())));
    commands
        .entity(window.expect_close_button())
        .observe(on_close_button);

    let font = fonts.two.clone();
    let text_px = font_px(0);
    let text_font = TextFont {
        font: font.clone().into(),
        font_size: FontSize::Px(text_px),
        ..default()
    };
    commands.entity(window.content).with_children(|content| {
        let image = |rect: Rect, path: String| {
            (
                game_window::abs_node(rect, s),
                ImageNode {
                    image: asset_server.load(path),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                Pickable::IGNORE,
            )
        };
        let label = |rect: Rect, text: String, color: Color, justify: Justify| {
            (
                game_window::abs_node(rect, s),
                Text::new(text),
                text_font.clone(),
                TextColor(color),
                TextLayout::justify(justify),
                Pickable::IGNORE,
            )
        };

        // The contribution window's own-donation panel: frame, empty PML, the
        // three authored labels and their (empty) value cells.
        if let Some(info) = layout.info.as_ref() {
            let (fx, fy, fw, fh) = info.frame;
            for ((x, y, w, h), piece) in frame_ring(fw, fh) {
                content.spawn(image(
                    (fx + x, fy + y, w, h),
                    format!("{FRAME_DIR}{piece}.ddj"),
                ));
            }
            content.spawn(label(
                info.pml,
                String::new(),
                Color::WHITE,
                Justify::Center,
            ));
            for (label_rect, key, value_rect) in info.labels.iter() {
                content.spawn(label(
                    *label_rect,
                    ui_strings.get_or(key.0, key.1).to_string(),
                    LABEL_COLOR,
                    Justify::Left,
                ));
                // `HAlign=2` on all three value cells — and empty: no field of
                // `0xB0E4` carries the viewer's own contribution.
                content.spawn(label(
                    *value_rect,
                    String::new(),
                    Color::WHITE,
                    Justify::Right,
                ));
            }
        }

        // `_STA_TITLE` — the heading, `HAlign 0`.
        content.spawn(label(
            layout.title_rect,
            ui_strings
                .get_or(layout.title.0, layout.title.1)
                .to_string(),
            Color::WHITE,
            Justify::Left,
        ));

        // `_BLACKSQUARE` — flat black plus its six 4 px trim pieces.
        let (px, py, pw, ph) = layout.plate;
        content.spawn((
            game_window::abs_node(layout.plate, s),
            BackgroundColor(Color::BLACK),
            Pickable::IGNORE,
        ));
        for ((x, y, w, h), piece) in blacksquare_ring(pw, ph) {
            content.spawn(image(
                (px + x, py + y, w, h),
                format!("{BLACKSQUARE_DIR}{piece}.ddj"),
            ));
        }

        // The column headers, art-sized, each with its centred caption.
        for header in layout.headers {
            content.spawn(image(header.rect, header.ddj.to_string()));
            content.spawn(label(
                header.rect,
                ui_strings.get_or(header.text.0, header.text.1).to_string(),
                Color::WHITE,
                Justify::Center,
            ));
        }
        if let Some((header, cap)) = layout.bar_header.as_ref() {
            for (rect, piece) in bar(header.rect, *cap) {
                content.spawn(image(rect, format!("{}{piece}.ddj", header.ddj)));
            }
            content.spawn(label(
                header.rect,
                ui_strings.get_or(header.text.0, header.text.1).to_string(),
                Color::WHITE,
                Justify::Center,
            ));
        }

        // Ten rows on pitch 23. A row past the answered page is not drawn at
        // all — the original leaves its slot plate up, but an empty plate and a
        // missing plate say the same thing and the empty one costs 40 nodes.
        for (index, entry) in page.entries.iter().take(ROWS).enumerate() {
            let (rx, _, rw, rh) = ROW_RECT;
            let ry = layout.first_row_y + ROW_PITCH * index as f32;
            for (rect, piece) in bar((rx, ry, rw, rh), ROW_BAR_CAP) {
                content.spawn(image(rect, format!("{ROW_BAR_DIR}{piece}.ddj")));
            }
            for slot in layout.slot {
                let (cx, cy, cw, ch) = slot.rect;
                content.spawn(label(
                    (rx + cx, ry + cy, cw, ch),
                    cell_text(slot.cell, entry),
                    Color::WHITE,
                    slot.justify,
                ));
            }
        }
    });
    window.root
}

/// What a cell shows for a row. `GradeName` is the declared-but-empty one — see
/// the module doc.
fn cell_text(cell: Cell, entry: &JobRankEntry) -> String {
    match cell {
        Cell::Rank => entry.rank.to_string(),
        Cell::Alias => entry.name.clone(),
        Cell::Grade => entry.job_level.to_string(),
        Cell::Points => entry.points.to_string(),
        Cell::GradeName => String::new(),
    }
}

/// The 3 pieces of a `com_bar0*_` plate over a rect: two caps and the middle.
fn bar(rect: Rect, cap: f32) -> [Piece; 3] {
    let (x, y, w, h) = rect;
    [
        ((x, y, cap, h), "left"),
        ((x + cap, y, w - 2.0 * cap, h), "mid"),
        ((x + w - cap, y, cap, h), "right"),
    ]
}

/// The six 4 px trim pieces of a `com_blacksquare_` plate over a `w x h` box.
fn blacksquare_ring(w: f32, h: f32) -> [Piece; 6] {
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

/// The eight pieces of a `frameg_wnd_` ring over a `w x h` box.
fn frame_ring(w: f32, h: f32) -> [Piece; 8] {
    let (sw, th) = (FRAME_SIDE, FRAME_TOP);
    [
        ((0.0, 0.0, sw, th), "left_up"),
        ((sw, 0.0, w - 2.0 * sw, th), "mid_up"),
        ((w - sw, 0.0, sw, th), "right_up"),
        ((0.0, th, sw, h - 2.0 * th), "left_side"),
        ((w - sw, th, sw, h - 2.0 * th), "right_side"),
        ((0.0, h - th, sw, th), "left_down"),
        ((sw, h - th, w - 2.0 * sw, th), "mid_down"),
        ((w - sw, h - th, sw, th), "right_down"),
    ]
}

/// The (X) closes the window.
fn on_close_button(
    activate: On<Activate>,
    mut commands: Commands,
    parents: Query<&ChildOf>,
    roots: Query<Entity, With<JobRankWindowRoot>>,
) {
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

/// Registers the two ranking windows.
pub struct JobRankingPlugin;

impl Plugin for JobRankingPlugin {
    fn build(&self, app: &mut App) {
        use crate::scenes::SceneState;

        // Same gate, same reason as `prev_info`: `FontAssets` exists only in
        // the HUD scenes, and an ungated `Update` system asking for it fails
        // parameter validation in the loading screen and takes the process
        // with it. `super::super::hud_scenes` is the condition the tree already
        // carries (`hud/mod.rs`) — not a second hand-written copy of it.
        app.add_systems(
            Update,
            sync_job_rank_window.run_if(super::super::hud_scenes),
        )
        .add_systems(OnExit(SceneState::GameWorld), cleanup_job_rank_window);
    }
}

/// Leaving the world takes the window with it.
fn cleanup_job_rank_window(
    mut commands: Commands,
    windows: Query<Entity, With<JobRankWindowRoot>>,
) {
    for entity in windows.iter() {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use packets::agent::job::JOB_RANK_KIND_ACTIVITY;

    /// The row geometry is the descriptors' own: ten slots, `346,24` at x=20,
    /// pitch 23 — and the same law in *both* trees. The y ladders are the
    /// authored `_STA_SLOT_01..10` values, not a derived series.
    #[test]
    fn the_ten_row_slots_sit_on_the_authored_pitch() {
        let ys = |layout: &RankLayout| -> Vec<f32> {
            (0..ROWS)
                .map(|i| layout.first_row_y + ROW_PITCH * i as f32)
                .collect()
        };
        assert_eq!(
            ys(&ACTIVITY),
            vec![91.0, 114.0, 137.0, 160.0, 183.0, 206.0, 229.0, 252.0, 275.0, 298.0]
        );
        assert_eq!(
            ys(&CONTRIBUTION),
            vec![220.0, 243.0, 266.0, 289.0, 312.0, 335.0, 358.0, 381.0, 404.0, 427.0]
        );
        // Ten rows of 24 on pitch 23 close *flush against the well's trim*:
        // the last row's bottom is exactly the 4 px `com_blacksquare_` piece
        // short of the plate's bottom edge — in both trees, which is what says
        // the ten-row count and the pitch belong to this plate height.
        for layout in [&ACTIVITY, &CONTRIBUTION] {
            let last_bottom = layout.first_row_y + ROW_PITCH * (ROWS - 1) as f32 + ROW_RECT.3;
            assert_eq!(
                last_bottom,
                layout.plate.1 + layout.plate.3 - BLACKSQUARE_PIECE
            );
        }
    }

    /// `rank_kind` picks a *window*, and the two windows are the authored ones:
    /// different extent, different title key, different cell count.
    #[test]
    fn the_two_rank_kinds_are_two_authored_windows_not_two_tabs() {
        assert_eq!(layout_for(JOB_RANK_KIND_ACTIVITY).window, (388.0, 369.0));
        assert_eq!(
            layout_for(JOB_RANK_KIND_CONTRIBUTION).window,
            (388.0, 500.0)
        );
        assert_eq!(ACTIVITY.slot.len(), 5, "ifjobrankslot.txt has five cells");
        assert_eq!(
            CONTRIBUTION.slot.len(),
            4,
            "ifjobcontributionrankslot.txt has four — no gradename column"
        );
        // The contribution slot's last cell is the right-aligned amount.
        let amount = CONTRIBUTION.slot.last().expect("four cells");
        assert_eq!(amount.cell, Cell::Points);
        assert_eq!(amount.justify, Justify::Right);
        // And only the contribution tree carries the own-donation panel.
        assert!(ACTIVITY.info.is_none());
        assert!(CONTRIBUTION.info.is_some());
        // An unknown `rank_kind` degrades to the activity window rather than
        // failing: a server push must not be able to blank the screen.
        assert_eq!(layout_for(0x42).window, ACTIVITY.window);
    }

    /// The cells that carry wire values carry them verbatim, and `_GRADENAME`
    /// stays empty — the key-set blank the module doc states.
    #[test]
    fn the_cells_show_the_wire_and_the_gradename_stays_blank() {
        let entry = JobRankEntry {
            rank: 3,
            name: "Alias".to_string(),
            job_level: 5,
            points: 1234,
            trailing: Some(9),
        };
        assert_eq!(cell_text(Cell::Rank, &entry), "3");
        assert_eq!(cell_text(Cell::Alias, &entry), "Alias");
        assert_eq!(cell_text(Cell::Grade, &entry), "5");
        assert_eq!(cell_text(Cell::Points, &entry), "1234");
        assert_eq!(cell_text(Cell::GradeName, &entry), "");
        // `trailing` is `[U]`: no cell shows it.
        for cell in [
            Cell::Rank,
            Cell::Alias,
            Cell::Grade,
            Cell::Points,
            Cell::GradeName,
        ] {
            assert_ne!(cell_text(cell, &entry), "9");
        }
    }

    /// Both windows are drawn on the family their own registration declares,
    /// and its interior inset is what makes the authored `_BG` rects close.
    #[test]
    fn both_windows_use_the_measured_msgbox2_chrome() {
        let chrome = &game_window::MSGBOX2_WINDOW;
        assert_eq!(chrome.dir, "media://interface/messagebox/msgbox2_window_");
        assert_eq!(
            (chrome.side_w, chrome.top_h, chrome.bottom_h),
            (16.0, 40.0, 16.0)
        );
        // The 1 px authored overshoot, recorded in the module doc: the trees'
        // own `_BG` rects are one taller than the shell's interior.
        for (layout, authored_bg) in [
            (&ACTIVITY, (16.0, 39.0, 356.0, 314.0)),
            (&CONTRIBUTION, (16.0, 40.0, 356.0, 445.0)),
        ] {
            let interior = (
                chrome.side_w,
                chrome.top_h,
                layout.window.0 - 2.0 * chrome.side_w,
                layout.window.1 - chrome.top_h - chrome.bottom_h,
            );
            assert_eq!(interior.2, authored_bg.2, "same width");
            assert_eq!(interior.3 + 1.0, authored_bg.3, "one px taller, authored");
        }
    }

    /// The defect a real client start found (2026-08-23): an ungated `Update`
    /// system asking for `FontAssets` fails in the loading screen, where that
    /// resource does not exist yet, and takes the process with it. Four
    /// systems across two lanes died that way — and `cargo test`, `cargo build`
    /// and `make ci` never start the app, so nothing here can *observe* it.
    ///
    /// So the gate is asserted the way `hud/mod.rs` asserts its plugin-group
    /// arity: on the registration text itself. Both window systems of this
    /// tree must be registered behind the world-scene condition every other
    /// HUD window uses (`hud/community/mod.rs`), and both must clean up on
    /// leaving it.
    #[test]
    fn both_job_windows_are_gated_on_the_world_scene() {
        // The tree's own condition (`hud/mod.rs`: GameWorld | UiTesting |
        // Skills), not a hand-written copy of it.
        const GATE: &str = ".run_if(super::super::hud_scenes)";
        for (module, source) in [
            ("ranking.rs", include_str!("ranking.rs")),
            ("prev_info.rs", include_str!("prev_info.rs")),
        ] {
            let registration = source
                .split("#[cfg(test)]")
                .next()
                .expect("split always yields a first part");
            assert!(
                registration.contains(GATE),
                "{module} registers an Update system without the world-scene gate"
            );
            assert!(
                registration.contains("OnExit(SceneState::GameWorld)"),
                "{module} leaves its window up when the world scene ends"
            );
            assert!(
                !registration.contains("Option<Res<FontAssets>>"),
                "{module}: the Option pattern is the net side's answer, not a window's"
            );
        }
    }

    /// The receive path: a `0xB0E4` page reaches the resource the window reads,
    /// through the real deserializer.
    #[test]
    fn a_contribution_page_lands_in_the_resource_the_window_reads() {
        use crate::plugins::net::job::{on_job_ranking, JobRanking};
        use bytes::Bytes;
        use packets::agent::job::JobRankingResponse;

        let mut app = App::new();
        app.init_resource::<JobRanking>()
            .add_message::<JobRankingResponse>()
            .add_systems(Update, on_job_ranking);
        // result=1, job_type=1, rank_kind=1 (contribution), count=1,
        // rank=1, "Bob", level=4, points=0x0000007B — no trailing byte on this
        // branch, which is what makes the body end here.
        let body = Bytes::from_static(&[
            0x01, 0x01, 0x01, 0x01, 0x01, 0x03, 0x00, b'B', b'o', b'b', 0x04, 0x7B, 0x00, 0x00,
            0x00,
        ]);
        app.world_mut()
            .write_message(JobRankingResponse::try_from(body).unwrap());
        app.update();

        let page = app
            .world()
            .resource::<JobRanking>()
            .0
            .clone()
            .expect("the page landed");
        assert_eq!(page.rank_kind, JOB_RANK_KIND_CONTRIBUTION);
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].name, "Bob");
        assert_eq!(page.entries[0].points, 123);
        assert_eq!(page.entries[0].trailing, None);
        // …and the window that page selects is the 388x500 one.
        assert_eq!(layout_for(page.rank_kind).window, (388.0, 500.0));
    }
}
