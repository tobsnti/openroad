//! `sro_peer` — our own headless test peer: gateway + agent leg, just enough
//! for **our own** client (`client/src/netcheck.rs`) to reach world entry
//! without any foreign server.
//!
//! Idea (the plan this implements: `docs/planning/OWN-TEST-PEER.md` §2.1/§4,
//! steps 2 and 3): the expensive halves already exist in this repo. The
//! *server* half of the SRO handshake, the framing and the massive rules are in
//! `tools/src/bin/sro_proxy.rs` (`serve_handshake:384`, `next_packet:627`), and
//! the *bodies* of every answer are on disk as real recorded wire bytes in
//! `packet_dump/<opcode>.log`. So this peer invents nothing: it replays the
//! recorded body for each answer and only *builds* the one packet that must
//! name our own endpoint — `0xA102`, whose `LoginInfo` redirects the client to
//! this process' own agent listener (`packets/src/login.rs`).
//!
//! Why the bodies are replayed rather than modelled: a self-invented 20-byte
//! `0x3013` does not bring the client into the world. The real CHARACTER_DATA
//! frame is 773..1497 bytes and our parser is staged
//! (`packets/src/agent/character_data.rs`), which is exactly why the sibling
//! repo's stub server cannot be ported as a wire source
//! (`docs/planning/OWN-TEST-PEER.md` §2.1, point 1).
//!
//! Two deliberate deviations from a real gateway, both cheap and both stated:
//! * **No compression bit** in the `0x5000` setup (flags `0x0E`): our client has
//!   no decompression (`client/src/net/` has no `compression.rs`), so offering
//!   it would break the session.
//! * **No captcha challenge** (`0x2322`) is ever sent; a `0x6323` is answered
//!   with the recorded `0xA323` success body. Headless has no image decoder for
//!   the IBUV image, and the captcha is not what a world-entry gate measures.
//!
//! Nothing here talks to a foreign server, and no SRO binary is executed. It is
//! a listener on loopback plus a directory of bytes we recorded ourselves.
//!
//! Run: `make peer` (or `cargo run -p tools --bin sro_peer`), then point the
//! client at it — see `docs/planning/OWN-TEST-PEER.md` §7.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use byteorder::{ByteOrder, LittleEndian};
use bytes::{Buf, BufMut, Bytes, BytesMut};

use client::net::blowfish::Blowfish;
use client::net::frame::{SilkroadFrame, SilkroadFrameError};
use client::net::handshake::{calc_challenge, calc_key, final_blowfish_key, g_pow_x_mod_p};
use client::net::security::SilkroadSecurityState;
use packets::gateway::{NoticeResponse, PatchResponse};
use packets::global::ModuleIdentification;
use packets::login::{LoginInfo, LoginResponse};
use packets::Packet;

/// Gateway leg. 15779 is the gateway port of the user's own testserver
/// (`tools/src/bin/sro_proxy.rs:DEFAULT_GATEWAY_UPSTREAM`), so a config that
/// already names a port keeps its meaning; loopback only, on purpose.
const DEFAULT_GATEWAY_LISTEN: &str = "127.0.0.1:15779";
/// Agent leg. Not the client's choice — we *name* it in our own `0xA102`, so any
/// free port works; 15884 is what the reference deployment uses
/// (`packet_dump/0xa102.log`: port `0x3e0c` = 15884).
const DEFAULT_AGENT_LISTEN: &str = "127.0.0.1:15884";
/// Where the recorded answer bodies come from: our own client's dump directory
/// (`client/src/plugins/net/packet_dump.rs`).
const DEFAULT_FIXTURE_DIR: &str = "packet_dump";

/// Full setup flags `blowfish|security_bytes|handshake` = `0x0E`, **without**
/// the compression bit — same value and same reasoning as
/// `tools/src/bin/sro_proxy.rs:SETUP_FLAGS`.
const SETUP_FLAGS: u8 = 0x0E;
/// `handshake_response` flag (xBot `SecurityAPI/Security.cs:13-31`).
const FLAG_HANDSHAKE_RESPONSE: u8 = 0x10;

const READ_CHUNK: usize = 4096;

// ---------------------------------------------------------------------------
// recorded bodies
// ---------------------------------------------------------------------------

/// The answer bodies, read once at startup from `packet_dump/<opcode>.log`.
///
/// Line format is the dumper's own (`packet_dump.rs`): `<rfc3339 ms> <hex>
/// [P|E]`, where the trailing letter is the frame's wire `0x8000` bit and is
/// *absent* in lines written before that column existed. An empty body (e.g.
/// `0x34A5`) therefore appears as a line with no hex field at all, which is why
/// the parser classifies fields instead of counting them.
struct Fixtures {
    dir: PathBuf,
    bodies: HashMap<u16, Bytes>,
    /// *Every* recorded body of an opcode, for the answers whose shape depends
    /// on what was asked. Only `0xB007` needs this: `0x7007` is a *multiplexed*
    /// request whose first byte is the action, and the recorded `0xB007` lines
    /// answer different actions (`packet_dump/proxy/0xb007.log` first bytes:
    /// `01` create, `02` list, `03`, `04` name check, `05`). Our own client only
    /// ever sends action `02`, the original client also sends `01`/`04`
    /// (`packet_dump/proxy/c2s/0x7007.log`), and answering a name check with a
    /// character list is worse than not answering at all.
    variants: HashMap<u16, Vec<Bytes>>,
}

/// Which line of a per-opcode log to take.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Pick {
    /// The last recorded body. Right for every answer whose body is constant
    /// across sessions (`0xA101`, `0xA106`, `0xB001`, …) — and the newest line
    /// is the one written by the newest client.
    Last,
    /// The longest recorded body. Only for `0x3013`: the CHARACTER_DATA parser
    /// is **staged** (`packets/src/agent/character_data.rs`), and a session that
    /// died mid-stream leaves a short body behind. A short body would still
    /// produce a *green* run — and prove nothing, because the later stages
    /// (inventory, avatar, mask/quest sections) were never reached. The longest
    /// recording is the one that covers the most stages, so that is the one a
    /// gate must replay.
    Longest,
}

impl Fixtures {
    /// `wanted` must be present (a missing one aborts the start); `optional` may
    /// be missing (only the *original* client asks for those, so a dump taken
    /// with our own client has no line for them — that is a fact about the
    /// recording, not a defect, and the peer degrades to "swallow + log").
    fn load(
        dir: PathBuf,
        wanted: &[(u16, Pick)],
        optional: &[(u16, Pick)],
        variant_opcodes: &[u16],
    ) -> Result<Self, String> {
        let mut bodies = HashMap::new();
        for (opcode, pick) in wanted {
            let path = find_log(&dir, *opcode)
                .ok_or_else(|| format!("{}: no such file", log_path(&dir, *opcode).display()))?;
            let body = read_body(&path, *pick).map_err(|e| format!("{}: {e}", path.display()))?;
            println!(
                "sro_peer: fixture {opcode:#06x} = {} byte(s) from {}",
                body.len(),
                path.display()
            );
            bodies.insert(*opcode, body);
        }
        for (opcode, pick) in optional {
            match find_log(&dir, *opcode).and_then(|path| {
                read_body(&path, *pick)
                    .map_err(|e| println!("sro_peer: fixture {opcode:#06x} unusable: {e}"))
                    .ok()
                    .map(|body| (path, body))
            }) {
                Some((path, body)) => {
                    println!(
                        "sro_peer: fixture {opcode:#06x} = {} byte(s) from {} (optional)",
                        body.len(),
                        path.display()
                    );
                    bodies.insert(*opcode, body);
                }
                None => println!(
                    "sro_peer: no recorded {opcode:#06x} — requests it answers will be swallowed \
                     with a log line"
                ),
            }
        }
        let mut variants = HashMap::new();
        for opcode in variant_opcodes {
            // **Both** dump directories here, not the first that exists: the
            // actions we are picking between were recorded by *different*
            // clients — action `02` by ours in `packet_dump/`, actions `01`/`04`
            // by the original one in `packet_dump/proxy/`. Taking only the
            // primary log would leave the original client's name check
            // unanswered even though we have its answer on disk.
            let mut all: Vec<Bytes> = Vec::new();
            for path in [
                log_path(&dir, *opcode),
                log_path(&dir.join("proxy"), *opcode),
            ] {
                if !path.is_file() {
                    continue;
                }
                match read_bodies(&path) {
                    Ok(bodies) => {
                        println!(
                            "sro_peer: fixture {opcode:#06x} += {} recorded variant(s) from {}",
                            bodies.len(),
                            path.display()
                        );
                        all.extend(bodies.into_iter().map(Bytes::from));
                    }
                    Err(e) => println!("sro_peer: {}: {e}", path.display()),
                }
            }
            if all.is_empty() {
                println!("sro_peer: no recorded {opcode:#06x} variants");
            } else {
                variants.insert(*opcode, all);
            }
        }
        Ok(Self {
            dir,
            bodies,
            variants,
        })
    }

    /// The recorded body, or `None` for an optional fixture we never recorded.
    fn body_opt(&self, opcode: u16) -> Option<Bytes> {
        self.bodies.get(&opcode).cloned()
    }

    /// The recorded body of `opcode` with exactly `len` bytes, if there is one.
    fn variants_len(&self, opcode: u16, len: usize) -> Option<Bytes> {
        self.variants
            .get(&opcode)?
            .iter()
            .find(|body| body.len() == len)
            .cloned()
    }

    /// The longest recorded body of `opcode` whose first byte is `action`.
    /// Longest, for the same reason as `Pick::Longest`: a longer answer of the
    /// same action carries more sections and exercises more of the parser.
    fn variant(&self, opcode: u16, action: u8) -> Option<Bytes> {
        self.variants
            .get(&opcode)?
            .iter()
            .filter(|body| body.first() == Some(&action))
            .max_by_key(|body| body.len())
            .cloned()
    }

    /// The recorded body for `opcode`. Absent only if the load list is wrong,
    /// which is a programming error, not a runtime condition.
    fn body(&self, opcode: u16) -> Bytes {
        self.bodies.get(&opcode).cloned().unwrap_or_else(|| {
            panic!(
                "fixture {opcode:#06x} was never loaded (dir {})",
                self.dir.display()
            )
        })
    }
}

/// Pull one body out of a dump log. Returns an error rather than an empty body
/// when the file is missing: a missing fixture means "we never recorded this
/// packet", and inventing one is precisely what
/// `docs/planning/OWN-TEST-PEER.md` forbids.
fn read_body(path: &Path, pick: Pick) -> Result<Bytes, String> {
    let all = read_bodies(path)?;
    let chosen = match pick {
        Pick::Last => all.last().cloned(),
        Pick::Longest => all.iter().max_by_key(|b| b.len()).cloned(),
    };
    chosen
        .map(Bytes::from)
        .ok_or_else(|| "no recorded body in this log".to_string())
}

/// Where a per-opcode log lives, primary location.
fn log_path(dir: &Path, opcode: u16) -> PathBuf {
    dir.join(format!("{opcode:#06x}.log"))
}

/// A log for `opcode`, in `dir` or in `dir/proxy`.
///
/// Two dump directories exist and they were written by different clients:
/// `packet_dump/` by **our** client and `packet_dump/proxy/` by `sro_proxy`
/// while the **original** client was talking through it
/// (`tools/src/bin/sro_proxy.rs`). Some answers exist only in the second one —
/// `0xB50E` for instance, because only the original client asks `0x750E`
/// (`packet_dump/proxy/c2s/0x750e.log`). Preferring the primary directory keeps
/// stage 1 byte-identical to the accepted run of §7.5.
fn find_log(dir: &Path, opcode: u16) -> Option<PathBuf> {
    let direct = log_path(dir, opcode);
    if direct.is_file() {
        return Some(direct);
    }
    let via_proxy = log_path(&dir.join("proxy"), opcode);
    via_proxy.is_file().then_some(via_proxy)
}

/// Every recorded body in one dump log, in file order.
fn read_bodies(path: &Path) -> Result<Vec<Vec<u8>>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut lines = 0usize;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        // field 1 is the timestamp; the body is the next field unless that
        // field is the flag column (`P`/`E`), i.e. unless the body was empty.
        let Some(_ts) = fields.next() else { continue };
        let hex = match fields.next() {
            Some("P") | Some("E") | None => "",
            Some(hex) => hex,
        };
        let Some(bytes) = unhex(hex) else {
            return Err(format!("line {} is not hex: {hex:?}", lines + 1));
        };
        lines += 1;
        out.push(bytes);
    }
    Ok(out)
}

fn unhex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let bytes = hex.as_bytes();
    let mut out = Vec::with_capacity(hex.len() / 2);
    for pair in bytes.chunks(2) {
        let s = std::str::from_utf8(pair).ok()?;
        out.push(u8::from_str_radix(s, 16).ok()?);
    }
    Some(out)
}

/// Lowercase hex, same encoding the dump uses (`sro_proxy.rs:hex`).
fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(s, "{byte:02x}");
    }
    s
}

fn stamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

// ---------------------------------------------------------------------------
// the server half of the handshake — ported from tools/src/bin/sro_proxy.rs
// ---------------------------------------------------------------------------
//
// Ported, not shared: the proxy keeps it inside its binary, and that file is
// the capture tool — changing it to expose a module would put a second caller
// on the code that attests our captures. The crypto itself is *reused* from
// `client/src/net/**` (same reasoning as the proxy's §5): a second copy of the
// crypto could drift from the client we ship.

struct ServerHandshake {
    state: SilkroadSecurityState,
}

#[derive(Debug)]
enum HandshakeError {
    Io(std::io::Error),
    Frame(String),
    ClientSignature { expected: u64, received: u64 },
    UnexpectedPacket(u16),
    BadBody(usize),
    Key,
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandshakeError::Io(e) => write!(f, "i/o error: {e}"),
            HandshakeError::Frame(e) => write!(f, "frame error: {e}"),
            HandshakeError::ClientSignature { expected, received } => write!(
                f,
                "client signature error: expected {expected:#018x}, received {received:#018x}"
            ),
            HandshakeError::UnexpectedPacket(opcode) => {
                write!(f, "unexpected opcode {opcode:#06x} during handshake")
            }
            HandshakeError::BadBody(len) => {
                write!(f, "handshake response body has {len} byte(s), needs 12")
            }
            HandshakeError::Key => write!(f, "derived an invalid blowfish key"),
        }
    }
}

impl From<std::io::Error> for HandshakeError {
    fn from(e: std::io::Error) -> Self {
        HandshakeError::Io(e)
    }
}

/// Everything the server half draws for one session
/// (`sro_proxy.rs:277 SetupParams`, xBot `Security.cs:411,419-421,428-432`):
/// full-width random blowfish/handshake keys, **8-bit** count and CRC seeds
/// widened to `u32` on the wire, and `x`,`g`,`p` masked to 31 bits so the modpow
/// cannot overflow. No fixed constant, so no magic number to source.
#[derive(Copy, Clone)]
struct SetupParams {
    blowfish_key: u64,
    sequence_seed: u32,
    crc_seed: u32,
    handshake_key: u64,
    private_exponent: u32,
    generator: u32,
    prime: u32,
    local_public: u32,
}

impl SetupParams {
    fn generate() -> Self {
        let generator = rand::random::<u32>() & 0x7FFF_FFFF;
        let prime = rand::random::<u32>() & 0x7FFF_FFFF;
        let private_exponent = rand::random::<u32>() & 0x7FFF_FFFF;
        Self {
            blowfish_key: rand::random(),
            sequence_seed: u32::from(rand::random::<u8>()),
            crc_seed: u32::from(rand::random::<u8>()),
            handshake_key: rand::random(),
            private_exponent,
            generator,
            prime,
            local_public: g_pow_x_mod_p(generator, private_exponent, prime),
        }
    }

    fn setup_body(&self) -> Bytes {
        let mut body = BytesMut::new();
        body.put_u8(SETUP_FLAGS);
        body.put_u64_le(self.blowfish_key);
        body.put_u32_le(self.sequence_seed);
        body.put_u32_le(self.crc_seed);
        body.put_u64_le(self.handshake_key);
        body.put_u32_le(self.generator);
        body.put_u32_le(self.prime);
        body.put_u32_le(self.local_public);
        body.freeze()
    }
}

fn challenge_body(challenge: u64) -> Bytes {
    let mut body = BytesMut::new();
    body.put_u8(FLAG_HANDSHAKE_RESPONSE);
    body.put_u64_le(challenge);
    body.freeze()
}

fn encrypt_u64(blowfish: &Blowfish, value: Bytes) -> u64 {
    let mut buf = BytesMut::new();
    buf.extend_from_slice(&value);
    blowfish.encrypt(&mut buf);
    LittleEndian::read_u64(&buf.freeze())
}

fn read_handshake_frame(
    stream: &mut TcpStream,
    security: &Arc<RwLock<SilkroadSecurityState>>,
) -> Result<SilkroadFrame, HandshakeError> {
    const MAX_HANDSHAKE_BYTES: usize = 4096;
    let mut pending: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(HandshakeError::Frame("peer closed mid-handshake".into()));
        }
        pending.extend_from_slice(&chunk[..read]);
        if pending.len() > MAX_HANDSHAKE_BYTES {
            return Err(HandshakeError::Frame("handshake frame too large".into()));
        }
        match SilkroadFrame::parse(&mut pending, security.clone()) {
            Ok((_, frame)) => return Ok(frame),
            Err(SilkroadFrameError::Incomplete) => continue,
            Err(e) => return Err(HandshakeError::Frame(e.to_string())),
        }
    }
}

