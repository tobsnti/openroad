use bevy::app::AppExit;
use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};
use bevy::input_focus::{FocusCause, InputFocus};
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::Activate;

use crate::assets::FontAssets;
use crate::plugins::settings::options::GameOptions;
use crate::plugins::textdata::ClientUiStrings;
use crate::plugins::ui_v2::style::{ButtonSound, ImageButtonStyle};
use crate::plugins::ui_v2::widgets::{
    image_button, label, label_aligned, password_input, text_input_justified,
};

use crate::scenes::loading_screen::{design_pct, DesignFit};

use super::assets::IntroV2Assets;
use super::fade::FadingIn;
use super::{intro_font_px, IntroV2State};

/// Root marker of the login form screen.
#[derive(Component, Default, Clone)]
pub struct LoginFormRoot;

/// Marker on the username `EditableText`.
#[derive(Component, Default, Clone)]
pub struct IdInput;

/// Marker on the password `EditableText`.
#[derive(Component, Default, Clone)]
pub struct PwInput;

/// Marker on the text showing the currently selected shard's name.
#[derive(Component, Default, Clone)]
pub struct ShardNameText;

#[derive(Component, Default, Clone)]
pub struct ConnectButton;

#[derive(Component, Default, Clone)]
pub struct ExitButton;

#[derive(Component, Default, Clone)]
pub struct ServerListButton;

/// Style of the intro's main buttons (Connect, Start, ...). These are the two
/// buttons that actually go disabled today — `net.rs` disables Connect while a
/// login request is in flight, `character_select.rs` disables Start while the
/// join is pending — so this is the call site that has to carry `disable`.
pub fn main_button_style(assets: &IntroV2Assets) -> ImageButtonStyle {
    ImageButtonStyle {
        normal: assets.button.clone(),
        hover: assets.button_focus.clone(),
        press: assets.button_press.clone(),
        disable: assets.button_disable.clone(),
    }
}

/// Style of the Exit button. Separate from [`main_button_style`] because the
/// data makes it separate: in `Media/resinfo/pstitle_europe.txt`,
/// `GDR_BTN_CANCEL` (`:467`, `Text=UIO_CTL_EXIT`) is drawn from
/// `interface\outer\button.ddj`, while `GDR_BTN_OK` (`:486`,
/// `Text=UIO_CTL_CONNECT`) is drawn from `button_europe.ddj`. Exit really is the
/// one main button the original leaves un-`#ifdef`ed; both buttons here used to
/// wear the Connect art with no line saying why.
fn exit_button_style(assets: &IntroV2Assets) -> ImageButtonStyle {
    ImageButtonStyle {
        normal: assets.exit_button.clone(),
        hover: assets.exit_button_focus.clone(),
        press: assets.exit_button_press.clone(),
        // Same `button_disable.ddj` the europe style borrows — for *this* button
        // it is the matching frame rather than a substitute, both being 91x40.
        disable: assets.button_disable.clone(),
    }
}

/// The list button is the only button on this screen whose disabled frame ships
/// (`list_button_disable.ddj`, 48x24, next to `_focus`/`_press`), so the style
/// is complete. It has no disabled state *today* — nothing inserts
/// `InteractionDisabled` on [`ServerListButton`] — but the empty slot made
/// `ui_v2::update_image_button_visuals` fall back to the normal frame with a
/// `warn_once`, i.e. a shipped asset was going unused for no stated reason.
fn list_button_style(assets: &IntroV2Assets) -> ImageButtonStyle {
    ImageButtonStyle {
        normal: assets.list_button.clone(),
        hover: assets.list_button_focus.clone(),
        press: assets.list_button_press.clone(),
        disable: assets.list_button_disable.clone(),
    }
}

/// The textuisystem keys the original's `pstitle` blocks bind to this screen's
/// five captions: `GDR_STATIC1/2/3` carry `Text=UIO_CTL_ID` /
/// `UIO_CTL_PASSWORD` / `UIO_CTL_SELECT_SERVER`, `GDR_BTN_OK`/`GDR_BTN_CANCEL`
/// carry `UIO_CTL_CONNECT`/`UIO_CTL_EXIT`. The fallbacks are the rows the data
/// actually ships (`textdata/textuisystem.txt:149-153`), so an unloaded table
/// renders the same words instead of nothing.
const ID_KEY: &str = "UIO_CTL_ID";
const PASSWORD_KEY: &str = "UIO_CTL_PASSWORD";
const SELECT_SERVER_KEY: &str = "UIO_CTL_SELECT_SERVER";
const CONNECT_KEY: &str = "UIO_CTL_CONNECT";
const EXIT_KEY: &str = "UIO_CTL_EXIT";

