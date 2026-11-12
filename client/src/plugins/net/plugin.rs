use bevy::app::{App, Plugin, PostUpdate, PreUpdate};
use bevy::log::{debug, error, info, trace};
use bevy::prelude::{
    warn, Added, Commands, Entity, Local, Message, MessageReader, MessageWriter, Query, Res,
    ResMut, Resource, Time, Timer, TimerMode, Update, With,
};
use bytes::Bytes;
use std::collections::HashSet;

use packets::global::{GlobalStateRequest, GlobalStateUpdate, KeepAlive};
use packets::judge::{judge_decoded, Verdict};
use packets::{NetworkExt, Packet, PacketError};

use crate::net::connection::SilkroadConnection;
use crate::plugins::config::ClientConfig;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::gateway::plugin::GatewayPlugin;
use crate::plugins::net::gateway::GatewayConnection;
use crate::plugins::net::packet_dump::PacketDump;
use sro_net::frame::SilkroadFrame;
use sro_net::stream::{flush_pending, next_packet, read_available, FrameStep, ReadEnd};

#[derive(Resource)]
pub struct KeepAliveTimer {
    timer: Timer,
}

#[derive(Resource, Default)]
pub struct NetworkState {
    pub gateway: bool,
    pub agent: bool,
}

impl Default for KeepAliveTimer {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(5.0, TimerMode::Repeating),
        }
    }
}

/// The rendering-independent networking core: the packet event wiring plus the
/// read/write/keep-alive/disconnect systems. Reused by both the GUI
/// [`NetworkPlugin`] and the headless net-check client (`crate::netcheck`),
/// which cannot pull in the `SceneState`-keyed [`GatewayPlugin`].
pub struct NetworkCorePlugin;
impl Plugin for NetworkCorePlugin {
    fn build(&self, app: &mut App) {
        // Config is optional here: the headless net-check client reuses this
        // plugin without a ClientConfig resource — dump by default there too.
        let dump_enabled = app
            .world()
            .get_resource::<ClientConfig>()
            .map(|c| c.network_settings.packet_dump)
            .unwrap_or(true);
        if dump_enabled {
            app.init_resource::<PacketDump>();
        }
        // Opt-in, and absent by default: sending an encrypted login the
        // server does not expect would break the only working flow we have.
        let encrypt_outbound = app
            .world()
            .get_resource::<ClientConfig>()
            .map(|c| c.network_settings.outbound_encryption)
            .unwrap_or(false);
        if encrypt_outbound {
            app.init_resource::<OutboundEncryption>();
        }
        app.add_network_events()
            .add_plugins(crate::plugins::net::entities::NetworkEntitiesPlugin)
            .add_plugins(crate::plugins::net::friend::FriendPlugin)
            .add_plugins(crate::plugins::net::party::PartyPlugin)
            .add_plugins(crate::plugins::net::guild::GuildPlugin)
            .add_plugins(crate::plugins::net::quest::QuestMarkPlugin)
            .add_plugins(crate::plugins::net::siege::SiegePlugin)
            .add_plugins(crate::plugins::net::silk::SilkPlugin)
            .add_plugins(crate::plugins::net::stall::StallEntitiesPlugin)
            .init_resource::<NetworkState>()
            .init_resource::<crate::plugins::net::gateway::GatewayConnectionStatus>()
            .add_message::<GatewayServiceDisconnected>()
            .add_message::<AgentConnectionDisconnected>()
            .add_systems(
                PreUpdate,
                (record_sessions, receive_packets, fetch_disconnects),
            )
            .add_systems(
                Update,
                (
                    keep_alive,
                    on_global_state_request,
                    on_global_state_update,
                    on_agent_disconnected,
                    on_gateway_disconnected,
                ),
            )
            .add_systems(PostUpdate, send_packets);
    }
}

pub struct NetworkPlugin;
impl Plugin for NetworkPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(NetworkCorePlugin);
        let client_config = app.world().resource::<ClientConfig>();
        if client_config.network_settings.enabled {
            app.add_plugins(GatewayPlugin);
        } else {
            warn!("networking disabled");
        }
    }
}

