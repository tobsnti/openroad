use std::io::Error;
use std::io::ErrorKind::InvalidData;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex, RwLock};

use bevy::log::debug;
use bevy::prelude::Component;
use crossbeam::channel::{Receiver, Sender, TryRecvError};
use packets::global::ModuleIdentification;

use crate::net::frame::SilkroadFrame;
use crate::net::handshake::{FrameReader, FrameStep};
use crate::net::security::SilkroadSecurityState;
use crate::net::transport::SilkroadTransport;
use crate::net::{handshake, module_identification_frame};

/// Initial capacity for the inbound accumulator: a few 4 KB reads' worth, so
/// the common post-join burst is buffered without reallocating.
const READ_BUF_CAPACITY: usize = 4096 * 4;

#[derive(Component)]
#[allow(dead_code)]
pub struct SilkroadConnection {
    pub security: Arc<RwLock<SilkroadSecurityState>>,
    pub i_receiver: Receiver<SilkroadFrame>,
    pub i_sender: Sender<SilkroadFrame>,
    pub o_receiver: Receiver<SilkroadFrame>,
    pub o_sender: Sender<SilkroadFrame>,
    pub dc_receiver: Receiver<()>,
    pub dc_sender: Sender<()>,
    /// The byte pipe, behind a trait object rather than a type parameter: this
    /// is a bevy `Component`, and 44 files name it in `Query<&SilkroadConnection>`
    /// -- a generic would infect every one of those signatures, while the two
    /// virtual calls per frame (`plugins/net/plugin.rs`) sit behind a
    /// `Mutex::lock()` and a syscall anyway.
    pub stream: Arc<Mutex<dyn SilkroadTransport>>,
    /// Inbound byte accumulator. Each TCP read is appended here and complete
    /// frames are drained off the front; a frame split across reads keeps its
    /// tail buffered instead of being discarded.
    pub read_buf: Vec<u8>,
}

impl SilkroadConnection {
    /// Blocking connect + handshake. Kept for callers that run off the render
    /// loop (the headless net-check client); the render loop — the gateway
    /// connect and the agent-server handoff — uses [`Self::connect_async`].
    pub fn new(addr: &str) -> Result<SilkroadConnection, Error> {
        Self::establish(Self::resolve_addr(addr)?)
    }