// The two full-screen statics of this screen are **scaled**, not native:
// they carry a real rect in the title tree's 1600x1200 canvas, so they are
// drawn at `(x,y,w,h) * k`, `k = view / 1600` (`splash.rs` `BIG_LOGO_RECT`
// spells `k` out). The rects are
// taken verbatim and turned into percentages of the same `DesignFit` box
// the loading chrome uses, so neither `k` nor a pixel size is written here.
//
// `GDR_STA_LOGO` id 8, `Rect="540,403,508,172"`, `logo.ddj`
// (`pstitle_europe.txt:600/605`): at 800x600 that is **254x86 at
// (270,201)** where the art was drawn at its native 508x172 — twice too
// large. Its bottom, 201+86 = 287, lands 19 px above the window's top edge
// 306, which is the gap the previous hand-anchoring produced.
const LOGO_RECT: (f32, f32, f32, f32) = (540.0, 403.0, 508.0, 172.0);
// `GDR_STA_TITLE` id 5, `Rect="47,110,184,36"`, `text-connect.ddj`
// (`pstitle_europe.txt:657/662`): 92x18 at (23,55) on an 800x600 client.
// The original paints it in the top left of the login screen; we painted
// nothing.
const TITLE_RECT: (f32, f32, f32, f32) = (47.0, 110.0, 184.0, 36.0);

// The gap the original draws, replacing a layout rule that produced a number
// by itself. `GDR_BTN_OK`/`GDR_BTN_CANCEL` are `Rect="0,0,91,41"`
// (`pstitle_europe.txt:491`, `:472`) — a `0,0` rect is code-placed, so the
// data gives the size and the original gives the position: on its 800x600
// client the blue fill of `button_europe.ddj` sits at x 309..381 and 418..491,
// i.e. the two 91x41 buttons start at **300** and **409**, a gap of **18 px**,
// the pair spanning 300..500 around the screen centre. `SpaceEvenly` over the
// 288-wide frame spread them ~35 px apart instead — a number nothing authored.
// Centred with this gap the pair lands at 44..244 inside the frame, i.e.
// 300..500 on screen with the frame's left edge 256.
const BUTTON_GAP: f32 = 18.0;

/// `GDR_BTN_OK`/`GDR_BTN_CANCEL` `Rect="0,0,91,41"` (`pstitle_europe.txt:491`,
/// `:472`) and `GDR_STA_LOGINWINDOW` `Rect="0,0,288,140"` (`:586`): a `0,0`
/// rect is code-placed, so these three sizes are drawn **native**, unscaled, as
/// in the original, whose 288x140 frame sits at (256,306) in an 800x600
/// client.
const BUTTON_SIZE: (f32, f32) = (91.0, 41.0);
const WINDOW_SIZE: (f32, f32) = (288.0, 140.0);

/// Tab order of the screen, in the order the original stacks its controls top
/// to bottom: the two edit rows come from `widgets::{text_input,password_input}`
/// as 0 and 1, so the list opener and the two main buttons continue the run.
///
/// **Deliberate deviation with a stated rationale.** The original's tab ring has
/// **exactly two members, ID <-> PW**; a third Tab lands back in the ID field.
/// The Server row and the list button are *not* in the ring. We put
/// Server/Connect/Exit in it anyway, for keyboard reachability of the only two
/// actions this screen has (A11y). Nothing about the layout changes with it.
///
/// Exit is *not* in this list, and that is what stops the client closing itself
/// a few seconds after the window opens: `bevy_ui_widgets` fires `Activate` for
/// a **focused** button on Enter, and Exit sat exactly one Tab past Connect.
/// With Connect a silent no-op without a selected server, a player tabbing and
/// hitting Enter to get *any* reaction walked into the quit, and nothing was
/// logged, so the session looked like it had died on its own.
///
/// Dropping Exit out of the ring also moves *towards* the original, whose ring
/// has two members (ID <-> PW); the mouse path to Exit is untouched, and quitting
/// from the keyboard is still one Cmd+Q / window-close away.
const TAB_SERVER_LIST: i32 = 2;
const TAB_CONNECT: i32 = 3;

/// Opens the screen's Tab ring, and [`close_tab_ring`] closes it again.
///
/// # The idea
///
/// The `TabGroup` is **not** authored on the root any more; it is inserted while
/// the login form is the screen on show and removed when it stops being that.
/// This is the same defect class as "the client closes itself", one screen
/// further on: `hide_screen` (`fade.rs`) only fades and sets
/// `Visibility::Hidden`, and nothing despawns `LoginFormRoot`
/// (`rg -n "LoginFormRoot" client/src` — only `show_screen`/`hide_screen`).
/// Bevy 0.19 collects Tab targets **purely structurally**: `TabNavigation`
/// (`bevy_input_focus-0.19.1/src/tab_navigation.rs`) queries `TabGroup` plus the
/// `TabIndex` descendants of each group and filters on neither `Visibility` nor
/// `InteractionDisabled`, and with a non-modal focus it gathers *all* non-modal
/// groups. `bevy_ui_widgets-0.19.1/src/button.rs` then fires `Activate` for the
/// focused button on Enter **or** Space. So on the character list a player who
/// pressed Tab landed in the invisible login form — and one Enter on the
/// invisible list button threw the session back to `ServerSelection`.
///
/// Removing the group is the narrow fix (one component on one entity, and it
/// takes the whole ring with it, members included) rather than stripping and
/// restoring five `TabIndex`es. The captcha shows the other half of the same
/// mechanism working as intended: it uses `TabGroup::modal()`
/// (`captcha.rs`), which is what keeps *its* ring closed while it is up.
pub fn open_tab_ring(root: Query<Entity, With<LoginFormRoot>>, mut commands: Commands) {
    for entity in root.iter() {
        commands.entity(entity).insert(TabGroup::new(0));
    }
}

