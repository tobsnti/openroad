//! The server as the test bench starts it (studio bench contract v1): no
//! arguments; the start world, the event file and the admin socket come from
//! the environment, and nothing outlives the process. A setting it cannot use
//! ends it with a message naming the setting, not with a panic.
//!
//! - `BENCH_FIXTURE`: the start world. Each `konto<suffix>` (`konto`,
//!   `konto_b`) becomes an account with `charakter<suffix>` as its character.
//!   As the fixture's own rule says, a suffixed entry names only what differs:
//!   it is laid over its unsuffixed one.
//! - `BENCH_START_POS`: `<region> <x> <y> <z>`, where the character stands.
//! - `BENCH_EVENTS`: a file the events go to, one JSON line each: `ready`,
//!   `player.joined`, `c2s.rejected`, `session.opened` (a handshake's seeds)
//!   and `session.ended` (why the server ended a connection, or the client
//!   left before its handshake was done; also a line on stderr).
//! - `BENCH_ADMIN`: a Unix socket; one command line in, one JSON line out.
//! - `OPENROAD_TEXTDATA_DIR`: the user's textdata, for the reference data.
//! - `BENCH_GATEWAY`, `BENCH_AGENT`: where to listen; the bench's ports
//!   127.0.0.1:4011 and 4012 unless set.
//! - `BENCH_AGENT_ADVERTISE`: the agent address the gateway tells clients
//!   (0xA102), for a client that reaches the agent through a relay;
//!   `BENCH_AGENT` unless set.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};
use server::gateway::Gateway;
use server::refdata::RefData;
use server::runtime::{self, Config, Notice, Peer, Running, Why, GATEWAY_IDLE, HANDSHAKE};
use server::store::{Character, MemoryStore, Position, Store};
use server::transport::Broken;
use server::world::{Event, Phase, Reason, World};

fn main() -> ExitCode {
    eprintln!("server build: commit {}", env!("OPENROAD_BUILD_COMMIT"));
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("openroad-server: {why}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let path = var("BENCH_FIXTURE")?;
    let fixture = fs::read_to_string(&path).map_err(|e| format!("BENCH_FIXTURE {path}: {e}"))?;
    let fixture: Value =
        serde_json::from_str(&fixture).map_err(|e| format!("BENCH_FIXTURE {path}: {e}"))?;
    let store = accounts(&fixture, start_position(&var("BENCH_START_POS")?)?)?;
    let store: Arc<dyn Store + Send + Sync> = Arc::new(store);
    let textdata = var("OPENROAD_TEXTDATA_DIR")?;
    let refdata = RefData::load(Path::new(&textdata))
        .map_err(|e| format!("OPENROAD_TEXTDATA_DIR {textdata}: {e}"))?;
    let events = var("BENCH_EVENTS")?;
    let events = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&events)
        .map_err(|e| format!("BENCH_EVENTS {events}: {e}"))?;
    let events = Events(Arc::new(Mutex::new(events)));

    let agent_listen = listen_on("BENCH_AGENT", 4012)?;
    let config = Config {
        gateway: gateway(address_or("BENCH_AGENT_ADVERTISE", agent_listen)?),
        gateway_listen: listen_on("BENCH_GATEWAY", 4011)?,
        agent_listen,
        handshake: HANDSHAKE,
        gateway_idle: GATEWAY_IDLE,
    };
    let world = World::new(store.clone(), refdata);
    let (placed, skipped) = (world.npcs().count(), world.skipped_npc_rows());
    eprintln!("npcpos.txt: {placed} NPCs placed, {skipped} rows skipped (no NPC model in the client's data)");
    let reporter = events.clone();
    let notices = events.clone();
    let running = runtime::start(
        config,
        world,
        store,
        move |world, event| reporter.report(world, event),
        move |notice| notices.notice(&notice),
    )
    .map_err(|e| format!("the listeners: {e}"))?;
    eprintln!("gateway on {}, agent on {}", running.gateway, running.agent);

    let admin = var("BENCH_ADMIN")?;
    // A socket file an earlier run left behind would make the bind fail.
    let _ = fs::remove_file(&admin);
    let admin = UnixListener::bind(&admin).map_err(|e| format!("BENCH_ADMIN {admin}: {e}"))?;
    let addresses =
        json!({"gateway": running.gateway.to_string(), "agent": running.agent.to_string()});
    events.write("ready", addresses);
    for stream in admin.incoming().flatten() {
        let mut command = String::new();
        if BufReader::new(&stream).read_line(&mut command).is_ok() {
            let _ = writeln!(&stream, "{}", answer(command.trim(), &running));
        }
    }
    Ok(())
}

/// Every `konto<suffix>` of the fixture with its `charakter<suffix>`.
fn accounts(fixture: &Value, start: Position) -> Result<MemoryStore, String> {
    let entries = fixture
        .as_object()
        .ok_or("BENCH_FIXTURE is not a JSON object")?;
    let mut store = MemoryStore::default();
    for key in entries.keys().filter(|key| key.starts_with("konto")) {
        let suffix = &key["konto".len()..];
        let account = laid_over(entries, "konto", key)?;
        let name = format!("charakter{suffix}");
        let character = character(&laid_over(entries, "charakter", &name)?, &name, start)?;
        let user = text(&account, key, "name")?;
        store.add_account(user, text(&account, key, "passwort")?, vec![character]);
    }
    Ok(store)
}

