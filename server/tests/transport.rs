//! The server's side of a connection, against a client built from the codec the
//! client uses: the handshake, the security bytes on every client frame, any
//! split of the stream, and the multi-part wrapping.

use bytes::Bytes;
use server::transport::{frame, Broken, Transport};
use sro_net::stream::{next_packet, FrameStep};

mod common;
use common::{feed, plain, Client};

const IDENT: &[u8] = b"\x09\x00SR_Client\x00";
const LOGIN: &[u8] = b"secret login";

fn connected(split: usize) -> (Transport, Client) {
    connected_on(Transport::open(), split)
}

/// The handshake on a transport that has sent its `opening`.
fn connected_on((mut server, opening): (Transport, Vec<u8>), split: usize) -> (Transport, Client) {
    let (client, answer) = Client::answer(&opening);
    let challenge = feed(&mut server, &answer, split);
    assert!(challenge.frames.is_empty());
    client.challenged(&challenge.reply);
    assert!(!server.established());
    let done = feed(&mut server, &client.send(0x9000, &[], false), split);
    assert!(done.frames.is_empty() && done.reply.is_empty());
    assert!(server.established());
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

/// The opening's seeds are bytes; a draw over the whole `u32` would leave
/// this range in all but one of 2^24 openings.
#[test]
fn the_opening_carries_both_seeds_as_bytes() {
    for _ in 0..1000 {
        let (_, opening) = Transport::open();
        let (_, body) = common::single(&opening);
        let seed = |at: usize| u32::from_le_bytes(body[at..at + 4].try_into().unwrap());
        assert!(
            seed(9) <= 0xFF && seed(13) <= 0xFF,
            "{:#x} {:#x}",
            seed(9),
            seed(13)
        );
    }
}

/// On the edge seeds, 0 included, the client's sequence and CRC bytes are the
/// ones the server checks for, frame after frame.
#[test]
fn a_client_on_the_edge_seeds_gets_its_frames_through() {
    for (sequence_seed, crc_seed) in [0, 1, 255]
        .into_iter()
        .flat_map(|s| [(s, 0), (s, 1), (s, 255)])
    {
        let (mut server, client) = connected_on(Transport::seeded(sequence_seed, crc_seed), 4096);
        let stream = [
            client.send(0x2001, IDENT, false),
            client.send(0x6102, LOGIN, true),
            client.send(0x2002, &[], false),
        ]
        .concat();
        let frames = feed(&mut server, &stream, 4096).frames;
        assert_eq!(frames.len(), 3, "seeds {sequence_seed} {crc_seed}");
    }
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
    // A replay shows only where the next sequence byte differs from the
    // replayed one, and for some seeds the sequence settles on a single value
    // (seen once in this test: it waited for a change for an hour). So the
    // replay goes to the first connection whose next byte differs.
    for _ in 0..64 {
        let (mut server, client) = connected(4096);
        let replay = client.send(0x2002, &[], false);
        server.receive(&replay).unwrap();
        if client.next_sequence_byte() != replay[4] {
            assert_eq!(server.receive(&replay).err(), Some(Broken::SecurityBytes));
            return;
        }
    }
    panic!("64 connections without a replay that shows");
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