/// Closes the Tab ring when the login form leaves the screen — see
/// [`open_tab_ring`] for why this is not a `Visibility` question.
///
/// It also drops the input focus if it is still standing on one of this screen's
/// *buttons*: `bevy_ui_widgets` activates a **focused** button on Enter/Space no
/// matter which group it belongs to, so leaving the caret on the invisible
/// Connect button would keep exactly the hole this closes. The two edit rows
/// keep their focus on purpose — [`focus_id_input`] is written to leave the
/// caret where it was when the player comes back from the server window, and a
/// focused text field activates nothing.
pub fn close_tab_ring(
    root: Query<Entity, With<LoginFormRoot>>,
    buttons: Query<
        (),
        Or<(
            With<ConnectButton>,
            With<ExitButton>,
            With<ServerListButton>,
        )>,
    >,
    mut focus: ResMut<InputFocus>,
    mut commands: Commands,
) {
    for entity in root.iter() {
        commands.entity(entity).remove::<TabGroup>();
    }
    if focus.get().is_some_and(|focused| buttons.contains(focused)) {
        focus.clear();
    }
}

/// Whether the login form is the screen currently on show.
///
/// The two actions of this screen (`Activate` on the list button and on Connect)
/// are observers, and an observer has no `run_if`; both therefore ask this
/// before they change the state. Belt to [`close_tab_ring`]'s braces: even if a
/// focus or a synthetic `Activate` reaches one of the (never despawned) buttons
/// from another screen, it cannot move the state machine any more.
///
/// `IntroV2State` is a sub-state, so its `State` resource is absent whenever the
/// scene itself is not running — absent counts as "not on screen".
pub fn login_form_is_on_screen(state: Option<&State<IntroV2State>>) -> bool {
    matches!(state.map(State::get), Some(IntroV2State::LoginForm))
}

/// Whether this screen may act on a click *now*.
///
/// On top of [`login_form_is_on_screen`]: a root that still carries
/// [`FadingIn`] is on its way in, and a click landing on it in that half second
/// was aimed at the screen it replaces. The measured case is a double-click on
/// the server window's Select button, whose area overlaps Connect — see
/// [`super::fade::FadingIn`].
pub fn login_form_accepts_clicks(
    state: Option<&State<IntroV2State>>,
    fading_root: &Query<(), (With<LoginFormRoot>, With<FadingIn>)>,
) -> bool {
    login_form_is_on_screen(state) && fading_root.is_empty()
}