/// Run condition for anything that starts a gateway connect outside
/// [`GatewayPlugin`]. `poll_gateway_connection` lives *only* in that plugin, so
/// a connect kicked off while networking is disabled is never delivered to the
/// ECS: the status stays `Connecting` forever and the login form waits on a
/// connection that cannot arrive. Callers outside the plugin must gate on this.
pub fn networking_enabled(config: Option<Res<ClientConfig>>) -> bool {
    config.is_some_and(|c| c.network_settings.enabled)
}

#[derive(Message)]
pub struct AgentConnectionDisconnected;
#[derive(Message)]
pub struct GatewayServiceDisconnected;

/// Opcodes the original client receives and deliberately discards, so ours must
/// not report them as unhandled. `0x2110`'s branch in the original's receive loop
/// skips the whole body without reading a byte (`sro_client.exe 00842ff0:90-92`)
/// and it appears in none of its registration tables; there is no layout to wire.
///
/// `0x3C81` (SERVER_ACADEMY_DATA) is the same case one table over: it *is*
/// registered (table B, handler `FUN_008986c0`), but the handler contains no
/// read call at all — the original consumes zero bytes of the message
/// (`docs/re/net/inbound/academy.md:118-127`, verdict do-not-wire `[V]`). A
/// struct for it would model nothing, so it is ignored by name instead of
/// showing up as an unhandled opcode forever (#261).
const KNOWN_IGNORED_OPCODES: &[u16] = &[0x2110, 0x3C81];

/// Decode and emit one received packet, or log why it could not be.
///
/// An opcode without a decoder drops its whole message and nothing else
/// reports it, so a run can look clean while data is lost. The first sighting
/// of each such opcode therefore warns, with the body length; repeats drop to
/// debug so a chatty opcode cannot flood the log. [`KNOWN_IGNORED_OPCODES`]
/// are discarded on purpose and stay silent.
///
/// A decoder that stops short fails nothing either: the packet dispatches and
/// the rest of the frame goes unread, which is how a wrong form stayed
/// invisible while a test pinned it. Reading stays lenient — the client
/// survives where the original would not — but the first short read of each
/// opcode warns with both lengths ([`judge_decoded`]).
fn dispatch_packet(
    opcode: u16,
    data: Bytes,
    reported: &mut HashSet<u16>,
    packets: &mut MessageWriter<Packet>,
) {
    let len = data.len();
    let body = data.clone();
    match Packet::deserialize(opcode, data) {
        Ok(packet) => {
            if !reported.contains(&opcode) {
                if let Verdict::Short(rest) = judge_decoded(opcode, packet.clone(), &body) {
                    reported.insert(opcode);
                    warn!(
                        "network: {opcode:#06X} decoded {} of {len} bytes, {rest} unread — \
                         the decoder's form is shorter than the frame; reported once",
                        len - rest
                    );
                }
            }
            packets.write(packet);
        }
        Err(PacketError::UnknownOpcode(opcode)) if KNOWN_IGNORED_OPCODES.contains(&opcode) => {
            trace!("network: ignoring opcode {opcode:#06X} (known-and-ignored)");
        }
        Err(PacketError::UnknownOpcode(opcode)) => {
            if reported.insert(opcode) {
                warn!("network: unhandled opcode {opcode:#06X} ({len} bytes) — no decoder, message dropped; repeats log at debug");
            } else {
                debug!("network: unhandled opcode {opcode:#06X} ({len} bytes)");
            }
        }
        Err(e) => {
            error!("network: failed to deserialize packet {opcode:#06X} ({len} bytes): {e}");
        }
    }
}

/// One `packet_dump/sessions.log` line per new connection, naming its peer.
fn record_sessions(
    connections: Query<&SilkroadConnection, Added<SilkroadConnection>>,
    mut dump: Option<ResMut<PacketDump>>,
) {
    let Some(dump) = dump.as_mut() else {
        return;
    };
    for conn in &connections {
        dump.session(&conn.peer);
    }
}