    /// Connects + handshakes on a dedicated worker thread so a slow or
    /// unreachable gateway never stalls a Bevy frame. Returns immediately with a
    /// [`PendingConnection`] the ECS can poll each frame; the established
    /// connection (or the error) is delivered over a channel. The blocking,
    /// `expect()`-heavy handshake runs inside `catch_unwind`, so a malformed
    /// handshake surfaces as an error instead of aborting the process.
    pub fn connect_async(addr: &str) -> PendingConnection {
        let (tx, rx) = crossbeam::channel::bounded(1);
        let addr = addr.to_string();
        // Dropping the JoinHandle detaches the worker. A spawn that FAILS is
        // reported as itself: dropping the closure made `poll` see a channel
        // disconnect, and the browser then printed "connect worker terminated
        // before delivering a result" for a target that simply has no threads.
        let spawned = std::thread::Builder::new()
            .name("sro-connect".to_string())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> Result<_, Error> { Self::establish(Self::resolve_addr(&addr)?) },
                ))
                .unwrap_or_else(|_| Err(Error::other("connection handshake panicked")));
                // The receiver is gone if the ECS abandoned the attempt; drop the
                // result rather than unwrapping.
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => PendingConnection::Worker(rx),
            Err(err) => PendingConnection::Unstartable(Some(Error::other(format!(
                "cannot start a connect worker on this target: {err}"
            )))),
        }
    }

    /// Resolves a gateway address string to a `SocketAddr`, accepting both
    /// `ip:port` and `hostname:port` (the latter via a blocking DNS lookup).
    /// MUST only run off the render loop — both callers (`new` and the
    /// `connect_async` worker thread) satisfy that.
    fn resolve_addr(addr: &str) -> Result<SocketAddr, Error> {
        addr.to_socket_addrs()
            .map_err(|e| Error::new(InvalidData, format!("cannot resolve gateway '{addr}': {e}")))?
            .next()
            .ok_or_else(|| {
                Error::new(
                    InvalidData,
                    format!("gateway '{addr}' resolved to no addresses"),
                )
            })
    }

    /// Performs the full blocking connect: TCP connect, DH/blowfish handshake,
    /// module identification, and the switch to non-blocking mode that the
    /// receive loop relies on. On success the returned connection is ready to be
    /// polled by `receive_packets`.
    fn establish(addr: SocketAddr) -> Result<SilkroadConnection, Error> {
        let mut stream = TcpStream::connect(addr)?;
        debug!("connected to {}", addr);

        let security = Arc::new(RwLock::new(SilkroadSecurityState::new()));

        // The blocking driver for [`Handshake`]: one poll per loop, and the
        // socket deadline is the only thing this driver adds. Without it a poll
        // that finds nothing would block forever on a blocking socket; with it
        // the read comes back as `WouldBlock`, which the machine reads as
        // "nothing yet". Waiting for the FIRST frame stays unbounded even so --
        // the reader only bounds a frame that is already partly here, so a slow
        // gateway is still waited for, in five-second hops.
        let mut handshake = Handshake::new(security.clone());
        stream.set_read_timeout(Some(handshake::FRAME_COMPLETION_TIMEOUT))?;
        while !handshake.poll(&mut stream)? {}
        stream.set_read_timeout(None)?;

        // The receive/send loop expects WouldBlock instead of blocking reads.
        stream.set_nonblocking(true)?;

        Ok(Self::from_parts(
            Arc::new(Mutex::new(stream)),
            security,
            handshake.take_tail(),
        ))
    }

    /// The connection the ECS sees, once a handshake is done. Shared by the
    /// blocking driver and the polled one, so both arms of
    /// [`PendingConnection`] hand out the same thing.
    ///
    /// `tail` is whatever the handshake reader read past the last handshake
    /// frame; it belongs to `receive_packets`, not to the floor. On a socket it
    /// is empty -- the handshake is one request per response -- but a transport
    /// that hands over whole messages can carry the first game frames in the
    /// same read.
    fn from_parts(
        stream: Arc<Mutex<dyn SilkroadTransport>>,
        security: Arc<RwLock<SilkroadSecurityState>>,
        tail: Vec<u8>,
    ) -> SilkroadConnection {
        let (i_sender, i_receiver) = crossbeam::channel::unbounded::<SilkroadFrame>();
        let (o_sender, o_receiver) = crossbeam::channel::unbounded::<SilkroadFrame>();
        let (dc_sender, dc_receiver) = crossbeam::channel::bounded(1);

        let mut read_buf = Vec::with_capacity(READ_BUF_CAPACITY);
        read_buf.extend_from_slice(&tail);

        Self {
            read_buf,
            security,
            dc_sender,
            dc_receiver,
            i_sender,
            i_receiver,
            o_sender,
            o_receiver,
            stream,
        }
    }

    #[allow(dead_code)]
    pub fn get_receiver(&self) -> Receiver<SilkroadFrame> {
        self.i_receiver.clone()
    }

    pub fn get_sender(&self) -> Sender<SilkroadFrame> {
        self.o_sender.clone()
    }

    pub fn get_security(&self) -> Arc<RwLock<SilkroadSecurityState>> {
        self.security.clone()
    }

    pub fn get_dc_receiver(&self) -> Receiver<()> {
        self.dc_receiver.clone()
    }
}

/// Which of the handshake's three round trips is waiting for its answer.
enum HandshakeStep {
    /// Waiting for `0x5000` phase 1; answers with the `0x5000` reply.
    Setup,
    /// Waiting for `0x5000` phase 2; answers with `0x9000`, then sends the
    /// module identification.
    Challenge,
    /// Waiting for `0x2001`, after which the connection is up.
    ModuleIdent,
}

/// The handshake as a state machine, driven by whoever owns the transport.
///
/// Idea. The three round trips used to be a straight line of blocking reads on
/// one thread. A browser cannot run that line at all: its bytes arrive in a
/// callback that cannot run while a thread blocks, so a blocking read there
/// never finishes -- a stall, not an error. What the line needed in order to be
/// interruptible is exactly what it kept on the stack: the partly-read frame,
/// and which round trip it is in. Both are fields here. Everything else the
/// handshake needs already lives in the shared `security` cell, and the key
/// agreement itself never spans two polls -- it runs between a read and the
/// answer to that read.
///
/// The transport is deliberately NOT a field: the driver passes it in. The
/// blocking driver ([`SilkroadConnection::establish`]) owns a `TcpStream` it
/// still has to switch to non-blocking afterwards -- `set_nonblocking` belongs
/// to whatever constructs the connection, not to the byte pipe (`transport.rs`)
/// -- and a callback-fed driver owns something else entirely. Neither has to
/// give up ownership to be polled.
pub(crate) struct Handshake {
    security: Arc<RwLock<SilkroadSecurityState>>,
    reader: FrameReader,
    step: HandshakeStep,
}

