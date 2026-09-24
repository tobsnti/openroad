//! COS (companion) window shell + its three-page host.
//!
//! Idea: this is step 1 of `docs/re/ui/cos-pet-window.md` §8 — the vanilla
//! shell and the frame that hosts its pages, with the pages themselves left as
//! empty containers for the follow-up work. Geometry is transcribed from the
//! user's own resinfo: the shell is `GDR_COS_WND:CIFCOS`
//! (`resinfo/ginterface.txt:1330`, id 120, `Rect="0,0,355,390"`,
//! `DDJ="interface\frame\mframe_wnd_"`, `Text=""` — a blank-caption
//! `mframe_wnd_` window, as `docs/re/ui/hud-game-window-chrome.md:62` lists
//! it), so it rides on the shared `game_window` chrome. Inside it,
//! `GDR_COS_IFRAME:CIFFrame` (`resinfo/ifcos.txt`, `int_window_` art) and all
//! three page controls share the single rect `12,66,331,314`; those window
//! rects are rebased into the shell's content space by subtracting its content
//! origin `(FRAME_VIS_SIDE + CHROME_PAD, CONTENT_TOP)` = `(12, 36)`, the same
//! way `character_info` does (never re-derive that origin from vanilla's
//! interior art, see #310).
//!
//! **The tab strip.** `ifcos.txt` declares no tab control,
//! which for a long time read as "vanilla has no tabs here" — and left the
//! fully built setup page (`setup.rs`, `0x7420`/`0xB420`) with no opener at
//! all: `CosPage::Setup` was never written anywhere in the tree. The strip is
//! not invented, it is *named by the game data*:
//! `textdata/textuisystem.txt` ships `UIIT_STT_COSNEWUI_TABMENU_BASICINFO` =
//! "Basic Info" and `UIIT_STT_COSNEWUI_TABMENU_TECHNOLOGY` = "Setting" — a
//! two-entry tab menu for exactly this window, and exactly two of its pages.
//! Like the command bar, the original assembles it in code, so no resinfo block
//! carries its rect (positive control for that search: the same file *does*
//! declare `GDR_COS_IFRAME` and all three page controls). Deliberate deviation
//! (ADR-0009), stated here rather than hidden in a constant:
//!   * the art is the shared `ifcommon/com_long_tab_{on,off}.ddj` (72x24),
//!     because `com_tab_*` is 60x24 and the authored caption "Basic Info" does
//!     not fit it; both belong to the same `ifcommon` tab family;
//!   * the placement (strip sitting on the page frame's top edge, left-aligned
//!     with the frame) is ours — vanilla's is unknown for want of a rect;
//!   * there are **two** tabs, not three: vanilla names two, and the cargo page
//!     keeps the opener it already has (the `UIIT_STT_COS_INVENTORY` command
//!     cell). While the cargo page shows, neither tab is lit — that state is
//!     pinned by a test rather than papered over with an invented third tab.
//!
//! The window's default position is unknown — `wndpos.dat` window[5] persists
//! it per user and openroad has no parser for that file yet, so the spawn
//! anchor below is ours, not vanilla's.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button};

use crate::assets::FontAssets;
use crate::plugins::hud::cos::info;
use crate::plugins::hud::cos::inventory;
use crate::plugins::hud::cos::model::{CosPage, CosWindowState};
use crate::plugins::hud::cos::setup;
use crate::plugins::hud::game_window::{self, abs_node};
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::textdata::ClientUiStrings;

// --- Layout constants (resinfo, window units) -------------------------------

/// Vanilla shell rect (`ginterface.txt:1330` `Rect="0,0,355,390"`).
const WINDOW_SIZE: (f32, f32) = (355.0, 390.0);
/// Content box derived from that rect, so the chrome's outer box *is* the
/// vanilla shell: the frame's visible margins and pad are subtracted rather
/// than a content size being picked (331x338 today).
const CONTENT_W: f32 =
    WINDOW_SIZE.0 - 2.0 * (game_window::FRAME_VIS_SIDE + game_window::CHROME_PAD);
const CONTENT_H: f32 = WINDOW_SIZE.1
    - game_window::CONTENT_TOP
    - game_window::CHROME_PAD
    - game_window::FRAME_VIS_BOTTOM;

/// The rect shared by `GDR_COS_IFRAME` and all three page controls, in vanilla
/// window space (`ifcos.txt`).
const PAGE_RECT_WINDOW: (f32, f32, f32, f32) = (12.0, 66.0, 331.0, 314.0);
/// The shared `int_window_` board kit ([`game_window::INT_WINDOW`]).
const FRAME_PIECE: f32 = game_window::INT_WINDOW.piece;
const FRAME_DIR: &str = game_window::INT_WINDOW.dir;

