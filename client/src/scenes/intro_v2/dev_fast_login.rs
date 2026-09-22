//! Optional developer fast-login for the intro_v2 scene (config
//! `dev_fast_login`).
//!
//! It drives the exact same packet flow the manual UI does, but from
//! `ClientConfig.dev_fast_login` instead of the widgets — skip the splash, fire
//! the login once the shard list is up, answer the captcha with the configured
//! code, and join the first character. The existing gateway/agent login response
//! handlers and the `SceneState::GameWorld` transition are reused unchanged;
//! these systems only supply the inputs the manual path would have. Every system
//! is gated on [`enabled`].
//!
//! This is **openroad-only** behaviour with zero counterpart in the original —
//! and it is not the original's "Auto Login", which is a login queue.
//! It stays off by default.
//!
//! The once-per-visit guards live in [`FastLoginProgress`] rather than in
//! `Local`s: a `Local` survives leaving and re-entering the intro scene, so a
//! second visit would skip the splash without re-sending the login, and the
//! scene would sit there wedged.

use bevy::prelude::*;

use packets::agent::prelude::{CharacterJoinRequest, CharacterSelectionActionResponse};
use packets::login::{LoginCaptchaChallenge, LoginCaptchaConfirmRequest, LoginRequest};
use packets::Packet;

use crate::net::connection::SilkroadConnection;
use crate::plugins::config::division::DivisionInfo;
use crate::plugins::config::ClientConfig;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::gateway::shard_list::ShardList;
use crate::plugins::net::gateway::GatewayConnection;

use super::captcha::{CaptchaImageV2, CaptchaModal};
use super::character_create::{CharCreateSelection, Race};
use super::character_select::{JoiningCharacter, PendingWorldJoin};
use super::net::LoginCredentials;
use super::server_select::SelectedShardV2;
use super::IntroV2State;

/// Run condition: developer fast-login is turned on — in the config, or for
/// one run through `OPENROAD_FAST_LOGIN=1`.
///
/// Why the env door exists: reaching a post-login screen needs a login, and a
/// switch that lives only in `config.yaml` is the user's own file. Leaving it
/// on there is easy to forget, and the client then drives past the login form
/// into the world on every ordinary start. An env var is the same class of
/// dev-only hook as [`jump_target`]'s `OPENROAD_INTRO_JUMP`: inert unless set,
/// and it leaves the user's file alone.
pub fn enabled(config: Res<ClientConfig>) -> bool {
    config.dev_fast_login.enabled || fast_login_forced()
}

/// `OPENROAD_FAST_LOGIN=1|true|yes` — anything else (including unset) is off.
fn fast_login_forced() -> bool {
    std::env::var("OPENROAD_FAST_LOGIN")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

/// Account the fast-login uses: `OPENROAD_ACCOUNT`/`OPENROAD_PASSWORD` override
/// `config.dev_fast_login`, a blank value counting as unset.
///
/// Same shape and same reason as `NETCHECK_ACCOUNT`/`NETCHECK_PASSWORD` in
/// `crate::netcheck` (and `BOT_ACCOUNT` in `crate::bot`) — with several clients
/// on one account it is regularly already logged in, and its `already connected`
/// rejection is an environment condition, not a result. `config.yaml` is the
/// user's file; a dev run must not edit it.
fn credentials(config: &ClientConfig) -> (String, String) {
    let non_blank = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    (
        non_blank("OPENROAD_ACCOUNT").unwrap_or_else(|| config.dev_fast_login.username.clone()),
        non_blank("OPENROAD_PASSWORD").unwrap_or_else(|| config.dev_fast_login.password.clone()),
    )
}

/// Per-visit progress of the fast-login flow.
///
/// Both flags are one-shot *per visit to the intro scene*, and both are cleared
/// by [`reset_progress`] on entering it. Keeping them in a resource rather than
/// in `Local`s is what rules out two failures:
///
/// - **re-entry dead-lock**: a `Local` survives the scene, so on a second visit
///   the splash would be skipped and the login never re-sent;
/// - **captcha re-fire**: an unguarded handler answers every challenge with the
///   same code, so a rejected code is resent in a loop instead of handing the
///   challenge back to the user.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub struct FastLoginProgress {
    pub login_sent: bool,
    pub captcha_answered: bool,
}

/// Clear the one-shot guards when the intro scene is (re-)entered.
pub fn reset_progress(mut progress: ResMut<FastLoginProgress>) {
    *progress = FastLoginProgress::default();
}

/// Skip the splash screen straight to the login form.
pub fn skip_splash(mut next: ResMut<NextState<IntroV2State>>) {
    next.set(IntroV2State::LoginForm);
}

