//! The world: the single writer of all game state.
//!
//! Every request reaches it as a strictly decoded packet through
//! [`World::receive`], every answer leaves through the session's outbox, and
//! time moves only in [`World::step`]. Nothing else holds game state, so
//! nothing needs a lock, and a test can drive it frame by frame.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use bytes::Bytes;
use packets::agent::ingame::{GROUP_DESPAWN, GROUP_SPAWN};
use packets::agent::prelude::{
    GroupEntitySpawnBegin, GroupEntitySpawnData, GroupEntitySpawnEnd, MovementResponse,
    SingleEntityDespawn,
};
use packets::judge::{decode_exact, Verdict};
use packets::region::is_dungeon;
use packets::Packet;
use rand_core::{OsRng, RngCore};

use crate::combat::{self, Attack};
use crate::monster::{self, Monster};
use crate::movement;
use crate::npc::Npc;
use crate::outbox::{Frame, Outbox};
use crate::refdata::RefData;
use crate::registry::{handling, Handling};
use crate::sight;
use crate::store::{AccountId, Changes, Character, Position, Store};
use crate::tick::{fires, AI_STEPS, STEPS_PER_SECOND};
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
    /// A character's model has no player row in the reference data; its
    /// join was refused.
    NoRefData { session: SessionId, ref_id: u32 },
    /// The client has loaded the world (0x3012): its character is in the game.
    Joined { session: SessionId, uid: u32 },
}

/// A character in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    pub session: SessionId,
    /// The character as the store keeps it, kept current while it is in the
    /// world (its position, for one) and written back when it leaves.
    pub character: Character,
    /// World units per second, the model's.
    pub walk_speed: f32,
    pub run_speed: f32,
    pub walk: Option<Walk>,
    pub attack: Option<Attack>,
    /// The target of an attack request waiting behind the running swing.
    pub queued: Option<u32>,
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

/// What the gateway hands over for one login: whose it is, under which name,
/// and the account's characters, read where the password was checked.
pub(crate) struct Ticket {
    pub account: AccountId,
    pub username: String,
    pub characters: Vec<Character>,
}

struct Session {
    phase: Phase,
    outbox: Outbox,
    log: WireLog,
    /// The step the session connected in.
    since: u64,
    /// The other characters' ids its client has been shown.
    known: BTreeSet<u32>,
    /// The account's characters, from its ticket on.
    characters: Vec<Character>,
    /// The entity its client selected last.
    selected: Option<u32>,
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
    /// Monsters by unique id, placed from the reference data and the bench's
    /// fixture at the start.
    monsters: BTreeMap<u32, Monster>,
    /// The NPCs and monsters of each region, for sight.
    placed_in: HashMap<u16, Vec<u32>>,
    /// Rows of `npcpos.txt` that name neither an NPC nor a monster model.
    skipped_npc_rows: usize,
    /// What the store has to write, in order (`take_commits`).
    commits: Vec<Changes>,
    tickets: HashMap<u32, Ticket>,
    last_session: u32,
    last_uid: u32,
    step: u64,
    events: Vec<Event>,
    /// The world's chance: the monsters' wander steps and rests, the damage.
    chance: fastrand::Rng,
    /// The last action instance (0xB070), counted up per swing as the vSRO
    /// server does [V capture, peer vSRO: 5853 to 5899 over 47 swings].
    last_action: u32,
    /// Dead monsters, oldest first, with the step their bodies go.
    pub(crate) bodies: VecDeque<(u64, u32)>,
    /// Monsters to come back: the step, the model and the row.
    pub(crate) returning: Vec<(u64, u32, Position)>,
}

impl World {
    pub fn new(store: Arc<dyn Store + Send + Sync>, refdata: RefData) -> World {
        World::seeded(store, refdata, fastrand::u64(..))
    }

