use bevy::image::TRANSPARENT_IMAGE_HANDLE;
use bevy::input::mouse::MouseWheel;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::Activate;

use packets::gateway::Shard;

use crate::assets::FontAssets;
use crate::plugins::net::gateway::shard_list::ShardList;
use crate::plugins::settings::options::GameOptions;
use crate::plugins::textdata::ClientUiStrings;
use crate::plugins::ui_v2::style::{ButtonSound, ImageButtonStyle, TargetColor};
use crate::plugins::ui_v2::widgets::{image_button, label};

use super::assets::IntroV2Assets;
use super::login_form::{main_button_style, ShardNameText};
use super::{intro_font_px, IntroV2State};

/// Root marker of the server selection screen.
#[derive(Component, Default, Clone)]
pub struct ServerSelectRoot;

/// Marker on the window image node; the dynamic shard rows are spawned as
/// its children.
#[derive(Component, Default, Clone)]
pub struct ServerWindow;

/// One row per shard, carrying the shard id.
#[derive(Component, Default, Clone)]
pub struct ShardRow(pub u16);

/// The shard **id** the list currently highlights (not yet committed), kept on
/// the [`ServerWindow`] entity.
///
/// # The idea
///
/// This used to be a `SelectedRow` marker on the row *entity*, and that broke
/// the moment the list learned to scroll: [`update_shard_rows`] despawns and
/// respawns every row, so scrolling silently dropped the highlight — and since
/// `Select` reads the highlight, the player was then one invisible step away
/// from a Connect that does nothing. Keying the highlight by shard id makes it
/// survive any number of row rebuilds, because the id is the only part of a row
/// that is stable across them.
///
/// It lives as a component on the window rather than as a `Resource` so it is
/// created and destroyed with the window itself (no registration, no stale
/// highlight surviving into the next visit of the screen).
#[derive(Component, Default, Clone)]
pub struct HighlightedShard(pub Option<u16>);

#[derive(Component, Default, Clone)]
pub struct SelectButton;

#[derive(Component, Default, Clone)]
pub struct CancelButton;

/// The shard id committed via the Select button; read by the Connect flow.
#[derive(Resource, Default)]
pub struct SelectedShardV2(pub Option<u16>);

/// Marker on the slider's draggable thumb, so it can be moved with the scroll
/// offset instead of sitting under the up arrow forever.
#[derive(Component, Default, Clone)]
pub struct SliderThumb;

/// Index of the first shard row the list shows. Idea: the list rect only fits
/// [`VISIBLE_ROWS`] rows, so with a longer shard list the rows past that were
/// clipped away by `Overflow::clip()` and the three slider buttons did nothing.
/// This is the one piece of list state the original keeps in its `CIFOListCtrl`;
/// we keep it beside the widgets because the rows are respawned, not scrolled.
#[derive(Resource, Default)]
pub struct ShardListScroll(pub usize);

// ---------------------------------------------------------------------------
// Geometry. Idea: this whole window is authored in `pstitle_europe.txt` and the
// rects below are that file, verbatim, not a redrawing of it. They are native
// pixels — a window rect in the intro trees is NOT in the 1600x1200 design
// space the two chrome bars use, so it does not scale with the screen. Every
// number is cross-checked against how the original draws this window, and where
// the check is exact that is noted.

/// `GDR_STA_SERVERWINDOW` (`pstitle_europe.txt:520`, id 10, `server_window.ddj`)
/// `Rect="0,0,240,340"`. On screen the visible gold frame is 238x338 at
/// (281,126), i.e. the art carries a 1px transparent margin per side. The
/// previous height of 300 was 40px short and had no source.
const WINDOW_W: f32 = 240.0;
const WINDOW_H: f32 = 340.0;

/// `GDR_LIST_SERVER` (`:162`, id 51, `CIFOListCtrl`) `Rect="7,32,200,300"`.
/// **Exact on the pixel path**: window art left edge 280 + 7 = 287, and the
/// original's selection bar spans x 287..486 — 200px, to the pixel. The row width
/// was 204 before, and the 5px margins that produced it were invented.
const LIST_X: f32 = 7.0;
const LIST_Y: f32 = 32.0;
const LIST_W: f32 = 200.0;
const LIST_H: f32 = 300.0;
/// Row height, as the original draws it (selection bar rows 157..176);
/// `LIST_H / ROW_H` = 15 visible rows, which is why the list rect is 300 and not
/// a round 320.
const ROW_H: f32 = 20.0;
/// How many rows fit at once — **derived from the list rect, not picked**:
/// `LIST_H / ROW_H` = 300 / 20 = 15, i.e. exactly 15 adjacent 20px rows. Kept as
/// the division so the two rects stay the
/// single source; [`tests::visible_rows_is_the_list_rect_divided_by_the_row`]
/// pins the value.
const VISIBLE_ROWS: usize = (LIST_H / ROW_H) as usize;

/// `GDR_SLI_SERVER` (`:143`, id 54, `CIFSliderCtrl`) `Rect="214,28,20,306"`.
/// Confirmed on the pixel path: the up arrow's top edge lands at 126+28 = 154
/// (~153 on screen) and the down arrow's at 28+306-20 = 314 -> 440 (~439).
/// `WINDOW_W - (214 + 20)` = 6 is where the existing `right: px(6)` came
/// from and it was right; the vertical offsets 25/45/275 were not.
const SLIDER_Y: f32 = 28.0;
const SLIDER_H: f32 = 306.0;
const SLIDER_BTN: f32 = 20.0;
const SLIDER_RIGHT: f32 = WINDOW_W - (214.0 + SLIDER_BTN);

