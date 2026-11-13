//! What idle connections cost the server, measured rather than assumed (the
//! studio lead's condition with the decision for std threads). Starts the
//! server in this process, opens N agent connections that complete the
//! handshake and then wait, and prints the process's resident memory and thread
//! count before and after, and how long accepting them took. Each connection
//! needs about three file descriptors (client end, server end, the writer's
//! copy); thread counts come from macOS `ps -M`:
//!
//! `ulimit -n 8192; cargo run --release -p server --example idle_connections -- 1000`

use std::net::{Ipv4Addr, SocketAddr};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use server::gateway::Gateway;
use server::refdata::RefData;
use server::runtime::{start, Config, GATEWAY_IDLE, HANDSHAKE};
use server::store::{MemoryStore, Store};
use server::world::World;

#[path = "../tests/common/mod.rs"]
mod common;
use common::Wire;

fn main() {
    let n: u16 = std::env::args()
        .nth(1)
        .map_or(1000, |n| n.parse().expect("a count"));
    let store: Arc<dyn Store + Send + Sync> = Arc::new(MemoryStore::default());
    let loopback = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    let config = Config {
        gateway: Gateway {
            farm_id: 20,
            farm_name: "OpenRoad".to_owned(),
            farm_ip: Ipv4Addr::LOCALHOST,
            shard_id: 64,
            shard_name: "Measure".to_owned(),
            capacity: n,
            agent_ip: "127.0.0.1".to_owned(),
            agent_port: 0,
        },
        gateway_listen: loopback,
        agent_listen: loopback,
        handshake: HANDSHAKE,
        gateway_idle: GATEWAY_IDLE,
    };
    let world = World::new(store.clone(), RefData::default());
    let server = start(config, world, store, |_, _| {}).expect("the server starts");
    thread::sleep(Duration::from_millis(200));
    let (rss_before, threads_before) = sample();

    let started = Instant::now();
    let wires: Vec<Wire> = (0..n).map(|_| Wire::connect(server.agent)).collect();
    let accepted = started.elapsed();
    thread::sleep(Duration::from_millis(500));
    let (rss_after, threads_after) = sample();

    println!("{n} connections, each through the handshake, then idle");
    println!(
        "accepted and handshaken in {accepted:?} ({:?} per connection)",
        accepted / u32::from(n)
    );
    println!("resident memory: {rss_before} KiB before, {rss_after} KiB after");
    println!("threads: {threads_before} before, {threads_after} after");
    println!(
        "per connection: {:.1} KiB, {:.2} threads",
        (rss_after - rss_before) as f64 / f64::from(n),
        (threads_after - threads_before) as f64 / f64::from(n)
    );
    drop(wires);
}

/// Resident memory in KiB and thread count of this process.
fn sample() -> (u64, usize) {
    let pid = std::process::id().to_string();
    let ps = |args: &[&str]| {
        let out = Command::new("ps")
            .args(args)
            .arg(&pid)
            .output()
            .expect("ps runs");
        String::from_utf8(out.stdout).expect("ps writes text")
    };
    let rss = ps(&["-o", "rss=", "-p"])
        .trim()
        .parse()
        .expect("ps gives the resident size");
    let threads = ps(&["-M", "-p"]).lines().count() - 1;
    (rss, threads)
}
