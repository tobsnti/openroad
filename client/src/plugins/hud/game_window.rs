//! Reusable in-game window chrome: the vanilla mframe_wnd_* framed window.
//!
//! Idea: every draggable in-game window (inventory, and future ones like
//! storage, skills, party) shares the same shell — the 8-piece mframe_wnd_
//! frame ring with its built-in title band, a com_bg_tile_d interior, a
//! title text, a close (X) button and drag-to-move via the title strip.
//! `spawn_game_window` builds that shell for a given content size and hands
//! back the root (for the caller's marker/visibility management), the content
//! container (for the caller's actual UI) and the close button (for the
//! caller's close behavior via an `Activate` observer).
//!
//! Frame geometry (measured off the decoded textures): all pieces are fully
//! opaque; corners/sides are 40px wide, the top strip is 68px tall and
//! carries the integrated title band (dark bar between gold trim lines at
//! y 6..28, double-line separator at y 29..31), the bottom strip is 48px
//! tall. Past the frame's detailed edge the art is not flat black but a
//! dithered dark maroon/olive, so the content tucks into the ring and only
//! the `FRAME_VIS_*` margins remain visible, with the bg tiling overlaying
//! the ring's flat inner region out to those edges.
//!
//! Three things are caller-supplied through [`GameWindowStyle`] because the
//! data varies per window: the interior tile letter (11 distinct letters
//! corpus-wide), whether there is a caption at all (`GDR_MAINPOPUP` declares
//! an empty one) and whether there is a close button (the bag and equipment
//! trees declare none).

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::Button;

use crate::assets::FontAssets;
use crate::plugins::ui_v2::style::ImageButtonStyle;

/// One 8-piece window-frame family, taken from its own art.
///
/// Idea: the shell used to hardcode `mframe_wnd_` because "a family name
/// guarantees nothing about the pieces behind it" (see [`MFRAME_WND`] below).
/// That reasoning holds — what does not follow from it is that a second
/// family cannot exist. `msgbox2_window_` is the shell of 14 resinfo
/// trees (39 references) and its pieces are a
/// different size in every dimension, so the way to add it is to make the
/// numbers data and require every field to come from the art.
///
/// Every field below comes from the decoded `.ddj`s. Do not add a
/// family without doing the same.
pub struct ChromeFamily {
    /// Asset path prefix of the eight pieces.
    pub dir: &'static str,
    /// Width of the corner and side pieces (`*_left_side.ddj`).
    pub side_w: f32,
    /// Height of the top strip (`*_mid_up.ddj`).
    pub top_h: f32,
    /// Height of the bottom strip (`*_mid_down.ddj`).
    pub bottom_h: f32,
    /// Visible painted border on the left edge, from the art's column profile.
    pub vis_left: f32,
    /// Visible painted border on the right edge. Separate from `vis_left`
    /// because `msgbox2_window_`'s two side pieces are **not** mirror images:
    /// left paints 5 columns, right paints 7.
    pub vis_right: f32,
    /// Visible top edge = trim + title band + separator.
    pub vis_top: f32,
    /// Visible bottom edge, from the bottom strip's row profile.
    pub vis_bottom: f32,
    /// Title band inside the top strip: origin and height.
    pub title_y: f32,
    pub title_h: f32,
    /// Whether the bottom strip has an authored horizontal period and must
    /// tile instead of stretch (`msgbox2_window_mid_down` repeats every 16 px;
    /// `mframe_wnd_mid_down` has no period).
    pub tile_mid_down: bool,
}

/// The 8-piece family the in-game windows have always drawn.
///
/// It is deliberately **one** family per constant and not an
/// `mframe_{name}_` helper: a family name guarantees nothing about the pieces
/// behind it. `mframe_alc_` (4th-gen alchemy) ships seven **4x4 stubs** and
/// packs the entire 376x376 window plate into its `right_up` slot, so a
/// generic 9-slice helper would stretch a 4x4 across three window edges and
/// hide the real art in a corner (#476). Measure every piece before adding a
/// family here.
pub const MFRAME_WND: ChromeFamily = ChromeFamily {
    dir: "media://interface/frame/mframe_wnd_",
    side_w: 40.0,
    top_h: 68.0,
    bottom_h: 48.0,
    vis_left: 8.0,
    vis_right: 8.0,
    vis_top: 32.0,
    vis_bottom: 12.0,
    title_y: 6.0,
    title_h: 22.0,
    tile_mid_down: false,
};

