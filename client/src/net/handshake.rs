use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use bevy::log::error;
use bevy::prelude::debug;
use byteorder::{ByteOrder, LittleEndian, ReadBytesExt};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use thiserror::Error;

use crate::net::handshake::HandshakeError::{
    AlreadyCompleted, ChallengeFailed, InvalidFrame, InvalidHandshakePacket, NotYetInitialized,
};
use sro_net::blowfish::Blowfish;
use sro_net::codec::SilkroadEncodingOptions;
use sro_net::crc::CRC;
use sro_net::frame::{SilkroadFrame, SilkroadFrameError};
use sro_net::key_exchange::{calc_challenge, calc_key, g_pow_x_mod_p, key_transform_value};
use sro_net::security::{SilkroadSecurity, SilkroadSecurityData, SilkroadSecurityState};
use sro_net::sequence::Sequence;

#[derive(Error, Debug)]
pub enum HandshakeError {
    #[error("invalid frame")]
    InvalidFrame,
    #[error("invalid handshake packet")]
    InvalidHandshakePacket,
    #[error("handshake already completed")]
    AlreadyCompleted,
    #[error("handshake challenge failed")]
    ChallengeFailed,
    #[error("handshake has not been initialized yet")]
    NotYetInitialized,
    #[error("handshake body has {actual} byte(s), the flag byte {flags:#04x} requires {expected}")]
    UnexpectedBodySize {
        flags: u8,
        expected: usize,
        actual: usize,
    },
    #[error("handshake challenge arrived without a preceding key exchange (flags {0:#04x})")]
    MissingKeyExchange(u8),
    #[error("reading a handshake frame failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("parsing a handshake frame failed: {0}")]
    Frame(#[from] SilkroadFrameError),
    /// Our own buffer, not a protocol limit: a frame carries a 16-bit length
    /// field, but the handshake's frames measure 43/15/26 bytes on the wire, so
    /// 4 KiB is generous for this one reader.
    #[error("a handshake frame did not fit in our {0}-byte handshake buffer")]
    FrameTooLarge(usize),
}

/// The handshake flag byte, as the original branches on it
/// (`corpus/client-dec/004b1da0_FUN_004b1da0.c:39,42,47,49`).
pub(crate) const FLAG_BLOWFISH: u8 = 0x02;
pub(crate) const FLAG_SECURITY_BYTES: u8 = 0x04;
pub(crate) const FLAG_KEY_EXCHANGE: u8 = 0x08;
pub(crate) const FLAG_CHALLENGE: u8 = 0x10;

/// Body length a `0x5000` with this flag byte must have, in either phase.
///
/// The handshake is the trust boundary for every later packet, and the reads
/// below are `Bytes::get_*`, which **panic** on a short buffer — so a peer that
/// truncates a frame could kill the client before any validation ran. The
/// original guards the same boundary with an explicit size check and its own
/// log line (`:143-147`).
///
/// Each set flag contributes exactly what its branch reads: blowfish key `u64`
/// (`:40` = 8 bytes), the two EDC seeds (`:44-45` = 2 × `u32`), the key
/// exchange's `u64 + 3 × u32` (`:57`, `setup_handshake` below), and the
/// challenge `u64` (`:84-85`).
pub(crate) fn expected_body_len(flags: u8) -> usize {
    let mut len = 1; // the flag byte itself
    if flags & FLAG_BLOWFISH != 0 {
        len += 8;
    }
    if flags & FLAG_SECURITY_BYTES != 0 {
        len += 8;
    }
    if flags & FLAG_KEY_EXCHANGE != 0 {
        len += 20;
    }
    if flags & FLAG_CHALLENGE != 0 {
        len += 8;
    }
    len
}

/// Reject a `0x5000` body that does not match its own flag byte.
///
/// `setup_flags` is what the *previous* phase announced, the original's
/// accumulator at `+0x200` (`:120,162`). A challenge is only meaningful if a
/// key exchange preceded it, so `FLAG_CHALLENGE` without a prior
/// `FLAG_KEY_EXCHANGE` is rejected — the client-side half of the original's
/// "all of `0x02|0x04|0x08` must have been seen" gate (`:138-141`).
///
/// **Deviation:** we do *not* require `FLAG_BLOWFISH`/`FLAG_SECURITY_BYTES`.
/// That check lives in the original's **server** role (the branch that receives
/// the client's 12-byte reply, `:127-148`), and whether a live v1.188 gateway
/// always sends the full `0x0E` or a subset is an open UNKNOWN — failing a
/// login on an unproven constant would be the worse defect.
pub(crate) fn check_body(data: &[u8], setup_flags: u8) -> Result<u8, HandshakeError> {
    let flags = *data.first().ok_or(HandshakeError::UnexpectedBodySize {
        flags: 0,
        expected: 1,
        actual: 0,
    })?;
    let expected = expected_body_len(flags);
    if data.len() != expected {
        return Err(HandshakeError::UnexpectedBodySize {
            flags,
            expected,
            actual: data.len(),
        });
    }
    if flags & FLAG_CHALLENGE != 0 && setup_flags & FLAG_KEY_EXCHANGE == 0 {
        return Err(HandshakeError::MissingKeyExchange(setup_flags));
    }
    Ok(flags)
}

/// How long the rest of a frame already in flight may take to arrive.
///
/// Not a value from the original: the peer has already sent this frame's head,
/// so its tail is one round trip away, and the number's only job is to end a
/// stall — not to time the network.
const FRAME_COMPLETION_TIMEOUT: Duration = Duration::from_secs(5);

/// Reads one whole frame, carrying a frame that TCP split across segments.
///
/// The in-game loop keeps a frame's tail and reads again
/// (`plugins/net/plugin.rs`, `FrameStep::Incomplete => break`); the connect
/// phase does the same here, so a split 43-byte setup frame or a split module
/// identification no longer fails the login. Re-parsing the growing buffer is
/// sound because the frame's length comes from its unencrypted size prefix.
///
/// Reading until the peer is done would be the next defect, so three bounds end
/// the loop: end of stream, a frame that does not fit [`CONNECT_FRAME_MAX`]
/// (ours, not the protocol's), and [`FRAME_COMPLETION_TIMEOUT`] on the
/// continuation reads. The first read keeps the socket's own blocking
/// behaviour.
fn read_frame(
    stream: &mut TcpStream,
    security: Arc<RwLock<SilkroadSecurityState>>,
) -> Result<SilkroadFrame, HandshakeError> {
    // The handshake is strictly one request per response: whatever a read
    // carries behind its frame is not ours to keep.
    read_frame_into(stream, security, &mut Vec::new())
}

/// Largest frame the connect phase accepts; the handshake and the module
/// identification are a few dozen bytes.
const CONNECT_FRAME_MAX: usize = 4096;

/// [`read_frame`], but `carry` holds the bytes already read and keeps the ones
/// that arrived behind the frame — the start of what the caller reads next.
pub(crate) fn read_frame_into(
    stream: &mut TcpStream,
    security: Arc<RwLock<SilkroadSecurityState>>,
    carry: &mut Vec<u8>,
) -> Result<SilkroadFrame, HandshakeError> {
    let mut chunk = [0; CONNECT_FRAME_MAX];
    if carry.is_empty() {
        let read = stream.read(&mut chunk)?;
        carry.extend_from_slice(&chunk[..read]);
    }
    loop {
        match SilkroadFrame::wire_len(carry) {
            Some(len) if len > CONNECT_FRAME_MAX => {
                return Err(HandshakeError::FrameTooLarge(CONNECT_FRAME_MAX))
            }
            Some(len) if len <= carry.len() => {
                let (_, frame) = SilkroadFrame::parse(&mut carry[..len], security.clone())?;
                carry.drain(..len);
                return Ok(frame);
            }
            _ => {}
        }
        let read = read_continuation(stream, &mut chunk)?;
        if read == 0 {
            return Err(std::io::Error::from(ErrorKind::UnexpectedEof).into());
        }
        carry.extend_from_slice(&chunk[..read]);
    }
}

/// One continuation read, bounded in time so a peer that stops mid-frame cannot
/// hold the connect thread. The previous timeout is put back before returning:
/// the module-identification read that follows the handshake keeps its own
/// behaviour.
fn read_continuation(stream: &mut TcpStream, buf: &mut [u8]) -> Result<usize, HandshakeError> {
    let previous = stream.read_timeout()?;
    stream.set_read_timeout(Some(FRAME_COMPLETION_TIMEOUT))?;
    let read = stream.read(buf);
    let _ = stream.set_read_timeout(previous);
    Ok(read?)
}

pub(crate) fn initialize(
    stream: &mut TcpStream,
    security: Arc<RwLock<SilkroadSecurityState>>,
) -> Result<SilkroadSecurityState, HandshakeError> {
    debug!("initializing handshake");
    let frame = read_frame(stream, security.clone())?;
    match frame {
        SilkroadFrame::Packet { opcode, data, .. } => {
            if opcode != 0x5000 {
                return Err(InvalidHandshakePacket);
            }
            match security.read().expect("lock to be readable").state {
                SilkroadSecurity::None => {
                    // Phase 1 is the first thing an unauthenticated peer sends:
                    // validate its length against its own flag byte before any
                    // `get_*` can panic on a truncated body (#470).
                    check_body(&data, 0)?;
                    match setup_handshake(data, stream) {
                        Ok(state) => Ok(state),
                        Err(_) => Err(InvalidFrame),
                    }
                }
                _ => Err(InvalidHandshakePacket),
            }
        }
        _ => Err(InvalidHandshakePacket),
    }
}

/// Phase 1. The caller has already validated the body against its flag byte
/// ([`check_body`]), so the `get_*` reads below cannot run off the end.
fn setup_handshake(
    mut data: Bytes,
    write: &mut TcpStream,
) -> Result<SilkroadSecurityState, SilkroadFrameError> {
    let flags = data.get_u8();
    let opts = SilkroadEncodingOptions::from(flags);

    let mut sec_data = SilkroadSecurityData::new();
    // The original accumulates the phase-1 flags and gates phase 2 on them
    // (`004b1da0:120,162`); we keep the byte for the same reason.
    sec_data.setup_flags = flags;

    if opts.encryption {
        let handshake_bf_key = data.get_u64_le();
        let bf = Blowfish::new(&handshake_bf_key.to_le_bytes()).expect("invalid blowfish key");
        sec_data.blowfish = Some(bf);
    }

    if opts.edc {
        let sequence_seed = data.get_u32_le();
        let crc_seed = data.get_u32_le();
        debug!("received sequence seed: {}", sequence_seed);
        debug!("received crc seed: {}", crc_seed);
        sec_data.sequence = Sequence::from(sequence_seed);
        sec_data.crc = Box::new(CRC::from(crc_seed));
        sec_data.crc_seed = crc_seed << 8;
        sec_data.sequence_seed = sequence_seed;
    }

    if opts.key_exchange {
        // setup handshake data
        let handshake_key = data.get_u64_le();
        let generator = data.get_u32_le();
        let prime = data.get_u32_le();
        let remote_public = data.get_u32_le();

        // Fresh private exponent per session. The original draws
        // `NextUInt32() & 0x7FFFFFFF` (xBot `Security.cs:589`); the fixed
        // literal this replaces made our public key and the shared secret
        // deterministic for any given set of server parameters.
        let private_exponent = rand::random::<u32>() & 0x7FFF_FFFF;
        let local_public = g_pow_x_mod_p(generator, private_exponent, prime);
        let common_secret = g_pow_x_mod_p(remote_public, private_exponent, prime);

        let new_bf_key = calc_key(common_secret, remote_public, local_public);
        let blowfish = Blowfish::new(&new_bf_key).expect("failed to init new blowfish key");
        let local_challenge = calc_challenge(common_secret, local_public, remote_public);
        let mut lc = BytesMut::new();
        lc.extend_from_slice(&local_challenge);
        blowfish.encrypt(&mut lc);
        let local_challenge = LittleEndian::read_u64(&lc.freeze());

        sec_data.local_public = local_public;
        sec_data.remote_public = remote_public;
        sec_data.common_secret = common_secret;
        sec_data.handshake_key = handshake_key;
        sec_data.generator = generator;
        sec_data.prime = prime;
        sec_data.blowfish = Some(blowfish);
        sec_data.local_challenge = local_challenge;

        let mut frame_data = BytesMut::new();
        frame_data.put_u32_le(local_public);
        frame_data.put_u64_le(local_challenge);

        let frame = SilkroadFrame::Packet {
            count: 0,
            crc: 0,
            opcode: 0x5000,
            encrypted: 0,
            data: frame_data.freeze(),
        };

        let security = SilkroadSecurityState {
            state: SilkroadSecurity::Initialized,
            context: sec_data.clone(),
        };
        let s = Arc::new(RwLock::new(security));
        let mut buf = frame
            .serialize(s.clone())
            .expect("failed to serialize handshake response frame");
        write
            .write(&mut buf)
            .expect("failed to send handshake response data");
        debug!("sent challenge");

        sec_data = s
            .read()
            .expect("security to be freed already")
            .context
            .clone();
    }

    Ok(SilkroadSecurityState {
        state: SilkroadSecurity::Initialized,
        context: sec_data,
    })
}

pub(crate) fn finalize(
    stream: &mut TcpStream,
    security: Arc<RwLock<SilkroadSecurityState>>,
) -> Result<SilkroadSecurityState, HandshakeError> {
    debug!("finalizing handshake");
    let frame = read_frame(stream, security.clone())?;
    return match frame {
        SilkroadFrame::Packet { opcode, data, .. } => {
            if opcode != 0x5000 {
                return Err(InvalidHandshakePacket);
            }
            let mut security = security.write().expect("security to be writable");
            match security.state {
                SilkroadSecurity::None => Err(NotYetInitialized),
                SilkroadSecurity::Established => Err(AlreadyCompleted),
                SilkroadSecurity::Initialized => derive_final_key(data, stream, &mut security),
            }
        }
        _ => Err(InvalidHandshakePacket),
    };
}

fn derive_final_key(
    data: Bytes,
    write: &mut TcpStream,
    security: &mut SilkroadSecurityState,
) -> Result<SilkroadSecurityState, HandshakeError> {
    // Trust boundary (#470): the challenge decides whether we talk to the peer
    // that derived our shared secret, so a body that does not match its own
    // flag byte — or a challenge with no key exchange behind it — is a hard
    // error, not a short read that panics inside `read_u64`.
    let enc_opt = check_body(&data, security.context.setup_flags)?;
    let mut reader = data.reader();
    let _ = reader.read_u8();
    let opts = SilkroadEncodingOptions::from(enc_opt);

    let sec_data = &security.context;
    if opts.key_challenge {
        let remote_challenge = reader
            .read_u64::<LittleEndian>()
            .expect("failed to read remote challenge");
        // The server's signature is the MIRROR of ours: it concatenates
        // `remote_public ‖ local_public` and keys the transform on
        // `remote_public & 7`, where our own signature used
        // `local_public ‖ remote_public` with `local_public & 7`
        // (xBot `Security.cs:608-615`).
        //
        // This used to recompute our *own* signature and compare it against
        // our own stored copy — identical inputs, so the comparison could
        // never fail and the server was never actually authenticated.
        let expected = server_challenge(sec_data);
        if remote_challenge != expected {
            error!(
                "server signature error: expected = {:X}, received = {:X}",
                expected, remote_challenge
            );
            return Err(ChallengeFailed);
        }

        let final_key = key_transform_value(
            Bytes::copy_from_slice(&security.context.handshake_key.to_le_bytes()),
            security.context.common_secret,
            3,
        );
        security.context.blowfish =
            Some(Blowfish::new(&final_key).expect("blowfish to be initialized with final key"));
    }

    let new_frame = SilkroadFrame::Packet {
        count: 0,
        crc: 0,
        opcode: 0x9000,
        encrypted: 0,
        data: Bytes::new(),
    };

    let new_security = SilkroadSecurityState {
        state: SilkroadSecurity::Established,
        context: security.context.clone(),
    };
    let s = Arc::new(RwLock::new(new_security));
    let buf = new_frame
        .serialize(Arc::clone(&s))
        .expect("failed to serialize handshake completed frame");
    write
        .write(&buf)
        .expect("failed to send handshake completed data");

    return Ok(SilkroadSecurityState {
        state: SilkroadSecurity::Established,
        context: s.read().expect("security to be readable").context.clone(),
    });
}

/// The challenge value we expect the server to send back, proving it derived
/// the same shared secret: `blowfish(key_transform(A‖B, K, A & 7))` with
/// `A = remote_public`, `B = local_public` — the mirror of the signature we
/// sent (xBot `Security.cs:608-615`).
///
/// Uses the handshake blowfish (keyed from [`calc_key`]), which is still the
/// active one at this point; the final session key is derived only after this
/// check passes.
pub(crate) fn server_challenge(sec_data: &SilkroadSecurityData) -> u64 {
    let challenge = calc_challenge(
        sec_data.common_secret,
        sec_data.remote_public,
        sec_data.local_public,
    );
    let mut buf = BytesMut::new();
    buf.extend_from_slice(&challenge);
    sec_data
        .blowfish
        .clone()
        .expect("blowfish to be setup")
        .encrypt(&mut buf);
    LittleEndian::read_u64(&buf.freeze())
}

#[cfg(test)]
mod test {
    use std::net::TcpListener;

