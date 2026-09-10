//! Headless net-check client: drives the full login → join → in-game network
//! roundtrip with no window/GPU, so real server packets can be captured and
//! verified fast (e.g. the group-spawn packets 0x3017/18/19).
//!
//! Idea: reuse the exact framing/handshake/dispatch stack the GUI uses
//! ([`NetworkCorePlugin`] + the `packets` crate) under `MinimalPlugins`, and
//! replace the UI-driven login/join handlers with tiny config-driven ones. The
//! gateway shard-list systems are reused verbatim; everything else is a headless
//! port of the `intro_v2`/`game_scene` packet handlers. Every inbound and
//! outbound frame is dumped (the `packets_in`/`packets_out` trace targets wired
//! in `plugins/net/plugin.rs`), and group-spawn payloads are hex-dumped
//! explicitly.
//!
//! Run with `make netcheck` (or `NETCHECK=1 cargo run -p client`). Credentials
//! come from `config.dev_fast_login` unless `NETCHECK_ACCOUNT` /
//! `NETCHECK_PASSWORD` override them (same shape as `BOT_ACCOUNT`/`BOT_PASSWORD`
//! in [`crate::bot`]) — needed because the configured account is usually the
//! owner's and a second session on it is answered `already connected`, which
//! makes the gate report an environment condition as a code defect. The chosen
//! account and its origin are logged once at startup. The captcha is answered
//! with `dev_fast_login.captcha_answer` when set, first operating shard
//! and first character are chosen automatically.
//!
//! With `NETCHECK_ACTIONS=1` the client also *drives* actions after join — it
//! walks a short square (0x7021 → 0xB021) and selects a nearby entity
//! (0x7045 → 0xB045) — so those C→S/response pairs are captured live without a
//! GUI. Item-use (0x704C) is intentionally not driven here (needs itemdata from
//! Data.pk2; see `docs/planning/ROADMAP_V2.md` Phase 2).

use std::env;
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use bytes::Bytes;
use mac_address::get_mac_address;

use packets::agent::character_data::parse_character_data;
use packets::agent::prelude::{
    CelestialPosition, CharacterDataBody, CharacterDataEnd, CharacterJoinRequest,
    CharacterJoinResponse, CharacterSelectionAction, CharacterSelectionActionRequest,
    CharacterSelectionActionResponse, GameReady, GroupEntitySpawnBegin, GroupEntitySpawnData,
    GroupEntitySpawnEnd, MovementRequest, MovementResponse, SelectEntityRequest,
    SelectEntityResponse, SingleEntitySpawn,
};

use crate::net::entity_spawn::{parse_group_spawn, RefResolver, RefType};
use packets::agent::{AgentLoginRequest, AgentLoginResponse};
use packets::login::{
    LoginCaptchaChallenge, LoginCaptchaConfirmRequest, LoginRequest, LoginResponse,
};
use packets::Packet;

use crate::net::connection::SilkroadConnection;
use crate::net::frame::SilkroadFrame;
use crate::plugins::config::division::DivisionInfo;
use crate::plugins::config::ClientConfig;
use crate::plugins::net::agent::{AgentConnection, AgentConnectionBundle};
use crate::plugins::net::gateway::shard_list::{
    on_shardlist_ping_response, on_shardlist_response, ShardList,
};
use crate::plugins::net::gateway::systems::{init_gateway_service, poll_gateway_connection};
use crate::plugins::net::gateway::GatewayConnection;
use crate::plugins::net::plugin::{NetworkCorePlugin, NetworkState};
use packets::hexdump;

/// Who to log in as, and which character to join. Split out of
/// `config.dev_fast_login` so the same login driver serves two callers: netcheck
/// (credentials from the config) and the remote-controlled bot
/// (`crate::bot`, credentials from the environment, one process per account).
#[derive(Resource, Clone)]
pub struct LoginIdentity {
    pub username: String,
    pub password: String,
    /// Character to join. `None` joins the first one the lobby lists, which is
    /// what netcheck always did.
    pub character: Option<String>,
    pub captcha_answer: Option<String>,
}

/// Login attempt bookkeeping, so a rejected login can be retried.
///
/// A killed session lingers on the server for a while: the very next start of
/// the same account is answered `already connected`, and a one-shot login turns
/// that into a process that is up, idle and silently useless. Retrying is
/// therefore part of the driver, not of the caller.
#[derive(Resource)]
pub struct LoginAttempt {
    /// Elapsed seconds at which the next attempt may fire.
    pub next_at: f64,
    pub attempts: u32,
    /// Set once a login has been accepted; stops further attempts.
    pub accepted: bool,
}

impl Default for LoginAttempt {
    fn default() -> Self {
        Self {
            next_at: 0.0,
            attempts: 0,
            accepted: false,
        }
    }
}

/// Seconds between login attempts. The lingering-session window on the
/// reference server is tens of seconds, so a tight retry only burns attempts.
const LOGIN_RETRY_SECS: f64 = 20.0;
/// Give up after this many, so a wrong password does not hammer the server.
const LOGIN_MAX_ATTEMPTS: u32 = 30;

