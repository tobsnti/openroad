//! The server on sockets: a client walks the gateway and enters the world over
//! loopback, and a connection's limits hold.

use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use packets::agent::prelude::{AgentLoginRequest, CharacterJoinRequest};
use packets::gateway::PatchRequest;
use packets::login::{LoginRequest, LoginResponse};
use packets::Packet;
use server::gateway::Gateway;
use server::refdata::RefData;
use server::runtime::{start, Config, Running, GATEWAY_IDLE, HANDSHAKE};
use server::store::{Character, MemoryStore, Position, Store};
use server::world::World;

mod common;
use common::Wire;

fn hero() -> Character {
    Character {
        name: "Hero".to_owned(),
        ref_id: 1907,
        scale: 34,
        level: 10,
        exp: 0,
        skill_exp: 0,
        gold: 100,
        skill_points: 0,
        strength: 20,
        intellect: 20,
        stat_points: 0,
        hp: 1000,
        mp: 1000,
        inventory_size: 45,
        masteries: Vec::new(),
        position: Position {
            region: 25000,
            x: 969.0,
            y: 0.0,
            z: 1419.0,
            heading: 0,
        },
    }
}

fn server(capacity: u16, handshake: Duration, gateway_idle: Duration) -> Running {
    let mut store = MemoryStore::default();
    store.add_account("admin", "secret", vec![hero()]);
    let store: Arc<dyn Store + Send + Sync> = Arc::new(store);
    let mut refdata = RefData::default();
    let mut row = vec![String::from("0"); 48];
    (row[1], row[46], row[47]) = ("1907".into(), "16".into(), "50".into());
    refdata.add_characters(&row.join("\t"));
    let loopback = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    let config = Config {
        gateway: Gateway {
            farm_id: 20,
            farm_name: "OpenRoad".to_owned(),
            farm_ip: Ipv4Addr::LOCALHOST,
            shard_id: 64,
            shard_name: "Testserver".to_owned(),
            capacity,
            agent_ip: "127.0.0.1".to_owned(),
            agent_port: 0,
        },
        gateway_listen: loopback,
        agent_listen: loopback,
        handshake,
        gateway_idle,
    };
    let world = World::new(store.clone(), refdata);
    start(config, world, store, |_, _| {}).expect("the server starts")
}

fn bytes(packet: Packet) -> Bytes {
    packet.into_serialize().1
}

#[test]
fn a_client_walks_the_gateway_and_enters_the_world_over_sockets() {
    let server = server(10, HANDSHAKE, GATEWAY_IDLE);
    let mut gateway = Wire::connect(server.gateway);
    // The original client encrypts these on the way out [V capture, original client].
    gateway.send(0x2001, b"\x09\x00SR_Client\x00", true);
    assert_eq!(gateway.recv().0, 0x2001);
    let patch = PatchRequest {
        locale: 22,
        module_name: "SR_Client".to_owned(),
        version: 188,
    };
    gateway.send(0x6100, &bytes(Packet::PatchRequest(patch)), true);
    assert_eq!(gateway.recv(), (0xA100, Bytes::from_static(&[0x01])));
    let login = LoginRequest {
        content_id: 22,
        username: "admin".to_owned(),
        password: "secret".to_owned(),
        shard_id: 64,
    };
    gateway.send(0x6102, &bytes(Packet::LoginRequest(login)), true);
    let (opcode, body) = gateway.recv();
    let Ok(Packet::LoginResponse(LoginResponse {
        login_info: Some(agent),
        ..
    })) = Packet::deserialize(opcode, body)
    else {
        panic!("a successful login");
    };
    assert_eq!(agent.agent_port, server.agent.port());

    let mut agent_wire = Wire::connect(server.agent);
    agent_wire.send(0x2001, b"\x09\x00SR_Client\x00", true);
    assert_eq!(agent_wire.recv().0, 0x2001);
    let login = AgentLoginRequest {
        token: agent.agent_token,
        username: "admin".to_owned(),
        password: "secret".to_owned(),
        content_id: 22,
        mac_address: [0; 6],
    };
    agent_wire.send(0x6103, &bytes(Packet::AgentLoginRequest(login)), true);
    assert_eq!(agent_wire.recv(), (0xA103, Bytes::from_static(&[0x01])));
    agent_wire.send(0x7007, &[0x02], false);
    assert_eq!(agent_wire.recv().0, 0xB007);
    let join = CharacterJoinRequest {
        character_name: "Hero".to_owned(),
    };
    agent_wire.send(0x7001, &bytes(Packet::CharacterJoinRequest(join)), false);
    let entry: Vec<u16> = (0..5).map(|_| agent_wire.recv().0).collect();
    assert_eq!(entry, [0xB001, 0x34A5, 0x3013, 0x34A6, 0x3020]);
    agent_wire.send(0x3012, &[], false);
    let names = || server.query(|world| world.players().map(|(_, p)| p.name.clone()).collect());
    let started = Instant::now();
    while names() != Some(vec!["Hero".to_owned()]) {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the character enters the game"
        );
    }
}

/// Reads what the server sends until it closes the connection; how long that
/// took.
fn until_closed(mut stream: TcpStream) -> Duration {
    let started = Instant::now();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut buf = [0u8; 256];
    while stream.read(&mut buf).expect("closed, not timed out") > 0 {}
    started.elapsed()
}

#[test]
fn a_connection_that_does_not_finish_its_handshake_is_closed() {
    let server = server(10, Duration::from_millis(300), GATEWAY_IDLE);
    for listener in [server.agent, server.gateway] {
        let silent = TcpStream::connect(listener).unwrap();
        assert!(until_closed(silent) < Duration::from_secs(2));
    }
}

#[test]
fn a_silent_gateway_connection_is_closed() {
    let server = server(10, HANDSHAKE, Duration::from_millis(300));
    let gateway = Wire::connect(server.gateway);
    assert!(until_closed(gateway.into_stream()) < Duration::from_secs(2));
}

/// Each connection binds threads, so a listener holds no more than the
/// shard's capacity; the next is closed at once.
#[test]
fn a_listener_holds_at_most_the_shards_capacity() {
    let server = server(2, HANDSHAKE, GATEWAY_IDLE);
    let _held = [Wire::connect(server.agent), Wire::connect(server.agent)];
    let third = TcpStream::connect(server.agent).unwrap();
    assert!(until_closed(third) < Duration::from_secs(1));
}
