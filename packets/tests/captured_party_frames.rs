//! Replay every captured party frame through the real packet types.
//!
//! IDEA. `packet_dump/` holds 265 party frames from real sessions, and until
//! now no test touched them — which is how three different places came to claim
//! that no sample existed. This replays each one and asserts that decoding and
//! re-encoding returns the captured bytes, so a model that drops a tail or
//! invents a field fails here instead of on the wire.
//!
//! The dumps stay out of the repository: they are runtime material and the
//! roster bodies carry real character names. So the sweep is opt-in through
//! `OPENROAD_PACKET_DUMP_DIR`, and without it the test reports that it found
//! nothing to do rather than passing silently on an empty set.

use std::path::Path;

use bytes::Bytes;
use packets::agent::party::{PartyData, PartyJoinResponse, PartyUpdate};

mod dump;
use dump::{bodies, dump_root, logs};

/// Decode + re-encode every frame of one opcode, returning how many were seen.
fn replay<T>(root: &Path, name: &str) -> usize
where
    T: TryFrom<Bytes> + Clone + Into<Bytes>,
    <T as TryFrom<Bytes>>::Error: std::fmt::Debug,
{
    let mut count = 0;
    for log in logs(root, name) {
        for (index, body) in bodies(&log).into_iter().enumerate() {
            let decoded = T::try_from(body.clone())
                .unwrap_or_else(|e| panic!("{}:{index} did not decode: {e:?}", log.display()));
            let back: Bytes = decoded.into();
            assert_eq!(
                back,
                body,
                "{}:{index} did not survive re-encoding",
                log.display()
            );
            count += 1;
        }
    }
    count
}

#[test]
fn captured_party_frames_round_trip() {
    let Some(root) = dump_root() else {
        eprintln!(
            "skipped: set OPENROAD_PACKET_DUMP_DIR to a packet_dump tree to replay \
             captured party frames"
        );
        return;
    };

    let roster = replay::<PartyData>(&root, "0x3065.log");
    let deltas = replay::<PartyUpdate>(&root, "0x3864.log");
    let join_acks = replay::<PartyJoinResponse>(&root, "0xb067.log");

    eprintln!("replayed {roster}x 0x3065, {deltas}x 0x3864, {join_acks}x 0xB067");
    assert!(
        roster + deltas + join_acks > 0,
        "OPENROAD_PACKET_DUMP_DIR is set but holds no party frames: {}",
        root.display()
    );
}
