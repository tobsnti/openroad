//! The server as the test bench starts it (studio bench contract v1): no
//! arguments; the start world, the event file and the admin socket come from
//! the environment, and nothing outlives the process.
//!
//! - `BENCH_FIXTURE`: the start world. Each `konto<suffix>` (`konto`,
//!   `konto_b`) becomes an account with `charakter<suffix>` as its character.
//! - `BENCH_START_POS`: `<region> <x> <y> <z>`, where the character stands.
//! - `BENCH_EVENTS`: a file the events go to, one JSON line each: `ready`,
//!   `player.joined`, `c2s.rejected`.
//! - `BENCH_ADMIN`: a Unix socket; one command line in, one JSON line out.
//! - `OPENROAD_TEXTDATA_DIR`: the user's textdata, for the reference data.
//! - `BENCH_GATEWAY`, `BENCH_AGENT`: where to listen; the bench's ports
//!   127.0.0.1:4011 and 4012 unless set.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use server::gateway::Gateway;
use server::refdata::RefData;
use server::runtime::{self, Config, Running, GATEWAY_IDLE, HANDSHAKE};
use server::store::{Character, MemoryStore, Position, Store};
use server::world::{Event, Phase, Reason, World};

fn main() {
    eprintln!("server build: commit {}", env!("OPENROAD_BUILD_COMMIT"));
    let fixture: Value =
        serde_json::from_str(&read(&var("BENCH_FIXTURE"))).expect("BENCH_FIXTURE is JSON");
    let start = start_position(&var("BENCH_START_POS"));
    let mut store = MemoryStore::default();
    // Every `konto<suffix>` with its `charakter<suffix>`: `konto` for the
    // first client, `konto_b` for the second.
    let accounts = fixture.as_object().expect("the fixture is an object");
    for (key, account) in accounts.iter().filter(|(key, _)| key.starts_with("konto")) {
        let suffix = &key["konto".len()..];
        let character = character(&fixture[format!("charakter{suffix}")], start);
        store.add_account(
            text(&account["name"]),
            text(&account["passwort"]),
            vec![character],
        );
    }
    let store: Arc<dyn Store + Send + Sync> = Arc::new(store);
    let refdata = RefData::load(Path::new(&var("OPENROAD_TEXTDATA_DIR")))
        .expect("OPENROAD_TEXTDATA_DIR is readable");
    let events = Events(Arc::new(Mutex::new(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(var("BENCH_EVENTS"))
            .expect("BENCH_EVENTS can be written"),
    )));

    let agent_listen = listen_on("BENCH_AGENT", 4012);
    let config = Config {
        gateway: gateway(agent_listen),
        gateway_listen: listen_on("BENCH_GATEWAY", 4011),
        agent_listen,
        handshake: HANDSHAKE,
        gateway_idle: GATEWAY_IDLE,
    };
    let world = World::new(store.clone(), refdata);
    let reporter = events.clone();
    let running = runtime::start(config, world, store, move |world, event| {
        reporter.report(world, event)
    })
    .expect("the listeners bind");
    eprintln!("gateway on {}, agent on {}", running.gateway, running.agent);

    let admin = var("BENCH_ADMIN");
    // A socket file an earlier run left behind would make the bind fail.
    let _ = fs::remove_file(&admin);
    let admin = UnixListener::bind(&admin).expect("BENCH_ADMIN can be bound");
    let addresses =
        json!({"gateway": running.gateway.to_string(), "agent": running.agent.to_string()});
    events.write("ready", addresses);
    for stream in admin.incoming().flatten() {
        let mut command = String::new();
        if BufReader::new(&stream).read_line(&mut command).is_ok() {
            let _ = writeln!(&stream, "{}", answer(command.trim(), &running));
        }
    }
}

/// The admin commands of the bench contract that this server can answer yet.
fn answer(command: &str, running: &Running) -> Value {
    match command.split_whitespace().next() {
        Some("players") => {
            let players = running.query(|world| {
                let row = |(uid, p): (u32, &server::world::Player)| {
                    let at = p.position;
                    json!({"uid": uid, "name": p.name, "region": at.region, "x": at.x,
                        "y": at.y, "z": at.z, "hp": p.hp, "mp": p.mp})
                };
                world.players().map(row).collect::<Vec<_>>()
            });
            json!({"ok": true, "players": players.unwrap_or_default()})
        }
        Some("loot" | "grant-exp") => json!({"ok": false, "error": "not yet"}),
        _ => json!({"ok": false, "error": "unknown command"}),
    }
}

