//! Intro chrome: the two full-width bars and the notice line that frame every
//! intro state.
//!
//! Idea: the bars are `GDR_STA_SCREENUP` / `GDR_STA_SCREENDOWN` (ids 1/2),
//! declared once per intro tree — and **each tree names its own art**, which is
//! why one handle cannot serve the whole scene:
//!
//! | state | tree | up / down art |
//! |---|---|---|
//! | title (splash, login, servers) | `pstitle_europe.txt:729/710` | `blackbar_up_18_europe` / `blackbar_down_copyright_europe` |
//! | character list + region select | `pscharacterselect_europe.txt:1010/991` | `blackbar_up_europe` / `blackbar_down_europe` |
//! | character create (European) | `pscharactercreate_europe.txt:253/234` | `redbar_up_europe` / `redbar_down_europe` |
//! | character create (Chinese) | `pscharactercreatechina.txt:253/234` | `redbar_up` / `redbar_down` |
//!
//! The create row is **two** rows because the create screen is two trees, one
//! per race — `_europe` in these filenames is the *race*, not the
//! `EUROPE_SYSTEM` `#ifdef` that resolves `pstitle.txt`. The two files differ
//! in their first section header (`Section = CreateEurope` vs
//! `Section = CreateChina`) and in five arts besides the bands. That is also
//! why there is a 4th `redbar` reference below.
//!
//! `blackbar` alone appears 13 times and in none of the create trees, which
//! reads as "the create screen has no chrome". It has one — in red. Across the
//! resinfo files there are 16 `interface\outer\*bar*` references: 13 `blackbar`
//! + 4 `redbar`. All eight arts are 1600x172 ARGB1555 and distinct.
//!
//! `#ifdef` resolution is unambiguous: `define.txt:7` defines `EUROPE_SYSTEM`
//! and `APPLY_GNGWC_SYSTEM_2007` is absent from its 22 symbols, so `pstitle.txt`
//! resolves to exactly the pre-resolved `pstitle_europe.txt` pair.
//!
//! Because the bars are spawned once for the whole scene, the art is swapped on
//! state change ([`update_chrome_art`]) rather than re-spawned.

use bevy::prelude::*;
use bevy::text::FontSourceTemplate;

use crate::assets::FontAssets;

use super::assets::IntroV2Assets;
use super::character_create::{CharCreateSelection, Race};
use super::{intro_font_px, IntroV2State};

/// Bar rects, verbatim: `Rect="0,0,1600,172"` and `Rect="0,1030,1600,172"` in
/// the trees' 1600x1200 design space. Height is expressed as a percentage of
/// that space so the bars scale with the window, as they did before; 1030+172
/// overruns the 1200 canvas by 2px, which is why the bottom bar is anchored to
/// the bottom edge instead of to y=1030.
/// The canvas height, from the one shared constant
/// ([`crate::plugins::ui_v2::RESINFO_CANVAS`]) — not a second literal `1200`.
const DESIGN_H: f32 = crate::plugins::ui_v2::RESINFO_CANVAS.1;
const BAR_H: f32 = 172.0;
/// 172/1200 = 14.33%.
///
/// `pub` because it is not only the bars' own height: the original places the
/// lobby's button row and info box *relative to the bands*
/// (`y = barDown.top + 27`, `y = barUp.bottom + 41`), so `character_select`
/// anchors against this same percentage instead of re-deriving it from a second
/// copy of 172/1200.
pub const BAR_H_PCT: f32 = 100.0 * BAR_H / DESIGN_H;

