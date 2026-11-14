use bytes::Bytes;
use packets::global::ModuleIdentification;

use sro_net::frame::SilkroadFrame;

pub mod connection;
mod handshake;

fn module_identification_frame() -> SilkroadFrame {
    let payload: Bytes = ModuleIdentification::client().into();
    SilkroadFrame::Packet {
        opcode: 0x2001,
        encrypted: 1,
        crc: 0,
        count: 0,
        data: payload,
    }
}

/// Logs what the spawn parser noted, one warning each, under the parser's own
/// log target: these are the lines it logged itself before the notes became
/// data, and the bench counts them by text.
pub fn log_spawn_notes(parsed: &packets::agent::entity_spawn::GroupSpawnParse) {
    for note in &parsed.notes {
        bevy::log::warn!(target: "client::net::entity_spawn", "{note}");
    }
}