/// Once the shard list and gateway connection are ready, pick the first
/// operating shard and send the login request with the configured credentials.
/// Fires once per visit to the intro scene ([`FastLoginProgress`]).
pub fn send_login(
    mut progress: ResMut<FastLoginProgress>,
    config: Res<ClientConfig>,
    shard_list: Option<Res<ShardList>>,
    mut selected_shard: ResMut<SelectedShardV2>,
    gateway: Query<&SilkroadConnection, With<GatewayConnection>>,
    division: Res<DivisionInfo>,
    mut commands: Commands,
) {
    if progress.login_sent {
        return;
    }
    let Some(shard_list) = shard_list else {
        return;
    };
    let Ok(conn) = gateway.single() else {
        return;
    };
    // Prefer an operating shard; fall back to the first listed.
    let Some(shard) = shard_list
        .0
        .shards
        .iter()
        .find(|s| s.is_operating)
        .or_else(|| shard_list.0.shards.first())
    else {
        warn!("dev_fast_login: shard list is empty");
        return;
    };

    let (username, password) = credentials(&config);
    selected_shard.0 = Some(shard.id);
    // Remembered for the follow-up agent login (see `net::on_gateway_login_response`).
    commands.insert_resource(LoginCredentials {
        username: username.clone(),
        password: password.clone(),
    });

    let frame = Packet::from(LoginRequest {
        content_id: division.content_id,
        username,
        password,
        shard_id: shard.id,
    })
    .into();
    if let Err(e) = conn.get_sender().send(frame) {
        error!("dev_fast_login: failed to send LoginRequest: {}", e.0);
        return;
    }
    info!(
        "dev_fast_login: sent LoginRequest for '{}' on shard {} ({})",
        credentials(&config).0,
        shard.id,
        shard.name
    );
    progress.login_sent = true;
}

/// Answer the IBUV challenge with the configured code and tear down the modal
/// if the manual path already spawned it.
///
/// Two things this deliberately does **not** do. It does not invent a code: the
/// challenge is a server-generated image, so with no `captcha_answer` configured
/// the only correct move is to leave the modal up and let the user read it — a
/// hardcoded `"1"` cannot survive a real challenge. And it answers at most once
/// per visit: a second challenge means the code was rejected, so resending it
/// would loop forever.
pub fn answer_captcha(
    mut events: MessageReader<LoginCaptchaChallenge>,
    config: Res<ClientConfig>,
    mut progress: ResMut<FastLoginProgress>,
    gateway: Query<&SilkroadConnection, With<GatewayConnection>>,
    modal: Query<Entity, With<CaptchaModal>>,
    mut commands: Commands,
) {
    if events.read().count() == 0 {
        return;
    }
    if progress.captcha_answered {
        info!("dev_fast_login: captcha challenged again — handing it to the modal");
        return;
    }
    let Some(code) = config.dev_fast_login.captcha_answer.clone() else {
        info!("dev_fast_login: no captcha_answer configured — leaving the modal up");
        return;
    };
    let Ok(conn) = gateway.single() else {
        return;
    };
    let frame = Packet::from(LoginCaptchaConfirmRequest { code }).into();
    if let Err(e) = conn.get_sender().send(frame) {
        error!("dev_fast_login: failed to send captcha confirm: {}", e.0);
        return;
    }
    progress.captcha_answered = true;
    // Suppress the manual modal (harmless if it never spawned).
    commands.remove_resource::<CaptchaImageV2>();
    for entity in modal.iter() {
        commands.entity(entity).despawn();
    }
    info!("dev_fast_login: submitted the configured captcha answer");
}

/// `OPENROAD_INTRO_JUMP=region_select|character_create`: after the character
/// list has arrived, jump straight to that sub-screen instead of joining the
/// world.
///
/// Why an env var and not a config key: the race board and the creation screen
/// sit behind a login *and* two mouse clicks, so neither is reachable without
/// driving the whole flow by hand. This is the same class of dev-only hook as
/// `OPENROAD_PREVIEW_SOLO` in the `ui_testing` scene: inert unless set. It has
/// no counterpart in the original client.
fn jump_target() -> Option<IntroV2State> {
    match std::env::var("OPENROAD_INTRO_JUMP").ok()?.as_str() {
        "region_select" => Some(IntroV2State::RegionSelect),
        "character_create" => Some(IntroV2State::CharacterCreate),
        // The line-up is already the state we are in, so there is nothing to
        // *set* — re-setting it would re-run the whole `CharacterList`
        // enter/exit chain (despawn the figures, restart the camera flight).
        // It is only a request to stay, honoured by `join_first_character`.
        "character_list" => None,
        other => {
            warn!("OPENROAD_INTRO_JUMP: unknown target '{other}' (region_select|character_create)");
            None
        }
    }
}