/// `GDR_BTN_SACCEPT` / `GDR_BTN_SCANCEL` (`:539` / `:558`) are `Rect="0,0,91,40"`.
/// Note the **40**: only `GDR_BTN_OK` / `GDR_BTN_CANCEL` on the login screen
/// (`:482` / `:463`) are 91x41. Both pairs use `button_europe.ddj`.
const LIST_BUTTON_W: f32 = 91.0;
const LIST_BUTTON_H: f32 = 40.0;
/// Gap between Select and Cancel, and it is the **same pair value** as
/// Connect/Exit on the login screen (`login_form::BUTTON_GAP`, 18 px from the
/// blue fills of `button_europe.ddj` at x 309..381 / 418..491 on an 800x600
/// client). Carried over: with the pair centred on the screen it puts Select at
/// `(800 - (2*91 + 18)) / 2 = 300`, against the **301** the original draws — one
/// pixel. What it replaces is `SpaceEvenly` over a 288-wide row, which spread the
/// two ~35 px apart: a number nothing authored.
const LIST_BUTTON_GAP: f32 = 18.0;

/// Where the server window's top edge sits, as a fraction of the client height.
///
/// **Taken from the original, like the login window's 51 %** (`login_form`
/// `WINDOW_TOP_PERCENT`): it draws the 240x340 frame at **(280,125)** in an
/// 800x600 client. We drew it at (280,110) — 15 px too high — because the
/// window was *centred*
/// vertically, and nothing in `pstitle_europe.txt` says it is: `Rect="0,0,..."`
/// (`:529`) authors the size and leaves the origin to the code.
///
/// Horizontally there *is* a rule and it is computed, not written down:
/// `(800 - 240) / 2 = 280` is exactly the original's x, so the window is centred
/// (`align_items: Center` below).
///
/// The Select/Cancel row keeps its 10 px below the frame
/// (125 + 340 + 10 = 475 = the original's Select top), so fixing this one number
/// puts the button row right as well.
const WINDOW_TOP_PERCENT: f32 = 100.0 * 125.0 / 600.0;

fn slider_style(
    normal: &Handle<Image>,
    hover: &Handle<Image>,
    press: &Handle<Image>,
) -> ImageButtonStyle {
    ImageButtonStyle {
        normal: normal.clone(),
        hover: hover.clone(),
        press: press.clone(),
        ..Default::default()
    }
}

/// The two button captions come from the resinfo `Text=` keys of the controls
/// they implement: `GDR_BTN_SACCEPT` carries `UIO_CTL_SELECT` and
/// `GDR_BTN_SCANCEL` carries `UIO_COMMON_CTL_CANCEL` (`pstitle_europe.txt`
/// `:539`/`:558`). The fallbacks are the English rows the data ships
/// (textuisystem.txt line 154 "Select", line 160 "Cancel"), so an unloaded table
/// renders the same words.
const SELECT_KEY: &str = "UIO_CTL_SELECT";
const CANCEL_KEY: &str = "UIO_COMMON_CTL_CANCEL";