/// `msgbox2_window_` — the shell of the job windows, the inventory, the guild
/// window, the letter, party matching, the item mall family and gacha.
///
/// `Media/interface/messagebox/msgbox2_window_*.ddj` carries
/// five distinct piece sizes — corners
/// 16x16 and 16x40, sides 16x64, mids 64x40 and 64x16 — all **A1R5G5B5**
/// (`pf.flags 0x41`), where `mframe_wnd_`'s mid/side pieces are opaque
/// R5G6B5. The corners really use that alpha: `left_up`/`right_up` carry a
/// 6-pixel transparent triangle, `left_down`/`right_down` a single corner
/// pixel — the window is rounded, and stretching a corner would smear it.
pub const MSGBOX2_WINDOW: ChromeFamily = ChromeFamily {
    dir: "media://interface/messagebox/msgbox2_window_",
    side_w: 16.0,
    top_h: 40.0,
    bottom_h: 16.0,
    // The interior inset is the **piece extent**, not where the paint stops:
    // 16 at the sides, 40 at the top, 16 at the bottom. Two authored rects
    // close on that independently — `GDR_PARTYMATCH_REGISTER` `0,0,314,373`
    // with `_MAIN_BG` `16,40,282,317`, and
    // `GDR_PREV_JOB_INFO` `0,0,364,164` with its `_BG` `16,40,332,108`
    // (`364 - 32 == 332`, `164 - 40 - 16 == 108`). Same numbers as
    // `hud/modal_dialog.rs`'s `MODAL_{SIDE,TOP,BOTTOM}`, which read this
    // family first for the dialog plates.
    //
    // (The *painted* trim is narrower — `left_side` paints 5 columns,
    // `right_side` 7, `mid_down` 7 rows — so the art's dither runs on under
    // the interior tile, exactly as it does in `mframe_wnd_`.)
    vis_left: 16.0,
    vis_right: 16.0,
    vis_top: 40.0,
    vis_bottom: 16.0,
    title_y: 7.0,
    title_h: 20.0,
    tile_mid_down: true,
};

/// One nine-piece **board** ring — the frame a window draws *inside* itself
/// around a content board, as opposed to [`ChromeFamily`], which is the
/// window's outer shell (title band, close button, drag strip).
///
/// Idea: the same kit was being re-declared per module. The literal
/// `"media://interface/inventory/int_window_"` stood at **nine** sites in
/// eight modules (`appearance_change`, `autopotion/ui`, `cos/setup`,
/// `cos/ui`, `inventory/ui`, `party_match/board`, `skill_window/ui`,
/// `stall/ui`) — up from four an iteration earlier — and each one carried its
/// own unsourced `16.0` next to it. The rects stay per-window (they are
/// authored resinfo numbers and differ), the *kit* is one thing.
pub struct BoardFrame {
    /// Asset path prefix of the nine pieces.
    pub dir: &'static str,
    /// Extent of the corner pieces, which is also the ring's border width:
    /// the side pieces are this wide and the mid strips this tall.
    pub piece: f32,
}

impl BoardFrame {
    /// Asset path of one piece of the kit — a ring piece (`left_up`,
    /// `mid_up`, …) or a sibling plate authored in the same prefix
    /// (`downbox`).
    pub fn piece_path(&self, name: &str) -> String {
        format!("{}{name}.ddj", self.dir)
    }
}

/// `int_window_` — the board ring of the inventory, skill, stall, COS,
/// autopotion, appearance-change and party-matching windows.
///
/// From the DDS headers of
/// `Media/interface/inventory/int_window_*.ddj` (`width`/`height` at `DDS_HEADER+16`
/// and `+12`, little-endian, the `.ddj`s carry a 20-byte `JMXVDDJ 1000`
/// prefix before the `DDS ` magic):
///
/// | piece | extent | pixel format |
/// |---|---|---|
/// | `left_up`, `mid_up`, `right_up` | 16x16 | A1R5G5B5 (`pf.flags 0x41`) |
/// | `left_side`, `right_side`, `left_down`, `right_down` | 16x16 | R5G6B5 (`0x40`) |
/// | `mid_down` | **24x16** | R5G6B5 (`0x40`) |
/// | `downbox` (sibling plate, not part of the ring) | 176x28 | R5G6B5 (`0x40`) |
///
/// So [`piece`](BoardFrame::piece) `= 16` is the art, not a guess: every
/// corner is 16x16, the sides are 16 wide and both mid strips are 16 tall.
/// `mid_down`'s 24 px width is not a layout input — the mids are stretched
/// across the span between the corners, as they have always been.
pub const INT_WINDOW: BoardFrame = BoardFrame {
    dir: "media://interface/inventory/int_window_",
    piece: 16.0,
};

/// Art extents of the default family's frame pieces. Kept as module constants
/// because callers and tests read them; the shell itself goes through
/// [`ChromeFamily`].
const CHROME_SIDE: f32 = MFRAME_WND.side_w;
const CHROME_TOP: f32 = MFRAME_WND.top_h;
const CHROME_BOTTOM: f32 = MFRAME_WND.bottom_h;
/// The title band inside the top strip (window units, from the art).
const TITLE_Y: f32 = MFRAME_WND.title_y;
const TITLE_H: f32 = MFRAME_WND.title_h;
/// Visible frame margins (see the module doc).
pub const FRAME_VIS_SIDE: f32 = MFRAME_WND.vis_left;
/// Top visible edge = the title band plus its separator lines.
pub const FRAME_VIS_TOP: f32 = MFRAME_WND.vis_top;
pub const FRAME_VIS_BOTTOM: f32 = MFRAME_WND.vis_bottom;
/// Padding between the visible frame edge and the content.
pub const CHROME_PAD: f32 = 4.0;
/// Content offset below the title band.
pub const CONTENT_TOP: f32 = FRAME_VIS_TOP + CHROME_PAD;

