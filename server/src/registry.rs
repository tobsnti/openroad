//! What the server does with each client opcode, in one table.
//!
//! Three kinds, because the original client treats requests differently. Two
//! of them lock their group in the client until the answer arrives — talking
//! to an NPC (0x7046) and leaving a stall (0x70B5) [V original handler; re-team
//! 2026-11-11] — so a `Locked` handler RETURNS its answer: the type allows
//! exactly one, and a body that does not decode still gets the refusal. An
//! `Answer` handler sends any number of frames; `Silent` sends none by design.
//! An opcode without an entry is reported, not answered.

use packets::Packet;

use crate::world::{self, SessionId, World};

pub enum Handling {
    Locked {
        answer: fn(&mut World, SessionId, Packet) -> Packet,
        refusal: fn() -> Packet,
    },
    Answer(fn(&mut World, SessionId, Packet)),
    Silent,
}

pub fn handling(opcode: u16) -> Option<Handling> {
    Some(match opcode {
        0x2002 => Handling::Silent,
        0x7046 => Handling::Locked {
            answer: world::talk,
            refusal: world::talk_refusal,
        },
        _ => return None,
    })
}
