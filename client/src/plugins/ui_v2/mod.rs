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
pub mod focus_ring;
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
                    focus_ring::update_focus_ring,
                    update_password_echo,
                    // The dialog needs the loaded font/string tables, and
                    // `UiV2Plugin` is scene-agnostic, so this system exists from
                    // the very first frame — before the loading screen has put
                    // `FontAssets`/`ClientUiStrings` in the world. Without this
                    // gate, parameter validation fails on frame 1 and takes the
                    // whole app down in every scene (§4b class of defect).
                    choice_confirm::sync_choice_confirm.run_if(
                        resource_exists::<FontAssets>.and_then(resource_exists::<ClientUiStrings>),
                    ),
                ),
            )
            // Global observer, not a system: `Activate` is the measured
            // release-inside-an-enabled-button of the original's generic
            // button class, and one hook covers every button in the tree —
            // pregame and HUD alike — instead of a marker per call site.
            // See `play_button_click_sound`.
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
/// **Why `Activate` and not `Added<Pressed>`.** The original pushes the handle
/// `snd_button_click` from two sites, and the interesting one sits in its
/// *generic* button class rather than in any per-window call. That handler
/// (`0x656160`) dispatches through the jump table at `0x656414` over
/// `msg - 0x200`, i.e. `[MOUSEMOVE, LBUTTONDOWN, LBUTTONUP, DBLCLK]` =
/// `[0x656324, 0x65619f, 0x656228, 0x65619f]`; the push at `0x65630c` lies
/// inside the **`LBUTTONUP`** arm (`0x656228..0x656324`), behind the enabled
/// byte (`cmp byte [esi+0x3d0], 0`, `0x656166`), the "was pressed" bit
/// (`test al, 0x02`, `0x65626f`) and a vtable hit test (`call edx` +
/// `test al, al`, `0x6562b7`). So the original's click is
/// release-inside-an-enabled-button, not press. [V: `sro_client.exe` md5
/// 45513a60a109001bdfdae2ce68ac00a2, PE32 ImageBase `0x400000`]
///
/// `bevy_ui_widgets` raises `Activate` under exactly those three conditions
/// (`button.rs:60-76`: `PointerClick` with `Pressed` set and no
/// `InteractionDisabled`), so the sound hangs on the measured semantics
/// instead of a re-implementation of them. Two consequences, both wanted: the
/// pregame's click moves from mouse-down to mouse-up, and **every HUD button
/// becomes audible**, because `ButtonSound` is set in `scenes/intro_v2/**` and
/// nowhere else while 20 of the 46 `image_button` call sites live under
/// `plugins/hud/`. Deliberate deviation: `button_on_key_event`
/// (`button.rs:40-58`) also raises `Activate` for Enter/Space, so a
/// keyboard-activated button sounds too — the same button press should not
/// depend on the input device.
///
/// Checkboxes and radios raise `ValueChange`, not `Activate`, and whether the
/// original's `CIFCheckBox` inherits this click is not measured — so they stay
/// silent rather than gaining a sound nobody sourced.
fn play_button_click_sound(
    activate: On<Activate>,
    sounds: Query<&ButtonSound>,
    options: Res<GameOptions>,
    effect_sounds: Option<Res<ClientEffectSounds>>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    let Some(playback) = options.audio.fx_playback() else {
        return;
    };
    let click = SoundAddress::new("UI", "SND_BUTTON_CLICK");
    // Through the shared helper, not a local lookup: the table ships two
    // `SND_BUTTON_CLICK` rows and `audio_events::play` is what picks among
    // them and folds the row volume into the FX channel. Reading only the
    // first row could never reach `uibutton_b.wav`.
    if let Some(table) = effect_sounds {
        if !table.sounds(&click).is_empty() {
            audio_events::play(
                &mut commands,
                &asset_server,
                table.as_ref(),
                &options.audio,
                &click,
            );
            return;
        }
    }
    // Before the table is loaded there is no sourced path, so only a button
    // that brought its own handle can sound; anything else stays silent
    // rather than loading an invented filename.
    if let Ok(sound) = sounds.get(activate.entity) {
        commands.spawn((AudioPlayer::new(sound.0.clone()), playback));
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
    /// by triggering `Activate` by hand — the question these tests answer is
    /// exactly *when* the widget raises it.
    ///
    /// The original's generic button handler (`0x656160`) plays
    /// `snd_button_click` (`0x65630c`) only from the `LBUTTONUP` arm of its
    /// jump table (`0x656414`), behind the enabled byte `+0x3d0`, the "was
    /// pressed" bit and a hit test. The tests below are that sentence, one
    /// clause each.
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
        // the component is set in `scenes/intro_v2/**` and in none of the HUD
        // files, and the old `Query<&ButtonSound, Added<Pressed>>` is the
        // reason every HUD button was silent.
        let mut button = app.world_mut().spawn(bevy::ui_widgets::Button);
        if !enabled {
            button.insert(InteractionDisabled);
        }
        let button = button.id();
        (app, button)
    }

    /// Where the pointer is does not matter to these events — the widget's own
    /// hit test has already happened by the time bevy raises them, the same
    /// split as the original's vtable `+0xb0` call.
    fn pointer() -> bevy::picking::events::Pointer {
        bevy::picking::events::Pointer::new(
            bevy::picking::pointer::PointerId::Mouse,
            bevy::picking::pointer::Location {
                target: bevy::camera::NormalizedRenderTarget::Image(
                    bevy::camera::ImageRenderTarget {
                        handle: Handle::default(),
                        scale_factor: 1.0,
                    },
                ),
                position: Vec2::ZERO,
            },
        )
    }

    fn hit() -> bevy::picking::backend::HitData {
        bevy::picking::backend::HitData::new(Entity::PLACEHOLDER, 0.0, None, None)
    }

    fn press(app: &mut App, button: Entity) {
        app.world_mut()
            .trigger(bevy::picking::events::PointerPress {
                entity: button,
                pointer: pointer(),
                button: PointerButton::Primary,
                hit: hit(),
                count: 1,
            });
        app.update();
    }

    fn release(app: &mut App, button: Entity) {
        app.world_mut()
            .trigger(bevy::picking::events::PointerClick {
                entity: button,
                pointer: pointer(),
                button: PointerButton::Primary,
                hit: hit(),
                duration: core::time::Duration::from_millis(50),
                count: 1,
            });
        app.update();
    }

    fn sounds_playing(app: &mut App) -> usize {
        app.world_mut()
            .query::<&AudioPlayer>()
            .iter(app.world())
            .count()
    }

    /// Guards (b)+(c) of the original's handler: pressing is not the click.
    /// Ours used to fire here (`Added<Pressed>`), which is the measured
    /// deviation this change removes.
    #[test]
    fn pressing_a_button_makes_no_sound() {
        let (mut app, button) = click_app(true, true, true);
        press(&mut app, button);
        assert_eq!(sounds_playing(&mut app), 0);
    }

    /// The point of the change: a HUD-shaped button — `Button` and nothing
    /// else — plays exactly one click on release, from the table.
    #[test]
    fn releasing_over_an_enabled_button_plays_exactly_one_click() {
        let (mut app, button) = click_app(true, true, true);
        press(&mut app, button);
        release(&mut app, button);
        assert_eq!(sounds_playing(&mut app), 1);
    }

    /// Both sides of the tree take the same path now: the pregame's buttons
    /// carry `ButtonSound`, the HUD's `image_button` call sites do not, and
    /// one observer serves both — the HUD gains its click without the pregame
    /// losing its own.
    #[test]
    fn an_intro_button_and_a_hud_button_both_sound() {
        let (mut app, hud_button) = click_app(true, true, true);
        let intro_button = app
            .world_mut()
            .spawn((bevy::ui_widgets::Button, ButtonSound(Handle::default())))
            .id();

        press(&mut app, intro_button);
        release(&mut app, intro_button);
        assert_eq!(sounds_playing(&mut app), 1, "pregame button went silent");

        press(&mut app, hud_button);
        release(&mut app, hud_button);
        assert_eq!(sounds_playing(&mut app), 2, "HUD button made no sound");
    }

    /// Guard (a), the enabled byte `+0x3d0`: the original returns before it
    /// reaches the sound, and bevy agrees — `button_on_pointer_click` requires
    /// `!InteractionDisabled`. So this is a checked agreement, not an
    /// assumption.
    #[test]
    fn a_disabled_button_stays_silent() {
        let (mut app, button) = click_app(false, true, true);
        press(&mut app, button);
        release(&mut app, button);
        assert_eq!(sounds_playing(&mut app), 0);
    }

    /// Option 1005 (FX on/off) — and through the same `fx_playback()` gate,
    /// 1002 (FX volume). A sound the player cannot switch off is a defect, so
    /// the switch is asserted rather than assumed.
    #[test]
    fn muted_fx_plays_no_click() {
        let (mut app, button) = click_app(true, false, true);
        press(&mut app, button);
        release(&mut app, button);
        assert_eq!(sounds_playing(&mut app), 0);
    }

    /// Before the table loads there is no sourced path, and a HUD button
    /// carries no `ButtonSound` fallback — so it stays silent rather than
    /// guessing a filename. Documents the accepted gap of the first frames.
    #[test]
    fn without_the_table_a_button_without_a_fallback_handle_is_silent() {
        let (mut app, button) = click_app(true, true, false);
        press(&mut app, button);
        release(&mut app, button);
        assert_eq!(sounds_playing(&mut app), 0);
    }
}