/// Notice-line colour of the title and select trees: `GDR_TEXT_MESSAGE`
/// (`CIFTextBox`, id 500) `FontColor="255,255,103,29"` in
/// `pstitle_europe.txt:634` **and** `pscharacterselect_europe.txt:497` —
/// resinfo COLOR is ARGB, so RGB(255,103,29). Byte-exact; do not "fix" it
/// against the editor-scratch `Color=` on the same block (`255,23,9,242` in
/// the title tree, `255,254,117,204` in the select tree — two different
/// scratch values for one identical `FontColor`, which is what makes them
/// scratch).
const NOTICE_COLOR: Color = Color::srgb_u8(255, 103, 29);
/// The **create** tree authors the same control WHITE:
/// `pscharactercreate_europe.txt:196` `GDR_TEXT_MESSAGE` `CIFTextBox`
/// `FontColor="255,255,255,255"` (its scratch `Color=` is `255,136,9,73`).
/// The notice line is spawned once for the whole scene, so without a per-tree
/// swap ([`update_chrome_art`]) it keeps the title tree's orange where the data
/// says white — and it sits on the RED bar there, which is where the original's
/// own contrast argument for white shows up.
const NOTICE_COLOR_CREATE: Color = Color::WHITE;

/// The notice colour the given state's tree authors (see the two constants).
fn notice_color(state: IntroV2State) -> Color {
    match bar_tree(state) {
        BarTree::Create => NOTICE_COLOR_CREATE,
        BarTree::Select | BarTree::Title => NOTICE_COLOR,
    }
}
/// **Sourced.** Id 500's `Rect="0,0,0,0"` says nothing about the size, but its
/// `FontIndex=0` does: the ladder behind [`intro_font_px`] resolves index 0
/// to **12 px** (`GDR_TEXT_MESSAGE`, `pstitle_europe.txt:634` and
/// `pscharacterselect_europe.txt:497`, both `FontIndex=0`).
fn notice_font_size() -> f32 {
    intro_font_px(0)
}
/// Height of exactly one notice line — Bevy's default line height
/// (`LineHeight::RelativeToFont(1.2)`) applied to [`notice_font_size`]. See
/// [`info_text`] for why the node is sized to one line.
const NOTICE_LINE_HEIGHT_FACTOR: f32 = 1.2;
fn notice_line_height() -> f32 {
    notice_font_size() * NOTICE_LINE_HEIGHT_FACTOR
}
/// The notice line does *not* clear the bottom bar — the original draws it **on
/// top of it**. In an 800x600 client the message ink occupies rows **542..552**
/// while the bottom bar band starts at row **515**. In the tree's 1600x1200
/// design space that is rows 1084..1104 against a bar starting at 1030, i.e. the
/// ink bottom sits **96 design px = 8% of the design height** above the bottom
/// edge.
///
/// Lifting the line out of the band would move it ~40 px off the original; the
/// bar art is its own UI root and stacked later, so readability is a z question
/// ([`NOTICE_Z`]), not a placement one.
const NOTICE_BOTTOM_PCT: f32 = 8.0;
/// **Sourced.** `GDR_TEXT_MESSAGE` is `HAlign=0` (left) in
/// `pstitle_europe.txt:634`, and in the original the ink starts at x=9..10 in
/// every error state — the short "Password entry has failed..." line and the
/// screen-wide "This user is already connected..." line begin at the same x, so
/// it is left-aligned, not centred. 20/1600 of the design width = 1.25%.
const NOTICE_LEFT_PCT: f32 = 1.25;
/// Above the two bars, below the fade overlay (`GlobalZIndex(100)`) and the
/// windows that own the screen (region select 500, character list 200). This is
/// what keeps the line readable while it sits *inside* the bar band.
const NOTICE_Z: i32 = 5;

/// Marker for the top bar.
#[derive(Component, Default, Clone)]
pub struct HeaderV2;

/// Marker for the bottom bar.
#[derive(Component, Default, Clone)]
pub struct FooterV2;

/// Marker for the notice line at the bottom of the screen.
#[derive(Component, Default, Clone)]
pub struct InfoTextV2;

/// Replaces the notice line's content.
#[derive(Message)]
pub struct InfoTextV2Update(pub String);

