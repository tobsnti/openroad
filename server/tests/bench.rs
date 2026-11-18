//! The binary as the test bench starts it: environment in, events and admin
//! answers out, a client walking from the gateway into the game in between.

use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use bytes::Bytes;
use packets::agent::prelude::{AgentLoginRequest, CharacterJoinRequest, LobbyCharacter, LobbyItem};
use packets::login::{LoginRequest, LoginResponse};
use packets::Packet;
use serde_json::{json, Value};
use server::refdata::RefData;

mod common;
use common::Wire;

/// The server process with its files; killed when the test ends, also on a
/// failure.
struct Server {
    child: Child,
    dir: PathBuf,
    events: PathBuf,
    admin: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Starts the binary with `fixture`, a textdata folder holding the model 1907
/// (or `textdata`, the user's), and ephemeral ports.
fn start(name: &str, fixture: &Path, textdata: Option<&Path>) -> Server {
    let dir = std::env::temp_dir().join(format!("or-bench-{}-{name}", std::process::id()));
    let own = dir.join("textdata");
    std::fs::create_dir_all(&own).unwrap();
    let mut row = vec![String::from("0"); 48];
    // A player's model (type ids 1/1) with its walk and run speeds.
    (row[1], row[9], row[10], row[46], row[47]) = (
        "1907".into(),
        "1".into(),
        "1".into(),
        "16".into(),
        "50".into(),
    );
    std::fs::write(own.join("characterdata_test.txt"), row.join("\t")).unwrap();
    let (events, admin) = (dir.join("events.jsonl"), dir.join("admin.sock"));
    let child = Command::new(env!("CARGO_BIN_EXE_openroad-server"))
        .env("BENCH_FIXTURE", fixture)
        .env("BENCH_START_POS", "25000 969 0 1419")
        .env("BENCH_EVENTS", &events)
        .env("BENCH_ADMIN", &admin)
        .env("OPENROAD_TEXTDATA_DIR", textdata.unwrap_or(&own))
        .env("BENCH_GATEWAY", "127.0.0.1:0")
        .env("BENCH_AGENT", "127.0.0.1:0")
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary starts");
    Server {
        child,
        dir,
        events,
        admin,
    }
}

/// `fixture` as a file of its own in a fresh folder.
fn fixture_file(name: &str, fixture: &Value) -> PathBuf {
    let path = std::env::temp_dir().join(format!("or-fixture-{}-{name}.json", std::process::id()));
    std::fs::write(&path, fixture.to_string()).unwrap();
    path
}

/// A fixture of the bench's shape: `charakter_b` names only what differs from
/// `charakter`.
fn bench_fixture() -> Value {
    json!({
        "konto": {"name": "test", "passwort": "test"},
        "konto_b": {"name": "test2", "passwort": "test2"},
        "charakter": {"name": "Gilgamesh", "ref": 1907, "level": 10, "exp": 0, "gold": 100,
            "hp": 1000, "mp": 1000, "str": 20, "int": 20, "statuspunkte": 0},
        "charakter_b": {"name": "Enkidu"},
    })
}

fn bytes(packet: Packet) -> Bytes {
    packet.into_serialize().1
}

/// The first event `ev` the server wrote, waiting for it up to 10 s.
fn event(file: &Path, ev: &str) -> Value {
    let started = Instant::now();
    loop {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        // Complete lines only: the server may be writing the next one.
        let complete = &text[..text.rfind('\n').map_or(0, |end| end + 1)];
        let found = complete
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("a JSON line"))
            .find(|line| line["ev"] == ev);
        if let Some(found) = found {
            return found;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "no {ev} event");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn address(ready: &Value, key: &str) -> SocketAddr {
    ready[key].as_str().unwrap().parse().unwrap()
}

fn admin(socket: &Path, command: &str) -> Value {
    let mut stream = UnixStream::connect(socket).expect("the admin socket accepts");
    writeln!(stream, "{command}").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).expect("one JSON line")
}

/// Logs `user` in at the gateway; the token for the agent.
fn token(gateway: SocketAddr, user: &str, password: &str) -> u32 {
    let mut gateway = Wire::connect(gateway);
    let login = LoginRequest {
        content_id: 22,
        username: user.to_owned(),
        password: password.to_owned(),
        shard_id: 64,
    };
    gateway.send(0x6102, &bytes(Packet::LoginRequest(login)), true);
    let (opcode, body) = gateway.recv();
    let Ok(Packet::LoginResponse(LoginResponse {
        login_info: Some(agent),
        ..
    })) = Packet::deserialize(opcode, body)
    else {
        panic!("the fixture's account {user} logs in");
    };
    agent.agent_token
}

fn enter_agent(agent: SocketAddr, token: u32, user: &str) -> Wire {
    let mut client = Wire::connect(agent);
    let login = AgentLoginRequest {
        token,
        username: user.to_owned(),
        password: String::new(),
        content_id: 22,
        mac_address: [0; 6],
    };
    client.send(0x6103, &bytes(Packet::AgentLoginRequest(login)), true);
    assert_eq!(client.recv(), (0xA103, Bytes::from_static(&[0x01])));
    client
}

/// The characters the agent lists for a logged-in client.
fn lobby(client: &mut Wire) -> Vec<LobbyCharacter> {
    client.send(0x7007, &[0x02], false);
    let (opcode, body) = client.recv();
    let Ok(Packet::CharacterSelectionActionResponse(list)) = Packet::deserialize(opcode, body)
    else {
        panic!("0xB007 decodes");
    };
    list.characters.expect("a list").characters
}

/// The names of the characters the agent lists for a logged-in client.
fn listed(client: &mut Wire) -> Vec<String> {
    lobby(client).into_iter().map(|c| c.name).collect()
}

#[test]
fn the_bench_sees_a_character_enter_and_its_rejected_frames() {
    let fixture = fixture_file("walk", &bench_fixture());
    let server = start("walk", &fixture, None);
    let ready = event(&server.events, "ready");

    let token_a = token(address(&ready, "gateway"), "test", "test");
    let mut client = enter_agent(address(&ready, "agent"), token_a, "test");
    let join = CharacterJoinRequest {
        character_name: "Gilgamesh".to_owned(),
    };
    client.send(0x7001, &bytes(Packet::CharacterJoinRequest(join)), false);
    let entry: Vec<u16> = (0..5).map(|_| client.recv().0).collect();
    assert_eq!(entry, [0xB001, 0x34A5, 0x3013, 0x34A6, 0x3020]);
    client.send(0x3012, &[], false);

    let joined = event(&server.events, "player.joined");
    assert_eq!(joined["name"], "Gilgamesh");
    let players = admin(&server.admin, "players");
    assert_eq!(players["ok"], true);
    let player = &players["players"][0];
    assert_eq!(player["uid"], joined["uid"]);
    let at = (&player["region"], &player["x"], &player["z"]);
    assert_eq!(at, (&json!(25000), &json!(969.0), &json!(1419.0)));
    assert_eq!(admin(&server.admin, "loot")["ok"], false);

    // Selecting an entity comes with the NPCs: until then the frame is
    // reported, not answered.
    client.send(0x7045, &[0x01, 0x10, 0x00, 0x00], false);
    let rejected = event(&server.events, "c2s.rejected");
    let expected = json!({"uid": joined["uid"], "op": "0x7045", "len": 4, "reason": "not_handled"});
    let fields = ["uid", "op", "len", "reason"].map(|k| (k, rejected[k].clone()));
    assert_eq!(Value::from_iter(fields), expected);

    // The second account has its own character, the rest laid over from
    // `charakter`.
    let token_b = token(address(&ready, "gateway"), "test2", "test2");
    let mut client_b = enter_agent(address(&ready, "agent"), token_b, "test2");
    assert_eq!(listed(&mut client_b), ["Enkidu"]);
    let _ = std::fs::remove_file(fixture);
}

/// What the server says when `wrong` ends it: exit 2 and a message, not a
/// panic. Waits for the end up to 10 s, like [`event`]: a server that takes
/// the fixture runs on.
fn ended_by(name: &str, wrong: &Value) -> String {
    let fixture = fixture_file(name, wrong);
    let mut server = start(name, &fixture, None);
    let started = Instant::now();
    let status = loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the server took the fixture"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stderr = String::new();
    let mut pipe = server.child.stderr.take().unwrap();
    std::io::Read::read_to_string(&mut pipe, &mut stderr).unwrap();
    let _ = std::fs::remove_file(fixture);
    assert_eq!(status.code(), Some(2), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
    stderr
}

/// A value the server cannot use ends it with a message that names the entry
/// it came from, not with a panic.
#[test]
fn a_wrong_fixture_value_ends_the_server_with_its_key() {
    let mut wrong = bench_fixture();
    wrong["charakter_b"]["ref"] = json!("x");
    let stderr = ended_by("wrong", &wrong);
    assert!(stderr.contains("charakter_b.ref"), "{stderr}");
}

/// A worn item the user's itemdata does not hold ends the server the same
/// way, naming the entry and the code name.
#[test]
fn an_unknown_worn_item_ends_the_server_with_its_key() {
    let mut wrong = bench_fixture();
    wrong["charakter"]["ausruestung"] = json!([{"platz": 6, "codename": "ITEM_NOWHERE"}]);
    let stderr = ended_by("unknown-item", &wrong);
    let key = "charakter.ausruestung[0].codename ITEM_NOWHERE";
    assert!(stderr.contains(key), "{stderr}");
}

/// The bench's own fixture: both of its accounts log in and list their
/// characters, wearing the items the fixture names, in slot order, as the
/// user's itemdata numbers them.
#[test]
#[ignore = "needs the bench's files: OPENROAD_BENCH_FIXTURE=<startwelt.json> \
            OPENROAD_TEXTDATA_DIR=<textdata> cargo test -p server --test bench -- --ignored"]
fn the_benchs_own_fixture_starts_the_server() {
    let fixture = PathBuf::from(std::env::var("OPENROAD_BENCH_FIXTURE").unwrap());
    let textdata = PathBuf::from(std::env::var("OPENROAD_TEXTDATA_DIR").unwrap());
    let entries: Value = serde_json::from_str(&std::fs::read_to_string(&fixture).unwrap()).unwrap();
    let refdata = RefData::load(&textdata).unwrap();
    let server = start("own", &fixture, Some(&textdata));
    let ready = event(&server.events, "ready");
    for (konto, charakter) in [("konto", "charakter"), ("konto_b", "charakter_b")] {
        let user = entries[konto]["name"].as_str().unwrap();
        let password = entries[konto]["passwort"]
            .as_str()
            .or(entries["konto"]["passwort"].as_str())
            .unwrap();
        let agent = token(address(&ready, "gateway"), user, password);
        let mut client = enter_agent(address(&ready, "agent"), agent, user);
        // A list the suffixed entry names replaces the unsuffixed one's.
        let named = |field: &str| -> Vec<u32> {
            let list = entries[charakter].get(field);
            let list = list.unwrap_or(&entries["charakter"][field]);
            let mut worn: Vec<(u64, u32)> = Vec::new();
            for w in list.as_array().unwrap() {
                let id = refdata.item_named(w["codename"].as_str().unwrap());
                worn.push((w["platz"].as_u64().unwrap(), id.unwrap()));
            }
            worn.sort_unstable();
            worn.into_iter().map(|(_, id)| id).collect()
        };
        let ids = |items: &[LobbyItem]| -> Vec<u32> { items.iter().map(|i| i.id).collect() };
        let shown: Vec<_> = (lobby(&mut client).iter())
            .map(|c| (c.name.clone(), ids(&c.char_items), ids(&c.avatar_items)))
            .collect();
        let name = entries[charakter]["name"].as_str().unwrap().to_owned();
        assert_eq!(shown, [(name, named("ausruestung"), named("avatar"))]);
    }
}
