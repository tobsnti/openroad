use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui::{InteractionDisabled, Pressed};
use bevy::ui_widgets::Activate;
use mac_address::get_mac_address;

use packets::agent::{describe_agent_auth_error, AgentLoginRequest, AgentLoginResponse};
use packets::login::{
    describe_login_error, login_error_text, LoginFailure, LoginRequest, LoginResponse,
};
use packets::Packet;

use crate::net::connection::SilkroadConnection;
use crate::plugins::config::division::DivisionInfo;
use crate::plugins::net::agent::AgentConnectionBundle;
use crate::plugins::net::gateway::{GatewayConnection, GatewayConnectionStatus};
use crate::plugins::net::plugin::NetworkState;
use crate::plugins::settings::options::GameOptions;
use crate::plugins::textdata::ClientUiStrings;

use super::assets::IntroV2Assets;
use super::chrome::InfoTextV2Update;
use super::fade::{FadeToBlack, FadeToBlackTimer};
use super::login_form::{login_form_is_on_screen, ConnectButton, ExitButton, IdInput, PwInput};
use super::server_select::SelectedShardV2;
use super::IntroV2State;

/// The credentials used to log in, set by [`on_connect_activate`] from the form
/// inputs (or by dev_fast_login from config). [`on_gateway_login_response`] reads
/// them for the follow-up agent login instead of re-reading the widgets, which
/// lets dev_fast_login drive the flow without touching the `EditableText` inputs.
#[derive(Resource, Clone)]
pub struct LoginCredentials {
    pub username: String,
    pub password: String,
}

/// Clears the disabled state left on the Connect and Exit buttons by
/// [`on_connect_activate`] when the login form is (re-)entered, e.g. after
/// cancelling out of the character selection. Also strips a stale `Pressed`:
/// the button's release observer skips its removal on disabled buttons, which
/// would leave the press art stuck.
pub fn reenable_connect_button(
    query: Query<
        Entity,
        (
            Or<(With<ConnectButton>, With<ExitButton>)>,
            Or<(With<InteractionDisabled>, With<Pressed>)>,
        ),
    >,
    mut commands: Commands,
) {
    // Entering the form is a fresh start: no request of the previous visit is
    // still being waited for.
    commands.remove_resource::<PendingLogin>();
    for entity in query.iter() {
        commands
            .entity(entity)
            .remove::<(InteractionDisabled, Pressed)>();
    }
}

/// A gateway login request that has gone out and is still waiting for its
/// answer. Its only job is that the wait cannot be infinite.
///
/// [`on_connect_activate`] disables Connect **and** Exit while the request is in
/// flight, and every arm that *answers* gives them back. A server that answers
/// nothing has no arm, so the screen stayed disabled for the rest of the
/// session: the status line on "...Requesting user confirmation...", and no
/// further click producing a request. That state is what
/// [`time_out_pending_login`] ends.
#[derive(Resource)]
pub struct PendingLogin {
    timer: Timer,
}

/// How long the screen waits for `0xA102` before it hands itself back.
///
/// **Ours, not the original's**: what the original does with a gateway that
/// never answers is unknown, and no data row names a timeout. Ten seconds is
/// chosen to be far longer than an answering gateway needs (a local login
/// answers inside a second) and short enough that a player does not conclude
/// the client is dead.
const LOGIN_ANSWER_BUDGET_SECS: f32 = 10.0;

impl Default for PendingLogin {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(LOGIN_ANSWER_BUDGET_SECS, TimerMode::Once),
        }
    }
}

/// Gives the login screen back when the gateway never answers, and says so.
///
/// The budget does not run while the *user* is the one being waited for: with
/// the captcha modal up, the request **was** answered and the two greyed buttons
/// are what the original shows, so the timer is held at zero instead of ticking
/// behind the modal.
pub fn time_out_pending_login(
    time: Res<Time>,
    mut pending: ResMut<PendingLogin>,
    captcha_modal: Query<(), With<super::captcha::CaptchaModal>>,
    buttons: Query<Entity, Or<(With<ConnectButton>, With<ExitButton>)>>,
    ui_strings: Res<ClientUiStrings>,
    mut info_text_writer: MessageWriter<InfoTextV2Update>,
    mut commands: Commands,
) {
    if !captcha_modal.is_empty() {
        pending.timer.reset();
        return;
    }
    if !pending.timer.tick(time.delta()).just_finished() {
        return;
    }
    warn!("[Login] no answer within {LOGIN_ANSWER_BUDGET_SECS} s — giving the screen back");
    info_text_writer.write(InfoTextV2Update(
        ui_strings.get_plain_or(AGENT_CONNECT_ERROR_KEY, "Failed to connect to server."),
    ));
    for entity in buttons.iter() {
        commands
            .entity(entity)
            .remove::<(InteractionDisabled, Pressed)>();
    }
    commands.remove_resource::<PendingLogin>();
}

