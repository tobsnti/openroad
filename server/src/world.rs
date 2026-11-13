//! The world: the single writer of all game state.
//!
//! Every request reaches it as a strictly decoded packet through
//! [`World::receive`], every answer leaves through the session's outbox, and
//! time moves only in [`World::step`]. Nothing else holds game state, so
//! nothing needs a lock, and a test can drive it frame by frame.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};

use bytes::Bytes;
use packets::agent::prelude::TalkResponse;
use packets::judge::{decode_exact, Verdict};
use packets::Packet;
use rand_core::{OsRng, RngCore};

use crate::outbox::{Frame, Outbox};
use crate::refdata::RefData;
use crate::registry::{handling, Handling};
use crate::store::{AccountId, Store};
use crate::wirelog::{Dir, WireLog};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub u32);

/// Why a client frame was not acted on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// No packet type exists for the opcode.
    UnknownOpcode,
    /// The body is not exactly one packet of its opcode: it does not decode,
    /// or it does not encode back to the same bytes (`packets::judge`).
    DecodeError,
    /// The packet is understood, but the server does not act on it in the
    /// session's phase.
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
    /// The server closed the session: the transport sends what is queued,
    /// then disconnects.
    Closed(SessionId),
    /// The session fell too far behind and was cut off.
    Overflowed(SessionId),
    /// A character's model has no reference-data row; its join was refused.
    NoRefData { session: SessionId, ref_id: u32 },
}

/// Where a session stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Connected, not logged in.
    Title,
    /// Logged in, choosing a character.
    Lobby(AccountId),
    /// The character record is sent; the client is loading the world.
    Joining { account: AccountId, uid: u32 },
    /// In the world as the entity `uid`.
    Game { account: AccountId, uid: u32 },
    /// Closed by the server; nothing more is acted on.
    Closed,
}

/// What the gateway hands over for one login: whose it is, under which name.
pub(crate) struct Ticket {
    pub account: AccountId,
    pub username: String,
}

struct Session {
    phase: Phase,
    outbox: Outbox,
    log: WireLog,
}

pub struct World {
    store: Box<dyn Store>,
    refdata: RefData,
    sessions: BTreeMap<SessionId, Session>,
    tickets: HashMap<u32, Ticket>,
    last_session: u32,
    last_uid: u32,
    step: u64,
    events: Vec<Event>,
}

impl World {
    pub fn new(store: impl Store + 'static, refdata: RefData) -> World {
        World {
            store: Box::new(store),
            refdata,
            sessions: BTreeMap::new(),
            tickets: HashMap::new(),
            last_session: 0,
            last_uid: 0,
            step: 0,
            events: Vec::new(),
        }
    }

    /// Let `account` in after the gateway checked its password: the returned
    /// token is good for one login as `username`.
    pub fn admit(&mut self, account: AccountId, username: &str) -> u32 {
        let ticket = Ticket {
            account,
            username: username.to_owned(),
        };
        loop {
            let token = OsRng.next_u32();
            if let Entry::Vacant(slot) = self.tickets.entry(token) {
                slot.insert(ticket);
                return token;
            }
        }
    }

    pub fn connect(&mut self) -> SessionId {
        self.last_session += 1;
        let id = SessionId(self.last_session);
        let session = Session {
            phase: Phase::Title,
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
        if s.phase == Phase::Closed {
            return;
        }
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
            (Ok(packet), Some(Handling::Answer(handler))) => {
                if let Err(reason) = handler(self, session, packet) {
                    self.events.push(reject(reason));
                }
            }
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

    pub fn phase(&self, session: SessionId) -> Option<Phase> {
        self.sessions.get(&session).map(|s| s.phase)
    }

    pub(crate) fn enter(&mut self, session: SessionId, phase: Phase) {
        if let Some(s) = self.sessions.get_mut(&session) {
            s.phase = phase;
            if phase == Phase::Closed {
                self.events.push(Event::Closed(session));
            }
        }
    }

    pub(crate) fn take_ticket(&mut self, token: u32) -> Option<Ticket> {
        self.tickets.remove(&token)
    }

    pub(crate) fn store(&self) -> &dyn Store {
        self.store.as_ref()
    }

    /// Characters in the game.
    pub(crate) fn online(&self) -> usize {
        let in_game = |s: &&Session| matches!(s.phase, Phase::Game { .. });
        self.sessions.values().filter(in_game).count()
    }

    /// Whether `account` has a session on the agent.
    pub(crate) fn connected(&self, account: AccountId) -> bool {
        self.sessions.values().any(|s| match s.phase {
            Phase::Lobby(a)
            | Phase::Joining { account: a, .. }
            | Phase::Game { account: a, .. } => a == account,
            Phase::Title | Phase::Closed => false,
        })
    }

    pub(crate) fn refdata(&self) -> &RefData {
        &self.refdata
    }

    /// Steps since the world started.
    pub(crate) fn steps(&self) -> u64 {
        self.step
    }

    /// A unique id for a new entity.
    pub(crate) fn next_uid(&mut self) -> u32 {
        self.last_uid += 1;
        self.last_uid
    }

    pub(crate) fn report(&mut self, event: Event) {
        self.events.push(event);
    }
}

pub(crate) fn reason_of(verdict: Verdict) -> Reason {
    match verdict {
        Verdict::Unwired => Reason::UnknownOpcode,
        _ => Reason::DecodeError,
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
