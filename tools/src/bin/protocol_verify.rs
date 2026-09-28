//! `protocol_verify` — replay our own recorded corpus through our own parser.
//!
//! Idea: `packet_dump/**/<opcode>.log` holds the *exact* byte slice the client
//! hands to `Packet::deserialize` (`client/src/plugins/net/plugin.rs:289` dumps
//! `data` and then deserializes that same `data`). So the dumps are not an
//! approximation of the wire — they are the parser's input, recorded. Replaying
//! them is therefore a real regression test of `packets/`, not a simulation.
//!
//! The trick that makes "silent" errors visible without touching the derive
//! macro: `Deserialize` is generated as a *reader* over a cursor and never
//! reports how many bytes it consumed, so a body that stops early parses
//! "successfully" while leaving payload on the floor. But every wired packet is
//! also `Serialize`, so re-serializing the parsed value reconstructs exactly the
//! bytes the parser accounted for. Comparing that against the recorded frame
//! splits into three verdicts:
//!
//! * identical              -> OK
//! * a strict *prefix*      -> RESIDUAL: the parser stopped n bytes early
//! * same length, different -> ROUNDTRIP: a field is written back differently
//!   (bool/enum normalisation, a padded string, a mis-sized field)
//!
//! and a longer re-serialization is reported as ROUNDTRIP too (the parser
//! invented bytes, e.g. a defaulted optional).
//!
//! Verdicts per opcode are the worst of its frames. Opcodes declared in
//! `packets/src/lib.rs` with no recorded frame are UNCOVERED; recorded opcodes
//! `Packet::deserialize` rejects with `UnknownOpcode` are UNWIRED.
//!
//! Run: `make protocol verify` (writes `docs/protocol/CORPUS-VERIFICATION.md`),
//! or `cargo run -q -p tools --bin protocol_verify -- --help`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use bytes::Bytes;
use packets::{Packet, PacketError};

/// One recorded frame: where it came from, when, and its body.
struct Frame {
    /// Corpus lane the line came from, e.g. `packet_dump` or `packet_dump/c2s`.
    lane: String,
    ts: String,
    body: Bytes,
    /// Pre-2026-08-14 line: no `E`/`P` flag column. Those dumps were written
    /// *after* Blowfish decryption but *before* the block padding was
    /// stripped, so a zero tail on one of them is a capture artefact, not
    /// payload — see [`is_block_padding`].
    legacy: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Verdict {
    Ok,
    /// The parse succeeded but the type's `Serialize` is a deliberate empty
    /// stub (server → client packets the client never sends write no bytes),
    /// so the re-serialization says nothing about the parse. Reported apart
    /// from RESIDUAL because it is a property of *this method*, not a defect:
    /// calling it "0 bytes accounted for" would be a false accusation.
    NoWriteback,
    Roundtrip,
    Residual,
    ParseError,
    Unwired,
}

impl Verdict {
    fn label(self) -> &'static str {
        match self {
            Verdict::Ok => "OK",
            Verdict::NoWriteback => "NO-WRITEBACK",
            Verdict::Roundtrip => "ROUNDTRIP",
            Verdict::Residual => "RESIDUAL",
            Verdict::ParseError => "PARSE-ERROR",
            Verdict::Unwired => "UNWIRED",
        }
    }
}