    /// A world whose chance starts from `seed`, so that a run repeats.
    pub fn seeded(store: Arc<dyn Store + Send + Sync>, refdata: RefData, seed: u64) -> World {
        let placed = refdata.npcs().to_vec();
        let mut world = World {
            store,
            refdata,
            sessions: BTreeMap::new(),
            players: BTreeMap::new(),
            npcs: BTreeMap::new(),
            monsters: BTreeMap::new(),
            placed_in: HashMap::new(),
            skipped_npc_rows: 0,
            commits: Vec::new(),
            tickets: HashMap::new(),
            last_session: 0,
            last_uid: 0,
            step: 0,
            events: Vec::new(),
            chance: fastrand::Rng::with_seed(seed),
            last_action: 0,
            bodies: VecDeque::new(),
            returning: Vec::new(),
        };
        for row in placed {
            let Ok(ref_id) = u32::try_from(row.ref_id) else {
                continue;
            };
            let position = Position {
                region: row.region,
                x: row.x,
                y: row.y,
                z: row.z,
                heading: 0,
            };
            if world.refdata.is_npc(ref_id) {
                let uid = world.next_uid();
                world.placed_in.entry(row.region).or_default().push(uid);
                world.npcs.insert(uid, Npc { ref_id, position });
            } else if world.place_monster(ref_id, position).is_none() {
                world.skipped_npc_rows += 1;
            }
        }
        world
    }

    /// Places a monster of model `ref_id` at `position`, at the full HP of its
    /// characterdata row, as the original does; its unique id, or `None` when
    /// `ref_id` is no monster model.
    pub fn place_monster(&mut self, ref_id: u32, position: Position) -> Option<u32> {
        let hp = self.refdata.monster(ref_id)?.max_hp;
        let uid = self.next_uid();
        self.placed_in.entry(position.region).or_default().push(uid);
        let monster = Monster {
            ref_id,
            position,
            hp,
            walk: None,
            home: position,
            rest_until: self.step + monster::rest(&mut self.chance),
        };
        self.monsters.insert(uid, monster);
        Some(uid)
    }

    /// Sends monster `uid` walking to `to`, a point in world coordinates, at
    /// its walk speed; the clients that know it are told where (0xB021), as
    /// for a character [V capture, peer vSRO: 4417 of 18164 monster moves
    /// carry no start position]. False when there is no such monster, or it
    /// stands in a dungeon, whose coordinates are outside the region grid.
    pub fn walk_monster(&mut self, uid: u32, to: Point) -> bool {
        let Some(monster) = self.monsters.get_mut(&uid) else {
            return false;
        };
        let at = monster.position;
        if is_dungeon(at.region) {
            return false;
        }
        let from = movement::in_world(
            at.region,
            Point {
                x: at.x,
                y: at.y,
                z: at.z,
            },
        );
        monster.walk = Some(Walk { from, to, steps: 0 });
        let (region, local) = movement::in_region(to);
        let answer = MovementResponse {
            unique_id: uid,
            has_destination: true,
            region,
            x: local.x as i32,
            y: local.y as i32,
            z: local.z as i32,
            angle: 0,
        };
        let observers = self.observers(uid);
        self.broadcast(&observers, Packet::MovementResponse(answer));
        true
    }

    /// Moves NPC or monster `uid` from the index of region `from` to that of
    /// region `to`.
    pub(crate) fn move_placed(&mut self, uid: u32, from: u16, to: u16) {
        if let Some(uids) = self.placed_in.get_mut(&from) {
            uids.retain(|placed| *placed != uid);
        }
        self.placed_in.entry(to).or_default().push(uid);
    }

    /// Rows of `npcpos.txt` that were not placed (`crate::npc`).
    pub fn skipped_npc_rows(&self) -> usize {
        self.skipped_npc_rows
    }

    pub fn monster(&self, uid: u32) -> Option<&Monster> {
        self.monsters.get(&uid)
    }

    pub(crate) fn monster_mut(&mut self, uid: u32) -> Option<&mut Monster> {
        self.monsters.get_mut(&uid)
    }

    pub(crate) fn remove_monster(&mut self, uid: u32) {
        if let Some(m) = self.monsters.remove(&uid) {
            let placed = self.placed_in.entry(m.position.region).or_default();
            placed.retain(|placed| *placed != uid);
        }
    }

    pub(crate) fn chance(&mut self) -> &mut fastrand::Rng {
        &mut self.chance
    }

    pub(crate) fn next_action(&mut self) -> u32 {
        self.last_action += 1;
        self.last_action
    }

    /// Whether a character attacks the monster `uid`.
    pub(crate) fn attacked(&self, uid: u32) -> bool {
        let at = |p: &Player| p.attack.is_some_and(|a| a.target == uid);
        self.players.values().any(at)
    }