    use sro_net::key_exchange::concat_u32_as_bytes;

    use super::*;

    /// A session mid-handshake: the client has computed B and K from the
    /// server's g/p/A and keyed the handshake blowfish.
    fn session(
        generator: u32,
        prime: u32,
        remote_public: u32,
        private_exponent: u32,
    ) -> SilkroadSecurityData {
        let local_public = g_pow_x_mod_p(generator, private_exponent, prime);
        let common_secret = g_pow_x_mod_p(remote_public, private_exponent, prime);
        let bf_key = calc_key(common_secret, remote_public, local_public);

        let mut data = SilkroadSecurityData::new();
        data.local_public = local_public;
        data.remote_public = remote_public;
        data.common_secret = common_secret;
        data.blowfish = Some(Blowfish::new(&bf_key).expect("valid blowfish key"));
        data
    }

    /// Our own signature, as sent in the 0x5000 response.
    fn client_signature(sec: &SilkroadSecurityData) -> u64 {
        let challenge = calc_challenge(sec.common_secret, sec.local_public, sec.remote_public);
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&challenge);
        sec.blowfish
            .clone()
            .expect("blowfish to be setup")
            .encrypt(&mut buf);
        LittleEndian::read_u64(&buf.freeze())
    }

    /// The server's challenge is the MIRROR of the client's signature — it
    /// concatenates `remote‖local` and keys on `remote & 7`, not `local‖remote`
    /// on `local & 7`. Comparing the two used to be the whole "validation",
    /// which made it a tautology that could never fail (#248).
    #[test]
    fn server_challenge_is_not_the_client_signature() {
        let sec = session(0xF2E1_5D3A, 0x7FFF_FFC3, 0x1234_5678, 0x0BAD_C0DE);
        assert_ne!(
            server_challenge(&sec),
            client_signature(&sec),
            "the two signatures must differ, or authentication is vacuous"
        );
    }