fn receive_packets(
    mut query: Query<(Entity, &mut SilkroadConnection)>,
    mut commands: Commands,
    mut packets: MessageWriter<Packet>,
    mut dump: Option<ResMut<PacketDump>>,
    mut reported: Local<HashSet<u16>>,
) {
    for (entity, mut conn) in query.iter_mut() {
        let stream = conn.stream.clone();
        let Ok(mut stream) = stream.lock() else {
            continue;
        };
        // Everything the socket holds is taken first and dispatched before a
        // close or an error is acted on: frames the server sent right before
        // closing still count.
        let read = read_available(&mut *stream, &mut conn.read_buf);
        // Drain every complete frame off the front of the accumulator. The
        // server packs the post-join burst (character-data begin/body/end,
        // celestial, ...) back-to-back and a read can end mid-frame; carrying
        // the unparsed tail across reads is what keeps a split frame — and
        // every frame after it — from being dropped.
        let security = conn.get_security();
        let mut consumed = 0;
        loop {
            match next_packet(&mut conn.read_buf[consumed..], &security) {
                FrameStep::Ready {
                    opcode,
                    data,
                    consumed: n,
                    encrypted,
                } => {
                    if let Some(dump) = dump.as_mut() {
                        dump.dump(opcode, &data, encrypted);
                    }
                    dispatch_packet(opcode, data, &mut reported, &mut packets);
                    consumed += n;
                }
                FrameStep::Skip { consumed: n } => consumed += n,
                FrameStep::Incomplete => break,
                FrameStep::Corrupt => {
                    warn!(
                        "network: desynchronised inbound stream, dropping {} buffered bytes",
                        conn.read_buf.len() - consumed
                    );
                    consumed = conn.read_buf.len();
                    break;
                }
            }
        }
        if consumed > 0 {
            conn.read_buf.drain(..consumed);
        }
        match read {
            Ok(ReadEnd::Drained) => continue,
            Ok(ReadEnd::Closed) => warn!("network: the server closed the connection"),
            Err(err) => error!("stop handling incoming packets because of: {}", err),
        }
        conn.dc_sender
            .send(())
            .expect("failed to send disconnected msg");
        // Should we rather despawn? Currently the connection exists in its own entity
        commands.entity(entity).remove::<SilkroadConnection>();
    }
}

/// Present only when `network_settings.outbound_encryption` is on; its absence
/// is what keeps the plaintext-login behaviour of #243 as the default.
#[derive(Resource, Default)]
pub struct OutboundEncryption;

/// Opcodes whose outbound body contains the account password in clear text:
/// the gateway login `0x6102` and the agent login `0x6103` (`packets/src/lib.rs`,
/// `docs/net-login-gateway.md`).
pub(crate) fn carries_credentials(opcode: u16) -> bool {
    matches!(opcode, 0x6102 | 0x6103)
}

/// The `packets_out` trace line for a frame.
///
/// `make netcheck` runs with `packets_out=trace`, which hex-dumps every frame —
/// including the two that carry the password, so the account password ended up
/// in terminal scrollback and in anything pasted from it
/// (`docs/re/ui/scene-intro-autologin.md` §Security). Those two frames are
/// summarised instead of dumped; the on-disk `packet_dump/` is unaffected,
/// since offline payload analysis is its whole point.
fn outbound_trace_line(frame: &SilkroadFrame) -> String {
    let opcode = match frame {
        SilkroadFrame::Packet { opcode, .. } | SilkroadFrame::MassiveHeader { opcode, .. } => {
            Some(*opcode)
        }
        SilkroadFrame::MassivePayload { .. } => None,
    };
    match opcode {
        Some(opcode) if carries_credentials(opcode) => {
            format!("{opcode:#06X} [redacted: carries credentials]")
        }
        _ => format!("{frame}"),
    }
}