/// The login → join sequence, without the netcheck-specific dumping and action
/// driving: connect, log in, answer the captcha, list characters, join, send
/// `GameReady`. Requires a [`LoginIdentity`] resource and an already-started
/// gateway connect (`init_gateway_service`).
pub struct LoginDriverPlugin;

impl Plugin for LoginDriverPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoginAttempt>().add_systems(
            Update,
            (
                poll_gateway_connection,
                on_shardlist_ping_response,
                on_shardlist_response,
                netcheck_login,
                netcheck_captcha,
                netcheck_on_login_response,
                netcheck_retry_after_rejection,
                netcheck_request_char_list,
                netcheck_join_first_character,
                netcheck_on_join_response,
                netcheck_game_ready,
            ),
        );
    }
}

/// Reconnect the gateway and re-arm the login after a rejection. The gateway
/// drops the connection right after a failed login, so a retry needs a fresh
/// connect — which is what [`init_gateway_service`] does.
fn netcheck_retry_after_rejection(
    mut attempt: ResMut<LoginAttempt>,
    mut rejections: MessageReader<LoginResponse>,
    time: Res<Time>,
    gateway: Query<Entity, With<GatewayConnection>>,
    mut commands: Commands,
) {
    let mut rejected = false;
    for res in rejections.read() {
        if res.login_error.is_some() {
            rejected = true;
        }
    }
    if !rejected || attempt.accepted || attempt.attempts >= LOGIN_MAX_ATTEMPTS {
        return;
    }
    attempt.next_at = time.elapsed_secs_f64() + LOGIN_RETRY_SECS;
    for gw in gateway.iter() {
        commands.entity(gw).despawn();
    }
    commands.run_system_cached(init_gateway_service);
    info!(
        "login: rejected — retrying in {:.0}s (attempt {} of {})",
        LOGIN_RETRY_SECS, attempt.attempts, LOGIN_MAX_ATTEMPTS
    );
}

/// Where the netcheck login account came from.
///
/// Idea: the smoke is a *gate*, and a gate whose input is invisible in the log
/// is how an environment condition gets read as a result — the config account is
/// usually the owner's and answers `already connected`, which looks exactly like
/// a code defect in the log. So the account and its origin are printed once at
/// startup and the origin is part of the return value, not a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountSource {
    /// `NETCHECK_ACCOUNT` was set.
    Env,
    /// Nothing in the environment; `config.dev_fast_login` decided.
    Config,
}

impl AccountSource {
    fn label(self) -> &'static str {
        match self {
            AccountSource::Env => "NETCHECK_ACCOUNT",
            AccountSource::Config => "config.yaml",
        }
    }
}

/// Pick the account the smoke logs in as: `NETCHECK_ACCOUNT`/`NETCHECK_PASSWORD`
/// override `config.dev_fast_login`, mirroring `BOT_ACCOUNT`/`BOT_PASSWORD` in
/// [`crate::bot`] (same shape, no new configuration layer).
///
/// The environment values are passed in rather than read here so the choice is
/// testable without touching the process environment. A blank value counts as
/// unset: `NETCHECK_ACCOUNT= make netcheck` must not try to log in as "".
/// The two variables are independent — an override of only the password keeps
/// the configured account, which is what a rotated password needs.
fn select_account(
    env_account: Option<String>,
    env_password: Option<String>,
    cfg_account: &str,
    cfg_password: &str,
) -> (String, String, AccountSource, AccountSource) {
    let non_blank = |v: Option<String>| v.filter(|s| !s.trim().is_empty());
    let (username, user_src) = match non_blank(env_account) {
        Some(u) => (u, AccountSource::Env),
        None => (cfg_account.to_string(), AccountSource::Config),
    };
    let (password, pass_src) = match non_blank(env_password) {
        Some(p) => (p, AccountSource::Env),
        None => (cfg_password.to_string(), AccountSource::Config),
    };
    (username, password, user_src, pass_src)
}

/// Build and run the headless net-check app. Blocks until the process is killed.
pub fn run_headless(config: ClientConfig) {
    let (username, password, user_src, pass_src) = select_account(
        env::var("NETCHECK_ACCOUNT").ok(),
        env::var("NETCHECK_PASSWORD").ok(),
        &config.dev_fast_login.username,
        &config.dev_fast_login.password,
    );
    let account_line = format!(
        "netcheck: account {} (from {}), password from {}",
        username,
        user_src.label(),
        if pass_src == AccountSource::Env {
            "NETCHECK_PASSWORD"
        } else {
            pass_src.label()
        }
    );
    let identity = LoginIdentity {
        username,
        password,
        character: None,
        captcha_answer: config.dev_fast_login.captcha_answer.clone(),
    };
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(5))))
        .add_plugins(LogPlugin {
            // Everything at info, plus the per-frame packet dumps at trace.
            level: Level::TRACE,
            filter: "info,packets_in=trace,packets_out=trace".to_string(),
            ..default()
        });
    // Only now is there a subscriber: `LogPlugin::build` installs it, so a line
    // printed before this point is silently dropped.
    info!("{}", account_line);
    app.insert_resource(config)
        .insert_resource(identity)
        .insert_resource(DivisionInfo::load_or_fallback())
        .insert_resource(ActionDriver {
            enabled: env::var("NETCHECK_ACTIONS").is_ok(),
            ..default()
        })
        .insert_resource(OpcodeProbe::from_env())
        .add_plugins(NetworkCorePlugin)
        // Kick off the (asynchronous) gateway connect. `poll_gateway_connection`
        // delivers the established connection and fires the shard-list ping, so
        // `netcheck_login` (which already waits for both the shard list and the
        // connection) proceeds once they arrive.
        .add_systems(Startup, init_gateway_service)
        // the shared login driver (also used by `crate::bot`)
        .add_plugins(LoginDriverPlugin)
        .add_systems(Update, netcheck_dump_group_spawns)
        // Action driver (only active with NETCHECK_ACTIONS=1); every system
        // early-returns when disabled.
        .add_systems(
            Update,
            (
                netcheck_capture_local_uid,
                netcheck_capture_local_pos,
                netcheck_drive_actions,
                netcheck_select_target,
                netcheck_log_action_responses,
                netcheck_fire_probe,
            ),
        )
        .run();
}

