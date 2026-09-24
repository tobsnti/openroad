//! Community window shell (`GDR_COMMUNITY`, id 23) + its six-page host.
//!
//! Idea: the vanilla shell is `ginterface.txt:541` `GDR_COMMUNITY:CIFCommunity`
//! — `Rect="0,0,477,393"`, `DDJ="interface\frame\mframe_wnd_"`,
//! `Text="UIIT_STT_COMMUNITY"` — so it rides on the shared `game_window`
//! chrome, and its content box is derived by subtracting that chrome's margins
//! from 477x393 rather than chosen. The six page controls of
//! `resinfo/ifcommunity.txt` all share the rect `13,61,451,320`, rebased here
//! into the shell's content space by subtracting the chrome's content origin
//! `(FRAME_VIS_SIDE + CHROME_PAD, CONTENT_TOP)` = `(12, 36)`, as
//! `character_info` does (#310: subtract the shell's own constants, never
//! re-derive the origin from vanilla's interior art).
//!
//! This is the owning shell mail needs; it
//! replaces the underbar's "community window not implemented yet" stub. Only
//! the Letter page has a body — the other five stay empty containers for the
//! guild / friend / war-state / blocking rows.
//!
//! `ifcommunity.txt` itself has no button block — the classic generation
//! composes its tab strip in the engine, not in data. The strip built here is
//! therefore
//! the one the 4th-gen shell declares, Friend / Blocking / Letter with the
//! original's art and pitch; see `TABS`. The guild page is not one of its tabs
//! in any generation and is reached from the under-bar's own Guild row.

use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button};

use crate::assets::FontAssets;
use crate::plugins::config::ClientConfig;
use crate::plugins::hud::community::friend::spawn_friend_page;
use crate::plugins::hud::community::guild::{
    spawn_guild_page, spawn_guild_relations_page, GuildRelationsState,
};
use crate::plugins::hud::community::letter::spawn_letter_page;
use crate::plugins::hud::community::model::{CommunityPage, CommunityState};
use crate::plugins::hud::game_window::{self, abs_node};
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::textdata::ClientUiStrings;

// --- Layout constants (resinfo, window units) -------------------------------

/// Vanilla shell rect (`ginterface.txt:541` `Rect="0,0,477,393"`).
const WINDOW_SIZE: (f32, f32) = (477.0, 393.0);
const CONTENT_W: f32 =
    WINDOW_SIZE.0 - 2.0 * (game_window::FRAME_VIS_SIDE + game_window::CHROME_PAD);
const CONTENT_H: f32 = WINDOW_SIZE.1
    - game_window::CONTENT_TOP
    - game_window::CHROME_PAD
    - game_window::FRAME_VIS_BOTTOM;

/// The rect all six page controls share (`ifcommunity.txt`), window space.
const PAGE_RECT_WINDOW: (f32, f32, f32, f32) = (13.0, 61.0, 451.0, 320.0);