/// The textuisystem rows this flow speaks, all present in
/// `Media/server_dep/silkroad/textdata/textuisystem.txt`:
///
/// ```text
/// UIO_MSG_ERROR_CITATION                  "…Requesting user confirmation…"
/// UIIT_STT_GLOBAL_PASSWORD_INPUT_ERROR    "Password entry has failed %d out of %d times."
/// UIO_MSG_ERROR_OVERLAP            (:176) "This user is already connected. …"
/// ```
///
/// The fallbacks below are the literals that stood here before, and they match
/// the data verbatim — so a missing table changes nothing visible.
/// The two `%d` in the password row are positional in the data, and we fill them
/// **(attempts, max)** — "failed 1 out of 6 times" — by the shared
/// [`super::fill_placeholders`], which replaced a two-argument twin of the same
/// loop that used to live in this file.
///
/// **That is a deliberate deviation, not a reproduction**, and this comment used
/// to claim the opposite ("like the original's printf-style substitution").
/// `0xa102`'s wrong-password payload is `max_attempts` then `cur_attempts` on the
/// wire (`packets/src/login.rs`), a failure with tolerance 6 and counter 1 reads
/// `02 01 06000000 01000000`, and the original client puts the *first* wire field
/// in the *first* `%d`: its screen says **"Password entry has failed 6 out of 1
/// times."** So the original tells the player they have failed six times on their
/// first mistake, and hides how many tries are left. We print the counter first
/// because that is the number the sentence is about.
///
/// `UIO_MSG_ERROR_OVERLAP` used to be a third constant here. It is now one row
/// of [`login_error_text`] (code `3`) — the whole point of that table is that
/// the code -> key mapping exists once, next to the wire type it belongs to.
const CONNECT_PROGRESS_KEY: &str = "UIO_MSG_ERROR_CITATION";
const PASSWORD_ATTEMPTS_KEY: &str = "UIIT_STT_GLOBAL_PASSWORD_INPUT_ERROR";
/// The two rows the empty-form refusal speaks, both shipped:
/// `UIO_MSG_ERROR_INPUT` (`textuisystem.txt:164`, English column "Invalid ID")
/// and `UIO_MSG_ERROR_PASSWORD` (`:165`, "Invalid ID or password.").
///
/// Which row the original picks for an *empty* field is unknown — it sends the
/// empty login rather than refusing it — so the choice of these two rows for
/// this refusal is ours: the ID row when the ID is missing, the password row
/// when only the password is, because that is what each sentence says.
const EMPTY_ID_KEY: &str = "UIO_MSG_ERROR_INPUT";
const EMPTY_PASSWORD_KEY: &str = "UIO_MSG_ERROR_PASSWORD";
/// `UIO_MSG_ERROR_SEVER_CONNECT` (`textuisystem.txt:166`, English column
/// "Failed to connect to server." — the key's misspelling is the original's).
/// The row the agent-connect failure path speaks; see that arm for why it goes
/// to the status line and not to a message box.
const AGENT_CONNECT_ERROR_KEY: &str = "UIO_MSG_ERROR_SEVER_CONNECT";

