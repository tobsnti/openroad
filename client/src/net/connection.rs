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
        // Dropping the JoinHandle detaches the worker; if the spawn itself fails
        // the closure (and `tx`) is dropped, which `PendingConnection::poll`
        // observes as a disconnect and reports as an error.
        let _ = std::thread::Builder::new()
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
        PendingConnection { rx }
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

        let (i_sender, i_receiver) = crossbeam::channel::unbounded::<SilkroadFrame>();
        let (o_sender, o_receiver) = crossbeam::channel::unbounded::<SilkroadFrame>();
        let (dc_sender, dc_receiver) = crossbeam::channel::bounded(1);

        // Whatever the reader read past the last handshake frame belongs to
        // `receive_packets`, not to the floor. On a socket this is empty -- the
        // handshake is one request per response -- but a transport that hands
        // over whole messages can carry the first game frames in the same read,
        // and dropping them only looked harmless while each read went into a
        // stack buffer that died with the call.
        let mut read_buf = Vec::with_capacity(READ_BUF_CAPACITY);
        read_buf.extend_from_slice(&handshake.take_tail());

        Ok(Self {
            read_buf,
            security,
            dc_sender,
            dc_receiver,
            i_sender,
            i_receiver,
            o_sender,
            o_receiver,
            stream: Arc::new(Mutex::new(stream)),
        })
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

/// Handle to an in-flight [`SilkroadConnection::connect_async`]. Lives as an ECS
/// component while a worker thread performs the blocking connect + handshake;
/// [`Self::poll`] is non-blocking, so the app keeps rendering until the
/// connection (or an error) is ready.
#[derive(Component)]
pub struct PendingConnection {
    rx: Receiver<Result<SilkroadConnection, Error>>,
}

impl PendingConnection {
    /// Non-blocking check for the connect result: `None` while still
    /// connecting, `Some(Ok)`/`Some(Err)` once the worker finished. A worker
    /// that vanished without delivering (e.g. a failed thread spawn) is reported
    /// as an error rather than leaving the caller polling forever.
    pub fn poll(&self) -> Option<Result<SilkroadConnection, Error>> {
        match self.rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(Error::other(
                "connect worker terminated before delivering a result",
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    /// Polls until a result arrives or the deadline passes, so the tests never
    /// hang if the worker misbehaves.
    fn poll_until_ready(
        pending: &PendingConnection,
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
        let pending = SilkroadConnection::connect_async(&addr);
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
        let pending = SilkroadConnection::connect_async("this is not a socket address");
        match poll_until_ready(&pending, Duration::from_secs(2)) {
            Some(Err(_)) => {}
            other => panic!(
                "expected an error result, got {:?}",
                other.map(|r| r.is_ok())
            ),
        }
    }
}
