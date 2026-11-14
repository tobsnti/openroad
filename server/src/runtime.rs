//! The server on sockets.
//!
//! One simulation thread owns the world and steps it every 1/30 s; between
//! steps it applies what the connections hand it, and after them it gives each
//! connection the frames the world queued. An agent connection has a thread
//! that reads it and, once its handshake is done, one that writes it; a
//! gateway connection has one thread, because each gateway request is answered
//! at once. Std threads rather than an async runtime (studio decision 0009,
//! addendum): the world and the gateway are synchronous, and the transport is
//! sans-IO, so a later switch stays inside this module.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use bytes::Bytes;

use crate::gateway::{Gateway, GatewayConnection, Shard};
use crate::store::{self, AccountId, Changes, Character, Store};
use crate::tick::STEP;
use crate::transport::{frame, Broken, Transport};
use crate::world::{Event, SessionId, World};

/// The handshake is two round trips without user input: twice the 5 s our own
/// client allows a handshake frame in flight (`client/src/net/handshake.rs`).
pub const HANDSHAKE: Duration = Duration::from_secs(10);

/// On the title screen the original client asks the gateway for the patch
/// state and the shard list every 5 s [V capture, original client]; a gateway
/// connection silent for six of those periods is gone.
pub const GATEWAY_IDLE: Duration = Duration::from_secs(30);

pub struct Config {
    /// What the gateway tells clients. Its capacity also caps the connections
    /// each listener holds at once, since every one binds threads. It
    /// announces its own agent address; an agent port of 0 announces the port
    /// the agent listener got.
    pub gateway: Gateway,
    pub gateway_listen: SocketAddr,
    pub agent_listen: SocketAddr,
    pub handshake: Duration,
    pub gateway_idle: Duration,
}

/// The connection a [`Notice`] is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// `gateway` or `agent`.
    pub listener: &'static str,
    pub address: Option<SocketAddr>,
    /// The sequence and CRC seeds its handshake drew.
    pub seeds: (u32, u32),
}

/// What the connection threads report besides the world: the seeds of every
/// handshake, and why a connection ended when the server ended it or the
/// client left before the handshake was done. The seeds go with both, since
/// a client that stamps the security bytes differently from the original
/// fails for some seeds only (measured 2026-11-14 on a proxy that lacks the
/// original's substitute for a zero seed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// The opening 0x5000 is out.
    Opened(Peer),
    Ended {
        peer: Peer,
        /// Whether the handshake was done.
        established: bool,
        why: Why,
    },
}

/// Why a connection ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// The client closed before the handshake was done.
    Closed,
    /// No frame in time: the handshake's deadline, or the gateway's idle time.
    Deadline,
    /// The socket failed.
    Socket(io::ErrorKind),
    /// The client's bytes did not check.
    Broken(Broken),
}

type Notify = dyn Fn(Notice) + Send + Sync;

/// A started server: where it listens, and a way to ask its world.
pub struct Running {
    pub gateway: SocketAddr,
    pub agent: SocketAddr,
    commands: Sender<Command>,
}

impl Running {
    /// `ask`'s answer about the world, computed on the simulation thread
    /// between two commands; `None` once the simulation has stopped.
    pub fn query<T: Send + 'static>(
        &self,
        ask: impl FnOnce(&World) -> T + Send + 'static,
    ) -> Option<T> {
        let (reply, answer) = mpsc::channel();
        let ask = move |world: &World| {
            let _ = reply.send(ask(world));
        };
        self.commands.send(Command::Query(Box::new(ask))).ok()?;
        answer.recv().ok()
    }
}

/// Binds both listeners and starts the simulation thread. `report` hears
/// every world event, on that thread; `notice` hears the connections, on
/// theirs.
pub fn start(
    mut config: Config,
    world: World,
    store: Arc<dyn Store + Send + Sync>,
    report: impl FnMut(&World, &Event) + Send + 'static,
    notice: impl Fn(Notice) + Send + Sync + 'static,
) -> io::Result<Running> {
    let gateway_listener = TcpListener::bind(config.gateway_listen)?;
    let agent_listener = TcpListener::bind(config.agent_listen)?;
    let (commands, inbox) = mpsc::channel();
    let running = Running {
        gateway: gateway_listener.local_addr()?,
        agent: agent_listener.local_addr()?,
        commands: commands.clone(),
    };
    if config.gateway.agent_port == 0 {
        config.gateway.agent_port = running.agent.port();
    }
    let cap = usize::from(config.gateway.capacity);
    let commits = store_writer(store.clone());
    thread::spawn(move || simulate(world, inbox, commits, report));

    let agent_commands = commands.clone();
    let handshake = config.handshake;
    let notice: Arc<Notify> = Arc::new(notice);
    let agent_notice = notice.clone();
    thread::spawn(move || {
        listen(agent_listener, cap, move |stream| {
            serve_agent(stream, &agent_commands, handshake, &*agent_notice)
        })
    });
    let gateway = Arc::new(config.gateway);
    let idle = config.gateway_idle;
    thread::spawn(move || {
        listen(gateway_listener, cap, move |stream| {
            let shard = Link {
                store: store.clone(),
                commands: commands.clone(),
            };
            serve_gateway(stream, &gateway, shard, handshake, idle, &*notice)
        })
    });
    Ok(running)
}