/// `Activate` observer of the Connect button: sends the gateway login
/// request. Port of the old `on_connect_button_clicked_system`.
pub fn on_connect_activate(
    activate: On<Activate>,
    state: Option<Res<State<IntroV2State>>>,
    id_query: Query<&EditableText, With<IdInput>>,
    pw_query: Query<&EditableText, With<PwInput>>,
    mut info_text_writer: MessageWriter<InfoTextV2Update>,
    selected_shard: Res<SelectedShardV2>,
    gateway_query: Query<&SilkroadConnection, With<GatewayConnection>>,
    exit_button: Query<Entity, With<ExitButton>>,
    division: Res<DivisionInfo>,
    ui_strings: Res<ClientUiStrings>,
    mut commands: Commands,
) {
    // The login form is never despawned, only hidden, and Bevy activates a
    // *focused* button on Enter/Space with no visibility filter — so this
    // observer can be reached from a screen that has nothing to do with logging
    // in. `login_form::close_tab_ring` takes the Tab route away; this closes the
    // door on the entity itself. See `login_form::login_form_is_on_screen`.
    if !login_form_is_on_screen(state.as_deref()) {
        return;
    }
    let Ok(id_input) = id_query.single() else {
        return;
    };
    let Ok(pw_input) = pw_query.single() else {
        return;
    };

    let username = id_input.value().to_string().trim().to_string();
    let password = pw_input.value().to_string().trim().to_string();

    // An empty field is refused here, in front of the wire. Without this the
    // click sent `0x6102` with an empty name and an empty password, the server
    // answered nothing at all, and the screen stayed on
    // "...Requesting user confirmation..." with Connect disabled — one early
    // click and the client was only killable. The local gates in front of the
    // send are the idiom the create screen already uses (`gate_selection`).
    if username.is_empty() || password.is_empty() {
        let (key, fallback) = if username.is_empty() {
            (EMPTY_ID_KEY, "Invalid ID")
        } else {
            (EMPTY_PASSWORD_KEY, "Invalid ID or password.")
        };
        info_text_writer.write(InfoTextV2Update(ui_strings.get_plain_or(key, fallback)));
        return;
    }

    // No shard committed = nothing to log into, because `LoginRequest` carries
    // the shard id. This used to `return` in silence, and with `autologin`
    // off that was the whole "Connect does nothing and says nothing" defect:
    // the Server row shows the remembered `RECENTSERVER` *name*
    // (server_select::update_shard_name_text) while `SelectedShardV2` is still
    // `None`, so the screen looks ready and neither the mouse nor Enter
    // (login_form's Enter triggers this very `Activate`) produces anything.
    //
    // The line is prose, not a textuisystem key: `textdata/textuisystem.txt` has
    // no row for it — `UIIT_STT_SERVER_SELECT` :5337 is one of the unused `0`
    // rows, and error ids 162-200 are all about a server that *answered* badly.
    // Same precedent as `describe_agent_auth_error` below. Stated deviation: the
    // original leaves this click silent, we answer it, because a dead button with
    // no feedback is a defect.
    let Some(shard_id) = selected_shard.0 else {
        info_text_writer.write(InfoTextV2Update(
            "Select a server first (the Server row's list button).".to_string(),
        ));
        return;
    };

    // Remembered for the follow-up agent login (see `on_gateway_login_response`).
    commands.insert_resource(LoginCredentials {
        username: username.clone(),
        password: password.clone(),
    });

    // No gateway connection = the click cannot go anywhere, and this used to
    // `return` in silence — one more of the class where an error path is
    // entered and then dropped.
    //
    // It is reachable while standing still on this screen: a refusal such as
    // `already connected` leaves the player here with no state change, and
    // `init_gateway_service` only runs on `OnEnter` (of `SceneState::Loading`
    // and of this screen), so nobody rebuilds a connection that is gone. The
    // player then clicks Connect and *nothing happens at all*.
    //
    // So say it, and start a new connect on the way out. `init_gateway_service`
    // is built for exactly this ("can double as a reconnect if needed",
    // `net/gateway/systems.rs`) and returns early when a connection or a
    // pending one already exists, so running it here cannot double-connect.
    // Deliberate deviation: the original leaves this click silent, the same
    // call the line above already makes for a missing server.
    let Ok(connection) = gateway_query.single() else {
        info_text_writer.write(InfoTextV2Update(
            "No connection to the server — reconnecting. Try again in a moment.".to_string(),
        ));
        commands.run_system_cached(crate::plugins::net::gateway::systems::init_gateway_service);
        return;
    };

    info!("[Login] user = {}, shard_id = {}", username, shard_id);
    // Also clear Pressed: the button's release observer won't remove it once
    // the button is disabled, leaving the press art stuck.
    commands
        .entity(activate.entity)
        .insert(InteractionDisabled)
        .remove::<Pressed>();
    // Exit greys out too while the request is in flight, as it does in the
    // original: in the captcha state that follows the Connect click, **Connect
    // and Exit are both greyed** with the status line showing "Requesting user
    // confirmation".
    // `reenable_connect_button` clears both again on (re-)entering the form.
    for exit in exit_button.iter() {
        commands.entity(exit).insert(InteractionDisabled);
    }
    // No sound here. `error.wav` used to play on *every* Connect click, which is
    // a suspected port bug: whether the original ever plays error.wav on Connect
    // is unknown. The click keeps its `uibutton_a` from the button's
    // `ButtonSound` (login_form.rs, ConnectButton); error.wav now hangs on the
    // real failure event in `on_gateway_login_response` instead.
    info_text_writer.write(InfoTextV2Update(
        ui_strings.get_plain_or(CONNECT_PROGRESS_KEY, "...Requesting user confirmation..."),
    ));

    let frame = Packet::from(LoginRequest {
        content_id: division.content_id,
        username,
        password,
        shard_id,
    })
    .into();

    if let Err(e) = connection.get_sender().send(frame) {
        error!("failed to send frame: {}", e.0);
    }
    // From here the screen is disabled until somebody answers — so start the
    // clock that guarantees somebody does ([`time_out_pending_login`]).
    commands.insert_resource(PendingLogin::default());
}

/// Surfaces an asynchronous gateway connect failure as intro info text. Because
/// the connect runs off-thread, the failure can arrive after the login form is
/// already visible; this runs both on login-form entry (to show a failure that
/// was already pending) and whenever [`GatewayConnectionStatus`] changes.
pub fn surface_gateway_error(
    status: Res<GatewayConnectionStatus>,
    mut info_text_writer: MessageWriter<InfoTextV2Update>,
) {
    if let GatewayConnectionStatus::Failed(msg) = &*status {
        info_text_writer.write(InfoTextV2Update(msg.clone()));
    }
}