/// Which intro tree frames a given state (see the table in the module doc).
/// Split out of the asset lookup so the mapping itself is testable.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum BarTree {
    /// `pstitle_europe.txt:729/710`
    Title,
    /// `pscharacterselect_europe.txt:1010/991` — the list and the region board
    /// are both sections of that one tree
    Select,
    /// `pscharactercreate_europe.txt:253/234` — red, not black
    Create,
}

pub fn bar_tree(state: IntroV2State) -> BarTree {
    match state {
        IntroV2State::CharacterCreate => BarTree::Create,
        IntroV2State::CharacterList | IntroV2State::RegionSelect => BarTree::Select,
        IntroV2State::Loading
        | IntroV2State::Splash
        | IntroV2State::LoginForm
        | IntroV2State::ServerSelection => BarTree::Title,
    }
}

/// The create screen's bands, per race.
///
/// Worth stating once, because the names invite a wrong shortcut: **only the
/// Chinese pair is red.** `redbar_up_europe`/`redbar_down_europe` are
/// teal/blue-grey with European scrollwork; `redbar_up`/`redbar_down` are red
/// with a pagoda-and-dragon silhouette. Both pairs are 1600x172 A1R5G5B5 (masks
/// `R=0x7c00 G=0x3e0 B=0x1f A=0x8000`): mid-band RGB is 153,28,0 for the Chinese
/// art against 49,83,92 for the European one, and 94.4% (up) / 97.7% (down) of
/// pixels differ.
///
/// The name is not wrong, it just names the *role* — "the ornamental band at
/// the top of the create screen" — while the colour is styling per race. The
/// data pair every such role by suffix (bare = Chinese, `_europe` = European),
/// which is what lets a caller swap one name part: 15 of the 19 `_europe`
/// files in this PK2 have exactly that bare counterpart. So the European art
/// is not "the red band in another language", it is the other half of a pair.
///
/// Choosing art by race is this screen's existing pattern, not a new one:
/// `region_select::spawn_loading_cut` already picks
/// `LOADING_CHINA_DDJ`/`LOADING_EUROPE_DDJ` off the same [`Race`].
/// A race the interface data does not pair art for gets the bare (un-suffixed)
/// half of the pair — the file that always ships — rather than a guessed
/// `_<code>` filename.
fn create_bar_art(race: Race, assets: &IntroV2Assets) -> (Handle<Image>, Handle<Image>) {
    if race == Race::EUROPEAN {
        (
            assets.redbar_up_europe.clone(),
            assets.redbar_down_europe.clone(),
        )
    } else {
        (
            assets.redbar_up_china.clone(),
            assets.redbar_down_china.clone(),
        )
    }
}

fn bar_art(
    state: IntroV2State,
    race: Race,
    assets: &IntroV2Assets,
) -> (Handle<Image>, Handle<Image>) {
    match bar_tree(state) {
        BarTree::Create => create_bar_art(race, assets),
        BarTree::Select => (assets.blackbar_up.clone(), assets.blackbar_down.clone()),
        BarTree::Title => (assets.title_bar_up.clone(), assets.title_bar_down.clone()),
    }
}

pub fn header(assets: &IntroV2Assets) -> impl Scene {
    // spawned in Loading; the title art is what the first visible state
    // (Splash) declares, and update_chrome_art swaps it from there on
    let image = assets.title_bar_up.clone();
    bsn! {
        HeaderV2
        Name("Header V2")
        ImageNode { image: {image}, image_mode: NodeImageMode::Stretch }
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            height: percent(BAR_H_PCT),
            top: px(0),
        }
        Pickable::IGNORE
    }
}

pub fn footer(assets: &IntroV2Assets) -> impl Scene {
    let image = assets.title_bar_down.clone();
    bsn! {
        FooterV2
        Name("Footer V2")
        ImageNode { image: {image}, image_mode: NodeImageMode::Stretch }
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            height: percent(BAR_H_PCT),
            bottom: px(0),
        }
        Pickable::IGNORE
    }
}