pub fn server_window(
    assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
) -> impl Scene {
    let select_label = ui_strings.get_or(SELECT_KEY, "Select").to_string();
    let cancel_label = ui_strings.get_or(CANCEL_KEY, "Cancel").to_string();
    let window = assets.server_list_window.clone();
    let font = fonts.nine.clone();
    // `GDR_BTN_SACCEPT`/`GDR_BTN_SCANCEL` (`pstitle_europe.txt:539,558`) are
    // `FontIndex=2` -> 16 px on the ladder (`intro_font_px`).
    let button_px = intro_font_px(2);
    // Both buttons get `SND_BUTTON_CLICK` and nothing else. They used to spawn
    // `uiwinclose.wav` on top of it, which has no basis: `resinfo/effectsound.txt`
    // maps `SND_BUTTON_CLICK` -> `ui\uibutton_a.wav`/`uibutton_b.wav` (`:52`/`:53`)
    // and `SND_WINDOW_CLOSE` -> `ui\uiwinclose.wav` (`:55`), while the original
    // uses `SND_WINDOW_CLOSE` only inside its `IFxxx` HUD windows — **never in
    // the pregame**. The server list is a `CIFStatic`
    // plate, not a closable window, so the click sound alone is the faithful
    // reading. (Paths are written lowercase deliberately: the table says
    // `Error.wav` with a capital E while Data.pk2 ships `error.wav`; our archive
    // index is case-insensitive, `bevy_pk2/src/pk2/archive.rs:66`.)
    let button_sound = assets.sound_button_sound_a.clone();
    let up = slider_style(
        &assets.server_list_slider_button_up,
        &assets.server_list_slider_button_up_focus,
        &assets.server_list_slider_button_up_press,
    );
    let mov = slider_style(
        &assets.server_list_slider_button_mov,
        &assets.server_list_slider_button_mov_focus,
        &assets.server_list_slider_button_mov_press,
    );
    let down = slider_style(
        &assets.server_list_slider_button_down,
        &assets.server_list_slider_button_down_focus,
        &assets.server_list_slider_button_down_press,
    );

    bsn! {
        ServerSelectRoot
        Name("Server Selection V2")
        Node {
            position_type: PositionType::Absolute,
            // The frame is **not** vertically centred — see
            // `WINDOW_TOP_PERCENT`. Horizontally it is, and that half stays an
            // alignment rather than a number.
            top: percent(WINDOW_TOP_PERCENT),
            align_items: AlignItems::Center,
            flex_direction: FlexDirection::Column,
            width: percent(100),
            height: percent(100),
        }
        Visibility::Hidden
        Children [
            (
                ServerWindow
                // The highlight is window state keyed by shard id, so scrolling
                // (which respawns the rows) cannot drop it.
                HighlightedShard
                ImageNode { image: {window}, color: Color::NONE, image_mode: NodeImageMode::Stretch }
                Node {
                    flex_direction: FlexDirection::Column,
                    align_self: AlignSelf::Center,
                    justify_content: JustifyContent::FlexStart,
                    width: px(WINDOW_W),
                    height: px(WINDOW_H),
                    overflow: Overflow::clip(),
                }
                Children [
                    (
                        image_button(up, SLIDER_BTN, SLIDER_BTN)
                        ImageNode { color: Color::NONE }
                        Node { position_type: PositionType::Absolute, top: px(SLIDER_Y), right: px(SLIDER_RIGHT), align_self: AlignSelf::FlexEnd }
                        on(|_activate: On<Activate>, mut scroll: ResMut<ShardListScroll>| {
                            scroll.0 = scroll.0.saturating_sub(1);
                        })
                    ),
                    (
                        // Start position; `update_slider_thumb` moves it with
                        // the scroll offset. Resting place with a list that does
                        // not scroll is directly under the up arrow, as in the
                        // original.
                        SliderThumb
                        image_button(mov, SLIDER_BTN, SLIDER_BTN)
                        ImageNode { color: Color::NONE }
                        Node { position_type: PositionType::Absolute, top: px(SLIDER_Y + SLIDER_BTN), right: px(SLIDER_RIGHT), align_self: AlignSelf::FlexEnd }
                    ),
                    (
                        image_button(down, SLIDER_BTN, SLIDER_BTN)
                        ImageNode { color: Color::NONE }
                        Node { position_type: PositionType::Absolute, top: px(SLIDER_Y + SLIDER_H - SLIDER_BTN), right: px(SLIDER_RIGHT), align_self: AlignSelf::FlexEnd }
                        on(|_activate: On<Activate>,
                            shard_list: Option<Res<ShardList>>,
                            mut scroll: ResMut<ShardListScroll>| {
                            let rows = shard_list.map_or(0, |list| list.0.shards.len());
                            scroll.0 = (scroll.0 + 1).min(max_scroll(rows));
                        })
                    ),
                ]
            ),
            // Select / Cancel button row
            (
                Node {
                    width: percent(100),
                    height: {px(LIST_BUTTON_H)},
                    align_self: AlignSelf::Center,
                    justify_content: JustifyContent::Center,
                    column_gap: {px(LIST_BUTTON_GAP)},
                    // the original sits the row 10 px under the frame
                    top: px(10),
                }
                Children [
                    (
                        image_button(main_button_style(assets), LIST_BUTTON_W, LIST_BUTTON_H)
                        SelectButton
                        ImageNode { color: Color::NONE }
                        ButtonSound({button_sound.clone()})
                        Children [ (label(select_label.as_str(), font.clone(), button_px)) ]
                        on(|_activate: On<Activate>,
                            highlighted: Query<&HighlightedShard, With<ServerWindow>>,
                            shard_list: Option<Res<ShardList>>,
                            mut selected_shard: ResMut<SelectedShardV2>,
                            mut options: ResMut<GameOptions>,
                            mut next_state: ResMut<NextState<IntroV2State>>| {
                            let Some(shard_id) = highlighted.iter().find_map(|h| h.0) else {
                                return;
                            };
                            selected_shard.0 = Some(shard_id);
                            // This is the one place the original's
                            // `RECENTSERVER` is written: the *committed* shard's
                            // name, persisted through the existing settings file
                            // by `settings::persistence::save_on_change`.
                            // Written without a let-chain on purpose: this crate
                            // still compiles as Rust 2021, where `if let ... && ...`
                            // is not accepted.
                            if let Some(name) = shard_list.and_then(|list| {
                                list.0
                                    .shards
                                    .iter()
                                    .find(|shard| shard.id == shard_id)
                                    .map(|shard| shard.name.clone())
                            }) {
                                if options.login.recent_server != name {
                                    options.login.recent_server = name;
                                }
                            }
                            next_state.set(IntroV2State::LoginForm);
                        })
                    ),
                    (
                        image_button(main_button_style(assets), LIST_BUTTON_W, LIST_BUTTON_H)
                        CancelButton
                        ImageNode { color: Color::NONE }
                        ButtonSound({button_sound})
                        Children [ (label(cancel_label.as_str(), font, button_px)) ]
                        on(|_activate: On<Activate>,
                            mut next_state: ResMut<NextState<IntroV2State>>| {
                            next_state.set(IntroV2State::LoginForm);
                        })
                    ),
                ]
            ),
        ]
    }
}