/// What a connection hands the simulation thread.
enum Command {
    Connect {
        writer: Sender<Vec<u8>>,
        reply: Sender<SessionId>,
    },
    Receive {
        session: SessionId,
        opcode: u16,
        body: Bytes,
    },
    Disconnect(SessionId),
    Online(Sender<usize>),
    Admit {
        account: AccountId,
        username: String,
        characters: Vec<Character>,
        reply: Sender<Option<u32>>,
    },
    Query(Box<dyn FnOnce(&World) + Send>),
}

/// The storage thread: writes what the world commits, in order, so the
/// simulation never waits for the store. A failed write is logged; the
/// world goes on.
fn store_writer(store: Arc<dyn Store + Send + Sync>) -> Sender<Changes> {
    let (commits, queue) = mpsc::channel::<Changes>();
    thread::spawn(move || {
        for changes in queue {
            if let Err(e) = store.commit(&changes) {
                eprintln!("store: a commit failed: {e}");
            }
        }
    });
    commits
}

fn simulate(
    mut world: World,
    inbox: Receiver<Command>,
    commits: Sender<Changes>,
    mut report: impl FnMut(&World, &Event),
) {
    let mut writers: HashMap<SessionId, Sender<Vec<u8>>> = HashMap::new();
    let mut due = Instant::now() + STEP;
    loop {
        match inbox.recv_timeout(due.saturating_duration_since(Instant::now())) {
            Ok(command) => {
                apply(&mut world, &mut writers, command);
                while let Ok(command) = inbox.try_recv() {
                    apply(&mut world, &mut writers, command);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                world.step();
                due += STEP;
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
        for (session, writer) in &writers {
            let frames = world.take_frames(*session);
            let wire: Vec<u8> = frames
                .iter()
                .flat_map(|f| frame(f.opcode, &f.body))
                .collect();
            if !wire.is_empty() {
                // A writer that is gone has a reader that reports the disconnect.
                let _ = writer.send(wire);
            }
        }
        for changes in world.take_commits() {
            let _ = commits.send(changes);
        }
        for event in world.take_events() {
            // Dropping the writer ends the writing thread, which closes the
            // socket after the frames already queued.
            if let Event::Closed(session) | Event::Overflowed(session) = event {
                writers.remove(&session);
            }
            report(&world, &event);
        }
    }
}

fn apply(world: &mut World, writers: &mut HashMap<SessionId, Sender<Vec<u8>>>, command: Command) {
    match command {
        Command::Connect { writer, reply } => {
            let session = world.connect();
            writers.insert(session, writer);
            let _ = reply.send(session);
        }
        Command::Receive {
            session,
            opcode,
            body,
        } => world.receive(session, opcode, body),
        Command::Disconnect(session) => {
            world.disconnect(session);
            writers.remove(&session);
        }
        Command::Online(reply) => {
            let _ = reply.send(world.online());
        }
        Command::Admit {
            account,
            username,
            characters,
            reply,
        } => {
            let _ = reply.send(world.admit(account, &username, characters));
        }
        Command::Query(ask) => ask(world),
    }
}

/// Serves connections while fewer than `cap` are open; one beyond that is
/// closed at once.
fn listen(listener: TcpListener, cap: usize, serve: impl Fn(TcpStream) + Send + Sync + 'static) {
    let serve = Arc::new(serve);
    let open = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if open.fetch_add(1, Ordering::SeqCst) >= cap {
            open.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let (serve, open) = (serve.clone(), open.clone());
        thread::spawn(move || {
            serve(stream);
            open.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// The handshake on this thread, then the world session: frames go to the
/// simulation thread, and a second thread writes what it hands back.
fn serve_agent(
    mut stream: TcpStream,
    commands: &Sender<Command>,
    handshake: Duration,
    notice: &Notify,
) {
    let started = Instant::now();
    let (mut transport, opening) = Transport::open();
    let peer = Peer {
        listener: "agent",
        address: stream.peer_addr().ok(),
        seeds: transport.seeds(),
    };
    let mut session = None;
    let mut buf = [0u8; 4096];
    if stream.write_all(&opening).is_err() {
        return;
    }
    notice(Notice::Opened(peer.clone()));
    loop {
        let limit = session.is_none().then(|| left(started, handshake));
        let read = stream
            .set_read_timeout(limit)
            .and_then(|_| stream.read(&mut buf));
        let n = match read {
            Ok(n @ 1..) => n,
            ended => {
                notice_end(notice, peer, &transport, read_end(&ended, &transport));
                break;
            }
        };
        let inbound = match transport.receive(&buf[..n]) {
            Ok(inbound) => inbound,
            Err(broken) => {
                notice_end(notice, peer, &transport, Some(Why::Broken(broken)));
                break;
            }
        };
        if stream.write_all(&inbound.reply).is_err() {
            break;
        }
        if session.is_none() && transport.established() {
            session = connect(commands, &stream);
        }
        let Some(session) = session else { continue };
        for (opcode, body) in inbound.frames {
            let _ = commands.send(Command::Receive {
                session,
                opcode,
                body,
            });
        }
    }
    if let Some(session) = session {
        let _ = commands.send(Command::Disconnect(session));
    }
}

/// A world session for the connection, and the thread that writes it.
fn connect(commands: &Sender<Command>, stream: &TcpStream) -> Option<SessionId> {
    let mut writing = stream.try_clone().ok()?;
    let (writer, wire) = mpsc::channel::<Vec<u8>>();
    let (reply, session) = mpsc::channel();
    commands.send(Command::Connect { writer, reply }).ok()?;
    let session = session.recv().ok()?;
    thread::spawn(move || {
        for bytes in wire {
            if writing.write_all(&bytes).is_err() {
                break;
            }
        }
        let _ = writing.shutdown(Shutdown::Both);
    });
    Some(session)
}

fn serve_gateway(
    mut stream: TcpStream,
    gateway: &Gateway,
    mut shard: Link,
    handshake: Duration,
    idle: Duration,
    notice: &Notify,
) {
    let started = Instant::now();
    let (mut transport, opening) = Transport::open();
    let peer = Peer {
        listener: "gateway",
        address: stream.peer_addr().ok(),
        seeds: transport.seeds(),
    };
    let mut connection = GatewayConnection::open();
    let mut buf = [0u8; 4096];
    if stream.write_all(&opening).is_err() {
        return;
    }
    notice(Notice::Opened(peer.clone()));
    loop {
        let limit = match transport.established() {
            true => idle,
            false => left(started, handshake),
        };
        let read = stream
            .set_read_timeout(Some(limit))
            .and_then(|_| stream.read(&mut buf));
        let n = match read {
            Ok(n @ 1..) => n,
            ended => {
                notice_end(notice, peer, &transport, read_end(&ended, &transport));
                return;
            }
        };
        let inbound = match transport.receive(&buf[..n]) {
            Ok(inbound) => inbound,
            Err(broken) => {
                notice_end(notice, peer, &transport, Some(Why::Broken(broken)));
                return;
            }
        };
        let mut wire = inbound.reply;
        for (opcode, body) in inbound.frames {
            match connection.receive(gateway, &mut shard, opcode, body) {
                Ok(Some(answer)) => wire.extend(frame(answer.opcode, &answer.body)),
                Ok(None) => {}
                Err(_) => break,
            }
            if connection.closed() {
                break;
            }
        }
        if stream.write_all(&wire).is_err() || connection.closed() {
            return;
        }
    }
}

/// Why a read that brought no frame ends the connection, if the end is worth
/// a notice: a client may always close an established connection.
fn read_end(read: &io::Result<usize>, transport: &Transport) -> Option<Why> {
    match read {
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            Some(Why::Deadline)
        }
        _ if transport.established() => None,
        Ok(_) => Some(Why::Closed),
        Err(e) => Some(Why::Socket(e.kind())),
    }
}

fn notice_end(notice: &Notify, peer: Peer, transport: &Transport, why: Option<Why>) {
    if let Some(why) = why {
        let established = transport.established();
        notice(Notice::Ended {
            peer,
            established,
            why,
        });
    }
}

/// What is left of `limit` since `started`; never zero, which a socket would
/// take as no timeout.
fn left(started: Instant, limit: Duration) -> Duration {
    limit
        .saturating_sub(started.elapsed())
        .max(Duration::from_millis(1))
}

/// The gateway's shard in the server: the password is checked here, on the
/// gateway connection's thread; the rest is asked of the simulation thread.
struct Link {
    store: Arc<dyn Store + Send + Sync>,
    commands: Sender<Command>,
}

impl Link {
    fn ask<T>(&self, command: impl FnOnce(Sender<T>) -> Command) -> Option<T> {
        let (reply, answer) = mpsc::channel();
        self.commands.send(command(reply)).ok()?;
        answer.recv().ok()
    }
}

impl Shard for Link {
    fn login(&mut self, username: &str, password: &str) -> Option<AccountId> {
        store::login(self.store.as_ref(), username, password)
    }

    fn online(&mut self) -> usize {
        self.ask(Command::Online).unwrap_or(0)
    }

    /// The account's characters are read here, on the gateway's thread, so
    /// the simulation never waits for the store.
    fn admit(&mut self, account: AccountId, username: &str) -> Option<u32> {
        let username = username.to_owned();
        let characters = self.store.characters(account);
        self.ask(|reply| Command::Admit {
            account,
            username,
            characters,
            reply,
        })
        .flatten()
    }
}