/// The notice line, with the UI face passed in explicitly.
///
/// # Why the font handle is not optional here
///
/// Without a `font:` handle Bevy falls back to its embedded default face
/// `FiraMono-subset.ttf` (`bevy_text-0.19.1/src/`). That subset has **no glyph
/// for U+2026**: a cmap lookup on it answers gid 0 (`.notdef`, i.e. the tofu
/// box) for U+2026 while `A` resolves to gid 36 — ASCII is in, the ellipsis is
/// not. And the string the connect path puts here is `UIO_MSG_ERROR_CITATION`
/// (`textdata/textuisystem.txt:190`), whose cell is literally
/// `U+2026 "Requesting user confirmation" U+2026` — an ellipsis on *both* ends,
/// i.e. two tofu boxes around that line.
///
/// Every face this client can actually end up with has the glyph (`영문서체.ttf`
/// gid 171, `기본서체`/`채팅서체` gid 1608, the bundled `FiraSans-Medium` gid
/// 2157), so the line is handed the same `fonts.nine` the rest of the intro
/// uses: no tofu, and the status line is in the UI face rather than a monospace
/// one.
///
/// # Why the node has a one-line height
///
/// The line is anchored by its *bottom* edge ([`NOTICE_BOTTOM_PCT`], which is a
/// one-line value), so a node sized to its content would let a three-line
/// message grow **upwards**: its first line would land two line heights above
/// where every one-line message sits.
///
/// The original does the opposite: a one-line message starts at client row
/// **542**, and a three-line one (`UIO_STT_CHAR_DEL_WAITING`) starts its *first*
/// line at **544** with the other two below it (line tops 544 / 562 / 577, pitch
/// ~16.5 px on the 800x600 client area). Within ~2 px the first line does not
/// move: the original's notice line is **top-anchored and grows downwards**.
///
/// So the node keeps its bottom edge and gets the height of exactly one line:
/// its *top* edge — and with it the first baseline — stays where the one-line
/// case puts it, and further lines overflow downwards over the band, as they do
/// in the original. `1.2 x font size` is Bevy's own default line height
/// (`TextLayout`'s default `LineHeight::RelativeToFont(1.2)`), so the height is
/// that rule applied, not a new number.
pub fn info_text(fonts: &FontAssets) -> impl Scene {
    let font = fonts.nine.clone();
    let line_h = notice_line_height();
    bsn! {
        InfoTextV2
        Name("Info Text V2")
        Text("")
        TextFont { font: FontSourceTemplate::Handle({font}), font_size: {FontSize::Px(notice_font_size())} }
        TextColor(NOTICE_COLOR)
        TextLayout::justify(Justify::Left)
        Node {
            position_type: PositionType::Absolute,
            bottom: percent(NOTICE_BOTTOM_PCT),
            left: percent(NOTICE_LEFT_PCT),
            right: px(0),
            // one line tall on purpose — see the function doc
            height: px(line_h),
            justify_content: JustifyContent::FlexStart,
        }
        GlobalZIndex({NOTICE_Z})
        Pickable::IGNORE
    }
}

