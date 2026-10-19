use bevy::input_focus::tab_navigation::TabIndex;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, FontSourceTemplate, TextCursorStyle};
use bevy::ui_widgets::{Button, TextInput};

use super::focus_ring::FocusRing;
use super::style::{ImageButtonStyle, PasswordEcho, TargetColor};

/// A game-styled image button built on the headless widget button: emits
/// `Activate` on click, visual state handled by `update_image_button_visuals`.
pub fn image_button(style: ImageButtonStyle, width: f32, height: f32) -> impl Scene {
    let ImageButtonStyle {
        normal,
        hover,
        press,
        disable,
    } = style;
    let initial = normal.clone();
    bsn! {
        Button
        Hovered
        FocusRing
        Node {
            width: px(width),
            height: px(height),
            justify_content: JustifyContent::Center,
            align_self: AlignSelf::Center,
        }
        ImageNode { image: {initial} }
        ImageButtonStyle { normal: {normal}, hover: {hover}, press: {press}, disable: {disable} }
    }
}

/// Centered label, typically spawned as a child of an [`image_button`].
#[allow(dead_code)]
pub fn button_label(text: &str, font: Handle<Font>) -> impl Scene {
    label(text, font, 16.0)
}

/// Label that starts fully transparent and carries a [`TargetColor`], so the
/// intro's fade systems can fade it in (mirrors the old intro's `text()`).
pub fn label(text: &str, font: Handle<Font>, font_size: f32) -> impl Scene {
    let text = text.to_string();
    bsn! {
        Text({text})
        TextFont { font: FontSourceTemplate::Handle({font}), font_size: {FontSize::Px(font_size)} }
        TextColor(Color::NONE)
        TargetColor(Srgba::WHITE)
        TextLayout::justify(Justify::Center)
        Node {
            justify_content: JustifyContent::Center,
            align_self: AlignSelf::Center,
        }
        Pickable::IGNORE
    }
}

/// Top padding that vertically centers the 12px input text (line box ≈14.4px)
/// in the 20px input fields; `EditableText` has no vertical alignment API.
const INPUT_PAD_TOP: f32 = 3.0;

/// Single-line text input. Click to focus, typing/caret/selection handled by
/// bevy's `TextInputPlugin`.
///
/// `TextInput` is not decoration: all eight keyboard, pointer and IME
/// observers in `bevy_ui_widgets::text_input` filter on `With<TextInput>`, so
/// an `EditableText` without it compiles and then silently accepts no input.
pub fn text_input(font: Handle<Font>, tab: i32) -> impl Scene {
    bsn! {
        TextInput
        EditableText { visible_lines: {Some(1.0)}, allow_newlines: false }
        FocusRing
        Node {
            width: percent(100),
            height: percent(100),
            padding: {UiRect::top(Val::Px(INPUT_PAD_TOP))},
        }
        TextFont { font: FontSourceTemplate::Handle({font}), font_size: {FontSize::Px(12.0)} }
        TextColor(Color::WHITE)
        TargetColor(Srgba::WHITE)
        TextCursorStyle { color: Color::WHITE }
        TabIndex({tab})
    }
}

