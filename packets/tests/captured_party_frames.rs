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

use std::fs;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use packets::agent::party::{PartyData, PartyJoinResponse, PartyUpdate};

/// One dump line is `<timestamp> <hex body> <marker>`; the body is the frame
/// payload without the opcode header.
fn bodies(path: &Path) -> Vec<Bytes> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter_map(|hex| {
            (hex.len() % 2 == 0)
                .then(|| {
                    (0..hex.len())
                        .step_by(2)
                        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
                        .collect::<Option<Vec<u8>>>()
                })
                .flatten()
        })
        .map(Bytes::from)
        .collect()
}

/// Every `<dir>/**/<name>` the dump tree holds — one file per opcode per
/// session, so the same opcode appears once per account directory.
fn logs(root: &Path, name: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let direct = root.join(name);
    if direct.is_file() {
        found.push(direct);
    }
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let nested = entry.path().join(name);
            if nested.is_file() {
                found.push(nested);
            }
        }
    }
    found
}

fn dump_root() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var("OPENROAD_PACKET_DUMP_DIR").ok()?);
    root.is_dir().then_some(root)
}

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
