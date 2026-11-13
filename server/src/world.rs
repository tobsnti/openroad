//! The world: the single writer of all game state.
//!
//! Every request reaches it as a strictly decoded packet through
//! [`World::receive`], every answer leaves through the session's outbox, and
//! time moves only in [`World::step`]. Nothing else holds game state, so
//! nothing needs a lock, and a test can drive it frame by frame.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use bytes::Bytes;
use packets::agent::ingame::{GROUP_DESPAWN, GROUP_SPAWN};
use packets::agent::prelude::{
    GroupEntitySpawnBegin, GroupEntitySpawnData, GroupEntitySpawnEnd, SingleEntityDespawn,
    TalkResponse,
};
use packets::judge::{decode_exact, Verdict};
use packets::Packet;
use rand_core::{OsRng, RngCore};

use crate::movement;
use crate::npc::Npc;
use crate::outbox::{Frame, Outbox};
use crate::refdata::RefData;
use crate::registry::{handling, Handling};
use crate::sight;
use crate::store::{AccountId, Position, Store};
use crate::tick::STEPS_PER_SECOND;
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
    /// The client has loaded the world (0x3012): its character is in the game.
    Joined { session: SessionId, uid: u32 },
}

/// A character in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    pub session: SessionId,
    pub name: String,
    pub position: Position,
    pub hp: u32,
    pub mp: u32,
    /// The characterdata row of its model.
    pub ref_id: u32,
    /// Height and build, a nibble each.
    pub scale: u8,
    pub inventory_size: u8,
    /// World units per second, the model's.
    pub walk_speed: f32,
    pub run_speed: f32,
    pub walk: Option<Walk>,
}

/// A walk in world coordinates (it may cross regions): from where, to where,
/// and the steps it has taken so far.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Walk {
    pub from: Point,
    pub to: Point,
    pub steps: u32,
}

/// A point in world units: region-local, or in world coordinates in a
/// [`Walk`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    pub z: f32,
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
    /// The step the session connected in.
    since: u64,
    /// The other characters' ids its client has been shown.
    known: BTreeSet<u32>,
}

/// The original client sends the agent login right after the handshake, 0.05
/// to 0.4 s after its gateway login [V capture, original client]. A session
/// that has not logged in after 30 s is gone, and its connection is closed.
const LOGIN_STEPS: u64 = 30 * STEPS_PER_SECOND;

pub struct World {
    store: Arc<dyn Store + Send + Sync>,
    refdata: RefData,
    sessions: BTreeMap<SessionId, Session>,
    /// Characters by unique id, from the join on.
    players: BTreeMap<u32, Player>,
    /// NPCs by unique id, placed from the reference data at the start.
    npcs: BTreeMap<u32, Npc>,
    /// The NPCs of each region, for sight.
    npcs_in: HashMap<u16, Vec<u32>>,
    /// Rows of `npcpos.txt` that name no NPC model (`crate::npc`).
    skipped_npc_rows: usize,
    tickets: HashMap<u32, Ticket>,
    last_session: u32,
    last_uid: u32,
    step: u64,
    events: Vec<Event>,
}

impl World {
    pub fn new(store: Arc<dyn Store + Send + Sync>, refdata: RefData) -> World {
        let placed = refdata.npcs().to_vec();
        let mut world = World {
            store,
            refdata,
            sessions: BTreeMap::new(),
            players: BTreeMap::new(),
            npcs: BTreeMap::new(),
            npcs_in: HashMap::new(),
            skipped_npc_rows: 0,
            tickets: HashMap::new(),
            last_session: 0,
            last_uid: 0,
            step: 0,
            events: Vec::new(),
        };
        for row in placed {
            let Ok(ref_id) = u32::try_from(row.ref_id) else {
                continue;
            };
            if !world.refdata.is_npc(ref_id) {
                world.skipped_npc_rows += 1;
                continue;
            }
            let position = Position {
                region: row.region,
                x: row.x,
                y: row.y,
                z: row.z,
                heading: 0,
            };
            let uid = world.next_uid();
            world.npcs_in.entry(row.region).or_default().push(uid);
            world.npcs.insert(uid, Npc { ref_id, position });
        }
        world
    }