pub const BG_TILE_DDJ: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_d.ddj";

/// The per-window parts of the shell that the data does **not** hold constant.
///
/// Idea: `spawn_game_window` used to hardcode all three, so a caller had no
/// way to say otherwise — the bg tile letter is per-window data (11 distinct
/// letters corpus-wide), `GDR_MAINPOPUP` declares an empty caption, and the
/// bag/equipment trees declare no close button at all. The default is the
/// shell every current caller already draws, so this is a seam, not a
/// behaviour change.
///
/// **The letter is deliberately not a lookup table.** Only equipment's `_d`
/// is independently confirmed (unit doc §9-U5); every other window's letter
/// is UNKNOWN. A caller that moves off `_d` must cite the `DDJ=` line of its
/// own resinfo tree in a comment — inventing a letter here would be exactly
/// the kind of unbacked constant this shell exists to avoid.
pub struct GameWindowStyle {
    /// Which frame family to draw. Defaults to [`MFRAME_WND`], the
    /// one every existing window uses; the job/inventory/guild/letter trees
    /// declare `msgbox2_window_` ([`MSGBOX2_WINDOW`]) in their own `DDJ=` line.
    pub chrome: &'static ChromeFamily,
    /// Interior tile asset. Callers with no located resinfo tree keep the
    /// default.
    pub bg_tile: &'static str,
    /// Whether to spawn the (X). `false` yields
    /// [`GameWindow::close_button`] `== None`, so an unwired page cannot hold
    /// a dangling observer target.
    pub close_button: bool,
    /// Caption colour. Defaults to [`TITLE_COLOR`], the 29-of-31 majority of
    /// the corpus; the two windows whose `mframe_wnd_` block declares the pale
    /// violet ([`TITLE_COLOR_VIOLET`]) pass it here rather than each deriving
    /// a colour of their own.
    pub title_color: Color,
}

impl Default for GameWindowStyle {
    fn default() -> Self {
        Self {
            chrome: &MFRAME_WND,
            bg_tile: BG_TILE_DDJ,
            close_button: true,
            title_color: TITLE_COLOR,
        }
    }
}
/// Native extent of the close art in all three states — the DDS headers of
/// `com_windowclose{,_focus,_press}.ddj` all read 16x16, so drawing it at 14
/// rescaled it.
const CLOSE_SIZE: f32 = 16.0;
/// Caption colour. `mframe_wnd_` blocks declare `FontColor="255,255,255,255"`
/// in 29 of their 31 corpus occurrences (`ifsystemwnd.txt:148` plus 28 in
/// `ginterface.txt`). The two exceptions are `GDR_COMMUNITY`
/// (`ginterface.txt:546`) and `GDR_QUESTINFO` (`:872`), both
/// `FontColor="255,239,153,255"`. resinfo `COLOR` is **A,R,G,B** — see the
/// parser at `assets/resinfo/interface_text/mod.rs:83-91`, and the corpus
/// proves it (`"84,252,122,0"` is a sane alpha-84 orange under ARGB but
/// invisible green under RGBA) — so those two are RGB(239,153,255), a pale
/// violet, *not* gold; gold is the rotated `"255,255,239,153"` (25 uses).
const TITLE_COLOR: Color = Color::srgb_u8(255, 255, 255);
/// The corpus' two exceptions, `GDR_COMMUNITY` and `GDR_QUESTINFO`: the same
/// derivation as [`TITLE_COLOR`], one line further. It was already written out
/// above and then thrown away by a hardcoded white — a derived value that
/// nothing consumed. `GameWindowStyle::title_color` is what consumes it.
pub const TITLE_COLOR_VIOLET: Color = Color::srgb_u8(239, 153, 255);

const CLOSE_DDJ: &str = "media://interface/ifcommon/com_windowclose.ddj";
const CLOSE_FOCUS_DDJ: &str = "media://interface/ifcommon/com_windowclose_focus.ddj";
const CLOSE_PRESS_DDJ: &str = "media://interface/ifcommon/com_windowclose_press.ddj";

/// How one frame piece fills its rect.
///
/// The two side pieces carry a vertical motif whose rows repeat every 16 px
/// (byte-identical rows 16 apart in the decoded art), so stretching them to
/// an arbitrary window height distorts a period the artist authored. They
/// tile on Y instead — the same primitive the interior bg already uses. The
/// mid pieces stay stretched: their variation is low-amplitude dither with no
/// period to preserve, and the corners are fixed-size by construction.
fn edge_image_mode(family: &ChromeFamily, piece: &str, s: f32) -> NodeImageMode {
    match piece {
        "left_side" | "right_side" => NodeImageMode::Tiled {
            tile_x: false,
            tile_y: true,
            stretch_value: s,
        },
        // `msgbox2_window_mid_down` repeats every 16 px horizontally (checked
        // byte-identical column blocks), so stretching it across a 364-wide
        // window would smear an authored period; `mframe_wnd_`'s has none.
        "mid_down" if family.tile_mid_down => NodeImageMode::Tiled {
            tile_x: true,
            tile_y: false,
            stretch_value: s,
        },
        _ => NodeImageMode::Stretch,
    }
}