/// Once the shard list and gateway connection are ready, send the login request
/// with the configured credentials on the first operating shard. Fires once.
fn netcheck_login(
    mut attempt: ResMut<LoginAttempt>,
    identity: Res<LoginIdentity>,
    shard_list: Option<Res<ShardList>>,
    gateway: Query<&SilkroadConnection, With<GatewayConnection>>,
    division: Res<DivisionInfo>,
    time: Res<Time>,
) {
    if attempt.accepted
        || attempt.attempts >= LOGIN_MAX_ATTEMPTS
        || time.elapsed_secs_f64() < attempt.next_at
    {
        return;
    }
    let Some(shard_list) = shard_list else {
        return;
    };
    let Ok(conn) = gateway.single() else {
        return;
    };
    let Some(shard) = shard_list
        .0
        .shards
        .iter()
        .find(|s| s.is_operating)
        .or_else(|| shard_list.0.shards.first())
    else {
        warn!("netcheck: shard list is empty");
        return;
    };

    let frame = Packet::from(LoginRequest {
        content_id: division.content_id,
        username: identity.username.clone(),
        password: identity.password.clone(),
        shard_id: shard.id,
    })
    .into();
    if let Err(e) = conn.get_sender().send(frame) {
        error!("netcheck: failed to send LoginRequest: {}", e.0);
        return;
    }
    attempt.attempts += 1;
    // Nothing may fire again until either the response accepts (`accepted`) or
    // the rejection handler schedules the next attempt.
    attempt.next_at = f64::INFINITY;
    info!(
        "netcheck: sent LoginRequest for '{}' on shard {} ({}) — attempt {}",
        identity.username, shard.id, shard.name, attempt.attempts
    );
}

/// Answer the captcha with the configured `dev_fast_login.captcha_answer`.
///
/// Headless has no modal to fall back on, so with no code configured this can
/// only say so. Guessing a code would hide the missing configuration.
fn netcheck_captcha(
    mut events: MessageReader<LoginCaptchaChallenge>,
    identity: Res<LoginIdentity>,
    gateway: Query<&SilkroadConnection, With<GatewayConnection>>,
) {
    if events.read().count() == 0 {
        return;
    }
    let Some(code) = identity.captcha_answer.clone() else {
        error!(
            "netcheck: server sent an IBUV captcha but dev_fast_login.captcha_answer is unset; \
             login cannot continue"
        );
        return;
    };
    let Ok(conn) = gateway.single() else {
        return;
    };
    let frame = Packet::from(LoginCaptchaConfirmRequest { code }).into();
    if let Err(e) = conn.get_sender().send(frame) {
        error!("netcheck: failed to send captcha confirm: {}", e.0);
        return;
    }
    info!("netcheck: submitted the configured captcha answer");
}

/// Port of `intro_v2::net::on_gateway_login_response`: open the agent connection
/// from the gateway's login info and send the agent login.
fn netcheck_on_login_response(
    mut reader: MessageReader<LoginResponse>,
    mut attempt: ResMut<LoginAttempt>,
    identity: Res<LoginIdentity>,
    mut network_state: ResMut<NetworkState>,
    gateway: Query<Entity, With<GatewayConnection>>,
    division: Res<DivisionInfo>,
    mut commands: Commands,
) {
    for res in reader.read() {
        if let Some(err) = &res.login_error {
            error!(
                "netcheck: login failed: {} ({:?})",
                packets::login::describe_login_error(err.error_code),
                err.failure()
            );
            continue;
        }
        let Some(info) = res.login_info.clone() else {
            continue;
        };
        attempt.accepted = true;

        if let Ok(gw) = gateway.single() {
            commands.entity(gw).despawn();
            network_state.gateway = false;
        }

        match SilkroadConnection::new(&format!("{}:{}", info.agent_ip, info.agent_port)) {
            Ok(conn) => {
                network_state.agent = true;
                let mac = get_mac_address()
                    .ok()
                    .flatten()
                    .map(|m| m.bytes())
                    .unwrap_or([0; 6]);
                let sender = conn.get_sender();
                let frame = Packet::from(AgentLoginRequest {
                    token: info.agent_token,
                    username: identity.username.clone(),
                    password: identity.password.clone(),
                    content_id: division.content_id,
                    mac_address: mac,
                })
                .into();
                if let Err(e) = sender.send(frame) {
                    error!("netcheck: failed to send AgentLoginRequest: {}", e.0);
                }
                commands.spawn(AgentConnectionBundle::new(conn));
                info!(
                    "netcheck: opened agent connection to {}:{}",
                    info.agent_ip, info.agent_port
                );
            }
            Err(e) => error!("netcheck: failed to connect to agent server: {}", e),
        }
    }
}