    /// `session`'s client selected `uid`.
    pub(crate) fn select(&mut self, session: SessionId, uid: u32) {
        if let Some(s) = self.sessions.get_mut(&session) {
            s.selected = Some(uid);
        }
    }

    /// The sessions whose clients have `uid` selected.
    pub(crate) fn selecting(&self, uid: u32) -> Vec<SessionId> {
        let selects = |(_, s): &(&SessionId, &Session)| s.selected == Some(uid);
        self.sessions
            .iter()
            .filter(selects)
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn monsters(&self) -> impl Iterator<Item = (u32, &Monster)> {
        self.monsters.iter().map(|(uid, monster)| (*uid, monster))
    }

    pub fn npc(&self, uid: u32) -> Option<&Npc> {
        self.npcs.get(&uid)
    }

    pub fn npcs(&self) -> impl Iterator<Item = (u32, &Npc)> {
        self.npcs.iter().map(|(uid, npc)| (*uid, npc))
    }

    /// The NPCs and monsters standing in `region`.
    pub(crate) fn placed_in(&self, region: u16) -> &[u32] {
        self.placed_in.get(&region).map_or(&[], Vec::as_slice)
    }

    /// Let `account` in after the gateway checked its password: the returned
    /// token is good for one login as `username`. `None` while the account has
    /// a session on the agent. The caller reads the account's `characters`
    /// from the store, so the step never waits for it.
    pub fn admit(
        &mut self,
        account: AccountId,
        username: &str,
        characters: Vec<Character>,
    ) -> Option<u32> {
        if self.connected(account) {
            return None;
        }
        let ticket = Ticket {
            account,
            username: username.to_owned(),
            characters,
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
            characters: Vec::new(),
            selected: None,
        };
        self.sessions.insert(id, session);
        id
    }

    /// The session ends; whoever was shown its character is told it left,
    /// and the character as it left is due to the store.
    pub fn disconnect(&mut self, session: SessionId) {
        self.sessions.remove(&session);
        let gone: Vec<u32> = self
            .players
            .iter()
            .filter(|(_, player)| player.session == session)
            .map(|(uid, _)| *uid)
            .collect();
        for uid in gone {
            let Some(player) = self.players.remove(&uid) else {
                continue;
            };
            self.commits.push(Changes {
                characters: vec![player.character],
            });
            for observer in self.observers(uid) {
                self.hide(observer, uid);
            }
        }
    }

    /// What the store has to write, oldest first. The runtime hands it to the
    /// storage thread, so the step never waits for the store.
    pub fn take_commits(&mut self) -> Vec<Changes> {
        std::mem::take(&mut self.commits)
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

    /// The entities `session`'s client was shown that are NPCs or monsters.
    pub(crate) fn shown_placed(&self, session: SessionId) -> Vec<u32> {
        self.sessions.get(&session).map_or(Vec::new(), |s| {
            s.known
                .iter()
                .filter(|uid| self.npcs.contains_key(uid) || self.monsters.contains_key(uid))
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
            let region = player.character.position.region;
            let at = &mut player.character.position;
            movement::advance(&mut player.walk, player.run_speed, at);
            if player.character.position.region != region {
                crossed.push(*uid);
            }
        }
        for uid in crossed {
            sight::update(self, uid);
        }
        let mut crossed = Vec::new();
        let walking = self.monsters.iter_mut().filter(|(_, m)| m.walk.is_some());
        for (uid, monster) in walking {
            let from = monster.position.region;
            let speed = self
                .refdata
                .monster(monster.ref_id)
                .map_or(0.0, |d| d.walk_speed);
            movement::advance(&mut monster.walk, speed, &mut monster.position);
            if monster.position.region != from {
                crossed.push((*uid, from));
            }
        }
        for (uid, from) in crossed {
            sight::monster_crossed(self, uid, from);
        }
        combat::step(self);
        monster::tend(self);
        if fires(self.step, AI_STEPS) {
            monster::think(self);
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

    /// The characters of `session`'s account, as its ticket brought them.
    pub(crate) fn characters(&self, session: SessionId) -> &[Character] {
        self.sessions
            .get(&session)
            .map_or(&[], |s| s.characters.as_slice())
    }

    pub(crate) fn hold_characters(&mut self, session: SessionId, characters: Vec<Character>) {
        if let Some(s) = self.sessions.get_mut(&session) {
            s.characters = characters;
        }
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