/// Single-line password input: the editable text renders transparent glyphs
/// (caret stays visible) while a sibling overlay shows asterisks. `M` is a
/// marker component placed on the inner `EditableText` entity so callers can
/// query the value.
pub fn password_input<M: Component + Default + Clone + Unpin>(
    font: Handle<Font>,
    tab: i32,
) -> impl Scene {
    let echo_font = font.clone();
    bsn! {
        // the ring belongs to the wrapper, not to the inner `EditableText`
        // that carries the `TabIndex` — see `focus_ring::update_focus_ring`
        FocusRing
        Node { width: percent(100), height: percent(100) }
        Children [
                M
                TextInput
                EditableText { visible_lines: {Some(1.0)}, allow_newlines: false }
                Node {
                    width: percent(100),
                    height: percent(100),
                    padding: {UiRect::top(Val::Px(INPUT_PAD_TOP))},
                }
                TextFont { font: FontSourceTemplate::Handle({font}), font_size: {FontSize::Px(12.0)} }
                TextColor(Color::NONE)
                TextCursorStyle { color: Color::WHITE }
                TabIndex({tab})
            --
                PasswordEcho
                Text("")
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px(0),
                    width: percent(100),
                    height: percent(100),
                    padding: {UiRect::top(Val::Px(INPUT_PAD_TOP))},
                }
                TextFont { font: FontSourceTemplate::Handle({echo_font}), font_size: {FontSize::Px(12.0)} }
                TextColor(Color::WHITE)
                // fade with the rest of the screen; the inner EditableText
                // must NOT get a TargetColor (its glyphs stay transparent so
                // the plaintext password is never shown)
                TargetColor(Srgba::WHITE)
                Pickable::IGNORE
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::app::TaskPoolPlugin;
    use bevy::asset::AssetPlugin;
    use bevy::clipboard::ClipboardPlugin;
    use bevy::input::keyboard::{Key, KeyCode, KeyboardInput};
    use bevy::input::{ButtonState, InputPlugin};
    use bevy::input_focus::{FocusCause, InputDispatchPlugin, InputFocus};
    use bevy::picking::events::{PointerRelease, PointerState};
    use bevy::scene::{ScenePlugin, WorldSceneExt};
    use bevy::text::{apply_text_edits, FontCx, LayoutCx};
    use bevy::time::TimePlugin;
    use bevy::ui::UiScale;
    use bevy::ui_widgets::TextInputPlugin;
    use bevy::window::{Ime, PrimaryWindow, Window};

    /// Bevy 0.20 split the text field in two: `EditableText` holds the buffer,
    /// `TextInput` marks the entity as a text input, and all eight keyboard,
    /// pointer and IME observers in `bevy_ui_widgets::text_input` filter on
    /// `With<TextInput>`. An `EditableText` without it still compiles, still
    /// renders and still takes focus — and swallows every keystroke. No
    /// compiler error and no failing build; only typing into the login field
    /// finds it.
    ///
    /// The gate is therefore a keystroke, not a component check: focus the
    /// field this module builds, write one `KeyboardInput`, let
    /// `apply_text_edits` run, read the buffer back. Measured red control:
    /// with `TextInput` removed from `text_input()` the buffer stays `""`
    /// where the test wants `"a"`.
    ///
    /// The plugin list is the minimum that makes bevy's own text-input
    /// observers runnable headless — Bevy fails parameter validation rather
    /// than skipping a system, so every `Res` and message they take has to be
    /// here or the test panics instead of failing.
    #[test]
    fn a_keystroke_reaches_the_text_input_widget() {
        let mut app = App::new();
        app.add_plugins((
            TaskPoolPlugin::default(),
            TimePlugin,
            AssetPlugin::default(),
            ScenePlugin,
            InputPlugin,
            InputDispatchPlugin,
            ClipboardPlugin,
            TextInputPlugin,
        ))
        .add_message::<Ime>()
        .add_message::<PointerRelease>()
        .init_resource::<InputFocus>()
        .init_resource::<PointerState>()
        .init_resource::<UiScale>()
        .init_resource::<FontCx>()
        .init_resource::<LayoutCx>()
        .add_systems(PostUpdate, apply_text_edits);

        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let field = app
            .world_mut()
            .spawn_scene(text_input(Handle::default(), 0))
            .expect("the widget scene must spawn")
            .id();

        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(field, FocusCause::Navigated);
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::KeyA,
            logical_key: Key::Character("a".into()),
            state: ButtonState::Pressed,
            text: Some("a".into()),
            repeat: false,
            window,
        });
        app.update();

        let typed = app
            .world()
            .get::<EditableText>(field)
            .expect("the widget must carry EditableText")
            .value()
            .to_string();
        assert_eq!(typed, "a", "the keystroke never reached the field");
    }
}
