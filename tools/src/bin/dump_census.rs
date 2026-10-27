//! `dump_census` — read every captured frame back through our own decoder.
//!
//! Idea: `packet_dump/` is the only record of what a server actually sent us,
//! and it has grown past a quarter of a million lines that nobody has read with
//! the client's own eyes. This walks the dumps and hands each body to
//! `Packet::deserialize` — the same registry the client dispatches through, so a
//! census result and the client's behaviour cannot drift apart.
//!
//! **A leftover is a finding, not an error.** A decoder that reads a prefix of a
//! body and stops is what an unfinished layout looks like, so the verdicts stay
//! apart: `exact` (re-encoding reproduces the captured bytes), `short`
//! (re-encoding is a strict prefix — N bytes never read), `lossy` (shorter but
//! not a prefix: a tail the decoder reads and drops on purpose), `differs`
//! (same length or longer and not equal — the shape that needs a look), `error`
//! (the decoder refused), `panic` (it crashed on a captured body — a finding of
//! its own), `no_encoder` (two acks deliberately cannot be re-encoded) and
//! `unwired` (no type bound to this opcode). The last column is the working
//! list: an opcode with frames and no type is work only nobody has looked at.
//!
//! **Where a frame came from decides what it can prove.** A body under
//! `proxy/**` was spoken to the *original* client; everything else came from our
//! own stub or peer, which can invent — measured: 50 `0x3041` frames with skill
//! names on the wire, two extra bytes in a lobby frame. Origin and verdict are
//! separate columns, never summed.
//!
//! The line format is `packet_dump.rs`'s: `<RFC3339-ms> <hex> <E|P>`, the flag
//! absent before 2026-08-14. Two traps, both measured on this tree's
//! 247,632-line corpus:
//!
//! 1. **An empty body is written as two spaces**, so `cut -d' ' -f2` — which
//!    that module's own doc suggests — yields the *flag*, and a naive parser
//!    drops the line. 155,244 lines (63%) are such frames and 150,244 of them
//!    are one opcode (`0x2002`): a census that drops them reports a third of the
//!    corpus it has.
//! 2. **The direction mark is absolute, the writer is not.** `c2s` means
//!    client→server whoever wrote the file, so our client and our stub file the
//!    same frame in the same subtree; what differs is whether that writer sent
//!    or received it. Hence the `writer` column beside `dir`: "the stub sent it"
//!    and "the client received it" are one frame from two witnesses.
//! 3. **Hex case differs between writers.** Measured: every body with hex
//!    letters in `/tmp/s9/dump` (stub `cb3d47a`) is **upper** case (26 of 26)
//!    and every one in the client's dumps and in `capture-orig` is **lower**
//!    case (2,827 and 5,326). File names are lower case in both. So a
//!    case-sensitive string compare across writers finds nothing — this parser
//!    reads either.
//! 4. **Some bodies are withheld on purpose, in three forms** — 181 lines,
//!    every one a login opcode: an inline `<withheld:credentials>`, a `#`
//!    comment where the line was removed, and the hex with the secret masked
//!    in place (`...0400xxxxxxxx0200`). They count as redacted, not as parse
//!    failures, so the number does not read like a parser bug
//!    ([`is_redacted`]).
//! 5. **`E` does not mean unreadable.** It is the frame's `0x8000` bit, kept for
//!    length ambiguity; the dumped payload is the body the client went on to
//!    deserialize. Only 8.0% of `E` bodies are a multiple of the blowfish block,
//!    so they are not stored block-padded. They decode like any other line and
//!    are counted separately so a reader can split them out again.
//!
//! Usage:
//!   cargo run --release -p tools --bin dump_census -- <root>... [--unwired]
//!              [--min-frames N] [--hist N] [--swap-directions] [--original]

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use bytes::Bytes;
use packets::{Packet, PacketError};

/// Default dump root, the one the client writes (`packet_dump.rs`).
const DEFAULT_ROOT: &str = "packet_dump";
/// Histogram entries printed per cell unless `--hist` says otherwise.
const DEFAULT_HIST: usize = 4;