pub fn login_form(
    assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
) -> impl Scene {
    let id_label = ui_strings.get_or(ID_KEY, "ID").to_string();
    let password_label = ui_strings.get_or(PASSWORD_KEY, "PW").to_string();
    let server_label = ui_strings.get_or(SELECT_SERVER_KEY, "Server").to_string();
    let connect_label = ui_strings.get_or(CONNECT_KEY, "Connect").to_string();
    let exit_label = ui_strings.get_or(EXIT_KEY, "Exit").to_string();
    let logo = assets.logo.clone();
    let title_art = assets.text_connect.clone();
    let window = assets.login_window.clone();
    let font = fonts.nine.clone();
    // Text sizes are the resinfo `FontIndex` of the control each label stands
    // for, on the ladder in `intro_font_px` — nothing here is picked by eye.
    // The three row captions `GDR_STATIC1/2/3` (`pstitle_europe.txt:120,101,82`)
    // and both main buttons `GDR_BTN_OK`/`GDR_BTN_CANCEL` (`:482`, `:463`) are
    // `FontIndex=2` -> 16 px; the shard name is `GDR_STA_SERVER` (`:25`) with
    // `FontIndex=0` -> 12 px, which is also what the two `CIFEdit` rows use
    // (`:63`, `:44`, both index 0 — `widgets::text_input`'s 12 px).
    let caption_px = intro_font_px(2);
    let button_px = intro_font_px(2);
    let shard_px = intro_font_px(0);
    let button_sound = assets.sound_button_sound_a.clone();
    let window_open_sound = assets.sound_window_open.clone();

    // Vertical placement of the window: it follows the original, not a centring
    // rule. The original draws the 288x140 frame at `Y0=306` in an 800x600
    // client, i.e. at 51 % of the client height, and no literal in the data says
    // so. Vertical *centring* would put it at 230 — 76 px too high — which is
    // what stood here with no line claiming a reason. Horizontally the original
    // *is* centred: `(800-288)/2 = 256 ~= 255`, so that half stays an alignment
    // rather than a number.
    const WINDOW_TOP_PERCENT: f32 = 51.0;
    // The Connect/Exit row hangs off the window instead of the screen so the two
    // keep the gap the original draws: frame bottom 306+140=446, Connect top
    // 460 -> 14 px. As a child of the frame that is one number in one place,
    // and it cannot drift when the frame moves.
    const BUTTON_ROW_TOP: f32 = WINDOW_SIZE.1 + 14.0;

    let (logo_l, logo_t, logo_w, logo_h) = design_pct(LOGO_RECT);
    let (title_l, title_t, title_w, title_h) = design_pct(TITLE_RECT);

    bsn! {
        LoginFormRoot
        Name("Login Form V2")
        Node {
            flex_direction: FlexDirection::Column,
            width: percent(100),
            height: percent(100),
            position_type: PositionType::Absolute,
        }
        Visibility::Hidden
        Children [
            // The two scaled statics (logo, title caption) live inside one
            // centred 4:3 box; `fit_design_surfaces` sizes the box, and the
            // rects above are percentages of it. The four `px(0)` are
            // placeholders that system overwrites, not a layout.
            (
                DesignFit { cover: false }
                Node { position_type: PositionType::Absolute, left: px(0), top: px(0), width: px(0), height: px(0) }
                Pickable::IGNORE
                Children [
                    (
                        ImageNode { image: {logo}, color: Color::NONE, image_mode: NodeImageMode::Stretch }
                        Node {
                            position_type: PositionType::Absolute,
                            left: percent(logo_l),
                            top: percent(logo_t),
                            width: percent(logo_w),
                            height: percent(logo_h),
                        }
                        Pickable::IGNORE
                    ),
                    (
                        ImageNode { image: {title_art}, color: Color::NONE, image_mode: NodeImageMode::Stretch }
                        Node {
                            position_type: PositionType::Absolute,
                            left: percent(title_l),
                            top: percent(title_t),
                            width: percent(title_w),
                            height: percent(title_h),
                        }
                        Pickable::IGNORE
                    ),
                ]
            ),
            // The login window with labels, inputs and the server-list button
            (
                ImageNode { image: {window}, color: Color::NONE, image_mode: NodeImageMode::Stretch }
                Node {
                    position_type: PositionType::Absolute,
                    top: percent(WINDOW_TOP_PERCENT),
                    width: {px(WINDOW_SIZE.0)},
                    height: {px(WINDOW_SIZE.1)},
                    flex_direction: FlexDirection::Column,
                    align_self: AlignSelf::Center,
                    justify_content: JustifyContent::FlexStart,
                }
                Children [
                    (
                        // `GDR_STATIC1` (`pstitle_europe.txt:120`) is
                        // `Rect="36,35,53,15"` — 53, not 59; only PW and Server
                        // (`:101`, `:82`) are 59 wide.
                        // `HAlign=0`/`VAlign=0` on this very control
                        // (`pstitle_europe.txt:127`/`:137`) — left and top, not
                        // the centred default of `label()`.
                        label_aligned(id_label.as_str(), font.clone(), caption_px, 0, 0)
                        Node { position_type: PositionType::Absolute, left: px(36), top: px(35), width: px(53), height: px(15), align_self: AlignSelf::FlexStart }
                    ),
                    (
                        // `GDR_STATIC2` `HAlign=0`/`VAlign=0`
                        // (`pstitle_europe.txt:108`/`:118`).
                        label_aligned(password_label.as_str(), font.clone(), caption_px, 0, 0)
                        Node { position_type: PositionType::Absolute, left: px(36), top: px(62), width: px(59), height: px(15), align_self: AlignSelf::FlexStart }
                    ),
                    (
                        // `GDR_STATIC3` `HAlign=0`/`VAlign=0`
                        // (`pstitle_europe.txt:89`/`:99`).
                        label_aligned(server_label.as_str(), font.clone(), caption_px, 0, 0)
                        Node { position_type: PositionType::Absolute, left: px(36), top: px(89), width: px(59), height: px(15), align_self: AlignSelf::FlexStart }
                    ),
                    (
                        // The three edit rows are `GDR_EDIT_ID` `Rect="110,32,135,20"`
                        // (`pstitle_europe.txt:63`), `GDR_EDIT_PASS` `"110,60,135,20"`
                        // (`:44`) and `GDR_STA_SERVER` `"110,88,86,20"` (`:25`):
                        // a clean 28px pitch, 32/60/88. What stood here was
                        // 35/62/88 — the *label* tops from `GDR_STATIC1/2`
                        // (`:120`, `:101`), copied one column across, which gave
                        // a pitch of 27 then 26. In the original the field
                        // content rows start at y 338/366/394 against a window
                        // origin of (256,306), i.e. 32/60/88.
                        // Centred, not left-flush: `GDR_EDIT_ID`'s `HAlign=1`
                        // (`pstitle_europe.txt:63`), and the original draws the
                        // text centred in the row.
                        text_input_justified(font.clone(), 0, Justify::Center)
                        IdInput
                        Node { position_type: PositionType::Absolute, left: px(110), top: px(32), width: px(135), height: px(20) }
                    ),
                    (
                        password_input::<PwInput>(font.clone(), 1)
                        Node { position_type: PositionType::Absolute, left: px(110), top: px(60), width: px(135), height: px(20) }
                    ),
                    (
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(110),
                            top: px(88),
                            width: px(86),
                            height: px(20),
                            justify_content: JustifyContent::Center,
                        }
                        Children [
                            (label("", font.clone(), shard_px) ShardNameText),
                        ]
                    ),
                    (
                        image_button(list_button_style(assets), 48.0, 24.0)
                        ServerListButton
                        ImageNode { color: Color::NONE }
                        Node { position_type: PositionType::Absolute, left: px(203), top: px(86), align_self: AlignSelf::FlexStart }
                        TabIndex({TAB_SERVER_LIST})
                        ButtonSound({button_sound.clone()})
                        on(move |_activate: On<Activate>,
                            state: Option<Res<State<IntroV2State>>>,
                            mut next_state: ResMut<NextState<IntroV2State>>,
                            options: Res<GameOptions>,
                            mut commands: Commands| {
                            // Guard, not decoration: see `login_form_is_on_screen`.
                            if !login_form_is_on_screen(state.as_deref()) {
                                return;
                            }
                            if let Some(playback) = options.audio.fx_playback() {
                                commands.spawn((
                                    AudioPlayer::new(window_open_sound.clone()),
                                    playback,
                                ));
                            }
                            next_state.set(IntroV2State::ServerSelection);
                        })
                    ),
                    // Connect / Exit button row, a child of the frame so the
                    // 14 px gap below it cannot drift (see `BUTTON_ROW_TOP`).
                    (
                        Node {
                            position_type: PositionType::Absolute,
                            top: px(BUTTON_ROW_TOP),
                            width: {px(WINDOW_SIZE.0)},
                            height: {px(BUTTON_SIZE.1)},
                            align_self: AlignSelf::Center,
                            justify_content: JustifyContent::Center,
                            column_gap: {px(BUTTON_GAP)},
                        }
                        Children [
                            (
                                image_button(main_button_style(assets), BUTTON_SIZE.0, BUTTON_SIZE.1)
                                ConnectButton
                                ImageNode { color: Color::NONE }
                                TabIndex({TAB_CONNECT})
                                ButtonSound({button_sound.clone()})
                                Children [ (label(connect_label.as_str(), font.clone(), button_px)) ]
                                on(super::net::on_connect_activate)
                            ),
                            (
                                image_button(exit_button_style(assets), BUTTON_SIZE.0, BUTTON_SIZE.1)
                                ExitButton
                                ImageNode { color: Color::NONE }
                                ButtonSound({button_sound})
                                Children [ (label(exit_label.as_str(), font, button_px)) ]
                                on(quit_app)
                            ),
                        ]
                    ),
                ]
            ),
        ]
    }
}