/// Drive the **server** side of the handshake against a connected client —
/// line-for-line the proxy's `serve_handshake` (`sro_proxy.rs:384`), whose
/// doc comment carries the full wire sequence and its citations.
///
/// The one invariant worth repeating here, because getting it wrong resets the
/// client: the server role stamps **no** security bytes — count and CRC stay
/// `0000` (`SilkroadSecurityState::as_server_role`, xBot `Security.cs:700-712`,
/// and the live gateway's zeroed S→C frames).
fn serve_handshake(stream: &mut TcpStream, leg: &str) -> Result<ServerHandshake, HandshakeError> {
    let params = SetupParams::generate();
    let plain = Arc::new(RwLock::new(SilkroadSecurityState::new().as_server_role()));
    let setup = SilkroadFrame::Packet {
        count: 0,
        crc: 0,
        opcode: 0x5000,
        encrypted: 0,
        data: params.setup_body(),
    };
    let buf = setup
        .serialize(plain.clone())
        .map_err(|e| HandshakeError::Frame(e.to_string()))?;
    stream.write_all(&buf)?;
    println!(
        "{} sro_peer[{leg}]: sent 0x5000 setup ({} bytes)",
        stamp(),
        buf.len()
    );

    let frame = read_handshake_frame(stream, &plain)?;
    let (opcode, data) = match frame {
        SilkroadFrame::Packet { opcode, data, .. } => (opcode, data),
        other => return Err(HandshakeError::Frame(other.to_string())),
    };
    if opcode != 0x5000 {
        return Err(HandshakeError::UnexpectedPacket(opcode));
    }
    if data.len() != 12 {
        return Err(HandshakeError::BadBody(data.len()));
    }
    let mut data = data;
    let remote_public = data.get_u32_le();
    let client_key = data.get_u64_le();

    let common_secret = g_pow_x_mod_p(remote_public, params.private_exponent, params.prime);
    let handshake_bf = Blowfish::new(&calc_key(common_secret, params.local_public, remote_public))
        .map_err(|_| HandshakeError::Key)?;
    let expected = encrypt_u64(
        &handshake_bf,
        calc_challenge(common_secret, remote_public, params.local_public),
    );
    if client_key != expected {
        return Err(HandshakeError::ClientSignature {
            expected,
            received: client_key,
        });
    }

    let challenge = encrypt_u64(
        &handshake_bf,
        calc_challenge(common_secret, params.local_public, remote_public),
    );
    let response = SilkroadFrame::Packet {
        count: 0,
        crc: 0,
        opcode: 0x5000,
        encrypted: 0,
        data: challenge_body(challenge),
    };
    let buf = response
        .serialize(plain.clone())
        .map_err(|e| HandshakeError::Frame(e.to_string()))?;
    stream.write_all(&buf)?;

    let frame = read_handshake_frame(stream, &plain)?;
    match frame {
        SilkroadFrame::Packet { opcode: 0x9000, .. } => {}
        SilkroadFrame::Packet { opcode, .. } => {
            return Err(HandshakeError::UnexpectedPacket(opcode));
        }
        other => return Err(HandshakeError::Frame(other.to_string())),
    }
    println!("{} sro_peer[{leg}]: handshake accepted", stamp());

    let final_key = final_blowfish_key(params.handshake_key, common_secret);
    let state =
        SilkroadSecurityState::established(&final_key, params.sequence_seed, params.crc_seed)
            .map_err(|_| HandshakeError::Key)?
            .as_server_role();
    Ok(ServerHandshake { state })
}

// ---------------------------------------------------------------------------
// frame reading
// ---------------------------------------------------------------------------

/// One logical packet off the front of the accumulator. Trimmed port of
/// `sro_proxy.rs:627 next_packet`: this peer never has to re-emit the frames it
/// took apart (it answers instead of relaying), so the relay bookkeeping is
/// gone. A `0x600D` massive run from the client would land in `Skip`; our client
/// sends none in this phase (`packet_dump/c2s/` has no `0x600d.log`).
enum Step {
    Ready {
        opcode: u16,
        data: Bytes,
        consumed: usize,
    },
    Skip {
        consumed: usize,
    },
    Incomplete,
    Corrupt,
}

fn next_frame(buf: &mut [u8], security: &Arc<RwLock<SilkroadSecurityState>>) -> Step {
    let Some(frame_len) = SilkroadFrame::wire_len(buf) else {
        return Step::Incomplete;
    };
    // size(2) + opcode(2) + count(1) + crc(1): anything shorter is desync.
    if frame_len < 6 {
        return Step::Corrupt;
    }
    if frame_len > buf.len() {
        return Step::Incomplete;
    }
    match SilkroadFrame::parse(&mut buf[..frame_len], security.clone()) {
        Ok((_, SilkroadFrame::Packet { opcode, data, .. })) => Step::Ready {
            opcode,
            data,
            consumed: frame_len,
        },
        Ok(_) => Step::Skip {
            consumed: frame_len,
        },
        Err(_) => Step::Corrupt,
    }
}

// ---------------------------------------------------------------------------
// the two legs
// ---------------------------------------------------------------------------

/// What the client is expected to ask, and what we answer. Kept as one table so
/// the whole conversation of stage 1 is readable in one place; every body is
/// either a recorded fixture or the single built `0xA102`.
struct Peer {
    fixtures: Fixtures,
    /// The agent endpoint our own `0xA102` advertises.
    agent_advertise: (String, u16),
    /// The token the gateway handed out, so the agent leg can check that the
    /// client echoed *our* token in `0x6103` (`netcheck.rs` passes
    /// `info.agent_token` straight through).
    token: Arc<Mutex<Option<u32>>>,
    /// Session replay (`--replay`): when present, `c2s 0x7001` is answered with
    /// a recorded window instead of the five modelled world-entry frames.
    replay: Option<ReplayPlan>,
}

/// C→S opcodes of stage 1 (`docs/planning/OWN-TEST-PEER.md` §1.1, counted in
/// `packet_dump/c2s/`).
/// `0x2001 GLOBAL_MODULE_IDENTIFICATION`. **Not** in `packet_dump/`, and that is
/// not evidence of absence: the client exchanges it inside
/// `SilkroadConnection::connect` — `send_module_identification`
/// (`client/src/net/connection.rs:143-186`) writes its own `0x2001` and then does
/// a **blocking read** that insists on a `0x2001` answer before the connection is
/// handed to the ECS. That is upstream of the dump-capable receive loop, which is
/// exactly why no log exists (`packets/src/global.rs:141-145` says the same about
/// the missing S→C capture). A peer that stays silent here therefore hangs the
/// client before its first request — measured 2026-08-22: the gateway leg saw
/// `c2s 0x2001` and then nothing for 38 s.
const C2S_IDENTIFICATION: u16 = 0x2001;
const C2S_KEEPALIVE: u16 = 0x2002;
const C2S_SHARDLIST_PING: u16 = 0x6106;
const C2S_SHARDLIST: u16 = 0x6101;
const C2S_LOGIN: u16 = 0x6102;
const C2S_CAPTCHA: u16 = 0x6323;
const C2S_AGENT_LOGIN: u16 = 0x6103;
const C2S_CHAR_SELECT: u16 = 0x7007;
const C2S_CHAR_JOIN: u16 = 0x7001;
const C2S_GAME_READY: u16 = 0x3012;

// ---------------------------------------------------------------------------
// what the ORIGINAL client asks and ours does not
// ---------------------------------------------------------------------------
//
// Counted in `packet_dump/proxy/c2s/` (written by `sro_proxy` while the v1.188
// client talked through it): `0x6100`, `0x6104`, `0x2113`, `0x4000`, `0x750E`,
// `0x70EA` — none of which has a log under `packet_dump/c2s/` (our client).

/// `0x6100 PatchRequest`. The launcher blocks on the verdict: with no answer it
/// never reaches the shard list (`sro_proxy.rs:877 synthesized_patch_ok`, and
/// the captured launcher moved on to `0x6104` once answered).
const C2S_PATCH: u16 = 0x6100;
/// `0x6104 NoticeRequest`, asked 3 ms after the patch verdict and likewise
/// blocking (`sro_proxy.rs:841 synthesized_notice_empty`;
/// `packet_dump/proxy/c2s/0x6104.log`, body `16`).
const C2S_NOTICE: u16 = 0x6104;
/// `0x4000` — 40 bytes of machine GUID, the very packet after which the Evolin
/// session dies 23 ms later (`artifacts/capture/ORIGINAL-CLIENT-OPS.md` §8.3;
/// `packet_dump/proxy/c2s/0x4000.log`, 12 identical lines
/// `26007b35386537…7d`, i.e. a length-prefixed `{58e71415-c76b-11ea-8487-806e6f6e6963}`).
/// **Swallowing it is the whole point of owning the peer**: no recorded answer
/// exists anywhere in `packet_dump/`, so a peer that invents one would be
/// guessing, and a peer that closes the socket would reproduce the defect we
/// are trying to escape.
const C2S_MACHINE_GUID: u16 = 0x4000;
/// `0x750E ConsignmentListRequest` (`packets/src/lib.rs:611`), empty body, sent
/// right after world entry; the recorded answer is `0xB50E` = `0100`.
const C2S_CONSIGNMENT_LIST: u16 = 0x750E;
/// `0x70EA`, body `0000010000000000`, sent 1 ms after `0x750E`
/// (`packet_dump/proxy/c2s/0x70ea.log`). **No recorded answer exists** — the
/// reference server sent none in the same window — so it is swallowed.
const C2S_UNKNOWN_70EA: u16 = 0x70EA;
/// `0x2113` is *server*-initiated (XTrap): in both recorded sessions that
/// reached world entry the client sent `0x3012` **without** any preceding
/// `s2c 0x2113` — the 16:20 session has no `0x2113` in that hour at all
/// (`packet_dump/proxy/0x2113.log`, hours 11,12,20..05), and in the 20:09
/// session the first `0x2113` arrives at 20:17:13, seven minutes *after*
/// `c2s 0x3012` at 20:09:46.349 (`packet_dump/proxy/c2s/0x3012.log`). So the
/// peer sends none; a `c2s 0x2113` (should one arrive) falls into the
/// swallow-and-log default. [V, 2026-08-22]
const C2S_XTRAP: u16 = 0x2113;

const S2C_PATCH: u16 = 0xA100;
/// The pair the REAL gateway answers `c2s 0x6100` with — see the comment at the
/// `C2S_PATCH` arm. Recorded in `packet_dump/proxy/0x2005.log` (10 B) and
/// `packet_dump/proxy/0x6005.log` (5 B); both are plain, unencrypted frames.
const S2C_GLOBAL_STATE: u16 = 0x2005;
const S2C_PATCH_STATE: u16 = 0x6005;
const S2C_NOTICE: u16 = 0xA104;
const S2C_CONSIGNMENT_LIST: u16 = 0xB50E;
const OPCODE_MASSIVE: u16 = 0x600D;

const S2C_SHARDLIST_PING: u16 = 0xA106;
const S2C_SHARDLIST: u16 = 0xA101;
const S2C_LOGIN: u16 = 0xA102;
const S2C_CAPTCHA: u16 = 0xA323;
const S2C_AGENT_LOGIN: u16 = 0xA103;
const S2C_CHAR_SELECT: u16 = 0xB007;
const S2C_CHAR_JOIN: u16 = 0xB001;
const S2C_CHAR_DATA_BEGIN: u16 = 0x34A5;
const S2C_CHAR_DATA: u16 = 0x3013;
/// Length of the one `0x3013` body the ORIGINAL client demonstrably accepted
/// (`packet_dump/proxy/0x3013.log`, 2026-08-21T16:20:51.772Z — that session
/// rendered the world).
const PROVEN_CHAR_DATA_LEN: usize = 1229;
const S2C_CHAR_DATA_END: u16 = 0x34A6;
const S2C_CELESTIAL_POSITION: u16 = 0x3020;

/// `0x600D` **header** body `[flag=1 | amount u16 | inner opcode u16]`, and
/// `0x600D` **payload** body `[flag=0 | data]` — both ported verbatim from
/// `sro_proxy.rs:610/620`, where they are shown to re-emit a live gateway's
/// massive run byte for byte.
fn massive_header_body(amount: u16, inner_opcode: u16) -> Bytes {
    let mut body = BytesMut::with_capacity(5);
    body.put_u8(1);
    body.put_u16_le(amount);
    body.put_u16_le(inner_opcode);
    body.freeze()
}

fn massive_payload_body(inner: &Bytes) -> Bytes {
    let mut body = BytesMut::with_capacity(1 + inner.len());
    body.put_u8(0);
    body.put_slice(inner);
    body.freeze()
}

/// Wrap one recorded frame in its own MASSIVE run.
///
/// # The idea
///
/// The proxy dump marks a frame `massive: true` when it arrived *inside* a
/// `0x600D` run, and it writes the decoded inner frame. So a `massive` column of
/// `true` for two different opcodes does not mean one run carrying both — a
/// massive header names exactly one inner opcode — it means each of them arrived
/// in a run of its own. `0x2005` and `0x6005` are `massive: true` in 109 of 109
/// recorded sessions (`docs/planning/RE-PEER-GATEWAY-HANDSHAKE.md`), while
/// `0x2001`, `0xA101` and `0x2113` are `false` in the same column — that is the
/// positive control that the flag distinguishes anything at all.
fn massive_wrap(opcode: u16, body: &Bytes) -> Vec<(u16, Bytes)> {
    vec![
        (OPCODE_MASSIVE, massive_header_body(1, opcode)),
        (OPCODE_MASSIVE, massive_payload_body(body)),
    ]
}

// ---------------------------------------------------------------------------
// session replay
// ---------------------------------------------------------------------------

/// The proxy's structured recording: one JSON object per wire frame, written by
/// `tools/src/bin/sro_proxy.rs`, with the columns this replay needs — `ts`,
/// `dir`, `opcode`, `massive`, `hex`.
const DEFAULT_SESSION_LOG: &str = "packet_dump/proxy/session.jsonl";

/// The documented default window for `--replay`: the ONLY recorded session in
/// which the original v1.188 client is measured to have rendered the world
/// (`docs/re/ui/live-pregame-measurements.md` §8). This stamp is the `0x3013`
/// frame of that session; the session it belongs to starts at
/// `2026-08-21T16:20:25.380Z` (`c2s 0x2001`) and its world-entry burst runs to
/// `16:20:54.782Z` (`s2c 0xB50E`) — 28 s2c frames after `c2s 0x7001`.
const DEFAULT_REPLAY_SESSION: &str = "2026-08-21T16:20:51.772Z";

/// Session boundary marker: every session in the recording opens with the
/// client's own `0x2001` identification (`packet_dump/proxy/session.jsonl`,
/// 12 boundaries between 2026-08-20 and 2026-08-22).
const SESSION_BOUNDARY: (&str, &str) = ("c2s", "0x2001");

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum ReplayTiming {
    /// Send every frame back to back, as fast as the socket takes them.
    Fast,
    /// Keep the recorded inter-frame deltas.
    Recorded,
}

/// One recorded s2c frame, ready to be put back on the wire.
#[derive(Clone, Debug)]
struct ReplayFrame {
    ts: String,
    opcode: u16,
    body: Bytes,
    /// `true` = arrived *inside* a `0x600D` run in the recording, so it has to
    /// go back out inside a run of its own (`massive_wrap`).
    massive: bool,
    /// Recorded gap to the previous replayed frame (for the first frame: to the
    /// `c2s 0x7001` that triggers the replay).
    delta: Duration,
}

/// A window of the recording, replayed on `c2s 0x7001`.
///
/// # The idea
///
/// Stage 1 answers world entry with five *modelled* frames (`0xB001`, `0x34A5`,
/// `0x3013`, `0x34A6`, `0x3020`) and the original client then asserts 4.7 s
/// later (`8V` dump, `docs/re/ui/live-pregame-measurements.md` §8). The
/// recording says why: those five are all the reference server sent *before*
/// the client's own `0x3012 GameReady`, but the client then asks three more
/// questions (`0x4000`, `0x750E`, `0x70EA`) and the server answers with a burst
/// of ~20 more frames (`0x3809`, `0x3017/19/18` pairs, `0x303D`, `0x30BF`,
/// `0x385F`, `0xB070/71`, `0xB0BD`, `0x3206`, `0x30D0`, `0x3077`, `0x3153`,
/// `0x3305`, `0xB50E`). We do not model any of those, and inventing them is
/// exactly what ADR-0009 forbids. So the peer replays them from the one session
/// that is measured to have worked, byte for byte, in recorded order.
struct ReplayPlan {
    source: PathBuf,
    /// Inclusive window, as the two stamps actually taken from the log.
    window: (String, String),
    timing: ReplayTiming,
    /// The world-entry burst: every `s2c` frame recorded *after* the triggering
    /// `c2s 0x7001`, in recorded order.
    frames: Vec<ReplayFrame>,
    /// The world-entry burst, but cut at the client's own questions: every
    /// recorded `c2s` frame opens a segment, and the `s2c` frames until the next
    /// `c2s` are that question's answer.
    ///
    /// Why this exists [V, 2026-08-22]: the recording is a DIALOGUE, not a
    /// monologue. After `0x7001` the reference server sends five frames, and the
    /// remaining ~20 arrive only after the client has asked — `c2s 0x70EA` is
    /// answered 80 ms later with `s2c 0x3809` and then the spawn groups, in both
    /// recorded sessions (16:20:54.586 → .666, 20:09:46.361 → .402). Blasting
    /// the whole window at `0x7001` delivers those frames while the client is
    /// still loading, and a frame the client does not consume in its current
    /// state is fatal (`MsgStreamBuffer.h:186`). So: answer question by question.
    segments: Vec<(u16, Vec<ReplayFrame>)>,
    /// Every `c2s` opcode that appears inside the window, answered or not.
    ///
    /// The difference matters: if the client asked something *in the recording*
    /// and the reference server stayed silent (`0x4000`, `0x750E`), then silence
    /// is the measured answer and our modelled fixture must not fill the gap —
    /// sending `0xB50E` at `0x750E` would deliver, three frames early, a frame the
    /// recording places at the very end of the `0x70EA` burst. Opcodes that never
    /// appear as a question in the window (e.g. `0x7001`, whose answer precedes
    /// it) still fall through to the modelled path.
    questions: Vec<u16>,
    /// The *lobby* half of the same connection: every `s2c` frame recorded
    /// before that `0x7001` (`0xA103`, `0xB007`, …), answered on request.
    ///
    /// Why this exists at all — the finding that made the burst insufficient
    /// [V, 2026-08-22]: with only the burst replayed, the client picks a
    /// character from the *fixture* `0xB007` (336 B, four characters, e.g.
    /// `orlVol1`) and then receives a `0x3013` describing a **different**
    /// character (`nummer6`, from the 16:20 session — other level, position,
    /// inventory and skills). A client that checks its `CHARACTER_DATA` against
    /// the selection it made has every reason to assert, and it would assert
    /// *late*, after loading — which is what 4.7 s looks like. Replaying the
    /// list from the same session makes the chain self-consistent: the list the
    /// client chooses from is the list the `0x3013` belongs to.
    lobby: Vec<ReplayFrame>,
}