/// Handles the gateway's login response: surfaces errors on the info text or
/// opens the agent connection. Port of the old `on_gateway_login_response`.
pub fn on_gateway_login_response(
    mut event_reader: MessageReader<LoginResponse>,
    mut info_text_writer: MessageWriter<InfoTextV2Update>,
    mut commands: Commands,
    mut network_state: ResMut<NetworkState>,
    credentials: Option<Res<LoginCredentials>>,
    gateway_query: Query<Entity, With<GatewayConnection>>,
    connect_button_query: Query<Entity, With<ConnectButton>>,
    exit_button_query: Query<Entity, With<ExitButton>>,
    division: Res<DivisionInfo>,
    ui_strings: Res<ClientUiStrings>,
    assets: Res<IntroV2Assets>,
    options: Res<GameOptions>,
) {
    let Some(credentials) = credentials else {
        return;
    };

    let Some(res) = event_reader.read().next() else {
        return;
    };

    let Ok(gateway_entity) = gateway_query.single() else {
        return;
    };

    let Ok(connect_button) = connect_button_query.single() else {
        return;
    };

    // The gateway answered, so the wait is over either way.
    commands.remove_resource::<PendingLogin>();

    let username = credentials.username.clone();
    let password = credentials.password.clone();

    if let Some(err) = &res.login_error {
        commands
            .entity(connect_button)
            .remove::<InteractionDisabled>();
        // Same pair as on the click: the request is over, so Exit is usable again.
        for exit in exit_button_query.iter() {
            commands.entity(exit).remove::<InteractionDisabled>();
        }
        error!(
            "[Login Error]: {} ({:?})",
            describe_login_error(err.error_code),
            err
        );
        // This — a refused login — is the event `error.wav` belongs to. It used
        // to fire on the Connect *click* instead, which is a suspected port
        // bug. The original's exact mapping is unknown, so playing it on the
        // failure is a stated openroad choice, not a claim about the original.
        super::play_error_sound(&mut commands, &assets, &options);
        if let Some(blocked_err) = &err.account_blocked_err {
            if let Some(ban_info) = &blocked_err.ban_info {
                info_text_writer.write(InfoTextV2Update(ban_info.reason.clone()));
            }
        }

        if let Some(wrong_attempt) = &err.wrong_attempt {
            let template = ui_strings.get_plain_or(
                PASSWORD_ATTEMPTS_KEY,
                "Password entry has failed %d out of %d times.",
            );
            // Counter first, tolerance second: "failed 1 out of 6 times". The
            // original prints the wire order into the row and thereby says
            // "6 out of 1" — stated deviation, reasoned at
            // `PASSWORD_ATTEMPTS_KEY`.
            info_text_writer.write(InfoTextV2Update(super::fill_placeholders(
                &template,
                &[wrong_attempt.cur_attempts, wrong_attempt.max_attempts],
            )));
        }

        // Codes with a payload rendered their own message above (attempt
        // counter / ban reason); everything else now goes through the *shipped*
        // row for its code instead of our own prose. `login_error_text` is the
        // pump's switch transcribed (the original's login switch) and returns
        // `None` for exactly
        // the two payload codes, so the match arms and the table cannot drift
        // apart. `describe_login_error` stays the prose for the log line.
        match err.failure() {
            LoginFailure::WrongPassword(_) | LoginFailure::Blocked(_) => {}
            _ => {
                if let Some((key, fallback)) = login_error_text(err.error_code) {
                    info_text_writer
                        .write(InfoTextV2Update(ui_strings.get_plain_or(key, fallback)));
                }
            }
        }
    } else {
        commands.entity(gateway_entity).despawn();
        network_state.gateway = false;
        let info = res.login_info.clone().unwrap();

        match SilkroadConnection::new(&format!("{}:{}", info.agent_ip, info.agent_port)) {
            Ok(conn) => {
                network_state.agent = true;
                let mac = get_mac_address().unwrap().unwrap();
                let sender = conn.get_sender();

                let frame = Packet::from(AgentLoginRequest {
                    token: info.agent_token,
                    username,
                    password,
                    content_id: division.content_id,
                    mac_address: mac.bytes(),
                })
                .into();

                if let Err(e) = sender.send(frame) {
                    error!("failed to send frame: {}", e.0);
                }

                commands.spawn(AgentConnectionBundle::new(conn));
            }
            Err(err) => {
                // The defect this repairs: this arm used to end here, in the
                // log.
                // The gateway connection is already despawned above and the
                // buttons are still disabled from `on_connect_activate`, so the
                // screen stayed on "...Requesting user confirmation..."
                // forever — no timeout, no message, and further clicks did
                // nothing at all.
                //
                // Same two halves as a rejected agent *login*
                // (`on_agent_login_response`): say it, and give the screen
                // back. The sentence is the shipped row for exactly this case,
                // `UIO_MSG_ERROR_SEVER_CONNECT` (`textuisystem.txt:166`,
                // "Failed to connect to server."), not our prose.
                //
                // **No message box, on purpose.** The title screen has an
                // authored error channel — `GDR_TEXT_MESSAGE`
                // (`pstitle_europe.txt:634`), the orange status line we already
                // draw — while a one-button message box is not authored
                // anywhere in `ifmessagebox.txt`: its only generic section,
                // `MsgBoxSimple`, is a yes/no box (`UIIT_CTL_YES`/`_NO`), and
                // `ifconfirmbox.txt` is the image-code window, not a message
                // box.
                error!("failed to connect to agent server: {}", err);
                report_agent_connect_failure(
                    &mut commands,
                    &mut info_text_writer,
                    &ui_strings,
                    connect_button,
                    exit_button_query.single().ok(),
                );
            }
        }
    }
}

/// The two halves of the repair: say what happened, and give the screen back.
///
/// A named function rather than three lines in the match arm because that arm
/// sits inside a closure-free `else` branch of a long system and because it is
/// the only part of the agent-connect failure that a test can hold: the connect
/// itself is a real socket. `on_agent_login_response` does the same two things
/// for a *rejected* login; this is the same repair for a connection that never
/// came up.
fn report_agent_connect_failure(
    commands: &mut Commands,
    info_text_writer: &mut MessageWriter<InfoTextV2Update>,
    ui_strings: &ClientUiStrings,
    connect_button: Entity,
    exit_button: Option<Entity>,
) {
    info_text_writer.write(InfoTextV2Update(
        ui_strings.get_plain_or(AGENT_CONNECT_ERROR_KEY, "Failed to connect to server."),
    ));
    for entity in std::iter::once(connect_button).chain(exit_button) {
        commands
            .entity(entity)
            .remove::<(InteractionDisabled, Pressed)>();
    }
}