/// `Activate` observer of the Exit button: end the session.
///
/// It is a named function with a log line rather than an inline closure because
/// this is the one place in the pre-game screens that ends the process on a
/// player's say-so, and it used to do so **silently** — "the client closes
/// itself" was this button, activated from the keyboard, with nothing in the log
/// to say who quit. One line makes the same accident self-diagnosing in the next
/// log.
pub fn quit_app(
    _activate: On<Activate>,
    state: Option<Res<State<IntroV2State>>>,
    fading_root: Query<(), (With<LoginFormRoot>, With<FadingIn>)>,
    mut exit: MessageWriter<AppExit>,
) {
    // Ending the process is the least recoverable thing this screen does, so it
    // is the last button that may act on a click aimed somewhere else — see
    // [`login_form_accepts_clicks`].
    if !login_form_accepts_clicks(state.as_deref(), &fading_root) {
        return;
    }
    info!("login form: Exit activated — ending the session");
    exit.write(AppExit::Success);
}

/// Puts the caret in the ID field when the login form is entered.
///
/// # The idea
///
/// The original's focus behaviour on this screen is unknown — no `resinfo` field
/// carries a default focus. Focusing the first edit row is therefore a **stated
/// openroad affordance**, not a
/// reproduction: without it the screen cannot be typed into at all until the
/// player finds the field with the mouse, and the field it picks is the one the
/// original draws first (`GDR_EDIT_ID`, `pstitle_europe.txt:63`).
///
/// It only takes focus when nothing on this screen holds it yet, so coming back
/// from the server window does not yank the caret out of the password row.
pub fn focus_id_input(
    mut focus: ResMut<InputFocus>,
    id_query: Query<Entity, With<IdInput>>,
    pw_query: Query<Entity, With<PwInput>>,
) {
    let Ok(id_input) = id_query.single() else {
        return;
    };
    let already_on_form = focus
        .get()
        .is_some_and(|current| current == id_input || pw_query.get(current).is_ok());
    if already_on_form {
        return;
    }
    focus.set(id_input, FocusCause::Navigated);
}