impl ReplayPlan {
    /// Everything after the last `c2s 0x7001` of the session that contains
    /// `stamp`. Session = the frames from one `c2s 0x2001` up to (excluding) the
    /// next one.
    fn for_session(path: &Path, stamp: &str, timing: ReplayTiming) -> Result<Self, String> {
        let rows = read_session_log(path)?;
        let target = rows
            .iter()
            .position(|r| r.ts == stamp)
            .ok_or_else(|| format!("{}: no frame recorded at {stamp}", path.display()))?;
        let start = rows[..=target]
            .iter()
            .rposition(|r| (r.dir.as_str(), r.opcode_hex.as_str()) == SESSION_BOUNDARY)
            .unwrap_or(0);
        let end = rows[start + 1..]
            .iter()
            .position(|r| (r.dir.as_str(), r.opcode_hex.as_str()) == SESSION_BOUNDARY)
            .map(|off| start + 1 + off)
            .unwrap_or(rows.len());
        let session = &rows[start..end];
        let join = session
            .iter()
            .rposition(|r| r.dir == "c2s" && r.opcode == C2S_CHAR_JOIN)
            .ok_or_else(|| {
                format!(
                    "{}: the session around {stamp} never reached world entry (no c2s 0x7001)",
                    path.display()
                )
            })?;
        // The lobby half starts at the *last* `0xB007` before world entry — the
        // list the client actually chose from — and not at the session start, so
        // an earlier list of the same connection (name check, create) cannot
        // shadow it. If the session has none, the plan carries no lobby and the
        // fixture answer stays in charge (logged at startup).
        let lobby_start = session[..join]
            .iter()
            .rposition(|r| r.dir == "s2c" && r.opcode == S2C_CHAR_SELECT)
            .unwrap_or(join);
        let mut plan = Self::from_rows(path, &session[join..], timing)?;
        plan.lobby = Self::plain_frames(&session[lobby_start..join])?;
        Ok(plan)
    }

    /// Every s2c frame with `from <= ts <= to`. String comparison is the right
    /// one here and not a shortcut: the dumper writes one fixed RFC-3339 shape
    /// with millisecond precision and a literal `Z`
    /// (`sro_proxy.rs`), so lexicographic order *is* chronological order.
    fn for_window(path: &Path, from: &str, to: &str, timing: ReplayTiming) -> Result<Self, String> {
        let rows = read_session_log(path)?;
        let slice: Vec<SessionRow> = rows
            .into_iter()
            .filter(|r| r.ts.as_str() >= from && r.ts.as_str() <= to)
            .collect();
        if slice.is_empty() {
            return Err(format!(
                "{}: no frame recorded in [{from} .. {to}]",
                path.display()
            ));
        }
        Self::from_rows(path, &slice, timing)
    }

    fn from_rows(path: &Path, rows: &[SessionRow], timing: ReplayTiming) -> Result<Self, String> {
        let base = rows
            .first()
            .ok_or_else(|| "empty replay window".to_string())?;
        let mut previous = parse_stamp(&base.ts)?;
        let mut frames = Vec::new();
        for row in rows.iter().filter(|r| r.dir == "s2c") {
            let at = parse_stamp(&row.ts)?;
            let delta = Duration::from_millis((at - previous).max(0) as u64);
            previous = at;
            frames.push(ReplayFrame {
                ts: row.ts.clone(),
                opcode: row.opcode,
                body: Bytes::from(row.body.clone()),
                massive: row.massive,
                delta,
            });
        }
        if frames.is_empty() {
            return Err(format!(
                "{}: the selected window contains no s2c frame",
                path.display()
            ));
        }
        let window = (frames[0].ts.clone(), frames[frames.len() - 1].ts.clone());
        // Cut the same rows into question/answer segments (see `segments`).
        let mut segments: Vec<(u16, Vec<ReplayFrame>)> = Vec::new();
        // The gap inside a segment is recorded, not decorative: after `0x70ea`
        // the original leaves 80 ms of silence before the spawn groups, and
        // that pause is the client's world load. A segment played in one
        // millisecond hands it a state a real server never produced, so the
        // delta here is measured from the question that opened the segment.
        let mut segment_previous: Option<i64> = None;
        for row in rows.iter() {
            let at = parse_stamp(&row.ts)?;
            if row.dir == "c2s" {
                segments.push((row.opcode, Vec::new()));
                segment_previous = Some(at);
            } else if let Some(last) = segments.last_mut() {
                let delta = match segment_previous {
                    Some(prev) => Duration::from_millis((at - prev).max(0) as u64),
                    None => Duration::ZERO,
                };
                segment_previous = Some(at);
                last.1.push(ReplayFrame {
                    ts: row.ts.clone(),
                    opcode: row.opcode,
                    body: Bytes::from(row.body.clone()),
                    massive: row.massive,
                    delta,
                });
            }
        }
        let questions: Vec<u16> = rows
            .iter()
            .filter(|r| r.dir == "c2s")
            .map(|r| r.opcode)
            .collect();
        segments.retain(|(_, answers)| !answers.is_empty());
        Ok(Self {
            source: path.to_path_buf(),
            window,
            timing,
            frames,
            segments,
            questions,
            lobby: Vec::new(),
        })
    }

    /// The `s2c` frames of a row range, without deltas — these are answered on
    /// request, so their recorded timing is the *client's* to reproduce.
    fn plain_frames(rows: &[SessionRow]) -> Result<Vec<ReplayFrame>, String> {
        Ok(rows
            .iter()
            .filter(|r| r.dir == "s2c")
            .map(|row| ReplayFrame {
                ts: row.ts.clone(),
                opcode: row.opcode,
                body: Bytes::from(row.body.clone()),
                massive: row.massive,
                delta: Duration::ZERO,
            })
            .collect())
    }

    /// The recorded answer of this session for one lobby opcode, if it has one.
    fn lobby_answer(&self, opcode: u16) -> Option<&ReplayFrame> {
        self.lobby.iter().rev().find(|f| f.opcode == opcode)
    }

    /// Count and *first* name of a recorded `0xB007` list, for the log line.
    ///
    /// Only the first name: after it the per-character record continues with
    /// items, avatars and masks whose length is variable
    /// (`packets/src/agent/lobby.rs`), and a wrong walk would print a wrong
    /// name. The prefix, however, is fixed — `action, result, count,
    /// ref_obj_id u32, name (u16 len + bytes)` — and that is all this needs.
    /// Verified against `packet_dump/proxy/session.jsonl` 16:20:26.269Z:
    /// `02 01 01 8b070000 0700 "nummer6"` -> (1, "nummer6").
    fn lobby_list_summary(&self) -> Option<(u8, String)> {
        let body = self.lobby_answer(S2C_CHAR_SELECT)?.body.clone();
        if body.len() < 10 || body[0] != 0x02 || body[1] != 0x01 {
            return None;
        }
        let count = body[2];
        let len = LittleEndian::read_u16(&body[7..9]) as usize;
        let name = body.get(9..9 + len)?;
        Some((count, String::from_utf8_lossy(name).to_string()))
    }

    fn describe(&self) -> String {
        let bytes: usize = self.frames.iter().map(|f| f.body.len()).sum();
        let massive = self.frames.iter().filter(|f| f.massive).count();
        let lobby = match self.lobby_list_summary() {
            Some((count, first)) => {
                format!(", lobby list of the same session: {count} character(s), first {first:?}")
            }
            None if self.lobby.is_empty() => {
                ", no lobby frames (fixture answers the lobby)".to_string()
            }
            None => format!(", {} lobby frame(s)", self.lobby.len()),
        };
        format!(
            "{} frames ({bytes} body bytes, {massive} massive) from {} [{} .. {}], timing {:?}{lobby}",
            self.frames.len(),
            self.source.display(),
            self.window.0,
            self.window.1,
            self.timing
        )
    }

    /// Put the window back on the wire.
    ///
    /// Two rules that are not cosmetic. (1) A `massive: true` frame goes out as
    /// its own `0x600D` run — header naming the inner opcode, then payload —
    /// because sending its inner opcode bare is what corrupted the run state and
    /// made the client hang up once already (see the `C2S_PATCH` arm and the
    /// nudge comment in `serve_session`). (2) Nothing else may write to this
    /// socket while the run is in flight, which is why the replay runs inline in
    /// the session loop rather than on a timer thread.
    /// Does this session have a recorded answer for that client question?
    fn has_segment(&self, opcode: u16) -> bool {
        self.segments.iter().any(|(op, _)| *op == opcode)
    }

    /// Did the client ask this *inside* the window? Then the recording decides,
    /// including when it decides "the server said nothing".
    fn was_asked(&self, opcode: u16) -> bool {
        self.questions.contains(&opcode)
    }

    /// Play the recorded answer for one client question, if the session has one.
    fn play_segment(
        &self,
        opcode: u16,
        stream: &mut TcpStream,
        security: &Arc<RwLock<SilkroadSecurityState>>,
        leg: &str,
    ) -> Result<bool, String> {
        let Some((_, answers)) = self.segments.iter().find(|(op, _)| *op == opcode) else {
            return Ok(false);
        };
        println!(
            "{} sro_peer[{leg}]: replay segment for c2s {opcode:#06x} — {} frame(s)",
            stamp(),
            answers.len()
        );
        for frame in answers {
            // The recorded gaps matter *inside* a segment too: the original
            // spreads the 19 frames after `0x70ea` over 116 ms, with 80 ms of
            // silence between `0x3809` and the spawn groups — that pause is the
            // client's world load. Sending them in one millisecond hands it a
            // state it never had on a real server, and "harmless" is a property
            // of the pair (opcode, state), not of the opcode.
            if self.timing == ReplayTiming::Recorded && !frame.delta.is_zero() {
                thread::sleep(frame.delta);
            }
            let out = if frame.massive {
                massive_wrap(frame.opcode, &frame.body)
            } else {
                vec![(frame.opcode, frame.body.clone())]
            };
            for (op, body) in out {
                send(stream, security, op, &body, leg)?;
            }
        }
        Ok(true)
    }

    fn play(
        &self,
        stream: &mut TcpStream,
        security: &Arc<RwLock<SilkroadSecurityState>>,
        leg: &str,
    ) -> Result<(), String> {
        println!(
            "{} sro_peer[{leg}]: REPLAY START — {}",
            stamp(),
            self.describe()
        );
        for (index, frame) in self.frames.iter().enumerate() {
            if self.timing == ReplayTiming::Recorded && !frame.delta.is_zero() {
                thread::sleep(frame.delta);
            }
            let out = if frame.massive {
                massive_wrap(frame.opcode, &frame.body)
            } else {
                vec![(frame.opcode, frame.body.clone())]
            };
            for (opcode, body) in out {
                send(stream, security, opcode, &body, leg)?;
            }
            if index + 1 == self.frames.len() {
                println!(
                    "{} sro_peer[{leg}]: REPLAY DONE — {} frames sent, last {:#06x} at recorded \
                     {}",
                    stamp(),
                    self.frames.len(),
                    frame.opcode,
                    frame.ts
                );
            }
        }
        Ok(())
    }
}

/// One line of `session.jsonl`, reduced to the columns a replay needs.
#[derive(Clone, Debug)]
struct SessionRow {
    ts: String,
    dir: String,
    opcode_hex: String,
    opcode: u16,
    massive: bool,
    body: Vec<u8>,
}

fn read_session_log(path: &Path) -> Result<Vec<SessionRow>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "{}: {e} — record one with `make proxy` first",
            path.display()
        )
    })?;
    let mut rows = Vec::new();
    for (number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| format!("{}:{}: {e}", path.display(), number + 1))?;
        let ts = value["ts"].as_str().unwrap_or_default().to_string();
        let dir = value["dir"].as_str().unwrap_or_default().to_string();
        let opcode_hex = value["opcode"].as_str().unwrap_or_default().to_string();
        let opcode = opcode_hex
            .strip_prefix("0x")
            .and_then(|h| u16::from_str_radix(h, 16).ok())
            .ok_or_else(|| {
                format!(
                    "{}:{}: {opcode_hex:?} is not an opcode",
                    path.display(),
                    number + 1
                )
            })?;
        // An empty body is a real recorded shape (`0x34A5`, `0x3018`, `0x3012`),
        // so a missing/empty `hex` field is data, not an error.
        let body = match value["hex"].as_str() {
            Some(h) if !h.is_empty() => unhex(h).ok_or_else(|| {
                format!("{}:{}: undecodable hex field", path.display(), number + 1)
            })?,
            _ => Vec::new(),
        };
        rows.push(SessionRow {
            ts,
            dir,
            opcode_hex,
            opcode,
            massive: value["massive"].as_bool().unwrap_or(false),
            body,
        });
    }
    if rows.is_empty() {
        return Err(format!("{}: no frames recorded", path.display()));
    }
    Ok(rows)
}

/// Milliseconds since the epoch of one dumper stamp. `chrono` is already a
/// dependency of this crate (the dumper writes with it), so this is the same
/// clock that wrote the line.
fn parse_stamp(ts: &str) -> Result<i64, String> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|t| t.timestamp_millis())
        .map_err(|e| format!("{ts:?} is not an RFC-3339 stamp: {e}"))
}

/// "You are up to date" — `0xA100` with `result = 1` and nothing else, sent as
/// a **massive** run because both wiki layouts mark `0xA100` massive and the
/// proxy's synthesised answer (which the captured launcher accepted) does the
/// same (`sro_proxy.rs:877 synthesized_patch_ok`).
fn synthesized_patch_ok() -> Vec<(u16, Bytes)> {
    let body: Bytes = Packet::from(PatchResponse {
        result: 1,
        error: None,
    })
    .into_serialize()
    .1;
    vec![
        (OPCODE_MASSIVE, massive_header_body(1, S2C_PATCH)),
        (OPCODE_MASSIVE, massive_payload_body(&body)),
    ]
}

/// The empty notice list — one byte, `noticeCount = 0`, an ordinary frame (the
/// wiki marks only `0xA100` massive). Same body and same reasoning as
/// `sro_proxy.rs:841 synthesized_notice_empty`; without it the launcher
/// keepalives forever and never offers its Start button.
fn synthesized_notice_empty() -> Vec<(u16, Bytes)> {
    let body: Bytes = Packet::from(NoticeResponse { notice_count: 0 })
        .into_serialize()
        .1;
    vec![(S2C_NOTICE, body)]
}

/// Every fixture the peer needs, with the picking rule per opcode.
fn fixture_list() -> Vec<(u16, Pick)> {
    vec![
        (S2C_SHARDLIST_PING, Pick::Last),
        (S2C_SHARDLIST, Pick::Last),
        (S2C_CAPTCHA, Pick::Last),
        (S2C_AGENT_LOGIN, Pick::Last),
        (S2C_CHAR_SELECT, Pick::Last),
        (S2C_CHAR_JOIN, Pick::Last),
        (S2C_CHAR_DATA_BEGIN, Pick::Last),
        // See `Pick::Longest`: the staged CHARACTER_DATA parser wants the most
        // complete recorded body, not the newest one.
        (S2C_CHAR_DATA, Pick::Longest),
        (S2C_CHAR_DATA_END, Pick::Last),
        (S2C_CELESTIAL_POSITION, Pick::Last),
    ]
}

/// Fixtures only the *original* client can trigger. Missing = swallow + log,
/// not a start failure: whether they exist depends on which client wrote the
/// dump, and a stage-1 run with our own client must not need them.
fn optional_fixture_list() -> Vec<(u16, Pick)> {
    vec![
        (S2C_CONSIGNMENT_LIST, Pick::Last),
        // The 0x6100 answer pair. Optional so a corpus without them still starts
        // (the C2S_PATCH arm then falls back to the synthesised 0xA100).
        (S2C_GLOBAL_STATE, Pick::Last),
        (S2C_PATCH_STATE, Pick::Last),
    ]
}

/// Opcodes whose answer is chosen per request byte, see `Fixtures::variants`.
fn variant_fixture_list() -> Vec<u16> {
    // 0x3013 is here not because it is multiplexed, but because the peer has to
    // be able to pick a *specific* recorded body (see `character_data`).
    vec![S2C_CHAR_SELECT, S2C_CHAR_DATA]
}

/// Our `0x2001` answer for a leg.
///
/// The two names are not invented: they are the ones the original client
/// classifies a peer by (`ModuleIdentification::peer_kind`, from the decompile
/// at `004ce9d0:62,91,138`) — `GatewayServer` and `AgentServer`.
///
/// The trailing zero is **not** optional here, and that cost a run to learn
/// [V, 2026-08-22 05:51Z]. `packets/src/global.rs:150-152` calls the byte
/// C→S-only because no S→C capture existed *in `packet_dump/`* — but the proxy
/// corpus has it: every recorded server answer in
/// `packet_dump/proxy/0x2001.log` is `0d00 "GatewayServer" 00` (16 bytes) or
/// `0b00 "AgentServer" 00` (14 bytes), i.e. length prefix, name, **one zero**.
/// Sending 15 bytes instead of 16 made the original client accept the
/// handshake, send its own `c2s 0x2001`, read our answer and then close the
/// socket immediately — our own client never noticed, because it reads the name
/// and ignores the rest. So: emit what the real server emits.
fn identification(leg: &str) -> Bytes {
    let module = match leg {
        "gateway" => "GatewayServer",
        _ => "AgentServer",
    };
    ModuleIdentification {
        module: module.to_owned(),
        tail: Some(0),
    }
    .into()
}

