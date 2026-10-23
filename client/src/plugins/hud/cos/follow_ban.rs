//! The refusals the *server* answers a `0x7074` object action with — and the
//! one of them that belongs to this group: "when a transport is summoned, you
//! cannot use the follow function".
//!
//! Idea: the original does **not** block the follow order locally. Its
//! `UIIT_MSG_COS_BAN_FOLLOW_COS` string is reachable from exactly one place in
//! the whole binary, and that place is the message router the server's error
//! codes flow through — so the vanilla surface for this rule is an arriving
//! `0xB074` refusal, not a client-side pre-check. The measurement is in the
//! local RE notes (`openroad-re/.../docs/re/systems/cos.md` §7, which records
//! `nrefs = 1` reachable only from the error → text router `007b8b40`,
//! category `0x19`, code `0x4005`); RE results stay out of this repository by
//! policy, so what is quoted here is the conclusion and where it lives, not
//! the disassembly.
//!
//! The category's error space is `0x4001..=0x4005` over a five-slot table:
//! `0x4001`/`0x4002` → `UIIT_SKILL_USE_FAIL_SEALED`, `0x4003`/`0x4004` → an
//! **empty** string (the original shows nothing), `0x4005` → the transport
//! follow ban.
//!
//! Capture, measured here rather than taken over: across the five
//! `packet_dump/` trees on this machine `0xb074.log` holds **68** `Failed`
//! frames and every one carries `error = 0x4004` — precisely the silent arm,
//! which is why the original said nothing for those and why a log-only
//! handling matched it so far. `0x4005` appears in **no** capture, so this
//! file's one visible sentence rests on the decompile alone; that is stated,
//! not hidden.
//!
//! Why this lives in the COS group rather than in `cos_command.rs`: the banned
//! "follow" is the **player's** auto-trace (`0x7074`, sent by `hud/action.rs`
//! for the action window's `CommandID` 1003), not the COS command bar's follow
//! order — that one is a `0x70C5` movement whose errors arrive on another
//! category. Putting the sentence into the bar's follow arm would attach a
//! vanilla string to the wrong command *and* invent the condition. What this
//! group owns is the reason a COS makes an action fail, so the reader lives
//! here.
//!
//! Why a second `MessageReader` on `0xB074`: `plugins::combat` already reads
//! the ack for the engagement state and puts this refusal in a `warn!` only
//! (`combat/mod.rs:484`). Bevy messages are broadcast, so a reader that turns
//! the two *named* codes into the original's own sentences is additive.
//!
//! **Maintenance note, because no test can catch it:** `0xB074` is read in two
//! places — here and `combat::on_object_action_response`. Whoever gives that
//! one a player-visible line must check that the same refusal does not produce
//! **two** chat lines.
//!
//! Placement, and the one name this port could not resolve `[U]`: the old file
//! said the table belongs next to `packets::agent::prelude::action_error_text_key`.
//! **No such item exists in this tree** — it was never published, and
//! inventing it would be a guess about another crate's shape. What does exist
//! is `packets::agent::ingame::ACTION_ERROR_INVALID_TARGET`/`_INVALID_DISTANCE`
//! (`ingame.rs:1074-1075`), whose own doc calls the resolving read "the
//! client's message-id table" — which is exactly the table below. Moving it
//! there later is a rename, not a re-measurement.

use bevy::prelude::*;

use packets::agent::ingame::ObjectActionResponse;

use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::textdata::ClientUiStrings;

/// `0x4001`/`0x4002` — the two codes the router answers with
/// `UIIT_SKILL_USE_FAIL_SEALED`.
pub const ACTION_REFUSAL_SEALED_A: u16 = 0x4001;
pub const ACTION_REFUSAL_SEALED_B: u16 = 0x4002;
/// `0x4003`/`0x4004` — the router's empty-string arm: the original shows
/// **nothing**. Named so a future reader does not "fix" the silence.
pub const ACTION_REFUSAL_SILENT_A: u16 = 0x4003;
pub const ACTION_REFUSAL_SILENT_B: u16 = 0x4004;
/// `0x4005` — "when a transport is summoned, you cannot use the follow
/// function".
pub const ACTION_REFUSAL_TRANSPORT_FOLLOW_BAN: u16 = 0x4005;

/// `(key, shipped English)`. The fallback is what the key carries in the
/// player's own `textdata/textuisystem.txt`, quoted with its line so a wrong
/// transcription shows against the file. A loaded [`ClientUiStrings`] wins.
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
/// dead button: the auto-trace action (`hud/action.rs`, `CommandID` 1003)
/// sends `0x7074`, the server answers `03 <code> 05 40` while a transport is
/// out, and today that refusal reaches a `warn!` nobody sees.
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
            // Not a gap: `0x4003`/`0x4004` reach the router's empty-string arm,
            // i.e. the original deliberately says nothing — and `0x4004` is the
            // code this machine's whole capture corpus consists of (68 of 68
            // frames), so the silence is the *measured* behaviour and must
            // stay. An unknown code has no arm in the router either. Do not
            // "repair" this into a message.
            None => debug!(
                "cos: 0xB074 refusal code {code} error {error:#06x} — the original shows no message"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The five-slot table, including the two entries that point at the
    /// empty-string arm — the silence is measured, not missing.
    #[test]
    fn the_category_0x19_table_is_the_five_measured_slots() {
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
        // Outside the space: the router's default arm. 0x300F is a 0xB070 code
        // and must NOT be answered here.
        for outside in [0x4000u16, 0x4006, 0x300F, 0] {
            assert!(action_refusal_text(outside).is_none());
        }
    }

    fn app() -> App {
        let mut app = App::new();
        // A `MessageReader` on an unregistered type panics in a test app.
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
    /// the captured `0x4004` stays silent, and a success ack says nothing.
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

    /// This system takes HUD resources, so its registration must sit inside the
    /// module's scene-gated `Update` tuple — `cargo build` cannot see that,
    /// hence the text assertion.
    #[test]
    fn the_refusal_reader_is_registered_inside_the_scene_gate() {
        let src = include_str!("mod.rs");
        let at = src
            .find("follow_ban::report_action_refusals")
            .expect("the system is registered");
        let gate = src
            .find("in_state(SceneState::GameWorld).or_else")
            .expect("the Update tuple is scene-gated");
        assert!(
            at < gate,
            "the refusal reader must be inside the scene-gated tuple"
        );
    }
}