/// Outer window size (window units) wrapping a content area, in the default
/// family.
pub fn outer_size(content_size: (f32, f32)) -> (f32, f32) {
    outer_size_in(&MFRAME_WND, content_size)
}

/// [`outer_size`] for an arbitrary family.
pub fn outer_size_in(family: &ChromeFamily, content_size: (f32, f32)) -> (f32, f32) {
    (
        content_size.0 + family.vis_left + family.vis_right + 2.0 * CHROME_PAD,
        family.vis_top + CHROME_PAD + content_size.1 + CHROME_PAD + family.vis_bottom,
    )
}

/// Where a window's content sits inside its shell.
///
/// Idea: the shell used to *impose* this — outer size derived from the content
/// size through its own margins. That is right for windows whose resinfo row
/// only declares the content, and wrong for the ones that declare **both**
/// frames: `GDR_NPCWINDOW` is `386x451` with a nested `GDR_NW_NPCTALK` at
/// `11,48,364,391`, which the derived margins cannot express (they land on
/// `388x443`). So a caller with authored numbers passes them in and the shell
/// stops guessing; [`WindowGeometry::from_content`] keeps the old behaviour for
/// everyone else.
#[derive(Clone, Copy)]
pub struct WindowGeometry {
    /// Outer window extent in window units.
    pub outer: (f32, f32),
    /// Content origin inside the window, in window units.
    pub content_at: (f32, f32),
}

impl WindowGeometry {
    /// The shell's own margins around a content area, in the default family.
    pub fn from_content(content_size: (f32, f32)) -> Self {
        Self::from_content_in(&MFRAME_WND, content_size)
    }

    /// [`WindowGeometry::from_content`] for an arbitrary family.
    pub fn from_content_in(family: &ChromeFamily, content_size: (f32, f32)) -> Self {
        Self {
            outer: outer_size_in(family, content_size),
            content_at: (family.vis_left + CHROME_PAD, family.vis_top + CHROME_PAD),
        }
    }
}

/// A nested 9-slice frame drawn inside the shell, **under** the content — the
/// second frame windows like the NPC conversation declare in their own
/// resinfo file.
#[derive(Clone, Copy)]
pub struct InnerFrame {
    /// Asset path prefix of the 8 pieces, e.g. `media://…/npc_conversation_window_`.
    pub dir: &'static str,
    /// Frame rect in window units.
    pub rect: (f32, f32, f32, f32),
    /// Corner extent of the pieces (square corners).
    pub corner: f32,
}

/// The entities a spawned window shell hands back to its owner.
pub struct GameWindow {
    /// Whole-window node (right/top-anchored). Insert your marker, z-index
    /// and visibility management here.
    pub root: Entity,
    /// Empty container at the content origin — put the window's UI in here.
    pub content: Entity,
    /// The (X) button on the title band; attach an `Activate` observer.
    /// `None` when the shell was spawned with
    /// [`GameWindowStyle::close_button`] `false` — the bag and equipment
    /// trees declare no close button.
    pub close_button: Option<Entity>,
}

impl GameWindow {
    /// The (X) of a shell that was spawned with one.
    ///
    /// For the callers that ask for the default chrome and then wire their
    /// close behaviour: the `None` case is unreachable for them, and making
    /// each of them re-state that would bury the two windows where `None` is
    /// the point.
    pub fn expect_close_button(&self) -> Entity {
        self.close_button
            .expect("window was spawned without a close button")
    }
}

/// The window a title-bar drag moves, stored on the title bar.
#[derive(Component)]
struct ChromeDragTarget(Entity);

/// Drag bookkeeping on the title bar: the root's anchor at drag start.
#[derive(Component)]
struct ChromeDragStart {
    right: f32,
    top: f32,
}

/// The design-space anchor a window was spawned with, kept on the root so the
/// shell can put the window back on screen if it does not fit.
///
/// HUD anchors are transcribed from the original's 1024x768 layout while the
/// window's *size* is multiplied by `hud_scale` — so on a 1600x900 screen at
/// scale 1.5 a legitimately transcribed window can hang off the bottom edge
/// (the teleport board: 556 units tall becomes 834 px, anchored at top 120,
/// which puts its pager at y=954 in a 900 px window — reported 2026-08-17 as
/// "sieht komplett falsch aus"). Rather than hand-tuning 21 anchor constants
/// away from the values the data gives, the shell fits the window into the
/// viewport once, and only while the player has not placed it themselves.
#[derive(Component, Clone, Copy)]
pub struct WindowAnchor {
    pub right: f32,
    pub top: f32,
}

/// Marks a window the player has actually dragged this session.
///
/// Position persistence keys off this, and not off "a window is open and the
/// mouse was released": without the distinction every left-click wrote every
/// open window's *spawn default* into `user_settings.yaml` as if the player
/// had placed it there (observed 2026-08-17 — `MainPopup [24.0, 60.0]` is
/// `inventory/ui.rs`'s own anchor). A stored default is not harmless: it
/// outranks the anchor forever after, including the clamp that a smaller
/// viewport applied once.
#[derive(Component)]
pub struct WindowDragged;