/// Tab strip art, `interface/ifcommon/com_long_tab_{on,off}.ddj` — 72x24 (see
/// the module note for why the long variant).
const TAB_ON_DDJ: &str = "media://interface/ifcommon/com_long_tab_on.ddj";
const TAB_OFF_DDJ: &str = "media://interface/ifcommon/com_long_tab_off.ddj";
const TAB_SIZE: (f32, f32) = (72.0, 24.0);
/// Gap between the two tabs, and the strip's inset from the page frame's left
/// edge — the same 2px/8px the inventory strip uses (`hud/inventory/ui.rs`),
/// so the two strips in this HUD look like one family. Ours, not vanilla's.
const TAB_GAP: f32 = 2.0;
const TAB_INSET: f32 = 8.0;
/// Caption colour of the tab labels: the `FontColor` every other caption in
/// this window carries (`ifcosinfo.txt`, `255,255,245,218`).
const TAB_TEXT: Color = Color::srgb_u8(255, 245, 218);

/// The two tabs vanilla names, in the order its string ids run.
const TABS: [(CosPage, &str, &str); 2] = [
    (
        CosPage::Info,
        "UIIT_STT_COSNEWUI_TABMENU_BASICINFO",
        "Basic Info",
    ),
    (
        CosPage::Setup,
        "UIIT_STT_COSNEWUI_TABMENU_TECHNOLOGY",
        "Setting",
    ),
];

/// Spawn anchor (right/top, physical px). **Ours**, not vanilla's — see the
/// module note on `wndpos.dat`.
const WINDOW_RIGHT: f32 = 360.0;
const WINDOW_TOP: f32 = 80.0;

/// `PAGE_RECT_WINDOW` rebased into the shell's content space.
fn page_rect() -> (f32, f32, f32, f32) {
    let (x, y, w, h) = PAGE_RECT_WINDOW;
    (
        x - (game_window::FRAME_VIS_SIDE + game_window::CHROME_PAD),
        y - game_window::CONTENT_TOP,
        w,
        h,
    )
}

/// Whether `page` is the one the shell currently shows.
fn page_display(selected: CosPage, page: CosPage) -> Display {
    if selected == page {
        Display::Flex
    } else {
        Display::None
    }
}

// --- Markers ----------------------------------------------------------------

#[derive(Component)]
pub struct CosWindowRoot;

/// One of the three page containers; the follow-up pages fill these.
#[derive(Component)]
pub struct CosPageRoot(pub CosPage);

/// A tab of the strip; carries the page it selects.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct CosTabButton(pub CosPage);

/// The tab strip's y in the shell's content space: it sits directly on top of
/// the page frame, so its bottom edge is the frame's top edge.
fn tab_strip_y() -> f32 {
    page_rect().1 - TAB_SIZE.1
}

/// The x of tab `index` in the shell's content space.
fn tab_x(index: usize) -> f32 {
    page_rect().0 + TAB_INSET + index as f32 * (TAB_SIZE.0 + TAB_GAP)
}

// --- Spawning ---------------------------------------------------------------