    /// Rows of `npcpos.txt` that were not placed (`crate::npc`).
    pub fn skipped_npc_rows(&self) -> usize {
        self.skipped_npc_rows
    }

    pub fn npc(&self, uid: u32) -> Option<&Npc> {
        self.npcs.get(&uid)
    }

    pub fn npcs(&self) -> impl Iterator<Item = (u32, &Npc)> {
        self.npcs.iter().map(|(uid, npc)| (*uid, npc))
    }

    /// The NPCs standing in `region`.
    pub(crate) fn npcs_in(&self, region: u16) -> &[u32] {
        self.npcs_in.get(&region).map_or(&[], Vec::as_slice)
    }

    /// Let `account` in after the gateway checked its password: the returned
    /// token is good for one login as `username`. `None` while the account has
    /// a session on the agent.
    pub fn admit(&mut self, account: AccountId, username: &str) -> Option<u32> {
        if self.connected(account) {
            return None;
        }
        let ticket = Ticket {
            account,
            username: username.to_owned(),
        };
        loop {
            let token = OsRng.next_u32();
            if let Entry::Vacant(slot) = self.tickets.entry(token) {
                slot.insert(ticket);
                return Some(token);
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
            since: self.step,
            known: BTreeSet::new(),
        };
        self.sessions.insert(id, session);
        id
    }

    /// The session ends; whoever was shown its character is told it left.
    pub fn disconnect(&mut self, session: SessionId) {
        self.sessions.remove(&session);
        let gone: Vec<u32> = self
            .players
            .iter()
            .filter(|(_, player)| player.session == session)
            .map(|(uid, _)| *uid)
            .collect();
        for uid in gone {
            self.players.remove(&uid);
            for observer in self.observers(uid) {
                self.hide(observer, uid);
            }
        }
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
            (Ok(_), None) => self.events.push(reject(Reason::NotHandled)),
            (Err(rejected), _) => self.events.push(reject(reason_of(rejected))),
        }
    }

    /// Queue `packet` for `session`. A session that cannot take it any more is
    /// cut off and reported.
    pub fn send(&mut self, session: SessionId, packet: Packet) {
        let (opcode, body) = packet.into_serialize();
        self.push(session, opcode, body);
    }

    /// Queue `packet` for every session in `sessions`, encoded once: the
    /// sessions share its bytes.
    pub(crate) fn broadcast(&mut self, sessions: &[SessionId], packet: Packet) {
        let (opcode, body) = packet.into_serialize();
        for session in sessions {
            self.push(*session, opcode, body.clone());
        }
    }

    fn push(&mut self, session: SessionId, opcode: u16, body: Bytes) {
        let Some(s) = self.sessions.get_mut(&session) else {
            return;
        };
        s.log.record(Dir::S2c, opcode, &body);
        if s.outbox.push(Frame { opcode, body }).is_err() {
            self.sessions.remove(&session);
            self.events.push(Event::Overflowed(session));
        }
    }

    /// Shows the character `uid` to `session` with `spawn`, unless its client
    /// knows it already: a second spawn of an id it knows asserts in the
    /// original client, and so does any frame about one it does not
    /// [V original handler].
    pub(crate) fn show(&mut self, session: SessionId, uid: u32, spawn: Packet) {
        let Some(s) = self.sessions.get_mut(&session) else {
            return;
        };
        if s.known.insert(uid) {
            self.send(session, spawn);
        }
    }

    /// Shows `session` the entities of `records` (uid and spawn record) that
    /// its client does not know yet, as one group (0x3017, 0x3019, 0x3018).
    pub(crate) fn show_group(&mut self, session: SessionId, records: Vec<(u32, Bytes)>) {
        let Some(s) = self.sessions.get_mut(&session) else {
            return;
        };
        let new: Vec<Bytes> = records
            .into_iter()
            .filter(|(uid, _)| s.known.insert(*uid))
            .map(|(_, record)| record)
            .collect();
        self.group(session, GROUP_SPAWN, new.len(), new.concat());
    }

