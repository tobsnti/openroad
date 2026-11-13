//! NPCs in-process: placed from the reference data, seen in groups by the
//! characters around them.

use std::sync::Arc;

use bytes::Bytes;
use packets::agent::prelude::{AgentLoginRequest, CharacterJoinRequest, MovementRequest};
use packets::Packet;
use server::refdata::RefData;
use server::store::{login, Character, MemoryStore, Position};
use server::world::{Phase, SessionId, World};

const REGION: u16 = 25000;

/// Rows as `npcpos.txt` has them: the smith in the start region (4 options),
/// a warehouse one region east (2 options), an NPC without options two
/// regions east, and one in a dungeon.
const NPCPOS: &str = "2003\t25000\t332.73001\t0.0\t1406.7\r\n\
                      2013\t25001\t980.79999\t-32.43\t989.13\r\n\
                      2028\t25002\t100.0\t0.0\t100.0\r\n\
                      15004\t-32764\t100.0\t0.0\t200.0\r\n";

fn hero() -> Character {
    Character {
        name: "Hero".to_owned(),
        ref_id: 1907,
        scale: 0x22,
        level: 10,
        exp: 0,
        skill_exp: 0,
        gold: 0,
        skill_points: 0,
        strength: 20,
        intellect: 20,
        stat_points: 0,
        hp: 1000,
        mp: 1000,
        inventory_size: 109,
        masteries: Vec::new(),
        position: Position {
            region: REGION,
            x: 969.0,
            y: 0.0,
            z: 1419.0,
            heading: 0,
        },
    }
}

fn world() -> (World, Arc<MemoryStore>) {
    let mut store = MemoryStore::default();
    store.add_account("a", "a", vec![hero()]);
    let mut refdata = RefData::default();
    let mut row = vec![String::from("0"); 48];
    (row[1], row[46], row[47]) = ("1907".into(), "16".into(), "50".into());
    refdata.add_characters(&row.join("\t"));
    refdata.add_npcs(NPCPOS);
    let store = Arc::new(store);
    (World::new(store.clone(), refdata), store)
}

fn send(world: &mut World, session: SessionId, packet: Packet) {
    let (opcode, body) = packet.into_serialize();
    world.receive(session, opcode, body);
}

/// Hero in the game; what the session was sent before 0x3012 is dropped.
fn enter((world, store): &mut (World, Arc<MemoryStore>)) -> (SessionId, u32) {
    let account = login(store.as_ref(), "a", "a").unwrap();
    let token = world.admit(account, "a").unwrap();
    let s = world.connect();
    let login = AgentLoginRequest {
        token,
        username: "a".to_owned(),
        password: String::new(),
        content_id: 22,
        mac_address: [0; 6],
    };
    send(world, s, Packet::AgentLoginRequest(login));
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));
    let join = CharacterJoinRequest {
        character_name: "Hero".to_owned(),
    };
    send(world, s, Packet::CharacterJoinRequest(join));
    world.take_frames(s);
    world.receive(s, 0x3012, Bytes::new());
    let Some(Phase::Game { uid, .. }) = world.phase(s) else {
        panic!("Hero is in the game");
    };
    (s, uid)
}

fn uid_of(world: &World, ref_id: u32) -> u32 {
    world.npcs().find(|(_, n)| n.ref_id == ref_id).unwrap().0
}

/// The kind of the one group in `frames` (1 spawn, 2 despawn) and the uids it
/// names: a despawn lists them, a spawn's records start with ref and uid and
/// are as long as their options make them.
fn group(frames: &[(u16, Bytes)]) -> (u8, Vec<u32>) {
    let codes: Vec<u16> = frames.iter().map(|f| f.0).collect();
    assert_eq!(codes, [0x3017, 0x3019, 0x3018]);
    let (kind, count) = (
        frames[0].1[0],
        u16::from_le_bytes([frames[0].1[1], frames[0].1[2]]),
    );
    let data = &frames[1].1;
    let u32_at = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
    let mut uids = Vec::new();
    let mut at = 0;
    while at < data.len() {
        if kind == 2 {
            uids.push(u32_at(at));
            at += 4;
            continue;
        }
        uids.push(u32_at(at + 4));
        // ref, uid, position 16, standing movement 5, state 17, then the
        // interaction: a 0, or 2, the count and the options.
        let interaction = at + 8 + 16 + 5 + 17;
        at = interaction
            + if data[interaction] == 0 {
                1
            } else {
                2 + data[interaction + 1] as usize
            };
    }
    assert_eq!((at, uids.len()), (data.len(), count as usize));
    (kind, uids)
}

/// The frames `session` was sent, but for the entering character's own values
/// (0x303D), which are not about sight.
fn frames(world: &mut World, session: SessionId) -> Vec<(u16, Bytes)> {
    let frames = world.take_frames(session).into_iter();
    frames
        .filter(|f| f.opcode != 0x303D)
        .map(|f| (f.opcode, f.body))
        .collect()
}

#[test]
fn npcs_stand_where_the_data_puts_them() {
    let (world, _) = world();
    let placed: Vec<(u32, u16, f32)> = world
        .npcs()
        .map(|(_, n)| (n.ref_id, n.position.region, n.position.x))
        .collect();
    assert_eq!(
        placed,
        [
            (2003, 25000, 332.73),
            (2013, 25001, 980.8),
            (2028, 25002, 100.0),
            (15004, 0x8004, 100.0)
        ]
    );
}

#[test]
fn a_character_entering_sees_the_npcs_around_it_in_one_group() {
    let mut shard = world();
    let (s, _) = enter(&mut shard);
    let world = &mut shard.0;
    let expected = vec![uid_of(world, 2003), uid_of(world, 2013)];
    assert_eq!(group(&frames(world, s)), (1, expected));
}

/// One region east, the NPC two regions east comes into sight; back home it
/// goes out of it again, each time as a group of one.
#[test]
fn walking_brings_npcs_into_sight_and_takes_them_back() {
    let mut shard = world();
    let (s, hero) = enter(&mut shard);
    let world = &mut shard.0;
    frames(world, s);
    let far = uid_of(world, 2028);
    for (region, x, kind) in [(REGION + 1, 100, 1), (REGION, 969, 2)] {
        let walk = MovementRequest::ToLocation {
            region,
            x,
            y: 0,
            z: 1419,
        };
        send(world, s, Packet::MovementRequest(walk));
        while world.player(hero).unwrap().walk.is_some() {
            world.step();
        }
        let seen = frames(world, s);
        assert_eq!(seen[0].0, 0xB021);
        assert_eq!(group(&seen[1..]), (kind, vec![far]));
    }
}
