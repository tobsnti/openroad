//! A keyboard focus you can see.
//!
//! IDEA. Bevy already holds both halves of the answer, so this only has to draw
//! them: [`InputFocus`](bevy::input_focus::InputFocus) says *what* has focus,
//! and [`InputFocusVisible`](bevy::input_focus::InputFocusVisible) says whether
//! the player arrived there with the keyboard — tab navigation sets it, a mouse
//! click clears it (`bevy_input_focus::tab_navigation`). One system mirrors the
//! pair onto an [`Outline`], which bevy draws *outside* a node's box and keeps
//! out of layout: no widget has to reserve room for the ring, nothing moves
//! when it appears, and the art underneath (`update_image_button_visuals`)
//! keeps its hover/press/disable states untouched.
//!
//! **Why an outline and not the original's own focus art.** The original ships
//! `button_europe.ddj`, `button_europe_focus.ddj` and `button_europe_press.ddj`
//! for these controls, and its `_focus` frame differs from `_normal` only
//! *inside* the gold frame: measured on the user's own PK2, 2211 of 3680 pixels
//! change, all of them within x 6..84 / y 6..33, mean delta +4/+46/+82 — it
//! floods the fill blue. That frame is not available to us as a focus state,
//! because it is already our *hover* art: the archive ships no `_hover`, so
//! `ImageButtonStyle::hover` is loaded from `_focus` at every call site (see
//! [`super::style::ImageButtonStyle`]). The ring is therefore a deliberate
//! addition, not a reproduction — and it is the one treatment that also works
//! for the controls with no art at all (the ID/PW fields), that cannot collide
//! with any of the four art frames, and that costs no layout.
//!
//! The white is not a new number: it is the colour the caret in these same
//! widgets already uses (`TextCursorStyle { color: Color::WHITE }` in
//! [`super::widgets`]), so the two keyboard markers read as one thing.

use bevy::input_focus::{IsFocused, IsFocusedHelper};
use bevy::prelude::*;

/// One pixel, one pixel clear of the control: the gap keeps the ring off the
/// art's own frame line, which is where it would otherwise be hardest to see.
const RING_WIDTH: Val = Val::Px(1.0);
const RING_OFFSET: Val = Val::Px(1.0);
/// The caret colour of the text widgets — see the module docs.
const RING_COLOR: Color = Color::WHITE;

/// Opt-in marker: this node shows the keyboard focus ring when it, or anything
/// inside it, holds a *visible* input focus.
///
/// The hidden [`Outline`] comes with the marker rather than from each call
/// site, and the ring is switched by colour instead of by inserting and
/// removing the component — bevy's own advice for a frequently toggled outline
/// (`bevy_ui::Outline::color`), because it avoids an archetype move per focus
/// change.
#[derive(Component, Default, Clone)]
#[require(Outline = hidden_ring())]
pub struct FocusRing;

fn hidden_ring() -> Outline {
    Outline {
        width: RING_WIDTH,
        offset: RING_OFFSET,
        color: Color::NONE,
    }
}

/// Lights the ring of whichever [`FocusRing`] node holds the visible focus.
///
/// `is_focus_within_visible` rather than `is_focused`: the password field keeps
/// its `TabIndex` on the inner `EditableText`, so the entity that owns the box
/// is the *parent* of the focused one. Reading focus through
/// [`IsFocusedHelper`] also keeps the system alive where bevy's
/// `InputFocusPlugin` is not built (the headless harnesses): its resources are
/// `Option<Res<_>>`, so a missing one leaves every ring dark instead of failing
/// parameter validation.
pub(super) fn update_focus_ring(
    focus: IsFocusedHelper,
    mut rings: Query<(Entity, &mut Outline), With<FocusRing>>,
) {
    for (entity, mut outline) in rings.iter_mut() {
        let wanted = if focus.is_focus_within_visible(entity) {
            RING_COLOR
        } else {
            Color::NONE
        };
        if outline.color != wanted {
            outline.color = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input_focus::{FocusCause, InputFocus, InputFocusVisible};

    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<InputFocus>()
            .init_resource::<InputFocusVisible>()
            .add_systems(Update, update_focus_ring);
        app
    }

    fn ring_color(app: &App, entity: Entity) -> Color {
        app.world().entity(entity).get::<Outline>().unwrap().color
    }

    /// The defect this slice closes, as a test: a control that has the
    /// keyboard focus used to look exactly like one that has not.
    #[test]
    fn the_visibly_focused_control_shows_the_ring() {
        let mut app = app();
        let entity = app.world_mut().spawn((Node::default(), FocusRing)).id();
        app.world_mut().resource_mut::<InputFocusVisible>().0 = true;
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(entity, FocusCause::Navigated);
        app.update();
        assert_eq!(ring_color(&app, entity), RING_COLOR);
    }

    /// A click focuses too, and clears `InputFocusVisible` while doing it
    /// (`tab_navigation::click_to_focus`). Mouse users must not get a ring.
    #[test]
    fn focus_without_visibility_stays_dark() {
        let mut app = app();
        let entity = app.world_mut().spawn((Node::default(), FocusRing)).id();
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(entity, FocusCause::Pressed);
        app.update();
        assert_eq!(ring_color(&app, entity), Color::NONE);
    }

    /// The password field's `TabIndex` sits on the inner `EditableText`, so the
    /// ring has to follow focus *within* — this is the arm that makes the ring
    /// appear on the box the player sees rather than on nothing.
    #[test]
    fn focus_on_a_child_lights_the_wrapper() {
        let mut app = app();
        let wrapper = app.world_mut().spawn((Node::default(), FocusRing)).id();
        let inner = app
            .world_mut()
            .spawn((Node::default(), ChildOf(wrapper)))
            .id();
        app.world_mut().resource_mut::<InputFocusVisible>().0 = true;
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(inner, FocusCause::Navigated);
        app.update();
        assert_eq!(ring_color(&app, wrapper), RING_COLOR);
    }

    /// Losing the focus has to put the ring out again, or the screen ends up
    /// claiming two focused controls.
    #[test]
    fn moving_the_focus_on_puts_the_old_ring_out() {
        let mut app = app();
        let first = app.world_mut().spawn((Node::default(), FocusRing)).id();
        let second = app.world_mut().spawn((Node::default(), FocusRing)).id();
        app.world_mut().resource_mut::<InputFocusVisible>().0 = true;
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(first, FocusCause::Navigated);
        app.update();
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(second, FocusCause::Navigated);
        app.update();
        assert_eq!(ring_color(&app, first), Color::NONE);
        assert_eq!(ring_color(&app, second), RING_COLOR);
    }

    /// The headless harnesses do not build `InputFocusPlugin`, and bevy does
    /// not skip a system whose `Res` is missing — it fails parameter validation
    /// and takes the schedule down. `IsFocusedHelper` reads both resources as
    /// `Option`, so this must simply run.
    #[test]
    fn a_world_without_input_focus_still_runs() {
        let mut app = App::new();
        app.add_systems(Update, update_focus_ring);
        let entity = app.world_mut().spawn((Node::default(), FocusRing)).id();
        app.update();
        assert_eq!(ring_color(&app, entity), Color::NONE);
    }
}
