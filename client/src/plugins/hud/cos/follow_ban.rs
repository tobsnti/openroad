//! The server's `0xB074` refusals that belong to the COS group — above all
//! "when a transport is summoned, you cannot use the follow function".
//!
//! Idea: the original does not block the follow order locally. The
//! `UIIT_MSG_COS_BAN_FOLLOW_COS` string is reachable only through the client's
//! message router, in message category `0x19`, whose error space is
//! `0x4001..=0x4005`: `0x4001`/`0x4002` show `UIIT_SKILL_USE_FAIL_SEALED`,
//! `0x4003`/`0x4004` show nothing, `0x4005` shows the follow ban. Category
//! `0x19` is passed by exactly one handler, the `0xB074` `result == 3` arm
//! ([`ObjectActionResponse::Failed`]). So the vanilla surface for this rule is
//! an arriving refusal, not a client-side pre-check.
//!
//! The banned "follow" is the player's auto-trace (`0x7074 Execute(Trace)`,
//! `hud/action.rs`), not the COS command bar's follow order — that one is a
//! `0x70C5` movement whose errors arrive on category `4`/`0x0C`.
//!
//! `plugins::combat` also reads `0xB074` and only logs the refusal; Bevy
//! messages are broadcast, so this second reader is additive. Whoever gives
//! the combat reader a player-visible line must check that one refusal does not
//! produce two chat lines.

use bevy::prelude::*;

use packets::agent::ingame::ObjectActionResponse;

use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::textdata::ClientUiStrings;

/// `0x4001`/`0x4002` — shown as `UIIT_SKILL_USE_FAIL_SEALED`.
pub const ACTION_REFUSAL_SEALED_A: u16 = 0x4001;
pub const ACTION_REFUSAL_SEALED_B: u16 = 0x4002;
/// `0x4003`/`0x4004` — the router's empty-string arm: the original shows
/// nothing. Named so a future reader does not "fix" the silence.
pub const ACTION_REFUSAL_SILENT_A: u16 = 0x4003;
pub const ACTION_REFUSAL_SILENT_B: u16 = 0x4004;
/// `0x4005` — "when a transport is summoned, you cannot use the follow
/// function".
pub const ACTION_REFUSAL_TRANSPORT_FOLLOW_BAN: u16 = 0x4005;

/// `(key, shipped English)`. The fallback is the string the key carries in
/// `Media/server_dep/silkroad/textdata/textuisystem.txt` (UTF-16LE,
/// tab-separated, key column 2, English column 10), quoted with its line so a
/// wrong transcription shows up against the file instead of hiding in a
/// literal. A loaded [`ClientUiStrings`] always wins.
type UiText = (&'static str, &'static str);

/// L538.
const BAN_FOLLOW_TEXT: UiText = (
    "UIIT_MSG_COS_BAN_FOLLOW_COS",
    "When a transport is summoned, you cannot use the follow function.",
);
/// L1611.
const SEALED_TEXT: UiText = (
    "UIIT_SKILL_USE_FAIL_SEALED",
    "Unable to use that skill at this moment",
);

/// The textuisystem key the original's router picks for a `0xB074`
/// `result == 3` error code, or `None` where it deliberately shows no message
/// (`0x4003`/`0x4004`) or has no arm at all (everything outside
/// `0x4001..=0x4005` falls to the router's default).
pub fn action_refusal_text(code: u16) -> Option<UiText> {
    match code {
        ACTION_REFUSAL_SEALED_A | ACTION_REFUSAL_SEALED_B => Some(SEALED_TEXT),
        ACTION_REFUSAL_TRANSPORT_FOLLOW_BAN => Some(BAN_FOLLOW_TEXT),
        _ => None,
    }
}