/// Swap the bars to the art the current intro state's tree declares — and the
/// notice line to the colour that tree authors for `GDR_TEXT_MESSAGE`. The bars
/// and the notice line are spawned once for the whole scene, so without this
/// the create screen keeps the select screen's black bars where the data says
/// red, and the title tree's orange notice where the data says white.
///
/// Both halves live in one system on purpose: they read the same state, they
/// are driven by the same "one entity, three trees" fact, and registering a
/// second system means editing the plugin registry in `mod.rs`.
pub fn update_chrome_art(
    // Option: the sub-state resource only exists while the intro scene runs
    state: Option<Res<State<IntroV2State>>>,
    assets: Option<Res<IntroV2Assets>>,
    // Option for the same reason as `state`, one level down: the creation
    // choice is inserted by the region board on the way into
    // `CharacterCreate` and removed on the way out, so it is absent for most
    // of the intro — and a system whose `Res` is missing does not get skipped,
    // it fails parameter validation and panics the schedule.
    selection: Option<Res<CharCreateSelection>>,
    mut headers: Query<&mut ImageNode, (With<HeaderV2>, Without<FooterV2>)>,
    mut footers: Query<&mut ImageNode, (With<FooterV2>, Without<HeaderV2>)>,
    mut notices: Query<&mut TextColor, With<InfoTextV2>>,
) {
    let (Some(state), Some(assets)) = (state, assets) else {
        return;
    };
    // Same idiom as `character_create.rs:1116`: the region board inserts the
    // choice before the state switch, and a screen entered any other way (dev
    // jump) gets the default race.
    let race = selection.map(|s| s.race).unwrap_or_default();
    let (up, down) = bar_art(**state, race, &assets);
    for mut image in headers.iter_mut() {
        if image.image != up {
            image.image = up.clone();
        }
    }
    for mut image in footers.iter_mut() {
        if image.image != down {
            image.image = down.clone();
        }
    }
    let colour = notice_color(**state);
    for mut text_color in notices.iter_mut() {
        if text_color.0 != colour {
            text_color.0 = colour;
        }
    }
}