/// What our decoder made of one body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Verdict {
    /// Decoded, and re-encoding reproduces the captured bytes.
    Exact,
    /// Decoded, and re-encoding is a strict prefix: the tail was never read.
    Short(usize),
    /// Decoded, and re-encoding is **shorter** than the capture without being a
    /// prefix of it: the decoder read a tail it does not keep, and the encoder
    /// writes the flag that says the tail is absent. `MovementResponse`
    /// (`0xB021`) does exactly that by design, which is 28,328 of this corpus'
    /// 28,609 non-exact round trips — a documented decision, not a defect.
    Lossy,
    /// Decoded, and re-encoding has the same length or is longer, but differs:
    /// the bytes themselves disagree. This is the shape that needs a look.
    Differs,
    /// The decoder refused this body.
    Error,
    /// The decoder **panicked** on a captured body. Its own verdict rather than
    /// the end of the census: a frame that crashes our decoder is a crash
    /// vector, and this is how it gets named.
    Panic,
    /// Decoded, but the type cannot be encoded, so the round trip says nothing.
    /// Deliberate in two cases: the registry binds both directions per opcode,
    /// so the server→client acks in `agent::alchemy` answer `From<_> for Bytes`
    /// with `unreachable!`.
    NoEncoder,
    /// No type is bound to this opcode.
    Unwired,
}

/// One captured line, as the dump writes it.
#[derive(PartialEq, Eq, Debug)]
struct Line {
    body: Vec<u8>,
    /// The frame's `0x8000` bit: `Some(true)` = `E`, `Some(false)` = `P`,
    /// `None` = a line written before the flag existed.
    encrypted: Option<bool>,
}

/// Split one dump line. The body sits between the first space and the optional
/// one-letter flag, **and it may be empty** — see the module header.
fn parse_line(line: &str) -> Option<Line> {
    let line = line.trim_end_matches(['\n', '\r']);
    let (_timestamp, rest) = line.split_once(' ')?;
    let (rest, encrypted) = match rest.trim_end().rsplit_once(' ') {
        Some((head, "E")) => (head, Some(true)),
        Some((head, "P")) => (head, Some(false)),
        _ => match rest.trim() {
            "E" => ("", Some(true)),
            "P" => ("", Some(false)),
            other => (other, None),
        },
    };
    let hex = rest.trim();
    if hex.len() % 2 != 0 {
        return None;
    }
    let mut body = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        body.push((hi * 16 + lo) as u8);
    }
    Some(Line { body, encrypted })
}

/// Did someone withhold this body on purpose?
///
/// Three forms, all measured on this corpus — together 181 lines, every one of
/// them a login opcode:
///
/// * `<withheld:credentials>` where the hex would be (156 lines),
/// * a `#` comment saying the line was removed because it held a plaintext
///   password (25),
/// * the hex with the secret masked in place, keeping the body length
///   (`...0400xxxxxxxx0200`, 2).
///
/// A census that called these parse failures would report a parser bug that
/// does not exist, which is why they get their own count.
fn is_redacted(line: &str) -> bool {
    line.contains("<withheld") || line.trim_start().starts_with('#') || line.contains("xxxx")
}

/// `0x3013.log` -> `0x3013`. Anything else is not a dump file.
fn opcode_of(path: &Path) -> Option<u16> {
    let stem = path.file_stem()?.to_str()?.to_ascii_lowercase();
    let digits = stem.strip_prefix("0x")?;
    u16::from_str_radix(digits, 16).ok()
}