/// Say why the server refused the action instead of leaving the player with a
/// dead button: today the auto-trace action (`hud/action.rs`, `CommandID`
/// 1003) sends `0x7074 Trace`, the server answers `03 <code> 05 40` while a
/// transport is out, and the refusal reaches a `warn!` nobody sees.
pub fn report_action_refusals(
    mut reader: MessageReader<ObjectActionResponse>,
    mut history: Option<ResMut<ChatHistory>>,
    strings: Option<Res<ClientUiStrings>>,
) {
    let default_strings = ClientUiStrings::default();
    for msg in reader.read() {
        let ObjectActionResponse::Failed { code, error } = msg else {
            continue;
        };
        let strings = strings.as_deref().unwrap_or(&default_strings);
        match action_refusal_text(*error) {
            Some((key, fallback)) => {
                let text = strings.get_or(key, fallback).to_string();
                match &mut history {
                    Some(history) => history.push(ChatLine::system(text)),
                    None => info!("cos (headless): {text}"),
                }
            }
            None => debug!(
                "cos: 0xB074 refusal code {code} error {error:#06x} — the original shows no message"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_category_0x19_table_is_the_five_router_slots() {
        assert_eq!(
            action_refusal_text(ACTION_REFUSAL_TRANSPORT_FOLLOW_BAN).map(|t| t.0),
            Some("UIIT_MSG_COS_BAN_FOLLOW_COS")
        );
        for sealed in [ACTION_REFUSAL_SEALED_A, ACTION_REFUSAL_SEALED_B] {
            assert_eq!(
                action_refusal_text(sealed).map(|t| t.0),
                Some("UIIT_SKILL_USE_FAIL_SEALED")
            );
        }
        for silent in [ACTION_REFUSAL_SILENT_A, ACTION_REFUSAL_SILENT_B] {
            assert!(action_refusal_text(silent).is_none());
        }
        // Outside the space: the router's default arm. 0x300F is a family-4
        // (0xB070) code and must NOT be answered here.
        for outside in [0x4000u16, 0x4006, 0x300F, 0] {
            assert!(action_refusal_text(outside).is_none());
        }
    }

    fn app() -> App {
        let mut app = App::new();
        // A MessageReader on an unregistered type panics in a test app.
        app.add_message::<ObjectActionResponse>()
            .init_resource::<ChatHistory>()
            .add_systems(Update, report_action_refusals);
        app
    }

    fn lines(app: &App) -> Vec<String> {
        app.world()
            .resource::<ChatHistory>()
            .iter()
            .map(|line| line.text.clone())
            .collect()
    }

    /// The acceptance: an arriving refusal with the follow-ban code puts the
    /// original's own sentence on screen. Positive control in the same world:
    /// `0x4004` stays silent, and a success ack says nothing.
    #[test]
    fn the_transport_follow_ban_reaches_the_chat_and_the_silent_code_does_not() {
        let mut app = app();
        app.world_mut().write_message(ObjectActionResponse::Failed {
            code: 0,
            error: ACTION_REFUSAL_TRANSPORT_FOLLOW_BAN,
        });
        app.update();
        assert_eq!(lines(&app), vec![BAN_FOLLOW_TEXT.1.to_string()]);

        app.world_mut().write_message(ObjectActionResponse::Failed {
            code: 0,
            error: ACTION_REFUSAL_SILENT_B,
        });
        app.world_mut()
            .write_message(ObjectActionResponse::Started { code: 1 });
        app.update();
        assert_eq!(lines(&app).len(), 1);
    }

    /// Rule 7: this system takes HUD resources, so its registration must sit
    /// inside the module's scene-gated `Update` tuple. `cargo build`/`make ci`
    /// cannot see that, hence the text assertion.
    #[test]
    fn the_refusal_reader_is_registered_inside_the_scene_gate() {
        let src = include_str!("mod.rs");
        let at = src
            .find("follow_ban::report_action_refusals")
            .expect("the system is registered");
        // The *scene* gate, not the first `.run_if(` in the file: an earlier merge
        // put a `run_if(not(text_field_focused))` on `toggle_cos_window` above
        // this system, and searching for any `.run_if(` then matched that one
        // and failed a registration that is perfectly inside the tuple.
        let gate = src
            .find("in_state(SceneState::GameWorld)")
            .expect("the Update tuple is scene-gated");
        assert!(
            at < gate,
            "the refusal reader must be inside the scene-gated tuple"
        );
    }
}
