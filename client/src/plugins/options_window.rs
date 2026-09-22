//! The in-game options window shell (Settings from the Esc menu).
//!
//! Idea: the frame, sizes and the bottom button row are transcribed from the
//! vanilla layout (`Media.pk2/resinfo/ifoption.txt`): five tab panes
//! (Video/Audio/Game/Input/Camera) at `11,62,364,H` — H is 313 for
//! video/input/game, 219 for audio and 212 for camera, they do not share
//! one rect —
//! and Default/Confirm/Cancel/Apply sit at y=379 (x 29/113/197/281,
//! `com_button.ddj`). The tab-bar placement itself is not part of that file
//! (the original draws it in code), so the tabs are laid out as a plain row
//! under the title, labelled from the vanilla `UIIT_CTL_MENU_*SET` keys. The
//! tab bodies live in their own modules — #195 video, #196 audio, #197
//! keybinds, #198 gameplay and #379 camera. All five are built.
//!
//! Footer contract: the four buttons of `ifoption.txt` (ids 50-53) need
//! a working state to differ at all, so opening the window starts an edit
//! session (`settings::edit_session`) that holds the confirmed baseline.
//! Confirm = commit + close, Apply = commit + stay, Cancel = restore + close,
//! Default = load this tab's factory values into the live options (undoable by
//! Cancel). Closing with the X or with Esc is deliberately left as it was —
//! neither is a button in `ifoption.txt`, so whether the original treats them
//! as Cancel or as Confirm is unknown. Keeping the edits is the
//! non-destructive of the two guesses.
//!
//! Esc-priority contract: while this window is open, Esc closes it and does
//! nothing else — `toggle_system_window` early-returns on an open options
//! window, and the shared `Closing`/PostUpdate teardown keeps button clicks
//! from falling through to a move order (see `system_window.rs`).

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::{Activate, Button};

use crate::plugins::options_audio::{refresh_audio_rows, spawn_audio_pane};
use crate::plugins::options_camera::{refresh_sight_radios, spawn_camera_pane};
use crate::plugins::options_game::{refresh_game_toggles, spawn_game_pane};
use crate::plugins::options_input_tab::{build_input_pane, reset_all_bindings};
use crate::plugins::options_video::{
    apply_bloom_option, apply_profile_tab, refresh_extra_rows, refresh_row_values,
    spawn_video_pane, VideoPane,
};
use crate::plugins::settings::edit_session::OptionsEditSession;
use crate::plugins::settings::options::{
    AudioOptions, CameraOptions, GameOptions, GameplayOptions, VideoOptions,
};
use crate::plugins::small_popup::spawn_frame;
use crate::plugins::system_window::Closing;
use crate::plugins::textdata::ClientUiStrings;
use crate::plugins::ui_v2::style::ImageButtonStyle;
use crate::scenes::SceneState;

// Window geometry from resinfo/ifoption.txt: the tab panes span the content
// rect x=11..375 (w 364) starting at y=62, the bottom buttons sit at y=379.
// The window hull wraps that with the same margins left/right (11px) and one
// button height + margin below.
const WINDOW_W: f32 = 386.0;
/// The hull is the authored one, not a derived one: `ifsystemwnd.txt`'s
/// `GDR_OPTION:CIFOption` id 98 carries `Rect="0,0,386,413"` outright (#597).
/// The earlier 410 was inferred from the content rect plus margins and came
/// out 3px short — the shell's own block was in a different file than the
/// panes', which is why it was inferred instead of read.
const WINDOW_H: f32 = 413.0;
const CONTENT_X: f32 = 11.0;
const CONTENT_Y: f32 = 62.0;
const CONTENT_W: f32 = 364.0;
/// The tallest pane height. The five panes do **not** share one rect: the
/// data gives `11,62,364,H` with H = 313 for video (`:167`), input (`:110`)
/// and game (`:91`), **219** for audio (`:148`) and **212** for camera
/// (`:129`) — see `OptionsTab::pane_height`.
const CONTENT_H: f32 = 313.0;
const BOTTOM_BTN_Y: f32 = 379.0;
// GDR_OPTION_BTN_{DEF,OK,CANC,APPLY} x positions from ifoption.txt.
const BOTTOM_BTN_XS: [f32; 4] = [29.0, 113.0, 197.0, 281.0];
// The rects in ifoption.txt carry no size, so it comes from the texture, and
// the texture decides it rather than the button's role: `interface/ifcommon/`
// ships four differently sized footer arts (com_button 76x24, com_mid_button
// 88x24, com_mid_button02 112x24, com_red_button 56x24). `com_button.ddj` is
// **76x24**.
const BTN_W: f32 = 76.0;
const BTN_H: f32 = 24.0;