/// Run one body through the client's own registry and ask how much of the frame
/// it accounts for.
///
/// Both halves run under `catch_unwind`, each for a measured reason: a decoder
/// panic is a finding we want named rather than fatal, and `into_serialize` is
/// `unreachable!` for two registered types — a census that ignored that died on
/// `0xB150` after a 2m42s build.
fn judge(opcode: u16, body: &[u8]) -> Verdict {
    let decoded =
        std::panic::catch_unwind(|| Packet::deserialize(opcode, Bytes::copy_from_slice(body)));
    let packet = match decoded {
        Err(_) => return Verdict::Panic,
        Ok(Err(PacketError::UnknownOpcode(_))) => return Verdict::Unwired,
        Ok(Err(_)) => return Verdict::Error,
        Ok(Ok(packet)) => packet,
    };
    let encoded =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| packet.into_serialize()));
    let Ok((_opcode, encoded)) = encoded else {
        return Verdict::NoEncoder;
    };
    if encoded.as_ref() == body {
        Verdict::Exact
    } else if encoded.len() < body.len() {
        if body.starts_with(encoded.as_ref()) {
            Verdict::Short(body.len() - encoded.len())
        } else {
            Verdict::Lossy
        }
    } else {
        Verdict::Differs
    }
}

/// Where a frame came from, which is what decides whether it can prove a form.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Origin {
    /// Spoken to or by the **original** client: a `proxy/` session of ours, or
    /// a root under `capture-orig/` (named sessions of the real client).
    Original,
    /// Our own stub, peer or bot: a body here may be invented.
    Peer,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Direction {
    /// What the client sent.
    C2s,
    /// What arrived at the client.
    S2c,
}

/// Who recorded a root. Not the same question as [`Direction`]: the direction
/// mark is **absolute** (server→client / client→server), so both peers file the
/// same frame in the same subtree — what differs is whether the writer *sent* or
/// *received* it. "Our stub sent it" and "our client received it" are one frame
/// from two witnesses, and that distinction is what makes a form claim hold.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Writer {
    /// Written by the client (`packet_dump.rs`). No record file.
    Client,
    /// Written by the stub server, which also writes `session.jsonl`.
    Stub,
}

/// What a root's own shape says about it.
///
/// The client writes received frames to `<root>/<opcode>.log` and sent ones to
/// `<root>/c2s/<opcode>.log` (`packet_dump.rs`), and the stub does the same —
/// measured on `/tmp/s9/dump` (stub `cb3d47a`, 45 records): 33 of 33 one-way
/// opcodes vote for that layout, 0 against. A foreign writer could still file it
/// the other way round, so the layout is **read off the root** rather than
/// assumed — see [`inverted_root`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Layout {
    /// `c2s/` holds *received* frames and the root holds sent ones.
    inverted: bool,
    origin: Origin,
    writer: Writer,
}