/// The entry `key` laid over the entry `base`, field by field.
fn laid_over(entries: &Map<String, Value>, base: &str, key: &str) -> Result<Value, String> {
    let object = |key: &str| {
        let entry = entries
            .get(key)
            .ok_or(format!("BENCH_FIXTURE has no {key}"))?;
        entry
            .as_object()
            .cloned()
            .ok_or(format!("BENCH_FIXTURE {key} is not an object"))
    };
    let mut merged = object(base)?;
    merged.extend(object(key)?);
    Ok(Value::Object(merged))
}

/// The admin commands of the bench contract that this server can answer yet.
fn answer(command: &str, running: &Running) -> Value {
    match command.split_whitespace().next() {
        Some("players") => {
            let players = running.query(|world| {
                let row = |(uid, p): (u32, &server::world::Player)| {
                    let c = &p.character;
                    let at = c.position;
                    json!({"uid": uid, "name": c.name, "region": at.region, "x": at.x,
                        "y": at.y, "z": at.z, "hp": c.hp, "mp": c.mp})
                };
                world.players().map(row).collect::<Vec<_>>()
            });
            json!({"ok": true, "players": players.unwrap_or_default()})
        }
        Some("npcs") => {
            let npcs = running.query(|world| {
                let row = |(uid, n): (u32, &server::npc::Npc)| {
                    let at = n.position;
                    json!({"uid": uid, "ref": n.ref_id, "region": at.region, "x": at.x,
                        "y": at.y, "z": at.z})
                };
                world.npcs().map(row).collect::<Vec<_>>()
            });
            json!({"ok": true, "npcs": npcs.unwrap_or_default()})
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
                let name = world.player(uid).map(|p| p.character.name.as_str());
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

    fn notice(&self, notice: &Notice) {
        let fields = |peer: &Peer| {
            let address = peer.address.map(|a| a.to_string());
            json!({"conn": peer.listener, "peer": address,
                   "count_seed": peer.seeds.0, "crc_seed": peer.seeds.1})
        };
        match notice {
            Notice::Opened(peer) => self.write("session.opened", fields(peer)),
            Notice::Ended {
                peer,
                established,
                why,
            } => {
                let (count, crc) = peer.seeds;
                let address = peer.address.map(|a| a.to_string()).unwrap_or_default();
                let phase = if *established { "session" } else { "handshake" };
                eprintln!(
                    "{} connection {address} ended in the {phase}: {why:?} (seeds {count}/{crc})",
                    peer.listener
                );
                let mut fields = fields(peer);
                fields["established"] = json!(established);
                fields["reason"] = json!(reason(why));
                self.write("session.ended", fields);
            }
        }
    }
}

/// The `reason` of a `session.ended`.
fn reason(why: &Why) -> &'static str {
    match why {
        Why::Closed => "closed",
        Why::Deadline => "deadline",
        Why::Socket(_) => "socket",
        Why::Broken(Broken::SecurityBytes) => "security",
        Why::Broken(Broken::Frame) => "frame",
        Why::Broken(Broken::Handshake(_)) => "handshake",
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

/// The fixture's character `key`. What the fixture does not name is set to
/// what the captured vSRO records carry [V capture, peer vSRO: 243 records]:
/// height and build 0x22 for this model, 109 inventory slots, the seven
/// Chinese masteries at level 0. Its bag (`tasche`) waits for the inventory.
fn character(c: &Value, key: &str, position: Position) -> Result<Character, String> {
    Ok(Character {
        name: text(c, key, "name")?.to_owned(),
        ref_id: number(c, key, "ref")?,
        scale: 0x22,
        level: number(c, key, "level")?,
        exp: number(c, key, "exp")?,
        skill_exp: 0,
        gold: number(c, key, "gold")?,
        skill_points: 0,
        strength: number(c, key, "str")?,
        intellect: number(c, key, "int")?,
        stat_points: number(c, key, "statuspunkte")?,
        hp: number(c, key, "hp")?,
        mp: number(c, key, "mp")?,
        inventory_size: 109,
        masteries: [257, 258, 259, 273, 274, 275, 276]
            .map(|id| (id, 0))
            .to_vec(),
        position,
    })
}

fn start_position(text: &str) -> Result<Position, String> {
    let wrong = || format!("BENCH_START_POS is `<region> <x> <y> <z>`, not `{text}`");
    let parts: Vec<&str> = text.split_whitespace().collect();
    let [region, x, y, z] = parts[..] else {
        return Err(wrong());
    };
    let coordinate = |v: &str| v.parse().map_err(|_| wrong());
    Ok(Position {
        region: region.parse().map_err(|_| wrong())?,
        x: coordinate(x)?,
        y: coordinate(y)?,
        z: coordinate(z)?,
        heading: 0,
    })
}

fn number<T: TryFrom<u64>>(entry: &Value, key: &str, field: &str) -> Result<T, String> {
    let value = entry[field].as_u64();
    value
        .and_then(|value| T::try_from(value).ok())
        .ok_or(format!(
            "BENCH_FIXTURE {key}.{field} is not a number in range"
        ))
}

fn text<'a>(entry: &'a Value, key: &str, field: &str) -> Result<&'a str, String> {
    entry[field]
        .as_str()
        .ok_or(format!("BENCH_FIXTURE {key}.{field} is not text"))
}

fn var(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is not set"))
}

fn listen_on(name: &str, port: u16) -> Result<SocketAddr, String> {
    address_or(name, SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
}

/// The address in the variable `name`, or `default` when it is not set.
fn address_or(name: &str, default: SocketAddr) -> Result<SocketAddr, String> {
    match std::env::var(name) {
        Ok(address) => address
            .parse()
            .map_err(|_| format!("{name} is not an address: {address}")),
        Err(_) => Ok(default),
    }
}