/// The default for anything the peer has no recorded answer for: **swallow it,
/// log it, keep the connection**.
///
/// This is the peer's most important behaviour, not its least. A peer that
/// answers FIN to an unknown opcode would reproduce exactly the failure we are
/// escaping (`artifacts/capture/ORIGINAL-CLIENT-OPS.md` §8.3: the Evolin session
/// dies 23 ms after `c2s 0x4000`), and then every measurement against it would
/// be measuring our own peer. `0x4000` gets its own line because it is the one
/// opcode this whole exercise exists for.
fn swallow(leg: &str, opcode: u16, body: &Bytes) -> Vec<(u16, Bytes)> {
    match opcode {
        C2S_MACHINE_GUID => println!(
            "{} sro_peer[{leg}]: SWALLOWED c2s 0x4000 machine GUID ({} bytes) — session stays \
             open, this is the opcode Evolin died on",
            stamp(),
            body.len()
        ),
        C2S_XTRAP => println!(
            "{} sro_peer[{leg}]: swallowed c2s 0x2113 XTrap ({} bytes) — we never sent a \
             challenge, so there is nothing to verify",
            stamp(),
            body.len()
        ),
        C2S_UNKNOWN_70EA => println!(
            "{} sro_peer[{leg}]: swallowed c2s 0x70ea ({} bytes) — no recorded answer exists \
             (packet_dump has no s2c line in that window)",
            stamp(),
            body.len()
        ),
        _ => println!(
            "{} sro_peer[{leg}]: no recorded answer for c2s {opcode:#06x} ({} bytes) — swallowed, \
             connection kept",
            stamp(),
            body.len()
        ),
    }
    Vec::new()
}

impl Peer {
    /// The gateway leg's answer to one client packet.
    /// The `0x3013` body to replay, preferring the one the original client is
    /// measured to accept (see the comment at `C2S_CHAR_JOIN`).
    /// `SRO_PEER_CHARDATA=longest` restores the old behaviour for a comparison run.
    fn character_data(&self) -> Bytes {
        let longest = self.fixtures.body(S2C_CHAR_DATA);
        if std::env::var("SRO_PEER_CHARDATA")
            .map(|v| v == "longest")
            .unwrap_or(false)
        {
            return longest;
        }
        match self
            .fixtures
            .variants_len(S2C_CHAR_DATA, PROVEN_CHAR_DATA_LEN)
        {
            Some(proven) => proven,
            None => longest,
        }
    }

    /// The recorded lobby answer for one s2c opcode, wrapped the way the
    /// recording had it (`massive` frames get their own `0x600D` run), or `None`
    /// when no replay is armed or the session has no such frame.
    fn replay_lobby(&self, opcode: u16) -> Option<Vec<(u16, Bytes)>> {
        let frame = self.replay.as_ref()?.lobby_answer(opcode)?;
        println!(
            "{} sro_peer[agent]: {opcode:#06x} from the replay lobby ({} bytes, recorded {})",
            stamp(),
            frame.body.len(),
            frame.ts
        );
        Some(if frame.massive {
            massive_wrap(frame.opcode, &frame.body)
        } else {
            vec![(frame.opcode, frame.body.clone())]
        })
    }

    /// Does the armed replay window already contain this s2c opcode?
    fn replay_carries(&self, opcode: u16) -> bool {
        self.replay
            .as_ref()
            .is_some_and(|plan| plan.frames.iter().any(|f| f.opcode == opcode))
    }

    fn gateway_answer(&self, opcode: u16, body: &Bytes) -> Vec<(u16, Bytes)> {
        match opcode {
            C2S_KEEPALIVE => Vec::new(),
            C2S_IDENTIFICATION => vec![(C2S_IDENTIFICATION, identification("gateway"))],
            C2S_SHARDLIST_PING => {
                // Asymmetry in the corpus, and it is worth knowing before you
                // "fix" it: our own client's dump has a `0xA106`
                // (`packet_dump/0xa106.log`, i.e. Evolin answered it), while the
                // proxy corpus — which is the ORIGINAL client's traffic — has no
                // `0xA106` file at all. So for the original client this request
                // went unanswered and it carried on regardless. Answering it is
                // therefore a candidate for why the original client hangs up
                // five seconds after the shard list, and `SRO_PEER_PING=swallow`
                // runs that experiment without breaking our own client, which is
                // the one that demonstrably wants the answer.
                if std::env::var("SRO_PEER_PING")
                    .map(|v| v == "swallow")
                    .unwrap_or(false)
                {
                    println!(
                        "{} sro_peer[gateway]: swallowed c2s 0x6106 (SRO_PEER_PING=swallow)",
                        stamp()
                    );
                    return Vec::new();
                }
                vec![(S2C_SHARDLIST_PING, self.fixtures.body(S2C_SHARDLIST_PING))]
            }
            C2S_SHARDLIST => vec![(S2C_SHARDLIST, self.fixtures.body(S2C_SHARDLIST))],
            C2S_CAPTCHA => vec![(S2C_CAPTCHA, self.fixtures.body(S2C_CAPTCHA))],
            // Original-client only, and mandatory: the launcher blocks on both
            // verdicts before it will show the shard list.
            C2S_PATCH => {
                // The real gateway answers `c2s 0x6100` with the pair `s2c 0x2005`
                // (10 B `01000104000500000002`) + `s2c 0x6005` (5 B `0300020002`),
                // 109 times out of 109 in `packet_dump/proxy/session.jsonl`, and
                // **both inside a MASSIVE run** (`massive: true` in every one of
                // them; `0x2001`/`0xA101`/`0x2113` are `false` in the same column).
                // `0xA100` never appears in that corpus at all — our proxy
                // synthesises it.
                //
                // Getting the framing wrong is what killed the original client on
                // 2026-08-22: replayed as two plain frames, it asserted 3.9 s
                // later (minidump `8V[2026-08-22 07-55-25]`, `0xC0000005` at
                // `0x00830dda`, the client's own generic assert handler). The
                // bytes were right; the wrapper was not. See
                // `docs/planning/RE-PEER-GATEWAY-HANDSHAKE.md` §8.9.
                let recorded: Vec<(u16, Bytes)> = [S2C_GLOBAL_STATE, S2C_PATCH_STATE]
                    .iter()
                    .filter_map(|opcode| self.fixtures.body_opt(*opcode).map(|b| (*opcode, b)))
                    .collect();
                // Runtime switch, because the two candidates are not equivalent
                // and only a live client can tell them apart: with the recorded
                // pair the original client sends NOTHING afterwards and drops the
                // socket on its next keepalive tick, while the synthesised
                // 0xA100 gets it to the login screen and makes it poll
                // 0x6106/0x6101. Default is therefore the one that works, and
                // `SRO_PEER_PATCH=recorded` re-runs the experiment.
                // Three answers, because the two obvious ones each fail differently
                // and only a live client tells them apart:
                //   synth    (default) 0xA100 result=1 — the client goes on to
                //                      0x6106/0x6101 and reaches the login screen.
                //   recorded           the real 0x2005+0x6005 pair — the client
                //                      accepts it but then sends NOTHING for 5 s.
                //   both               the pair first, then 0xA100. The pair is what
                //                      the real gateway sends, and `FUN_00844cd0`
                //                      (the only builder of the item seek index)
                //                      hangs off the version-check scene — so a
                //                      client that never sees the pair may never
                //                      load itemdata, which is the leading theory
                //                      for "every item lookup returns NULL"
                //                      (docs/planning/RE-PEER-GATEWAY-HANDSHAKE.md §20).
                let patch_mode = std::env::var("SRO_PEER_PATCH").unwrap_or_default();
                let want_recorded = patch_mode == "recorded" || patch_mode == "both";
                let want_synth_too = patch_mode == "both";
                if want_recorded && recorded.len() == 2 {
                    println!(
                        "{} sro_peer[gateway]: 0x6100 patch check -> recorded 0x2005 + 0x6005, each in its own massive run",
                        stamp()
                    );
                    let mut frames: Vec<(u16, Bytes)> = recorded
                        .iter()
                        .flat_map(|(opcode, body)| massive_wrap(*opcode, body))
                        .collect();
                    if want_synth_too {
                        println!(
                            "{} sro_peer[gateway]: … and the synthesised 0xA100 on top (SRO_PEER_PATCH=both)",
                            stamp()
                        );
                        frames.extend(synthesized_patch_ok());
                    }
                    frames
                } else {
                    println!(
                        "{} sro_peer[gateway]: 0x6100 patch check -> no recording, synthesised 0xA100 \"up to date\"",
                        stamp()
                    );
                    synthesized_patch_ok()
                }
            }
            C2S_NOTICE => {
                // The real gateway NEVER answers `0x6104` [V, census over
                // `packet_dump/proxy/session.jsonl`]: the original client sends it
                // unsolicited 2-3 ms after `0x6100`, and the only frames that come
                // back are the `0x2005`+`0x6005` pair — e.g. 11:34:10.984 `0x6100`,
                // .987 `0x6104`, 11:34:11.005/.006 the pair, and nothing else. In
                // 102 recorded `0x6100` exchanges there is not one `0xA104`.
                //
                // Answering it anyway is the same mistake as the `0xA106` nudge
                // that killed the client in the world scene: a frame the client
                // does not consume in its current state. And this one lands in
                // `CPSVersionCheck` — the very scene whose `FUN_00844cd0` builds
                // the item seek index (`docs/planning/RE-PEER-GATEWAY-HANDSHAKE.md`
                // §20), which is the leading suspect for "every item lookup
                // returns NULL" in the world.
                //
                // So: swallow by default, exactly like the measured server.
                // `SRO_PEER_NOTICE=synth` restores the old synthesised answer for
                // a comparison run.
                if std::env::var("SRO_PEER_NOTICE")
                    .map(|v| v == "synth")
                    .unwrap_or(false)
                {
                    println!(
                        "{} sro_peer[gateway]: 0x6104 notice request -> synthesised empty 0xA104 (SRO_PEER_NOTICE=synth)",
                        stamp()
                    );
                    return synthesized_notice_empty();
                }
                println!(
                    "{} sro_peer[gateway]: swallowed c2s 0x6104 — the real gateway never answers it (102/102)",
                    stamp()
                );
                Vec::new()
            }
            C2S_LOGIN => {
                if let Ok(Packet::LoginRequest(req)) = Packet::deserialize(C2S_LOGIN, body.clone())
                {
                    println!(
                        "{} sro_peer[gateway]: login for '{}' on shard {}",
                        stamp(),
                        req.username,
                        req.shard_id
                    );
                }
                // The only *built* packet in stage 1: it has to name our own
                // agent listener, which no recording can do for us. The token
                // is drawn fresh per login — the real gateway's is a one-shot
                // value the agent validates (`sro_proxy.rs:rewrite_login_response`),
                // so a fixed constant would be a magic number with no source.
                let token: u32 = rand::random();
                *self.token.lock().expect("token mutex") = Some(token);
                let body: Bytes = Packet::from(LoginResponse {
                    result: 1,
                    login_info: Some(LoginInfo {
                        agent_token: token,
                        agent_ip: self.agent_advertise.0.clone(),
                        agent_port: self.agent_advertise.1,
                    }),
                    login_error: None,
                    custom: None,
                })
                .into_serialize()
                .1;
                vec![(S2C_LOGIN, body)]
            }
            _ => swallow("gateway", opcode, body),
        }
    }

    /// The agent leg's answer to one client packet.
    fn agent_answer(&self, opcode: u16, body: &Bytes) -> Vec<(u16, Bytes)> {
        match opcode {
            C2S_KEEPALIVE => Vec::new(),
            C2S_IDENTIFICATION => vec![(C2S_IDENTIFICATION, identification("agent"))],
            C2S_AGENT_LOGIN => {
                if let Ok(Packet::AgentLoginRequest(req)) =
                    Packet::deserialize(C2S_AGENT_LOGIN, body.clone())
                {
                    match *self.token.lock().expect("token mutex") {
                        Some(expected) if expected == req.token => println!(
                            "{} sro_peer[agent]: token {:#010x} accepted for '{}'",
                            stamp(),
                            req.token,
                            req.username
                        ),
                        // Logged, not rejected: a token mismatch here is a
                        // finding about *our* client, and answering `0xA103`
                        // anyway keeps the run going long enough to see it.
                        other => println!(
                            "{} sro_peer[agent]: token mismatch — client sent {:#010x}, gateway \
                             issued {:?}",
                            stamp(),
                            req.token,
                            other
                        ),
                    }
                }
                match self.replay_lobby(S2C_AGENT_LOGIN) {
                    Some(answer) => answer,
                    None => vec![(S2C_AGENT_LOGIN, self.fixtures.body(S2C_AGENT_LOGIN))],
                }
            }
            // `0x7007` multiplexes on its first byte (action). Our own client
            // sends only `02` (list); the original also sends `01` (create) and
            // `04` (name check) — `packet_dump/proxy/c2s/0x7007.log`. Answer
            // with a recorded `0xB007` of the *same* action, so a name check
            // never gets a character list.
            // In replay mode the LIST comes from the same session as the
            // `0x3013`, so the client chooses from the list its character data
            // belongs to (see the `lobby` field for why that matters). Only the
            // list action (`02`) is redirected — a name check must still get a
            // name check answer.
            C2S_CHAR_SELECT
                if body.first().copied() == Some(0x02)
                    && self
                        .replay
                        .as_ref()
                        .is_some_and(|p| p.lobby_answer(S2C_CHAR_SELECT).is_some()) =>
            {
                match self.replay_lobby(S2C_CHAR_SELECT) {
                    Some(answer) => answer,
                    None => vec![(S2C_CHAR_SELECT, self.fixtures.body(S2C_CHAR_SELECT))],
                }
            }
            C2S_CHAR_SELECT => {
                let action = body.first().copied();
                match action.and_then(|a| self.fixtures.variant(S2C_CHAR_SELECT, a)) {
                    Some(answer) => vec![(S2C_CHAR_SELECT, answer)],
                    None => {
                        // Action 2 is the one stage 1 must never miss, so it
                        // keeps the plain fixture as a fallback.
                        if action == Some(0x02) {
                            vec![(S2C_CHAR_SELECT, self.fixtures.body(S2C_CHAR_SELECT))]
                        } else {
                            println!(
                                "{} sro_peer[agent]: 0x7007 action {:?} has no recorded 0xB007 — \
                                 swallowed",
                                stamp(),
                                action
                            );
                            Vec::new()
                        }
                    }
                }
            }
            // In replay mode the recorded window already carries the answer the
            // reference server gave (`0xB50E` at 16:20:54.782Z, 116 ms after the
            // client's `0x750E`), so answering from the fixture as well would put
            // the same frame on the wire twice — a duplicate is a difference from
            // the recording, and this whole mode exists to remove differences.
            C2S_CONSIGNMENT_LIST if self.replay_carries(S2C_CONSIGNMENT_LIST) => {
                println!(
                    "{} sro_peer[agent]: 0x750e — answer comes from the replay window, not the \
                     fixture",
                    stamp()
                );
                Vec::new()
            }
            C2S_CONSIGNMENT_LIST => match self.fixtures.body_opt(S2C_CONSIGNMENT_LIST) {
                Some(answer) => vec![(S2C_CONSIGNMENT_LIST, answer)],
                None => swallow("agent", opcode, body),
            },
            // World entry, in the order the reference server sent it — all four
            // stamps within the same millisecond, `0x3020` last
            // (`packet_dump/0xb001.log`, `0x34a5.log`, `0x3013.log`,
            // `0x34a6.log`, `0x3020.log`, sessions of 2026-08-16T18:46:15).
            C2S_CHAR_JOIN => vec![
                (S2C_CHAR_JOIN, self.fixtures.body(S2C_CHAR_JOIN)),
                (S2C_CHAR_DATA_BEGIN, self.fixtures.body(S2C_CHAR_DATA_BEGIN)),
                // Which `0x3013` we replay is not cosmetic — it decides whether
                // the ORIGINAL client survives the world entry [V, 2026-08-22]:
                //
                // * `Pick::Longest` (2753 B, from our own client's dump) makes it
                //   assert on the spot: dump `8V[2026-08-22 08-35-51]`.
                // * The 1229-byte body in `packet_dump/proxy/0x3013.log`
                //   (16:20:51.772Z) is the only recorded CHARACTER_DATA the
                //   original client is *measured* to have accepted — that session
                //   rendered the world and produced 2.7 s of world traffic.
                //
                // A fresh character's frame is the worst choice of all: every one
                // of them carries item id 46551, which this client's itemdata
                // cannot resolve (reproduced end to end on 2026-08-22, see
                // `docs/re/ui/live-pregame-measurements.md` §7). So prefer the
                // proven body, and only fall back to the longest one when the
                // proxy corpus is absent (our own client parses both).
                (S2C_CHAR_DATA, self.character_data()),
                (S2C_CHAR_DATA_END, self.fixtures.body(S2C_CHAR_DATA_END)),
                (
                    S2C_CELESTIAL_POSITION,
                    self.fixtures.body(S2C_CELESTIAL_POSITION),
                ),
            ],
            C2S_GAME_READY => {
                println!(
                    "{} sro_peer[agent]: client sent GameReady (0x3012) — WORLD ENTRY COMPLETE",
                    stamp()
                );
                Vec::new()
            }
            _ => swallow("agent", opcode, body),
        }
    }
}

// ---------------------------------------------------------------------------
// guild end-to-end probe (`SRO_PEER_GUILD`)
// ---------------------------------------------------------------------------
//
// Idea: two guild paths of our client are unit-tested but have never met a
// socket — the chunked record transfer (0x34B3 / 0x3101 / 0x34B4, consumed in
// `client/src/plugins/net/guild.rs`) and the live nameplate pair
// (0x30FF / 0x3100, consumed in `client/src/plugins/net/entities.rs`). This
// probe pushes them once the client is in the world, so the question the run
// answers is end-to-end ("does our client process it?") rather than "does the
// struct round-trip?".
//
// Why it is an option and not peer behaviour: no `packet_dump/0x34b3.log`,
// `0x3101.log`, `0x30ff.log` or `0x3100.log` exists, so unlike every other
// answer this peer gives, these bodies cannot be replayed — they are *built*
// from the layouts in `packets/src/agent/guild.rs` (read out of the original's
// own parsers; `docs/net-guild-0x3101.md`, `docs/re/net/inbound/guild.md`
// §0x30FF). Building a body is what this peer otherwise refuses to do
// (ADR-0009), so it stays behind a named switch that is off by default. The
// *content* below (guild name, member names, numbers) is deliberately
// recognisable probe content and claims no origin; only the field layout is
// sourced, and the layout is what is under test.
//
// 0x3102 (alliance) is deliberately NOT probed: its layout today is only read
// statically (`FUN_0099A810`), so replaying our own assumption and recognising
// it again would be circular.

