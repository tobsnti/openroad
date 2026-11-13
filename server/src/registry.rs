//! What the server does with each client opcode, in one table.
//!
//! Two kinds, because the original client treats requests differently. Two
//! of them lock their group in the client until the answer arrives — talking
//! to an NPC (0x7046) and leaving a stall (0x70B5) [V original handler] — so a
//! `Locked` handler RETURNS its answer: the type allows exactly one, and a
//! body that does not decode still gets the refusal. An `Answer` handler sends
//! any number of frames, or reports why it did not act. An opcode without an
//! entry is reported, not answered.

use packets::Packet;

use crate::world::{self, Reason, SessionId, World};
use crate::{join, lobby, movement};

pub enum Handling {
    Locked {
        answer: fn(&mut World, SessionId, Packet) -> Packet,
        refusal: fn() -> Packet,
    },
    Answer(fn(&mut World, SessionId, Packet) -> Result<(), Reason>),
}

pub fn handling(opcode: u16) -> Option<Handling> {
    Some(match opcode {
        0x2001 => Handling::Answer(lobby::identify),
        0x2002 => Handling::Answer(lobby::keep_alive),
        0x3012 => Handling::Answer(join::game_ready),
        0x6103 => Handling::Answer(lobby::login),
        0x7001 => Handling::Answer(join::join),
        0x7021 => Handling::Answer(movement::request),
        0x7007 => Handling::Answer(lobby::lobby),
        0x7046 => Handling::Locked {
            answer: world::talk,
            refusal: world::talk_refusal,
        },
        _ => return None,
    })
}