fn send_packets(
    mut query: Query<(Entity, &mut SilkroadConnection)>,
    mut commands: Commands,
    mut dump: Option<ResMut<PacketDump>>,
    encrypt_outbound: Option<Res<OutboundEncryption>>,
) {
    for (entity, mut conn) in query.iter_mut() {
        if conn.o_receiver.is_empty() && conn.write_buf.is_empty() {
            continue;
        }

        let stream = conn.stream.clone();
        let Ok(mut stream) = stream.lock() else {
            continue;
        };
        let security = conn.get_security();
        let outbound = conn.o_receiver.clone();
        for mut frame in outbound.try_iter() {
            // Full outbound-frame dump for the headless net-check client.
            trace!(target: "packets_out", "send {}", outbound_trace_line(&frame));
            // Mirror of the inbound dump, taken before `serialize` so the
            // recorded body stays plaintext once outbound encryption is on.
            if let Some(dump) = dump.as_mut() {
                match &frame {
                    SilkroadFrame::Packet { opcode, data, .. }
                    | SilkroadFrame::MassiveHeader { opcode, data, .. } => {
                        dump.dump_sent(*opcode, data)
                    }
                    // continuation chunks carry no opcode of their own
                    SilkroadFrame::MassivePayload { .. } => {}
                }
            }
            // Always applied: the gameplay opcodes the original encrypts per
            // packet (item use) are not part of the login-encryption opt-in.
            frame.apply_send_encryption(&security, encrypt_outbound.is_some());
            let output = frame
                .serialize(security.clone())
                .expect("failed to serialize frame");
            conn.write_buf.extend_from_slice(&output);
        }
        if let Err(err) = flush_pending(&mut *stream, &mut conn.write_buf) {
            error!("stop handling outgoing packets because of: {}", err);
            conn.dc_sender
                .send(())
                .expect("failed to send disconnected msg");
            // Should we rather despawn? Currently the connection exists in its own entity
            commands.entity(entity).remove::<SilkroadConnection>();
        }
    }
}

fn fetch_disconnects(
    mut query: Query<(
        &mut SilkroadConnection,
        Option<&GatewayConnection>,
        Option<&AgentConnection>,
    )>,
    mut agent_dc_event_writer: MessageWriter<AgentConnectionDisconnected>,
    mut gateway_dc_event_writer: MessageWriter<GatewayServiceDisconnected>,
) {
    for (conn, gateway, agent) in query.iter_mut() {
        let disconnected = conn.get_dc_receiver().try_iter().next();
        if disconnected.is_some() {
            if gateway.is_some() {
                gateway_dc_event_writer.write(GatewayServiceDisconnected);
            } else if agent.is_some() {
                agent_dc_event_writer.write(AgentConnectionDisconnected);
            }
            continue;
        }
    }
}

fn keep_alive(
    mut query: Query<&mut SilkroadConnection>,
    time: Res<Time>,
    mut timer: Local<KeepAliveTimer>,
) {
    let timer = timer.timer.tick(time.delta());
    if !timer.just_finished() {
        return;
    }
    query.iter_mut().for_each(|conn| {
        let sender = conn.get_sender();
        let (opcode, data) = Packet::from(KeepAlive).into_serialize();
        let frame = SilkroadFrame::Packet {
            count: 0,
            crc: 0,
            opcode,
            encrypted: 0,
            data,
        };
        if let Err(e) = sender.send(frame) {
            error!("failed to send frame: {}", e.0);
        }
    });
}

pub(crate) fn on_global_state_update(mut events: MessageReader<GlobalStateUpdate>) {
    if let Some(res) = events.read().next() {
        info!("[GlobalStateUpdate]: {:?}", res);
    }
}

pub(crate) fn on_global_state_request(mut events: MessageReader<GlobalStateRequest>) {
    if let Some(res) = events.read().next() {
        info!("[GlobalStateRequest]: {:?}", res);
    }
}