#[derive(Clone)]
struct Events(Arc<Mutex<File>>);

impl Events {
    fn write(&self, ev: &str, mut fields: Value) {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        fields["t_ms"] = json!(since_epoch.as_millis() as u64);
        fields["ev"] = json!(ev);
        // One write per line, so a reader never sees half of one.
        let line = format!("{fields}\n");
        let mut file = self.0.lock().expect("no writer panicked");
        file.write_all(line.as_bytes())
            .expect("BENCH_EVENTS can be written");
    }

    fn report(&self, world: &World, event: &Event) {
        match *event {
            Event::Joined { uid, .. } => {
                let name = world.player(uid).map(|p| p.name.as_str());
                self.write("player.joined", json!({"uid": uid, "name": name}));
            }
            Event::Rejected {
                session,
                opcode,
                len,
                reason,
            } => {
                let uid = match world.phase(session) {
                    Some(Phase::Joining { uid, .. } | Phase::Game { uid, .. }) => Some(uid),
                    _ => None,
                };
                let reason = match reason {
                    Reason::UnknownOpcode => "unknown_opcode",
                    Reason::DecodeError => "decode_error",
                    Reason::NotHandled => "not_handled",
                };
                let op = format!("0x{opcode:04X}");
                self.write(
                    "c2s.rejected",
                    json!({"uid": uid, "op": op, "len": len, "reason": reason}),
                );
            }
            _ => {}
        }
    }
}

/// The captured vSRO gateway's farm and shard ids and capacity (0xA101: farm
/// 20, shard 64, capacity 1000 [V capture, peer vSRO]) under our names; the
/// farm address to ping is the loopback, which that gateway sends as well.
fn gateway(agent: SocketAddr) -> Gateway {
    Gateway {
        farm_id: 20,
        farm_name: "OpenRoad".to_owned(),
        farm_ip: Ipv4Addr::LOCALHOST,
        shard_id: 64,
        shard_name: "OpenRoad".to_owned(),
        capacity: 1000,
        agent_ip: agent.ip().to_string(),
        agent_port: agent.port(),
    }
}

/// The fixture's character. What the fixture does not name is set to what
/// the captured vSRO records carry [V capture, peer vSRO: 243 records]:
/// height and build 0x22 for this model, 109 inventory slots, the seven
/// Chinese masteries at level 0. Its bag (`tasche`) waits for the inventory.
fn character(c: &Value, position: Position) -> Character {
    Character {
        name: text(&c["name"]).to_owned(),
        ref_id: number(c, "ref"),
        scale: 0x22,
        level: number(c, "level"),
        exp: number(c, "exp"),
        skill_exp: 0,
        gold: number(c, "gold"),
        skill_points: 0,
        strength: number(c, "str"),
        intellect: number(c, "int"),
        stat_points: number(c, "statuspunkte"),
        hp: number(c, "hp"),
        mp: number(c, "mp"),
        inventory_size: 109,
        masteries: [257, 258, 259, 273, 274, 275, 276]
            .map(|id| (id, 0))
            .to_vec(),
        position,
    }
}

fn start_position(text: &str) -> Position {
    let parts: Vec<&str> = text.split_whitespace().collect();
    let [region, x, y, z] = parts[..] else {
        panic!("BENCH_START_POS is `<region> <x> <y> <z>`");
    };
    let coordinate = |v: &str| v.parse().expect("BENCH_START_POS holds numbers");
    Position {
        region: region.parse().expect("BENCH_START_POS names a region"),
        x: coordinate(x),
        y: coordinate(y),
        z: coordinate(z),
        heading: 0,
    }
}

fn number<T: TryFrom<u64>>(c: &Value, key: &str) -> T {
    let value = c[key]
        .as_u64()
        .unwrap_or_else(|| panic!("charakter.{key} is a number"));
    T::try_from(value).unwrap_or_else(|_| panic!("charakter.{key} is out of range"))
}

fn text(value: &Value) -> &str {
    value.as_str().expect("the fixture names it as text")
}

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is set"))
}

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|_| panic!("{path} is readable"))
}

fn listen_on(name: &str, port: u16) -> SocketAddr {
    std::env::var(name).map_or(SocketAddr::from((Ipv4Addr::LOCALHOST, port)), |a| {
        a.parse().unwrap_or_else(|_| panic!("{name} is an address"))
    })
}
