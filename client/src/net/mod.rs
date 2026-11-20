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

/// Reads a single spawn (0x3015) to its end, or says that it did not, under the
/// parser's log target and in the words the bench counts an unparsed spawn
/// record by — the group frames' notes say it the same way.
pub fn read_single_spawn(
    raw: &[u8],
    resolver: &impl packets::agent::entity_spawn::RefResolver,
) -> Option<packets::agent::entity_spawn::SpawnedEntity> {
    let read = packets::agent::entity_spawn::parse_single_spawn(raw, resolver);
    if read.is_none() {
        bevy::log::warn!(
            target: "client::net::entity_spawn",
            "entity_spawn: could not parse spawn record of a single spawn, {} bytes: {}",
            raw.len(),
            packets::hexdump(raw, 192)
        );
    }
    read.map(|(entity, _tail)| entity)
}