/// Spawn the (initially hidden) COS window.
pub fn spawn_cos_window(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    state: Res<CosWindowState>,
    cam_query: Query<Entity, With<Camera2d>>,
) {
    let Ok(camera) = cam_query.single() else {
        warn!("cos window: no 2d camera to attach to");
        return;
    };
    let s = hud_scale();

    // vanilla's caption is empty (`Text=STRING,""`), so the title band stays blank
    let window = game_window::spawn_game_window(
        &mut commands,
        &asset_server,
        &fonts,
        camera,
        "",
        (CONTENT_W, CONTENT_H),
        (WINDOW_RIGHT, WINDOW_TOP),
        s,
    );
    commands
        .entity(window.root)
        .insert((CosWindowRoot, Name::from("COS Window"), GlobalZIndex(55)))
        .entry::<Node>()
        .and_modify(|mut node| node.display = Display::None);
    commands
        .entity(window.expect_close_button())
        .observe(on_close_button);

    let (px, py, pw, ph) = page_rect();
    commands.entity(window.content).with_children(|content| {
        // GDR_COS_IFRAME: the int_window_ ring around the page area
        let pieces = [
            ((0.0, 0.0, FRAME_PIECE, FRAME_PIECE), "left_up"),
            (
                (
                    FRAME_PIECE - 1.0,
                    0.0,
                    pw - 2.0 * FRAME_PIECE + 2.0,
                    FRAME_PIECE,
                ),
                "mid_up",
            ),
            (
                (pw - FRAME_PIECE, 0.0, FRAME_PIECE, FRAME_PIECE),
                "right_up",
            ),
            (
                (
                    0.0,
                    FRAME_PIECE - 1.0,
                    FRAME_PIECE,
                    ph - 2.0 * FRAME_PIECE + 2.0,
                ),
                "left_side",
            ),
            (
                (
                    pw - FRAME_PIECE,
                    FRAME_PIECE - 1.0,
                    FRAME_PIECE,
                    ph - 2.0 * FRAME_PIECE + 2.0,
                ),
                "right_side",
            ),
            (
                (0.0, ph - FRAME_PIECE, FRAME_PIECE, FRAME_PIECE),
                "left_down",
            ),
            (
                (
                    FRAME_PIECE - 1.0,
                    ph - FRAME_PIECE,
                    pw - 2.0 * FRAME_PIECE + 2.0,
                    FRAME_PIECE,
                ),
                "mid_down",
            ),
            (
                (pw - FRAME_PIECE, ph - FRAME_PIECE, FRAME_PIECE, FRAME_PIECE),
                "right_down",
            ),
        ];
        for ((x, y, w, h), piece) in pieces {
            content.spawn((
                abs_node((px + x, py + y, w, h), s),
                ImageNode {
                    image: asset_server.load(format!("{FRAME_DIR}{piece}.ddj")),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }

        // the tab strip vanilla names in textdata (see the module note) — the
        // opener the setup page never had
        for (index, (page, key, fallback)) in TABS.iter().enumerate() {
            content
                .spawn((
                    CosTabButton(*page),
                    Button,
                    Hovered::default(),
                    abs_node((tab_x(index), tab_strip_y(), TAB_SIZE.0, TAB_SIZE.1), s),
                    ImageNode {
                        image: asset_server.load(if state.page == *page {
                            TAB_ON_DDJ
                        } else {
                            TAB_OFF_DDJ
                        }),
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                ))
                .observe(on_tab_button)
                .with_children(|tab| {
                    tab.spawn((
                        Text::new(ui_strings.get_or(key, fallback).to_string()),
                        TextFont {
                            font: fonts.two.clone().into(),
                            font_size: FontSize::Px(8.0 * s),
                            ..default()
                        },
                        TextColor(TAB_TEXT),
                        TextLayout::justify(Justify::Center),
                        Node {
                            width: Val::Percent(100.0),
                            top: Val::Px(6.0 * s),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                });
        }

        // the three page containers, all on the one shared rect
        for page in [CosPage::Info, CosPage::Inventory, CosPage::Setup] {
            let mut node = abs_node((px, py, pw, ph), s);
            node.display = page_display(state.page, page);
            let mut container = content.spawn((CosPageRoot(page), node, Pickable::IGNORE));
            match page {
                CosPage::Info => {
                    container.with_children(|info_page| {
                        info::build_info_page(info_page, &asset_server, &fonts, &ui_strings, s);
                    });
                }
                CosPage::Inventory => {
                    container.with_children(|bag_page| {
                        inventory::build_inventory_page(
                            bag_page,
                            &asset_server,
                            &fonts,
                            &ui_strings,
                            s,
                        );
                    });
                }
                CosPage::Setup => {
                    container.with_children(|setup_page| {
                        setup::build_setup_page(setup_page, &asset_server, &fonts, &ui_strings, s);
                    });
                }
            }
        }
    });
}

pub fn cleanup_cos_window(mut commands: Commands, windows: Query<Entity, With<CosWindowRoot>>) {
    for entity in windows.iter() {
        commands.entity(entity).despawn();
    }
}

// --- Behavior ---------------------------------------------------------------

fn on_close_button(_: On<Activate>, mut state: ResMut<CosWindowState>) {
    state.open = false;
}

/// A tab click selects its page — the only write to `CosWindowState.page`
/// besides the cargo cell, and the reason the setup page is reachable at all.
fn on_tab_button(
    activate: On<Activate>,
    tabs: Query<&CosTabButton>,
    mut state: ResMut<CosWindowState>,
) {
    let Ok(tab) = tabs.get(activate.entity) else {
        return;
    };
    state.page = tab.0;
}

/// Mirror `CosWindowState` onto the shell and its pages.
pub fn apply_cos_visibility(
    state: Res<CosWindowState>,
    asset_server: Res<AssetServer>,
    mut roots: Query<&mut Node, (With<CosWindowRoot>, Without<CosPageRoot>)>,
    mut pages: Query<(&CosPageRoot, &mut Node), Without<CosWindowRoot>>,
    mut tabs: Query<(&CosTabButton, &mut ImageNode)>,
) {
    if !state.is_changed() {
        return;
    }
    // lit tab follows the page; on the cargo page neither is lit, because
    // vanilla names no third tab (module note).
    for (tab, mut image) in tabs.iter_mut() {
        image.image = asset_server.load(if state.page == tab.0 {
            TAB_ON_DDJ
        } else {
            TAB_OFF_DDJ
        });
    }
    for mut node in roots.iter_mut() {
        node.display = if state.open {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (page, mut node) in pages.iter_mut() {
        node.display = page_display(state.page, page.0);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The chrome's outer box must come out as vanilla's own shell rect
    /// (`ginterface.txt:1330` `Rect="0,0,355,390"`) — the content box is
    /// derived from it, not chosen.
    #[test]
    fn shell_outer_box_is_the_vanilla_cos_rect() {
        assert_eq!(game_window::outer_size((CONTENT_W, CONTENT_H)), WINDOW_SIZE);
    }

    /// The page host keeps vanilla's `12,66,331,314` after the rebase into the
    /// shell's content space, i.e. rebasing is a pure translation by the
    /// chrome's content origin (12, 36).
    #[test]
    fn page_rect_rebases_onto_the_shared_vanilla_rect() {
        assert_eq!(page_rect(), (0.0, 30.0, 331.0, 314.0));
        let (x, y, w, h) = page_rect();
        assert_eq!(
            (
                x + game_window::FRAME_VIS_SIDE + game_window::CHROME_PAD,
                y + game_window::CONTENT_TOP,
                w,
                h
            ),
            PAGE_RECT_WINDOW
        );
    }

    /// The strip carries exactly the two tabs vanilla names in textdata —
    /// `..._TABMENU_BASICINFO` and `..._TABMENU_TECHNOLOGY` — and no invented
    /// third one for the cargo page, which keeps its command-cell opener.
    #[test]
    fn the_strip_is_the_two_tabs_vanilla_names() {
        assert_eq!(TABS.map(|(page, ..)| page), [CosPage::Info, CosPage::Setup]);
        assert!(
            !TABS.iter().any(|(page, ..)| *page == CosPage::Inventory),
            "the cargo page has the UIIT_STT_COS_INVENTORY command cell, not a tab"
        );
    }

    /// The strip sits on the page frame's top edge and stays inside the shell's
    /// content box — the only two constraints the placement has, since vanilla
    /// ships no rect for it.
    #[test]
    fn the_tab_strip_sits_on_the_page_frame_and_fits_the_shell() {
        assert_eq!(tab_strip_y() + TAB_SIZE.1, page_rect().1);
        assert!(
            tab_strip_y() >= 0.0,
            "the strip must not leave the content box"
        );
        let right = tab_x(TABS.len() - 1) + TAB_SIZE.0;
        assert!(
            right <= CONTENT_W,
            "strip runs past the shell: {right} > {CONTENT_W}"
        );
    }

    /// The chain the setup page never had: a click on the "Setting" tab is the
    /// write to `CosWindowState.page` that makes `setup.rs` (and with it
    /// `0x7420`) reachable. Positive control in the same world: the other tab
    /// switches back.
    #[test]
    fn clicking_the_setting_tab_selects_the_setup_page() {
        let mut app = App::new();
        app.init_resource::<CosWindowState>();
        let tabs: Vec<Entity> = TABS
            .iter()
            .map(|(page, ..)| {
                app.world_mut()
                    .spawn(CosTabButton(*page))
                    .observe(on_tab_button)
                    .id()
            })
            .collect();
        assert_eq!(app.world().resource::<CosWindowState>().page, CosPage::Info);

        app.world_mut().trigger(Activate { entity: tabs[1] });
        assert_eq!(
            app.world().resource::<CosWindowState>().page,
            CosPage::Setup,
            "the Setting tab must open the setup page"
        );
        app.world_mut().trigger(Activate { entity: tabs[0] });
        assert_eq!(app.world().resource::<CosWindowState>().page, CosPage::Info);
    }

    /// `ifcos.txt` gives the three pages one rect and the grammar has no
    /// `Visible` key, so exactly one page may be displayed at a time ([S]).
    #[test]
    fn exactly_one_page_is_displayed() {
        for selected in [CosPage::Info, CosPage::Inventory, CosPage::Setup] {
            let shown = [CosPage::Info, CosPage::Inventory, CosPage::Setup]
                .into_iter()
                .filter(|p| page_display(selected, *p) == Display::Flex)
                .count();
            assert_eq!(shown, 1, "page {selected:?}");
        }
    }
}