const COM_BUTTON_DDJ: &str = "media://interface/ifcommon/com_button.ddj";
const CLOSE_DDJ: &str = "media://interface/ifcommon/com_windowclose.ddj";
const CLOSE_FOCUS_DDJ: &str = "media://interface/ifcommon/com_windowclose_focus.ddj";
const CLOSE_PRESS_DDJ: &str = "media://interface/ifcommon/com_windowclose_press.ddj";
const BUTTON_FONT: &str = crate::assets::BUNDLED_FALLBACK_FACE;

/// Tab-strip art. The data models a tab as a **texture swap** with three
/// states, not as a recoloured label on a generic push-button, and that
/// mechanism is generation-independent — so it is what we build even though
/// *which* generation v1.188 renders is still UNKNOWN.
///
/// The art is the 4th-gen `opt_long_tab_*` triple, and it is named as such:
/// the classic tree has no tab strip at all (the original draws it in code), so
/// there is no classic art to prefer and no classic rect to transcribe. At its
/// native 120x24 a five-tab strip would need 5x125 = 625px against a 386px hull,
/// so it is drawn at the strip's existing width — the geometry stays the fill it
/// already was, only the *signal* changes.
/// The window's own caption key, taken from the window's own block:
/// `ifsystemwnd.txt` `GDR_OPTION:CIFOption` carries `Text="UIIT_PAG_OPTION"`
/// (textuisystem 805). We resolved `UIIT_CTL_OPTION` (802) with an invented
/// "Settings" fallback. Both render "Option" in English, which is exactly why
/// it survived review — the defect only surfaces once a non-English table
/// loads, and then it is a wrong *key*, not just a wrong fallback (#597).
const TITLE_KEY: &str = "UIIT_PAG_OPTION";
const TITLE_FALLBACK: &str = "Option";

const TAB_ON_DDJ: &str = "media://interface/option/opt_long_tab_on.ddj";
const TAB_OFF_DDJ: &str = "media://interface/option/opt_long_tab_off.ddj";
/// Both labels stay white. The active tab is now distinguished by its
/// texture, so colour is no longer carrying the state on its own — which is
/// what WCAG 2.2 asks for and, here, is also what the data does.
const TAB_LABEL_COLOR: Color = Color::WHITE;

/// The five vanilla option tabs (panes in ifoption.txt).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum OptionsTab {
    #[default]
    Video,
    Audio,
    Game,
    Input,
    Camera,
}

impl OptionsTab {
    const ALL: [OptionsTab; 5] = [
        OptionsTab::Video,
        OptionsTab::Audio,
        OptionsTab::Game,
        OptionsTab::Input,
        OptionsTab::Camera,
    ];

    /// The pane's own height from `ifoption.txt` — `GDR_OPTION_WND_*` are
    /// all `11,62,364,H` but H differs per pane: video `:167`, input `:110`
    /// and game `:91` are 313, audio `:148` is 219, camera `:129` is 212.
    fn pane_height(self) -> f32 {
        match self {
            OptionsTab::Video | OptionsTab::Game | OptionsTab::Input => CONTENT_H,
            OptionsTab::Audio => 219.0,
            OptionsTab::Camera => 212.0,
        }
    }