/// Classify a dump file by its path relative to the root it was found under.
fn classify(relative: &Path, layout: Layout) -> (Origin, Direction) {
    let parts: Vec<_> = relative
        .parent()
        .map(|dir| {
            dir.components()
                .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let origin = if parts.first().is_some_and(|first| first == "proxy") {
        Origin::Original
    } else {
        layout.origin
    };
    let in_c2s_dir = parts.iter().any(|part| part == "c2s");
    let direction = match (in_c2s_dir, layout.inverted) {
        (true, false) | (false, true) => Direction::C2s,
        (false, false) | (true, true) => Direction::S2c,
    };
    (origin, direction)
}

/// Does this root hold its directions the other way round?
///
/// Idea: a root written by our stub ships a `session.jsonl` whose records carry
/// their own `dir`, so the layout can be **measured** instead of guessed. Every
/// opcode the record file saw in exactly one direction votes for the subtree its
/// `.log` actually sits in; the majority decides. A root without that file keeps
/// the client's layout, and `--swap-directions` forces it.
///
/// Measured on `fixes/origcrash/dump-lauf-c` (431 records): 23 of its 24 root
/// files are `s2c`-only and 13 of its 14 `c2s/` files are `c2s`-only, i.e. that
/// stub root uses the *client's* layout — which is why this is read per root and
/// not switched on "is a stub".
fn inverted_root(root: &Path) -> Option<bool> {
    let records = std::fs::read_to_string(root.join("session.jsonl")).ok()?;
    let mut seen: BTreeMap<String, (bool, bool)> = BTreeMap::new();
    for line in records.lines() {
        let Some(opcode) = field(line, "opcode") else {
            continue;
        };
        let Some(direction) = field(line, "dir") else {
            continue;
        };
        let entry = seen.entry(opcode.to_ascii_lowercase()).or_default();
        match direction.as_str() {
            "c2s" => entry.0 = true,
            "s2c" => entry.1 = true,
            _ => {}
        }
    }
    let (mut as_client, mut as_inverted) = (0usize, 0usize);
    for (opcode, (is_c2s, is_s2c)) in &seen {
        let one_way = is_c2s != is_s2c;
        if !one_way {
            continue;
        }
        let in_root = root.join(format!("{opcode}.log")).is_file();
        let in_c2s = root.join("c2s").join(format!("{opcode}.log")).is_file();
        if in_root && !in_c2s {
            if *is_s2c {
                as_client += 1;
            } else {
                as_inverted += 1;
            }
        } else if in_c2s && !in_root {
            if *is_c2s {
                as_client += 1;
            } else {
                as_inverted += 1;
            }
        }
    }
    (as_client + as_inverted > 0).then_some(as_inverted > as_client)
}

/// One string field out of a JSONL record, without a JSON parser: the writer
/// emits `"key":"value"` with no escapes in either, and a census must not fail
/// on a half-written last line.
fn field<'a>(line: &'a str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":\"");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

#[derive(Default)]
struct Tally {
    frames: usize,
    /// The distinct bodies behind `frames`. Kept whole rather than hashed: the
    /// corpus is 413,656 frames and most of them are short, so the set costs
    /// tens of MB and needs no collision argument.
    ///
    /// Why it is here at all: **frame count is not evidence width.** `0x3074`
    /// has 2,328 frames and 371 distinct bodies, none from the original client
    /// — our own peer repeating itself. `0x6100` has 3,116 frames and **one**
    /// body, 1,028 of them from the original client — a form that is proven and
    /// a version dependency that is not measured by a single one of them.
    bodies: std::collections::BTreeSet<Vec<u8>>,
    empty: usize,
    encrypted: usize,
    unflagged: usize,
    exact: usize,
    short: usize,
    short_bytes: usize,
    lossy: usize,
    differs: usize,
    error: usize,
    panics: usize,
    no_encoder: usize,
    unwired: usize,
    lengths: BTreeMap<usize, usize>,
    /// First body byte — the discriminator most bodies in this protocol lead
    /// with. Named as what it is, not interpreted.
    first_bytes: BTreeMap<u8, usize>,
}

impl Tally {
    fn add(&mut self, line: &Line, verdict: Verdict) {
        self.frames += 1;
        self.bodies.insert(line.body.clone());
        match line.encrypted {
            Some(true) => self.encrypted += 1,
            Some(false) => {}
            None => self.unflagged += 1,
        }
        *self.lengths.entry(line.body.len()).or_default() += 1;
        match line.body.first() {
            Some(byte) => *self.first_bytes.entry(*byte).or_default() += 1,
            None => self.empty += 1,
        }
        match verdict {
            Verdict::Exact => self.exact += 1,
            Verdict::Short(tail) => {
                self.short += 1;
                self.short_bytes += tail;
            }
            Verdict::Lossy => self.lossy += 1,
            Verdict::Differs => self.differs += 1,
            Verdict::Error => self.error += 1,
            Verdict::Panic => self.panics += 1,
            Verdict::NoEncoder => self.no_encoder += 1,
            Verdict::Unwired => self.unwired += 1,
        }
    }
}

/// `key:count` pairs, most frequent first, at most `top` of them. Ties keep the
/// lower key, or two runs over the same corpus print different tables.
fn histogram<K: Copy + Ord>(
    counts: &BTreeMap<K, usize>,
    top: usize,
    key: impl Fn(K) -> String,
) -> String {
    let mut pairs: Vec<_> = counts.iter().map(|(k, v)| (*k, *v)).collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    pairs
        .iter()
        .take(top)
        .map(|(k, count)| format!("{}:{count}", key(*k)))
        .collect::<Vec<_>>()
        .join(",")
}

fn log_files(root: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            log_files(&path, into);
        } else if opcode_of(&path).is_some() && path.extension().is_some_and(|e| e == "log") {
            into.push(path);
        }
    }
}

