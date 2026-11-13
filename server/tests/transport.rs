//! The server's side of a connection, against a client built from the codec the
//! client uses: the handshake, the security bytes on every client frame, any
//! split of the stream, and the multi-part wrapping.

use std::sync::{Arc, RwLock};

use bytes::{Bytes, BytesMut};
use server::transport::{frame, Broken, Inbound, Transport};
use sro_net::blowfish::Blowfish;
use sro_net::crc::CRC;
use sro_net::frame::SilkroadFrame;
use sro_net::key_exchange::{calc_challenge, calc_key, g_pow_x_mod_p, key_transform_value};
use sro_net::security::{SilkroadSecurity, SilkroadSecurityState};
use sro_net::sequence::Sequence;
use sro_net::stream::{next_packet, FrameStep};

const IDENT: &[u8] = b"\x09\x00SR_Client\x00";
const LOGIN: &[u8] = b"secret login";

fn plain() -> Arc<RwLock<SilkroadSecurityState>> {
    Arc::new(RwLock::new(SilkroadSecurityState::new()))
}

/// Opcode and body of the one frame in `wire`.
fn single(wire: &[u8]) -> (u16, Bytes) {
    let mut wire = wire.to_vec();
    let Ok((_, SilkroadFrame::Packet { opcode, data, .. })) =
        SilkroadFrame::parse(&mut wire, plain())
    else {
        panic!("a single frame");
    };
    (opcode, data)
}

/// The client's half, computed as `client/src/net/handshake.rs` does, writing
/// its frames with the client's serializer.
struct Client {
    security: Arc<RwLock<SilkroadSecurityState>>,
    secret: u32,
    public: u32,
    server_public: u32,
    handshake_key: u64,
}

impl Client {
    /// Reads the opening; the client and the bytes of its answer.
    fn answer(opening: &[u8]) -> (Client, Vec<u8>) {
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
    fn challenged(&self, reply: &[u8]) {
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

    fn send(&self, opcode: u16, body: &[u8], encrypted: bool) -> Vec<u8> {
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
    fn next_sequence_byte(&self) -> u8 {
        let mut sequence = self.security.read().unwrap().context.sequence;
        sequence.next()
    }
}

/// `bytes` handed to the server `split` bytes at a time.
fn feed(server: &mut Transport, bytes: &[u8], split: usize) -> Inbound {
    let mut all = Inbound::default();
    for part in bytes.chunks(split) {
        let inbound = server.receive(part).expect("accepted");
        all.frames.extend(inbound.frames);
        all.reply.extend(inbound.reply);
    }
    all
}

fn connected(split: usize) -> (Transport, Client) {
    let (mut server, opening) = Transport::open();
    let (client, answer) = Client::answer(&opening);
    let challenge = feed(&mut server, &answer, split);
    assert!(challenge.frames.is_empty());
    client.challenged(&challenge.reply);
    let done = feed(&mut server, &client.send(0x9000, &[], false), split);
    assert!(done.frames.is_empty() && done.reply.is_empty());
    (server, client)
}

/// Byte by byte or in larger pieces, the same frames come out, including an
/// encrypted one.
#[test]
fn any_split_of_the_stream_gives_the_same_frames() {
    for split in [1, 2, 3, 5, 8, 13, 4096] {
        let (mut server, client) = connected(split);
        let stream = [
            client.send(0x2001, IDENT, false),
            client.send(0x6102, LOGIN, true),
            client.send(0x2002, &[], false),
        ]
        .concat();
        let frames = feed(&mut server, &stream, split).frames;
        let expected = [
            (0x2001, Bytes::from_static(IDENT)),
            (0x6102, Bytes::from_static(LOGIN)),
            (0x2002, Bytes::new()),
        ];
        assert_eq!(frames, expected, "split {split}");
    }
}

/// The frame that ends the handshake and an encrypted frame in one read: the
/// second is read with the key the first one switched on.
#[test]
fn a_frame_glued_behind_the_handshake_is_read_with_the_session_key() {
    let (mut server, opening) = Transport::open();
    let (client, answer) = Client::answer(&opening);
    client.challenged(&server.receive(&answer).unwrap().reply);
    let glued = [
        client.send(0x9000, &[], false),
        client.send(0x6102, LOGIN, true),
    ]
    .concat();
    let frames = server.receive(&glued).unwrap().frames;
    assert_eq!(frames, [(0x6102, Bytes::from_static(LOGIN))]);
}

#[test]
fn a_wrong_crc_ends_the_connection() {
    let (mut server, client) = connected(4096);
    let mut tampered = client.send(0x2001, IDENT, false);
    tampered[5] = tampered[5].wrapping_add(1);
    assert_eq!(server.receive(&tampered).err(), Some(Broken::SecurityBytes));
}

/// A replayed frame carries a sequence byte the server has used up.
#[test]
fn a_replayed_frame_ends_the_connection() {
    let (mut server, client) = connected(4096);
    let mut replay = client.send(0x2002, &[], false);
    server.receive(&replay).unwrap();
    // Two positions of the sequence can share a value; replay one that does not.
    while client.next_sequence_byte() == replay[4] {
        replay = client.send(0x2002, &[], false);
        server.receive(&replay).unwrap();
    }
    assert_eq!(server.receive(&replay).err(), Some(Broken::SecurityBytes));
}

#[test]
fn a_multi_part_opcode_leaves_only_inside_0x600d() {
    for opcode in [0xA100, 0xA104] {
        let mut wire = frame(opcode, &[0x01]);
        let len = wire.len();
        assert_eq!(u16::from_le_bytes([wire[2], wire[3]]), 0x600D);
        let FrameStep::Ready {
            opcode: inner,
            data,
            consumed,
            ..
        } = next_packet(&mut wire, &plain())
        else {
            panic!("the client reassembles it");
        };
        assert_eq!((inner, &data[..], consumed), (opcode, &[0x01][..], len));
    }
    let wire = frame(0xA101, &[0x00]);
    assert_eq!(u16::from_le_bytes([wire[2], wire[3]]), 0xA101);
}
