use bevy::input_focus::tab_navigation::TabNavigationPlugin;
use bevy::log::warn_once;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui::{InteractionDisabled, Pressed};
use bevy::ui_widgets::Activate;

use crate::assets::textdata::effectsound::SoundAddress;
use crate::assets::FontAssets;
use crate::plugins::audio_events;
use crate::plugins::settings::options::GameOptions;
use crate::plugins::textdata::{ClientEffectSounds, ClientUiStrings};
use style::{ButtonSound, ImageButtonStyle, PasswordEcho};

pub mod choice_confirm;
pub mod style;
pub mod widgets;

/// Widget library built on bevy 0.19's headless widgets (`bevy_ui_widgets`),
/// styled with the game's own image assets. Used by the intro v2 scene.
pub struct UiV2Plugin;

impl Plugin for UiV2Plugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(TabNavigationPlugin)
            .init_resource::<choice_confirm::ChoiceConfirmState>()
            .add_message::<choice_confirm::ChoiceConfirmed>()
            .add_systems(
                Update,
                (
                    update_image_button_visuals,
                    update_password_echo,
                    // The dialog needs the loaded font/string tables, and
                    // `UiV2Plugin` is scene-agnostic, so this system exists from
                    // the very first frame — before the loading screen has put
                    // `FontAssets`/`ClientUiStrings` in the world. Without this
                    // gate, parameter validation fails on frame 1 and takes the
                    // whole app down in every scene.
                    choice_confirm::sync_choice_confirm.run_if(
                        resource_exists::<FontAssets>.and_then(resource_exists::<ClientUiStrings>),
                    ),
                ),
            )
            // Global observer, not a system: the click is the original's
            // release-inside-an-enabled-button, which is what `Activate` means
            // here. One hook covers every button in the
            // tree — pregame and all 57 HUD files — instead of a `ButtonSound`
            // per call site. See `play_button_click_sound`.
            .add_observer(play_button_click_sound);
    }
}

fn update_image_button_visuals(
    mut query: Query<(
        &Hovered,
        Has<Pressed>,
        Has<InteractionDisabled>,
        &ImageButtonStyle,
        &mut ImageNode,
    )>,
) {
    for (hovered, pressed, disabled, style, mut image) in query.iter_mut() {
        let target = match (disabled, pressed, hovered.get()) {
            (true, _, _) => disabled_art(style),
            (_, true, _) => &style.press,
            (_, _, true) => &style.hover,
            _ => &style.normal,
        };
        if image.image != *target {
            image.image = target.clone();
        }
    }
}

/// The `disable` slot, or `normal` when the call site has no `_disable` art.
/// The fallback is legitimate (not every button in the archive ships a
/// disabled frame), but it is reported once so an unset slot on art that
/// *does* have one cannot hide as "looks normal".
fn disabled_art(style: &ImageButtonStyle) -> &Handle<Image> {
    if style.disable == Handle::default() {
        warn_once!(
            "ui_v2: disabled button has no `disable` art, falling back to `normal` ({:?})",
            style.normal.path()
        );
        &style.normal
    } else {
        &style.disable
    }
}

/// Plays the click through the `effectsound.txt` registry (#773): the row
/// `UI / SND_BUTTON_CLICK` names both the `.wav` and the volume the data wants
/// it at (80), which no call site could have invented. The `ButtonSound`
/// component stays as the fallback for the frames before the table is loaded —
/// `UiV2Plugin` is scene-agnostic and runs from frame 1, so the registry
/// resource is read as `Option<Res<_>>` (same reason as the run condition on
/// `sync_choice_confirm` above).
///
/// **Why an `Activate` observer and not `Added<Pressed>`**: the original plays
/// this handle from its *generic* button class, not per window, and only in
/// the mouse-up handler behind three guards — the button is enabled, it was
/// pressed, and the pointer is still inside it. So the original's click is
/// **release-inside-an-enabled-button**, not press.
///
/// `bevy_ui_widgets` raises `Activate` under exactly those conditions
/// (`button.rs:57-72`: `Pointer<Click>` with `Pressed` set and no
/// `InteractionDisabled`), so hanging the sound there reuses those semantics
/// rather than re-implementing them. Two consequences, both
/// wanted: the pregame's click moves from mouse-down to mouse-up, and
/// **every HUD button is audible**, not only the ones carrying a
/// `ButtonSound` component.
///
/// Not covered, and left that way deliberately: checkboxes and radio buttons
/// raise `ValueChange`, not `Activate` (`checkbox.rs:65`, `radio.rs:199`), and
/// whether the original's `CIFCheckBox` clicks at all is unknown — a guess here
/// would add a sound nothing states.
fn play_button_click_sound(
    activate: On<Activate>,
    sounds: Query<&ButtonSound>,
    options: Res<GameOptions>,
    effect_sounds: Option<Res<ClientEffectSounds>>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    // Options 1002 (FX volume) and 1005 (FX on/off) both live in
    // `fx_playback()`; muted FX must not reach either arm below.
    let Some(playback) = options.audio.fx_playback() else {
        return;
    };
    let click = SoundAddress::new("UI", "SND_BUTTON_CLICK");
    let table_has_click = effect_sounds
        .as_ref()
        .is_some_and(|s| !s.sounds(&click).is_empty());
    match (&effect_sounds, table_has_click) {
        // The table answers: `audio_events::play` picks among the two
        // click rows and folds the row volume (80) into the FX channel.
        // Going through the shared helper is what keeps `uibutton_b.wav`
        // reachable: `first()` could only ever play `_a`.
        (Some(sounds), true) => audio_events::play(
            &mut commands,
            &asset_server,
            sounds.as_ref(),
            &options.audio,
            &click,
        ),
        // Fallback for the frames before the table is loaded: `UiV2Plugin`
        // is scene-agnostic and runs from frame 1. It needs a handle, so it
        // only fires for the pregame buttons that carry `ButtonSound`; a HUD
        // button clicked in those first frames is silent rather than loaded
        // from an invented path.
        _ => {
            if let Ok(sound) = sounds.get(activate.entity) {
                commands.spawn((AudioPlayer::new(sound.0.clone()), playback));
            }
        }
    }
}