/// Enter in the ID row goes to the password row; Enter in the password row
/// activates Connect.
///
/// # The idea
///
/// `bevy_ui_widgets`' button already activates on Enter when the button itself
/// holds focus, so the only gap is Enter *inside* the two edit rows — which is
/// where a login form is actually typed. The two rows are handled explicitly
/// (and no other focus target is), so this can never double-fire against the
/// widget's own Enter handling on a focused button.
///
/// **Deliberate deviation, and not a guess about an unknown.** Enter on the
/// filled form sends the original **no packet at all** — no `0x6102`, no
/// `0xA102`, no `0x2322` — while a mouse click on Connect sends `0x6102`. So the
/// original does *not* submit on Enter; we do, because a login form that cannot
/// be completed from the keyboard is a usability defect. It goes
/// through `Activate` rather than duplicating
/// [`super::net::on_connect_activate`], so a disabled Connect (request in
/// flight) stays disabled and the sound/observer path is the one the mouse
/// takes.
pub fn submit_on_enter(
    keys: Res<ButtonInput<KeyCode>>,
    mut focus: ResMut<InputFocus>,
    id_query: Query<Entity, With<IdInput>>,
    pw_query: Query<Entity, With<PwInput>>,
    connect_query: Query<Entity, (With<ConnectButton>, Without<InteractionDisabled>)>,
    mut commands: Commands,
) {
    if !keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) {
        return;
    }
    let Some(focused) = focus.get() else {
        return;
    };
    if id_query.get(focused).is_ok() {
        if let Ok(pw_input) = pw_query.single() {
            focus.set(pw_input, FocusCause::Navigated);
        }
        return;
    }
    if pw_query.get(focused).is_ok() {
        if let Ok(connect) = connect_query.single() {
            commands.trigger(Activate { entity: connect });
        }
    }
}