impl Handshake {
    pub(crate) fn new(security: Arc<RwLock<SilkroadSecurityState>>) -> Self {
        Self {
            security,
            reader: FrameReader::new(),
            step: HandshakeStep::Setup,
        }
    }

    /// One step. `false` means "nothing arrived yet, call again" -- which is a
    /// state, not a failure; `true` means the handshake is complete and the
    /// caller may build the connection.
    ///
    /// The loop is not an idle spin: it only goes round when a frame was
    /// actually handled, because one read can carry more than one frame.
    pub(crate) fn poll(&mut self, stream: &mut dyn SilkroadTransport) -> Result<bool, Error> {
        loop {
            let step = self
                .reader
                .poll(&mut *stream, self.security.clone())
                .map_err(|e| Error::new(InvalidData, e))?;
            let FrameStep::Ready(frame) = step else {
                return Ok(false);
            };
            match self.step {
                HandshakeStep::Setup => {
                    let next = handshake::initialize(frame, &mut *stream, self.security.clone())
                        .map_err(|e| Error::new(InvalidData, e))?;
                    publish(&self.security, next);
                    debug!("finished handshake init");
                    self.step = HandshakeStep::Challenge;
                }
                HandshakeStep::Challenge => {
                    let next = handshake::finalize(frame, &mut *stream, self.security.clone())
                        .map_err(|e| Error::new(InvalidData, e))?;
                    publish(&self.security, next);
                    // A write is not a wait boundary -- 18 bytes and no answer
                    // expected in this step -- so sending the module
                    // identification rides along here instead of costing a state
                    // of its own.
                    send_module_identification(&self.security, &mut *stream)?;
                    self.step = HandshakeStep::ModuleIdent;
                }
                HandshakeStep::ModuleIdent => {
                    check_module_identification(frame)?;
                    return Ok(true);
                }
            }
        }
    }

    /// See [`FrameReader::take_tail`]: the bytes read past the last handshake
    /// frame, for the connection's own accumulator.
    pub(crate) fn take_tail(&mut self) -> Vec<u8> {
        self.reader.take_tail()
    }
}

/// Copies a finished phase into the shared cell, as `do_handshake` did: the
/// state is what every later frame's sequence byte and CRC hang off.
fn publish(security: &Arc<RwLock<SilkroadSecurityState>>, next: SilkroadSecurityState) {
    let mut s = security.write().expect("security to be writable");
    s.context = next.context;
    s.state = next.state;
}

/// Sends our module identification. Pure write: nothing is read here, so this
/// is not a state of its own.
fn send_module_identification(
    security: &Arc<RwLock<SilkroadSecurityState>>,
    stream: &mut dyn SilkroadTransport,
) -> Result<(), Error> {
    let frame = module_identification_frame();
    let buf = frame.serialize(Arc::clone(security)).map_err(|_| {
        Error::new(
            InvalidData,
            "failed to serialize module identification frame",
        )
    })?;
    stream.write_all(&buf)?;
    Ok(())
}

/// Checks the gateway's module identification. The `0x2001` check is unchanged;
/// its bytes now come from the frame reader, which is what also gives this
/// third round trip the split-frame handling the other two have had since #107
/// -- it used to be a bare `read` into a stack buffer, so a `0x2001` that TCP
/// split failed the connect.
fn check_module_identification(frame: SilkroadFrame) -> Result<(), Error> {
    match frame {
        SilkroadFrame::Packet { opcode, data, .. } => {
            if opcode != 0x2001 {
                return Err(Error::new(
                    InvalidData,
                    format!("invalid opcode: {}", opcode),
                ));
            }
            // Typed parse rather than an inline read: a truncated body used
            // to index past the end of `data` and panic the connect thread.
            let ident = ModuleIdentification::try_from(data).map_err(|e| {
                Error::new(
                    InvalidData,
                    format!("failed to parse module identification packet: {e}"),
                )
            })?;
            debug!(
                "module identified: {} ({:?})",
                ident.module,
                ident.peer_kind()
            );
            Ok(())
        }
        _ => Err(Error::new(
            InvalidData,
            "received unexpected massive packet",
        )),
    }
}