/// 0x34B3 — guild record begin (`packets/src/lib.rs:446`).
const S2C_GUILD_DATA_BEGIN: u16 = 0x34B3;
/// 0x3101 — one chunk of the record (`packets/src/lib.rs:447`).
const S2C_GUILD_DATA_BODY: u16 = 0x3101;
/// 0x34B4 — record complete (`packets/src/lib.rs:448`).
const S2C_GUILD_DATA_END: u16 = 0x34B4;
/// 0x30FF — an entity's guild affiliation changed (`packets/src/lib.rs:449`).
const S2C_ENTITY_GUILD_UPDATE: u16 = 0x30FF;
/// 0x3100 — an entity is no longer in a guild (`packets/src/lib.rs:460`).
const S2C_ENTITY_GUILD_REMOVE: u16 = 0x3100;

/// One scheduled group of frames of the guild probe.
struct GuildProbeStep {
    at: Duration,
    what: &'static str,
    frames: Vec<(u16, Bytes)>,
}

/// The probe's schedule, started when the client reports world entry.
///
/// The steps are spread over seconds on purpose: sending BEGIN, both chunks and
/// END in one batch would let the client consume them in a single Bevy frame,
/// which is precisely the case the unit test already covers. A chunk that
/// arrives a second after its BEGIN is what proves the accumulator survives
/// frames, and the 0x3100 that follows its 0x30FF two seconds later is what
/// makes "the guild line disappears again" observable at all.
struct GuildProbeScript {
    started: Instant,
    steps: std::collections::VecDeque<GuildProbeStep>,
}

impl GuildProbeScript {
    fn due(&mut self) -> Option<GuildProbeStep> {
        let elapsed = self.started.elapsed();
        if self.steps.front().is_some_and(|s| elapsed >= s.at) {
            self.steps.pop_front()
        } else {
            None
        }
    }
}

/// The probe content of the chunked record. Layout: `GuildData` /
/// `GuildMember` in `packets/src/agent/guild.rs`.
fn guild_probe_record() -> Bytes {
    let member = |member_id: u32, name: &str, permissions: u32, is_master: bool| {
        packets::agent::guild::GuildMember {
            member_id,
            name: name.to_string(),
            unk_u8_01: 0,
            level: 40,
            guild_points: 0,
            permissions,
            unk_u32_01: 0,
            unk_u32_02: 0,
            unk_u32_03: 0,
            nickname: String::new(),
            model_id: 1907,
            is_master,
            is_offline: false,
        }
    };
    let members = vec![
        member(
            1,
            "PeerMaster",
            packets::agent::guild::GuildPermissions::MASTER,
            true,
        ),
        member(
            2,
            "PeerGrunt",
            packets::agent::guild::GuildPermissions::ALL,
            false,
        ),
    ];
    packets::agent::guild::GuildData {
        guild_id: 0x5052_4F42,
        name: "PeerProbeGuild".into(),
        level: 3,
        guild_points: 1234,
        notice: "probe notice".into(),
        message: "probe message".into(),
        unk_u32_00: 0,
        unk_u8_00: 0,
        member_count: members.len() as u8,
        members,
        // The record ends with a second counted list, and the peer sends it
        // empty because that is what the only live record measured so far
        // carried: the founder-only capture ends `01 01 00`, i.e.
        // `is_master = 1`, `is_offline = 1`, then a count of `0`
        // (`packets/src/agent/guild.rs:93-98` and `:127-130`). The old branch
        // read that trailing `00` as a third *member* byte (`unk_u8_02`)
        // instead; the port follows the chain's reading, which is the one with
        // the list behind it.
        election_count: 0,
        elections: Vec::new(),
    }
    .into()
}

/// The entity the nameplate probe talks about: the unique id the client gives
/// its own character. It comes out of the replayed `0x3020` body
/// (`CelestialPosition.unique_id`, `packets/src/agent/ingame.rs:58`), because
/// that is the id the GUI client writes into `NetworkId`
/// (`client/src/scenes/game_scene.rs:754`) — i.e. the only entity that is
/// guaranteed to exist in a peer session, which spawns nobody else.
fn guild_probe_entity_id(fixtures: &Fixtures) -> Option<u32> {
    let body = fixtures.body_opt(S2C_CELESTIAL_POSITION)?;
    (body.len() >= 4).then(|| LittleEndian::read_u32(&body[..4]))
}

/// Build the schedule for `SRO_PEER_GUILD`. Known modes, combinable as a
/// comma-separated list: `records`, `nameplate`, and `control` (the positive
/// control below). `None` = the switch is unset or names nothing known.
///
/// `spawn` is the `SRO_PEER_SPAWN=recorded` batch, if that switch is on: it is
/// replayed first (t+0.5 s) and, being a **foreign** entity, it is what the
/// nameplate arm then addresses instead of our own character.
fn guild_probe_script(
    spec: &str,
    fixtures: &Fixtures,
    spawn: Option<&RecordedSpawn>,
) -> Option<GuildProbeScript> {
    let modes: Vec<&str> = spec
        .split(',')
        .map(|m| m.trim())
        .filter(|m| !m.is_empty())
        .collect();
    let mut steps: Vec<GuildProbeStep> = Vec::new();

    // Half a second after world entry, and as one step: the three frames of a
    // batch belong together (the recording has all three at the same
    // millisecond), and the client's own decoder needs the begin before the
    // data (`client/src/scenes/game_scene.rs:1097-1106`, which warns and drops a
    // 0x3019 without a preceding 0x3017). The delay only keeps it clear of the
    // world-entry burst in the log.
    if let Some(spawn) = spawn {
        steps.push(GuildProbeStep {
            at: Duration::from_millis(500),
            what: "recorded foreign spawn: 0x3017 + 0x3019 + 0x3018",
            frames: vec![
                (S2C_GROUP_SPAWN_BEGIN, spawn.begin.clone()),
                (S2C_GROUP_SPAWN_DATA, spawn.data.clone()),
                (S2C_GROUP_SPAWN_END, Bytes::new()),
            ],
        });
    }

    if modes.contains(&"records") {
        let record = guild_probe_record();
        let (head, tail) = record.split_at(record.len() / 2);
        steps.push(GuildProbeStep {
            at: Duration::from_millis(1000),
            what: "guild record: 0x34B3 begin + first 0x3101 chunk",
            frames: vec![
                (S2C_GUILD_DATA_BEGIN, Bytes::new()),
                (S2C_GUILD_DATA_BODY, Bytes::copy_from_slice(head)),
            ],
        });
        steps.push(GuildProbeStep {
            at: Duration::from_millis(2000),
            what: "guild record: second 0x3101 chunk + 0x34B4 end",
            frames: vec![
                (S2C_GUILD_DATA_BODY, Bytes::copy_from_slice(tail)),
                (S2C_GUILD_DATA_END, Bytes::new()),
            ],
        });
    }

    if modes.contains(&"nameplate") {
        // The foreign entity when there is one, our own id otherwise. Which one
        // it was is printed, because it decides what the run can prove: only the
        // foreign id can reach the `remotes` query that reads `GuildTag`.
        let entity_id = spawn
            .map(|s| s.unique_id)
            .or_else(|| guild_probe_entity_id(fixtures));
        match entity_id {
            Some(entity_id) => {
                println!(
                    "{} sro_peer[agent]: nameplate probe addresses uid {} ({})",
                    stamp(),
                    entity_id,
                    if spawn.is_some() {
                        "the replayed foreign spawn"
                    } else {
                        "our own character, from the 0x3020 fixture"
                    }
                );
                let update: Bytes = packets::agent::guild::EntityGuildUpdate {
                    entity_id,
                    guild_id: 0x5052_4F42,
                    guild_name: "PeerProbeGuild".into(),
                    affiliation: Some(packets::agent::guild::EntityGuildAffiliation {
                        granted_nick: "Probe".into(),
                        crest_rev: 0,
                        union_id: 0,
                        union_crest_rev: 0,
                        fortress_position: 0,
                        relation_flag: 0,
                    }),
                }
                .into();
                steps.push(GuildProbeStep {
                    at: Duration::from_millis(3000),
                    what: "nameplate: 0x30FF sets the guild line",
                    frames: vec![(S2C_ENTITY_GUILD_UPDATE, update)],
                });
                let remove: Bytes = packets::agent::guild::EntityGuildRemove { entity_id }.into();
                steps.push(GuildProbeStep {
                    at: Duration::from_millis(6000),
                    what: "nameplate: 0x3100 clears it again",
                    frames: vec![(S2C_ENTITY_GUILD_REMOVE, remove)],
                });
            }
            None => println!(
                "{} sro_peer[agent]: SRO_PEER_GUILD=nameplate needs a 0x3020 fixture for the \
                 entity id — skipped",
                stamp()
            ),
        }
    }

    // The positive control for the nameplate arm's absence claim. 0x30FF and
    // 0x3100 have no visible consumer in a headless run (nothing there ever
    // inserts `NetworkId`, so `NetworkEntities` is empty), which makes
    // "the client decoded them" an argument from the *absence* of an error
    // line. This mode sends a deliberately truncated 0x30FF — 6 bytes, i.e. an
    // entity id and half a guild id — so the same read path is made to log
    // `network: failed to deserialize packet`
    // (`client/src/plugins/net/plugin.rs:153`) in the same run. Without it the
    // silence proves nothing.
    if modes.contains(&"control") {
        steps.push(GuildProbeStep {
            at: Duration::from_millis(8000),
            what: "positive control: a truncated 0x30FF must be logged as a decode failure",
            frames: vec![(
                S2C_ENTITY_GUILD_UPDATE,
                Bytes::from_static(&[0x01, 0x00, 0x00, 0x00, 0x02, 0x00]),
            )],
        });
    }

    (!steps.is_empty()).then(|| {
        steps.sort_by_key(|s| s.at);
        GuildProbeScript {
            started: Instant::now(),
            steps: steps.into(),
        }
    })
}

// ---------------------------------------------------------------------------
// recorded foreign spawn (`SRO_PEER_SPAWN`)
// ---------------------------------------------------------------------------
//
// Why this exists: a peer session has exactly one entity, the client's own
// character, so every probe above can only talk *about ourselves* — and the one
// path we want to see is only read for **remote** entities. `update_nameplates`
// (`client/src/plugins/hud/nameplates.rs:265-282`) carries `Option<&GuildTag>`
// in its `remotes` query; the `local` query (`:283`, `With<Player>`) has none.
// So a nameplate proof needs a FOREIGN character in the world first
// (`docs/planning/OWN-TEST-PEER.md` §7.5, point 3).
//
// And it must be a real one. A spawn is a triple — `0x3017` begin (kind +
// count), `0x3019` data (one itemdata-dependent record per entity), `0x3018`
// end — and the record layout depends on the client's own item tables
// (`client/src/net/entity_spawn.rs:497-513`), i.e. exactly the sort of body
// this peer refuses to invent. It does not have to: `packet_dump/0x3017.log`,
// `0x3019.log` and `0x3018.log` hold recorded batches of real sessions, so this
// option replays one **verbatim** and only *reads* the unique id out of it.
// Unset, the peer behaves exactly as before.

/// 0x3017 — group spawn/despawn begin (`packets/src/lib.rs:197`).
const S2C_GROUP_SPAWN_BEGIN: u16 = 0x3017;
/// 0x3018 — the (empty) end marker of the batch (`packets/src/lib.rs:198`).
const S2C_GROUP_SPAWN_END: u16 = 0x3018;
/// 0x3019 — the batch payload (`packets/src/lib.rs:199`).
const S2C_GROUP_SPAWN_DATA: u16 = 0x3019;

/// One recorded group-spawn batch, ready to be put back on the wire.
struct RecordedSpawn {
    /// The recorded `0x3017` body (`kind` + `count`), verbatim.
    begin: Bytes,
    /// The recorded `0x3019` body, verbatim.
    data: Bytes,
    /// The record's leading ref id — logged so a reader can look the character
    /// up in characterdata instead of taking the peer's word for it.
    ref_id: u32,
    /// The unique id **read out of** the record (never guessed), i.e. the id the
    /// client will file this entity under in `NetworkEntities`.
    unique_id: u32,
    /// Which line of the two logs the batch came from (1-based), so the exact
    /// bytes are findable again.
    line: usize,
}

/// The unique id inside a recorded **player** spawn record, plus its ref id and
/// the region that follows the id — or `None` when the bytes do not parse under
/// that layout. Never guesses: every step below is a field of the record.
///
/// The walk (field names from `client/src/net/entity_spawn.rs:parse_player`,
/// offsets verified byte for byte against `packet_dump/0x3019.log` line 904,
/// `2026-08-21T12:44:12.043Z`, 141 B — the GUI character `1234` seen by a bot):
///
/// ```text
///  0  u32  ref_id                                     73070000 = 1907
///  4  u8×5 scale, hwan, pvp_cape, auto_invest,
///          inventory_size                             22 00 00 01 6d (=109)
///  9  u8   inventory item count                       07
/// 10       count × (u32 ref id + u8 opt level)        7 × 5 B
/// 45  u8   avatar list size                           05
/// 46  u8   avatar item count                          00
/// 47  u8   has_mask                                   00
/// 48  u32  unique_id                                  29aa0200 = 174633
/// 52  u16  region                                     a860 = 24744 (Jangan)
/// ```
///
/// The 5-byte item stride is the same assumption `blanked_avatar_list` already
/// makes above (the opt-level byte is written only for equipment, and the
/// equipment a spawn record lists *is* equipment); it holds for both recorded
/// bodies this file knows. The checks around it are what keeps a mis-parse from
/// becoming an invented number: an NPC or monster record walked as a player
/// lands on an implausible inventory size or on region 0, and is rejected
/// instead of yielding a plausible-looking id.
fn player_spawn_unique_id(record: &[u8]) -> Option<(u32, u32, u16)> {
    const HEAD: usize = 4 + 4; // ref id + scale/hwan/pvp_cape/auto_invest
    let ref_id = record.get(..4).map(LittleEndian::read_u32)?;
    let inventory_size = *record.get(HEAD)? as usize;
    // 45 is the starting inventory of a vSRO character and 109 the maximum; a
    // record whose byte 8 is outside that range was not a player record.
    if !(45..=109).contains(&inventory_size) {
        return None;
    }
    let item_count = *record.get(HEAD + 1)? as usize;
    if item_count > inventory_size {
        return None;
    }
    let avatar_at = HEAD + 2 + item_count * 5;
    let avatar_size = *record.get(avatar_at)? as usize;
    let avatar_count = *record.get(avatar_at + 1)? as usize;
    if avatar_size > 32 || avatar_count > avatar_size {
        return None;
    }
    let unique_at = avatar_at + 2 + avatar_count * 5 + 1; // + has_mask
    let unique_id = record
        .get(unique_at..unique_at + 4)
        .map(LittleEndian::read_u32)?;
    let region = record
        .get(unique_at + 4..unique_at + 6)
        .map(LittleEndian::read_u16)?;
    (unique_id != 0 && region != 0).then_some((ref_id, unique_id, region))
}

/// Pick a recorded foreign spawn out of the dump directory.
///
/// Pairing: the dumper writes one line per received packet
/// (`client/src/plugins/net/packet_dump.rs`), and a batch contributes exactly
/// one line to each of the three logs, so line *i* of `0x3017.log` and line *i*
/// of `0x3019.log` belong to the same batch. Equal line counts are the check
/// that no batch lost its partner (989/989 on 2026-08-24); if they differ the
/// pairing basis is gone and this returns an error rather than a guess.
///
/// Chosen is the **newest** batch that is a spawn (`kind == GROUP_SPAWN`) of
/// exactly one record whose record parses as a player: one record keeps the
/// replayed payload interpretable, and a player is the kind whose nameplate
/// carries a guild line at all.
fn recorded_foreign_spawn(dir: &Path) -> Result<RecordedSpawn, String> {
    let begin_path = find_log(dir, S2C_GROUP_SPAWN_BEGIN).ok_or_else(|| {
        format!(
            "{}: no such file",
            log_path(dir, S2C_GROUP_SPAWN_BEGIN).display()
        )
    })?;
    let data_path = find_log(dir, S2C_GROUP_SPAWN_DATA).ok_or_else(|| {
        format!(
            "{}: no such file",
            log_path(dir, S2C_GROUP_SPAWN_DATA).display()
        )
    })?;
    let begins = read_bodies(&begin_path).map_err(|e| format!("{}: {e}", begin_path.display()))?;
    let datas = read_bodies(&data_path).map_err(|e| format!("{}: {e}", data_path.display()))?;
    if begins.len() != datas.len() {
        return Err(format!(
            "{} has {} lines but {} has {} — no line-by-line pairing possible",
            begin_path.display(),
            begins.len(),
            data_path.display(),
            datas.len()
        ));
    }
    for (i, (begin, data)) in begins.iter().zip(datas.iter()).enumerate().rev() {
        if begin.len() < 3 || begin[0] != packets::agent::ingame::GROUP_SPAWN {
            continue;
        }
        if LittleEndian::read_u16(&begin[1..3]) != 1 {
            continue;
        }
        if let Some((ref_id, unique_id, region)) = player_spawn_unique_id(data) {
            println!(
                "sro_peer: recorded foreign spawn from {} line {} — ref {} uid {} region {} ({} B)",
                data_path.display(),
                i + 1,
                ref_id,
                unique_id,
                region,
                data.len()
            );
            return Ok(RecordedSpawn {
                begin: Bytes::from(begin.clone()),
                data: Bytes::from(data.clone()),
                ref_id,
                unique_id,
                line: i + 1,
            });
        }
    }
    Err(format!(
        "{}: no recorded single-record player spawn in {} batches",
        data_path.display(),
        begins.len()
    ))
}