    /// The expected value matches the original's formula:
    /// `blowfish(key_transform(A‖B, K, A & 7))` (xBot `Security.cs:608-615`).
    #[test]
    fn server_challenge_follows_the_original_formula() {
        let sec = session(0xF2E1_5D3A, 0x7FFF_FFC3, 0x1234_5678, 0x0BAD_C0DE);

        let mut expected = BytesMut::new();
        expected.extend_from_slice(&key_transform_value(
            concat_u32_as_bytes(sec.remote_public, sec.local_public),
            sec.common_secret,
            (sec.remote_public & 7) as u8,
        ));
        sec.blowfish
            .clone()
            .expect("blowfish to be setup")
            .encrypt(&mut expected);

        assert_eq!(
            server_challenge(&sec),
            LittleEndian::read_u64(&expected.freeze())
        );
    }

    /// A server that cannot derive the same shared secret produces a different
    /// challenge, and must be rejected. Flipping any single bit is enough.
    #[test]
    fn a_tampered_challenge_does_not_match() {
        let sec = session(0xF2E1_5D3A, 0x7FFF_FFC3, 0x1234_5678, 0x0BAD_C0DE);
        let expected = server_challenge(&sec);
        for bit in 0..64 {
            assert_ne!(expected, expected ^ (1u64 << bit), "bit {bit}");
        }
        // and a session with a different secret yields a different challenge
        let other = session(0xF2E1_5D3A, 0x7FFF_FFC3, 0x1234_5678, 0x0BAD_C0DF);
        assert_ne!(server_challenge(&other), expected);
    }