struct OpcodeReport {
    opcode: u16,
    frames: usize,
    /// Frames whose "residual" is the old dumper's block padding
    /// ([`is_block_padding`]) — counted, not held against the parser.
    padded: usize,
    /// Frames recorded in a `c2s` lane, i.e. bodies *we* sent. An opcode that
    /// carries traffic both ways (0x3080) parses only one of the two
    /// directions here, because `Packet::deserialize` maps an opcode to one
    /// type; the split is printed so that is not read as a parser defect.
    sent: usize,
    lanes: BTreeSet<String>,
    verdict: Verdict,
    /// One line naming the cause, with the offending frame's ts + hex excerpt.
    finding: String,
    /// Number of frames per verdict, worst first.
    counts: BTreeMap<Verdict, usize>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "protocol_verify — replay packet_dump/ through packets/\n\
             \n\
             Usage: cargo run -q -p tools --bin protocol_verify -- [options]\n\
             \n\
             Options:\n\
             \x20 --dump <dir>   corpus root (default: packet_dump)\n\
             \x20 --lib <file>   opcode declarations (default: packets/src/lib.rs)\n\
             \x20 --out <file>   write the markdown report (default: stdout summary only)\n\
             \x20 --strict       exit 1 when any opcode is worse than OK\n"
        );
        return;
    }
    let opt = |name: &str, default: &str| -> String {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_else(|| default.to_string())
    };
    let dump_dir = PathBuf::from(opt("--dump", "packet_dump"));
    let lib_file = PathBuf::from(opt("--lib", "packets/src/lib.rs"));
    let out = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let strict = args.iter().any(|a| a == "--strict");

    let declared = read_declared_opcodes(&lib_file);
    if declared.is_empty() {
        eprintln!(
            "no opcodes declared in {} — wrong --lib?",
            lib_file.display()
        );
        std::process::exit(2);
    }
    let corpus = read_corpus(&dump_dir);
    if corpus.is_empty() {
        eprintln!("no frames under {} — wrong --dump?", dump_dir.display());
        std::process::exit(2);
    }

    let mut reports: Vec<OpcodeReport> = Vec::new();
    for (opcode, frames) in &corpus {
        reports.push(verify_opcode(*opcode, frames));
    }
    let uncovered: Vec<u16> = declared
        .iter()
        .copied()
        .filter(|op| !corpus.contains_key(op))
        .collect();

    print_summary(&reports, &uncovered, &declared);

    if let Some(path) = out {
        let md = render_markdown(&reports, &uncovered, &declared, &dump_dir);
        if let Some(parent) = Path::new(&path).parent() {
            let _ = fs::create_dir_all(parent);
        }
        match fs::write(&path, md) {
            Ok(()) => println!("\nwrote {path}"),
            Err(e) => {
                eprintln!("could not write {path}: {e}");
                std::process::exit(2);
            }
        }
    }

    if strict && reports.iter().any(|r| r.verdict != Verdict::Ok) {
        std::process::exit(1);
    }
}

/// The opcode table is the same line-anchored form `packets/src/lib.rs` names as
/// the counting rule, so this tool and `scripts/check_opcode_ledger.py` cannot
/// disagree about what "declared" means.
fn read_declared_opcodes(lib: &Path) -> BTreeSet<u16> {
    let text = match fs::read_to_string(lib) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("could not read {}: {e}", lib.display());
            return BTreeSet::new();
        }
    };
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("0x") else {
            continue;
        };
        let hex: String = rest.chars().take(4).collect();
        if hex.len() != 4 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        if !rest[4..].trim_start().starts_with("=>") {
            continue;
        }
        if let Ok(op) = u16::from_str_radix(&hex, 16) {
            out.insert(op);
        }
    }
    out
}