/// The shard-load vocabulary and its cut-offs, both from
/// `textdata/textuisystem.txt`. Idea: the original does not compute these
/// words, it looks them up — and the table ships the thresholds next to the
/// words, which is why nothing here is a hand-picked fraction any more. The
/// words are read through [`ClientUiStrings`] as well now, not just cited: the
/// fallbacks below are the English rows this PK2 happens to ship, so a
/// non-English table localizes the cells instead of pinning them to English.
///
/// ```text
/// UIO_STT_EXCELLENT       -> "Easy"      (:224)
/// UIO_STT_CROWDEDNESS     -> "Populated" (:225)
/// UIO_STT_SERVER_FULL     -> "Crowded"   (:226)
/// UIO_STT_SERVER_MAX_FULL -> "FULL"      (:227)
/// UIO_STT_SERVER_TEST     -> "Check"     (:228)
/// UIO_STT_SERVER_35       -> "35"        (:231) <- lower threshold, percent
/// UIO_STT_SERVER_75       -> "97"        (:232) <- upper threshold, percent
/// ```
///
/// The two threshold rows are keyed on the original Korean defaults (35/75) but
/// carry the values this build actually ships, **35 and 97**. Reading the value
/// rather than the key name is the whole point of them being data.
///
/// `UIO_STT_TEST` ("Closed", `:223`) is deliberately **not** used: nothing on
/// the wire distinguishes "closed" from "not operating", and the documented
/// mapping for a non-operating shard is `UIO_STT_SERVER_TEST` ("Check").
const STATUS_EASY_KEY: &str = "UIO_STT_EXCELLENT";
const STATUS_POPULATED_KEY: &str = "UIO_STT_CROWDEDNESS";
const STATUS_CROWDED_KEY: &str = "UIO_STT_SERVER_FULL";
const STATUS_FULL_KEY: &str = "UIO_STT_SERVER_MAX_FULL";
const STATUS_CHECK_KEY: &str = "UIO_STT_SERVER_TEST";
const THRESHOLD_LOW_KEY: &str = "UIO_STT_SERVER_35";
const THRESHOLD_HIGH_KEY: &str = "UIO_STT_SERVER_75";

/// Fallback percentages, the values this PK2's own rows carry.
const SHARD_LOAD_EASY_MAX_PCT: f32 = 35.0;
const SHARD_LOAD_POPULATED_MAX_PCT: f32 = 97.0;

/// The two threshold percentages, read out of the table rather than compiled in
/// — the whole point of them being data is that this operator retuned the
/// second one from 75 to 97 without touching a binary.
fn load_thresholds(ui_strings: &ClientUiStrings) -> (f32, f32) {
    let read = |key: &str, fallback: f32| {
        ui_strings
            .get(key)
            .and_then(|value| value.trim().parse::<f32>().ok())
            .unwrap_or(fallback)
    };
    let low = read(THRESHOLD_LOW_KEY, SHARD_LOAD_EASY_MAX_PCT);
    let high = read(THRESHOLD_HIGH_KEY, SHARD_LOAD_POPULATED_MAX_PCT);
    // a table with the rows swapped must not invert the ladder
    if low <= high {
        (low, high)
    } else {
        (high, low)
    }
}

/// The colour the original paints the "Easy" cell in: RGB(109,255,239). The
/// intro renders text without antialiasing, so it is that one colour exactly. It
/// has no `resinfo` origin — nothing in the `resinfo/` tree carries
/// `109,255,239`, while `255,255,103,29` does sit in `pstitle_europe.txt` — so
/// the colour is code-side in the original too.
/// `css::AQUAMARINE` (0x7FFFD4), which stood here before, is a different colour.
const STATUS_EASY: Srgba = Srgba::new(109.0 / 255.0, 1.0, 239.0 / 255.0, 1.0);
/// **openroad choice.** The original's colours for the loaded states are
/// unknown. These keep the traffic-light reading the screen already had — the
/// middle bucket keeps the yellow it already rendered, and the newly reachable
/// "Crowded" step sits between it and the red of a shard at capacity. Replace
/// them once the original's own colours for a busy shard are known.
const STATUS_POPULATED: Srgba = bevy::color::palettes::css::YELLOW;
const STATUS_CROWDED: Srgba = bevy::color::palettes::css::ORANGE;
const STATUS_FULL: Srgba = bevy::color::palettes::css::RED;
const STATUS_CHECK: Srgba = bevy::color::palettes::css::GRAY;

/// Maps a shard to its status cell. Split out of [`shard_row`] so the ladder is
/// testable without spawning a UI tree.
///
/// # The idea
///
/// The original looks the word up, it does not compute it: four load words plus
/// two threshold percentages sit next to each other in `textuisystem.txt`, so
/// both the text and the cut-offs come from the table here.
///
/// **Which word sits in which bucket is a reading, not a fact**: the table gives
/// two cut-offs and four words, and the original's loaded states are unknown.
/// The reading below is the
/// one that uses each word exactly once and matches the key names — `<= 35%`
/// Easy, `<= 97%` Populated, above it Crowded, and FULL only at capacity, where
/// no further login fits. What it replaces was worse than speculative: it
/// labelled everything from 35% to 97% "Crowded", never emitted "Populated" at
/// all, and showed "FULL" for a shard at 98% that still had room.
fn shard_status(shard: &Shard, ui_strings: &ClientUiStrings) -> (Srgba, String) {
    let check = || {
        (
            STATUS_CHECK,
            ui_strings.get_or(STATUS_CHECK_KEY, "Check").to_string(),
        )
    };
    if !shard.is_operating {
        // `UIO_STT_SERVER_TEST` -> "Check". "Maintenance", which stood here
        // before, appears nowhere in `textuisystem.txt`; positive control on
        // the same read path: "Crowded" is found, as `UIO_STT_SERVER_FULL`.
        return check();
    }
    // capacity 0 would be a divide-by-zero and is not a load state
    if shard.capacity == 0 {
        return check();
    }
    let (easy_max, populated_max) = load_thresholds(ui_strings);
    let load_pct = 100.0 * shard.online_count as f32 / shard.capacity as f32;
    if shard.online_count >= shard.capacity {
        return (
            STATUS_FULL,
            ui_strings.get_or(STATUS_FULL_KEY, "FULL").to_string(),
        );
    }
    if load_pct <= easy_max {
        (
            STATUS_EASY,
            ui_strings.get_or(STATUS_EASY_KEY, "Easy").to_string(),
        )
    } else if load_pct <= populated_max {
        (
            STATUS_POPULATED,
            ui_strings
                .get_or(STATUS_POPULATED_KEY, "Populated")
                .to_string(),
        )
    } else {
        (
            STATUS_CROWDED,
            ui_strings.get_or(STATUS_CROWDED_KEY, "Crowded").to_string(),
        )
    }
}