/// Serve one connected client on one leg until it closes.
fn serve_session(mut stream: TcpStream, leg: &'static str, peer: Arc<Peer>) {
    if let Err(e) = stream.set_nodelay(true) {
        println!("{} sro_peer[{leg}]: set_nodelay failed: {e}", stamp());
    }
    let handshake = match serve_handshake(&mut stream, leg) {
        Ok(h) => h,
        Err(e) => {
            println!("{} sro_peer[{leg}]: handshake failed: {e}", stamp());
            return;
        }
    };
    let security = Arc::new(RwLock::new(handshake.state));
    // A read timeout only exists so the loop can notice a shutdown; there is
    // nothing to do on a quiet socket, the client keepalives on its own.
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));

    let mut pending: Vec<u8> = Vec::new();
    let mut chunk = [0u8; READ_CHUNK];
    let trace_all = std::env::var("SRO_PEER_TRACE")
        .map(|v| v == "1")
        .unwrap_or(false);
    // The nudge is a probe everywhere except in replay mode, where it is
    // *required* behaviour: the client hangs up 5.00 s after our last frame, and
    // a replay ends with nothing left to say. It must not start earlier than the
    // last replayed frame though — a nudge in the middle of a `0x600D` run
    // corrupts the run and costs the whole run (measured 2026-08-22).
    // `nudge_off` is gone on purpose: the nudge is never armed *after* a replay
    // any more (see the `C2S_CHAR_JOIN` arm), so there is nothing left to opt out
    // of in replay mode. `SRO_PEER_NUDGE=1` still arms it for the login phase,
    // which is the one place it is measured to be both needed and harmless.
    let nudge_env = std::env::var("SRO_PEER_NUDGE").ok();
    let replay_mode = leg == "agent" && peer.replay.is_some();
    // `SRO_PEER_NUDGE=gateway` arms the keep-alive on the gateway leg only.
    // Measured 2026-08-22: the login phase dies after exactly 5 s without it, but
    // on the AGENT leg the nudge puts four `0xA106` on the wire that the recorded
    // reference session never carried — a diff of the two s2c streams shows them
    // as the only surplus frames before `0xB001`. Since "harmless" is a property
    // of the pair (opcode, state), the leg that needs the nudge and the leg that
    // must not have it are separated here rather than traded off.
    let mut nudge = match nudge_env.as_deref() {
        Some("1") => true,
        Some("gateway") => leg == "gateway",
        _ => false,
    };
    let mut last_sent_at: Option<Instant> = None;
    // `SRO_PEER_GUILD=records|nameplate|control` (comma-separated): armed on
    // the agent leg when the client reports world entry, see
    // `guild_probe_script`. Unset = the peer behaves exactly as before.
    let guild_probe = std::env::var("SRO_PEER_GUILD").unwrap_or_default();
    // `SRO_PEER_SPAWN=recorded`: replay one recorded group-spawn batch after
    // world entry, so the session contains a foreign entity at all (see
    // `recorded_foreign_spawn`). Unset = nothing extra goes on the wire.
    let spawn_mode = std::env::var("SRO_PEER_SPAWN").unwrap_or_default();
    let mut guild_script: Option<GuildProbeScript> = None;
    loop {
        // Driven at the top of the loop rather than in the read-timeout arm:
        // the schedule must keep running while the client is chatty (its own
        // 0x2002 every 5 s would otherwise starve it of timeouts).
        if let Some(script) = guild_script.as_mut() {
            while let Some(step) = script.due() {
                println!("{} sro_peer[{leg}]: guild probe — {}", stamp(), step.what);
                for (out_opcode, body) in step.frames {
                    if let Err(e) = send(&mut stream, &security, out_opcode, &body, leg) {
                        println!("{} sro_peer[{leg}]: send failed: {e}", stamp());
                        return;
                    }
                    last_sent_at = Some(Instant::now());
                }
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) => {
                println!("{} sro_peer[{leg}]: client closed the connection", stamp());
                return;
            }
            Ok(read) => pending.extend_from_slice(&chunk[..read]),
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                // Experiment E2 (`SRO_PEER_NUDGE=1`): keep the line warm.
                //
                // The original client hangs up exactly 5.00 s after our last
                // answer without sending anything first — no keepalive, no
                // second poll (measured 2026-08-22 with `SRO_PEER_TRACE=1`). The
                // two remaining explanations are an IDLE timer (5000 ms is
                // written into a session field at `0x004a024d`) or a one-shot
                // timer that the `0xA101` handler failed to re-arm. Re-sending
                // the last frame every two seconds separates them: if the FIN
                // disappears it is idle-driven, if it still arrives 5 s after the
                // FIRST answer the frame itself was rejected. Off by default —
                // this is a probe, not peer behaviour.
                if nudge {
                    if let Some(at) = last_sent_at {
                        if at.elapsed() >= Duration::from_secs(2) {
                            // Nudge with a COMPLETE, harmless, plain frame, never
                            // with the last one: the first version of this probe
                            // resent whatever went out last, which right after the
                            // patch answer is a bare `0x600D` **payload** frame —
                            // a payload without a header, i.e. we corrupted the
                            // massive run state ourselves and the client hung up
                            // 101 ms after the next frame. Lesson: a probe must
                            // not be able to break the thing it measures.
                            let body = peer.fixtures.body_opt(S2C_SHARDLIST_PING);
                            if let Some(body) = body {
                                println!("{} sro_peer[{leg}]: nudge — 0xa106", stamp());
                                if send(&mut stream, &security, S2C_SHARDLIST_PING, &body, leg)
                                    .is_err()
                                {
                                    return;
                                }
                                last_sent_at = Some(Instant::now());
                            }
                        }
                    }
                }
            }
            Err(e) => {
                println!("{} sro_peer[{leg}]: read error: {e}", stamp());
                return;
            }
        }

        loop {
            match next_frame(&mut pending, &security) {
                Step::Ready {
                    opcode,
                    data,
                    consumed,
                } => {
                    pending.drain(..consumed);
                    // Keepalives are hidden by default (7818 of them in the
                    // corpus would drown every other line), but hiding them made
                    // one question unanswerable: does the original client even
                    // send its 5-second `0x2002` before it hangs up? So
                    // `SRO_PEER_TRACE=1` shows every frame, length 0 included.
                    if opcode != C2S_KEEPALIVE || trace_all {
                        println!(
                            "{} sro_peer[{leg}]: c2s {opcode:#06x} ({} bytes)",
                            stamp(),
                            data.len()
                        );
                    }
                    // Replay mode takes world entry over completely: the
                    // modelled five-frame answer is what leaves the original
                    // client asserting 4.7 s later, and mixing the two would
                    // duplicate `0xB001`/`0x3013`.
                    if replay_mode
                        && (opcode == C2S_CHAR_JOIN
                            || peer.replay.as_ref().is_some_and(|p| p.has_segment(opcode)))
                    {
                        let plan = peer.replay.as_ref().expect("replay_mode implies a plan");
                        // Request-driven by default: the recording is a dialogue,
                        // and delivering an answer before its question is what a
                        // client cannot consume (`MsgStreamBuffer.h:186`).
                        // `SRO_PEER_REPLAY=burst` restores the old "blast the whole
                        // window at 0x7001" behaviour for a comparison run.
                        let burst = std::env::var("SRO_PEER_REPLAY")
                            .map(|v| v == "burst")
                            .unwrap_or(false);
                        let result = if burst {
                            plan.play(&mut stream, &security, leg).map(|_| true)
                        } else {
                            plan.play_segment(opcode, &mut stream, &security, leg)
                        };
                        match result {
                            Ok(true) => {}
                            Ok(false) if plan.was_asked(opcode) => {
                                println!(
                                    "{} sro_peer[{leg}]: c2s {opcode:#06x} — the recording shows no answer here, staying silent",
                                    stamp()
                                );
                                continue;
                            }
                            Ok(false) => {
                                // Never asked inside the window: fall through to
                                // the modelled path below.
                                let answers = match leg {
                                    "gateway" => peer.gateway_answer(opcode, &data),
                                    _ => peer.agent_answer(opcode, &data),
                                };
                                for (out_opcode, body) in answers {
                                    if let Err(e) =
                                        send(&mut stream, &security, out_opcode, &body, leg)
                                    {
                                        println!("{} sro_peer[{leg}]: send failed: {e}", stamp());
                                        return;
                                    }
                                    last_sent_at = Some(Instant::now());
                                }
                                continue;
                            }
                            Err(e) => {
                                println!("{} sro_peer[{leg}]: replay failed: {e}", stamp());
                                return;
                            }
                        }
                        last_sent_at = Some(Instant::now());
                        // The nudge STOPS at world entry, it does not start there
                        // [V, 2026-08-22, dump `8V[2026-08-22 08-38-18]`]: the
                        // 0xA106 keep-warm frame is what killed the client, not a
                        // slow load. The assert is `MsgStreamBuffer.h:186`
                        // ("read less than was written"), class `CPSMission`,
                        // **MSGID 0xA106, R=0, W=6** — the client built a message
                        // object and then nobody consumed its body in that state.
                        // Timeline: join 06:38:13.297, nudge :15.633, nudge
                        // :17.640, assert :18.342.
                        //
                        // The rule this proves is wider than one opcode: any s2c
                        // frame the client does not consume *in its current state*
                        // is fatal, whatever it contains. "Harmless frame" is not
                        // a property of an opcode, it is a property of the pair
                        // (opcode, state) — the same 0xA106 passed three times
                        // during login. After the join the client drives the
                        // socket itself (0x4000, 0x3012, its own 0x2002 every 5 s),
                        // and any traffic resets the idle timer, so nothing needs
                        // to be nudged.
                        nudge = false;
                        continue;
                    }
                    let answers = match leg {
                        "gateway" => peer.gateway_answer(opcode, &data),
                        _ => peer.agent_answer(opcode, &data),
                    };
                    for (out_opcode, body) in answers {
                        if let Err(e) = send(&mut stream, &security, out_opcode, &body, leg) {
                            println!("{} sro_peer[{leg}]: send failed: {e}", stamp());
                            return;
                        }
                        last_sent_at = Some(Instant::now());
                    }
                    // World entry is the earliest point at which a guild push
                    // means anything: before it the client has no character and
                    // no entity index to resolve an id against.
                    if leg == "agent"
                        && opcode == C2S_GAME_READY
                        && guild_script.is_none()
                        && !(guild_probe.trim().is_empty() && spawn_mode.trim().is_empty())
                    {
                        // A spawn mode other than the one implemented is a typo,
                        // not a silent no-op: say so and replay nothing.
                        let spawn = match spawn_mode.trim() {
                            "" => None,
                            "recorded" => match recorded_foreign_spawn(&peer.fixtures.dir) {
                                Ok(spawn) => {
                                    println!(
                                        "{} sro_peer[agent]: SRO_PEER_SPAWN=recorded — replaying \
                                         0x3017/0x3019/0x3018 line {} (ref {}, uid {})",
                                        stamp(),
                                        spawn.line,
                                        spawn.ref_id,
                                        spawn.unique_id
                                    );
                                    Some(spawn)
                                }
                                Err(e) => {
                                    println!(
                                        "{} sro_peer[agent]: SRO_PEER_SPAWN=recorded — no usable \
                                         recording: {e}",
                                        stamp()
                                    );
                                    None
                                }
                            },
                            other => {
                                println!(
                                    "{} sro_peer[agent]: SRO_PEER_SPAWN={other:?} is unknown \
                                     (expected `recorded`) — ignored",
                                    stamp()
                                );
                                None
                            }
                        };
                        guild_script =
                            guild_probe_script(&guild_probe, &peer.fixtures, spawn.as_ref());
                        if guild_script.is_some() {
                            println!(
                                "{} sro_peer[agent]: probe armed (SRO_PEER_GUILD={}, \
                                 SRO_PEER_SPAWN={})",
                                stamp(),
                                guild_probe.trim(),
                                spawn_mode.trim()
                            );
                        }
                    }
                }
                Step::Skip { consumed } => {
                    println!(
                        "{} sro_peer[{leg}]: skipped a non-packet frame ({consumed} bytes)",
                        stamp()
                    );
                    pending.drain(..consumed);
                }
                Step::Incomplete => break,
                Step::Corrupt => {
                    println!(
                        "{} sro_peer[{leg}]: undecodable frame, dropping the session ({} pending \
                         bytes: {})",
                        stamp(),
                        pending.len(),
                        hex(&pending[..pending.len().min(32)])
                    );
                    return;
                }
            }
        }
    }
}

/// Write one plaintext S→C frame.
///
/// Plaintext, and that is a decision with a source: the reference server's own
/// S→C frames are mixed, and our client accepts plaintext ones — `0xA323` and
/// `0xB001` are logged with the `P` flag in `packet_dump/` (the dumper writes
/// the frame's wire `0x8000` bit, `client/src/plugins/net/packet_dump.rs`). So
/// encryption buys this peer nothing and costs a failure mode.
fn send(
    stream: &mut TcpStream,
    security: &Arc<RwLock<SilkroadSecurityState>>,
    opcode: u16,
    body: &Bytes,
    leg: &str,
) -> Result<(), String> {
    let frame = SilkroadFrame::Packet {
        count: 0,
        crc: 0,
        opcode,
        encrypted: 0,
        data: body.clone(),
    };
    let buf = frame
        .serialize(security.clone())
        .map_err(|e| e.to_string())?;
    stream.write_all(&buf).map_err(|e| e.to_string())?;
    println!(
        "{} sro_peer[{leg}]: s2c {opcode:#06x} ({} body bytes)",
        stamp(),
        body.len()
    );
    Ok(())
}

fn listen(addr: String, leg: &'static str, peer: Arc<Peer>) {
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("sro_peer[{leg}]: cannot bind {addr}: {e}");
            return;
        }
    };
    println!("{} sro_peer[{leg}]: listening on {addr}", stamp());
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let peer = peer.clone();
                let who = stream
                    .peer_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|_| "?".into());
                println!("{} sro_peer[{leg}]: client connected from {who}", stamp());
                thread::spawn(move || serve_session(stream, leg, peer));
            }
            Err(e) => eprintln!("sro_peer[{leg}]: accept failed: {e}"),
        }
    }
}

/// Minimal flag parsing. `clap` 2 is in the crate already, but four optional
/// strings do not need a builder — and a hand-rolled parser cannot silently
/// reinterpret an unknown flag as a positional argument.
///
/// Each option takes a list of accepted spellings: the first is the canonical
/// one (`--gateway-listen`, matching `sro_proxy`'s `--gateway-listen`), the rest
/// are the names this binary shipped with in §7.2 and keeps working, because a
/// documented command line that silently stops parsing is worse than a synonym.
fn arg(args: &[String], names: &[&str], default: &str) -> String {
    args.windows(2)
        .find(|w| names.contains(&w[0].as_str()))
        .map(|w| w[1].clone())
        .unwrap_or_else(|| default.to_string())
}

/// Split `host:port`, or take the port from the leg we listen on when only a
/// host was given: `--advertise-host <lan-ip>` then means "the same port I
/// am listening on, but under the address the *client* can reach me at". That
/// distinction is the whole reason the flag exists — the original client on a
/// second machine resolves the host its `divisioninfo` names to *this* machine
/// (hosts file), so the redirect in `0xA102` must not say `127.0.0.1`.
///
/// Every address in this file is a placeholder (`<lan-ip>`, `peer.example.com`).
/// That is not cosmetic: the publication gate `scripts/check_no_private.py`
/// refuses a literal endpoint in a patch, and a LAN address is one.
fn split_endpoint(advertise: &str, listen: &str) -> Result<(String, u16), String> {
    if let Some((host, port)) = advertise.rsplit_once(':') {
        let port = port
            .parse::<u16>()
            .map_err(|e| format!("{advertise:?} has no valid port: {e}"))?;
        return Ok((host.to_string(), port));
    }
    let port = listen
        .rsplit_once(':')
        .and_then(|(_, p)| p.parse::<u16>().ok())
        .ok_or_else(|| format!("{advertise:?} has no port and {listen:?} has none either"))?;
    Ok((advertise.to_string(), port))
}

/// Build the replay plan from the command line, or `None` for the default
/// (modelled) world entry. `--replay` without a stamp means the documented
/// default session; an explicit `--replay-from/--replay-to` pair wins over it.
/// Rewrite an entity-spawn body so that its avatar-item list is empty.
///
/// Returns `(new_body, removed_count)`, or `None` when the body does not parse
/// as an entity spawn under the layout in `--replay-blank-avatar` (short body,
/// implausible list sizes, or a list that runs past the end). Never guesses.
fn blanked_avatar_list(body: &[u8]) -> Option<(Vec<u8>, usize)> {
    const HEAD: usize = 4 + 4; // refObjId + scale/berserk/pvpCape/autoInvest
    if body.len() < HEAD + 2 {
        return None;
    }
    let inv_count = body[HEAD + 1] as usize;
    let avatar_at = HEAD + 2 + inv_count * 5;
    if avatar_at + 2 > body.len() {
        return None;
    }
    let avatar_size = body[avatar_at] as usize;
    let avatar_count = body[avatar_at + 1] as usize;
    // The recorded corpus has inventory size 109 and avatar size 5; anything
    // outside a sane slot count means we mis-parsed and must not edit.
    if avatar_size > 32 || avatar_count > avatar_size {
        return None;
    }
    let list_end = avatar_at + 2 + avatar_count * 5;
    if list_end > body.len() {
        return None;
    }
    if avatar_count == 0 {
        return Some((body.to_vec(), 0));
    }
    let mut out = Vec::with_capacity(body.len() - avatar_count * 5);
    out.extend_from_slice(&body[..avatar_at + 1]);
    out.push(0);
    out.extend_from_slice(&body[list_end..]);
    Some((out, avatar_count))
}