/// On a successful agent login, request the character list.
fn netcheck_request_char_list(
    mut reader: MessageReader<AgentLoginResponse>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
) {
    let mut ok = false;
    for res in reader.read() {
        if res.error_code.is_none() {
            ok = true;
        } else {
            if let Some(code) = res.error_code {
                error!(
                    "netcheck: agent login failed: code {} ({})",
                    code,
                    packets::agent::describe_agent_auth_error(code)
                );
            }
        }
    }
    if !ok {
        return;
    }
    let Ok(conn) = conn.single() else {
        return;
    };
    let frame = Packet::from(CharacterSelectionActionRequest {
        action: CharacterSelectionAction::List,
        name: None,
        create: None,
    })
    .into();
    if let Err(e) = conn.get_sender().send(frame) {
        error!("netcheck: failed to request character list: {}", e.0);
        return;
    }
    info!("netcheck: requested character list");
}

/// Which listed character to join: the wanted name (case-insensitive, as the
/// server treats names), or the first listed when none is wanted. A wanted name
/// the account does not own is a configuration error, not a reason to silently
/// play someone else, so it joins nothing.
fn pick_character<'a>(wanted: Option<&str>, listed: &[&'a str]) -> Option<&'a str> {
    match wanted {
        Some(wanted) => listed
            .iter()
            .copied()
            .find(|name| name.eq_ignore_ascii_case(wanted)),
        None => listed.first().copied(),
    }
}