/// `OPENROAD_INTRO_JUMP=character_list`: stay in the line-up instead of joining
/// the world.
///
/// The character line-up is the one pre-game screen a fast-login run *passes
/// through* rather than stops at — the world join fires on the same list
/// response that spawns the figures — so without this it cannot be looked at
/// (see the lobby's sit pose, [`super::lobby_sit`]). Same dev-only hook class as
/// the two jump targets above, inert unless set, no counterpart in the
/// original.
fn stay_in_character_list() -> bool {
    std::env::var("OPENROAD_INTRO_JUMP").is_ok_and(|value| value == "character_list")
}

/// Runs in `CharacterList`: waits for the list response (so the stage, the
/// origin and the agent connection are all up, exactly as after a manual
/// click) and then sets the requested state once.
pub fn jump_to_requested_screen(
    mut commands: Commands,
    time: Res<Time>,
    mut reader: MessageReader<CharacterSelectionActionResponse>,
    mut next: ResMut<NextState<IntroV2State>>,
    mut armed_at: Local<Option<f32>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(target) = jump_target() else {
        return;
    };
    if armed_at.is_none() && reader.read().any(|res| res.characters.is_some()) {
        *armed_at = Some(time.elapsed_secs());
    }
    let Some(armed) = *armed_at else {
        return;
    };
    // `OPENROAD_INTRO_JUMP_DELAY=<seconds>`: wait that long on the line-up
    // before jumping.
    //
    // Why it exists: the jump is armed by the very response that spawns the
    // figures, and the state switch happens before
    // `character_select::on_char_selection_action_response` — which only runs
    // *in* `CharacterList` — ever gets to spawn them, so an immediate jump
    // leaves the stage empty. A delay is the smallest thing that keeps the
    // figures on stage while the board flight runs.
    if time.elapsed_secs() - armed < jump_delay_secs() {
        return;
    }
    if target == IntroV2State::CharacterCreate {
        // The race board is what normally inserts this resource, and it is the
        // one thing a jump skips — without it every jumped-to creation screen
        // is Chinese and the European stage is unreachable (same reason the
        // jump itself exists). Inert unless set.
        let race = jump_race();
        info!("OPENROAD_INTRO_JUMP_RACE: creating as {race:?}");
        commands.insert_resource(CharCreateSelection { race, ..default() });
    }
    info!("OPENROAD_INTRO_JUMP: going to {target:?}");
    next.set(target);
    *done = true;
}

/// `OPENROAD_INTRO_JUMP_RACE=chinese|european`, the race a jumped-to creation
/// screen is entered with; Chinese (the resource's own default) otherwise.
fn jump_race() -> Race {
    match std::env::var("OPENROAD_INTRO_JUMP_RACE").ok().as_deref() {
        Some("european") => Race::EUROPEAN,
        Some("chinese") | None => Race::CHINESE,
        Some(other) => {
            warn!("OPENROAD_INTRO_JUMP_RACE: unknown race '{other}' (chinese|european)");
            Race::CHINESE
        }
    }
}

/// Seconds to linger on the character list before an `OPENROAD_INTRO_JUMP`
/// fires; `0.0` (the default) jumps immediately.
fn jump_delay_secs() -> f32 {
    std::env::var("OPENROAD_INTRO_JUMP_DELAY")
        .ok()
        .and_then(|raw| raw.trim().parse::<f32>().ok())
        .filter(|secs| secs.is_finite() && *secs >= 0.0)
        .unwrap_or(0.0)
}

/// On the character list response, join the first character (mirrors
/// `character_select::on_start_activate`, minus the UI). The existing
/// `on_character_join_response` then transitions to the game world.
pub fn join_first_character(
    pending: Option<Res<PendingWorldJoin>>,
    mut reader: MessageReader<CharacterSelectionActionResponse>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    mut commands: Commands,
) {
    if pending.is_some() {
        return;
    }
    // A dev run asked for a sub-screen (or for the line-up itself); joining
    // the world would drive straight past it.
    if jump_target().is_some() || stay_in_character_list() {
        return;
    }
    let mut first_character = None;
    for res in reader.read() {
        if let Some(characters) = &res.characters {
            if let Some(first) = characters.characters.first() {
                first_character = Some(first.clone());
            }
        }
    }
    let Some(first) = first_character else {
        return;
    };
    let Ok(conn) = conn.single() else {
        return;
    };

    let frame = Packet::from(CharacterJoinRequest {
        character_name: first.name.clone(),
    })
    .into();
    if let Err(e) = conn.get_sender().send(frame) {
        error!(
            "dev_fast_login: failed to send CharacterJoinRequest: {}",
            e.0
        );
        return;
    }
    commands.insert_resource(PendingWorldJoin {
        character_name: first.name.clone(),
    });
    commands.insert_resource(JoiningCharacter(first.clone()));
    info!("dev_fast_login: joining first character '{}'", first.name);
}