fn on_agent_disconnected(
    mut reader: MessageReader<AgentConnectionDisconnected>,
    mut network_state: ResMut<NetworkState>,
    mut commands: Commands,
    query: Query<Entity, With<AgentConnection>>,
) {
    if let Some(_) = reader.read().next() {
        warn!("Agent Disconnected");
        network_state.agent = false;
        let Ok(entity) = query.single() else {
            return;
        };
        commands.entity(entity).despawn();
    }
}

fn on_gateway_disconnected(
    mut reader: MessageReader<GatewayServiceDisconnected>,
    mut network_state: ResMut<NetworkState>,
    mut commands: Commands,
    query: Query<Entity, With<GatewayConnection>>,
) {
    if let Some(_) = reader.read().next() {
        warn!("Gateway Disconnected");
        network_state.gateway = false;
        let Ok(entity) = query.single() else {
            return;
        };
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `0x2110` is received and dropped by the original without reading its body
    /// (`00842ff0:90-92`), so it must be listed as known-and-ignored rather than
    /// wired — and it must stay out of the `packets!` macro.
    #[test]
    fn known_ignored_opcodes_cover_0x2110_and_it_stays_unwired() {
        assert!(KNOWN_IGNORED_OPCODES.contains(&0x2110));
        assert!(matches!(
            Packet::deserialize(0x2110, Bytes::new()),
            Err(PacketError::UnknownOpcode(0x2110))
        ));
    }

    /// `0x3C81`'s handler in the original reads no bytes at all, so it is
    /// ignored rather than typed (#261). The second assertion is the one that
    /// matters: if someone later adds a body for it to the `packets!` macro,
    /// the ignore entry becomes a lie and this fails.
    #[test]
    fn academy_data_0x3c81_is_ignored_and_stays_unwired() {
        assert!(KNOWN_IGNORED_OPCODES.contains(&0x3C81));
        assert!(matches!(
            Packet::deserialize(0x3C81, Bytes::new()),
            Err(PacketError::UnknownOpcode(0x3C81))
        ));
    }

    /// A `SilkroadFrame::Packet` carrying `data`, as `send_packets` sees it.
    fn outbound_packet(opcode: u16, data: &[u8]) -> SilkroadFrame {
        SilkroadFrame::Packet {
            count: 0,
            crc: 0,
            opcode,
            encrypted: 0,
            data: Bytes::copy_from_slice(data),
        }
    }

    /// The password must not reach the `packets_out` trace: `make netcheck`
    /// runs that target at trace level, so anything logged there lands in
    /// terminal scrollback (`docs/re/ui/scene-intro-autologin.md` §Security).
    #[test]
    fn outbound_trace_redacts_the_login_frames() {
        // 0x6102 LoginRequest body shape: content_id | username | password | shard
        let mut body = vec![22u8];
        body.extend_from_slice(&4u16.to_le_bytes());
        body.extend_from_slice(b"user");
        body.extend_from_slice(&7u16.to_le_bytes());
        body.extend_from_slice(b"hunter2");
        body.extend_from_slice(&1u16.to_le_bytes());

        let gateway = outbound_trace_line(&outbound_packet(0x6102, &body));
        assert!(!gateway.contains("hunter2"), "{gateway}");
        assert!(
            !gateway.to_ascii_uppercase().contains("68756E74657232"),
            "{gateway}"
        );
        assert!(
            gateway.contains("0x6102") && gateway.contains("redacted"),
            "{gateway}"
        );

        // the agent login (0x6103) repeats the same password
        let agent = outbound_trace_line(&outbound_packet(0x6103, &body));
        assert!(!agent.contains("hunter2"), "{agent}");
        assert!(agent.contains("0x6103"), "{agent}");
    }

    /// Every other opcode keeps its full dump — the trace's whole purpose.
    #[test]
    fn outbound_trace_keeps_non_credential_frames_intact() {
        let line = outbound_trace_line(&outbound_packet(0x7021, &[0x01, 0x02, 0x03]));
        assert!(line.contains("Packet"), "{line}");
        assert!(!line.contains("redacted"), "{line}");
    }
}