/// Join the character named by [`LoginIdentity`], or the first one the lobby
/// listed when no name is configured. Fires once.
fn netcheck_join_first_character(
    mut fired: Local<bool>,
    mut reader: MessageReader<CharacterSelectionActionResponse>,
    identity: Res<LoginIdentity>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
) {
    if *fired {
        return;
    }
    let mut name = None;
    for res in reader.read() {
        if let Some(characters) = &res.characters {
            info!(
                "lobby: {} character(s): {}",
                characters.characters.len(),
                characters
                    .characters
                    .iter()
                    .map(|c| format!("{} (lv {})", c.name, c.level))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let listed: Vec<&str> = characters
                .characters
                .iter()
                .map(|c| c.name.as_str())
                .collect();
            name = pick_character(identity.character.as_deref(), &listed).map(String::from);
            if let (Some(wanted), None) = (&identity.character, &name) {
                error!("login: character '{}' is not on this account", wanted);
            }
        }
    }
    let Some(name) = name else {
        return;
    };
    let Ok(conn) = conn.single() else {
        return;
    };
    let frame = Packet::from(CharacterJoinRequest {
        character_name: name.clone(),
    })
    .into();
    if let Err(e) = conn.get_sender().send(frame) {
        error!("netcheck: failed to send CharacterJoinRequest: {}", e.0);
        return;
    }
    info!("netcheck: joining first character '{}'", name);
    *fired = true;
}

/// Log the join outcome.
fn netcheck_on_join_response(mut reader: MessageReader<CharacterJoinResponse>) {
    for res in reader.read() {
        if res.result == 1 {
            info!("netcheck: joined the world — now capturing in-game packets (Ctrl-C to stop)");
        } else {
            error!("netcheck: join failed with error {:?}", res.error);
        }
    }
}

/// After the character-data stream ends, send `GameReady` so the server proceeds
/// to stream the surrounding entities. Fires once.
fn netcheck_game_ready(
    mut fired: Local<bool>,
    mut reader: MessageReader<CharacterDataEnd>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
) {
    if reader.read().count() == 0 || *fired {
        return;
    }
    let Ok(conn) = conn.single() else {
        return;
    };
    let frame = Packet::from(GameReady).into();
    if let Err(e) = conn.get_sender().send(frame) {
        error!("netcheck: failed to send GameReady: {}", e.0);
        return;
    }
    info!("netcheck: sent GameReady (0x3012)");
    *fired = true;
}

/// Explicitly surface the group-spawn packets we are verifying, hex-dumping the
/// data payload (cross-check against `net::entity_spawn`).
fn netcheck_dump_group_spawns(
    mut begins: MessageReader<GroupEntitySpawnBegin>,
    mut datas: MessageReader<GroupEntitySpawnData>,
    mut ends: MessageReader<GroupEntitySpawnEnd>,
) {
    for b in begins.read() {
        info!(
            "netcheck: GroupEntitySpawnBegin (0x3017) kind={} count={}",
            b.kind, b.count
        );
    }
    for d in datas.read() {
        info!(
            "netcheck: GroupEntitySpawnData (0x3019) {} bytes: {}",
            d.raw.len(),
            hexdump(&d.raw, 512)
        );
    }
    for _ in ends.read() {
        info!("netcheck: GroupEntitySpawnEnd (0x3018)");
    }
}

// ---------------------------------------------------------------------------
// Action driver (NETCHECK_ACTIONS=1)
//
// Idea: once joined, "play" a little so the server's C->S responses are
// captured live — walk a short square (0x7021 -> 0xB021) and select a nearby
// entity (0x7045 -> 0xB045). Item-use (0x704C) is deliberately NOT driven
// here: resolving a real consumable slot -> packed type_id needs the client's
// itemdata tables (Data.pk2), which the headless harness does not load, so it
// belongs to a GUI play session (see docs/planning/ROADMAP_V2.md, Phase 2).
// ---------------------------------------------------------------------------

/// Shared state for the post-join action sequence.
#[derive(Resource, Default)]
struct ActionDriver {
    enabled: bool,
    local_uid: Option<u32>,
    pos_known: bool,
    region: u16,
    base_x: i32,
    base_y: i32,
    base_z: i32,
    moves_sent: u8,
    /// Wall-clock (elapsed secs) at which the next move may be sent; `None`
    /// until the settle delay after position is known has been scheduled.
    next_move_at: Option<f64>,
    select_sent: bool,
    /// A CHARACTER_DATA body that arrived before the unique id did, kept so the
    /// scan can be retried once `CelestialPosition` names the id.
    pending_body: Option<Bytes>,
}

/// Minimal [`RefResolver`] for headless spawn parsing: with no itemdata tables
/// we cannot classify ref ids, so treat every record as an NPC — the leading
/// `unique_id` + position are read before any itemdata-dependent branch, which
/// is all we need to pick a select target. Player/item/structure records simply
/// fail to parse and are skipped (`parse_group_spawn` never panics), so a wrong
/// guess costs at most a skipped target.
///
/// NPC and **not** monster: the two records are identical up to the monster's
/// trailing rarity byte, so the NPC shape is the common prefix. Guessing
/// "monster" loses every NPC record — the record ends after its talk block and
/// the demanded rarity byte is a short read (`0x3015` single spawns, where the
/// frame's own trailing byte can make it succeed with the *wrong* value, or a
/// genuine truncation at the end of a batch). Guessing "NPC" reads the same
/// uid and position from either kind and leaves a monster's rarity byte for
/// the caller, which the harness ignores anyway.
struct HeadlessResolver;

impl RefResolver for HeadlessResolver {
    fn resolve(&self, _ref_id: u32) -> RefType {
        RefType::Npc
    }
    fn item_is_equipment(&self, _ref_id: u32) -> bool {
        false
    }
}

/// Record the local player's in-world id from the celestial-position packet.
fn netcheck_capture_local_uid(
    mut driver: ResMut<ActionDriver>,
    mut reader: MessageReader<CelestialPosition>,
) {
    if !driver.enabled {
        return;
    }
    for p in reader.read() {
        if driver.local_uid.is_none() {
            driver.local_uid = Some(p.unique_id);
            info!("netcheck: local player unique_id = {}", p.unique_id);
            // The body almost always arrived first (see `netcheck_capture_local_pos`),
            // and its message is gone by now — retry the stashed copy with the id
            // the scan was missing.
            if let Some(raw) = driver.pending_body.take() {
                lock_position(&mut driver, &raw);
            }
        }
    }
}

/// Parse the local player's spawn position out of the CHARACTER_DATA body so we
/// have a valid region + coords to move from. Resolver-free (uses the anchor
/// scanner), so it works without any itemdata.
fn netcheck_capture_local_pos(
    mut driver: ResMut<ActionDriver>,
    mut reader: MessageReader<CharacterDataBody>,
) {
    if !driver.enabled || driver.pos_known {
        return;
    }
    for body in reader.read() {
        if lock_position(&mut driver, &body.raw) {
            continue;
        }
        // No position yet. On the live server the body (0x3013) beats
        // `CelestialPosition` (0x3020) by ~20 ms, so the scan runs
        // WITHOUT a unique id, and the id-less scan deliberately refuses a body
        // that offers more than one plausible position
        // (`packets/src/agent/character_data.rs:696-703`). A real body offers
        // dozens. Keep it until the id arrives instead of dropping it: the
        // message itself does not survive to the next frame.
        driver.pending_body = Some(body.raw.clone());
    }
}

/// Try to pin the driver's spawn position from a CHARACTER_DATA body. Returns
/// whether the position is now known, so the caller can decide whether to stash
/// the body for a retry.
fn lock_position(driver: &mut ActionDriver, raw: &Bytes) -> bool {
    if driver.pos_known {
        return true;
    }
    let Some(pc) = parse_character_data(raw, driver.local_uid) else {
        return false;
    };
    if driver.local_uid.is_none() {
        driver.local_uid = Some(pc.unique_id);
    }
    driver.region = pc.region;
    driver.base_x = pc.x.round() as i32;
    driver.base_y = pc.y.round() as i32;
    driver.base_z = pc.z.round() as i32;
    driver.pos_known = true;
    info!(
        "netcheck: local position region={} ({}, {}, {}) — driving actions",
        pc.region, driver.base_x, driver.base_y, driver.base_z
    );
    true
}

/// Walk a small square around the spawn point (four legs, ~150 region-local
/// units each, paced ~1.5 s apart after a 2 s settle), sending one
/// [`MovementRequest`] per leg.
fn netcheck_drive_actions(
    mut driver: ResMut<ActionDriver>,
    time: Res<Time>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
) {
    if !driver.enabled || !driver.pos_known || driver.moves_sent >= 4 {
        return;
    }
    let now = time.elapsed_secs_f64();
    let due = *driver.next_move_at.get_or_insert(now + 2.0);
    if now < due {
        return;
    }
    let Ok(conn) = conn.single() else {
        return;
    };
    const STEP: i32 = 150;
    let (dx, dz) = match driver.moves_sent {
        0 => (STEP, 0),
        1 => (0, STEP),
        2 => (-STEP, 0),
        _ => (0, -STEP),
    };
    let req = MovementRequest {
        region: driver.region,
        x: driver.base_x + dx,
        y: driver.base_y,
        z: driver.base_z + dz,
    };
    let (region, x, y, z) = (req.region, req.x, req.y, req.z);
    if let Err(e) = conn.get_sender().send(Packet::from(req).into()) {
        error!("netcheck: failed to send MovementRequest: {}", e.0);
        return;
    }
    info!(
        "netcheck: sent MovementRequest #{} -> region={} ({}, {}, {})",
        driver.moves_sent + 1,
        region,
        x,
        y,
        z
    );
    driver.moves_sent += 1;
    driver.next_move_at = Some(now + 1.5);
}

/// Select the first parseable nearby entity from a single-entity spawn, once.
fn netcheck_select_target(
    mut driver: ResMut<ActionDriver>,
    mut reader: MessageReader<SingleEntitySpawn>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
) {
    if !driver.enabled || driver.select_sent {
        return;
    }
    let Ok(conn) = conn.single() else {
        return;
    };
    for spawn in reader.read() {
        let parsed = parse_group_spawn(&spawn.raw, true, 1, &HeadlessResolver);
        let Some(entity) = parsed.spawns.first() else {
            continue;
        };
        if entity.unique_id == 0 {
            continue;
        }
        let uid = entity.unique_id;
        if let Err(e) = conn
            .get_sender()
            .send(Packet::from(SelectEntityRequest { unique_id: uid }).into())
        {
            error!("netcheck: failed to send SelectEntityRequest: {}", e.0);
            return;
        }
        info!(
            "netcheck: sent SelectEntityRequest for nearby entity uid={} at region={}",
            uid, entity.position.region
        );
        driver.select_sent = true;
        break;
    }
}

/// Surface the action responses explicitly (they are also in the raw frame
/// trace, but this annotates the capture).
fn netcheck_log_action_responses(
    driver: Res<ActionDriver>,
    mut moves: MessageReader<MovementResponse>,
    mut selects: MessageReader<SelectEntityResponse>,
) {
    if !driver.enabled {
        return;
    }
    for m in moves.read() {
        if Some(m.unique_id) == driver.local_uid {
            info!(
                "netcheck: MovementResponse (0xB021) local: has_dest={} region={} ({}, {}, {}) angle={}",
                m.has_destination, m.region, m.x, m.y, m.z, m.angle
            );
        }
    }
    for s in selects.read() {
        info!(
            "netcheck: SelectEntityResponse (0xB045) result={} uid={} tail={}B",
            s.result,
            s.unique_id,
            s.tail.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build `[u32 id][u16 region][f32 x][f32 y][f32 z][u16 heading]`.
    fn id_and_position(id: u32, region: u16, x: f32, y: f32, z: f32) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&id.to_le_bytes());
        v.extend_from_slice(&region.to_le_bytes());
        v.extend_from_slice(&x.to_le_bytes());
        v.extend_from_slice(&y.to_le_bytes());
        v.extend_from_slice(&z.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v
    }

    /// The headless resolver must guess NPC, not monster. An NPC record is the
    /// monster record without the trailing rarity byte, so guessing "monster"
    /// demands a byte that is not there and the whole record — and with it the
    /// select target — is lost. The `Monster` arm below is the control: same
    /// bytes, no spawn.
    #[test]
    fn the_headless_resolver_reads_an_npc_record() {
        const NPC_BATCH: &[u8] = &[
            0xd5, 0x07, 0x00, 0x00, 0xf3, 0x00, 0x00, 0x00, 0xa8, 0x61, 0x8f, 0x02, 0xc6, 0x44,
            0x00, 0x00, 0x00, 0x00, 0x48, 0xe9, 0xaf, 0x44, 0xb5, 0x80, 0x00, 0x01, 0x00, 0xb5,
            0x80, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0xc8, 0x42, 0x00, 0x02, 0x02, 0x01, 0x02,
        ];

        let parsed = parse_group_spawn(NPC_BATCH, true, 1, &HeadlessResolver);
        let entity = parsed
            .spawns
            .first()
            .expect("the headless resolver must read an NPC record");
        assert_eq!(entity.unique_id, 243);
        assert_eq!(entity.position.region, 0x61A8);

        struct MonsterGuess;
        impl RefResolver for MonsterGuess {
            fn resolve(&self, _ref_id: u32) -> RefType {
                RefType::Monster
            }
            fn item_is_equipment(&self, _ref_id: u32) -> bool {
                false
            }
        }
        assert!(
            parse_group_spawn(NPC_BATCH, true, 1, &MonsterGuess)
                .spawns
                .is_empty(),
            "the old monster guess loses the record on its missing rarity byte"
        );
    }

    #[test]
    fn a_body_without_the_unique_id_cannot_be_pinned() {
        let mut raw = vec![0u8; 8];
        raw.extend(id_and_position(119_948, 25_000, 800.0, 20.0, 900.0));
        raw.extend(id_and_position(4_242, 24_000, 700.0, 10.0, 600.0));
        let raw = Bytes::from(raw);

        assert!(parse_character_data(&raw, None).is_none());
        assert!(parse_character_data(&raw, Some(119_948)).is_some());
    }

    /// The gate's own input: `NETCHECK_ACCOUNT` must win over `config.yaml`,
    /// because a smoke that always logs in as the owner's account reports
    /// `already connected` — an environment condition read as a result.
    #[test]
    fn netcheck_account_env_overrides_the_config() {
        let (user, pass, user_src, pass_src) = select_account(
            Some("env-user".to_string()),
            Some("env-pw".to_string()),
            "cfg-user",
            "cfg-pw",
        );
        assert_eq!((user.as_str(), pass.as_str()), ("env-user", "env-pw"));
        assert_eq!(user_src, AccountSource::Env);
        assert_eq!(pass_src, AccountSource::Env);
        assert_eq!(user_src.label(), "NETCHECK_ACCOUNT");
    }

    /// Backwards control: with nothing in the environment the configured
    /// account is still used, and the log says so.
    #[test]
    fn netcheck_account_falls_back_to_the_config() {
        let (user, pass, user_src, pass_src) = select_account(None, None, "cfg-user", "cfg-pw");
        assert_eq!((user.as_str(), pass.as_str()), ("cfg-user", "cfg-pw"));
        assert_eq!(user_src, AccountSource::Config);
        assert_eq!(pass_src, AccountSource::Config);
        assert_eq!(user_src.label(), "config.yaml");
    }

    /// `NETCHECK_ACCOUNT= make netcheck` (exported but empty) must not try to
    /// log in as "", and the two variables are independent: a password-only
    /// override keeps the configured account.
    #[test]
    fn netcheck_account_ignores_blanks_and_handles_each_variable_alone() {
        let (user, pass, user_src, pass_src) =
            select_account(Some("   ".to_string()), None, "cfg-user", "cfg-pw");
        assert_eq!((user.as_str(), pass.as_str()), ("cfg-user", "cfg-pw"));
        assert_eq!(
            (user_src, pass_src),
            (AccountSource::Config, AccountSource::Config)
        );

        let (user, pass, user_src, pass_src) =
            select_account(None, Some("rotated".to_string()), "cfg-user", "cfg-pw");
        assert_eq!((user.as_str(), pass.as_str()), ("cfg-user", "rotated"));
        assert_eq!(
            (user_src, pass_src),
            (AccountSource::Config, AccountSource::Env)
        );
    }

    /// The name rule: `BOT_CHAR`/`LoginIdentity::character` matches the lobby
    /// list case-insensitively, an unknown name joins nobody (never "the first
    /// one instead"), and no name at all keeps the old netcheck behaviour.
    #[test]
    fn a_wanted_character_is_matched_case_insensitively_or_not_at_all() {
        let listed = ["Alpha", "Beta"];
        assert_eq!(pick_character(Some("beta"), &listed), Some("Beta"));
        assert_eq!(pick_character(Some("Gamma"), &listed), None);
        assert_eq!(pick_character(None, &listed), Some("Alpha"));
        assert_eq!(pick_character(None, &[]), None);
    }

    /// The body is kept and re-scanned when the id arrives, so an out-of-order
    /// arrival is not fatal.
    #[test]
    fn the_stashed_body_is_pinned_once_the_unique_id_arrives() {
        let mut raw = vec![0u8; 8];
        raw.extend(id_and_position(119_948, 25_000, 800.0, 20.0, 900.0));
        raw.extend(id_and_position(4_242, 24_000, 700.0, 10.0, 600.0));
        let raw = Bytes::from(raw);

        let mut driver = ActionDriver {
            enabled: true,
            ..default()
        };
        // Body first, id unknown: refused, so the caller stashes it.
        assert!(!lock_position(&mut driver, &raw));
        assert!(!driver.pos_known);

        // CelestialPosition names the id; the retry succeeds.
        driver.local_uid = Some(119_948);
        assert!(lock_position(&mut driver, &raw));
        assert!(driver.pos_known);
        assert_eq!(driver.region, 25_000);
        assert_eq!(
            (driver.base_x, driver.base_y, driver.base_z),
            (800, 20, 900)
        );
    }
}

// ---------------------------------------------------------------------------
// Raw opcode probe (NETCHECK_PROBE=<opcode-hex>:<body-hex>[,<opcode>:<body>…])
//
// Idea: some questions about this server can only be answered by sending one
// exact frame and watching what comes back — the item-use gate (0x704C) is the
// standing example. A typed request cannot
// express the negative control the experiment needs (an opcode the server has
// no handler for at all), so this sends a frame built by hand: opcode + body
// bytes, encryption flag 0, exactly like the original's FUN_00841780(op, 0).
//
// It fires once, `NETCHECK_PROBE_DELAY` seconds after the world join (default
// 5), and logs the wire bytes with a millisecond timestamp so the send can be
// lined up against the dumps and against a connection reset.
// ---------------------------------------------------------------------------

/// One parsed `opcode:body` probe plus the schedule state for firing it.
#[derive(Resource, Default)]
struct OpcodeProbe {
    frames: Vec<(u16, Bytes)>,
    delay: f64,
    /// Seconds between two frames of the list. 0 (the default) keeps the
    /// upstream behaviour — the whole list goes out in one tick. A positive
    /// value turns the list into a *sequence*, which is what walking a
    /// character to a fixed world position needs: one 0x7021 per waypoint,
    /// spaced far enough apart that the server has actually moved us before
    /// the next order arrives (a single far order is refused when the straight
    /// line is blocked).
    interval: f64,
    due: Option<f64>,
    sent: bool,
}

impl OpcodeProbe {
    /// Parse `NETCHECK_PROBE`; an unparsable entry is a hard error rather than
    /// a silent skip — a probe that quietly did not fire would be read as
    /// "the server accepted it".
    fn from_env() -> Self {
        let Ok(spec) = env::var("NETCHECK_PROBE") else {
            return Self::default();
        };
        let mut frames = Vec::new();
        for entry in spec.split(',').filter(|s| !s.is_empty()) {
            let (op, body) = entry.split_once(':').unwrap_or((entry, ""));
            let opcode = u16::from_str_radix(op.trim_start_matches("0x"), 16)
                .unwrap_or_else(|e| panic!("NETCHECK_PROBE: bad opcode {op:?}: {e}"));
            let body = body.trim();
            let bytes: Vec<u8> = (0..body.len())
                .step_by(2)
                .map(|i| {
                    u8::from_str_radix(&body[i..(i + 2).min(body.len())], 16)
                        .unwrap_or_else(|e| panic!("NETCHECK_PROBE: bad body {body:?}: {e}"))
                })
                .collect();
            frames.push((opcode, Bytes::from(bytes)));
        }
        Self {
            frames,
            delay: env::var("NETCHECK_PROBE_DELAY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5.0),
            interval: env::var("NETCHECK_PROBE_INTERVAL")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
            ..Self::default()
        }
    }
}

/// The placeholder a probe body uses for "our own in-world unique id": the
/// id is assigned per session, so a body that has to name us cannot be a
/// literal. `FF FF FF FE` is not a plausible id and is replaced at fire time
/// with the id `netcheck_capture_local_uid` read from 0x3020 (so this needs
/// `NETCHECK_ACTIONS=1`, the flag that arms that read).
const PROBE_LOCAL_UID_PLACEHOLDER: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFE];

/// Fire the configured probe frames once, after the join has settled.
fn netcheck_fire_probe(
    mut probe: ResMut<OpcodeProbe>,
    time: Res<Time>,
    driver: Res<ActionDriver>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
) {
    if probe.sent || probe.frames.is_empty() {
        return;
    }
    let Ok(conn) = conn.single() else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let delay = probe.delay;
    let due = *probe.due.get_or_insert(now + delay);
    if now < due {
        return;
    }
    // With an interval configured, take exactly one frame per due time and
    // re-arm; otherwise take the whole list at once (upstream behaviour).
    let frames: Vec<(u16, Bytes)> = if probe.interval > 0.0 {
        let head = probe.frames.remove(0);
        if probe.frames.is_empty() {
            probe.sent = true;
        } else {
            let interval = probe.interval;
            probe.due = Some(now + interval);
        }
        vec![head]
    } else {
        probe.sent = true;
        std::mem::take(&mut probe.frames)
    };
    for (opcode, mut data) in frames {
        // Substitute the local-uid placeholder, if the body carries it.
        if let Some(at) = data
            .windows(4)
            .position(|w| w == PROBE_LOCAL_UID_PLACEHOLDER)
        {
            match driver.local_uid {
                Some(uid) => {
                    let mut bytes = data.to_vec();
                    bytes[at..at + 4].copy_from_slice(&uid.to_le_bytes());
                    data = Bytes::from(bytes);
                    info!("netcheck: PROBE local-uid placeholder -> {}", uid);
                }
                None => {
                    error!(
                        "netcheck: PROBE body wants the local uid but none was read \
                         (NETCHECK_ACTIONS=1 arms that) — NOT sending"
                    );
                    continue;
                }
            }
        }
        info!(
            "netcheck: PROBE sending opcode {:#06x} body [{}] ({} bytes)",
            opcode,
            hexdump(&data, 64),
            data.len()
        );
        let frame = SilkroadFrame::Packet {
            count: 0,
            crc: 0,
            opcode,
            encrypted: 0,
            data,
        };
        if let Err(e) = conn.get_sender().send(frame) {
            error!("netcheck: PROBE send failed: {}", e.0);
        }
    }
}
