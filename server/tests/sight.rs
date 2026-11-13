//! Who sees whom, in-process: characters in one region are shown to each
//! other, see each other walk, and are taken back when they leave.

use std::sync::Arc;

use bytes::Bytes;
use packets::agent::prelude::{AgentLoginRequest, CharacterJoinRequest, MovementRequest};
use packets::Packet;
use server::refdata::RefData;
use server::store::{login, Character, MemoryStore, Position};
use server::world::{Phase, SessionId, World};

const REGION: u16 = 25000;

fn character(name: &str, region: u16) -> Character {
    Character {
        name: name.to_owned(),
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
            region,
            x: 969.0,
            y: 0.0,
            z: 1419.0,
            heading: 0,
        },
    }
}

/// Accounts a, b, c (each with its name as password) with Hero and Kiro in
/// one region and Far in the next; the store, for the gateway's login.
fn world() -> (World, Arc<MemoryStore>) {
    let mut store = MemoryStore::default();
    store.add_account("a", "a", vec![character("Hero", REGION)]);
    store.add_account("b", "b", vec![character("Kiro", REGION)]);
    store.add_account("c", "c", vec![character("Far", REGION + 1)]);
    let mut refdata = RefData::default();
    let mut row = vec![String::from("0"); 48];
    (row[1], row[46], row[47]) = ("1907".into(), "16".into(), "50".into());
    refdata.add_characters(&row.join("\t"));
    let store = Arc::new(store);
    (World::new(store.clone(), refdata), store)
}

fn send(world: &mut World, session: SessionId, packet: Packet) {
    let (opcode, body) = packet.into_serialize();
    world.receive(session, opcode, body);
}

/// `user`'s character in the game, as its client gets it there; what the
/// session was sent on the way is dropped.
fn enter(
    (world, store): &mut (World, Arc<MemoryStore>),
    user: &str,
    name: &str,
) -> (SessionId, u32) {
    let account = login(store.as_ref(), user, user).unwrap();
    let token = world.admit(account, user).unwrap();
    let s = world.connect();
    let login = AgentLoginRequest {
        token,
        username: user.to_owned(),
        password: String::new(),
        content_id: 22,
        mac_address: [0; 6],
    };
    send(world, s, Packet::AgentLoginRequest(login));
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));
    let join = CharacterJoinRequest {
        character_name: name.to_owned(),
    };
    send(world, s, Packet::CharacterJoinRequest(join));
    world.take_frames(s);
    world.receive(s, 0x3012, Bytes::new());
    let Some(Phase::Game { uid, .. }) = world.phase(s) else {
        panic!("{name} is in the game");
    };
    (s, uid)
}

/// The frames `session` was sent, but for the entering character's own values
/// (0x303D), which are not about sight.
fn opcodes(world: &mut World, session: SessionId) -> Vec<(u16, Bytes)> {
    let frames = world.take_frames(session).into_iter();
    frames
        .filter(|f| f.opcode != 0x303D)
        .map(|f| (f.opcode, f.body))
        .collect()
}

/// The unique id in a player's single spawn: behind the model id, five bytes
/// of build and inventory, and the four of its empty item sections.
fn spawned(body: &Bytes) -> u32 {
    u32::from_le_bytes(body[13..17].try_into().unwrap())
}

#[test]
fn two_characters_in_one_region_are_shown_to_each_other() {
    let mut shard = world();
    let (a, hero) = enter(&mut shard, "a", "Hero");
    assert!(
        opcodes(&mut shard.0, a).is_empty(),
        "alone, nothing to show"
    );
    let (b, kiro) = enter(&mut shard, "b", "Kiro");
    let to_a = opcodes(&mut shard.0, a);
    let to_b = opcodes(&mut shard.0, b);
    assert_eq!(
        (to_a.len(), to_a[0].0, spawned(&to_a[0].1)),
        (1, 0x3015, kiro)
    );
    assert_eq!(
        (to_b.len(), to_b[0].0, spawned(&to_b[0].1)),
        (1, 0x3015, hero)
    );
    // A standing character without gear or guild: 90 bytes and its name,
    // ending in the PK flag and the single spawn's last byte.
    let body = &to_a[0].1;
    assert_eq!(
        (body.len(), &body[body.len() - 2..]),
        (90 + 4, &[0xFF, 0x01][..])
    );
}

#[test]
fn a_walk_is_seen_by_whoever_was_shown_the_walker() {
    let mut shard = world();
    let (a, hero) = enter(&mut shard, "a", "Hero");
    let (b, _) = enter(&mut shard, "b", "Kiro");
    opcodes(&mut shard.0, a);
    opcodes(&mut shard.0, b);
    let walk = MovementRequest::ToLocation {
        region: REGION,
        x: 1119,
        y: 0,
        z: 1419,
    };
    send(&mut shard.0, a, Packet::MovementRequest(walk));
    for session in [a, b] {
        let seen = opcodes(&mut shard.0, session);
        assert_eq!(seen.len(), 1);
        assert_eq!(
            (seen[0].0, &seen[0].1[..4]),
            (0xB021, &hero.to_le_bytes()[..])
        );
    }
}

#[test]
fn a_character_that_enters_is_shown_a_walker_walking() {
    let mut shard = world();
    let (a, _) = enter(&mut shard, "a", "Hero");
    let walk = MovementRequest::ToLocation {
        region: REGION,
        x: 1119,
        y: 0,
        z: 1419,
    };
    send(&mut shard.0, a, Packet::MovementRequest(walk));
    let (b, _) = enter(&mut shard, "b", "Kiro");
    let to_b = opcodes(&mut shard.0, b);
    // Behind the 16-byte position, the movement block names the destination
    // (flag, type, region, three u16) instead of the standing source and
    // heading: five bytes more.
    let mut block = vec![1, 1];
    for v in [REGION, 1119, 0, 1419] {
        block.extend(v.to_le_bytes());
    }
    let body = &to_b[0].1;
    assert_eq!((to_b[0].0, body.len()), (0x3015, 94 + 5));
    assert_eq!(&body[33..43], &block[..]);
}

#[test]
fn a_character_in_another_region_is_not_shown() {
    let mut shard = world();
    let (a, _) = enter(&mut shard, "a", "Hero");
    let (c, _) = enter(&mut shard, "c", "Far");
    assert!(opcodes(&mut shard.0, a).is_empty());
    assert!(opcodes(&mut shard.0, c).is_empty());
}

#[test]
fn a_character_that_leaves_is_taken_back_once() {
    let mut shard = world();
    let (a, _) = enter(&mut shard, "a", "Hero");
    let (b, kiro) = enter(&mut shard, "b", "Kiro");
    opcodes(&mut shard.0, a);
    shard.0.disconnect(b);
    let gone = opcodes(&mut shard.0, a);
    assert_eq!(
        gone,
        [(0x3016, Bytes::copy_from_slice(&kiro.to_le_bytes()))]
    );
    shard.0.disconnect(b);
    assert!(opcodes(&mut shard.0, a).is_empty());
}
