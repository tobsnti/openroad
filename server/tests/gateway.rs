//! The gateway in-process: everything a client does before the agent.

use std::net::Ipv4Addr;
use std::sync::Arc;

use bytes::Bytes;
use packets::agent::prelude::AgentLoginRequest;
use packets::gateway::{
    NoticeRequest, PatchRequest, ShardListPingRequest, ShardListRequest, ShardListResponse,
};
use packets::global::{KeepAlive, ModuleIdentification};
use packets::login::{LoginRequest, LoginResponse};
use packets::Packet;
use server::gateway::{Gateway, GatewayConnection};
use server::refdata::RefData;
use server::store::MemoryStore;
use server::world::{Reason, World};

fn gateway() -> Gateway {
    Gateway {
        farm_id: 20,
        farm_name: "OpenRoad".to_owned(),
        farm_ip: Ipv4Addr::LOCALHOST,
        shard_id: 64,
        shard_name: "Testserver".to_owned(),
        capacity: 1000,
        agent_ip: "127.0.0.1".to_owned(),
        agent_port: 15884,
    }
}

fn world() -> World {
    let mut store = MemoryStore::default();
    store.add_account("admin", "secret", Vec::new());
    World::new(Arc::new(store), RefData::default())
}

/// The answer to `packet`, as opcode and body.
fn ask(
    conn: &mut GatewayConnection,
    world: &mut World,
    packet: Packet,
) -> Result<(u16, Vec<u8>), Reason> {
    let (opcode, body) = packet.into_serialize();
    let frame = conn.receive(&gateway(), world, opcode, body)?;
    Ok((frame.opcode, frame.body.to_vec()))
}

fn login(conn: &mut GatewayConnection, world: &mut World, password: &str) -> (u16, Vec<u8>) {
    let request = LoginRequest {
        content_id: 22,
        username: "admin".to_owned(),
        password: password.to_owned(),
        shard_id: 64,
    };
    ask(conn, world, Packet::LoginRequest(request)).expect("a login is answered")
}

/// Logs `token` in on a new agent session.
fn enter_agent(world: &mut World, token: u32) {
    let s = world.connect();
    let request = AgentLoginRequest {
        token,
        username: "admin".to_owned(),
        password: "secret".to_owned(),
        content_id: 22,
        mac_address: [0; 6],
    };
    let (opcode, body) = Packet::AgentLoginRequest(request).into_serialize();
    world.receive(s, opcode, body);
    let answers: Vec<_> = world.take_frames(s).into_iter().map(|f| f.opcode).collect();
    assert_eq!(answers, [0xA103]);
}

fn token(answer: (u16, Vec<u8>)) -> u32 {
    let Ok(Packet::LoginResponse(LoginResponse {
        result: 1,
        login_info: Some(info),
        ..
    })) = Packet::deserialize(answer.0, Bytes::from(answer.1))
    else {
        panic!("a successful login");
    };
    assert_eq!(
        (info.agent_ip.as_str(), info.agent_port),
        ("127.0.0.1", 15884)
    );
    info.agent_token
}

#[test]
fn a_client_walks_the_gateway_and_enters_the_agent_with_its_token() {
    let mut world = world();
    let mut conn = GatewayConnection::open();
    let mut ask = |packet| ask(&mut conn, &mut world, packet).expect("answered");

    let ident = ModuleIdentification::client();
    assert_eq!(
        ask(Packet::ModuleIdentification(ident)),
        (0x2001, b"\x0d\x00GatewayServer\x00".to_vec())
    );
    let patch = PatchRequest {
        locale: 22,
        module_name: "SR_Client".to_owned(),
        version: 188,
    };
    assert_eq!(ask(Packet::PatchRequest(patch)), (0xA100, vec![0x01]));
    let notices = NoticeRequest { content_id: 22 };
    assert_eq!(ask(Packet::NoticeRequest(notices)), (0xA104, vec![0x00]));
    let (opcode, body) = ask(Packet::ShardListRequest(ShardListRequest));
    let Ok(Packet::ShardListResponse(ShardListResponse { farms, shards })) =
        Packet::deserialize(opcode, Bytes::from(body))
    else {
        panic!("0xA101 decodes");
    };
    assert_eq!(
        (farms.len(), farms[0].id, farms[0].name.as_str()),
        (1, 20, "OpenRoad")
    );
    let shard = &shards[0];
    assert_eq!(
        (shard.id, shard.online_count, shard.capacity, shard.farm_id),
        (64, 0, 1000, 20)
    );
    let ping = ask(Packet::ShardListPingRequest(ShardListPingRequest));
    assert_eq!(ping, (0xA106, vec![0x01, 20, 127, 0, 0, 1]));

    let token = token(login(&mut conn, &mut world, "secret"));
    enter_agent(&mut world, token);
}

/// The refusal names the attempt and the limit, "n of 6", as the captured
/// vSRO gateway does; an unknown account reads the same.
#[test]
fn a_wrong_password_is_counted_and_the_sixth_ends_the_connection() {
    let mut world = world();
    let mut conn = GatewayConnection::open();
    for attempt in 1..=6u32 {
        let answer = login(&mut conn, &mut world, "wrong");
        let mut refused = vec![0x02, 0x01, 6, 0, 0, 0];
        refused.extend(attempt.to_le_bytes());
        assert_eq!(answer, (0xA102, refused));
    }
    assert!(conn.closed());

    let mut unknown = GatewayConnection::open();
    let request = LoginRequest {
        content_id: 22,
        username: "nobody".to_owned(),
        password: "secret".to_owned(),
        shard_id: 64,
    };
    let answer = ask(&mut unknown, &mut world, Packet::LoginRequest(request));
    assert_eq!(
        answer,
        Ok((0xA102, vec![0x02, 0x01, 6, 0, 0, 0, 1, 0, 0, 0]))
    );
}

#[test]
fn an_account_already_on_the_agent_is_refused() {
    let mut world = world();
    let token = token(login(&mut GatewayConnection::open(), &mut world, "secret"));
    enter_agent(&mut world, token);
    let again = login(&mut GatewayConnection::open(), &mut world, "secret");
    assert_eq!(again, (0xA102, vec![0x02, 0x03]));
}

/// Strict reading holds at the gateway too, and before the login there is
/// nothing to keep: the connection ends.
#[test]
fn a_frame_that_is_not_exactly_a_gateway_request_ends_the_connection() {
    let mut world = world();
    let mut conn = GatewayConnection::open();
    let longer = conn.receive(&gateway(), &mut world, 0x6101, Bytes::from_static(&[0]));
    assert_eq!(longer.err(), Some(Reason::DecodeError));
    assert!(conn.closed());
}

/// The original client sends heartbeats from its first frames on; one closed
/// the gateway before (measured), now it gets an empty one back.
#[test]
fn a_heartbeat_is_answered_and_keeps_the_connection() {
    let mut world = world();
    let mut conn = GatewayConnection::open();
    let beat = ask(&mut conn, &mut world, Packet::KeepAlive(KeepAlive));
    assert_eq!(beat, Ok((0x2002, Vec::new())));
    assert!(!conn.closed());
    let notices = NoticeRequest { content_id: 22 };
    let answer = ask(&mut conn, &mut world, Packet::NoticeRequest(notices));
    assert_eq!(answer, Ok((0xA104, vec![0x00])));
}