/// Applies notice-line messages — and empties the line on a screen change.
///
/// Idea: the notice line is **one entity for the whole intro scene**
/// ([`info_text`]), while every sentence in it belongs to exactly one screen.
/// Without a clear-on-change, a sentence written for one screen is still
/// standing on the next one: click the data-blocked European plate on region
/// select, pick the Chinese one instead, and its rejection ("Out of service
/// area." / `region_select::PLATE_DISABLED_REASON`) rides along into
/// character creation — reported from the playtest as "unten steht irgendwie
/// out of service area". Clearing was per-screen handwork until now
/// (`character_select`, `net`), so every new writer had to remember it.
///
/// The original does empty the band on a screen change, and both frames of the
/// same recorded session are on disk: a peer capture
/// `22-after-delete.png` carries the three-line "The character's deletion is
/// reserved." notice on the select screen, and `27-create-screen.png` a few
/// clicks later shows the same band **empty**.
///
/// Why the clear cannot swallow a legitimate message: Bevy runs the
/// `StateTransition` schedule *before* `Update`, so a message an `OnEnter`
/// system wrote for the state we just entered is still unread when this system
/// runs — it is applied by the loop below, after the clear, in the very same
/// frame. The clear therefore removes only what the *previous* screen left
/// behind. Do not "simplify" this into an `OnEnter` system: that would run
/// before those writers and lose the ordering guarantee.
pub fn update_info_text(
    // Option: the sub-state resource only exists while the intro scene runs
    state: Option<Res<State<IntroV2State>>>,
    mut reader: MessageReader<InfoTextV2Update>,
    mut query: Query<&mut Text, With<InfoTextV2>>,
) {
    let Ok(mut text) = query.single_mut() else {
        return;
    };

    if state.is_some_and(|state| state.is_changed()) {
        text.0.clear();
    }

    for update in reader.read() {
        text.0 = update.0.clone();
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::scenes::SceneState;

    /// The bar height is the authored one, not a round percentage: 172 of the
    /// 1600x1200 design space. The old 15% was unsourced and 8px too tall.
    #[test]
    fn the_bar_height_is_the_authored_rect() {
        assert_eq!(BAR_H, 172.0);
        assert!((BAR_H_PCT - 14.3333).abs() < 0.001);
        assert_ne!(BAR_H_PCT, 15.0);
        // the authored bottom bar starts at 1030 and overruns the canvas by 2px
        assert_eq!(1030.0 + BAR_H - DESIGN_H, 2.0);
    }

    /// The notice line sits **inside** the bottom bar band, as it does in the
    /// original (ink rows 542..552 of 600, bar band from 515). Readability is
    /// [`NOTICE_Z`]'s job, not the layout's.
    #[test]
    fn the_notice_line_sits_on_the_bottom_bar_like_the_original() {
        const { assert!(NOTICE_BOTTOM_PCT < BAR_H_PCT) };
        // 552/600 of the client area = 1104/1200 of the design space, so the
        // ink bottom is 96 design px up; the node bottom is that, in percent
        assert_eq!(NOTICE_BOTTOM_PCT, 100.0 * (DESIGN_H - 1104.0) / DESIGN_H);
        // z-order, not layout, is what stops the bar art painting over it
        const { assert!(NOTICE_Z > 0) };
    }

    /// `GDR_TEXT_MESSAGE` is `HAlign=0`. In the original both the short message
    /// and the one that spans almost the whole screen start at the same
    /// x=9..10, which is only possible if the line is left-aligned.
    #[test]
    fn the_notice_line_is_left_aligned_not_centred() {
        // 10px of 800 client px = 20 of 1600 design px
        assert_eq!(NOTICE_LEFT_PCT, 100.0 * 20.0 / 1600.0);
    }

    /// Three trees, three bar pairs — and the create screen's are red, in the
    /// Chinese tree (the European create tree pairs the same role with a teal
    /// art; see [`create_bar_art`]). A search for `blackbar` alone misses that
    /// entirely, and the two black pairs are different arts, so "black for
    /// everything but create" is also wrong.
    #[test]
    fn each_intro_state_gets_its_own_trees_art() {
        assert_eq!(bar_tree(IntroV2State::CharacterCreate), BarTree::Create);
        assert_eq!(bar_tree(IntroV2State::CharacterList), BarTree::Select);
        assert_eq!(bar_tree(IntroV2State::RegionSelect), BarTree::Select);
        for state in [
            IntroV2State::Loading,
            IntroV2State::Splash,
            IntroV2State::LoginForm,
            IntroV2State::ServerSelection,
        ] {
            assert_eq!(bar_tree(state), BarTree::Title, "{state:?}");
        }
        // the title and select trees name DIFFERENT blackbar variants
        // (blackbar_up_18_europe vs blackbar_up_europe), so they are two
        // trees, not one shared pair
        assert_ne!(
            bar_tree(IntroV2State::Splash),
            bar_tree(IntroV2State::CharacterList)
        );
    }

    /// Builds the smallest world `update_info_text` needs: the state graph it
    /// watches, the message it reads, and the one notice entity it writes.
    fn notice_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<SceneState>()
            .add_sub_state::<IntroV2State>()
            .add_message::<InfoTextV2Update>()
            .add_systems(Update, update_info_text);
        app.world_mut().spawn((InfoTextV2, Text::new("")));
        app.world_mut()
            .resource_mut::<NextState<SceneState>>()
            .set(SceneState::IntroV2);
        app.update();
        app
    }

    fn notice_line(app: &mut App) -> String {
        let mut query = app.world_mut().query_filtered::<&Text, With<InfoTextV2>>();
        query.single(app.world()).unwrap().0.clone()
    }

    fn go_to(app: &mut App, state: IntroV2State) {
        app.world_mut()
            .resource_mut::<NextState<IntroV2State>>()
            .set(state);
        app.update();
    }

    /// Half one: a sentence written for one screen must not survive the screen
    /// change. The notice line is one entity for the whole intro scene, so
    /// nothing else would ever remove it — this is red without the clear in
    /// [`update_info_text`].
    #[test]
    fn a_screen_change_empties_the_notice_line() {
        let mut app = notice_app();
        go_to(&mut app, IntroV2State::RegionSelect);

        app.world_mut()
            .write_message(InfoTextV2Update("Out of service area.".into()));
        app.update();
        assert_eq!(notice_line(&mut app), "Out of service area.");

        // the region plate was blocked, the player picks the other race
        go_to(&mut app, IntroV2State::CharacterCreate);
        assert_eq!(notice_line(&mut app), "");
    }

    /// Half two: a message written *for the state being entered* — `OnEnter`
    /// runs in `StateTransition`, before `Update` — must survive the clear in
    /// the same frame. Without the ordering argument in [`update_info_text`]
    /// this is exactly what a clear-on-enter would eat, leaving the screen mute.
    #[test]
    fn a_message_written_for_the_new_screen_survives_the_clear() {
        let mut app = notice_app();
        go_to(&mut app, IntroV2State::CharacterList);
        app.world_mut()
            .write_message(InfoTextV2Update("stale".into()));
        app.update();
        assert_eq!(notice_line(&mut app), "stale");

        // what an OnEnter writer does: the message lands before Update runs
        app.world_mut()
            .resource_mut::<NextState<IntroV2State>>()
            .set(IntroV2State::CharacterCreate);
        app.world_mut()
            .write_message(InfoTextV2Update("name is already taken".into()));
        app.update();
        assert_eq!(notice_line(&mut app), "name is already taken");

        // and it is not itself wiped by the next frame of the same state
        app.update();
        assert_eq!(notice_line(&mut app), "name is already taken");
    }

    /// The notice colour is the block's `FontColor` (ARGB), not its `Color=`.
    #[test]
    fn the_notice_colour_is_the_fontcolor_not_the_scratch_color() {
        assert_eq!(NOTICE_COLOR, Color::srgb_u8(255, 103, 29));
        // Color="255,23,9,242" on the same block is editor scratch
        assert_ne!(NOTICE_COLOR, Color::srgb_u8(23, 9, 242));
        // ...and so is the select tree's, which is a DIFFERENT scratch value
        // (255,254,117,204) on an identical FontColor
        assert_ne!(NOTICE_COLOR, Color::srgb_u8(254, 117, 204));
    }

    /// Three trees, and the create one authors the notice line WHITE
    /// (`pscharactercreate_europe.txt:196` `FontColor="255,255,255,255"`)
    /// while title and select author RGB(255,103,29). One entity carries the
    /// line through all three states, so the colour follows the state exactly
    /// like the bar art does.
    #[test]
    fn the_notice_colour_follows_the_states_tree_like_the_bars_do() {
        assert_eq!(notice_color(IntroV2State::CharacterCreate), Color::WHITE);
        for state in [
            IntroV2State::Loading,
            IntroV2State::Splash,
            IntroV2State::LoginForm,
            IntroV2State::ServerSelection,
            IntroV2State::CharacterList,
            IntroV2State::RegionSelect,
        ] {
            assert_eq!(notice_color(state), NOTICE_COLOR, "{state:?}");
        }
        // the create screen's is not the title colour, which is what the
        // single spawned entity would otherwise keep
        assert_ne!(
            notice_color(IntroV2State::CharacterCreate),
            notice_color(IntroV2State::Splash)
        );
    }

    /// The two source files this screen's band choice is spread over. The
    /// check below is a *cross-file* one, because each file on its own can be
    /// self-consistent: `assets.rs` may declare one pair and `chrome.rs` use it
    /// for both races.
    const ASSETS_RS: &str = include_str!("assets.rs");
    const CHROME_RS: &str = include_str!("chrome.rs");

    /// `field name -> ddj file name` for every `redbar_*` asset declared in
    /// `assets.rs`, read out of the `#[asset(path = ...)]` attribute that
    /// precedes each field.
    fn declared_redbar_assets() -> Vec<(String, String)> {
        let mut out = Vec::new();
        let lines: Vec<&str> = ASSETS_RS.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let Some(rest) = line
                .trim()
                .strip_prefix(r#"#[asset(path = "media://interface/outer/redbar_"#)
            else {
                continue;
            };
            let file = format!("redbar_{}", rest.split('"').next().unwrap_or_default());
            let field = lines[i + 1]
                .trim()
                .strip_prefix("pub ")
                .and_then(|f| f.split(':').next())
                .unwrap_or_default()
                .to_string();
            out.push((field, file));
        }
        out
    }

    /// The `redbar_*` field names named inside one match arm of
    /// [`create_bar_art`]. Scoped to that function's body first (up to its
    /// closing brace) so the doc comments elsewhere in this file, which name
    /// the same identifiers, cannot be mistaken for code.
    fn fields_in_create_arm(race_variant: &str) -> Vec<String> {
        let body = CHROME_RS
            .split_once("fn create_bar_art(")
            .expect("create_bar_art exists")
            .1
            .split_once("\n}\n")
            .expect("create_bar_art is closed")
            .0;
        // The race list is data-borne, so the function is a comparison against
        // the one race whose art the data pairs (`Race::EUROPEAN`) plus the
        // bare fallback branch — not two enum arms.
        let (europe_branch, fallback_branch) = body
            .split_once("} else {")
            .expect("create_bar_art has a fallback branch");
        let arm = match race_variant {
            "European" => europe_branch,
            "Chinese" => fallback_branch,
            other => panic!("no {other} branch"),
        };
        arm.split("assets.")
            .skip(1)
            .filter_map(|s| s.split('.').next())
            .filter(|s| s.starts_with("redbar_"))
            .map(str::to_string)
            .collect()
    }

    /// Both create trees must be loaded, and they must be four *different*
    /// files. With only the European pair declared, the Chinese stage is framed
    /// in the European art.
    #[test]
    fn the_asset_collection_declares_one_band_pair_per_race() {
        let declared = declared_redbar_assets();
        assert_eq!(
            declared.len(),
            4,
            "expected four redbar assets (up/down x china/europe), got {declared:?}"
        );
        let files: Vec<&str> = declared.iter().map(|(_, f)| f.as_str()).collect();
        for want in [
            "redbar_up.ddj",
            "redbar_down.ddj",
            "redbar_up_europe.ddj",
            "redbar_down_europe.ddj",
        ] {
            assert!(files.contains(&want), "{want} is not declared: {files:?}");
        }
    }

    /// The cross-file invariant: the branch that answers a race the data pairs
    /// no `_europe` art for must name the fields that carry the files
    /// **without** the `_europe` suffix, and vice versa.
    /// `pscharactercreatechina.txt:253/234` names
    /// `redbar_up.ddj`/`redbar_down.ddj`; `pscharactercreate_europe.txt` at the
    /// same two lines names the `_europe` pair.
    ///
    /// This is what a swapped pair of match arms fails on — the case that is
    /// invisible to the compiler, since all four fields have the same type.
    #[test]
    fn the_create_bands_follow_the_race() {
        let declared: std::collections::HashMap<String, String> =
            declared_redbar_assets().into_iter().collect();

        let chinese = fields_in_create_arm("Chinese");
        let european = fields_in_create_arm("European");
        assert_eq!(chinese.len(), 2, "Chinese arm names {chinese:?}");
        assert_eq!(european.len(), 2, "European arm names {european:?}");

        for field in &chinese {
            let file = declared
                .get(field)
                .unwrap_or_else(|| panic!("{field} is not a declared asset"));
            assert!(
                !file.contains("_europe"),
                "the Chinese arm uses {field} -> {file}, which is the European tree's art"
            );
        }
        for field in &european {
            let file = declared
                .get(field)
                .unwrap_or_else(|| panic!("{field} is not a declared asset"));
            assert!(
                file.contains("_europe"),
                "the European arm uses {field} -> {file}, which is the Chinese tree's art"
            );
        }
        // and the two races must not share a band
        for field in &chinese {
            assert!(!european.contains(field), "{field} is used for both races");
        }
    }
}