fn shard_row(
    shard: &Shard,
    _assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
) -> impl Scene {
    let shard_id = shard.id;
    let name = shard.name.clone();
    let font = fonts.nine.clone();
    let status_font = fonts.nine.clone();
    // The rows are the list control's own text: `GDR_LIST_SERVER`
    // (`pstitle_europe.txt:162`) is `FontIndex=0` -> 12 px.
    let row_px = intro_font_px(0);

    let (status_color, status_text) = shard_status(shard, ui_strings);

    bsn! {
        bevy::ui_widgets::Button
        Hovered
        ShardRow({shard_id})
        ImageNode { image: {TRANSPARENT_IMAGE_HANDLE}, color: Color::NONE, image_mode: NodeImageMode::Stretch }
        Node {
            // `GDR_LIST_SERVER Rect="7,32,200,300"` — the list rect already
            // carries the inset, so the row is 200 wide at x=7, not 204 at
            // x=7+5. The original's selection bar spans x 287..486 with the
            // window art at 280, i.e. exactly 7..207.
            width: px(LIST_W),
            height: px(ROW_H),
            margin: {UiRect::left(Val::Px(LIST_X))},
            top: px(LIST_Y),
            padding: {UiRect::horizontal(Val::Px(5.0))},
            justify_content: JustifyContent::SpaceBetween,
            flex_direction: FlexDirection::Row,
            position_type: PositionType::Relative,
        }
        BackgroundColor(Color::NONE)
        Children [
            (
                label(name.as_str(), font, row_px)
                Node { justify_content: JustifyContent::FlexStart, align_self: AlignSelf::Center, max_width: px(120) }
            ),
            (
                label(status_text.as_str(), status_font, row_px)
                TargetColor({status_color})
                Node { justify_content: JustifyContent::FlexEnd, align_self: AlignSelf::Center, max_width: px(60) }
            ),
        ]
        on(move |_activate: On<Activate>,
            mut window: Query<&mut HighlightedShard, With<ServerWindow>>| {
            // Highlight by id, not by entity: the rows are respawned on every
            // scroll step (see `HighlightedShard`).
            if let Ok(mut highlighted) = window.single_mut() {
                highlighted.0 = Some(shard_id);
            }
        })
    }
}

/// Run condition of [`update_shard_rows`]: the rows must (re)build when the
/// shard list changes — or when the server window itself has just spawned.
/// The gateway can deliver the list during scene loading, before the chrome
/// exists; with a plain `resource_exists_and_changed` gate that change tick
/// is consumed against a missing window and the server list stays empty: the
/// response can arrive ~0.1 s after connect, while the scene is still in Splash.
pub fn shard_rows_need_refresh(
    shard_list: Option<Res<ShardList>>,
    scroll: Res<ShardListScroll>,
    new_window: Query<(), Added<ServerWindow>>,
) -> bool {
    match shard_list {
        Some(list) => list.is_changed() || scroll.is_changed() || !new_window.is_empty(),
        None => false,
    }
}

/// Largest first-visible index that still fills the list: everything past it
/// would scroll empty rows into view.
fn max_scroll(row_count: usize) -> usize {
    row_count.saturating_sub(VISIBLE_ROWS)
}

/// (Re-)spawns one row per shard whenever [`shard_rows_need_refresh`] fires.
pub fn update_shard_rows(
    shard_list: Res<ShardList>,
    mut scroll: ResMut<ShardListScroll>,
    window_query: Query<Entity, With<ServerWindow>>,
    existing_rows: Query<Entity, With<ShardRow>>,
    assets: Res<IntroV2Assets>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    mut commands: Commands,
) {
    let Ok(window) = window_query.single() else {
        return;
    };

    for row in existing_rows.iter() {
        commands.entity(row).despawn();
    }

    // A shorter list (reconnect to a different gateway) must not leave the
    // offset pointing past its end.
    let ceiling = max_scroll(shard_list.0.shards.len());
    if scroll.0 > ceiling {
        scroll.0 = ceiling;
    }

    let rows: Vec<_> = shard_list
        .0
        .shards
        .iter()
        .skip(scroll.0)
        .take(VISIBLE_ROWS)
        .map(|shard| shard_row(shard, &assets, &fonts, &ui_strings))
        .collect();
    commands
        .entity(window)
        .queue_spawn_related_scenes::<Children>(rows);
}

