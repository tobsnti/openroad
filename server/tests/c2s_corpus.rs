//! Every request the original client was recorded sending passes strict
//! reading, except the forms `packets` does not model yet, listed below. This
//! keeps the corpus measurement behind ADR 0013 §5 as a guard: a change that
//! stops accepting a real request fails here, and so does a list entry that
//! has since become accepted.
//!
//! The recordings are the user's own and stay outside the repository.
//! `OPENROAD_C2S_ROOTS` names a file with one directory per line (first
//! tab-separated field). Each directory holds the client-to-server half of a
//! session as `0xNNNN.log`, one `timestamp hex [E|P]` line per frame. A frame
//! recorded in several directories counts once.

use std::collections::{BTreeMap, BTreeSet};

use packets::judge::decode_exact;

/// Opcodes with recorded forms that strict reading rejects today.
const NOT_MODELLED: [(u16, &str); 4] = [
    (0x705A, "the five-byte form"),
    (0x7302, "no packet type"),
    (0x730B, "no packet type"),
    (0x730D, "no packet type"),
];

#[test]
#[ignore = "needs recordings: OPENROAD_C2S_ROOTS=<list> cargo test -p server --test c2s_corpus -- --ignored"]
fn every_recorded_original_request_passes_strict_reading() {
    let list = std::env::var("OPENROAD_C2S_ROOTS").expect("OPENROAD_C2S_ROOTS names the list");
    let mut frames = BTreeSet::new();
    for line in std::fs::read_to_string(list)
        .expect("the list is readable")
        .lines()
    {
        let dir = line.split('\t').next().unwrap_or_default().trim();
        if dir.is_empty() || dir.starts_with('#') {
            continue;
        }
        for entry in std::fs::read_dir(dir).expect("a listed directory exists") {
            let path = entry.expect("the directory is readable").path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let hex = name.strip_prefix("0x").and_then(|n| n.strip_suffix(".log"));
            let Some(opcode) = hex.and_then(|h| u16::from_str_radix(h, 16).ok()) else {
                continue;
            };
            let log = std::fs::read_to_string(&path).expect("the log is readable");
            for line in log.lines() {
                let mut fields = line.split_whitespace();
                let (Some(time), body) = (fields.next(), fields.next().unwrap_or_default()) else {
                    continue;
                };
                let body = if matches!(body, "E" | "P") { "" } else { body };
                // A body the recorder withheld (credentials) is not hex.
                if let Some(body) = unhex(body) {
                    frames.insert((time.to_owned(), opcode, body));
                }
            }
        }
    }
    assert!(!frames.is_empty(), "the listed directories hold no frames");

    let mut seen: BTreeMap<u16, (usize, usize)> = BTreeMap::new();
    for (_, opcode, body) in &frames {
        let tally = seen.entry(*opcode).or_default();
        tally.0 += 1;
        tally.1 += usize::from(decode_exact(*opcode, body).is_err());
    }
    let rejected: Vec<_> = seen.iter().filter(|(_, t)| t.1 > 0).collect();
    eprintln!(
        "{} frames, {} opcodes; rejected (opcode, (frames, rejected)): {rejected:x?}",
        frames.len(),
        seen.len()
    );
    let unexpected: Vec<_> = rejected
        .iter()
        .filter(|(op, _)| !NOT_MODELLED.iter().any(|(m, _)| m == *op))
        .collect();
    assert!(
        unexpected.is_empty(),
        "recorded requests rejected: {unexpected:x?}"
    );
    let accepted_now: Vec<_> = NOT_MODELLED
        .iter()
        .filter(|(m, _)| seen.get(m).is_some_and(|t| t.1 == 0))
        .collect();
    assert!(
        accepted_now.is_empty(),
        "accepted now, drop from NOT_MODELLED: {accepted_now:x?}"
    );
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}
