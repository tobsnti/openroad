//! A client connection's bytes, for the server.
//!
//! The server opens the security handshake ([`crate::handshake`]). From the
//! client's first answer on, every client frame carries a sequence byte and a
//! CRC derived from seeds the server drew, and a frame whose bytes do not
//! match ends the connection: they prove the frame comes from the client that
//! took part in the handshake. Client frames the client encrypted are
//! decrypted with the session key. Server frames carry neither byte and go out
//! in clear, as the Silkroad security protocol has the server role do [S].
//!
//! The two opcodes the original client reads only from a 0x600D multi-part
//! (0xA100, 0xA104) are wrapped here, so no handler can send them bare: a bare
//! frame calls their handler without its container and crashes the client
//! [V original handler].

use std::sync::{Arc, RwLock};

use bytes::Bytes;
use sro_net::crc::CRC;
use sro_net::frame::SilkroadFrame;
use sro_net::security::{SilkroadSecurity, SilkroadSecurityState};
use sro_net::sequence::Sequence;

use crate::handshake::{Handshake, Refused, Step};

/// Opcodes the original client reads only from a 0x600D multi-part.
const MULTI_PART: [u16; 2] = [0xA100, 0xA104];

/// The largest body one frame carries: the size field has 15 bits.
const MAX_BODY: usize = 0x7FFF;

/// Why a connection ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Broken {
    /// Bytes that are no client frame.
    Frame,
    /// A sequence byte or CRC the handshake's seeds do not give.
    SecurityBytes,
    /// The handshake failed.
    Handshake(Refused),
}

/// What a read from the client completes.
#[derive(Debug, Default)]
pub struct Inbound {
    /// The client's frames after the handshake, in order: opcode and body.
    pub frames: Vec<(u16, Bytes)>,
    /// Bytes to send back: the handshake's answers.
    pub reply: Vec<u8>,
}

pub struct Transport {
    handshake: Option<Handshake>,
    /// What the client's frames are decrypted with once the handshake is done.
    security: Arc<RwLock<SilkroadSecurityState>>,
    sequence: Sequence,
    crc: CRC,
    buf: Vec<u8>,
}

impl Transport {
    /// A new connection, and the bytes to send first: the opening handshake.
    pub fn open() -> (Transport, Vec<u8>) {
        Self::start(Handshake::open())
    }

    /// [`Self::open`] with the handshake's seeds given ([`Handshake::seeded`]).
    pub fn seeded(sequence_seed: u8, crc_seed: u8) -> (Transport, Vec<u8>) {
        Self::start(Handshake::seeded(sequence_seed, crc_seed))
    }

    fn start((handshake, opening): (Handshake, Bytes)) -> (Transport, Vec<u8>) {
        let (sequence_seed, crc_seed) = handshake.seeds();
        let transport = Transport {
            handshake: Some(handshake),
            security: Arc::new(RwLock::new(SilkroadSecurityState::new())),
            sequence: Sequence::from(sequence_seed),
            crc: CRC::from(crc_seed),
            buf: Vec::new(),
        };
        (transport, frame(0x5000, &opening))
    }

    /// Whether the handshake is done.
    pub fn established(&self) -> bool {
        self.handshake.is_none()
    }

    /// Bytes as the socket delivered them, in any split. A frame that is not
    /// complete yet waits for the next read.
    pub fn receive(&mut self, bytes: &[u8]) -> Result<Inbound, Broken> {
        self.buf.extend_from_slice(bytes);
        let mut inbound = Inbound::default();
        while let Some((opcode, body)) = self.next_frame()? {
            match self.handshake.take() {
                None => inbound.frames.push((opcode, body)),
                Some(handshake) => match handshake.receive(opcode, &body) {
                    Ok(Step::Answer(next, answer)) => {
                        self.handshake = Some(next);
                        inbound.reply.extend(frame(0x5000, &answer));
                    }
                    Ok(Step::Done(keys)) => {
                        let mut security = self.security.write().expect("one owner");
                        security.state = SilkroadSecurity::Established;
                        security.context.blowfish = Some(keys.blowfish);
                    }
                    Err(refused) => return Err(Broken::Handshake(refused)),
                },
            }
        }
        Ok(inbound)
    }

    /// The next complete client frame, its security bytes checked.
    fn next_frame(&mut self) -> Result<Option<(u16, Bytes)>, Broken> {
        let Some(len) = SilkroadFrame::wire_len(&self.buf) else {
            return Ok(None);
        };
        if len < 6 {
            return Err(Broken::Frame);
        }
        if len > self.buf.len() {
            return Ok(None);
        }
        let parsed = SilkroadFrame::parse(&mut self.buf[..len], self.security.clone());
        let Ok((
            _,
            SilkroadFrame::Packet {
                opcode,
                count,
                crc,
                data,
                ..
            },
        )) = parsed
        else {
            return Err(Broken::Frame);
        };
        // The CRC covers the frame as the client wrote it before encrypting:
        // size, opcode, sequence byte, a zero in place of the CRC, body.
        let mut written = self.buf[..6 + data.len()].to_vec();
        written[5] = 0;
        if count != self.sequence.next() || crc != self.crc.calculate(&written) {
            return Err(Broken::SecurityBytes);
        }
        self.buf.drain(..len);
        Ok(Some((opcode, data)))
    }
}

/// The wire bytes of one server frame: a multi-part for the opcodes that need
/// one, otherwise a single frame.
pub fn frame(opcode: u16, body: &[u8]) -> Vec<u8> {
    if !MULTI_PART.contains(&opcode) {
        return single(opcode, body);
    }
    // The original's part size is unknown [U]; a part here is as large as a
    // frame allows.
    let parts: Vec<&[u8]> = match body {
        [] => vec![body],
        _ => body.chunks(MAX_BODY - 1).collect(),
    };
    let amount = u16::try_from(parts.len()).expect("a multi-part counts its parts in a u16");
    let mut head = vec![1];
    head.extend(amount.to_le_bytes());
    head.extend(opcode.to_le_bytes());
    let mut wire = single(0x600D, &head);
    for part in parts {
        wire.extend(single(0x600D, &[&[0][..], part].concat()));
    }
    wire
}

/// Size, opcode, a zero sequence byte and CRC, body.
fn single(opcode: u16, body: &[u8]) -> Vec<u8> {
    let size = u16::try_from(body.len())
        .ok()
        .filter(|&size| usize::from(size) <= MAX_BODY)
        .expect("a frame body fits the size field");
    let mut wire = Vec::with_capacity(6 + body.len());
    wire.extend(size.to_le_bytes());
    wire.extend(opcode.to_le_bytes());
    wire.extend([0, 0]);
    wire.extend(body);
    wire
}
