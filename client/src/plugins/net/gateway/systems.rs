use bevy::log::{error, info, warn};
use bevy::prelude::{Commands, Entity, Name, Query, Res, ResMut, With};

use packets::gateway::ShardListPingRequest;
use packets::Packet;

use crate::net::connection::{PendingConnection, SilkroadConnection};
use crate::plugins::config::division::DivisionInfo;
use crate::plugins::config::ClientConfig;
use crate::plugins::net::gateway::{GatewayConnection, GatewayConnectionStatus};
use crate::plugins::net::plugin::NetworkState;

/// Where the gateway address came from — the log line's second half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GatewaySource {
    /// `NETCHECK_GATEWAY` was set in a netcheck run.
    Env,
    /// `config.yaml` (or, through it, `Media.pk2`) decided.
    Config,
}

/// Pick the gateway to dial: in a **netcheck run only**, `NETCHECK_GATEWAY`
/// overrides the configured address; otherwise the config decides.
///
/// Idea: the headless gate has to be pointable at our own test peer
/// (`tools/src/bin/sro_peer.rs`) without editing the user's `config.yaml` — that
/// file is theirs and gitignored, and a CI gate that mutates it is a gate that
/// changes the thing it measures. Same shape as
/// `NETCHECK_ACCOUNT`/`NETCHECK_PASSWORD` in [`crate::netcheck`]: a blank value
/// counts as unset, and the two environment values are passed in rather than
/// read here so the choice is testable without touching the process environment.
///
/// The `NETCHECK` guard is deliberate: a stray `NETCHECK_GATEWAY` in a shell
/// must not silently redirect a normal GUI start.
pub(crate) fn select_gateway(
    env_netcheck: Option<String>,
    env_gateway: Option<String>,
    configured: Option<String>,
) -> Option<(String, GatewaySource)> {
    let non_blank = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if env_netcheck.is_some() {
        if let Some(address) = non_blank(env_gateway) {
            return Some((address, GatewaySource::Env));
        }
    }
    configured.map(|address| (address, GatewaySource::Config))
}

/// Kicks off the gateway connect on a worker thread. Does nothing if a gateway
/// connection already exists or is still being established (the
/// [`GatewayConnection`] marker covers both), so it can double as a "reconnect
/// if needed" system (e.g. when re-entering the login form after a successful
/// login despawned the connection).
///
/// The blocking connect + handshake runs off-thread via
/// [`SilkroadConnection::connect_async`]; [`poll_gateway_connection`] delivers
/// the finished connection to the ECS, so the app never freezes here.
pub(crate) fn init_gateway_service(
    existing: Query<(), With<GatewayConnection>>,
    config: Option<Res<ClientConfig>>,
    division: Res<DivisionInfo>,
    mut status: ResMut<GatewayConnectionStatus>,
    mut commands: Commands,
) {
    if !existing.is_empty() {
        return;
    }

    let Some(config) = config else {
        warn!("client config does not exist");
        return;
    };

    let Some((address, source)) = select_gateway(
        std::env::var("NETCHECK").ok(),
        std::env::var("NETCHECK_GATEWAY").ok(),
        config.network_settings.resolve_gateway(&division),
    ) else {
        warn!(
            "no gateway address: config.yaml names none and Media.pk2's \
             divisioninfo.txt/gateport.txt could not supply one"
        );
        return;
    };
    if source == GatewaySource::Env {
        // Say which address the run really used, in the same spirit as
        // netcheck's account line (`crate::netcheck`, AGENTS.md): a gate whose
        // input is invisible in the log is how an environment condition gets
        // read as a code defect.
        info!("netcheck: gateway {} (from NETCHECK_GATEWAY)", address);
    } else if std::env::var("NETCHECK").is_ok() {
        info!("netcheck: gateway {} (from config.yaml)", address);
    }

    let pending = SilkroadConnection::connect_async(address.as_str());
    commands.spawn((pending, GatewayConnection, Name::from("GatewayService")));
    *status = GatewayConnectionStatus::Connecting;
    info!("connecting to gateway service ...");
}

/// Delivers an in-flight gateway connect to the ECS. On success the established
/// [`SilkroadConnection`] replaces the [`PendingConnection`] on the same entity
/// and the initial shard-list ping is sent right away — driving it off
/// connection-establishment (rather than scene-enter) is what keeps it correct
/// now that the connect is asynchronous. On failure the entity is despawned and
/// the error is surfaced via [`GatewayConnectionStatus`].
pub(crate) fn poll_gateway_connection(
    query: Query<(Entity, &PendingConnection), With<GatewayConnection>>,
    mut network_state: ResMut<NetworkState>,
    mut status: ResMut<GatewayConnectionStatus>,
    mut commands: Commands,
) {
    for (entity, pending) in query.iter() {
        let Some(result) = pending.poll() else {
            continue;
        };
        match result {
            Ok(conn) => {
                // The connection is fully established (handshake done, socket
                // switched to non-blocking) before it ever reaches the ECS, so
                // the first `receive_packets` tick that sees it cannot race the
                // handshake.
                let sender = conn.get_sender();
                commands
                    .entity(entity)
                    .remove::<PendingConnection>()
                    .insert(conn);
                network_state.gateway = true;
                *status = GatewayConnectionStatus::Connected;
                info!("initialized gateway service");

                let frame = Packet::from(ShardListPingRequest).into();
                if let Err(e) = sender.send(frame) {
                    error!("failed to request shard list: {}", e.0);
                }
            }
            Err(e) => {
                error!("failed to init gateway service: {}", e);
                *status =
                    GatewayConnectionStatus::Failed(format!("Failed to connect to server: {e}"));
                commands.entity(entity).despawn();
            }
        }
    }
}

#[cfg(test)]
mod gateway_selection_tests {
    use super::*;

    #[test]
    fn netcheck_gateway_overrides_the_config() {
        assert_eq!(
            select_gateway(
                Some("1".into()),
                Some("127.0.0.1:15779".into()),
                // A placeholder, not the address this was developed against:
                // `select_gateway` resolves nothing, so the configured side is
                // pure test data — and `scripts/check_no_private.py` refuses an
                // address literal in a patch whatever it points at.
                Some("gateway.example.com:15779".into())
            ),
            Some(("127.0.0.1:15779".to_string(), GatewaySource::Env))
        );
    }

    /// The guard: without `NETCHECK` the variable must not touch a GUI start,
    /// and a blank value counts as unset (`NETCHECK_GATEWAY= make netcheck`).
    #[test]
    fn the_override_only_applies_to_a_netcheck_run() {
        assert_eq!(
            select_gateway(None, Some("127.0.0.1:15779".into()), Some("cfg:1".into())),
            Some(("cfg:1".to_string(), GatewaySource::Config))
        );
        assert_eq!(
            select_gateway(Some("1".into()), Some("   ".into()), Some("cfg:1".into())),
            Some(("cfg:1".to_string(), GatewaySource::Config))
        );
        assert_eq!(select_gateway(Some("1".into()), None, None), None);
    }
}
