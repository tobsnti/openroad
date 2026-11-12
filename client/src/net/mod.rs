use bytes::Bytes;
use packets::global::ModuleIdentification;

use sro_net::frame::SilkroadFrame;

pub mod connection;
pub mod entity_spawn;
mod handshake;
pub mod reader;

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