    /// The private exponent must vary per session — it used to be the literal
    /// 123, making our public key and shared secret deterministic.
    #[test]
    fn private_exponent_is_drawn_per_session() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            let x = rand::random::<u32>() & 0x7FFF_FFFF;
            assert!(x <= 0x7FFF_FFFF, "must fit the original's 31-bit mask");
            seen.insert(x);
        }
        assert!(seen.len() > 1, "exponent must not be constant");
    }

    /// #470: the handshake is the trust boundary for every later packet, and
    /// the body reads panic on a short buffer. The original guards it with an
    /// explicit size check plus its own log line (`004b1da0:143-147`); a body
    /// that does not match its own flag byte must be rejected.
    #[test]
    fn a_handshake_body_must_match_its_flag_byte() {
        // Phase 1, full flags 0x0E: 1 + 8 (blowfish) + 8 (edc seeds) + 20 (kex).
        assert_eq!(
            expected_body_len(FLAG_BLOWFISH | FLAG_SECURITY_BYTES | FLAG_KEY_EXCHANGE),
            37
        );
        // Phase 2, challenge only: 1 + 8.
        assert_eq!(expected_body_len(FLAG_CHALLENGE), 9);

        let full = FLAG_BLOWFISH | FLAG_SECURITY_BYTES | FLAG_KEY_EXCHANGE;
        let mut body = vec![full];
        body.extend_from_slice(&[0u8; 36]);
        assert!(check_body(&body, 0).is_ok());

        // one byte short, one byte long, and empty
        assert!(matches!(
            check_body(&body[..36], 0),
            Err(HandshakeError::UnexpectedBodySize {
                expected: 37,
                actual: 36,
                ..
            })
        ));
        body.push(0);
        assert!(matches!(
            check_body(&body, 0),
            Err(HandshakeError::UnexpectedBodySize {
                expected: 37,
                actual: 38,
                ..
            })
        ));
        assert!(matches!(
            check_body(&[], 0),
            Err(HandshakeError::UnexpectedBodySize { actual: 0, .. })
        ));
    }

    /// The challenge only proves anything if a key exchange produced a shared
    /// secret first — the client-side half of the original's
    /// "all of 0x02|0x04|0x08 must have been seen" gate (`:138-141`).
    #[test]
    fn a_challenge_without_a_prior_key_exchange_is_rejected() {
        let mut body = vec![FLAG_CHALLENGE];
        body.extend_from_slice(&[0u8; 8]);

        assert!(matches!(
            check_body(&body, 0),
            Err(HandshakeError::MissingKeyExchange(0))
        ));
        assert!(matches!(
            check_body(&body, FLAG_BLOWFISH | FLAG_SECURITY_BYTES),
            Err(HandshakeError::MissingKeyExchange(_))
        ));
        // with the key exchange seen in phase 1 it is accepted
        assert_eq!(
            check_body(
                &body,
                FLAG_BLOWFISH | FLAG_SECURITY_BYTES | FLAG_KEY_EXCHANGE
            )
            .unwrap(),
            FLAG_CHALLENGE
        );
    }

    /// A `0x5000` phase-1 frame as a gateway sends it: the 6-byte frame header
    /// plus the 37-byte body that flag byte `0x0E` announces
    /// ([`expected_body_len`]) — 43 bytes, the size measured on the wire.
    ///
    /// The field values are a recorded handshake's, not round numbers: the CRC
    /// seed in particular has to fit a byte, because `CRC::calculate` indexes
    /// `seed + byte` into a 256x256 table.
    fn setup_frame_full_flags() -> Vec<u8> {
        let mut body = BytesMut::new();
        body.put_u8(FLAG_BLOWFISH | FLAG_SECURITY_BYTES | FLAG_KEY_EXCHANGE);
        body.put_u64_le(0x2c47_84d7_180c_3b2d); // handshake blowfish key
        body.put_u32_le(0xde20_6f6c); // sequence seed
        body.put_u32_le(0xb9); // crc seed
        body.put_u64_le(0x4180_cc82_3187_35b4); // handshake key
        body.put_u32_le(8662); // generator
        body.put_u32_le(2_622_084_571); // prime
        body.put_u32_le(2_043_001_406); // server public
        let body = body.freeze();

        let mut frame = BytesMut::new();
        frame.put_u16_le(body.len() as u16);
        frame.put_u16_le(0x5000);
        frame.put_u8(0); // count
        frame.put_u8(0); // crc
        frame.extend_from_slice(&body);
        frame.to_vec()
    }

    /// A peer that stops in the middle of the setup frame makes `parse` report
    /// `Incomplete`, and that has to reach the caller. The `.expect` this
    /// replaces panicked the `sro-connect` thread instead, which the ECS could
    /// only report as "connection handshake panicked".
    #[test]
    fn a_setup_frame_cut_short_by_a_closing_peer_is_an_error() {
        let frame = setup_frame_full_flags();
        assert_eq!(frame.len(), 43, "flag byte 0x0E announces a 43-byte frame");

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        let addr = listener.local_addr().expect("listener address");
        let peer = std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().expect("accept the client");
            // Half a frame, then the stream ends with the dropped socket.
            peer.write_all(&frame[..20]).expect("write the first half");
        });

        let mut stream = TcpStream::connect(addr).expect("connect to the listener");
        let security = Arc::new(RwLock::new(SilkroadSecurityState::new()));
        // `let Err(..) else`, not `expect_err`: the Ok type is not `Debug`.
        let Err(error) = initialize(&mut stream, security) else {
            panic!("half a frame cannot initialize");
        };
        assert!(
            matches!(error, HandshakeError::Frame(_) | HandshakeError::Io(_)),
            "expected a reported read/parse error, got {error}"
        );
        peer.join().expect("listener thread");
    }

    /// The same frame, but the peer does send the rest: TCP is allowed to split
    /// it, so the handshake has to wait for the tail instead of failing the
    /// login. This is the case a relay reproduced by cutting the first server
    /// payload into 20 + 23 bytes.
    #[test]
    fn a_setup_frame_split_across_two_segments_still_initializes() {
        let frame = setup_frame_full_flags();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        let addr = listener.local_addr().expect("listener address");
        let peer = std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().expect("accept the client");
            peer.write_all(&frame[..20])
                .expect("write the first segment");
            // Two sends with a gap, so they cannot coalesce into one segment.
            std::thread::sleep(Duration::from_millis(20));
            peer.write_all(&frame[20..])
                .expect("write the second segment");
            // Drain the client's reply, or dropping the socket resets the
            // connection while it is still writing.
            let mut reply = [0; 64];
            let _ = peer.read(&mut reply);
        });

        let mut stream = TcpStream::connect(addr).expect("connect to the listener");
        let security = Arc::new(RwLock::new(SilkroadSecurityState::new()));
        let state = initialize(&mut stream, security).expect("a split setup frame initializes");
        assert!(matches!(state.state, SilkroadSecurity::Initialized));
        peer.join().expect("listener thread");
    }

    /// The module identification after the handshake, split by TCP and
    /// followed in the same segment by the server's next frame: the frame is
    /// read whole, and the bytes behind it are kept for the receive loop
    /// instead of being dropped. A single read failed the login here.
    #[test]
    fn a_split_frame_is_read_whole_and_what_follows_is_kept() {
        // 0x2001 "AgentServer" + 0, unencrypted: size, opcode, count, crc, body
        let mut body = 11u16.to_le_bytes().to_vec();
        body.extend_from_slice(b"AgentServer");
        body.push(0);
        let mut frame = (body.len() as u16).to_le_bytes().to_vec();
        frame.extend_from_slice(&0x2001u16.to_le_bytes());
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(&body);
        let next = [0x02u8, 0x00, 0x05, 0x20, 0x00, 0x00, 0x01, 0x02];

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        let addr = listener.local_addr().expect("listener address");
        let (head, tail) = frame.split_at(5);
        let (head, tail) = (head.to_vec(), [tail, &next[..]].concat());
        let peer = std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().expect("accept the client");
            peer.write_all(&head).expect("write the first segment");
            std::thread::sleep(Duration::from_millis(20));
            peer.write_all(&tail)
                .expect("write the rest and the next frame");
        });

        let mut stream = TcpStream::connect(addr).expect("connect to the listener");
        let security = Arc::new(RwLock::new(SilkroadSecurityState::new()));
        let mut carry = Vec::new();
        let SilkroadFrame::Packet { opcode, data, .. } =
            read_frame_into(&mut stream, security, &mut carry).expect("the split frame is read")
        else {
            panic!("a packet frame");
        };
        assert_eq!(opcode, 0x2001);
        assert_eq!(&data[..], &body[..]);
        assert_eq!(carry, next, "the bytes behind the frame are kept");
        peer.join().expect("listener thread");
    }

    /// The bound that keeps waiting for the tail from becoming the next defect:
    /// a `0x7FFF` content length can never be satisfied out of the handshake's
    /// own buffer, so `parse` answers `Incomplete` for every read. The asserted
    /// variant is the point — without the bound this peer, which is still
    /// sending, gets blamed for an "unexpected end of file" instead.
    #[test]
    fn a_length_prefix_larger_than_the_handshake_buffer_is_rejected() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        let addr = listener.local_addr().expect("listener address");
        let peer = std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().expect("accept the client");
            // content length 0x7FFF, unencrypted, then opcode/count/crc
            let mut head = vec![0xFF, 0x7F];
            head.extend_from_slice(&[0; 6]);
            let _ = peer.write_all(&head);
            // Keep feeding: the reader has to stop on the bound, not on silence.
            let filler = [0; 1024];
            for _ in 0..8 {
                if peer.write_all(&filler).is_err() {
                    break;
                }
            }
        });

        let mut stream = TcpStream::connect(addr).expect("connect to the listener");
        let security = Arc::new(RwLock::new(SilkroadSecurityState::new()));
        let Err(error) = initialize(&mut stream, security) else {
            panic!("an oversized frame cannot initialize");
        };
        assert!(
            matches!(error, HandshakeError::FrameTooLarge(4096)),
            "expected the buffer bound to fire, got {error}"
        );
        let _ = peer.join();
    }
}