/// Greys the Exit button while the captcha modal is up.
///
/// # The idea
///
/// In the original, while the image-code window is open, **Connect *and* Exit
/// are both greyed** and the status line holds
/// `...Requesting user confirmation...` — the login screen is modal behind the
/// captcha, input goes to the code field only.
///
/// Only the Exit half lives here: Connect is already disabled from the moment
/// the request goes out ([`super::net::on_connect_activate`]) and is re-enabled
/// by the response, so giving it a second owner would race that one. This system
/// therefore closes exactly the gap that was left.
pub fn lock_exit_while_captcha_is_open(
    captcha_modal: Query<(), With<super::captcha::CaptchaModal>>,
    exit_button: Query<(Entity, Has<InteractionDisabled>), With<ExitButton>>,
    mut commands: Commands,
) {
    let modal_open = !captcha_modal.is_empty();
    for (entity, disabled) in exit_button.iter() {
        if modal_open && !disabled {
            commands.entity(entity).insert(InteractionDisabled);
        } else if !modal_open && disabled {
            commands.entity(entity).remove::<InteractionDisabled>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This file up to its test module: the scans below are about the authored
    /// markup, and a scan that also reads the assertions naming the very
    /// strings it looks for cannot fail.
    fn authored_markup() -> &'static str {
        let source = include_str!("login_form.rs");
        &source[..source.find("#[cfg(test)]").expect("the test module")]
    }

    /// `GDR_STA_LOGO` and `GDR_STA_TITLE` carry real rects, so they are scaled
    /// by the one title-scene factor — 254x86 at (270,201) and 92x18 at (23,55)
    /// on an 800x600 client.
    #[test]
    fn the_two_statics_are_scaled_not_native() {
        use crate::scenes::loading_screen::design_fit_rect;

        let drawn = |rect: (f32, f32, f32, f32), win_w: f32, win_h: f32| {
            let (bx, by, bw, bh) = design_fit_rect(win_w, win_h, DesignFit::CONTAIN);
            let (l, t, w, h) = design_pct(rect);
            (
                bx + bw * l / 100.0,
                by + bh * t / 100.0,
                bw * w / 100.0,
                bh * h / 100.0,
            )
        };

        assert_eq!(drawn(LOGO_RECT, 800.0, 600.0), (270.0, 201.5, 254.0, 86.0));
        assert_eq!(drawn(TITLE_RECT, 800.0, 600.0), (23.5, 55.0, 92.0, 18.0));
        // Positive control: at the design size both are their data rect.
        assert_eq!(drawn(LOGO_RECT, 1600.0, 1200.0), LOGO_RECT);
        assert_eq!(drawn(TITLE_RECT, 1600.0, 1200.0), TITLE_RECT);
        // The logo's bottom edge clears the window's top edge (306).
        let (_, top, _, height) = drawn(LOGO_RECT, 800.0, 600.0);
        assert!(top + height < 306.0);
    }

    /// The title caption is actually authored into the scene — it used to be
    /// missing entirely. Source scan for the reason the file's other markup
    /// tests give: no test app can spawn this scene.
    #[test]
    fn the_title_caption_is_drawn() {
        assert!(authored_markup().contains("title_art"));
        assert!(include_str!("assets.rs").contains("outer/text-connect.ddj"));
    }

    /// In the original the two 91x41 buttons start at 300 and 409 on an 800x600
    /// client, 18 px apart, the pair spanning 300..500 about the screen centre.
    /// They hang off the 288-wide frame, whose left edge is (800-288)/2 = 256,
    /// so centring them with that gap has to reproduce exactly those screen
    /// positions.
    #[test]
    fn connect_and_exit_sit_18px_apart_centred_on_the_screen() {
        let pair = 2.0 * BUTTON_SIZE.0 + BUTTON_GAP;
        let frame_left = (800.0 - WINDOW_SIZE.0) / 2.0;
        let connect_left = frame_left + (WINDOW_SIZE.0 - pair) / 2.0;
        let exit_left = connect_left + BUTTON_SIZE.0 + BUTTON_GAP;
        assert_eq!((connect_left, exit_left), (300.0, 409.0));
        assert_eq!(exit_left + BUTTON_SIZE.0, 500.0);
        assert_eq!((connect_left + exit_left + BUTTON_SIZE.0) / 2.0, 400.0);
        // Red control for the layout that stood here: `SpaceEvenly` puts three
        // equal gaps in the row, i.e. ~35 px between the buttons, not 18.
        let space_evenly_gap = (WINDOW_SIZE.0 - 2.0 * BUTTON_SIZE.0) / 3.0;
        assert!(space_evenly_gap > 34.0 && space_evenly_gap < 36.0);
        assert_ne!(space_evenly_gap, BUTTON_GAP);
        // and the markup no longer asks for it (scan the authored half only —
        // the line above names `SpaceEvenly` itself)
        assert!(!authored_markup().contains("JustifyContent::SpaceEvenly"));
    }

    /// `HAlign`/`VAlign` are fields on each control, and this screen's three
    /// captions carry `0`/`0` — left and top (`pstitle_europe.txt:127`, `:108`,
    /// `:89`). The widget takes them as a parameter, so the test checks the
    /// mapping *and* that the three call sites pass the data values.
    #[test]
    fn the_captions_are_left_aligned_because_the_data_says_so() {
        use crate::plugins::ui_v2::widgets::{resinfo_align_self, resinfo_justify};

        assert_eq!(resinfo_justify(0), Justify::Left);
        // positive control: the same function centres what the data centres
        // (the edit rows and both buttons are `HAlign=1`)
        assert_eq!(resinfo_justify(1), Justify::Center);
        assert_eq!(resinfo_align_self(0), AlignSelf::FlexStart);
        assert_eq!(resinfo_align_self(1), AlignSelf::Center);

        assert_eq!(authored_markup().matches("label_aligned(").count(), 3);
    }

    /// The mouse path to Exit is untouched by the Tab-ring change: activating
    /// the button still ends the session.
    #[test]
    fn activating_exit_ends_the_session() {
        use bevy::ui_widgets::Activate;

        let mut app = App::new();
        app.add_message::<AppExit>()
            .insert_resource(State::new(IntroV2State::LoginForm));
        app.world_mut().add_observer(quit_app);
        let exit = app.world_mut().spawn(ExitButton).id();
        app.world_mut().trigger(Activate { entity: exit });

        let messages = app.world().resource::<Messages<AppExit>>();
        let mut cursor = messages.get_cursor();
        assert_eq!(cursor.read(messages).next(), Some(&AppExit::Success));
    }

    /// A click on a screen that is still fading in belongs to the screen it
    /// replaces, and Exit is the button where that matters most: the measured
    /// path (a double-click on the server window's Select, whose area overlaps
    /// the form's button row) put the second click on this row half a second
    /// before it was visible.
    ///
    /// Both directions: with the root fading the session stays, and the test
    /// above is the negative control — the same trigger without
    /// [`FadingIn`] still quits.
    #[test]
    fn a_click_on_the_fading_form_does_not_quit() {
        use bevy::ui_widgets::Activate;

        let mut app = App::new();
        app.add_message::<AppExit>()
            .insert_resource(State::new(IntroV2State::LoginForm));
        app.world_mut().add_observer(quit_app);
        app.world_mut().spawn((LoginFormRoot, FadingIn::default()));
        let exit = app.world_mut().spawn(ExitButton).id();
        app.world_mut().trigger(Activate { entity: exit });

        assert!(
            app.world().resource::<Messages<AppExit>>().is_empty(),
            "a click aimed at the previous screen ended the session"
        );
    }

    /// **From another screen there must be no Tab target inside the login
    /// form.** The form is hidden,
    /// never despawned, and `TabNavigation` filters on neither `Visibility` nor
    /// `InteractionDisabled` — so the only thing that takes its members out of
    /// the ring is the `TabGroup` going away with the screen.
    ///
    /// The red control is the first half of this test: with the group in place
    /// (login form on show) navigation *does* find the Connect button, so the
    /// `NoTabGroups` afterwards is the fix and not a broken query. This test was
    /// seen failing before `close_tab_ring` existed.
    #[test]
    fn leaving_the_login_form_takes_its_buttons_out_of_the_tab_ring() {
        use bevy::ecs::system::SystemState;
        use bevy::input_focus::tab_navigation::{NavAction, TabNavigation};
        use bevy::input_focus::InputFocus;

        let mut app = App::new();
        app.init_resource::<InputFocus>();
        let world = app.world_mut();
        let id = world.spawn((IdInput, TabIndex(0))).id();
        let connect = world.spawn((ConnectButton, TabIndex(TAB_CONNECT))).id();
        let root = world
            .spawn((LoginFormRoot, TabGroup::new(0)))
            .add_children(&[id, connect])
            .id();

        // Positive control: while the screen is on show, Tab from the ID row
        // reaches Connect.
        let mut nav_state = SystemState::<(TabNavigation, Res<InputFocus>)>::new(world);
        {
            let (nav, focus) = nav_state.get(world).expect("valid system params");
            let mut focus = focus.clone();
            focus.set(id, FocusCause::Navigated);
            assert_eq!(nav.navigate(&focus, NavAction::Next), Ok(connect));
        }

        // Leaving the screen: this is what `OnExit(LoginForm)` runs.
        world
            .resource_mut::<InputFocus>()
            .set(connect, FocusCause::Navigated);
        world.run_system_cached(close_tab_ring).unwrap();

        let (nav, focus) = nav_state.get(world).expect("valid system params");
        assert_eq!(
            nav.navigate(&focus, NavAction::Next),
            Err(bevy::input_focus::tab_navigation::TabNavigationError::NoTabGroups),
            "the hidden login form is still collecting Tab targets"
        );
        assert!(
            world.get::<TabGroup>(root).is_none(),
            "TabGroup survived the screen it belongs to"
        );
        assert!(
            world.resource::<InputFocus>().get().is_none(),
            "focus stayed on the invisible Connect button: Enter there still activates it"
        );
    }

    /// The second door on the same defect: the state guard both actions of this
    /// screen ask before they move the state machine. Red control included —
    /// the same helper answers `true` for the screen it belongs to.
    #[test]
    fn only_the_login_form_may_run_the_login_forms_actions() {
        assert!(login_form_is_on_screen(Some(&State::new(
            IntroV2State::LoginForm
        ))));
        assert!(!login_form_is_on_screen(Some(&State::new(
            IntroV2State::CharacterList
        ))));
        assert!(!login_form_is_on_screen(Some(&State::new(
            IntroV2State::CharacterCreate
        ))));
        // Sub-state resource absent = the intro scene is not even running.
        assert!(!login_form_is_on_screen(None));
    }

    /// Both action paths of the screen actually ask that guard. Source scan for
    /// the same reason as the Tab-ring scan below: the list button's observer is
    /// an inline closure in the authored markup, which no test app can spawn.
    #[test]
    fn both_actions_are_behind_the_state_guard() {
        let list_button_observer = include_str!("login_form.rs")
            .split(
                "ServerListButton
",
            )
            .nth(1)
            .expect("the authored list button block");
        assert!(
            list_button_observer.contains("login_form_is_on_screen"),
            "the list button can still jump to ServerSelection from another screen"
        );
        // Positive control for the scan: the Connect observer, which lives in
        // the sibling file, asks the same guard — through
        // [`login_form_accepts_clicks`], which is that state guard plus the
        // fade one.
        assert!(include_str!("net.rs").contains("login_form_accepts_clicks("));
        assert!(authored_markup().contains("login_form_is_on_screen(state) && fading_root"));
    }

    /// The regression this pins: "the client closes itself a few seconds after
    /// the window opens" was the *Exit button*, activated from the
    /// keyboard — `bevy_ui_widgets` activates a focused button on Enter, and
    /// Exit sat one `TabIndex` past Connect while Connect was a silent no-op
    /// without a selected server. So the authored Exit block must carry no
    /// `TabIndex`, and the ring must stop at Connect.
    ///
    /// Source scan, same technique as `plugins::dev` and `settings::live`: what
    /// has to hold is a property of the authored markup, and no test app can
    /// spawn this scene (it needs `IntroV2Assets` + `FontAssets` from the PK2).
    #[test]
    fn the_tab_ring_stops_at_connect_and_never_reaches_exit() {
        let source = include_str!("login_form.rs");
        let exit_block = source
            .split("ExitButton\n")
            .nth(1)
            .expect("the authored Exit button block");
        let exit_block = &exit_block[..exit_block.find("on(quit_app)").expect("Exit's observer")];
        assert!(
            !exit_block.contains("TabIndex"),
            "Exit is back in the Tab ring: one Enter past Connect quits the client"
        );
        // Positive control for the scan itself: the same shape *does* find the
        // Connect button's TabIndex, so a green assert above is a fact about
        // the markup and not a broken search.
        let connect_block = source
            .split("ConnectButton\n")
            .nth(1)
            .expect("the authored Connect button block");
        assert!(connect_block[..200].contains("TabIndex({TAB_CONNECT})"));
    }
}