/// Every `<hex>.log` under the corpus root, at any depth. The file name is the
/// opcode; lanes (`c2s`, `proxy`, per-account capture dirs, ...) are kept only
/// for provenance — the parser is direction-agnostic because
/// `Packet::deserialize` is. The lane names are not spelled out here: a capture
/// directory is named after the account it was recorded on, and
/// `scripts/check_no_private.py` refuses an account name in a patch.
fn read_corpus(root: &Path) -> BTreeMap<u16, Vec<Frame>> {
    let mut out: BTreeMap<u16, Vec<Frame>> = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(stem) = name.strip_suffix(".log") else {
                continue;
            };
            let Some(hex) = stem.strip_prefix("0x") else {
                continue;
            };
            let Ok(opcode) = u16::from_str_radix(hex, 16) else {
                continue;
            };
            let lane = path
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for line in text.lines() {
                // `<RFC3339-ms> <hex payload> [E|P]`; an empty body leaves the
                // middle column empty, and pre-2026-08-14 lines have no flag.
                let mut cols = line.split(' ');
                let Some(ts) = cols.next() else { continue };
                if ts.is_empty() {
                    continue;
                }
                let payload = cols.next().unwrap_or("");
                let Some(body) = decode_hex(payload) else {
                    continue;
                };
                let legacy = !matches!(cols.next(), Some("E") | Some("P"));
                out.entry(opcode).or_default().push(Frame {
                    lane: lane.clone(),
                    ts: ts.to_string(),
                    body: Bytes::from(body),
                    legacy,
                });
            }
        }
    }
    out
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

fn hex_of(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for byte in b {
        let _ = write!(s, "{byte:02x}");
    }
    s
}

/// At most `max` bytes of hex, with an ellipsis marker, so a 1.5 kB
/// CHARACTER_DATA frame does not drown the report.
fn hex_excerpt(b: &[u8], max: usize) -> String {
    if b.len() <= max {
        hex_of(b)
    } else {
        format!("{}… ({} bytes)", hex_of(&b[..max]), b.len())
    }
}

/// Is `tail` the Blowfish block padding of a pre-2026-08-14 dump rather than
/// unread payload? Three conditions together, and they are not a guess: the
/// line has no `E`/`P` flag (so it predates the dumper that records one), the
/// tail is all zero, and the *frame* — 4 bytes of header (`u16 size`,
/// `u16 opcode`) plus the body — is exactly the parsed length rounded up to the
/// next multiple of 8, the cipher's block size.
///
/// Positive control from the corpus: `0xa102` from the same server is 28 bytes
/// on every flagless line (23 parsed + 5 zero) and 23 bytes on every flagged
/// one, and `0xa103` is 4 (1 parsed + 3 zero) vs 1. Nothing about the server
/// changed between them; the dumper did.
fn is_block_padding(frame: &Frame, parsed_len: usize, tail: &[u8]) -> bool {
    if !frame.legacy || !tail.iter().all(|b| *b == 0) {
        return false;
    }
    let framed = 4 + parsed_len;
    framed.div_ceil(8) * 8 == 4 + frame.body.len()
}

fn verify_opcode(opcode: u16, frames: &[Frame]) -> OpcodeReport {
    let mut counts: BTreeMap<Verdict, usize> = BTreeMap::new();
    let mut lanes = BTreeSet::new();
    let mut worst = Verdict::Ok;
    let mut finding = String::new();
    let mut padded = 0;
    let mut sent = 0;
    for frame in frames {
        lanes.insert(frame.lane.clone());
        if frame.lane.split('/').next_back() == Some("c2s") {
            sent += 1;
        }
        let outcome = verify_frame(opcode, frame);
        if outcome.padded {
            padded += 1;
        }
        *counts.entry(outcome.verdict).or_insert(0) += 1;
        if outcome.verdict > worst {
            worst = outcome.verdict;
            finding = outcome.why;
        }
    }
    if worst == Verdict::Ok {
        finding = format!("all {} frames re-serialize byte-identically", frames.len());
        if padded > 0 {
            let _ = write!(
                finding,
                " ({padded} of them after dropping the old dumper's block padding)"
            );
        }
    }
    OpcodeReport {
        opcode,
        frames: frames.len(),
        padded,
        sent,
        lanes,
        verdict: worst,
        finding,
        counts,
    }
}

/// One frame's result: its verdict, the line that names the cause, and whether
/// the difference was the corpus's own padding artefact.
struct FrameOutcome {
    verdict: Verdict,
    why: String,
    padded: bool,
}

impl FrameOutcome {
    fn new(verdict: Verdict, why: String) -> Self {
        Self {
            verdict,
            why,
            padded: false,
        }
    }
}