fn replay_plan(args: &[String]) -> Result<Option<ReplayPlan>, String> {
    let log = PathBuf::from(arg(args, &["--session-log"], DEFAULT_SESSION_LOG));
    let timing = match arg(args, &["--replay-timing"], "fast").as_str() {
        "fast" => ReplayTiming::Fast,
        "recorded" => ReplayTiming::Recorded,
        other => return Err(format!("--replay-timing {other:?}: expected fast|recorded")),
    };
    let from = args
        .windows(2)
        .find(|w| w[0] == "--replay-from")
        .map(|w| w[1].clone());
    let to = args
        .windows(2)
        .find(|w| w[0] == "--replay-to")
        .map(|w| w[1].clone());
    match (from, to) {
        (Some(from), Some(to)) => {
            return ReplayPlan::for_window(&log, &from, &to, timing).map(Some);
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err("--replay-from and --replay-to come as a pair".to_string());
        }
        (None, None) => {}
    }
    let session = args
        .windows(2)
        .find(|w| w[0] == "--replay-session")
        .map(|w| w[1].clone());
    let plan = match session {
        Some(ts) => ReplayPlan::for_session(&log, &ts, timing).map(Some),
        None if args.iter().any(|a| a == "--replay") => {
            ReplayPlan::for_session(&log, DEFAULT_REPLAY_SESSION, timing).map(Some)
        }
        None => Ok(None),
    }?;
    // Two ways to take frames out of a replayed burst, and the second one exists
    // because the first one broke the recording [V, 2026-08-22]:
    //
    // `--replay-skip <opcodes>` drops whole TRANSACTIONS, not single frames. A
    // group spawn is a triple — `0x3017` opens (3 B: type + count), `0x3019`
    // carries the entities, `0x3018` closes — and dropping only the data frame
    // left the client with the announcement and nothing to read: assert
    // `MsgStreamBuffer.h:247`, MSGID 0x3019, **R=3 W=3 T=4** (it had consumed the
    // three announcement bytes and then asked for a dword that never came).
    // Same shape as the `0x600D` payload-without-header mistake: begin/data/end
    // triples and massive runs are indivisible.
    //
    // `--replay-drop-item <refid>` is the oracle-driven form: the client's crash
    // names the item ref id it cannot resolve (EAX 0x774 / fault 0x7D5, id at
    // esp+8), so we drop exactly the transaction whose body carries that id and
    // keep every other one. That is how ref id 9375 was traced to the 201-byte
    // `0x3019` at 16:20:54.726Z while the 47-byte spawn right after it — free of
    // that id — stays in the replay.
    let skip = args
        .windows(2)
        .find(|w| w[0] == "--replay-skip")
        .map(|w| w[1].clone())
        .unwrap_or_default();
    let skip: Vec<u16> = skip
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            let s = s.trim();
            let hex = s.trim_start_matches("0x").trim_start_matches("0X");
            u16::from_str_radix(hex, 16)
                .map_err(|_| format!("--replay-skip {s:?}: not a hex opcode"))
        })
        .collect::<Result<_, _>>()?;
    // `--replay-fix-item <old>:<new>` — the other half of the oracle loop.
    //
    // Dropping the transaction works, but it can cost more than it fixes
    // [V, 2026-08-22 07:18Z]: with the 201-byte group spawn removed the client
    // stopped short of `GameReady` entirely (no `0x4000`, no `0x3012`, one
    // keepalive, then the 5 s idle close), so that spawn carries something the
    // world entry needs. Rewriting the one unresolvable ref id keeps the
    // transaction intact instead. It is a SYNTHESIS, not a recording — the log
    // line says so on every run.
    let fix_items = args
        .windows(2)
        .find(|w| w[0] == "--replay-fix-item")
        .map(|w| w[1].clone())
        .unwrap_or_default();
    let fix_items: Vec<(u32, u32)> = fix_items
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            let (a, b) = s
                .trim()
                .split_once(':')
                .ok_or_else(|| format!("--replay-fix-item {s:?}: expected <old>:<new>"))?;
            Ok((
                a.parse::<u32>().map_err(|_| format!("bad ref id {a:?}"))?,
                b.parse::<u32>().map_err(|_| format!("bad ref id {b:?}"))?,
            ))
        })
        .collect::<Result<_, String>>()?;
    let drop_items = args
        .windows(2)
        .find(|w| w[0] == "--replay-drop-item")
        .map(|w| w[1].clone())
        .unwrap_or_default();
    let drop_items: Vec<u32> = drop_items
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            let s = s.trim();
            s.parse::<u32>()
                .map_err(|_| format!("--replay-drop-item {s:?}: not a decimal ref id"))
        })
        .collect::<Result<_, _>>()?;
    // `--replay-blank-avatar` — the structural counterpart to `--replay-fix-item`.
    //
    // Measured on 2026-08-22 (RE-PEER-GATEWAY-HANDSHAKE.md §19): the client dies
    // in the AVATAR-inventory loop of a foreign entity spawn, and it dies there
    // no matter WHICH ref id sits in that slot — `9375` and the substituted,
    // definitely-present `3632` produce the identical fault (EIP 0x9d13f8,
    // EAX 0x774, fault 0x7D5, EBX 0). So the interesting experiment is not
    // another id but NO avatar item at all: keep the spawn transaction and every
    // other field intact and set the avatar item count to zero, removing exactly
    // the `count * 5` bytes of that list. The layout is the one the client itself
    // confirmed (it read the count byte at body offset 51 and the ref id at 52):
    //   u32 refObjId | u8 scale | u8 berserk | u8 pvpCape | u8 autoInvest
    //   u8 invSize | u8 invCount | invCount * (u32 refId + u8 plus)
    //   u8 avatarSize | u8 avatarCount | avatarCount * (u32 refId + u8 plus)
    // Only bodies that parse consistently are touched; anything else is left
    // alone and logged, because a half-understood body must not be rewritten.
    let blank_avatar = args.iter().any(|a| a == "--replay-blank-avatar");
    Ok(plan.map(|mut plan| {
        if blank_avatar {
            for f in plan.frames.iter_mut() {
                if f.opcode != 0x3019 {
                    continue;
                }
                match blanked_avatar_list(&f.body) {
                    Some((body, removed)) => {
                        println!(
                            "sro_peer: SYNTHESIS — dropped {removed} avatar item(s) from {:#06x} \
                             ({} B -> {} B, recorded {})",
                            f.opcode,
                            f.body.len(),
                            body.len(),
                            f.ts
                        );
                        f.body = Bytes::from(body);
                    }
                    None => println!(
                        "sro_peer: --replay-blank-avatar: {:#06x} ({} B, recorded {}) does not \
                         parse as an entity spawn — left untouched",
                        f.opcode,
                        f.body.len(),
                        f.ts
                    ),
                }
            }
        }
        // Same trap as the drop filter, and it cost two more runs
        // [V, 2026-08-22 18:45Z/18:47Z]: rewriting only `frames` leaves the
        // request/answer `segments` untouched, and since the world entry is
        // answered FROM THE SEGMENTS, the peer kept sending the original body
        // while logging "SYNTHESIS — rewrote 1 occurrence(s)". Two skill-id runs
        // looked like they had disproved the misalignment theory; both dumps
        // still carried the ORIGINAL value 9375 at `esp+8`. So: rewrite every
        // list the peer can send from, and say how many lists were touched.
        let mut targets: Vec<&mut Vec<ReplayFrame>> = vec![&mut plan.frames];
        for (_, answers) in plan.segments.iter_mut() {
            targets.push(answers);
        }
        for (old, new) in &fix_items {
            for f in targets.iter_mut().flat_map(|t| t.iter_mut()) {
                let mut body = f.body.to_vec();
                let mut hits = 0;
                let mut i = 0;
                while i + 4 <= body.len() {
                    if u32::from_le_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]]) == *old {
                        body[i..i + 4].copy_from_slice(&new.to_le_bytes());
                        hits += 1;
                        i += 4;
                    } else {
                        i += 1;
                    }
                }
                if hits > 0 {
                    println!(
                        "sro_peer: SYNTHESIS — rewrote {hits} occurrence(s) of item ref {old} to {new} in {:#06x} ({} B, recorded {})",
                        f.opcode,
                        body.len(),
                        f.ts
                    );
                    f.body = Bytes::from(body);
                }
            }
        }
        if !skip.is_empty() || !drop_items.is_empty() {
            // BOTH lists have to be filtered, and forgetting the second one cost
            // a run [V, 2026-08-22 17:16Z]: the burst list was cleaned, the
            // request/answer segments were not, so the peer still replayed the
            // 201-byte `0x3019` at `c2s 0x70EA` and the client died on ref id
            // 9375 exactly as before — with a log line claiming the frame had
            // been dropped.
            let mut dropped = drop_transactions(&mut plan.frames, &skip, &drop_items);
            for (_, answers) in plan.segments.iter_mut() {
                dropped += drop_transactions(answers, &skip, &drop_items);
            }
            println!(
                "sro_peer: replay filter dropped {dropped} frame(s) — whole transactions, in the burst AND in every segment"
            );
        }
        plan
    }))
}