    /// Loads this tab's factory values into the live options — the
    /// `UIIT_STT_DEFAULT_VALUE` button (`ifoption.txt:63`, id 50).
    ///
    /// **Per tab, not global.** The data names only the button, never its
    /// scope, so the original's scope is unknown. We pick per-tab because it is
    /// the smaller damage if the guess is wrong
    /// (a player who wanted everything reset presses it five times; a player
    /// who wanted one tab reset cannot un-reset four others) and because it is
    /// the trivially reversible direction — one `match` arm becomes a loop.
    /// Reversible either way, in fact: Default writes into the edit session's
    /// working state, so Cancel takes it back.
    fn apply_defaults(self, options: &mut GameOptions) {
        match self {
            OptionsTab::Video => {
                // `window_mode: None` is not a default *value*, it means "the
                // player never expressed a preference" and hands the window to
                // `config.yaml` at the next start (`VideoOptions::window_mode`,
                // `apply_window_mode`). Stamping it back here would be a change
                // the button did not visibly make, so it is kept.
                let window_mode = options.video.window_mode_override;
                options.video = VideoOptions {
                    window_mode_override: window_mode,
                    ..VideoOptions::default()
                };
            }
            OptionsTab::Audio => options.audio = AudioOptions::default(),
            OptionsTab::Game => options.gameplay = GameplayOptions::default(),
            // The keymap default is per-action (`reset_key` falls back to the
            // shipped binding), not an empty map, so it keeps its own reset.
            // `mouse_shortcut_swapped` (id 3101) is part of the same tab.
            OptionsTab::Input => {
                reset_all_bindings(options);
                options.keymap.mouse_shortcut_swapped = false;
            }
            OptionsTab::Camera => options.camera = CameraOptions::default(),
        }
    }

    /// textuisystem key + English fallback of the tab caption.
    fn label(self) -> (&'static str, &'static str) {
        match self {
            OptionsTab::Video => ("UIIT_CTL_MENU_VIDEOSET", "Video"),
            OptionsTab::Audio => ("UIIT_CTL_MENU_AUDIOSET", "Audio"),
            OptionsTab::Game => ("UIIT_CTL_MENU_GAMESET", "Setting"),
            OptionsTab::Input => ("UIIT_CTL_MENU_INPUTSET", "Key Map"),
            OptionsTab::Camera => ("UIIT_CTL_MENU_CAMERASET", "View"),
        }
    }
}

/// Root marker of the options window; carries the active tab.
///
/// `active` is `pub(crate)` so the offline preview scene can show a tab other
/// than the default one (`scenes/testing/options_ui.rs`) — the tab bar's own
/// observer writes the same field.
#[derive(Component, Default)]
pub(crate) struct OptionsWindow {
    pub(crate) active: OptionsTab,
}

/// A tab-bar button, whose texture swaps when the active tab changes.
/// (Was `OptionsTabLabel` and sat on the caption, back when the state was a
/// label colour — #597 moved both the marker and the signal onto the button.)
#[derive(Component)]
struct OptionsTabButton(OptionsTab);

/// A tab body pane, shown only while its tab is active.
#[derive(Component)]
struct OptionsPane(OptionsTab);

pub struct OptionsWindowPlugin;

impl Plugin for OptionsWindowPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                close_on_esc,
                apply_active_tab,
                apply_profile_tab,
                refresh_row_values,
                refresh_extra_rows,
                refresh_game_toggles,
                refresh_sight_radios,
                refresh_audio_rows,
            )
                // `UiTesting` as well as the in-game scene, so the offline
                // preview scene renders the *same* window instead of a copy of
                // it — the house pattern of every previewable HUD window
                // (`hud/chat/mod.rs`, `hud/cos/mod.rs`, `hud/party/mod.rs` all
                // read `in_state(GameWorld).or_else(in_state(UiTesting))`).
                // Nothing here touches the world; the systems only repaint the
                // window that the scene spawned.
                .run_if(in_state(SceneState::GameWorld).or_else(in_state(SceneState::UiTesting))),
        )
        // The applied options must survive the window being closed, so this
        // runs regardless of whether it is open. The window mode is not here:
        // it is resolved from `config.yaml` plus the session-only override by
        // `config::window::apply_window_settings`, the single writer.
        .add_systems(Update, apply_bloom_option);
    }
}