/// Handle to a connection that is not established yet.
///
/// Two shapes, because two targets. On a target with threads the connect and
/// the handshake run on a worker and arrive through a channel; on one without,
/// nothing can be handed off, so the caller supplies the transport and the ECS
/// drives [`Handshake`] itself -- one step per [`Self::poll`], which is what
/// that machine was built for (#119). Either way the ECS sees the same
/// `Option<Result<..>>` and the same component swap, so no caller has to know
/// which arm it holds.
#[derive(Component)]
pub enum PendingConnection {
    /// A worker thread is doing connect + handshake.
    Worker(Receiver<Result<SilkroadConnection, Error>>),
    /// No worker: this polls the handshake over a transport the caller already
    /// opened.
    ///
    /// Not constructed by the binary yet, and clippy says so ("variant
    /// `Driving` is never constructed"). That is accurate: the first target
    /// that needs it is a browser, and a browser also needs a transport that
    /// does not exist yet -- `TcpStream::connect` answers `Unsupported` there.
    /// It is kept rather than deferred because the tests below drive a whole
    /// handshake through it on every `cargo test`, so it cannot rot while it
    /// waits, and because the slice that adds the transport then adds no new
    /// shape, only an implementation. **Delete this `allow` with that slice.**
    #[allow(dead_code)]
    Driving {
        stream: Arc<Mutex<dyn SilkroadTransport>>,
        security: Arc<RwLock<SilkroadSecurityState>>,
        handshake: Handshake,
    },
    /// The attempt could not be started at all on this target. Carried rather
    /// than swallowed: the browser used to report a failed `thread::spawn` as
    /// "connect worker terminated before delivering a result", which names the
    /// symptom and hides the cause (measured in a browser, 2026-10-17).
    Unstartable(Option<Error>),
}

impl PendingConnection {
    /// Drives a handshake over an already-open transport, for targets that
    /// cannot hand the work to a thread. The transport is expected to answer
    /// `WouldBlock` rather than block -- in a browser that is the only thing it
    /// can do.
    ///
    /// See [`PendingConnection::Driving`] for why this has no caller in the
    /// binary yet.
    #[allow(dead_code)]
    pub fn driving(
        stream: Arc<Mutex<dyn SilkroadTransport>>,
        security: Arc<RwLock<SilkroadSecurityState>>,
    ) -> Self {
        let handshake = Handshake::new(security.clone());
        PendingConnection::Driving {
            stream,
            security,
            handshake,
        }
    }