/// The page tabs, in the original's own order and art.
///
/// Idea: the classic generation lays out **no** tab strip in resinfo — the
/// buttons are engine-composed (0 of 247 resinfo files reference any
/// `gil_*_tab_*` art). The one strip that *is*
/// declared anywhere is the 4th-gen shell's, `cumunity_friend.2dt[1][2][3]`:
/// three 72x24 tab buttons, `gil_friend_tab_off` / `gil_cut_tab_off` /
/// `gil_note_tab_off`, icon-only with no caption. So the strip is the
/// original's — the *social* trio Friend / Blocking / Letter.
///
/// **Guild tab, corrected**: the guild page is not tab-less in the original, it simply lives in a shell of
/// its own. `res_ui/guild.2dt` (`CNIFGuildWnd`, root id 176) declares three
/// `Type=16` tab buttons — ids 72/73/74 at `127|204|280,122,72,24`, pointing at
/// `ContentId` 47 (guild info), 164 (`guild_r.2dt`, relations) and 548
/// (`guild_w.2dt`, war). Two facts fall out: the guild's own tab art stem is
/// `gil_info_tab` (`interface/guild/gil_info_tab_{off,on}.ddj`), and that strip
/// authors the very same non-uniform pitch, 77 then 76. Since *we* host page 10
/// as the community window's guild page rather than as a second shell
/// (`ifcommunity.txt:125` id 10 is where the classic generation puts it), its
/// tab belongs on this strip: the art and the pitch are the original's, only
/// the host is ours. Without it the page is unreachable by construction.
///
/// **Ours (ADR-0009 deviation, geometry only)**: the 4th-gen x/y (313/390/466
/// at y=231) are full-screen design coordinates and cannot be transcribed into
/// the classic 477x393 shell. The authored *pitch* is kept (77 then 76,
/// non-uniform), the origin is the page rect's own x, and the strip sits in the
/// 24-unit gap the classic layout leaves above the shared page rect
/// (window y 37 = page y 61 minus the tab height).
const TAB_SIZE: (f32, f32) = (72.0, 24.0);
const TAB_Y_WINDOW: f32 = PAGE_RECT_WINDOW.1 - TAB_SIZE.1;
const TABS: [(CommunityPage, f32, &str); 5] = [
    (CommunityPage::Friend, PAGE_RECT_WINDOW.0, "gil_friend_tab"),
    (
        CommunityPage::Blocking,
        PAGE_RECT_WINDOW.0 + 77.0,
        "gil_cut_tab",
    ),
    (
        CommunityPage::Letter,
        PAGE_RECT_WINDOW.0 + 77.0 + 76.0,
        "gil_note_tab",
    ),
    // The guild shell's own info tab, continuing the authored 77/76
    // alternation (`guild.2dt` ids 72/73/74 step 77 then 76 as well).
    (
        CommunityPage::Guild,
        PAGE_RECT_WINDOW.0 + 77.0 + 76.0 + 77.0,
        "gil_info_tab",
    ),
    // Guild relations (`ifcommunity.txt:106` id 11), art stem from the same
    // family: `guild.2dt` id 73 is the relations tab and its off/on pair ships
    // as `interface/guild/gil_relationship_tab_{off,on}.ddj`.
    //
    // **Structural deviation from the original, deliberate.**
    // The 4th-gen client shows "Guild rel." as a *sub-tab inside a guild window
    // of its own* (`guild.2dt` id 73, `ContentId=164` → `guild_r.2dt`). We show
    // it as a sibling page of the community shell, because that is what the
    // generation we actually build is: our guild page *is* community page 10
    // (`ifcommunity.txt:125`) and this is community page 11 on the same shared
    // rect — one shell, six pages, tab strip composed in code because the
    // classic data declares none. Anyone comparing the two windows side by side
    // will see this difference; it is the hosting generation, not a mistake.
    (
        CommunityPage::GuildRelations,
        PAGE_RECT_WINDOW.0 + 77.0 + 76.0 + 77.0 + 76.0,
        "gil_relationship_tab",
    ),
];

fn tab_art(stem: &str, on: bool) -> String {
    format!(
        "media://interface/guild/{stem}_{}.ddj",
        if on { "on" } else { "off" }
    )
}

/// Spawn anchor (right/top, physical px). **Ours**: `GDR_COMMUNITY` is one of
/// the framed windows `wndpos.dat` does not persist, so vanilla's own default
/// position is not in the data.
const WINDOW_RIGHT: f32 = 320.0;
const WINDOW_TOP: f32 = 70.0;

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

fn page_display(selected: CommunityPage, page: CommunityPage) -> Display {
    if selected == page {
        Display::Flex
    } else {
        Display::None
    }
}

// --- Markers ----------------------------------------------------------------

#[derive(Component)]
pub struct CommunityWindowRoot;

#[derive(Component)]
pub struct CommunityPageRoot(pub CommunityPage);

