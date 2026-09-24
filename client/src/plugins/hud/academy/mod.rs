//! The Academy ("Training Camp") member panel — `resinfo/ifapprenticeship.txt`'s
//! `GDR_APPRENTICESHIP` page, and the two openers that reach it.
//!
//! Idea: the Academy was a *reachability* hole, not a rendering one. The tree
//! already carried the chat channel (`hud/chat`), the guardian-appraisal
//! dialog (`hud/academy_appraisal.rs`), the world-map marker kind and the
//! three wire requests (`packets/src/agent/academy.rs`) — but there was no
//! window, so `KeyAcademy` (id 3033, default `L`, offered by the Key Map tab
//! since the keymap landed) did nothing, and the under-bar MENU row "Academy
//! ( L )" answered in chat that it was not built. This module is that window
//! plus its two openers, so the key and the menu row now reach something.
//!
//! What this panel can and cannot show, stated rather than papered over: the
//! member roster has **no wire source in this tree**. `0x3C81`, the academy
//! info push, is deliberately unwired because the original's own handler
//! reads zero bytes of it — there is no layout to model. So the seven slots
//! render empty and the message board says so with the client's own shipped
//! sentence; inventing a roster decode to fill them is exactly the unsourced
//! value ADR-0009 forbids.
//!
//! Layout, art and colours are transcribed from the shipped resources
//! (`resinfo/ifapprenticeship.txt`, `resinfo/ifapprenticeshipslot.txt`);
//! every constant in [`ui`] names the line it comes from.

pub mod ui;

use bevy::prelude::*;

use crate::plugins::hud::chat::model::ChatState;
use crate::plugins::settings::keymap::KEY_ACADEMY;
use crate::plugins::settings::options::GameOptions;

/// Open/closed state of the member panel.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub struct AcademyState {
    pub open: bool,
}

/// `KeyAcademy` (id 3033, `L` by default) toggles the panel.
///
/// The binding comes from the client's own string table
/// (`UIIT_CTL_TC_SHORTKEY_L` = "Academy ( L )") and was already in
/// `KEY_ACTIONS`; nothing consumed the id, which is the same dead-wire shape
/// `hud/action.rs` fixed for `KeyAction`.
pub fn toggle_academy_window(
    keys: Res<ButtonInput<KeyCode>>,
    chat: Res<ChatState>,
    options: Res<GameOptions>,
    mut state: ResMut<AcademyState>,
) {
    let Some(key) = options.key_for(KEY_ACADEMY) else {
        return;
    };
    if keys.just_pressed(key) && !chat.input_open {
        state.open = !state.open;
    }
}

/// Self-registration for the academy member panel (house pattern of #558).
pub struct AcademyWindowPlugin;

impl Plugin for AcademyWindowPlugin {
    fn build(&self, app: &mut App) {
        use crate::scenes::SceneState;

        app.init_resource::<AcademyState>()
            .add_systems(OnEnter(SceneState::GameWorld), ui::spawn_academy_window)
            .add_systems(OnExit(SceneState::GameWorld), ui::cleanup_academy_window)
            .add_systems(
                Update,
                (
                    toggle_academy_window
                        .run_if(not(crate::plugins::settings::keymap::text_field_focused)),
                    ui::apply_academy_window_visibility,
                )
                    .chain()
                    // Narrower than `hud_scenes` on purpose: the panel is
                    // spawned `OnEnter(GameWorld)` only, so in the offline
                    // preview scenes the hotkey would flip a state no window
                    // reads — a dead key rather than a working one.
                    .run_if(in_state(SceneState::GameWorld)),
            );
    }
}

#[cfg(test)]
mod tests {
    /// A defect only a real client start shows, and no build, test or
    /// `make ci` run can: an ungated `Update` system that asks for a scene
    /// resource (`FontAssets`, `ClientUiStrings`) runs in the loading screen,
    /// fails parameter validation and takes the process down.
    /// So the gate is asserted on the registration text itself, exactly like
    /// `hud/job/ranking.rs`'s `both_job_windows_are_gated_on_the_world_scene`.
    #[test]
    fn the_academy_window_is_gated_on_the_world_scene() {
        for (module, source) in [
            ("academy/mod.rs", include_str!("mod.rs")),
            ("academy/ui.rs", include_str!("ui.rs")),
        ] {
            let registration = source
                .split("#[cfg(test)]")
                .next()
                .expect("split always yields a first part");
            if module == "academy/mod.rs" {
                assert!(
                    registration.contains(".run_if(in_state(SceneState::GameWorld))"),
                    "{module} registers an Update system without the world-scene gate"
                );
                assert!(
                    !registration.contains("super::hud_scenes"),
                    "{module}: the panel only exists in GameWorld, so the wider \
                     hud_scenes gate would leave the hotkey dead in the preview scenes"
                );
                assert!(
                    registration.contains("OnExit(SceneState::GameWorld)"),
                    "{module} leaves its window up when the world scene ends"
                );
            }
            assert!(
                !registration.contains("Option<Res<FontAssets>>"),
                "{module}: the Option pattern is the net side's answer, not a window's"
            );
        }
    }

    /// Typing a capital `L` into the party-match title box used to open this
    /// window, because the toggle only knew the chat guard. It must carry the
    /// general text-field guard every other keybind toggle carries
    /// (`settings::keymap::text_field_focused`).
    #[test]
    fn the_academy_toggle_respects_a_focused_text_field() {
        let registration = include_str!("mod.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("split always yields a first part");
        let update = registration
            .find("Update,")
            .expect("the plugin registers Update systems");
        let after = &registration[update..];
        let toggle = after
            .find("toggle_academy_window")
            .expect("the toggle is registered");
        let after = &after[toggle..];
        let guard = after
            .find("text_field_focused")
            .expect("the toggle carries no text-field guard");
        let next = after
            .find("ui::apply_academy_window_visibility")
            .expect("the visibility system follows the toggle");
        assert!(
            guard < next,
            "the text-field guard must sit on the toggle itself"
        );
    }
}