/// Drop whole transactions from one frame list.
///
/// A group spawn is a triple — `0x3017` opens, `0x3019` carries, `0x3018` closes
/// — and removing only the data frame leaves the client with an announcement it
/// cannot fulfil (`MsgStreamBuffer.h:247`, R=3 W=3 T=4). So a hit anywhere in a
/// run removes the run.
fn drop_transactions(frames: &mut Vec<ReplayFrame>, skip: &[u16], drop_items: &[u32]) -> usize {
    if skip.is_empty() && drop_items.is_empty() {
        return 0;
    }
    let before = frames.len();
    let mut run_of = vec![usize::MAX; frames.len()];
    let mut current: Option<usize> = None;
    let mut next_run = 0usize;
    for (i, f) in frames.iter().enumerate() {
        match f.opcode {
            0x3017 => {
                current = Some(next_run);
                next_run += 1;
                run_of[i] = current.unwrap();
            }
            0x3018 => {
                run_of[i] = current.unwrap_or(usize::MAX);
                current = None;
            }
            _ => {
                if let Some(run) = current {
                    run_of[i] = run;
                }
            }
        }
    }
    let mut doomed_runs: Vec<usize> = Vec::new();
    let mut doomed_single: Vec<usize> = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        let by_opcode = skip.contains(&f.opcode);
        let by_item = drop_items.iter().any(|id| {
            f.body
                .windows(4)
                .any(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]) == *id)
        });
        if by_opcode || by_item {
            if by_item {
                println!(
                    "sro_peer: --replay-drop-item hit in {:#06x} ({} B, recorded {})",
                    f.opcode,
                    f.body.len(),
                    f.ts
                );
            }
            if run_of[i] == usize::MAX {
                doomed_single.push(i);
            } else {
                doomed_runs.push(run_of[i]);
            }
        }
    }
    let keep: Vec<bool> = (0..frames.len())
        .map(|i| {
            let in_run = run_of[i] != usize::MAX && doomed_runs.contains(&run_of[i]);
            let single = run_of[i] == usize::MAX && doomed_single.contains(&i);
            !(in_run || single)
        })
        .collect();
    let mut it = keep.iter();
    frames.retain(|_| *it.next().unwrap());
    before - frames.len()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "sro_peer — our own gateway+agent test peer (stage 1: world entry)\n\
             \n\
             --gateway-listen <addr>   gateway listener   (default {DEFAULT_GATEWAY_LISTEN})\n\
             --agent-listen <addr>     agent listener     (default {DEFAULT_AGENT_LISTEN})\n\
             --advertise-host <h[:p]>  what 0xA102 names  (default = --agent-listen)\n\
             --fixtures <dir>          recorded bodies    (default {DEFAULT_FIXTURE_DIR})\n\
             \n\
             Session replay (world entry from the recording instead of five modelled frames):\n\
             --replay                  replay the documented default session\n\
             --replay-session <ts>     replay the session containing this recorded stamp\n\
             --replay-from <ts> --replay-to <ts>   replay an explicit inclusive window\n\
             --replay-timing fast|recorded         keep the recorded gaps, or blast (default fast)\n\
             --session-log <path>      the proxy recording (default {DEFAULT_SESSION_LOG})\n\
             --replay-skip <ops>       drop the TRANSACTIONS containing these opcodes\n\
             --replay-drop-item <ids>  drop the transaction whose body carries this item ref id\n\
             --replay-fix-item <o:n>   rewrite an unresolvable item ref id (synthesis, logged)\n\
             --replay-blank-avatar     empty the avatar-item list of every replayed 0x3019\n\
             Default session: {DEFAULT_REPLAY_SESSION} — the only recorded run in which the\n\
             original v1.188 client rendered the world. In replay mode the 2 s 0xA106 nudge is\n\
             ON by default, armed only after the last replayed frame (SRO_PEER_NUDGE=0 to opt out).\n\
             \n\
             Aliases kept: --gateway, --agent, --advertise.\n\
             For the original v1.188 client on another machine, listen on the ports its\n\
             divisioninfo names and advertise this machine's LAN address, e.g.\n\
             --gateway-listen 0.0.0.0:4001 --agent-listen 0.0.0.0:4002 \\\n\
             --advertise-host <this-machine-lan-ip>:4002\n\
             \n\
             Point the client at it: see docs/planning/OWN-TEST-PEER.md §7/§8."
        );
        return;
    }
    let gateway_addr = arg(
        &args,
        &["--gateway-listen", "--gateway"],
        DEFAULT_GATEWAY_LISTEN,
    );
    let agent_addr = arg(&args, &["--agent-listen", "--agent"], DEFAULT_AGENT_LISTEN);
    let advertise = arg(
        &args,
        &["--advertise-host", "--advertise"],
        &agent_addr.clone(),
    );
    let fixture_dir = arg(&args, &["--fixtures"], DEFAULT_FIXTURE_DIR);
    let replay = match replay_plan(&args) {
        Ok(plan) => plan,
        Err(e) => {
            eprintln!("sro_peer: {e}");
            return;
        }
    };

    let (host, port) = match split_endpoint(&advertise, &agent_addr) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("sro_peer: --advertise-host {e}");
            return;
        }
    };

    let fixtures = match Fixtures::load(
        PathBuf::from(&fixture_dir),
        &fixture_list(),
        &optional_fixture_list(),
        &variant_fixture_list(),
    ) {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "sro_peer: missing recorded body — {e}\n\
                 A missing fixture is a result, not a bug to work around: stage 1 replays only \
                 packets we recorded ourselves (docs/planning/OWN-TEST-PEER.md §2.1)."
            );
            return;
        }
    };
    let peer = Arc::new(Peer {
        fixtures,
        agent_advertise: (host.clone(), port),
        token: Arc::new(Mutex::new(None)),
        replay,
    });

    if let Some(plan) = peer.replay.as_ref() {
        println!(
            "{} sro_peer: replay armed on c2s 0x7001 — {}",
            stamp(),
            plan.describe()
        );
    }

    println!(
        "{} sro_peer: gateway {gateway_addr}, agent {agent_addr}, 0xA102 advertises {host}:{port}",
        stamp()
    );
    let agent_peer = peer.clone();
    let agent_thread = thread::spawn(move || listen(agent_addr, "agent", agent_peer));
    listen(gateway_addr, "gateway", peer);
    let _ = agent_thread.join();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 201-byte `0x3019` body of `2026-08-21T16:20:54.726Z`, verbatim from
    /// `packet_dump/proxy/0x3019.log`.
    const RECORDED_SPAWN_201: &str = "8b070000000000026d0835060000007d060000005906000000c506000000a106000000e90600000081000000086a1000000705019f24000000000dab0200a861e286654431a9c5be2061b14054d90101a861960300000500010000049a9919420100f0420000c84201c69800000118000004004465766901020000000000000000000000000000000000000000000000000000000000fff507000057010000a8617b8468446893d0beae47f541933e000100933e0100000000000000000000000000c8420002020208";

    /// The three line shapes the dumper has produced: with flag column, without
    /// it, and an empty body (`0x34A5`, whose lines are `<ts> P`).
    #[test]
    fn dump_lines_of_all_three_shapes_are_read() {
        let dir = std::env::temp_dir().join("sro_peer_fixture_test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("0x0001.log");
        std::fs::write(
            &path,
            "2026-08-10T18:19:20.802Z 0102\n2026-08-16T18:45:52.996Z 01 E\n",
        )
        .expect("write");
        assert_eq!(
            read_body(&path, Pick::Last).expect("last"),
            Bytes::from_static(&[0x01])
        );
        assert_eq!(
            read_body(&path, Pick::Longest).expect("longest"),
            Bytes::from_static(&[0x01, 0x02])
        );

        std::fs::write(&path, "2026-08-16T18:46:15.613Z  P\n").expect("write");
        assert!(read_body(&path, Pick::Last).expect("empty body").is_empty());
    }

    /// The client blocks on this answer inside `connect`
    /// (`client/src/net/connection.rs:143-186`), so it must decode to a name the
    /// original recognizes.
    #[test]
    fn the_identification_answer_names_a_known_peer_kind() {
        use packets::global::PeerKind;
        let gw = ModuleIdentification::try_from(identification("gateway")).expect("decodes");
        assert_eq!(gw.peer_kind(), PeerKind::Gateway);
        let agent = ModuleIdentification::try_from(identification("agent")).expect("decodes");
        assert_eq!(agent.peer_kind(), PeerKind::Agent);
    }

    /// A missing log must fail loudly: inventing a body is the one thing this
    /// peer may not do.
    #[test]
    fn a_missing_fixture_is_an_error() {
        let path = std::env::temp_dir().join("sro_peer_no_such_dump.log");
        let _ = std::fs::remove_file(&path);
        assert!(read_body(&path, Pick::Last).is_err());
    }

    /// The 144-byte `0x3019` body of `2026-08-21T13:40:38.112Z`, verbatim from
    /// `packet_dump/0x3019.log` line 946 (its `0x3017` is `010100`, i.e. a spawn
    /// of one record): the bot character `nummer6`, ref 1931, uid 175028,
    /// region 24744. The name is readable at the end of the record
    /// (`07 00 "nummer6"`), which is the independent check that the walk below
    /// landed on the right fields.
    const RECORDED_SPAWN_144: &str = "8b070000200000006d080f050000007b050000005705000000c3050000009f05000000e7050000004f00000000fb00000000050000b4ab0200a86000c062446be738be0040c844261c000100261c01000004cdcc0c420000dc420000c84201c69800000518000007006e756d6d65723600010000000000000000000000000000000000000000000000000000000000ff";

    /// The 36-byte gate-building record of `packet_dump/0x3019.log` line 5
    /// (`2026-08-10T18:19:22.552Z`, ref 2094, uid 12) — a *non*-player record of
    /// the same log, used as the negative case below.
    const RECORDED_STRUCTURE_36: &str =
        "2e0800000c000000a86100c09c440000c0c000c0ab440000010000010000000000000000";

    fn unhex_str(s: &str) -> Vec<u8> {
        unhex(s).expect("test fixture is hex")
    }

    /// The unique id of a replayed spawn must be **read**, not guessed — and a
    /// record that is not a player record must be refused rather than yield a
    /// plausible-looking number.
    #[test]
    fn a_recorded_player_spawn_yields_the_unique_id_in_it() {
        let one = unhex_str(RECORDED_SPAWN_144);
        assert_eq!(one.len(), 144);
        assert_eq!(player_spawn_unique_id(&one), Some((1931, 175028, 24744)));
        // Cross-check on the same bytes: the name follows the record's id and
        // position, so a walk that hit the right unique id also has the right
        // character.
        assert!(one.windows(7).any(|w| w == b"nummer6"));

        let two = unhex_str(RECORDED_SPAWN_201);
        assert_eq!(player_spawn_unique_id(&two), Some((1931, 174861, 25000)));
        assert!(two.windows(4).any(|w| w == b"Devi"));

        // A structure record of the same log: byte 8 is part of its position,
        // not an inventory size, so the walk refuses it.
        assert_eq!(
            player_spawn_unique_id(&unhex_str(RECORDED_STRUCTURE_36)),
            None
        );
        // Truncated, and a zeroed body: both must refuse.
        assert_eq!(player_spawn_unique_id(&one[..40]), None);
        assert_eq!(player_spawn_unique_id(&[0u8; 144]), None);
    }

    /// The batch is picked from the dump by its own `0x3017` (spawn, one
    /// record), newest first, and the two logs are paired line by line.
    #[test]
    fn the_replayed_spawn_is_the_newest_single_record_player_batch() {
        let dir = std::env::temp_dir().join("sro_peer_spawn_test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let begin = dir.join("0x3017.log");
        let data = dir.join("0x3019.log");
        // line 1: a despawn (kind 2) — must be skipped
        // line 2: a spawn of one record that is no player — must be skipped
        // line 3: the player batch — the one to take
        // line 4: a spawn of two records — skipped, its payload is ambiguous
        std::fs::write(
            &begin,
            "2026-08-10T18:19:22.552Z 020100\n2026-08-10T18:19:22.552Z 010100\n\
             2026-08-21T13:40:38.112Z 010100\n2026-08-21T13:41:00.000Z 010200\n",
        )
        .expect("write");
        std::fs::write(
            &data,
            format!(
                "2026-08-10T18:19:22.552Z 0c000000\n2026-08-10T18:19:22.552Z {RECORDED_STRUCTURE_36}\n\
                 2026-08-21T13:40:38.112Z {RECORDED_SPAWN_144}\n2026-08-21T13:41:00.000Z {RECORDED_SPAWN_201}\n"
            ),
        )
        .expect("write");

        let spawn = recorded_foreign_spawn(&dir).expect("a usable batch");
        assert_eq!(spawn.line, 3);
        assert_eq!(spawn.ref_id, 1931);
        assert_eq!(spawn.unique_id, 175028);
        assert_eq!(spawn.begin, Bytes::from_static(&[0x01, 0x01, 0x00]));
        assert_eq!(spawn.data.len(), 144);

        // Lost pairing is an error, never a guess.
        std::fs::write(&begin, "2026-08-10T18:19:22.552Z 020100\n").expect("write");
        assert!(recorded_foreign_spawn(&dir).is_err());
        let _ = std::fs::remove_file(&begin);
        let _ = std::fs::remove_file(&data);
    }

    /// `--advertise-host` exists for exactly this case: the listener is
    /// wildcard, the redirect must name a routable address.
    ///
    /// The routable address is spelled as the RFC 2606 placeholder
    /// `peer.example.com`, not as the LAN address this was developed against:
    /// `split_endpoint` never resolves anything, so the literal is pure test
    /// data — and `scripts/check_no_private.py` refuses an address literal in a
    /// patch whatever it points at.
    #[test]
    fn advertise_host_takes_the_listen_port_when_given_no_port() {
        assert_eq!(
            split_endpoint("peer.example.com", "0.0.0.0:4002").expect("host only"),
            ("peer.example.com".to_string(), 4002)
        );
        assert_eq!(
            split_endpoint("peer.example.com:4002", "0.0.0.0:4002").expect("host:port"),
            ("peer.example.com".to_string(), 4002)
        );
        assert!(split_endpoint("peer.example.com:noport", "0.0.0.0:4002").is_err());
        assert!(split_endpoint("peer.example.com", "a-pipe-name").is_err());
    }

    /// Both spellings must reach the same option, or §7.2's documented command
    /// line breaks silently.
    #[test]
    fn the_old_flag_names_still_parse() {
        let args: Vec<String> = ["sro_peer", "--gateway", "0.0.0.0:4001"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            arg(&args, &["--gateway-listen", "--gateway"], "default"),
            "0.0.0.0:4001"
        );
        let args: Vec<String> = ["sro_peer", "--gateway-listen", "0.0.0.0:4001"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            arg(&args, &["--gateway-listen", "--gateway"], "default"),
            "0.0.0.0:4001"
        );
        assert_eq!(arg(&args, &["--fixtures"], "packet_dump"), "packet_dump");
    }

    /// The launcher blocks on this verdict, so its shape matters: a massive
    /// header naming `0xA100`, then one payload that decodes as `result = 1`.
    #[test]
    fn the_patch_verdict_is_a_massive_run_carrying_result_one() {
        let frames = synthesized_patch_ok();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].0, OPCODE_MASSIVE);
        assert_eq!(frames[1].0, OPCODE_MASSIVE);
        assert_eq!(&frames[0].1[..], &[1, 1, 0, 0x00, 0xA1]);
        assert_eq!(frames[1].1[0], 0);
        let inner = frames[1].1.slice(1..);
        let Ok(Packet::PatchResponse(response)) = Packet::deserialize(S2C_PATCH, inner) else {
            panic!("the synthesised body must decode as a PatchResponse");
        };
        assert_eq!(response.result, 1);
        assert!(response.error.is_none());

        let notice = synthesized_notice_empty();
        assert_eq!(notice.len(), 1);
        assert_eq!(notice[0].0, S2C_NOTICE);
        let Ok(Packet::NoticeResponse(response)) =
            Packet::deserialize(S2C_NOTICE, notice[0].1.clone())
        else {
            panic!("the synthesised body must decode as a NoticeResponse");
        };
        assert_eq!(response.notice_count, 0);
    }

    /// The peer must never answer a name check with a character list.
    #[test]
    fn char_select_answers_are_chosen_by_action_byte() {
        let mut variants = HashMap::new();
        variants.insert(
            S2C_CHAR_SELECT,
            vec![
                Bytes::from_static(&[0x02, 0x01, 0x00]),
                Bytes::from_static(&[0x02, 0x01, 0x01, 0xAA]),
                Bytes::from_static(&[0x04, 0x01]),
            ],
        );
        let fixtures = Fixtures {
            dir: PathBuf::from("packet_dump"),
            bodies: HashMap::new(),
            variants,
        };
        // longest of the two action-2 lines
        assert_eq!(
            fixtures.variant(S2C_CHAR_SELECT, 0x02).expect("list"),
            Bytes::from_static(&[0x02, 0x01, 0x01, 0xAA])
        );
        assert_eq!(
            fixtures.variant(S2C_CHAR_SELECT, 0x04).expect("name check"),
            Bytes::from_static(&[0x04, 0x01])
        );
        assert!(fixtures.variant(S2C_CHAR_SELECT, 0x05).is_none());
    }

    /// `0x4000` is the reason this peer exists: swallowed, never answered,
    /// never a reason to close.
    #[test]
    fn the_machine_guid_is_swallowed_and_answered_with_nothing() {
        assert!(swallow("agent", C2S_MACHINE_GUID, &Bytes::from_static(&[0u8; 40])).is_empty());
        assert!(swallow("agent", 0xFFFF, &Bytes::new()).is_empty());
    }

    /// A two-session recording: the replay must pick the *second* session (the
    /// one containing the stamp), start after its LAST `c2s 0x7001`, keep only
    /// `s2c` rows, keep the `massive` column, and read an empty body as data.
    fn write_session_log(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(name);
        let lines = [
            r#"{"ts":"2026-08-21T10:00:00.000Z","dir":"c2s","opcode":"0x2001","len":1,"massive":false,"hex":"00"}"#,
            r#"{"ts":"2026-08-21T10:00:00.100Z","dir":"c2s","opcode":"0x7001","len":0,"massive":false,"hex":""}"#,
            r#"{"ts":"2026-08-21T10:00:00.200Z","dir":"s2c","opcode":"0xb001","len":1,"massive":false,"hex":"01"}"#,
            r#"{"ts":"2026-08-21T16:20:25.380Z","dir":"c2s","opcode":"0x2001","len":1,"massive":false,"hex":"00"}"#,
            r#"{"ts":"2026-08-21T16:20:51.361Z","dir":"c2s","opcode":"0x7001","len":1,"massive":false,"hex":"02"}"#,
            r#"{"ts":"2026-08-21T16:20:51.771Z","dir":"s2c","opcode":"0xb001","len":1,"massive":false,"hex":"01"}"#,
            r#"{"ts":"2026-08-21T16:20:51.771Z","dir":"s2c","opcode":"0x34a5","len":0,"massive":false,"hex":""}"#,
            r#"{"ts":"2026-08-21T16:20:52.099Z","dir":"c2s","opcode":"0x4000","len":1,"massive":false,"hex":"07"}"#,
            r#"{"ts":"2026-08-21T16:20:54.726Z","dir":"s2c","opcode":"0x2005","len":2,"massive":true,"hex":"0102"}"#,
        ];
        std::fs::write(&path, lines.join("\n") + "\n").expect("write session log");
        path
    }

    #[test]
    fn a_session_replay_takes_the_s2c_frames_after_the_last_world_entry_request() {
        let path = write_session_log("sro_peer_session_test.jsonl");
        let plan = ReplayPlan::for_session(&path, "2026-08-21T16:20:51.771Z", ReplayTiming::Fast)
            .expect("plan");
        let opcodes: Vec<u16> = plan.frames.iter().map(|f| f.opcode).collect();
        assert_eq!(opcodes, vec![S2C_CHAR_JOIN, S2C_CHAR_DATA_BEGIN, 0x2005]);
        // Empty `hex` is a real body, not a parse failure.
        assert!(plan.frames[1].body.is_empty());
        // First delta is measured against the triggering `c2s 0x7001`.
        assert_eq!(plan.frames[0].delta, Duration::from_millis(410));
        assert_eq!(plan.frames[1].delta, Duration::ZERO);
        assert_eq!(plan.frames[2].delta, Duration::from_millis(2955));
        assert_eq!(plan.window.1, "2026-08-21T16:20:54.726Z");
        std::fs::remove_file(&path).ok();
    }

    /// A `massive: true` row must go back out as a run of its own — header
    /// naming the inner opcode, then payload — never as a bare inner frame.
    #[test]
    fn a_massive_row_is_replayed_as_its_own_run() {
        let path = write_session_log("sro_peer_massive_test.jsonl");
        let plan = ReplayPlan::for_session(&path, "2026-08-21T16:20:54.726Z", ReplayTiming::Fast)
            .expect("plan");
        let last = plan.frames.last().expect("frames");
        assert!(last.massive);
        let wrapped = massive_wrap(last.opcode, &last.body);
        assert_eq!(wrapped.len(), 2);
        assert_eq!(wrapped[0].0, OPCODE_MASSIVE);
        assert_eq!(
            wrapped[0].1,
            Bytes::from_static(&[0x01, 0x01, 0x00, 0x05, 0x20])
        );
        assert_eq!(wrapped[1].1, Bytes::from_static(&[0x00, 0x01, 0x02]));
        assert!(!plan.frames[0].massive);
        std::fs::remove_file(&path).ok();
    }

    /// An explicit window is inclusive on both ends and ignores session
    /// boundaries, so a partial burst can be replayed on its own.
    #[test]
    fn an_explicit_window_is_inclusive_and_s2c_only() {
        let path = write_session_log("sro_peer_window_test.jsonl");
        let plan = ReplayPlan::for_window(
            &path,
            "2026-08-21T16:20:51.771Z",
            "2026-08-21T16:20:52.099Z",
            ReplayTiming::Recorded,
        )
        .expect("plan");
        assert_eq!(plan.frames.len(), 2);
        assert_eq!(plan.timing, ReplayTiming::Recorded);
        assert!(ReplayPlan::for_window(
            &path,
            "2027-01-01T00:00:00.000Z",
            "2027-01-02T00:00:00.000Z",
            ReplayTiming::Fast
        )
        .is_err());
        std::fs::remove_file(&path).ok();
    }

    /// The flags: `--replay` alone means the documented default session, a
    /// half-given window is an error, and no flag at all means no replay.
    #[test]
    fn the_replay_flags_parse() {
        let path = write_session_log("sro_peer_flags_test.jsonl");
        let log = path.display().to_string();
        let argv = |extra: &[&str]| -> Vec<String> {
            let mut v = vec![
                "sro_peer".to_string(),
                "--session-log".to_string(),
                log.clone(),
            ];
            v.extend(extra.iter().map(|s| s.to_string()));
            v
        };
        assert!(replay_plan(&argv(&[])).expect("no replay").is_none());
        assert!(replay_plan(&argv(&["--replay-from", "2026-08-21T16:20:51.771Z"])).is_err());
        assert!(replay_plan(&argv(&["--replay-timing", "sideways", "--replay"])).is_err());
        let plan = replay_plan(&argv(&[
            "--replay-session",
            "2026-08-21T16:20:51.771Z",
            "--replay-timing",
            "recorded",
        ]))
        .expect("plan")
        .expect("some");
        assert_eq!(plan.frames.len(), 3);
        assert_eq!(plan.timing, ReplayTiming::Recorded);
        // The default stamp is not in this fixture, so `--replay` must say so
        // rather than silently replay nothing.
        assert!(replay_plan(&argv(&["--replay"])).is_err());
        std::fs::remove_file(&path).ok();
    }

    /// The lobby half: the list the client chooses from must come from the SAME
    /// session as the `0x3013`, and the log line must be able to name it.
    #[test]
    fn the_replay_carries_the_lobby_list_of_the_same_session() {
        let path = std::env::temp_dir().join("sro_peer_lobby_test.jsonl");
        let lines = [
            r#"{"ts":"2026-08-21T16:20:25.380Z","dir":"c2s","opcode":"0x2001","len":1,"massive":false,"hex":"00"}"#,
            r#"{"ts":"2026-08-21T16:20:25.558Z","dir":"s2c","opcode":"0xa103","len":1,"massive":false,"hex":"01"}"#,
            // an earlier list of the same connection must NOT win
            r#"{"ts":"2026-08-21T16:20:26.100Z","dir":"s2c","opcode":"0xb007","len":3,"massive":false,"hex":"020100"}"#,
            r#"{"ts":"2026-08-21T16:20:26.269Z","dir":"s2c","opcode":"0xb007","len":16,"massive":false,"hex":"0201018b07000007006e756d6d657236"}"#,
            r#"{"ts":"2026-08-21T16:20:51.361Z","dir":"c2s","opcode":"0x7001","len":1,"massive":false,"hex":"02"}"#,
            r#"{"ts":"2026-08-21T16:20:51.771Z","dir":"s2c","opcode":"0xb001","len":1,"massive":false,"hex":"01"}"#,
        ];
        std::fs::write(&path, lines.join("\n") + "\n").expect("write");
        let plan = ReplayPlan::for_session(&path, "2026-08-21T16:20:51.771Z", ReplayTiming::Fast)
            .expect("plan");
        // The burst is only what came after world entry.
        assert_eq!(plan.frames.len(), 1);
        assert_eq!(plan.frames[0].opcode, S2C_CHAR_JOIN);
        // The lobby starts at the LAST list before world entry, so the earlier
        // 3-byte one is not part of it and cannot be answered by accident.
        assert_eq!(plan.lobby.len(), 1);
        assert_eq!(
            plan.lobby_answer(S2C_CHAR_SELECT).expect("list").body.len(),
            16
        );
        assert!(plan.lobby_answer(S2C_AGENT_LOGIN).is_none());
        assert_eq!(
            plan.lobby_list_summary(),
            Some((1, "nummer6".to_string())),
            "the log line has to be able to name the character the user must click"
        );
        std::fs::remove_file(&path).ok();
    }

    /// The `0xA102` we build must decode back to the endpoint we advertise —
    /// this is the packet `netcheck.rs:on_login_response` dials the agent leg
    /// from.
    #[test]
    fn the_built_login_response_names_our_own_agent_leg() {
        let body: Bytes = Packet::from(LoginResponse {
            result: 1,
            login_info: Some(LoginInfo {
                agent_token: 0x0123_4567,
                agent_ip: "127.0.0.1".to_string(),
                agent_port: 15884,
            }),
            login_error: None,
            custom: None,
        })
        .into_serialize()
        .1;
        let Ok(Packet::LoginResponse(decoded)) = Packet::deserialize(S2C_LOGIN, body) else {
            panic!("0xA102 must round-trip");
        };
        let info = decoded.login_info.expect("login info");
        assert_eq!(info.agent_ip, "127.0.0.1");
        assert_eq!(info.agent_port, 15884);
        assert_eq!(info.agent_token, 0x0123_4567);
    }

    /// The recorded 201-byte group spawn (`0x3019` at 16:20:54.726Z): blanking
    /// its avatar list must remove exactly five bytes — the one `{u32 refId,
    /// u8 plus}` pair — and leave the head, the equipment list and everything
    /// behind the list byte-identical. Anything that does not parse is refused
    /// rather than guessed at.
    #[test]
    fn blanking_the_avatar_list_removes_exactly_that_list() {
        let body: Vec<u8> = RECORDED_SPAWN_201
            .as_bytes()
            .chunks(2)
            .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect();
        assert_eq!(body.len(), 201);
        assert_eq!(body[50], 5, "avatar list size");
        assert_eq!(body[51], 1, "avatar item count");
        assert_eq!(&body[52..56], &9375u32.to_le_bytes(), "the crashing ref id");

        let (out, removed) = blanked_avatar_list(&body).expect("the recorded body parses");
        assert_eq!(removed, 1);
        assert_eq!(out.len(), 196);
        assert_eq!(&out[..51], &body[..51], "head and equipment list untouched");
        assert_eq!(out[51], 0, "avatar count zeroed");
        assert_eq!(
            &out[52..],
            &body[57..],
            "the tail moves up by exactly five bytes"
        );
        // Idempotent: a body without an avatar item is returned unchanged.
        assert_eq!(blanked_avatar_list(&out), Some((out.clone(), 0)));
        // Refusals: truncated, and a list size no real spawn carries.
        assert_eq!(blanked_avatar_list(&body[..40]), None);
        let mut bogus = body.clone();
        bogus[50] = 200;
        assert_eq!(blanked_avatar_list(&bogus), None);
    }
}