/// Mirrors the sibling password `EditableText` as asterisks (see
/// [`PasswordEcho`]).
fn update_password_echo(
    mut echoes: Query<(&ChildOf, &mut Text), With<PasswordEcho>>,
    children_query: Query<&Children>,
    sources: Query<&EditableText>,
) {
    for (child_of, mut text) in echoes.iter_mut() {
        let Ok(siblings) = children_query.get(child_of.parent()) else {
            continue;
        };
        let Some(source) = siblings.iter().find_map(|e| sources.get(e).ok()) else {
            continue;
        };

        let masked = "*".repeat(source.value().to_string().chars().count());
        if text.0 != masked {
            text.0 = masked;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the real system in an `App`: the disabled arm used to resolve to
    /// `normal`, which is exactly the defect (#640) — a button that cannot be
    /// pressed looked like one that can.
    fn app_with_button(disable: Option<&str>) -> (App, Entity, Handle<Image>, Handle<Image>) {
        let mut app = App::new();
        // TaskPoolPlugin BEFORE AssetPlugin: `asset_server.load()` touches the
        // IoTaskPool, so the order decides green/red, not the test itself.
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ))
        // `AssetPlugin` installs the asset *server*, not any asset *type*:
        // `Image` is registered by `ImagePlugin`, which drags in the whole
        // render stack. So register just the one type this test loads, or
        // `assets.load::<Image>` panics "asset type has not been initialized".
        .init_asset::<Image>()
        .add_systems(Update, update_image_button_visuals);
        let assets = app.world().resource::<AssetServer>().clone();
        let normal: Handle<Image> = assets.load("normal.png");
        let disable_handle: Handle<Image> = match disable {
            Some(path) => assets.load(path.to_string()),
            None => Handle::default(),
        };
        let style = ImageButtonStyle {
            normal: normal.clone(),
            hover: assets.load("hover.png"),
            press: assets.load("press.png"),
            disable: disable_handle.clone(),
        };
        let entity = app
            .world_mut()
            .spawn((
                Hovered::default(),
                style,
                ImageNode {
                    image: normal.clone(),
                    ..Default::default()
                },
                InteractionDisabled,
            ))
            .id();
        (app, entity, normal, disable_handle)
    }

    #[test]
    fn a_disabled_button_shows_the_disable_art() {
        let (mut app, entity, _normal, disable) = app_with_button(Some("disable.png"));
        app.update();
        let image = app.world().entity(entity).get::<ImageNode>().unwrap();
        assert_eq!(image.image, disable);
    }

    #[test]
    fn a_disabled_button_without_disable_art_falls_back_to_normal() {
        let (mut app, entity, normal, _) = app_with_button(None);
        app.update();
        let image = app.world().entity(entity).get::<ImageNode>().unwrap();
        assert_eq!(image.image, normal);
    }

    /// The click sound, driven through bevy's real button widget rather than
    /// by triggering `Activate` by hand
    /// — the question is exactly *when* the widget raises it. The original
    /// plays `snd_button_click` only on mouse-up, behind the enabled flag, the
    /// "was pressed" bit and a hit test. These four tests are that sentence,
    /// one clause each.
    fn click_app(enabled: bool, fx_enabled: bool, table: bool) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ))
        .init_asset::<AudioSource>()
        .add_plugins(bevy::ui_widgets::ButtonPlugin)
        .add_observer(play_button_click_sound);

        let mut options = GameOptions::default();
        options.audio.fx_enabled = fx_enabled;
        app.insert_resource(options);
        if table {
            app.insert_resource(ClientEffectSounds::from_table(
                crate::assets::textdata::effectsound::EffectSoundTable::parse(concat!(
                    "\tUI\tSND_BUTTON_CLICK\t-\t-\t-\t-\t0\tui\\\tuibutton_a.wav\t80\tclick#1\t\r\n",
                    "\tUI\tSND_BUTTON_CLICK\t-\t-\t-\t-\t0\tui\\\tuibutton_b.wav\t80\tclick#2\t\r\n",
                )),
            ));
        }

        // A bare `Button`, with **no `ButtonSound`** — that is the HUD case:
        // the component is set in `scenes/intro_v2/**` and in none of the 57
        // HUD files, so a query filtered on it would miss every HUD button.
        let mut button = app.world_mut().spawn(bevy::ui_widgets::Button);
        if !enabled {
            button.insert(InteractionDisabled);
        }
        let button = button.id();
        (app, button)
    }

    /// Where the pointer is does not matter to these events — the widget's
    /// own hit test already happened by the time bevy raises them.
    fn pointer_location() -> bevy::picking::pointer::Location {
        bevy::picking::pointer::Location {
            target: bevy::camera::NormalizedRenderTarget::Image(bevy::camera::ImageRenderTarget {
                handle: Handle::default(),
                scale_factor: 1.0,
            }),
            position: Vec2::ZERO,
        }
    }

    fn hit() -> bevy::picking::backend::HitData {
        bevy::picking::backend::HitData::new(Entity::PLACEHOLDER, 0.0, None, None)
    }

    fn press(app: &mut App, button: Entity) {
        let event = bevy::picking::events::Pointer::new(
            bevy::picking::pointer::PointerId::Mouse,
            pointer_location(),
            bevy::picking::events::Press {
                button: bevy::picking::pointer::PointerButton::Primary,
                hit: hit(),
                count: 1,
            },
            button,
        );
        app.world_mut().trigger(event);
        app.update();
    }

    fn release(app: &mut App, button: Entity) {
        let event = bevy::picking::events::Pointer::new(
            bevy::picking::pointer::PointerId::Mouse,
            pointer_location(),
            bevy::picking::events::Click {
                button: bevy::picking::pointer::PointerButton::Primary,
                hit: hit(),
                duration: core::time::Duration::from_millis(50),
                count: 1,
            },
            button,
        );
        app.world_mut().trigger(event);
        app.update();
    }

    fn sounds_playing(app: &mut App) -> usize {
        app.world_mut()
            .query::<&AudioPlayer>()
            .iter(app.world())
            .count()
    }

    /// Pressing is not the click: firing on `Added<Pressed>` would be a
    /// deviation from the original.
    #[test]
    fn pressing_a_button_makes_no_sound() {
        let (mut app, button) = click_app(true, true, true);
        press(&mut app, button);
        assert_eq!(sounds_playing(&mut app), 0);
    }

    /// A HUD-shaped button — `Button` and
    /// nothing else — plays exactly one click on release, from the table
    /// (`uibutton_a.wav` or `uibutton_b.wav`, both at row volume 80).
    #[test]
    fn releasing_over_an_enabled_button_plays_exactly_one_click() {
        let (mut app, button) = click_app(true, true, true);
        press(&mut app, button);
        release(&mut app, button);
        assert_eq!(sounds_playing(&mut app), 1);
    }

    /// The enabled guard: the original returns before it reaches the sound, and
    /// bevy agrees — `button_on_pointer_click` requires `!InteractionDisabled`.
    #[test]
    fn a_disabled_button_stays_silent() {
        let (mut app, button) = click_app(false, true, true);
        press(&mut app, button);
        release(&mut app, button);
        assert_eq!(sounds_playing(&mut app), 0);
    }

    /// Option 1005 (FX on/off) — and by the same `fx_playback()` gate, 1002
    /// (FX volume). A sound the player cannot switch off is a defect, so the
    /// switch is asserted rather than assumed.
    #[test]
    fn muted_fx_plays_no_click() {
        let (mut app, button) = click_app(true, false, true);
        press(&mut app, button);
        release(&mut app, button);
        assert_eq!(sounds_playing(&mut app), 0);
    }

    /// Before the table loads there is no path to play, and a HUD button
    /// carries no `ButtonSound` fallback — so it is silent rather than
    /// guessing a filename. Documents the accepted gap of the first frames.
    #[test]
    fn without_the_table_a_button_without_a_fallback_handle_is_silent() {
        let (mut app, button) = click_app(true, true, false);
        press(&mut app, button);
        release(&mut app, button);
        assert_eq!(sounds_playing(&mut app), 0);
    }
}