fn verify_frame(opcode: u16, frame: &Frame) -> FrameOutcome {
    let original = frame.body.clone();
    let parsed = catch_unwind(AssertUnwindSafe(|| {
        Packet::deserialize(opcode, original.clone())
    }));
    let packet = match parsed {
        Err(_) => {
            return FrameOutcome::new(
                Verdict::ParseError,
                format!(
                    "parser PANICKED on {} {} `{}`",
                    frame.lane,
                    frame.ts,
                    hex_excerpt(&frame.body, 48)
                ),
            )
        }
        Ok(Err(PacketError::UnknownOpcode(_))) => {
            return FrameOutcome::new(
                Verdict::Unwired,
                format!(
                    "no parser declared; first {} {} `{}`",
                    frame.lane,
                    frame.ts,
                    hex_excerpt(&frame.body, 48)
                ),
            )
        }
        Ok(Err(e)) => {
            return FrameOutcome::new(
                Verdict::ParseError,
                format!(
                    "{e} — {} {} `{}`",
                    frame.lane,
                    frame.ts,
                    hex_excerpt(&frame.body, 48)
                ),
            )
        }
        Ok(Ok(p)) => p,
    };

    let reser = catch_unwind(AssertUnwindSafe(|| packet.into_serialize()));
    let Ok((_, back)) = reser else {
        return FrameOutcome::new(
            Verdict::ParseError,
            format!(
                "serializer PANICKED after a successful parse of {} {} `{}`",
                frame.lane,
                frame.ts,
                hex_excerpt(&frame.body, 48)
            ),
        );
    };

    if back.as_ref() == frame.body.as_ref() {
        return FrameOutcome::new(Verdict::Ok, String::new());
    }
    if back.is_empty() && !frame.body.is_empty() {
        return FrameOutcome::new(
            Verdict::NoWriteback,
            format!(
                "parses, but the type's `From<_> for Bytes` writes nothing (send-side stub), \
                 so the replay cannot check the parse — {} {} `{}`",
                frame.lane,
                frame.ts,
                hex_excerpt(&frame.body, 48)
            ),
        );
    }
    if back.len() < frame.body.len() && frame.body.starts_with(back.as_ref()) {
        let tail = &frame.body[back.len()..];
        if is_block_padding(frame, back.len(), tail) {
            return FrameOutcome {
                verdict: Verdict::Ok,
                why: String::new(),
                padded: true,
            };
        }
        return FrameOutcome::new(
            Verdict::Residual,
            format!(
                "{} byte(s) left unread at offset {} — tail `{}` ({} {}, frame `{}`)",
                tail.len(),
                back.len(),
                hex_excerpt(tail, 32),
                frame.lane,
                frame.ts,
                hex_excerpt(&frame.body, 48)
            ),
        );
    }
    let first_diff = frame
        .body
        .iter()
        .zip(back.iter())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| frame.body.len().min(back.len()));
    FrameOutcome::new(
        Verdict::Roundtrip,
        format!(
            "re-serialization differs at offset {} (recorded {} bytes, written back {}) — recorded `{}` vs written `{}` ({} {})",
            first_diff,
            frame.body.len(),
            back.len(),
            hex_excerpt(&frame.body[first_diff.min(frame.body.len())..], 24),
            hex_excerpt(&back[first_diff.min(back.len())..], 24),
            frame.lane,
            frame.ts
        ),
    )
}