/// One tab of the strip, carrying the page it selects and its art stem.
#[derive(Component)]
pub struct CommunityTab(pub CommunityPage, pub &'static str);

// --- Spawning ---------------------------------------------------------------

/// Spawn the (initially hidden) community window.
pub fn spawn_community_window(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    state: Res<CommunityState>,
    relations_state: Res<GuildRelationsState>,
    config: Res<ClientConfig>,
    cam_query: Query<Entity, With<Camera2d>>,
) {
    let Ok(camera) = cam_query.single() else {
        warn!("community window: no 2d camera to attach to");
        return;
    };
    let s = hud_scale();

    let window = game_window::spawn_game_window(
        &mut commands,
        &asset_server,
        &fonts,
        camera,
        ui_strings.get_or("UIIT_STT_COMMUNITY", "Community"),
        (CONTENT_W, CONTENT_H),
        (WINDOW_RIGHT, WINDOW_TOP),
        s,
    );
    commands
        .entity(window.root)
        .insert((CommunityWindowRoot, GlobalZIndex(55)))
        .entry::<Node>()
        .and_modify(|mut node| node.display = Display::None);
    commands
        .entity(window.expect_close_button())
        .observe(on_close_button);

    let (px, py, pw, ph) = page_rect();
    let tab_y = TAB_Y_WINDOW - game_window::CONTENT_TOP;
    commands.entity(window.content).with_children(|content| {
        for (page, x, stem) in TABS {
            content
                .spawn((
                    CommunityTab(page, stem),
                    Button,
                    abs_node(
                        (
                            x - (game_window::FRAME_VIS_SIDE + game_window::CHROME_PAD),
                            tab_y,
                            TAB_SIZE.0,
                            TAB_SIZE.1,
                        ),
                        s,
                    ),
                    ImageNode {
                        image: asset_server.load(tab_art(stem, page == state.page)),
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                ))
                .observe(on_tab_press);
        }
        for page in CommunityPage::ALL {
            let mut node = abs_node((px, py, pw, ph), s);
            node.display = page_display(state.page, page);
            let mut entity = content.spawn((CommunityPageRoot(page), node, Pickable::IGNORE));
            if page == CommunityPage::Letter {
                entity.with_children(|letter| {
                    spawn_letter_page(letter, &asset_server, &fonts, &ui_strings, s);
                });
            }
            if page == CommunityPage::Friend {
                entity.with_children(|friend| {
                    spawn_friend_page(friend, &asset_server, &fonts, &ui_strings, s);
                });
            }
            if page == CommunityPage::Guild {
                entity.with_children(|guild| {
                    spawn_guild_page(
                        guild,
                        &asset_server,
                        &fonts,
                        &ui_strings,
                        config.guild.position_grant,
                        s,
                    );
                });
            }
            if page == CommunityPage::GuildRelations {
                entity.with_children(|relations| {
                    spawn_guild_relations_page(
                        relations,
                        &asset_server,
                        &fonts,
                        &ui_strings,
                        &relations_state,
                        s,
                    );
                });
            }
        }
    });
}

pub fn cleanup_community_window(
    mut commands: Commands,
    windows: Query<Entity, With<CommunityWindowRoot>>,
) {
    for entity in windows.iter() {
        commands.entity(entity).despawn();
    }
}

// --- Behavior ---------------------------------------------------------------

fn on_close_button(_: On<Activate>, mut state: ResMut<CommunityState>) {
    state.open = false;
}

/// The strip selects a page; it never closes the window.
///
/// Two of the four pages it can reach (Friend 13, Blocking 15) are still empty
/// containers here, so the tab shows an empty pane until those bodies land —
/// that is the honest state of the tree, and it is still strictly better than
/// the page being unreachable by construction. The fourth tab is the guild
/// page, which is *not* empty: the net-fed lines of `community/guild.rs` that
/// no player could reach before this strip existed.
fn on_tab_press(
    activate: On<Activate>,
    tabs: Query<&CommunityTab>,
    mut state: ResMut<CommunityState>,
) {
    if let Ok(tab) = tabs.get(activate.entity) {
        state.page = tab.0;
    }
}

/// Mirror `CommunityState` onto the shell and its pages.
pub fn apply_community_visibility(
    state: Res<CommunityState>,
    asset_server: Res<AssetServer>,
    mut roots: Query<&mut Node, (With<CommunityWindowRoot>, Without<CommunityPageRoot>)>,
    mut pages: Query<(&CommunityPageRoot, &mut Node), Without<CommunityWindowRoot>>,
    mut tabs: Query<(&CommunityTab, &mut ImageNode)>,
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
    for (page, mut node) in pages.iter_mut() {
        node.display = page_display(state.page, page.0);
    }
    for (tab, mut image) in tabs.iter_mut() {
        image.image = asset_server.load(tab_art(tab.1, tab.0 == state.page));
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The chrome's outer box must come out as vanilla's shell rect
    /// (`ginterface.txt:541` `0,0,477,393`).
    #[test]
    fn shell_outer_box_is_the_vanilla_community_rect() {
        assert_eq!(game_window::outer_size((CONTENT_W, CONTENT_H)), WINDOW_SIZE);
    }

    /// Rebasing the shared page rect is a pure translation by the chrome's
    /// content origin (12, 36), and the page still fits the content width.
    #[test]
    fn page_rect_rebases_onto_the_shared_vanilla_rect() {
        assert_eq!(page_rect(), (1.0, 25.0, 451.0, 320.0));
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
        assert!(x + w <= CONTENT_W);
    }

    /// The strip is the original's art in the original's order and pitch: the
    /// declared social trio from `cumunity_friend.2dt`, plus the guild shell's
    /// own `gil_info_tab` (`guild.2dt` id 72) because *we* host page 10 here
    /// instead of in a second shell. Every stem is a shipped
    /// `interface/guild/*_off|on.ddj` pair; the pitch (77 then 76, then 77
    /// again) is the alternation both 2dt strips author.
    #[test]
    fn the_tab_strip_is_the_declared_social_trio_plus_the_guild_info_tab() {
        assert_eq!(
            TABS.map(|(page, _, stem)| (page, stem)),
            [
                (CommunityPage::Friend, "gil_friend_tab"),
                (CommunityPage::Blocking, "gil_cut_tab"),
                (CommunityPage::Letter, "gil_note_tab"),
                (CommunityPage::Guild, "gil_info_tab"),
                (CommunityPage::GuildRelations, "gil_relationship_tab"),
            ]
        );
        assert_eq!(TABS[1].1 - TABS[0].1, 77.0);
        assert_eq!(TABS[2].1 - TABS[1].1, 76.0);
        assert_eq!(TABS[3].1 - TABS[2].1, 77.0);
        assert_eq!(TABS[4].1 - TABS[3].1, 76.0);
        // and it sits in the gap the shared page rect leaves above itself,
        // inside the shell — not over the pages and not off the frame
        assert_eq!(TAB_Y_WINDOW, 37.0);
        assert!(TAB_Y_WINDOW + TAB_SIZE.1 <= PAGE_RECT_WINDOW.1);
        assert!(TABS[4].1 + TAB_SIZE.0 <= PAGE_RECT_WINDOW.0 + PAGE_RECT_WINDOW.2);
        assert_eq!(
            tab_art("gil_note_tab", true),
            "media://interface/guild/gil_note_tab_on.ddj"
        );
        assert_eq!(
            tab_art("gil_info_tab", false),
            "media://interface/guild/gil_info_tab_off.ddj"
        );
    }

    /// A tab press selects its page and leaves the window open — the switch
    /// `CommunityState.page` never had: the field was written in exactly one
    /// place, its own `Default`.
    #[test]
    fn a_tab_press_selects_its_page() {
        let mut app = App::new();
        app.init_resource::<CommunityState>();
        app.world_mut().resource_mut::<CommunityState>().open = true;
        for (page, _, stem) in TABS {
            let tab = app
                .world_mut()
                .spawn(CommunityTab(page, stem))
                .observe(on_tab_press)
                .id();
            app.world_mut().trigger(Activate { entity: tab });
            app.update();
            let state = app.world().resource::<CommunityState>();
            assert_eq!(state.page, page, "tab {stem} did not select its page");
            assert!(state.open, "a tab press must not close the window");
        }
    }

    /// Six pages share one rect and the grammar has no `Visible` key, so
    /// exactly one may be displayed at a time.
    #[test]
    fn exactly_one_community_page_is_displayed() {
        for selected in CommunityPage::ALL {
            let shown = CommunityPage::ALL
                .into_iter()
                .filter(|p| page_display(selected, *p) == Display::Flex)
                .count();
            assert_eq!(shown, 1, "page {selected:?}");
        }
    }
}