/// Swaps row art for hover/selection, mirroring the old
/// `on_server_list_item_selected` visuals.
pub fn update_shard_row_visuals(
    mut rows: Query<(&Hovered, &ShardRow, &mut ImageNode, &mut BackgroundColor)>,
    highlighted: Query<&HighlightedShard, With<ServerWindow>>,
    assets: Res<IntroV2Assets>,
) {
    let highlighted_id = highlighted.iter().find_map(|h| h.0);
    for (hovered, row, mut image, mut bg_color) in rows.iter_mut() {
        let selected = highlighted_id == Some(row.0);
        if selected {
            let target = &assets.server_list_item_select;
            if image.image != *target {
                image.image = target.clone();
            }
            bg_color.0 = Color::NONE;
        } else if hovered.get() {
            let target = &assets.server_list_item_hover;
            if image.image != *target {
                image.image = target.clone();
            }
            bg_color.0 = Color::NONE;
        } else {
            if image.image != TRANSPARENT_IMAGE_HANDLE {
                image.image = TRANSPARENT_IMAGE_HANDLE;
            }
            bg_color.0 = Color::NONE;
        }
    }
}

/// Commits the remembered `RECENTSERVER` as the selection, once the shard list
/// names it.
///
/// # The idea
///
/// **Stated deviation with a rationale, and it replaces a real trap rather than
/// an unknown.** The original prefills the Server row from `RECENTSERVER` while
/// its *selection* stays empty (see [`update_shard_name_text`]), so its own
/// Connect click sends nothing and says nothing. openroad already answered that
/// click with a status line (`net::on_connect_activate`), and that is only half
/// a fix: the player reads a server name in the row, so a hint that says "select
/// a server first" reads as a lie about what is on screen.
///
/// The remembered name is therefore turned into a real selection here, which is
/// what the row has been claiming all along. Two guards keep it honest:
///
/// * it only ever fires while **nothing** is committed, so it cannot overrule
///   the player's own `Select`;
/// * it only commits a shard that is in the current list **and operating** — a
///   remembered shard that is gone or down is left uncommitted, and then the
///   status line is telling the truth again.
pub fn commit_remembered_shard(
    mut selected_shard: ResMut<SelectedShardV2>,
    shard_list: Res<ShardList>,
    options: Res<GameOptions>,
) {
    if selected_shard.0.is_some() {
        return;
    }
    let remembered = options.login.recent_server.trim();
    if remembered.is_empty() {
        return;
    }
    let Some(shard) = shard_list
        .0
        .shards
        .iter()
        .find(|shard| shard.is_operating && shard.name == remembered)
    else {
        return;
    };
    info!(
        "login form: pre-selecting the remembered server '{}' (shard {})",
        shard.name, shard.id
    );
    selected_shard.0 = Some(shard.id);
}

/// Shows the committed shard's name in the login form — and, before anything is
/// committed, the remembered `RECENTSERVER` name.
///
/// # The idea
///
/// The original prefills the Server row from `RECENTSERVER`
/// (`HKCU\Software\<vendor>\Silkroad`), and that carries a trap: the row can
/// read a server name while **no shard is selected**, so Connect sends nothing
/// at all.
///
/// This system therefore only ever writes *text*: it never selects anything, and
/// [`tests::a_remembered_name_is_displayed_without_becoming_a_selection`] pins
/// that. The *selection* side of the remembered name has its own owner,
/// [`commit_remembered_shard`], so on a normal start the two agree — but
/// keeping the two states separate here is what lets the display survive a
/// remembered shard that is down or gone (no commit, name still shown).
/// The shard list is `Option` because the name is shown on the login screen,
/// which the player reaches before any gateway response exists.
pub fn update_shard_name_text(
    selected_shard: Res<SelectedShardV2>,
    shard_list: Option<Res<ShardList>>,
    options: Res<GameOptions>,
    mut query: Query<&mut Text, With<ShardNameText>>,
) {
    let Ok(mut text) = query.single_mut() else {
        return;
    };

    let committed = selected_shard.0.and_then(|selected| {
        shard_list.as_ref().and_then(|list| {
            list.0
                .shards
                .iter()
                .find(|shard| shard.id == selected)
                .map(|shard| shard.name.clone())
        })
    });

    // No commit yet -> the remembered name, which is display only. An empty
    // `recent_server` (first run) leaves the row empty, as it was before.
    let wanted = match committed {
        Some(name) => name,
        None => options.login.recent_server.clone(),
    };

    if wanted.is_empty() || text.0 == wanted {
        return;
    }
    text.0 = wanted;
}

/// Wheel scrolling for the shard list.
///
/// # The idea
///
/// The two arrow buttons step one row per click; a list of 20+ shards needs
/// something faster, and the wheel is what a player will try. **Stated
/// deviation**: the original's wheel behaviour over `CIFOListCtrl` is unknown,
/// so this is ours. It
/// runs only while the server-selection screen is up, where this list is the
/// only scrollable surface, so it cannot steal the wheel from anything else.
pub fn scroll_shard_list_with_wheel(
    mut wheel: MessageReader<MouseWheel>,
    shard_list: Option<Res<ShardList>>,
    mut scroll: ResMut<ShardListScroll>,
) {
    let ceiling = max_scroll(shard_list.map_or(0, |list| list.0.shards.len()));
    for event in wheel.read() {
        // wheel up (positive y) moves towards the top of the list
        let steps = event.y.round() as i32;
        if steps == 0 {
            continue;
        }
        let next = scroll.0 as i32 - steps;
        scroll.0 = next.clamp(0, ceiling as i32) as usize;
    }
}