/// On successful agent login, fade to black and enter character selection — and
/// on a *rejected* one, give the screen back to the user.
///
/// Idea (and the defect it fixes): this handler used to `error!` and return.
/// But the Connect click disables **both** Connect and Exit
/// ([`on_connect_activate`]), and the only thing that re-enables them is
/// [`reenable_connect_button`] on `OnEnter(IntroV2State::LoginForm)` — a state
/// the client never leaves on this path, because the fade to `CharacterList`
/// is exactly what does not happen here. A rejected agent login therefore left
/// a login screen with two dead buttons, no message and no way out but killing
/// the process. So the re-enable, not the message, is the substance of this
/// arm: it is done here, where the rejection arrives, rather than on a state
/// transition that does not occur.
///
/// Deliberate deviation from the original, stated because it is one: the
/// original's login-UI pump has **no error arm for this message**. Internal id
/// `0x1002` is the `0xA103` result and it walks straight into the
/// character-select transition without ever reading the result byte — no
/// `textuisystem` string is selected anywhere in that arm (contrast
/// `0x1003`/`0xA323` right below it, which *does* pick
/// `UIIT_STT_GLOBAL_AUTHENTICATION_INPUT_ERROR` on `result == 2`). Whether the
/// original therefore shows nothing, or whether its gateway dispatcher handles
/// it, is unknown. We choose to say something rather than dead-end the user, and
/// we use [`describe_agent_auth_error`]'s prose rather than inventing a
/// `textuisystem` key for it: no key is known for this path, and a key derived
/// from a name rule is precisely the mistake this repo keeps paying for.
pub fn on_agent_login_response(
    mut reader: MessageReader<AgentLoginResponse>,
    mut info_text_writer: MessageWriter<InfoTextV2Update>,
    mut fade_writer: MessageWriter<FadeToBlack>,
    buttons: Query<Entity, Or<(With<ConnectButton>, With<ExitButton>)>>,
    // Only the error *sound* needs these, and the re-enable below must work
    // without them: taken as `Option` so the arm can be exercised in a bare
    // test app (and degrades to a silent, still-usable screen) instead of
    // failing parameter validation — the same stance `plugins/net/**` takes
    // towards HUD resources (AGENTS.md).
    assets: Option<Res<IntroV2Assets>>,
    options: Option<Res<GameOptions>>,
    mut commands: Commands,
) {
    for res in reader.read() {
        if let Some(code) = res.error_code {
            error!(
                "[Agent]: Login failed because of {} (code {})",
                describe_agent_auth_error(code),
                code
            );
            info_text_writer.write(InfoTextV2Update(format!(
                "Could not enter the game server: {}.",
                describe_agent_auth_error(code)
            )));
            // The screen is usable again: same pair as the click disabled, and
            // `Pressed` goes too — the release observer skips disabled buttons,
            // so it would otherwise leave the pressed art stuck.
            for entity in buttons.iter() {
                commands
                    .entity(entity)
                    .remove::<(InteractionDisabled, Pressed)>();
            }
            if let (Some(assets), Some(options)) = (assets.as_ref(), options.as_ref()) {
                super::play_error_sound(&mut commands, assets, options);
            }
        } else {
            info_text_writer.write(InfoTextV2Update(String::new()));
            commands.insert_resource(FadeToBlackTimer::to(IntroV2State::CharacterList));
            fade_writer.write(FadeToBlack);
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::message::Messages;
    use bevy::prelude::*;

    use crate::scenes::intro_v2::chrome::InfoTextV2Update;
    use crate::scenes::intro_v2::fill_placeholders;

    /// The data row carries two `%d`;
    /// they fill left to right, and the caller passes the attempt counter before
    /// the maximum. Note the wire order is the *other* way round
    /// (`max_attempts` then `cur_attempts`) — the swap is the stated deviation
    /// documented at `PASSWORD_ATTEMPTS_KEY`, not the wire order restated.
    #[test]
    fn fills_both_percent_d_in_order() {
        assert_eq!(
            fill_placeholders("Password entry has failed %d out of %d times.", &[2, 3]),
            "Password entry has failed 2 out of 3 times."
        );
    }

    /// A table without the placeholders (or a localized row that dropped them)
    /// must not panic or duplicate text.
    #[test]
    fn tolerates_a_row_without_placeholders() {
        assert_eq!(
            fill_placeholders("Wrong password.", &[1, 5]),
            "Wrong password."
        );
    }

    /// A *failed agent connection* — not a rejected login — used to end in the
    /// log. The screen
    /// then stood on "...Requesting user confirmation..." with Connect disabled
    /// for good, which is a client the user can only kill. So: the shipped
    /// sentence on the status line, and both buttons back.
    #[test]
    fn a_failed_agent_connection_says_so_and_unlocks_the_screen() {
        use bevy::ecs::system::SystemState;
        use bevy::ui::{InteractionDisabled, Pressed};

        use crate::plugins::textdata::ClientUiStrings;
        use crate::scenes::intro_v2::chrome::InfoTextV2Update;
        use crate::scenes::intro_v2::login_form::{ConnectButton, ExitButton};

        let mut app = App::new();
        app.add_message::<InfoTextV2Update>()
            .init_resource::<ClientUiStrings>();
        let connect = app
            .world_mut()
            .spawn((ConnectButton, InteractionDisabled, Pressed))
            .id();
        let exit = app
            .world_mut()
            .spawn((ExitButton, InteractionDisabled, Pressed))
            .id();

        // `SystemState` rather than `run_system_cached`: the call needs the two
        // spawned entities, and a capturing closure is not a cacheable system.
        let mut state = SystemState::<(
            Commands,
            MessageWriter<InfoTextV2Update>,
            Res<ClientUiStrings>,
        )>::new(app.world_mut());
        {
            let (mut commands, mut info, strings) =
                state.get_mut(app.world_mut()).expect("valid system params");
            super::report_agent_connect_failure(
                &mut commands,
                &mut info,
                &strings,
                connect,
                Some(exit),
            );
        }
        state.apply(app.world_mut());

        for (entity, what) in [(connect, "Connect"), (exit, "Exit")] {
            assert!(
                app.world().get::<InteractionDisabled>(entity).is_none(),
                "{what} is still disabled after a failed agent connection"
            );
            assert!(
                app.world().get::<Pressed>(entity).is_none(),
                "{what} still carries the pressed art"
            );
        }

        let messages = app.world().resource::<Messages<InfoTextV2Update>>();
        let mut cursor = messages.get_cursor();
        let line = cursor
            .read(messages)
            .next()
            .expect("a failed connection must put a line on the status bar")
            .0
            .clone();
        // The fallback is the data row verbatim
        // (`UIO_MSG_ERROR_SEVER_CONNECT`, textuisystem.txt:166), so an empty
        // string table renders the same sentence.
        assert_eq!(line, "Failed to connect to server.");
    }

    /// The substance of the fix, not the message: after a rejected agent login
    /// the login screen must be usable again. `on_connect_activate` disables
    /// Connect *and* Exit, and nothing on this path enters
    /// `IntroV2State::LoginForm` again, so without this arm the two buttons
    /// stay dead forever — a client the user can only kill.
    #[test]
    fn a_rejected_agent_login_gives_the_buttons_back() {
        use bevy::ui::{InteractionDisabled, Pressed};
        use packets::agent::AgentLoginResponse;

        use crate::scenes::intro_v2::chrome::InfoTextV2Update;
        use crate::scenes::intro_v2::fade::FadeToBlack;
        use crate::scenes::intro_v2::login_form::{ConnectButton, ExitButton};

        let mut app = App::new();
        app.add_message::<AgentLoginResponse>()
            .add_message::<InfoTextV2Update>()
            .add_message::<FadeToBlack>()
            .add_systems(Update, super::on_agent_login_response);

        let connect = app
            .world_mut()
            .spawn((ConnectButton, InteractionDisabled, Pressed))
            .id();
        let exit = app
            .world_mut()
            .spawn((ExitButton, InteractionDisabled, Pressed))
            .id();

        // code 4 = server full (`describe_agent_auth_error`)
        app.world_mut().write_message(AgentLoginResponse {
            result: 2,
            error_code: Some(4),
        });
        app.update();

        for (entity, what) in [(connect, "Connect"), (exit, "Exit")] {
            assert!(
                app.world().get::<InteractionDisabled>(entity).is_none(),
                "{what} is still disabled after a rejected agent login"
            );
            assert!(
                app.world().get::<Pressed>(entity).is_none(),
                "{what} still carries the pressed art"
            );
        }

        // ...and the user is told why. The prose is `describe_agent_auth_error`
        // deliberately (no textuisystem key is known for this path).
        let messages = app.world().resource::<Messages<InfoTextV2Update>>();
        let mut cursor = messages.get_cursor();
        let line = cursor
            .read(messages)
            .next()
            .expect("a rejection must put a line on the status bar")
            .0
            .clone();
        assert!(line.contains("server full"), "status line was {line:?}");
        // no fade: the client stays on the login screen
        assert!(app.world().resource::<Messages<FadeToBlack>>().is_empty());
    }

    /// The defect this pins: Connect with no committed shard
    /// used to `return` in silence, so with `autologin: false` the button (and
    /// Enter, which triggers the same `Activate`) did nothing and said nothing
    /// while the Server row displayed the remembered name. Now it answers.
    ///
    /// The negative control is the second half of the same test: with a shard
    /// committed, this early return must NOT fire — the message on the bus is
    /// then the connect-progress line, not the hint. (No `app.update()` between
    /// trigger and read, so the message bus still holds it.)
    #[test]
    fn connect_without_a_selected_server_says_so() {
        use bevy::text::EditableText;
        use bevy::ui_widgets::Activate;

        use crate::plugins::config::division::DivisionInfo;
        use crate::plugins::textdata::ClientUiStrings;
        use crate::scenes::intro_v2::chrome::InfoTextV2Update;
        use crate::scenes::intro_v2::login_form::{ConnectButton, IdInput, PwInput};
        use crate::scenes::intro_v2::server_select::SelectedShardV2;
        use crate::scenes::intro_v2::IntroV2State;

        fn app_with_shard(shard: Option<u16>) -> (App, Entity) {
            let mut app = App::new();
            app.add_message::<InfoTextV2Update>()
                .init_resource::<ClientUiStrings>()
                .insert_resource(DivisionInfo::default())
                // The observer only acts on its own screen (the form is hidden,
                // not despawned, so it stays reachable from elsewhere — see
                // `login_form::login_form_is_on_screen`). Without this resource
                // the guard reads "intro not running" and this test goes silent,
                // which is how it doubles as the guard's red control.
                .insert_resource(State::new(IntroV2State::LoginForm))
                .insert_resource(SelectedShardV2(shard));
            app.world_mut().add_observer(super::on_connect_activate);
            app.world_mut().spawn((IdInput, EditableText::new("user")));
            app.world_mut().spawn((PwInput, EditableText::new("1234")));
            let connect = app.world_mut().spawn(ConnectButton).id();
            (app, connect)
        }

        fn first_line(app: &App) -> Option<String> {
            let messages = app.world().resource::<Messages<InfoTextV2Update>>();
            let mut cursor = messages.get_cursor();
            cursor.read(messages).next().map(|line| line.0.clone())
        }

        let (mut app, connect) = app_with_shard(None);
        app.world_mut().trigger(Activate { entity: connect });
        let line = first_line(&app).expect("a silent Connect is the defect");
        assert!(
            line.to_lowercase().contains("server"),
            "status line was {line:?}"
        );

        // Negative control: with a shard the *server* hint must be gone. The
        // observer then walks on to the send, which needs a live
        // `SilkroadConnection` — a test app has no business owning one, so it
        // stops at that lookup.
        //
        // This used to assert the bus stays **empty**, which encoded the silent
        // return as intended behaviour. It is not: a refusal such as `already
        // connected` leaves the player on this screen with the connection gone
        // and nothing rebuilding it, so that click produced no line at all and
        // the player could not log in with another account. The lookup now
        // speaks and starts a reconnect, so the assertion becomes: the line is
        // the *gateway* one, not the server one, and nothing else was invented.
        let (mut app, connect) = app_with_shard(Some(64));
        app.world_mut().trigger(Activate { entity: connect });
        let line = first_line(&app).expect("a silent Connect is the defect, here too");
        assert!(
            line.to_lowercase().contains("connection") && line.to_lowercase().contains("reconnect"),
            "expected the gateway line, got {line:?}"
        );
        assert!(
            !line.to_lowercase().contains("select a server"),
            "the server hint must not fire once a shard is committed: {line:?}"
        );
    }

    /// The worst of the pre-game defects: Connect on an **empty** form sent
    /// `0x6102` with an empty name and an empty password, the server answered
    /// nothing, and the screen was disabled for the rest of the session. Two
    /// halves, both asserted here: nothing goes out, and nothing locks.
    ///
    /// Negative control is the second half: a filled form must still walk past
    /// this gate (it then stops at the gateway lookup, which a test app has no
    /// business owning).
    #[test]
    fn an_empty_form_is_refused_and_does_not_lock_the_screen() {
        use bevy::text::EditableText;
        use bevy::ui::InteractionDisabled;
        use bevy::ui_widgets::Activate;

        use crate::plugins::config::division::DivisionInfo;
        use crate::plugins::textdata::ClientUiStrings;
        use crate::scenes::intro_v2::chrome::InfoTextV2Update;
        use crate::scenes::intro_v2::login_form::{ConnectButton, IdInput, PwInput};
        use crate::scenes::intro_v2::server_select::SelectedShardV2;
        use crate::scenes::intro_v2::IntroV2State;

        fn app_with_form(id: &str, pw: &str) -> (App, Entity) {
            let mut app = App::new();
            app.add_message::<InfoTextV2Update>()
                .init_resource::<ClientUiStrings>()
                .insert_resource(DivisionInfo::default())
                .insert_resource(State::new(IntroV2State::LoginForm))
                // A shard *is* committed: the empty form was refused by nothing
                // else, which is how the packet went out.
                .insert_resource(SelectedShardV2(Some(1)));
            app.world_mut().add_observer(super::on_connect_activate);
            app.world_mut().spawn((IdInput, EditableText::new(id)));
            app.world_mut().spawn((PwInput, EditableText::new(pw)));
            let connect = app.world_mut().spawn(ConnectButton).id();
            (app, connect)
        }

        fn first_line(app: &App) -> Option<String> {
            let messages = app.world().resource::<Messages<InfoTextV2Update>>();
            let mut cursor = messages.get_cursor();
            cursor.read(messages).next().map(|line| line.0.clone())
        }

        // Both fields empty, and blanks count as empty (the value is trimmed).
        let (mut app, connect) = app_with_form("", "   ");
        app.world_mut().trigger(Activate { entity: connect });
        // The shipped row for a missing ID, verbatim (`textuisystem.txt:164`).
        assert_eq!(first_line(&app).as_deref(), Some("Invalid ID"));
        assert!(
            app.world().get::<InteractionDisabled>(connect).is_none(),
            "a refused click must not grey the button it refused"
        );
        assert!(
            app.world().get_resource::<super::PendingLogin>().is_none(),
            "nothing was sent, so nothing is being waited for"
        );

        // Only the password missing: the other row, and still no lock.
        let (mut app, connect) = app_with_form("player", "");
        app.world_mut().trigger(Activate { entity: connect });
        assert_eq!(first_line(&app).as_deref(), Some("Invalid ID or password."));
        assert!(app.world().get::<InteractionDisabled>(connect).is_none());

        // Negative control: a filled form passes this gate and reaches the
        // gateway lookup, whose line is the *connection* one.
        let (mut app, connect) = app_with_form("player", "secret");
        app.world_mut().trigger(Activate { entity: connect });
        let line = first_line(&app).expect("the filled form must not be refused here");
        assert!(
            line.to_lowercase().contains("reconnecting"),
            "expected the gateway line, got {line:?}"
        );
    }

    /// The locked screen itself, reproduced: Connect and Exit greyed, the
    /// progress line standing, and no answer ever arriving. Before
    /// `time_out_pending_login` that state was terminal — the only way out was
    /// killing the client.
    #[test]
    fn a_login_nobody_answers_gives_the_screen_back() {
        use std::time::Duration;

        use bevy::ui::{InteractionDisabled, Pressed};

        use crate::plugins::textdata::ClientUiStrings;
        use crate::scenes::intro_v2::chrome::InfoTextV2Update;
        use crate::scenes::intro_v2::login_form::{ConnectButton, ExitButton};

        let mut app = App::new();
        app.add_message::<InfoTextV2Update>()
            .init_resource::<ClientUiStrings>()
            .init_resource::<Time>()
            .init_resource::<super::PendingLogin>()
            .add_systems(Update, super::time_out_pending_login);

        let connect = app
            .world_mut()
            .spawn((ConnectButton, InteractionDisabled, Pressed))
            .id();
        let exit = app
            .world_mut()
            .spawn((ExitButton, InteractionDisabled, Pressed))
            .id();

        // Half the budget: the screen is still waiting, because a gateway that
        // is merely slow must not be declared dead.
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(
                super::LOGIN_ANSWER_BUDGET_SECS / 2.0,
            ));
        app.update();
        assert!(
            app.world().get::<InteractionDisabled>(connect).is_some(),
            "the wait ended too early"
        );

        // Past the budget: both buttons back, the shipped sentence on the line,
        // and nothing left waiting.
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(super::LOGIN_ANSWER_BUDGET_SECS));
        app.update();
        for (entity, what) in [(connect, "Connect"), (exit, "Exit")] {
            assert!(
                app.world().get::<InteractionDisabled>(entity).is_none(),
                "{what} is still disabled after a login nobody answered"
            );
            assert!(
                app.world().get::<Pressed>(entity).is_none(),
                "{what} still carries the pressed art"
            );
        }
        let messages = app.world().resource::<Messages<InfoTextV2Update>>();
        let mut cursor = messages.get_cursor();
        let line = cursor
            .read(messages)
            .next()
            .expect("the timeout must put a line on the status bar")
            .0
            .clone();
        assert_eq!(line, "Failed to connect to server.");
        assert!(app.world().get_resource::<super::PendingLogin>().is_none());
    }

    /// The captcha half of the same budget: while the image-code window is up
    /// the request *was* answered and the user is the one being waited for, so
    /// the budget must not run out behind the modal.
    #[test]
    fn the_captcha_modal_holds_the_wait_open() {
        use std::time::Duration;

        use bevy::ui::InteractionDisabled;

        use crate::plugins::textdata::ClientUiStrings;
        use crate::scenes::intro_v2::captcha::CaptchaModal;
        use crate::scenes::intro_v2::chrome::InfoTextV2Update;
        use crate::scenes::intro_v2::login_form::ConnectButton;

        let mut app = App::new();
        app.add_message::<InfoTextV2Update>()
            .init_resource::<ClientUiStrings>()
            .init_resource::<Time>()
            .init_resource::<super::PendingLogin>()
            .add_systems(Update, super::time_out_pending_login);

        let connect = app
            .world_mut()
            .spawn((ConnectButton, InteractionDisabled))
            .id();
        app.world_mut().spawn(CaptchaModal);

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(
                super::LOGIN_ANSWER_BUDGET_SECS * 3.0,
            ));
        app.update();

        assert!(
            app.world().get::<InteractionDisabled>(connect).is_some(),
            "the modal's greyed Connect is what the original shows"
        );
        assert!(
            app.world().get_resource::<super::PendingLogin>().is_some(),
            "the wait belongs to the user now, it is not over"
        );
    }

    /// The success path must be untouched by the arm above: still no status
    /// line, still a fade into the character list.
    #[test]
    fn an_accepted_agent_login_still_fades_into_the_character_list() {
        use packets::agent::AgentLoginResponse;

        use crate::scenes::intro_v2::chrome::InfoTextV2Update;
        use crate::scenes::intro_v2::fade::{FadeToBlack, FadeToBlackTimer};

        let mut app = App::new();
        app.add_message::<AgentLoginResponse>()
            .add_message::<InfoTextV2Update>()
            .add_message::<FadeToBlack>()
            .add_systems(Update, super::on_agent_login_response);

        app.world_mut().write_message(AgentLoginResponse {
            result: 1,
            error_code: None,
        });
        app.update();

        assert!(!app.world().resource::<Messages<FadeToBlack>>().is_empty());
        assert!(app.world().get_resource::<FadeToBlackTimer>().is_some());
    }
}