    /// Takes the entities `uids` back from `session`, those it was shown, as
    /// one group.
    pub(crate) fn hide_group(&mut self, session: SessionId, uids: Vec<u32>) {
        let Some(s) = self.sessions.get_mut(&session) else {
            return;
        };
        let gone: Vec<u32> = uids.into_iter().filter(|uid| s.known.remove(uid)).collect();
        let raw: Vec<u8> = gone.iter().flat_map(|uid| uid.to_le_bytes()).collect();
        self.group(session, GROUP_DESPAWN, gone.len(), raw);
    }

    fn group(&mut self, session: SessionId, kind: u8, count: usize, raw: Vec<u8>) {
        if count == 0 {
            return;
        }
        let count = u16::try_from(count).expect("a group counts in u16");
        let begin = GroupEntitySpawnBegin { kind, count };
        self.send(session, Packet::GroupEntitySpawnBegin(begin));
        let data = GroupEntitySpawnData { raw: raw.into() };
        self.send(session, Packet::GroupEntitySpawnData(data));
        self.send(session, Packet::GroupEntitySpawnEnd(GroupEntitySpawnEnd));
    }

    /// The entities `session`'s client was shown that are NPCs.
    pub(crate) fn shown_npcs(&self, session: SessionId) -> Vec<u32> {
        self.sessions.get(&session).map_or(Vec::new(), |s| {
            s.known
                .iter()
                .filter(|uid| self.npcs.contains_key(uid))
                .copied()
                .collect()
        })
    }

    /// Whether `session`'s client was shown the character `uid`.
    pub(crate) fn knows(&self, session: SessionId, uid: u32) -> bool {
        self.sessions
            .get(&session)
            .is_some_and(|s| s.known.contains(&uid))
    }

    /// Takes the character `uid` back from `session` (0x3016), if it was shown.
    pub(crate) fn hide(&mut self, session: SessionId, uid: u32) {
        let Some(s) = self.sessions.get_mut(&session) else {
            return;
        };
        if s.known.remove(&uid) {
            let gone = SingleEntityDespawn { unique_id: uid };
            self.send(session, Packet::SingleEntityDespawn(gone));
        }
    }

    /// The sessions whose clients know the character `uid`: its own and those
    /// it was shown to.
    pub(crate) fn observers(&self, uid: u32) -> Vec<SessionId> {
        let own = self.players.get(&uid).map(|p| p.session);
        let knows = |(id, s): (&SessionId, &Session)| s.known.contains(&uid) || Some(*id) == own;
        self.sessions
            .iter()
            .filter(|e| knows(*e))
            .map(|(id, _)| *id)
            .collect()
    }

    /// Advance the world by one step of [`crate::tick::STEP`].
    pub fn step(&mut self) {
        self.step += 1;
        let mut crossed = Vec::new();
        for (uid, player) in &mut self.players {
            let region = player.position.region;
            movement::advance(player);
            if player.position.region != region {
                crossed.push(*uid);
            }
        }
        for uid in crossed {
            sight::update(self, uid);
        }
        let late = |s: &Session| s.phase == Phase::Title && self.step - s.since >= LOGIN_STEPS;
        let late: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, s)| late(s))
            .map(|(id, _)| *id)
            .collect();
        for session in late {
            self.enter(session, Phase::Closed);
        }
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

    /// The characters in the game, by unique id.
    pub fn players(&self) -> impl Iterator<Item = (u32, &Player)> {
        let in_game = |p: &Player| matches!(self.phase(p.session), Some(Phase::Game { .. }));
        self.players
            .iter()
            .filter(move |(_, p)| in_game(p))
            .map(|(uid, p)| (*uid, p))
    }

    pub fn player(&self, uid: u32) -> Option<&Player> {
        self.players.get(&uid)
    }

    pub(crate) fn player_mut(&mut self, uid: u32) -> Option<&mut Player> {
        self.players.get_mut(&uid)
    }

    pub(crate) fn add_player(&mut self, uid: u32, player: Player) {
        self.players.insert(uid, player);
    }

    /// Characters in the game.
    pub fn online(&self) -> usize {
        let in_game = |s: &&Session| matches!(s.phase, Phase::Game { .. });
        self.sessions.values().filter(in_game).count()
    }

    /// Whether `account` has a session on the agent.
    fn connected(&self, account: AccountId) -> bool {
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