fn print_summary(reports: &[OpcodeReport], uncovered: &[u16], declared: &BTreeSet<u16>) {
    let mut by_verdict: BTreeMap<Verdict, usize> = BTreeMap::new();
    for r in reports {
        *by_verdict.entry(r.verdict).or_insert(0) += 1;
    }
    let frames: usize = reports.iter().map(|r| r.frames).sum();
    println!("corpus: {frames} frames over {} opcodes", reports.len());
    println!("declared in packets/: {}", declared.len());
    for verdict in [
        Verdict::Ok,
        Verdict::NoWriteback,
        Verdict::Roundtrip,
        Verdict::Residual,
        Verdict::ParseError,
        Verdict::Unwired,
    ] {
        println!(
            "  {:<12} {}",
            verdict.label(),
            by_verdict.get(&verdict).copied().unwrap_or(0)
        );
    }
    println!("  {:<12} {}", "UNCOVERED", uncovered.len());
    for r in reports {
        if r.verdict != Verdict::Ok {
            println!("{:#06x} {:<12} {}", r.opcode, r.verdict.label(), r.finding);
        }
    }
}

fn render_markdown(
    reports: &[OpcodeReport],
    uncovered: &[u16],
    declared: &BTreeSet<u16>,
    dump_dir: &Path,
) -> String {
    let mut s = String::new();
    let frames: usize = reports.iter().map(|r| r.frames).sum();
    let mut by_verdict: BTreeMap<Verdict, usize> = BTreeMap::new();
    for r in reports {
        *by_verdict.entry(r.verdict).or_insert(0) += 1;
    }
    let count = |v: Verdict| by_verdict.get(&v).copied().unwrap_or(0);

    s.push_str("# Corpus verification\n\n");
    s.push_str(
        "GENERATED FILE — do not edit by hand. Regenerate with `make protocol verify`\n\
         (`tools/src/bin/protocol_verify.rs`). What the first full run found, which\n\
         findings were real defects and which were artefacts of the capture, is written\n\
         up once in\n\
         [`docs/planning/PROTOCOL-corpus-verification-2026-08-24.md`](../planning/PROTOCOL-corpus-verification-2026-08-24.md)\n\
         — put interpretation there, not here: this file is overwritten on every run.\n\n",
    );
    s.push_str(&format!(
        "Every recorded frame under `{}` is replayed through `Packet::deserialize` and\n\
         written back with `into_serialize()`. The dumps are the parser's own input\n\
         (`client/src/plugins/net/plugin.rs` dumps the body it then deserializes), so a\n\
         difference here is normally a defect in `packets/` rather than a capture\n\
         artefact — with two known exceptions the tool now filters or labels itself:\n\
         pre-2026-08-14 lines (no `E`/`P` flag) still carry the 8-byte Blowfish block\n\
         padding, and `c2s` frames are bodies *we* built, so an opcode that carries\n\
         traffic both ways parses only one of its two directions here.\n\n",
        dump_dir.display()
    ));
    s.push_str(
        "| Verdict | Meaning |\n|---|---|\n\
         | OK | every recorded frame re-serializes byte-identically |\n\
         | NO-WRITEBACK | parses, but the type's serializer is a deliberate empty stub, so the replay cannot check the parse at all |\n\
         | RESIDUAL | the frame parsed, but the parser accounted for fewer bytes than were recorded — the silent failure this report exists for |\n\
         | ROUNDTRIP | the re-serialization has the same or greater length but different bytes (normalisation, wrong field width, invented default) |\n\
         | PARSE-ERROR | `Packet::deserialize` returned an error or panicked |\n\
         | UNWIRED | frames recorded, no parser declared in `packets/src/lib.rs` |\n\
         | UNCOVERED | declared in `packets/src/lib.rs`, not one frame in the corpus |\n\n",
    );
    s.push_str(&format!(
        "**{} frames / {} recorded opcodes.** Declared in `packets/src/lib.rs`: {}. \
         OK {} · NO-WRITEBACK {} · RESIDUAL {} · ROUNDTRIP {} · PARSE-ERROR {} · UNWIRED {} · UNCOVERED {}.\n\n",
        frames,
        reports.len(),
        declared.len(),
        count(Verdict::Ok),
        count(Verdict::NoWriteback),
        count(Verdict::Residual),
        count(Verdict::Roundtrip),
        count(Verdict::ParseError),
        count(Verdict::Unwired),
        uncovered.len()
    ));

    s.push_str(
        "A single-frame OK is not a verified layout: it says one recorded shape survives\n\
         the round trip, nothing about the arms the capture never hit. The Frames column\n\
         is therefore part of the result, not decoration.\n\n",
    );

    s.push_str("## Findings\n\n");
    s.push_str("| Opcode | Frames | Result | Finding |\n|---|---|---|---|\n");
    let mut sorted: Vec<&OpcodeReport> = reports.iter().collect();
    sorted.sort_by_key(|r| (std::cmp::Reverse(r.verdict), r.opcode));
    for r in &sorted {
        if r.verdict == Verdict::Ok {
            continue;
        }
        s.push_str(&format!(
            "| `{:#06x}` | {} | {} | {} |\n",
            r.opcode,
            r.frames,
            r.verdict.label(),
            md_escape(&format!(
                "{} [{}{}]",
                r.finding,
                breakdown(&r.counts),
                if r.sent > 0 {
                    format!(
                        "; {} of {} frames are c2s, i.e. our own sends",
                        r.sent, r.frames
                    )
                } else {
                    String::new()
                }
            ))
        ));
    }
    if sorted.iter().all(|r| r.verdict == Verdict::Ok) {
        s.push_str("| — | — | — | no finding: every recorded opcode is OK |\n");
    }

    s.push_str("\n## Clean opcodes\n\n");
    s.push_str("| Opcode | Frames | Result | Finding |\n|---|---|---|---|\n");
    for r in reports.iter().filter(|r| r.verdict == Verdict::Ok) {
        s.push_str(&format!(
            "| `{:#06x}` | {} | OK | {} |\n",
            r.opcode,
            r.frames,
            if r.frames == 1 {
                "1 frame, one shape — round-trips, but not generalisable".to_string()
            } else if r.padded > 0 {
                format!(
                    "{} frames round-trip byte-identically ({} of them are pre-2026-08-14 dumps \
                     that still carry the 8-byte block padding; that zero tail is a capture \
                     artefact, not payload)",
                    r.frames, r.padded
                )
            } else {
                format!("{} frames round-trip byte-identically", r.frames)
            }
        ));
    }

    s.push_str("\n## Uncovered (declared, never recorded)\n\n");
    s.push_str(&format!(
        "{} of {} declared opcodes have no frame in the corpus, so this report says\n\
         nothing about them — they are spec-derived until a capture exists.\n\n",
        uncovered.len(),
        declared.len()
    ));
    let mut line = String::new();
    for (i, op) in uncovered.iter().enumerate() {
        let _ = write!(line, "`{op:#06x}`");
        if i + 1 != uncovered.len() {
            line.push_str(", ");
        }
        if line.len() > 90 {
            s.push_str(&line);
            s.push('\n');
            line.clear();
        }
    }
    if !line.is_empty() {
        s.push_str(&line);
        s.push('\n');
    }

    s.push_str("\n## Lanes\n\n");
    s.push_str("| Opcode | Frames | Lanes |\n|---|---|---|\n");
    for r in reports {
        s.push_str(&format!(
            "| `{:#06x}` | {} | {} |\n",
            r.opcode,
            r.frames,
            r.lanes
                .iter()
                .map(|l| l.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    s
}

/// `12 RESIDUAL, 3 OK` — how the opcode's frames split across verdicts, so a
/// finding that only one capture in fifty hits is not read as the whole opcode.
fn breakdown(counts: &BTreeMap<Verdict, usize>) -> String {
    let mut parts: Vec<(Verdict, usize)> = counts.iter().map(|(v, n)| (*v, *n)).collect();
    parts.sort_by_key(|(v, _)| std::cmp::Reverse(*v));
    parts
        .iter()
        .map(|(v, n)| format!("{n} {}", v.label()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn md_escape(s: &str) -> String {
    s.replace('|', "\\|")
}