/// Spawns the options window. `pub(crate)` — opened by the Esc menu's
/// Settings button (`system_window::on_settings`).
pub(crate) fn spawn_options_window(
    commands: &mut Commands,
    asset_server: &AssetServer,
    ui_strings: &ClientUiStrings,
    options: &GameOptions,
    camera: Entity,
) {
    let font = asset_server.load::<Font>(BUTTON_FONT);
    let button_style = ImageButtonStyle {
        normal: asset_server.load(COM_BUTTON_DDJ),
        hover: asset_server.load(COM_BUTTON_DDJ),
        press: asset_server.load(COM_BUTTON_DDJ),
        ..Default::default()
    };
    let close_style = ImageButtonStyle {
        normal: asset_server.load(CLOSE_DDJ),
        hover: asset_server.load(CLOSE_FOCUS_DDJ),
        press: asset_server.load(CLOSE_PRESS_DDJ),
        ..Default::default()
    };

    // Opening the window begins an edit session: everything the panes change
    // from here is undoable by `UIIT_CTL_CANCEL` (`ifoption.txt:25`, id 52).
    // See `settings::edit_session` for why the baseline (and not a pending
    // copy) is what we keep.
    let mut session = OptionsEditSession::default();
    session.begin(options);
    commands.insert_resource(session);

    commands
        .spawn((
            OptionsWindow::default(),
            Name::from("Options Window"),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(WINDOW_W),
                height: Val::Px(WINDOW_H),
                margin: UiRect::all(Val::Auto),
                ..default()
            },
            GlobalZIndex(150),
            UiTargetCamera(camera),
        ))
        .with_children(|w| {
            spawn_frame(w, asset_server);
            // Title, centered in the frame's top bar like the System window.
            w.spawn((
                // The window's own key, from its own block: `ifsystemwnd.txt`
                // `GDR_OPTION:CIFOption` carries `Text="UIIT_PAG_OPTION"`
                // (805). We resolved `UIIT_CTL_OPTION` (802) with an invented
                // "Settings" fallback. Both keys render "Option" in English,
                // which is exactly why this survived review — it is only
                // visible once a non-English table loads (#597).
                Text::new(ui_strings.get_or(TITLE_KEY, TITLE_FALLBACK).to_string()),
                TextFont {
                    font: font.clone().into(),
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
                TextColor(Color::srgb(0.92, 0.86, 0.62)),
                TextLayout::justify(Justify::Center),
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(18.0),
                    width: Val::Percent(100.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
            w.spawn((
                Button,
                Hovered::default(),
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(14.0),
                    top: Val::Px(14.0),
                    width: Val::Px(16.0),
                    height: Val::Px(16.0),
                    ..default()
                },
                ImageNode::new(close_style.normal.clone()),
                close_style,
            ))
            .observe(on_close);

            // Tab bar: a row of the five vanilla tabs above the content rect.
            w.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(CONTENT_X),
                    top: Val::Px(38.0),
                    width: Val::Px(CONTENT_W),
                    height: Val::Px(BTN_H),
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(2.0),
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|bar| {
                for tab in OptionsTab::ALL {
                    let (key, fallback) = tab.label();
                    bar.spawn((
                        Button,
                        Hovered::default(),
                        Node {
                            width: Val::Px(BTN_W - 4.0),
                            height: Val::Px(BTN_H),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        // The tab carries its own marker so `apply_active_tab`
                        // can swap *this* node's texture. No `ImageButtonStyle`
                        // here on purpose: that component drives its own
                        // hover/press swaps and would fight the state swap.
                        OptionsTabButton(tab),
                        ImageNode::new(asset_server.load(TAB_OFF_DDJ)),
                    ))
                    .observe(
                        move |_activate: On<Activate>, mut window: Query<&mut OptionsWindow>| {
                            if let Ok(mut win) = window.single_mut() {
                                if win.active != tab {
                                    win.active = tab;
                                }
                            }
                        },
                    )
                    .with_children(|b| {
                        b.spawn((
                            Text::new(ui_strings.get_or(key, fallback).to_string()),
                            TextFont {
                                font: font.clone().into(),
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                            TextColor(TAB_LABEL_COLOR),
                            Pickable::IGNORE,
                        ));
                    });
                }
            });

            // Content panes, one per tab, each on its own vanilla rect
            // (`11,62,364,H`, H per pane — audio and camera are shorter);
            // only the active one is shown. With Audio (#196) there is no
            // placeholder pane left: Video (#195), Game (#198), Input (#197),
            // Camera (#379) and Audio are all built.
            for tab in OptionsTab::ALL {
                let mut pane = w.spawn((
                    OptionsPane(tab),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(CONTENT_X),
                        top: Val::Px(CONTENT_Y),
                        width: Val::Px(CONTENT_W),
                        height: Val::Px(tab.pane_height()),
                        display: Display::None,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.25)),
                    Pickable::IGNORE,
                ));
                // Each tab's body lives in its own module: #195 video, #198
                // game, #197 key map.
                if tab == OptionsTab::Video {
                    pane.insert(VideoPane::default());
                    pane.with_children(|p| {
                        spawn_video_pane(p, asset_server, ui_strings, options);
                    });
                } else if tab == OptionsTab::Game {
                    pane.with_children(|p| {
                        spawn_game_pane(p, asset_server, &font, ui_strings, options);
                    });
                } else if tab == OptionsTab::Input {
                    pane.with_children(|p| {
                        build_input_pane(p, asset_server, ui_strings, options);
                    });
                } else if tab == OptionsTab::Camera {
                    pane.with_children(|p| {
                        spawn_camera_pane(p, asset_server, &font, ui_strings, options);
                    });
                } else if tab == OptionsTab::Audio {
                    pane.with_children(|p| {
                        spawn_audio_pane(p, asset_server, &font, ui_strings, options);
                    });
                }
            }

            // Bottom row per ifoption.txt: Default / Confirm / Cancel / Apply.
            let bottom: [(&str, &str, BottomAction); 4] = [
                ("UIIT_STT_DEFAULT_VALUE", "Default", BottomAction::Default),
                ("UIIT_CTL_CONFIRM", "Confirm", BottomAction::Confirm),
                ("UIIT_CTL_CANCEL", "Cancel", BottomAction::Cancel),
                ("UIIT_CTL_APPLY", "Apply", BottomAction::Apply),
            ];
            for ((key, fallback, action), x) in bottom.into_iter().zip(BOTTOM_BTN_XS) {
                w.spawn((
                    Button,
                    Hovered::default(),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(x),
                        top: Val::Px(BOTTOM_BTN_Y),
                        width: Val::Px(BTN_W),
                        height: Val::Px(BTN_H),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    ImageNode::new(button_style.normal.clone()),
                    button_style.clone(),
                ))
                .observe(
                    move |_activate: On<Activate>,
                          window: Query<(Entity, &OptionsWindow)>,
                          mut options: ResMut<GameOptions>,
                          mut session: ResMut<OptionsEditSession>,
                          mut commands: Commands| {
                        // The four buttons of `ifoption.txt` (ids 50-53) can
                        // only differ if the window edits a working state:
                        // Apply writes through and stays, Confirm writes
                        // through and closes, Cancel restores the baseline,
                        // Default loads factory values *into* the working
                        // state. The data names only the buttons; these
                        // semantics are openroad's reading of them.
                        match action {
                            BottomAction::Confirm => {
                                session.commit(&options);
                                for (entity, _) in window.iter() {
                                    commands.entity(entity).insert(Closing);
                                }
                            }
                            BottomAction::Cancel => {
                                // `set_if_neq`-style guard: reverting an
                                // untouched set would still flag the resource
                                // changed and re-run every apply/persist
                                // system for nothing.
                                if session.is_dirty(&options) {
                                    session.revert(&mut options);
                                }
                                for (entity, _) in window.iter() {
                                    commands.entity(entity).insert(Closing);
                                }
                            }
                            BottomAction::Default => {
                                let Some(active) = window.iter().next().map(|(_, w)| w.active)
                                else {
                                    return;
                                };
                                active.apply_defaults(&mut options);
                            }
                            // Write-through without closing. Edits already
                            // reach `GameOptions` live (the audio preview), so
                            // Apply's real work is to make them survive a
                            // later Cancel.
                            BottomAction::Apply => session.commit(&options),
                        }
                    },
                )
                .with_children(|b| {
                    b.spawn((
                        Text::new(ui_strings.get_or(key, fallback).to_string()),
                        TextFont {
                            font: font.clone().into(),
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        Pickable::IGNORE,
                    ));
                });
            }
        });
}

#[derive(Debug, Clone, Copy)]
enum BottomAction {
    Default,
    Confirm,
    Cancel,
    Apply,
}

/// Swaps each tab's texture and toggles pane visibility whenever the active
/// tab changes (and once on spawn, via the added-component change).
///
/// The swap is the point (#597): the tab strip used to signal its state by
/// recolouring the label on a generic `com_button.ddj`, which is neither what
/// the data models nor distinguishable by anything but colour. `CNIFTabButton`
/// carries on/off/disable *textures*, so the state lives in the image.
fn apply_active_tab(
    window: Query<&OptionsWindow, Changed<OptionsWindow>>,
    asset_server: Res<AssetServer>,
    mut tabs: Query<(&OptionsTabButton, &mut ImageNode)>,
    mut panes: Query<(&OptionsPane, &mut Node)>,
) {
    let Ok(win) = window.single() else {
        return;
    };
    for (tab, mut image) in tabs.iter_mut() {
        let art = if tab.0 == win.active {
            TAB_ON_DDJ
        } else {
            TAB_OFF_DDJ
        };
        image.image = asset_server.load(art);
    }
    for (pane, mut node) in panes.iter_mut() {
        node.display = if pane.0 == win.active {
            Display::Flex
        } else {
            Display::None
        };
    }
}

/// Esc closes the options window (before the Esc menu logic runs —
/// `toggle_system_window` early-returns while this window exists).
fn close_on_esc(
    keys: Res<ButtonInput<KeyCode>>,
    chat: Res<crate::plugins::hud::chat::model::ChatState>,
    window: Query<Entity, With<OptionsWindow>>,
    mut commands: Commands,
) {
    if !keys.just_pressed(KeyCode::Escape) || chat.input_open {
        return;
    }
    for entity in window.iter() {
        commands.entity(entity).insert(Closing);
    }
}

fn on_close(
    _activate: On<Activate>,
    window: Query<Entity, With<OptionsWindow>>,
    mut commands: Commands,
) {
    for entity in window.iter() {
        commands.entity(entity).insert(Closing);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// `ifoption.txt` gives the footer buttons no size, so the size comes from
    /// the texture, not from the button's role: `interface/ifcommon/` ships four
    /// differently sized footer arts (76x24, 88x24, 112x24, 56x24). This is
    /// `com_button.ddj`'s extent; it had drifted to 22 high.
    #[test]
    fn the_footer_button_matches_the_com_button_extent() {
        assert_eq!((BTN_W, BTN_H), (76.0, 24.0));
    }

    /// #597-2. The hull is authored outright — `ifsystemwnd.txt`'s
    /// `GDR_OPTION:CIFOption` id 98 is `Rect="0,0,386,413"` — not derived from
    /// the content rect plus margins, which is how it came out 3px short.
    #[test]
    fn the_hull_is_the_authored_rect_not_a_derived_one() {
        assert_eq!((WINDOW_W, WINDOW_H), (386.0, 413.0));

        // The derivation that produced the old 410 is still a sanity floor:
        // the hull has to clear the bottom button row and its margin.
        assert!(BOTTOM_BTN_Y + BTN_H <= WINDOW_H);
    }

    /// #597-3. The title resolves the window's own key. Both keys render
    /// "Option" in English, so nothing on an English table can catch this —
    /// only the key itself can, which is why it is asserted rather than
    /// eyeballed. "Settings" was invented copy and is gone.
    #[test]
    fn the_title_uses_the_windows_own_key_and_the_vanilla_word() {
        assert_eq!(TITLE_KEY, "UIIT_PAG_OPTION");
        assert_eq!(TITLE_FALLBACK, "Option");
        assert_ne!(TITLE_KEY, "UIIT_CTL_OPTION", "802 is a different string");
    }

    /// #597-4. The active tab must be signalled by its **texture**, because
    /// that is what `CNIFTabButton` models (on/off/disable) and because a
    /// colour-only signal fails "not by colour alone". If someone reintroduces
    /// a single art for both states, the swap silently stops meaning anything.
    #[test]
    fn the_active_tab_is_signalled_by_a_different_texture() {
        assert_ne!(TAB_ON_DDJ, TAB_OFF_DDJ);
        for art in [TAB_ON_DDJ, TAB_OFF_DDJ] {
            assert!(
                art.starts_with("media://interface/option/opt_long_tab_"),
                "{art} is not tab art from the corpus"
            );
        }

        // And the label may no longer carry the state on its own.
        assert_eq!(TAB_LABEL_COLOR, Color::WHITE);
    }

    /// Default is wired **per tab**: pressing it on one tab may not touch
    /// another tab's group. If someone later makes it global, this is the test
    /// that has to be changed deliberately.
    #[test]
    fn default_touches_only_the_active_tabs_group() {
        let mut edited = GameOptions::default();
        edited.video.graphic1.brightness = 4;
        edited.audio.bgm_volume = 99;
        edited.gameplay.toggles.insert(2001, false);
        edited.keymap.mouse_shortcut_swapped = true;
        edited.camera.sight = crate::plugins::settings::options::SightMode::Quarter;

        for tab in OptionsTab::ALL {
            let mut opts = edited.clone();
            tab.apply_defaults(&mut opts);
            let d = GameOptions::default();

            assert_eq!(opts.video == d.video, tab == OptionsTab::Video, "{tab:?}");
            assert_eq!(opts.audio == d.audio, tab == OptionsTab::Audio, "{tab:?}");
            assert_eq!(
                opts.gameplay == d.gameplay,
                tab == OptionsTab::Game,
                "{tab:?}"
            );
            assert_eq!(opts.keymap == d.keymap, tab == OptionsTab::Input, "{tab:?}");
            assert_eq!(
                opts.camera == d.camera,
                tab == OptionsTab::Camera,
                "{tab:?}"
            );
        }
    }

    /// The Video tab's Default keeps `window_mode_override`: `None` is not a
    /// factory *value* but "no preference expressed", and writing it back would
    /// hand the window to `config.yaml` mid-session — a change the button did
    /// not visibly make (`VideoOptions::window_mode_override`).
    #[test]
    fn video_default_keeps_the_window_mode_preference() {
        let mut opts = GameOptions::default();
        opts.video.window_mode_override = Some(true);
        opts.video.graphic2.brightness = 3;

        OptionsTab::Video.apply_defaults(&mut opts);
        assert_eq!(opts.video.window_mode_override, Some(true));
        assert_eq!(
            opts.video.graphic2,
            GameOptions::default().video.graphic2,
            "everything else on the tab does reset"
        );
    }

    /// `ifoption.txt` — the five `GDR_OPTION_WND_*` panes are `11,62,364,H`
    /// with H **per pane**: video `:167`, input `:110` and game `:91` are
    /// 313, audio `:148` is 219 and camera `:129` is 212. The shell used to
    /// apply 313 to all five, leaving the camera pane +101 too tall.
    #[test]
    fn each_options_pane_uses_its_own_ifoption_height() {
        assert_eq!(OptionsTab::Video.pane_height(), 313.0);
        assert_eq!(OptionsTab::Input.pane_height(), 313.0);
        assert_eq!(OptionsTab::Game.pane_height(), 313.0);
        assert_eq!(OptionsTab::Audio.pane_height(), 219.0);
        assert_eq!(OptionsTab::Camera.pane_height(), 212.0);
    }

    /// Every pane starts at the shared origin and stays inside the hull:
    /// `CONTENT_Y` + the tallest pane must clear the bottom button row
    /// (`GDR_OPTION_BTN_*` at y 379).
    #[test]
    fn option_panes_share_the_content_origin_and_clear_the_button_row() {
        for tab in OptionsTab::ALL {
            assert!(tab.pane_height() <= CONTENT_H);
            assert!(CONTENT_Y + tab.pane_height() <= BOTTOM_BTN_Y);
        }
        assert_eq!((CONTENT_X, CONTENT_Y, CONTENT_W), (11.0, 62.0, 364.0));
    }
}
