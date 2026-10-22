//! Shared reader for the gitignored `packet_dump/` tree.
//!
//! The captures are runtime material — a body carries the player's name and
//! position — so they stay out of the repository and the replay sweeps read
//! them from the user's own tree behind `OPENROAD_PACKET_DUMP_DIR`.

use std::fs;
use std::path::{Path, PathBuf};

use bytes::Bytes;

/// One dump line is `<timestamp> <hex body> <marker>`; the body is the frame
/// payload without the opcode header.
pub fn bodies(path: &Path) -> Vec<Bytes> {
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
pub fn logs(root: &Path, name: &str) -> Vec<PathBuf> {
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

/// The opt-in dump tree, or `None` when the caller has no captures.
pub fn dump_root() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var("OPENROAD_PACKET_DUMP_DIR").ok()?);
    root.is_dir().then_some(root)
}
