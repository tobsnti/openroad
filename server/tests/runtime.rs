//! The server on sockets: a client walks the gateway and enters the world over
//! loopback, and a connection's limits hold.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use packets::agent::prelude::{AgentLoginRequest, CharacterJoinRequest, MovementRequest};
use packets::gateway::PatchRequest;
use packets::login::{LoginInfo, LoginRequest, LoginResponse};
use packets::Packet;
use server::gateway::Gateway;
use server::refdata::RefData;
use server::runtime::{start, Config, Notice, Peer, Running, Why, GATEWAY_IDLE, HANDSHAKE};
use server::store::{Character, MemoryStore, Position, Store};
use server::transport::Broken;
use server::world::World;
use sro_net::sequence::Sequence;

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

/// A running server with `admin` (password `secret`) and Hero, and its store.
fn server(
    capacity: u16,
    handshake: Duration,
    gateway_idle: Duration,
) -> (Running, Arc<MemoryStore>) {
    noticed(capacity, handshake, gateway_idle, |_| {})
}

/// [`server`], its connections' notices going to `notice`.
fn noticed(
    capacity: u16,
    handshake: Duration,
    gateway_idle: Duration,
    notice: impl Fn(Notice) + Send + Sync + 'static,
) -> (Running, Arc<MemoryStore>) {
    started(config(capacity, handshake, gateway_idle), notice)
}

/// Loopback listeners on ports of their own; the gateway announces the
/// agent's.
fn config(capacity: u16, handshake: Duration, gateway_idle: Duration) -> Config {
    let loopback = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    Config {
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
    }
}

/// A server on `config` with `admin` (password `secret`) and Hero.
fn started(
    config: Config,
    notice: impl Fn(Notice) + Send + Sync + 'static,
) -> (Running, Arc<MemoryStore>) {
    let mut store = MemoryStore::default();
    store.add_account("admin", "secret", vec![hero()]);
    let store = Arc::new(store);
    let mut refdata = RefData::default();
    let mut row = vec![String::from("0"); 48];
    (row[1], row[46], row[47]) = ("1907".into(), "16".into(), "50".into());
    refdata.add_characters(&row.join("\t"));
    let world = World::new(store.clone(), refdata);
    let running =
        start(config, world, store.clone(), |_, _| {}, notice).expect("the server starts");
    (running, store)
}

fn bytes(packet: Packet) -> Bytes {
    packet.into_serialize().1
}

/// `admin` logs in at the gateway: where it says the agent is.
fn gateway_login(server: &Running) -> LoginInfo {
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
    agent
}

#[test]
fn a_client_walks_the_gateway_and_enters_the_world_over_sockets() {
    let (server, store) = server(10, HANDSHAKE, GATEWAY_IDLE);
    let agent = gateway_login(&server);
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
    let names = || {
        server.query(|world| {
            world
                .players()
                .map(|(_, p)| p.character.name.clone())
                .collect()
        })
    };
    let started = Instant::now();
    while names() != Some(vec!["Hero".to_owned()]) {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the character enters the game"
        );
    }

    // It turns and leaves: the storage thread writes it as it left.
    let turn = MovementRequest::ToAngle { angle: 0x4000 };
    agent_wire.send(0x7021, &bytes(Packet::MovementRequest(turn)), false);
    drop(agent_wire);
    let account = store.account("admin").expect("seeded").0;
    let heading = || store.characters(account)[0].position.heading;
    let started = Instant::now();
    while heading() != 0x4000 {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the store has the character as it left"
        );
    }
}

/// A client that reaches the agent through a relay is told the relay's
/// address, not the one the agent listens on.
#[test]
fn the_gateway_announces_the_agent_address_it_is_given() {
    let mut config = config(10, HANDSHAKE, GATEWAY_IDLE);
    (config.gateway.agent_ip, config.gateway.agent_port) = ("192.0.2.7".to_owned(), 4002);
    let (server, _) = started(config, |_| {});
    let agent = gateway_login(&server);
    assert_eq!(
        (agent.agent_ip.as_str(), agent.agent_port),
        ("192.0.2.7", 4002)
    );
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
    let (server, _) = server(10, Duration::from_millis(300), GATEWAY_IDLE);
    for listener in [server.agent, server.gateway] {
        let silent = TcpStream::connect(listener).unwrap();
        assert!(until_closed(silent) < Duration::from_secs(2));
    }
}

#[test]
fn a_silent_gateway_connection_is_closed() {
    let (server, _) = server(10, HANDSHAKE, Duration::from_millis(300));
    let gateway = Wire::connect(server.gateway);
    assert!(until_closed(gateway.into_stream()) < Duration::from_secs(2));
}

/// Each connection binds threads, so a listener holds no more than the
/// shard's capacity; the next is closed at once.
#[test]
fn a_listener_holds_at_most_the_shards_capacity() {
    let (server, _) = server(2, HANDSHAKE, GATEWAY_IDLE);
    let _held = [Wire::connect(server.agent), Wire::connect(server.agent)];
    let third = TcpStream::connect(server.agent).unwrap();
    assert!(until_closed(third) < Duration::from_secs(1));
}

/// The server says why it let a connection go, with the seeds the client was
/// to stamp its frames with: here an answer whose sequence byte is wrong.
#[test]
fn a_connection_ended_over_its_security_bytes_is_noticed_with_its_seeds() {
    let notices = Arc::new(Mutex::new(Vec::new()));
    let heard = notices.clone();
    let (server, _) = noticed(10, HANDSHAKE, GATEWAY_IDLE, move |notice| {
        heard.lock().unwrap().push(notice)
    });
    let mut client = TcpStream::connect(server.agent).unwrap();
    let mut opening = [0u8; 6 + 37];
    client.read_exact(&mut opening).unwrap();
    let seed = |at: usize| u32::from_le_bytes(opening[6 + at..10 + at].try_into().unwrap());
    let seeds = (seed(9), seed(13));
    let wrong = Sequence::from(seeds.0).next() ^ 0xFF;
    let mut answer = vec![12, 0, 0x00, 0x50, wrong, 0];
    answer.extend([0u8; 12]);
    client.write_all(&answer).unwrap();
    let address = client.local_addr().ok();
    until_closed(client);

    let peer = Peer {
        listener: "agent",
        address,
        seeds,
    };
    let ended = Notice::Ended {
        peer: peer.clone(),
        established: false,
        why: Why::Broken(Broken::SecurityBytes),
    };
    assert_eq!(*notices.lock().unwrap(), [Notice::Opened(peer), ended]);
}
