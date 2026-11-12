//! Login and lobby in-process: the gateway's ticket, the agent's answers.

use bytes::Bytes;
use packets::agent::prelude::*;
use packets::global::ModuleIdentification;
use packets::Packet;
use server::store::{login, Character, MemoryStore};
use server::world::{Event, Phase, Reason, SessionId, World};

fn hero() -> Character {
    Character {
        name: "Hero".to_owned(),
        ref_id: 1907,
        scale: 34,
        level: 10,
        exp: 120,
        strength: 29,
        intellect: 29,
        stat_points: 3,
        hp: 489,
        mp: 489,
    }
}

/// A world whose store holds `admin` with one character, and a ticket the
/// gateway would have issued after checking the password.
fn admitted() -> (World, u32) {
    let mut store = MemoryStore::default();
    store.add_account("admin", "secret", vec![hero()]);
    let account = login(&store, "admin", "secret").expect("the seeded password logs in");
    let mut world = World::new(store);
    let ticket = world.admit(account, "admin");
    (world, ticket)
}

fn send(world: &mut World, session: SessionId, packet: Packet) {
    let (opcode, body) = packet.into_serialize();
    world.receive(session, opcode, body);
}

fn log_in(world: &mut World, session: SessionId, token: u32, username: &str) {
    let request = AgentLoginRequest {
        token,
        username: username.to_owned(),
        password: "secret".to_owned(),
        content_id: 0,
        mac_address: [0; 6],
    };
    send(world, session, Packet::AgentLoginRequest(request));
}

fn frames(world: &mut World, session: SessionId) -> Vec<(u16, Vec<u8>)> {
    let frames = world.take_frames(session);
    frames
        .into_iter()
        .map(|f| (f.opcode, f.body.to_vec()))
        .collect()
}

#[test]
fn a_client_with_a_ticket_logs_in_and_lists_its_characters() {
    let (mut world, ticket) = admitted();
    let s = world.connect();
    send(
        &mut world,
        s,
        Packet::ModuleIdentification(ModuleIdentification::client()),
    );
    log_in(&mut world, s, ticket, "admin");
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));

    let mut out = frames(&mut world, s).into_iter();
    assert_eq!(
        out.next(),
        Some((0x2001, b"\x0b\x00AgentServer\x00".to_vec()))
    );
    assert_eq!(out.next(), Some((0xA103, vec![0x01])));
    let (opcode, body) = out.next().expect("the character list");
    assert_eq!(opcode, 0xB007);
    let Ok(Packet::CharacterSelectionActionResponse(list)) =
        Packet::deserialize(opcode, body.into())
    else {
        panic!("0xB007 does not decode");
    };
    let listed = list.characters.expect("a list on success").characters;
    let listed: Vec<_> = listed.iter().map(|c| (c.name.as_str(), c.level)).collect();
    assert_eq!(listed, [("Hero", 10)]);
    assert_eq!(out.next(), None);
    assert!(world.take_events().is_empty());
}

#[test]
fn a_ticket_is_good_for_one_login() {
    let (mut world, ticket) = admitted();
    let first = world.connect();
    log_in(&mut world, first, ticket, "admin");
    let second = world.connect();
    log_in(&mut world, second, ticket, "admin");
    assert_eq!(frames(&mut world, second), [(0xA103, vec![0x02, 0x01])]);
    assert_eq!(world.take_events(), [Event::Closed(second)]);
    assert_eq!(world.phase(second), Some(Phase::Closed));
}

#[test]
fn a_ticket_is_bound_to_the_name_it_was_issued_for() {
    let (mut world, ticket) = admitted();
    let s = world.connect();
    log_in(&mut world, s, ticket, "someone");
    assert_eq!(frames(&mut world, s), [(0xA103, vec![0x02, 0x01])]);
    assert_eq!(world.phase(s), Some(Phase::Closed));
}

#[test]
fn the_lobby_answers_only_after_login() {
    let (mut world, _) = admitted();
    let s = world.connect();
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));
    assert!(frames(&mut world, s).is_empty());
    let rejected = Event::Rejected {
        session: s,
        opcode: 0x7007,
        len: 1,
        reason: Reason::NotHandled,
    };
    assert_eq!(world.take_events(), [rejected]);
}