/// Moves the slider thumb so it reflects the scroll offset.
///
/// # The idea
///
/// The original's `CIFSliderCtrl` owns thumb, track and arrows as one control
/// (`GDR_SLI_SERVER` Rect `214,28,20,306`); we spawn its three parts as
/// buttons, so the thumb has to be positioned from the same state the rows are
/// built from. The travel is what is left of the track once the two arrows and
/// the thumb have taken their `SLIDER_BTN` each — no free constant.
pub fn update_slider_thumb(
    shard_list: Option<Res<ShardList>>,
    scroll: Res<ShardListScroll>,
    mut thumb: Query<&mut Node, With<SliderThumb>>,
) {
    let Ok(mut node) = thumb.single_mut() else {
        return;
    };
    let rows = shard_list.map_or(0, |list| list.0.shards.len());
    let ceiling = max_scroll(rows);
    let travel = SLIDER_H - 3.0 * SLIDER_BTN;
    let offset = if ceiling == 0 {
        0.0
    } else {
        travel * (scroll.0 as f32 / ceiling as f32)
    };
    let top = px(SLIDER_Y + SLIDER_BTN + offset);
    if node.top != top {
        node.top = top;
    }
}

/// Disables the Select button while no row is highlighted.
///
/// # The idea
///
/// Its `Activate` handler already returns early without a [`SelectedRow`], so
/// the button was clickable but inert — the original's own `button_disable.ddj`
/// state exists for exactly this (`ImageButtonStyle::disable` is filled by
/// [`main_button_style`], `assets.rs:38`). Marking it `InteractionDisabled`
/// makes the same rule visible instead of silent. Whether the original greys
/// this particular button is unknown, so this is a stated openroad affordance;
/// the shipped art it uses is the original's.
pub fn update_select_button_enabled(
    highlighted: Query<&HighlightedShard, With<ServerWindow>>,
    select_button: Query<(Entity, Has<InteractionDisabled>), With<SelectButton>>,
    mut commands: Commands,
) {
    let has_selection = highlighted.iter().any(|h| h.0.is_some());
    for (entity, disabled) in select_button.iter() {
        if has_selection && disabled {
            commands.entity(entity).remove::<InteractionDisabled>();
        } else if !has_selection && !disabled {
            commands.entity(entity).insert(InteractionDisabled);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frame's origin in the original is (280,125) in an 800x600 client, and
    /// we drew it at (280,110) — vertically centred, which nothing authors. x is
    /// the centring *rule*, y is the original's value.
    #[test]
    fn the_server_window_sits_where_the_original_does() {
        let (client_w, client_h) = (800.0, 600.0);
        let left = (client_w - WINDOW_W) / 2.0;
        let top = client_h * WINDOW_TOP_PERCENT / 100.0;
        assert_eq!((left, top), (280.0, 125.0));
        // The frame is deliberately **not** vertically centred: centring gives
        // 130 here, and the original draws 125. (What we actually drew was 110,
        // because the old markup centred the window *and* its button row as one
        // column, so the frame rose by half the row. Neither 130 nor 110 is the
        // original's value, which is why this number is a named constant.)
        let centred = (client_h - WINDOW_H) / 2.0;
        assert_ne!(top, centred);
        assert_eq!(centred, 130.0);
    }

    /// Select's top follows from the frame — the row keeps its 10 px under it,
    /// so 125 + 340 + 10 = 475 is the original's value without a second number
    /// for it.
    #[test]
    fn the_button_row_follows_the_frame() {
        let top = 600.0 * WINDOW_TOP_PERCENT / 100.0 + WINDOW_H + 10.0;
        assert_eq!(top, 475.0);
    }

    /// The other axis: the pair is centred on the screen with the same 18 px gap
    /// as Connect/Exit, which puts Select at 300 against the original's 301.
    /// `SpaceEvenly` over the old 288-wide row is the red control: ~35 px, and
    /// Select 10 px to the left of where the original draws it.
    #[test]
    fn select_and_cancel_are_centred_18px_apart() {
        let pair = 2.0 * LIST_BUTTON_W + LIST_BUTTON_GAP;
        let select_left = (800.0 - pair) / 2.0;
        assert_eq!(select_left, 300.0);
        assert!((select_left - 301.0).abs() <= 1.0);
        let old_row_w = 288.0;
        let space_evenly_gap = (old_row_w - 2.0 * LIST_BUTTON_W) / 3.0;
        let old_select_left = (800.0 - old_row_w) / 2.0 + space_evenly_gap;
        // 291.33 against the 291 we drew — the old layout explains the defect
        // to within a third of a pixel, which is what makes `SpaceEvenly` the
        // identified cause and not a guess.
        assert!((old_select_left - 291.0).abs() < 0.5);
        assert!(space_evenly_gap > 34.0 && space_evenly_gap < 36.0);
    }

    /// The 1 px the original does *not* share: this window's buttons are
    /// `Rect="0,0,91,40"` (`pstitle_europe.txt:548`, `:567`) while the login
    /// screen's are `0,0,91,41` (`:491`). Pinned so nobody tidies them into one
    /// constant.
    #[test]
    fn the_list_buttons_are_a_pixel_shorter_than_the_login_buttons() {
        assert_eq!((LIST_BUTTON_W, LIST_BUTTON_H), (91.0, 40.0));
        assert_eq!(LIST_BUTTON_H + 1.0, 41.0);
    }

    /// The row count is the list rect divided by the row height, not a
    /// hand-picked 15 — this is the assertion that keeps it that way.
    #[test]
    fn visible_rows_is_the_list_rect_divided_by_the_row() {
        assert_eq!(VISIBLE_ROWS as f32 * ROW_H, LIST_H);
    }

    #[test]
    fn a_list_that_fits_does_not_scroll() {
        assert_eq!(max_scroll(0), 0);
        assert_eq!(max_scroll(VISIBLE_ROWS), 0);
    }

    #[test]
    fn a_longer_list_stops_with_a_full_last_page() {
        assert_eq!(max_scroll(VISIBLE_ROWS + 1), 1);
        assert_eq!(max_scroll(VISIBLE_ROWS + 7), 7);
    }

    fn shard(id: u16, name: &str) -> Shard {
        Shard {
            id,
            name: name.to_string(),
            online_count: 0,
            capacity: 100,
            is_operating: true,
            farm_id: 1,
        }
    }

    /// The measured original trap (order `031`): the Server row *shows*
    /// `RECENTSERVER` while nothing is selected, and Connect then sends no
    /// `0x6102` at all. Our prefill must reproduce the display and **not** the
    /// selection — `SelectedShardV2` stays `None`, which is the single condition
    /// `net::on_connect_activate` returns on before it builds a request.
    #[test]
    fn a_remembered_name_is_displayed_without_becoming_a_selection() {
        let mut app = App::new();
        app.init_resource::<SelectedShardV2>();
        let mut options = GameOptions::default();
        options.login.recent_server = "Testserver".to_string();
        app.insert_resource(options);
        app.insert_resource(ShardList(packets::gateway::ShardListResponse {
            farms: Vec::new(),
            shards: vec![shard(42, "Testserver")],
        }));
        let text = app.world_mut().spawn((Text::default(), ShardNameText)).id();
        app.add_systems(Update, update_shard_name_text);
        app.update();

        assert_eq!(
            app.world().get::<Text>(text).expect("text").0,
            "Testserver",
            "the remembered name is shown"
        );
        assert!(
            app.world().resource::<SelectedShardV2>().0.is_none(),
            "showing a name must never commit a shard - that is the measured \
             original behaviour and what makes Connect inert"
        );
    }

    /// The playtest report of 2026-08-25: "select a server first" appeared while
    /// the Server row named a server. [`commit_remembered_shard`] closes that gap
    /// by turning the remembered name into the selection it looks like — and the
    /// two negative controls are the point of the test: a shard that is **down**
    /// and a name that is **not in the list** stay uncommitted, so the hint is
    /// only ever shown when it is true.
    #[test]
    fn the_remembered_server_becomes_the_selection_when_the_list_offers_it() {
        fn app_with(remembered: &str, listed: Vec<Shard>) -> App {
            let mut app = App::new();
            app.init_resource::<SelectedShardV2>();
            let mut options = GameOptions::default();
            options.login.recent_server = remembered.to_string();
            app.insert_resource(options);
            app.insert_resource(ShardList(packets::gateway::ShardListResponse {
                farms: Vec::new(),
                shards: listed,
            }));
            app.add_systems(Update, commit_remembered_shard);
            app.update();
            app
        }

        let app = app_with("TestShard", vec![shard(42, "TestShard")]);
        assert_eq!(app.world().resource::<SelectedShardV2>().0, Some(42));

        let mut down = shard(42, "TestShard");
        down.is_operating = false;
        let app = app_with("TestShard", vec![down]);
        assert_eq!(
            app.world().resource::<SelectedShardV2>().0,
            None,
            "a remembered shard that is not operating must not be committed"
        );

        let app = app_with("Gone", vec![shard(42, "TestShard")]);
        assert_eq!(
            app.world().resource::<SelectedShardV2>().0,
            None,
            "a remembered name the list does not offer must not be committed"
        );

        let app = app_with("", vec![shard(42, "TestShard")]);
        assert_eq!(
            app.world().resource::<SelectedShardV2>().0,
            None,
            "a first run (no RECENTSERVER) selects nothing"
        );
    }

    /// The player's own `Select` wins: the prefill only ever fills an empty
    /// selection, so it can never drag a session back to the remembered shard.
    #[test]
    fn the_prefill_never_overrules_a_committed_shard() {
        let mut app = App::new();
        app.insert_resource(SelectedShardV2(Some(7)));
        let mut options = GameOptions::default();
        options.login.recent_server = "Testserver".to_string();
        app.insert_resource(options);
        app.insert_resource(ShardList(packets::gateway::ShardListResponse {
            farms: Vec::new(),
            shards: vec![shard(42, "Testserver")],
        }));
        app.add_systems(Update, commit_remembered_shard);
        app.update();

        assert_eq!(app.world().resource::<SelectedShardV2>().0, Some(7));
    }

    /// Point of [`HighlightedShard`]: a highlight keyed by shard id survives the
    /// row rebuild that every scroll step performs. With the old entity marker
    /// the selection was silently gone after scrolling.
    #[test]
    fn the_highlight_survives_a_row_rebuild() {
        let mut app = App::new();
        let window = app
            .world_mut()
            .spawn((ServerWindow, HighlightedShard(Some(7))))
            .id();
        // Stand-in for the respawn: the rows are children, the highlight is not.
        let row = app.world_mut().spawn(ShardRow(7)).id();
        app.world_mut().entity_mut(row).despawn();
        let _new_row = app.world_mut().spawn(ShardRow(7)).id();

        assert_eq!(
            app.world()
                .get::<HighlightedShard>(window)
                .expect("window state")
                .0,
            Some(7)
        );
    }
}
