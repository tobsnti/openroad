//! The binary as the test bench starts it: environment in, events and admin
//! answers out, a client walking from the gateway into the game in between.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use bytes::Bytes;
use packets::agent::prelude::{AgentLoginRequest, CharacterJoinRequest};
use packets::login::{LoginRequest, LoginResponse};
use packets::Packet;
use serde_json::{json, Value};

mod common;
use common::Wire;

/// The server process; killed when the test ends, also on a failure.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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

fn admin(socket: &Path, command: &str) -> Value {
    let mut stream = UnixStream::connect(socket).expect("the admin socket accepts");
    writeln!(stream, "{command}").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).expect("one JSON line")
}

#[test]
fn the_bench_sees_a_character_enter_and_its_rejected_frames() {
    let dir: PathBuf = std::env::temp_dir().join(format!("or-bench-{}", std::process::id()));
    let textdata = dir.join("textdata");
    std::fs::create_dir_all(&textdata).unwrap();
    let mut row = vec![String::from("0"); 48];
    (row[1], row[46], row[47]) = ("1907".into(), "16".into(), "50".into());
    std::fs::write(textdata.join("characterdata_test.txt"), row.join("\t")).unwrap();
    let character = |name: &str| {
        json!({"name": name, "ref": 1907, "level": 10, "exp": 0, "gold": 100, "hp": 1000,
            "mp": 1000, "str": 20, "int": 20, "statuspunkte": 0})
    };
    let fixture = json!({
        "konto": {"name": "test", "passwort": "test"},
        "charakter": character("Gilgamesh"),
        "konto_b": {"name": "test2", "passwort": "test2"},
        "charakter_b": character("Enkidu"),
    });
    std::fs::write(dir.join("startwelt.json"), fixture.to_string()).unwrap();
    let (events, socket) = (dir.join("events.jsonl"), dir.join("admin.sock"));
    let _server = Server(
        Command::new(env!("CARGO_BIN_EXE_openroad-server"))
            .env("BENCH_FIXTURE", dir.join("startwelt.json"))
            .env("BENCH_START_POS", "25000 969 0 1419")
            .env("BENCH_EVENTS", &events)
            .env("BENCH_ADMIN", &socket)
            .env("OPENROAD_TEXTDATA_DIR", &textdata)
            .env("BENCH_GATEWAY", "127.0.0.1:0")
            .env("BENCH_AGENT", "127.0.0.1:0")
            .spawn()
            .expect("the binary starts"),
    );
    let ready = event(&events, "ready");
    let address = |key: &str| ready[key].as_str().unwrap().parse().unwrap();

    let agent = token(address("gateway"), "test");

    let mut client = enter_agent(address("agent"), agent, "test");
    let join = CharacterJoinRequest {
        character_name: "Gilgamesh".to_owned(),
    };
    client.send(0x7001, &bytes(Packet::CharacterJoinRequest(join)), false);
    let entry: Vec<u16> = (0..5).map(|_| client.recv().0).collect();
    assert_eq!(entry, [0xB001, 0x34A5, 0x3013, 0x34A6, 0x3020]);
    client.send(0x3012, &[], false);

    let joined = event(&events, "player.joined");
    assert_eq!(joined["name"], "Gilgamesh");
    let players = admin(&socket, "players");
    assert_eq!(players["ok"], true);
    let player = &players["players"][0];
    assert_eq!(player["uid"], joined["uid"]);
    let at = (&player["region"], &player["x"], &player["z"]);
    assert_eq!(at, (&json!(25000), &json!(969.0), &json!(1419.0)));
    assert_eq!(admin(&socket, "loot")["ok"], false);

    // Movement comes with S6.3: until then the frame is reported, not answered.
    let walk = [0x01, 0xA8, 0x61, 0xC9, 0x03, 0x00, 0x00, 0x8B, 0x05];
    client.send(0x7021, &walk, false);
    let rejected = event(&events, "c2s.rejected");
    let expected = json!({"uid": joined["uid"], "op": "0x7021", "len": 9, "reason": "not_handled"});
    let fields = ["uid", "op", "len", "reason"].map(|k| (k, rejected[k].clone()));
    assert_eq!(Value::from_iter(fields), expected);

    // The second account, for the bench's second client, has its own character.
    let token_b = token(address("gateway"), "test2");
    let mut client_b = enter_agent(address("agent"), token_b, "test2");
    client_b.send(0x7007, &[0x02], false);
    let (opcode, body) = client_b.recv();
    let Ok(Packet::CharacterSelectionActionResponse(list)) = Packet::deserialize(opcode, body)
    else {
        panic!("0xB007 decodes");
    };
    let names: Vec<String> = list
        .characters
        .unwrap()
        .characters
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["Enkidu"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Logs `user` in at the gateway (password = user name, as in the fixture
/// above); the token for the agent.
fn token(gateway: std::net::SocketAddr, user: &str) -> u32 {
    let mut gateway = Wire::connect(gateway);
    let login = LoginRequest {
        content_id: 22,
        username: user.to_owned(),
        password: user.to_owned(),
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

fn enter_agent(agent: std::net::SocketAddr, token: u32, user: &str) -> Wire {
    let mut client = Wire::connect(agent);
    let login = AgentLoginRequest {
        token,
        username: user.to_owned(),
        password: user.to_owned(),
        content_id: 22,
        mac_address: [0; 6],
    };
    client.send(0x6103, &bytes(Packet::AgentLoginRequest(login)), true);
    assert_eq!(client.recv(), (0xA103, Bytes::from_static(&[0x01])));
    client
}
