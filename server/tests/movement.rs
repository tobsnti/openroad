//! Walking in-process: a character in the game asks to go somewhere and the
//! world moves it there at its run speed.

use std::sync::Arc;

use bytes::Bytes;
use packets::agent::prelude::{
    AgentLoginRequest, CharacterJoinRequest, MovementRequest, MovementResponse,
};
use packets::Packet;
use server::refdata::RefData;
use server::store::{login, Character, MemoryStore, Position};
use server::tick::STEPS_PER_SECOND;
use server::world::{Event, Phase, Reason, SessionId, World};

const REGION: u16 = 25000;

fn hero() -> Character {
    Character {
        name: "Hero".to_owned(),
        ref_id: 1907,
        scale: 34,
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
        inventory_size: 45,
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

fn send(world: &mut World, session: SessionId, packet: Packet) {
    let (opcode, body) = packet.into_serialize();
    world.receive(session, opcode, body);
}

/// A world with Hero, whose model runs at 50 units/s, and its session up to
/// the given point: joined (0x7001) and, with `ready`, in the game (0x3012).
fn joined(ready: bool) -> (World, SessionId, u32) {
    let mut store = MemoryStore::default();
    store.add_account("admin", "secret", vec![hero()]);
    let account = login(&store, "admin", "secret").unwrap();
    let mut refdata = RefData::default();
    let mut row = vec![String::from("0"); 48];
    (row[1], row[46], row[47]) = ("1907".into(), "16".into(), "50".into());
    refdata.add_characters(&row.join("\t"));
    let mut world = World::new(Arc::new(store), refdata);
    let token = world.admit(account, "admin").unwrap();
    let s = world.connect();
    let login = AgentLoginRequest {
        token,
        username: "admin".to_owned(),
        password: String::new(),
        content_id: 22,
        mac_address: [0; 6],
    };
    send(&mut world, s, Packet::AgentLoginRequest(login));
    // The client lists its characters before it joins one; the reading
    // contract's checker takes the lobby phase from this request.
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));
    let join = CharacterJoinRequest {
        character_name: "Hero".to_owned(),
    };
    send(&mut world, s, Packet::CharacterJoinRequest(join));
    if ready {
        world.receive(s, 0x3012, Bytes::new());
    }
    world.take_frames(s);
    world.take_events();
    let (Some(Phase::Joining { uid, .. }) | Some(Phase::Game { uid, .. })) = world.phase(s) else {
        panic!("the character joined");
    };
    (world, s, uid)
}

fn x_of(world: &World, uid: u32) -> f32 {
    world.player(uid).unwrap().position.x
}

#[test]
fn a_character_walks_to_its_destination_at_its_run_speed() {
    let (mut world, s, uid) = joined(true);
    let walk = MovementRequest::ToLocation {
        region: REGION,
        x: 1119,
        y: 0,
        z: 1419,
    };
    send(&mut world, s, Packet::MovementRequest(walk));
    let answer = MovementResponse {
        unique_id: uid,
        has_destination: true,
        region: REGION,
        x: 1119,
        y: 0,
        z: 1419,
        angle: 0,
    };
    let frames = world.take_frames(s);
    let expected = Packet::MovementResponse(answer).into_serialize();
    assert_eq!(frames.len(), 1);
    assert_eq!((frames[0].opcode, frames[0].body.clone()), expected);

    // 150 units at 50 units/s take three seconds.
    for _ in 1..3 * STEPS_PER_SECOND {
        world.step();
    }
    let left = 1119.0 - x_of(&world, uid);
    let step = 50.0 / STEPS_PER_SECOND as f32;
    assert!(
        left > 0.0 && left <= step + 1e-3,
        "one step short, {left} left"
    );
    world.step();
    assert_eq!(x_of(&world, uid), 1119.0);
    assert_eq!(world.player(uid).unwrap().walk, None);
}

#[test]
fn a_turn_stops_the_walk_and_is_answered_with_the_heading() {
    let (mut world, s, uid) = joined(true);
    let walk = MovementRequest::ToLocation {
        region: REGION,
        x: 1119,
        y: 0,
        z: 1419,
    };
    send(&mut world, s, Packet::MovementRequest(walk));
    world.step();
    send(
        &mut world,
        s,
        Packet::MovementRequest(MovementRequest::ToAngle { angle: 0x4000 }),
    );
    let frames = world.take_frames(s);
    let mut turned = uid.to_le_bytes().to_vec();
    turned.extend([0x00, 0x01, 0x00, 0x40, 0x00]);
    assert_eq!(
        (frames[1].opcode, frames[1].body.to_vec()),
        (0xB021, turned)
    );
    let stopped = x_of(&world, uid);
    world.step();
    assert_eq!(x_of(&world, uid), stopped);
}

/// A refused move gets no answer, as in the original.
#[test]
fn a_target_outside_the_region_is_refused_without_an_answer() {
    let (mut world, s, uid) = joined(true);
    for (region, x) in [(REGION, 1921), (REGION + 1, 100), (0x8001, 100)] {
        let walk = MovementRequest::ToLocation {
            region,
            x,
            y: 0,
            z: 100,
        };
        send(&mut world, s, Packet::MovementRequest(walk));
    }
    assert!(world.take_frames(s).is_empty());
    assert!(world.take_events().is_empty());
    assert_eq!(world.player(uid).unwrap().walk, None);
}

#[test]
fn a_move_before_the_game_is_not_handled() {
    let (mut world, s, _) = joined(false);
    let walk = MovementRequest::ToLocation {
        region: REGION,
        x: 1119,
        y: 0,
        z: 1419,
    };
    send(&mut world, s, Packet::MovementRequest(walk));
    assert!(world.take_frames(s).is_empty());
    let rejected = Event::Rejected {
        session: s,
        opcode: 0x7021,
        len: 9,
        reason: Reason::NotHandled,
    };
    assert_eq!(world.take_events(), [rejected]);
}