struct Options {
    roots: Vec<PathBuf>,
    unwired_only: bool,
    min_frames: usize,
    hist: usize,
    /// Force the inverted layout on roots that carry no `session.jsonl`.
    swap: bool,
    /// Call every root an original-client capture, whatever its path says.
    original: bool,
}

fn options() -> Options {
    let mut options = Options {
        roots: Vec::new(),
        unwired_only: false,
        min_frames: 0,
        hist: DEFAULT_HIST,
        swap: false,
        original: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--unwired" => options.unwired_only = true,
            "--swap-directions" => options.swap = true,
            "--original" => options.original = true,
            "--hist" => {
                options.hist = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(DEFAULT_HIST);
            }
            "--min-frames" => {
                options.min_frames = args.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            }
            other => options.roots.push(PathBuf::from(other)),
        }
    }
    if options.roots.is_empty() {
        options.roots.push(PathBuf::from(DEFAULT_ROOT));
    }
    options
}

fn main() {
    let options = options();
    // Two registered types panic by design (see `judge`); over a corpus this
    // size the default hook would print the same backtrace thousands of times.
    // The count is the finding, not the message.
    std::panic::set_hook(Box::new(|_| {}));
    let mut tallies: BTreeMap<(u16, Origin, Writer, Direction), Tally> = BTreeMap::new();
    let mut files = 0usize;
    let mut unreadable = 0usize;
    let mut redacted = 0usize;

    for root in &options.roots {
        let mut paths = Vec::new();
        log_files(root, &mut paths);
        // `capture-orig/**` are named sessions of the real client, so the whole
        // root is original — the `proxy/` component inside a client dump is the
        // other way in (see `classify`).
        let named_capture = root.to_string_lossy().contains("capture-orig");
        let layout = Layout {
            inverted: inverted_root(root).unwrap_or(options.swap),
            origin: if options.original || named_capture {
                Origin::Original
            } else {
                Origin::Peer
            },
            writer: if root.join("session.jsonl").is_file() {
                Writer::Stub
            } else {
                Writer::Client
            },
        };
        for path in paths {
            let (Some(opcode), Ok(file)) = (opcode_of(&path), File::open(&path)) else {
                continue;
            };
            files += 1;
            let relative = path.strip_prefix(root).unwrap_or(&path);
            let (origin, direction) = classify(relative, layout);
            let tally = tallies
                .entry((opcode, origin, layout.writer, direction))
                .or_default();
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                match parse_line(&line) {
                    Some(parsed) => {
                        let verdict = judge(opcode, &parsed.body);
                        tally.add(&parsed, verdict);
                    }
                    None if line.trim().is_empty() => {}
                    // A body someone deliberately withheld: the two login
                    // opcodes in the proxy roots carry `<withheld:credentials>`
                    // where the hex would be. Counted as what it is, so nobody
                    // hunts a parser bug over 181 lines of redaction.
                    None if is_redacted(&line) => redacted += 1,
                    None => unreadable += 1,
                }
            }
        }
    }

    println!(
        "# dump_census: {files} files, {} groups, {redacted} redacted bodies, \
         {unreadable} unreadable lines",
        tallies.len()
    );
    println!(
        "opcode\torigin\twriter\tdir\tframes\tdistinct\tempty\tenc\tunflagged\texact\tshort\tshort_bytes\tlossy\tdiffers\terror\tpanic\tno_encoder\tunwired\tlengths\tfirst_bytes"
    );
    // Row order: the deterministic key order by default, and for the working
    // list the order that produces work — biggest opcode first, its
    // original-client row before the peer's. Sorting by frames alone is what
    // put `0x3074` (2,328 frames, 371 bodies, no original) above `0x6100`
    // (3,116 frames, one body, 1,028 of them original) in the first pass.
    let opcode_frames: BTreeMap<u16, usize> =
        tallies
            .iter()
            .fold(BTreeMap::new(), |mut totals, (key, tally)| {
                *totals.entry(key.0).or_default() += tally.frames;
                totals
            });
    let mut rows: Vec<_> = tallies.iter().collect();
    if options.unwired_only {
        rows.sort_by_key(|(key, tally)| {
            let total = opcode_frames.get(&key.0).copied().unwrap_or(0);
            (
                std::cmp::Reverse(total),
                key.0,
                key.1,
                tally.bodies.len(),
                key.2,
                key.3,
            )
        });
    }
    for ((opcode, origin, writer, direction), tally) in rows {
        if tally.frames < options.min_frames {
            continue;
        }
        if options.unwired_only && tally.unwired == 0 {
            continue;
        }
        let lengths = histogram(&tally.lengths, options.hist, |len| len.to_string());
        let first_bytes = histogram(&tally.first_bytes, options.hist, |byte| {
            format!("{byte:02x}")
        });
        println!(
            "{opcode:#06x}\t{origin:?}\t{writer:?}\t{direction:?}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{lengths}\t{first_bytes}",
            tally.frames,
            tally.bodies.len(),
            tally.empty,
            tally.encrypted,
            tally.unflagged,
            tally.exact,
            tally.short,
            tally.short_bytes,
            tally.lossy,
            tally.differs,
            tally.error,
            tally.panics,
            tally.no_encoder,
            tally.unwired,
        );
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// `(body, flag)` of a line, so the cases below read as a table.
    fn parsed(line: &str) -> Option<(Vec<u8>, Option<bool>)> {
        parse_line(line).map(|line| (line.body, line.encrypted))
    }

    /// The empty body is the format's trap: two spaces, so the flag lands where
    /// a naive split expects the payload. 63% of this tree's corpus is such a
    /// line, and the pre-flag form of it is a bare timestamp.
    #[test]
    fn a_line_parses_with_an_empty_body_and_without_a_flag() {
        let cases = [
            ("2026-08-17T12:01:58.685Z  P", (vec![], Some(false))),
            (
                "2026-08-20T13:13:17.872Z 01e8 E",
                (vec![0x01, 0xe8], Some(true)),
            ),
            ("2026-08-11T09:43:27.961Z 01e8", (vec![0x01, 0xe8], None)),
            ("2026-08-11T09:43:27.961Z ", (vec![], None)),
        ];
        for (line, want) in cases {
            assert_eq!(parsed(line).expect("parses"), want, "{line}");
        }
    }

    /// All three withholding forms, as found in the corpus. A census that read
    /// them as parse failures would report a parser bug that does not exist.
    #[test]
    fn a_withheld_body_is_recognised_in_all_three_forms() {
        for line in [
            "2026-10-02T16:10:46.323Z <withheld:credentials> E",
            "# 1 LoginRequest line(s) removed: they carry the account password in plaintext",
            "2026-09-24T11:40:52.075Z 1609007465737475736572310400xxxxxxxx0200 P",
        ] {
            assert!(is_redacted(line), "{line}");
        }
        assert!(!is_redacted("2026-08-20T13:13:17.872Z 01e8 E"));
    }

    #[test]
    fn half_a_byte_is_not_a_body() {
        assert!(parsed("2026-08-20T13:13:17.872Z 01e P").is_none());
        assert!(parsed("2026-08-20T13:13:17.872Z zz P").is_none());
    }

    #[test]
    fn the_file_name_is_the_opcode() {
        assert_eq!(opcode_of(Path::new("packet_dump/0x3013.log")), Some(0x3013));
        assert_eq!(opcode_of(Path::new("packet_dump/0xA102.log")), Some(0xa102));
        assert_eq!(opcode_of(Path::new("packet_dump/notes.log")), None);
    }

    /// Only `proxy/` as the first component is the original client; `c2s`
    /// anywhere in the path is the other direction of whatever the root's
    /// layout says.
    #[test]
    fn the_path_carries_origin_and_direction() {
        let client = Layout {
            inverted: false,
            origin: Origin::Peer,
            writer: Writer::Client,
        };
        let cases = [
            ("proxy/0x3013.log", (Origin::Original, Direction::S2c)),
            ("proxy/c2s/0x7021.log", (Origin::Original, Direction::C2s)),
            ("admin6/c2s/0x7021.log", (Origin::Peer, Direction::C2s)),
            ("0x3013.log", (Origin::Peer, Direction::S2c)),
        ];
        for (path, want) in cases {
            assert_eq!(classify(Path::new(path), client), want, "{path}");
        }
        // An inverted root swaps both sides, and nothing else about it.
        let inverted = Layout {
            inverted: true,
            origin: Origin::Original,
            writer: Writer::Stub,
        };
        assert_eq!(
            classify(Path::new("0x3013.log"), inverted),
            (Origin::Original, Direction::C2s)
        );
        assert_eq!(
            classify(Path::new("c2s/0x7021.log"), inverted),
            (Origin::Original, Direction::S2c)
        );
    }

    #[test]
    fn a_record_field_is_read_without_a_json_parser() {
        let line = r#"{"ts":"2026-10-02T09:12:44.953Z","dir":"c2s","opcode":"0x2001","len":12}"#;
        assert_eq!(field(line, "dir").as_deref(), Some("c2s"));
        assert_eq!(field(line, "opcode").as_deref(), Some("0x2001"));
        assert_eq!(field(line, "len"), None); // a number, not a string
        assert_eq!(field(r#"{"dir":"#, "dir"), None); // half-written last line
    }

    /// A root that ships `session.jsonl` votes on its own layout: each opcode
    /// the record saw in one direction only says which subtree that direction
    /// lives in. Here `0x3013` arrived and sits in the root while `0x7021` was
    /// sent and sits in `c2s/`, which is the client's layout.
    #[test]
    fn a_root_with_a_record_file_votes_on_its_own_layout() {
        let root = std::env::temp_dir().join("dump_census_layout_vote");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("c2s")).expect("temp dir");
        std::fs::write(root.join("0x3013.log"), "").expect("write");
        std::fs::write(root.join("c2s").join("0x7021.log"), "").expect("write");
        std::fs::write(
            root.join("session.jsonl"),
            "{\"dir\":\"s2c\",\"opcode\":\"0x3013\"}\n{\"dir\":\"c2s\",\"opcode\":\"0x7021\"}\n",
        )
        .expect("write");
        assert_eq!(inverted_root(&root), Some(false));
        // and the record file is also what names the writer
        assert!(root.join("session.jsonl").is_file());

        // The same files with the directions exchanged read as inverted.
        std::fs::write(
            root.join("session.jsonl"),
            "{\"dir\":\"c2s\",\"opcode\":\"0x3013\"}\n{\"dir\":\"s2c\",\"opcode\":\"0x7021\"}\n",
        )
        .expect("write");
        assert_eq!(inverted_root(&root), Some(true));

        // No record file, no vote — the caller keeps its default.
        std::fs::remove_file(root.join("session.jsonl")).expect("remove");
        assert_eq!(inverted_root(&root), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Ordered by count, not by key, or two runs over the same corpus print
    /// different tables.
    #[test]
    fn the_histogram_is_ordered_by_count_then_key() {
        let counts = BTreeMap::from([(24usize, 7), (14, 9), (44, 9)]);
        let decimal = |len: usize| len.to_string();
        assert_eq!(histogram(&counts, 3, decimal), "14:9,44:9,24:7");
        assert_eq!(histogram(&counts, 1, decimal), "14:9");
        assert_eq!(histogram(&counts, 0, decimal), "");
    }
}
