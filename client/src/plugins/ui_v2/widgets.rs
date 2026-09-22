use bevy::input_focus::tab_navigation::TabIndex;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, FontSourceTemplate, TextCursorStyle};
use bevy::ui_widgets::Button;

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
    label_sized(text, font, FontSize::Px(font_size))
}

/// [`label`] with the size given as a [`FontSize`] rather than logical pixels.
///
/// Exists because a surface whose geometry is expressed in viewport units
/// (`Val::Vh`, the character-creation screen) needs its text in the *same*
/// unit — a `FontSize::Px` caption inside a `Val::Vh` box is the one part that
/// would not grow with the window, which is exactly the defect the viewport
/// units were introduced to fix. Every existing caller keeps passing pixels
/// through [`label`], so this is an added entry point, not a changed one.
pub fn label_sized(text: &str, font: Handle<Font>, font_size: FontSize) -> impl Scene {
    let text = text.to_string();
    bsn! {
        Text({text})
        TextFont { font: FontSourceTemplate::Handle({font}), font_size: {font_size} }
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

/// Single-line text input, left-aligned. Click to focus, typing/caret/selection
/// handled by bevy's `EditableTextInputPlugin`.
///
/// Left is the alignment every *in-game* input wants (chat, quantity, name
/// entry), so it stays the default here — the login screen's centred rows go
/// through [`text_input_justified`] instead, which is what keeps this widget's
/// HUD call sites untouched.
pub fn text_input(font: Handle<Font>, tab: i32) -> impl Scene {
    text_input_justified(font, tab, Justify::Left)
}

/// Single-line text input with an explicit horizontal alignment of the value.
///
/// # The idea
///
/// The original's login rows draw their *content* centred: `GDR_EDIT_ID` /
/// `GDR_EDIT_PASS` (`pstitle_europe.txt:63`, `:44`) carry `HAlign=1`, and the
/// original draws it that way: the typed value sits centred in the field, not
/// flush left. So alignment is a per-call-site property of
/// this widget, not a global style: the HUD's chat/quantity/name rows are
/// left-aligned and must stay that way, which is why [`text_input`] keeps
/// `Justify::Left` and only the intro passes `Justify::Center`.
pub fn text_input_justified(font: Handle<Font>, tab: i32, justify: Justify) -> impl Scene {
    bsn! {
        EditableText { visible_lines: {Some(1.0)}, allow_newlines: false }
        TextLayout::justify(justify)
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
///
/// Two properties here come **from the original**, not from taste — do not
/// "tidy" them away:
/// * the echo is **one `*` per character** (three characters -> three stars),
///   which is what `update_password_echo` in `ui_v2/mod.rs` produces — no fixed
///   number of stars, no per-character width padding.
/// * the echo is **centred** in the field, like the ID row's value, matching
///   `HAlign=1` on `GDR_EDIT_PASS` (`pstitle_europe.txt:44`).
///
/// This widget only has the login screen as a call site, so the centring is set
/// here rather than being a parameter.
pub fn password_input<M: Component + Default + Clone + Unpin>(
    font: Handle<Font>,
    tab: i32,
) -> impl Scene {
    let echo_font = font.clone();
    bsn! {
        Node { width: percent(100), height: percent(100) }
        Children [
            (
                M
                EditableText { visible_lines: {Some(1.0)}, allow_newlines: false }
                // centred so the (invisible) glyphs and therefore the caret sit
                // where the asterisk echo below draws them
                TextLayout::justify(Justify::Center)
                Node {
                    width: percent(100),
                    height: percent(100),
                    padding: {UiRect::top(Val::Px(INPUT_PAD_TOP))},
                }
                TextFont { font: FontSourceTemplate::Handle({font}), font_size: {FontSize::Px(12.0)} }
                TextColor(Color::NONE)
                TextCursorStyle { color: Color::WHITE }
                TabIndex({tab})
            ),
            (
                PasswordEcho
                Text("")
                TextLayout::justify(Justify::Center)
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
            ),
        ]
    }
}