/// Build the framed-window shell. The root is anchored by its right/top
/// corner (`anchor_right_top`, physical px) — the drag observers rely on
/// that anchoring.
#[allow(clippy::too_many_arguments)]
pub fn spawn_game_window(
    commands: &mut Commands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    camera: Entity,
    title: &str,
    content_size: (f32, f32),
    anchor_right_top: (f32, f32),
    scale: f32,
) -> GameWindow {
    spawn_game_window_styled(
        commands,
        asset_server,
        fonts,
        camera,
        title,
        content_size,
        anchor_right_top,
        scale,
        GameWindowStyle::default(),
    )
}

/// [`spawn_game_window`] with the per-window parts spelled out. See
/// [`GameWindowStyle`].
#[allow(clippy::too_many_arguments)]
pub fn spawn_game_window_styled(
    commands: &mut Commands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    camera: Entity,
    title: &str,
    content_size: (f32, f32),
    anchor_right_top: (f32, f32),
    scale: f32,
    style: GameWindowStyle,
) -> GameWindow {
    spawn_game_window_with(
        commands,
        asset_server,
        fonts,
        camera,
        title,
        WindowGeometry::from_content(content_size),
        None,
        anchor_right_top,
        scale,
        style,
    )
}

/// [`spawn_game_window_styled`] with the geometry supplied instead of derived,
/// plus the optional nested frame drawn under the content. This is the full
/// shell; the other two are the common defaults of it.
#[allow(clippy::too_many_arguments)]
pub fn spawn_game_window_with(
    commands: &mut Commands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    camera: Entity,
    title: &str,
    geometry: WindowGeometry,
    inner_frame: Option<InnerFrame>,
    anchor_right_top: (f32, f32),
    scale: f32,
    style: GameWindowStyle,
) -> GameWindow {
    let s = scale;
    let (window_w, window_h) = geometry.outer;
    let family = style.chrome;
    let piece = |name: &str| asset_server.load::<Image>(format!("{}{name}.ddj", family.dir));

    let root = commands
        .spawn((
            Name::from(format!("{title} Window")),
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(anchor_right_top.0),
                top: Val::Px(anchor_right_top.1),
                width: Val::Px(window_w * s),
                height: Val::Px(window_h * s),
                ..default()
            },
            WindowAnchor {
                right: anchor_right_top.0,
                top: anchor_right_top.1,
            },
            UiTargetCamera(camera),
        ))
        .id();

    let mut content = None;
    let mut close_button = None;
    commands.entity(root).with_children(|window| {
        // the 8-piece frame ring: fixed corners, stretched mids and sides,
        // drawn at the art's full extents (which reach under the content).
        // The later-drawn mids/sides overlap the earlier pieces by 1 unit —
        // butt-joined rects can land on fractional pixels at scaled/dragged
        // positions and open a 1px antialiasing seam at the joins; the art
        // is opaque and pattern-continuous there, so the overlap is invisible.
        let sw = family.side_w;
        let th = family.top_h;
        let bh = family.bottom_h;
        let edges = [
            // (rect, piece)
            ((0.0, 0.0, sw, th), "left_up"),
            ((sw - 1.0, 0.0, window_w - 2.0 * sw + 2.0, th), "mid_up"),
            ((window_w - sw, 0.0, sw, th), "right_up"),
            ((0.0, th - 1.0, sw, window_h - th - bh + 2.0), "left_side"),
            (
                (window_w - sw, th - 1.0, sw, window_h - th - bh + 2.0),
                "right_side",
            ),
            ((0.0, window_h - bh, sw, bh), "left_down"),
            (
                (sw - 1.0, window_h - bh, window_w - 2.0 * sw + 2.0, bh),
                "mid_down",
            ),
            ((window_w - sw, window_h - bh, sw, bh), "right_down"),
        ];
        for (rect, name) in edges {
            window.spawn((
                abs_node(rect, s),
                ImageNode {
                    image: piece(name),
                    image_mode: edge_image_mode(family, name, s),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }

        // interior bg tiling on top of the ring's flat black, out to the
        // visible frame edges
        window.spawn((
            abs_node(
                (
                    family.vis_left,
                    family.vis_top,
                    window_w - family.vis_left - family.vis_right,
                    window_h - family.vis_top - family.vis_bottom,
                ),
                s,
            ),
            ImageNode {
                image: asset_server.load(style.bg_tile),
                image_mode: NodeImageMode::Tiled {
                    tile_x: true,
                    tile_y: true,
                    stretch_value: s,
                },
                ..default()
            },
            Pickable::IGNORE,
        ));

        // the title band is part of the top strip's art; this node carries
        // the text, the close button and the drag observers
        window
            .spawn((
                abs_node((0.0, family.title_y, window_w, family.title_h), s),
                Name::from(format!("{title} Title Bar")),
                ChromeDragTarget(root),
            ))
            .observe(on_chrome_drag_start)
            .observe(on_chrome_drag)
            .with_children(|bar| {
                // an empty caption is real data, not a missing string:
                // `GDR_MAINPOPUP` declares one, so it gets no text node
                if !title.is_empty() {
                    bar.spawn((
                        Text::new(title.to_string()),
                        TextFont {
                            font: fonts.two.clone().into(),
                            font_size: FontSize::Px(8.5 * s),
                            ..default()
                        },
                        TextColor(style.title_color),
                        TextLayout::justify(Justify::Center),
                        Node {
                            position_type: PositionType::Absolute,
                            top: Val::Px(5.0 * s),
                            width: Val::Percent(100.0),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                }
                if style.close_button {
                    let close_style = ImageButtonStyle {
                        normal: asset_server.load(CLOSE_DDJ),
                        hover: asset_server.load(CLOSE_FOCUS_DDJ),
                        press: asset_server.load(CLOSE_PRESS_DDJ),
                        ..Default::default()
                    };
                    close_button = Some(
                        bar.spawn((
                            Button,
                            Hovered::default(),
                            close_button_node(s),
                            ImageNode {
                                image: close_style.normal.clone(),
                                image_mode: NodeImageMode::Stretch,
                                ..default()
                            },
                            close_style,
                        ))
                        .id(),
                    );
                }
            });

        // the nested frame, if the window declares one — under the content
        if let Some(frame) = inner_frame {
            let (fx, fy, fw, fh) = frame.rect;
            let c = frame.corner;
            let inner = |name: &str| asset_server.load::<Image>(format!("{}{name}.ddj", frame.dir));
            // same 1-unit overlap at the joins as the outer ring, for the same
            // antialiasing reason
            let ring = [
                ((fx, fy, c, c), "left_up"),
                ((fx + c - 1.0, fy, fw - 2.0 * c + 2.0, c), "mid_up"),
                ((fx + fw - c, fy, c, c), "right_up"),
                ((fx, fy + c - 1.0, c, fh - 2.0 * c + 2.0), "left_side"),
                (
                    (fx + fw - c, fy + c - 1.0, c, fh - 2.0 * c + 2.0),
                    "right_side",
                ),
                ((fx, fy + fh - c, c, c), "left_down"),
                (
                    (fx + c - 1.0, fy + fh - c, fw - 2.0 * c + 2.0, c),
                    "mid_down",
                ),
                ((fx + fw - c, fy + fh - c, c, c), "right_down"),
            ];
            for (rect, name) in ring {
                window.spawn((
                    abs_node(rect, s),
                    ImageNode {
                        image: inner(name),
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            }
        }

        // empty content container for the caller's UI
        content = Some(
            window
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(geometry.content_at.0 * s),
                        top: Val::Px(geometry.content_at.1 * s),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .id(),
        );
    });

    // Opt the window into the Escape chain, here rather than at each of the
    // two dozen call sites.
    //
    // The (X) *is* the closer (`hud::focus`), so a window that has one can be
    // closed by Escape and a window that declares none — the bag and equipment
    // trees — cannot. That is the correct rule and it is already expressed by
    // the data, so deriving it here means no caller can forget to opt in, and
    // Escape can never do something different from clicking the button.
    if let Some(close_button) = close_button {
        commands
            .entity(root)
            .insert(crate::plugins::hud::focus::HudWindow { close_button });
    }

    GameWindow {
        root,
        content: content.expect("content spawned"),
        close_button,
    }
}

/// `(x, y, w, h)` scaled uniformly — the resinfo-rect-to-pixels step shared by
/// all HUD windows.
pub fn scaled(rect: (f32, f32, f32, f32), s: f32) -> (f32, f32, f32, f32) {
    (rect.0 * s, rect.1 * s, rect.2 * s, rect.3 * s)
}

/// Absolutely positioned node from a `(x, y, w, h)` resinfo rect at scale `s`.
pub fn abs_node(rect: (f32, f32, f32, f32), s: f32) -> Node {
    let (x, y, w, h) = scaled(rect, s);
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(x),
        top: Val::Px(y),
        width: Val::Px(w),
        height: Val::Px(h),
        ..default()
    }
}

/// The close (X) button's box, right-anchored in the title band. Split out of
/// the spawn so the size can be asserted without running the app.
fn close_button_node(s: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        right: Val::Px(10.0 * s),
        top: Val::Px(4.0 * s),
        width: Val::Px(CLOSE_SIZE * s),
        height: Val::Px(CLOSE_SIZE * s),
        ..default()
    }
}

/// Start a window drag from the title bar: remember the root's anchor.
fn on_chrome_drag_start(
    drag: On<Pointer<DragStart>>,
    targets: Query<&ChromeDragTarget>,
    roots: Query<&Node>,
    mut commands: Commands,
) {
    let Ok(target) = targets.get(drag.entity) else {
        return;
    };
    let Ok(node) = roots.get(target.0) else {
        return;
    };
    let (Val::Px(right), Val::Px(top)) = (node.right, node.top) else {
        return;
    };
    commands
        .entity(drag.entity)
        .insert(ChromeDragStart { right, top });
}

/// Move the window with the pointer (the root is right/top-anchored).
fn on_chrome_drag(
    drag: On<Pointer<Drag>>,
    starts: Query<(&ChromeDragStart, &ChromeDragTarget)>,
    mut roots: Query<&mut Node>,
    mut commands: Commands,
) {
    let Ok((start, target)) = starts.get(drag.entity) else {
        return;
    };
    let Ok(mut node) = roots.get_mut(target.0) else {
        return;
    };
    node.right = Val::Px(start.right - drag.event.distance.x);
    node.top = Val::Px(start.top + drag.event.distance.y);
    // Only a window that actually moved may be persisted (see `WindowDragged`).
    commands.entity(target.0).insert(WindowDragged);
}

#[cfg(test)]
mod test {
    use super::*;

    /// The close art is 16x16 in every state (DDS headers of
    /// `interface/ifcommon/com_windowclose{,_focus,_press}.ddj`), so the button
    /// must carry the art's own extent through the HUD scale rather than a
    /// hand-picked size — 14 downscaled it by 12.5% at every consumer.
    #[test]
    fn close_button_carries_its_art_extent_through_the_scale() {
        assert_eq!(CLOSE_SIZE, 16.0);
        let node = close_button_node(1.5);
        assert_eq!(node.width, Val::Px(24.0));
        assert_eq!(node.height, Val::Px(24.0));
    }

    /// `FontColor="255,255,255,255"` on 29 of the 31 `mframe_wnd_` blocks —
    /// and the two exceptions are not a rounding error to be ignored but a
    /// per-window value, which is why the style carries it. The default has to
    /// stay the majority colour; a window that wants the violet says so.
    #[test]
    fn the_caption_colour_defaults_to_the_corpus_majority_and_the_exception_is_reachable() {
        assert_eq!(TITLE_COLOR.to_srgba(), Srgba::WHITE);
        assert_eq!(
            GameWindowStyle::default().title_color.to_srgba(),
            Srgba::WHITE
        );
        // `ginterface.txt:546/:872` — ARGB "255,239,153,255", so RGB is the
        // pale violet, not the rotated gold (which the corpus uses 25 times).
        assert_eq!(
            TITLE_COLOR_VIOLET.to_srgba(),
            Srgba::rgb_u8(239, 153, 255),
            "the derived exception must be the violet, not gold"
        );
    }

    /// `left_side`/`right_side` carry a vertical motif that repeats every
    /// 16 px, so stretching them to an arbitrary window height distorts a
    /// period the art authored. The mids keep `Stretch` — their variation is
    /// low-amplitude dither with no period — and the corners are fixed size.
    #[test]
    fn side_pieces_tile_over_their_sixteen_px_motif() {
        for piece in ["left_side", "right_side"] {
            assert!(
                matches!(
                    edge_image_mode(&MFRAME_WND, piece, 1.5),
                    NodeImageMode::Tiled {
                        tile_x: false,
                        tile_y: true,
                        stretch_value,
                    } if stretch_value == 1.5
                ),
                "{piece} must tile on Y at the HUD scale"
            );
        }
        for piece in [
            "mid_up",
            "mid_down",
            "left_up",
            "right_up",
            "left_down",
            "right_down",
        ] {
            assert!(
                matches!(
                    edge_image_mode(&MFRAME_WND, piece, 1.5),
                    NodeImageMode::Stretch
                ),
                "{piece} must keep stretching"
            );
        }
    }

    /// The second family, off
    /// `Media/interface/messagebox/msgbox2_window_*.ddj`. These are the numbers a window built on
    /// this shell inherits, so they are asserted rather than trusted.
    #[test]
    fn the_msgbox2_family_carries_its_own_piece_sizes() {
        let m = &MSGBOX2_WINDOW;
        // corners 16x16 / 16x40, sides 16x64, mids 64x40 and 64x16
        assert_eq!((m.side_w, m.top_h, m.bottom_h), (16.0, 40.0, 16.0));
        // `mid_up` row profile: band rows 7..26 inclusive = 20 rows, inside
        // the 40-tall top strip
        assert_eq!((m.title_y, m.title_h), (7.0, 20.0));
        assert!(m.title_y + m.title_h <= m.top_h);
        // The interior inset is the piece extent, and two authored rects close
        // on it: 314 - 2*16 == 282 (ifpartymatch) and 364 - 2*16 == 332,
        // 164 - 40 - 16 == 108 (ifprevjobinfo).
        assert_eq!((m.vis_left, m.vis_top, m.vis_bottom), (16.0, 40.0, 16.0));
        assert_eq!(364.0 - m.vis_left - m.vis_right, 332.0);
        assert_eq!(164.0 - m.vis_top - m.vis_bottom, 108.0);
        // and it agrees with the module that read this family first
        assert_eq!(m.vis_left, crate::plugins::hud::modal_dialog::MODAL_SIDE);
        assert_eq!(m.vis_top, crate::plugins::hud::modal_dialog::MODAL_TOP);
        assert_eq!(
            m.vis_bottom,
            crate::plugins::hud::modal_dialog::MODAL_BOTTOM
        );
        // and it is a smaller shell than mframe_wnd_ in every dimension
        assert!(m.side_w < MFRAME_WND.side_w);
        assert!(m.top_h < MFRAME_WND.top_h);
        assert!(m.bottom_h < MFRAME_WND.bottom_h);
    }

    /// `msgbox2_window_mid_down` repeats every 16 px horizontally (byte-identical
    /// column blocks in the decoded art), so it tiles where `mframe_wnd_`'s
    /// stretches.
    #[test]
    fn the_msgbox2_bottom_strip_tiles_where_mframes_stretches() {
        assert!(matches!(
            edge_image_mode(&MSGBOX2_WINDOW, "mid_down", 1.0),
            NodeImageMode::Tiled { tile_x: true, .. }
        ));
        assert!(matches!(
            edge_image_mode(&MFRAME_WND, "mid_down", 1.0),
            NodeImageMode::Stretch
        ));
        // the top strip has no period in either family
        assert!(matches!(
            edge_image_mode(&MSGBOX2_WINDOW, "mid_up", 1.0),
            NodeImageMode::Stretch
        ));
    }

    /// Escape closes a window by pressing its own (X), and the shell is what
    /// wires that up — so a window with a close button is Escape-closable and
    /// one that declares none is not.
    ///
    /// Pinned at the source level because the alternative is two dozen call
    /// sites each remembering to opt in, which is exactly the per-window
    /// bookkeeping this shell exists to remove. The rule is derived from the
    /// data (`GameWindowStyle::close_button`), not chosen per window.
    #[test]
    fn the_shell_opts_its_window_into_the_escape_chain() {
        let src = include_str!("game_window.rs");
        let build = src
            .split("fn spawn_game_window_with")
            .nth(1)
            .expect("the full shell builder moved");
        assert!(
            build.contains("focus::HudWindow"),
            "the shell no longer marks its root as Escape-closable"
        );
        // ...and only when there is a button to press
        let marker_at = build.find("focus::HudWindow").expect("checked above");
        let guard_at = build
            .find("if let Some(close_button) = close_button")
            .expect("the close-button guard around the opt-in is gone");
        assert!(
            guard_at < marker_at,
            "the opt-in must stay inside the close-button guard: a window with \
             no (X) has nothing for Escape to activate"
        );
    }

    /// The style seam must not change what any existing caller draws: the
    /// default is still `com_bg_tile_d` plus an (X). Only equipment's `_d` is
    /// independently confirmed (unit doc §9-U5), so callers without a located
    /// resinfo tree keep it rather than guessing a letter.
    #[test]
    fn default_style_is_the_shell_every_current_caller_draws() {
        let style = GameWindowStyle::default();
        assert_eq!(style.bg_tile, BG_TILE_DDJ);
        assert!(style.bg_tile.ends_with("com_bg_tile_d.ddj"));
        assert!(style.close_button);
    }

    /// The board kit is the *inner* ring, not a third window shell: it must
    /// keep the path and the 16 px border every one of its eight callers used
    /// before they were pointed here, or this refactor moved pixels. Both
    /// values are stated in [`INT_WINDOW`]'s doc comment (DDS headers of
    /// `int_window_*.ddj`: corners 16x16, sides 16 wide, mids 16 tall).
    #[test]
    fn the_int_window_board_kit_is_the_authored_one() {
        assert_eq!(INT_WINDOW.dir, "media://interface/inventory/int_window_");
        assert_eq!(INT_WINDOW.piece, 16.0);
        // it is a board ring, not a window shell — neither chrome family
        // shares its art
        assert_ne!(INT_WINDOW.dir, MFRAME_WND.dir);
        assert_ne!(INT_WINDOW.dir, MSGBOX2_WINDOW.dir);
        // the eight ring pieces and the `downbox` plate authored beside them
        assert_eq!(
            INT_WINDOW.piece_path("left_up"),
            "media://interface/inventory/int_window_left_up.ddj"
        );
        assert_eq!(
            INT_WINDOW.piece_path("downbox"),
            "media://interface/inventory/int_window_downbox.ddj"
        );
    }

    /// The reason [`INT_WINDOW`] exists. The literal
    /// `"media://interface/inventory/int_window_"` had grown to **nine** sites
    /// in eight modules (from four an iteration earlier), each with its own
    /// `16.0` beside it, so the next window to draw this ring copied a number
    /// with no source. The kit lives here; a module keeps only its own
    /// authored rect. Scanned rather than trusted, the same way
    /// `hud/scale.rs::no_module_declares_its_own_hud_scale_constant` scans for
    /// re-declared HUD scales.
    ///
    /// The needle carries its opening quote, so prose and doc comments that
    /// merely name `int_window_` (`small_popup.rs`, `collection.rs`,
    /// `stall_network.rs`, …) are untouched — only an actual asset path is an
    /// offence.
    #[test]
    fn only_the_board_kit_declares_the_int_window_asset_path() {
        const NEEDLE: &str = "\"media://interface/inventory/int_window_";
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                // This file is where the one kit is allowed to live.
                if path.file_name().and_then(|f| f.to_str()) == Some("game_window.rs") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for (line, text) in text.lines().enumerate() {
                    if text.contains(NEEDLE) {
                        offenders.push(format!("{}:{}", path.display(), line + 1));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "these sites re-declare the `int_window_` board kit instead of \
             using `game_window::INT_WINDOW`, so its piece extent is a copied \
             number again: {offenders:?}"
        );
    }
}
