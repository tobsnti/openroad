//! The world: the single writer of all game state.
//!
//! Every request reaches it as a strictly decoded packet through
//! [`World::receive`], every answer leaves through the session's outbox, and
//! time moves only in [`World::step`]. Nothing else holds game state, so
//! nothing needs a lock, and a test can drive it frame by frame.

use std::collections::BTreeMap;

use bytes::Bytes;
use packets::agent::prelude::TalkResponse;
use packets::Packet;

use crate::inbound::{decode_exact, Rejected};
use crate::outbox::{Frame, Outbox};
use crate::registry::{handling, Handling};
use crate::wirelog::{Dir, WireLog};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub u32);

/// Why a client frame was not acted on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// No packet type exists for the opcode.
    UnknownOpcode,
    /// The body is not exactly one packet of its opcode ([`crate::inbound`]).
    DecodeError,
    /// The packet is understood, but the server does not act on it.
    NotHandled,
}

/// What happened that the wire does not show.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Rejected {
        session: SessionId,
        opcode: u16,
        len: usize,
        reason: Reason,
    },
    /// The session fell too far behind and was cut off.
    Overflowed(SessionId),
}

struct Session {
    outbox: Outbox,
    log: WireLog,
}

#[derive(Default)]
pub struct World {
    sessions: BTreeMap<SessionId, Session>,
    last_session: u32,
    step: u64,
    events: Vec<Event>,
}

impl World {
    pub fn new() -> World {
        World::default()
    }

    pub fn connect(&mut self) -> SessionId {
        self.last_session += 1;
        let id = SessionId(self.last_session);
        let session = Session {
            outbox: Outbox::default(),
            log: WireLog::open("agent"),
        };
        self.sessions.insert(id, session);
        id
    }

    pub fn disconnect(&mut self, session: SessionId) {
        self.sessions.remove(&session);
    }

    /// One client frame, as the transport cut it out of the stream.
    pub fn receive(&mut self, session: SessionId, opcode: u16, body: Bytes) {
        let Some(s) = self.sessions.get_mut(&session) else {
            return;
        };
        s.log.record(Dir::C2s, opcode, &body);
        let len = body.len();
        let reject = |reason| Event::Rejected {
            session,
            opcode,
            len,
            reason,
        };
        match (decode_exact(opcode, &body), handling(opcode)) {
            (Ok(packet), Some(Handling::Locked { answer, .. })) => {
                let answer = answer(self, session, packet);
                self.send(session, answer);
            }
            (Err(rejected), Some(Handling::Locked { refusal, .. })) => {
                self.events.push(reject(reason_of(rejected)));
                self.send(session, refusal());
            }
            (Ok(packet), Some(Handling::Answer(handler))) => handler(self, session, packet),
            (Ok(_), Some(Handling::Silent)) => {}
            (Ok(_), None) => self.events.push(reject(Reason::NotHandled)),
            (Err(rejected), _) => self.events.push(reject(reason_of(rejected))),
        }
    }

    /// Queue `packet` for `session`. A session that cannot take it any more is
    /// cut off and reported.
    pub fn send(&mut self, session: SessionId, packet: Packet) {
        let Some(s) = self.sessions.get_mut(&session) else {
            return;
        };
        let (opcode, body) = packet.into_serialize();
        s.log.record(Dir::S2c, opcode, &body);
        if s.outbox.push(Frame { opcode, body }).is_err() {
            self.sessions.remove(&session);
            self.events.push(Event::Overflowed(session));
        }
    }

    /// Advance the world by one step of [`crate::tick::STEP`].
    pub fn step(&mut self) {
        self.step += 1;
    }

    /// The frames `session` has to send, oldest first.
    pub fn take_frames(&mut self, session: SessionId) -> Vec<Frame> {
        self.sessions
            .get_mut(&session)
            .map(|s| s.outbox.take())
            .unwrap_or_default()
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}

fn reason_of(rejected: Rejected) -> Reason {
    match rejected {
        Rejected::UnknownOpcode => Reason::UnknownOpcode,
        Rejected::Undecodable | Rejected::NotExact => Reason::DecodeError,
    }
}

/// 0x7046. This world holds no NPC yet, so every talk is refused.
pub(crate) fn talk(_: &mut World, _: SessionId, _: Packet) -> Packet {
    talk_refusal()
}

/// Code 0, which the client words as "invalid target" [V original handler].
pub(crate) fn talk_refusal() -> Packet {
    Packet::TalkResponse(TalkResponse::Failure(0))
}
