//! The world without sockets: frames in, frames out.

use bytes::Bytes;
use server::outbox::CAPACITY;
use server::world::{Event, Reason, World};

/// 0x7046 locks the client's talk requests until its answer arrives, so it
/// gets exactly one — also when its body does not decode.
#[test]
fn a_talk_is_answered_exactly_once_even_when_it_does_not_decode() {
    let mut world = World::new();
    let s = world.connect();
    world.receive(
        s,
        0x7046,
        Bytes::from_static(&[0x02, 0x10, 0x00, 0x00, 0x01]),
    );
    world.receive(s, 0x7046, Bytes::from_static(&[0x02, 0x10, 0x00, 0x00]));
    let frames = world.take_frames(s);
    assert_eq!(frames.len(), 2);
    for frame in frames {
        assert_eq!(
            (frame.opcode, frame.body.as_ref()),
            (0xB046, &[0x02, 0x00, 0x00][..])
        );
    }
    let rejected = Event::Rejected {
        session: s,
        opcode: 0x7046,
        len: 4,
        reason: Reason::DecodeError,
    };
    assert_eq!(world.take_events(), vec![rejected]);
}

#[test]
fn a_keep_alive_is_taken_without_an_answer() {
    let mut world = World::new();
    let s = world.connect();
    world.receive(s, 0x2002, Bytes::new());
    assert!(world.take_frames(s).is_empty());
    assert!(world.take_events().is_empty());
}

/// A longer body is not read as its prefix.
#[test]
fn a_frame_with_extra_bytes_is_rejected_not_truncated() {
    let mut world = World::new();
    let s = world.connect();
    world.receive(s, 0x2002, Bytes::from_static(&[0x00]));
    let rejected = Event::Rejected {
        session: s,
        opcode: 0x2002,
        len: 1,
        reason: Reason::DecodeError,
    };
    assert_eq!(world.take_events(), vec![rejected]);
}

#[test]
fn a_request_without_a_handler_is_reported_not_answered() {
    let mut world = World::new();
    let s = world.connect();
    // 0x7021 go to (region 25000; 969, 0, 1419) — decodes, nothing handles it yet.
    let walk = Bytes::from_static(&[0x01, 0xA8, 0x61, 0xC9, 0x03, 0x00, 0x00, 0x8B, 0x05]);
    world.receive(s, 0x7021, walk);
    assert!(world.take_frames(s).is_empty());
    let rejected = Event::Rejected {
        session: s,
        opcode: 0x7021,
        len: 9,
        reason: Reason::NotHandled,
    };
    assert_eq!(world.take_events(), vec![rejected]);
}

#[test]
fn a_session_that_stops_reading_is_cut_off() {
    let mut world = World::new();
    let s = world.connect();
    for _ in 0..=CAPACITY {
        world.receive(
            s,
            0x7046,
            Bytes::from_static(&[0x02, 0x10, 0x00, 0x00, 0x01]),
        );
    }
    assert_eq!(world.take_events(), vec![Event::Overflowed(s)]);
    assert!(world.take_frames(s).is_empty(), "the session is gone");
}
