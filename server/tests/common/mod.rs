//! A client for tests, built from the codec the client uses (`sro_net`): its
//! side of the handshake as `client/src/net/handshake.rs` computes it, and its
//! frames written by the client's serializer.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use server::transport::{Inbound, Transport};
use sro_net::blowfish::Blowfish;
use sro_net::crc::CRC;
use sro_net::frame::SilkroadFrame;
use sro_net::key_exchange::{calc_challenge, calc_key, g_pow_x_mod_p, key_transform_value};
use sro_net::security::{SilkroadSecurity, SilkroadSecurityState};
use sro_net::sequence::Sequence;
use sro_net::stream::{next_packet, FrameStep};

pub fn plain() -> Arc<RwLock<SilkroadSecurityState>> {
    Arc::new(RwLock::new(SilkroadSecurityState::new()))
}

/// Opcode and body of the one frame in `wire`.
pub fn single(wire: &[u8]) -> (u16, Bytes) {
    let mut wire = wire.to_vec();
    let Ok((_, SilkroadFrame::Packet { opcode, data, .. })) =
        SilkroadFrame::parse(&mut wire, plain())
    else {
        panic!("a single frame");
    };
    (opcode, data)
}

/// The client's half of a connection.
pub struct Client {
    security: Arc<RwLock<SilkroadSecurityState>>,
    secret: u32,
    public: u32,
    server_public: u32,
    handshake_key: u64,
}

impl Client {
    /// Reads the opening; the client and the bytes of its answer.
    pub fn answer(opening: &[u8]) -> (Client, Vec<u8>) {
        let (opcode, body) = single(opening);
        assert_eq!((opcode, body[0], body.len()), (0x5000, 0x0E, 37));
        let u32_at = |at: usize| u32::from_le_bytes(body[at..at + 4].try_into().unwrap());
        let handshake_key = u64::from_le_bytes(body[17..25].try_into().unwrap());
        let (generator, prime, server_public) = (u32_at(25), u32_at(29), u32_at(33));
        let private = 0x0BAD_C0DE;
        let public = g_pow_x_mod_p(generator, private, prime);
        let secret = g_pow_x_mod_p(server_public, private, prime);
        let blowfish = Blowfish::new(&calc_key(secret, server_public, public)).unwrap();
        let mut signature = BytesMut::from(&calc_challenge(secret, public, server_public)[..]);
        blowfish.encrypt(&mut signature);

        let mut state = SilkroadSecurityState::new();
        state.state = SilkroadSecurity::Initialized;
        state.context.sequence = Sequence::from(u32_at(9));
        state.context.crc = Box::new(CRC::from(u32_at(13)));
        state.context.blowfish = Some(blowfish);
        let client = Client {
            security: Arc::new(RwLock::new(state)),
            secret,
            public,
            server_public,
            handshake_key,
        };
        let answer = [&public.to_le_bytes()[..], &signature[..]].concat();
        let wire = client.send(0x5000, &answer, false);
        (client, wire)
    }

    /// Checks the server's signature and switches to the session key.
    pub fn challenged(&self, reply: &[u8]) {
        let (opcode, body) = single(reply);
        let mut state = self.security.write().unwrap();
        let handshake = state.context.blowfish.unwrap();
        let mut expected =
            BytesMut::from(&calc_challenge(self.secret, self.server_public, self.public)[..]);
        handshake.encrypt(&mut expected);
        assert_eq!(
            (opcode, &body[..]),
            (0x5000, &[&[0x10][..], &expected].concat()[..])
        );
        let key = Bytes::copy_from_slice(&self.handshake_key.to_le_bytes());
        let session = key_transform_value(key, self.secret, 3);
        state.context.blowfish = Some(Blowfish::new(&session).unwrap());
        state.state = SilkroadSecurity::Established;
    }

    pub fn send(&self, opcode: u16, body: &[u8], encrypted: bool) -> Vec<u8> {
        let frame = SilkroadFrame::Packet {
            opcode,
            encrypted: u8::from(encrypted),
            count: 0,
            crc: 0,
            data: Bytes::copy_from_slice(body),
        };
        frame.serialize(self.security.clone()).unwrap().to_vec()
    }

    /// The sequence byte the client writes on its next frame.
    pub fn next_sequence_byte(&self) -> u8 {
        let mut sequence = self.security.read().unwrap().context.sequence;
        sequence.next()
    }
}

/// `bytes` handed to the server `split` bytes at a time.
pub fn feed(server: &mut Transport, bytes: &[u8], split: usize) -> Inbound {
    let mut all = Inbound::default();
    for part in bytes.chunks(split) {
        let inbound = server.receive(part).expect("accepted");
        all.frames.extend(inbound.frames);
        all.reply.extend(inbound.reply);
    }
    all
}

/// The client's end of a connection to a running server.
pub struct Wire {
    stream: TcpStream,
    buf: Vec<u8>,
    pub client: Client,
}

impl Wire {
    /// Connects and completes the handshake.
    pub fn connect(server: SocketAddr) -> Wire {
        let stream = TcpStream::connect(server).expect("the server accepts");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut buf = Vec::new();
        let opening = raw_frame(&stream, &mut buf);
        let (client, answer) = Client::answer(&opening);
        (&stream).write_all(&answer).unwrap();
        client.challenged(&raw_frame(&stream, &mut buf));
        (&stream)
            .write_all(&client.send(0x9000, &[], false))
            .unwrap();
        Wire {
            stream,
            buf,
            client,
        }
    }

    pub fn into_stream(self) -> TcpStream {
        self.stream
    }

    pub fn send(&mut self, opcode: u16, body: &[u8], encrypted: bool) {
        let wire = self.client.send(opcode, body, encrypted);
        self.stream.write_all(&wire).unwrap();
    }

    /// The next packet, a multi-part put together as the client does it.
    pub fn recv(&mut self) -> (u16, Bytes) {
        loop {
            match next_packet(&mut self.buf, &plain()) {
                FrameStep::Ready {
                    opcode,
                    data,
                    consumed,
                    ..
                } => {
                    self.buf.drain(..consumed);
                    return (opcode, data);
                }
                FrameStep::Skip { consumed } => {
                    self.buf.drain(..consumed);
                }
                FrameStep::Incomplete => fill(&self.stream, &mut self.buf),
                FrameStep::Corrupt => panic!("the server sent bytes that are no frame"),
            }
        }
    }
}

/// The next frame's bytes as they arrived.
fn raw_frame(stream: &TcpStream, buf: &mut Vec<u8>) -> Vec<u8> {
    loop {
        if let Some(len) = SilkroadFrame::wire_len(buf).filter(|&len| len <= buf.len()) {
            return buf.drain(..len).collect();
        }
        fill(stream, buf);
    }
}

fn fill(mut stream: &TcpStream, buf: &mut Vec<u8>) {
    let mut chunk = [0u8; 4096];
    let n = stream.read(&mut chunk).expect("the server answers");
    assert!(n > 0, "the server closed the connection");
    buf.extend_from_slice(&chunk[..n]);
}