    /// Non-blocking check for the connect result: `None` while still
    /// connecting, `Some(Ok)`/`Some(Err)` once it is decided.
    pub fn poll(&mut self) -> Option<Result<SilkroadConnection, Error>> {
        match self {
            PendingConnection::Worker(rx) => match rx.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                // A worker that vanished without delivering is reported rather
                // than leaving the caller polling forever.
                Err(TryRecvError::Disconnected) => Some(Err(Error::other(
                    "connect worker terminated before delivering a result",
                ))),
            },
            PendingConnection::Driving {
                stream,
                security,
                handshake,
            } => {
                let Ok(mut guard) = stream.lock() else {
                    return Some(Err(Error::other("handshake transport lock poisoned")));
                };
                match handshake.poll(&mut *guard) {
                    Ok(false) => None,
                    Ok(true) => {
                        let tail = handshake.take_tail();
                        drop(guard);
                        Some(Ok(SilkroadConnection::from_parts(
                            stream.clone(),
                            security.clone(),
                            tail,
                        )))
                    }
                    Err(e) => Some(Err(e)),
                }
            }
            // Reported once; a caller that keeps polling gets a plain repeat
            // instead of a second, different error.
            PendingConnection::Unstartable(err) => Some(Err(err
                .take()
                .unwrap_or_else(|| Error::other("connection attempt was not startable")))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::Duration;

    use bevy::platform::time::Instant;

    /// Polls until a result arrives or the deadline passes, so the tests never
    /// hang if the worker misbehaves.
    fn poll_until_ready(
        pending: &mut PendingConnection,
        timeout: Duration,
    ) -> Option<Result<SilkroadConnection, Error>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(result) = pending.poll() {
                return Some(result);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn resolve_addr_accepts_literal_ip_port() {
        let addr = SilkroadConnection::resolve_addr("127.0.0.1:1").expect("ip:port resolves");
        assert_eq!(addr.port(), 1);
        assert!(addr.ip().is_loopback());
    }

    #[test]
    fn resolve_addr_rejects_malformed_without_dns() {
        // No `host:port` shape -> rejected during parsing; no DNS lookup, so this
        // stays offline and cannot block.
        assert!(SilkroadConnection::resolve_addr("not-an-addr").is_err());
    }

    #[test]
    fn connect_async_returns_immediately_and_polls_none_while_connecting() {
        // A listener that accepts but never speaks: the worker connects, then
        // blocks in the handshake read. The point is that `connect_async`
        // returned control instantly and the ECS-facing poll never blocks.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        let addr = listener.local_addr().expect("listener addr").to_string();

        let start = Instant::now();
        let mut pending = SilkroadConnection::connect_async(&addr);
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "connect_async must not block the caller"
        );
        assert!(
            pending.poll().is_none(),
            "poll must report None while the handshake is still in flight"
        );
        // `listener` is kept alive until here so the worker stays blocked in its
        // read rather than seeing a connection reset.
        drop(listener);
    }

    #[test]
    fn connect_async_surfaces_invalid_address_as_error() {
        let mut pending = SilkroadConnection::connect_async("this is not a socket address");
        match poll_until_ready(&mut pending, Duration::from_secs(2)) {
            Some(Err(_)) => {}
            other => panic!(
                "expected an error result, got {:?}",
                other.map(|r| r.is_ok())
            ),
        }
    }

    /// THE ARM THE BROWSER NEEDS: a whole handshake through `poll`, with no
    /// thread and no socket anywhere. Each poll that finds nothing answers
    /// `None` -- which is what an ECS system does with the rest of its frame --
    /// and the bytes are fed in between polls, the way a `message` callback
    /// would feed them.
    #[test]
    fn the_driving_arm_finishes_a_handshake_without_a_thread() {
        use crate::net::handshake::test_fixtures::{
            challenge_frame, module_ident_response, server_challenge_for, setup_frame_full_flags,
        };
        use crate::net::security::SilkroadSecurity;
        use crate::net::transport::MemPipe;

        let pipe = Arc::new(Mutex::new(MemPipe::new([setup_frame_full_flags()])));
        let security = Arc::new(RwLock::new(SilkroadSecurityState::new()));
        let mut pending = PendingConnection::driving(pipe.clone(), security.clone());

        // Round 1: the setup frame is there, the rest is not.
        assert!(
            pending.poll().is_none(),
            "phase 1 does not finish the connect"
        );
        // Two polls with nothing at all. Neither is a failure.
        assert!(pending.poll().is_none());
        assert!(pending.poll().is_none());

        // Round 2 and 3, fed in between polls.
        let challenge = server_challenge_for(&security);
        pipe.lock().expect("pipe").push(challenge_frame(challenge));
        assert!(
            pending.poll().is_none(),
            "phase 2 does not finish it either"
        );
        pipe.lock()
            .expect("pipe")
            .push(module_ident_response("GatewayServer"));

        let conn = pending
            .poll()
            .expect("the third round trip finishes the connect")
            .expect("over a pipe, the handshake succeeds");
        assert!(
            conn.read_buf.is_empty(),
            "one request per response leaves nothing over"
        );
        assert!(matches!(
            security.read().expect("readable").state,
            SilkroadSecurity::Established
        ));
    }

    /// A target without threads must say so. This used to arrive as "connect
    /// worker terminated before delivering a result" -- the symptom of a
    /// dropped channel, not the cause -- which is exactly what a browser
    /// printed before this arm existed.
    #[test]
    fn an_unstartable_attempt_reports_its_own_cause() {
        let mut pending = PendingConnection::Unstartable(Some(Error::other(
            "cannot start a connect worker on this target: whatever the OS said",
        )));
        // `let Err(..) else`, not `expect_err`: the Ok type is not `Debug`.
        let Some(Err(err)) = pending.poll() else {
            panic!("an unstartable attempt is decided at once, as an error");
        };
        assert!(
            err.to_string().contains("cannot start a connect worker"),
            "the cause has to survive, got {err}"
        );
        // A caller that keeps polling gets a plain repeat, not a second story.
        let Some(Err(again)) = pending.poll() else {
            panic!("still decided, still an error");
        };
        assert!(again.to_string().contains("not startable"));
    }
}
