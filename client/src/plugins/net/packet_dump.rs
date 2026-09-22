//! Per-opcode log of every packet payload for offline inspection:
//! `packet_dump/<opcode>.log` gets one `<RFC3339-ms UTC> <hex payload> <E|P>`
//! line per received packet, appended across runs.
//!
//! The third column is the frame's wire `0x8000` bit — `E` = the body arrived
//! blowfish-encrypted, `P` = plaintext (#459). Without it a length read out of
//! an old log is ambiguous: an encrypted frame's payload is padded to the
//! block, so a real body length cannot be told from a padded one. The flag is
//! a *suffix*, so `cut -d' ' -f2` still yields the hex payload and older lines
//! — which simply lack a third column — stay parseable, with their encryption
//! state unknown. Payloads are recorded *before* deserialization so
//! unknown/unhandled opcodes are logged too, and file handles are cached per
//! opcode so the hot receive path only pays for a buffered line write.
//!
//! Sent packets go to the `c2s/<opcode>.log` subdirectory instead, recorded before
//! the frame is serialized so the body is plaintext even once the outbound
//! encryption policy is on. The separate directory is what keeps the two
//! directions apart: several opcodes (`0x2001`, `0x6100`, ...) are used in
//! both, so a shared file would interleave request and response bodies.

use bevy::log::warn;
use bevy::prelude::Resource;
use chrono::SecondsFormat;
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;

const DUMP_DIR: &str = "packet_dump";
/// Subdirectory of [`DUMP_DIR`] holding the client → server direction.
const SENT_SUBDIR: &str = "c2s";

/// How many received frames [`PacketDump::recent`] keeps in memory.
///
/// The ring exists so a driving process can read the frames a request
/// provoked *without* tailing the log files, over the remote protocol the
/// client already speaks. It is filled from [`PacketDump::dump`] — the one
/// call the receive loop already makes — precisely so it cannot drift from
/// `packet_dump/`: a second, independently placed hook is how a "the log says
/// X but the RPC says Y" mystery gets made. No in-tree consumer yet.
pub const RECENT_FRAMES: usize = 256;
/// Cap on the payload bytes retained *per* frame. A 16-bit length field allows a
/// 64 KiB body, so an uncapped ring is a 16 MiB resident buffer in the worst
/// case. Above this the retained hex is cut and `truncated` says so; `len` is
/// always the true body length, and the full body is in the log file.
const RECENT_PAYLOAD_BYTES: usize = 4096;

/// One received frame as the log saw it. `hex` is the same payload hex the log
/// line carries, `encrypted` the same `E`/`P` bit.
#[derive(Clone)]
pub struct RecentFrame {
    pub ts: String,
    pub opcode: u16,
    /// True body length, even when `hex` was cut at [`RECENT_PAYLOAD_BYTES`].
    pub len: usize,
    pub hex: String,
    pub truncated: bool,
    pub encrypted: bool,
}

#[derive(Resource)]
pub struct PacketDump {
    /// Dump root; only tests point it anywhere but [`DUMP_DIR`].
    root: PathBuf,
    /// Keyed by (subdirectory, opcode) so the two directions never share a
    /// file handle for an opcode both of them use.
    files: HashMap<(&'static str, u16), File>,
    /// Set after the first unrecoverable I/O error so we warn once instead of
    /// spamming the log every packet.
    failed: bool,
    /// The last [`RECENT_FRAMES`] received frames, oldest first.
    recent: VecDeque<RecentFrame>,
}

impl Default for PacketDump {
    fn default() -> Self {
        Self {
            root: dump_root(),
            files: HashMap::new(),
            failed: false,
            recent: VecDeque::new(),
        }
    }
}

/// Where the dump goes. `PACKET_DUMP_DIR` overrides [`DUMP_DIR`] because two
/// clients running at once — the only way to exercise party, exchange, trade
/// or a stall against a real server — would otherwise append into the same
/// `<opcode>.log` and interleave two sessions into one file, which destroys
/// exactly the property the dump exists for: that a line is one packet of one
/// session. Env rather than config, like `SCENE`/`NETCHECK`/`BRP_EXTRAS_PORT`:
/// it identifies the *run*, not the installation.
fn dump_root() -> PathBuf {
    match std::env::var("PACKET_DUMP_DIR") {
        Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(DUMP_DIR),
    }
}

impl PacketDump {
    /// Record a received (server → client) payload. `encrypted` is the frame's
    /// wire `0x8000` bit, recorded as the line's third column.
    pub fn dump(&mut self, opcode: u16, data: &[u8], encrypted: bool) {
        self.remember(opcode, data, encrypted);
        self.write("", opcode, data, encrypted);
    }

    /// Push a received frame into the in-memory ring. Called from [`Self::dump`]
    /// and nowhere else, so the ring and the per-opcode log are fed by
    /// the same event; unlike the file it survives `failed`, since a full disk
    /// should not empty the ring.
    fn remember(&mut self, opcode: u16, data: &[u8], encrypted: bool) {
        let keep = data.len().min(RECENT_PAYLOAD_BYTES);
        let mut hex = String::with_capacity(keep * 2);
        for byte in &data[..keep] {
            let _ = write!(hex, "{byte:02x}");
        }
        if self.recent.len() == RECENT_FRAMES {
            self.recent.pop_front();
        }
        self.recent.push_back(RecentFrame {
            ts: chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            opcode,
            len: data.len(),
            hex,
            truncated: keep < data.len(),
            encrypted,
        });
    }

    /// The newest `limit` received frames, newest first, optionally restricted to
    /// `opcodes`. An empty `opcodes` slice means "every opcode".
    pub fn recent(&self, limit: usize, opcodes: &[u16]) -> Vec<RecentFrame> {
        self.recent
            .iter()
            .rev()
            .filter(|f| opcodes.is_empty() || opcodes.contains(&f.opcode))
            .take(limit)
            .cloned()
            .collect()
    }

    /// Record a sent (client → server) payload. Call before serialization so
    /// the recorded body is plaintext regardless of the encryption policy —
    /// hence the `P` column on every c2s line: it describes the bytes in the
    /// log, not what went on the wire.
    pub fn dump_sent(&mut self, opcode: u16, data: &[u8]) {
        self.write(SENT_SUBDIR, opcode, data, false);
    }

    fn write(&mut self, subdir: &'static str, opcode: u16, data: &[u8], encrypted: bool) {
        if self.failed {
            return;
        }
        if let Err(e) = self.try_dump(subdir, opcode, data, encrypted) {
            warn!("packet_dump: disabled after I/O error on {opcode:#06x}.log: {e}");
            self.failed = true;
        }
    }

    fn try_dump(
        &mut self,
        subdir: &'static str,
        opcode: u16,
        data: &[u8],
        encrypted: bool,
    ) -> std::io::Result<()> {
        if !self.files.contains_key(&(subdir, opcode)) {
            let dir = self.root.join(subdir);
            std::fs::create_dir_all(&dir)?;
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(format!("{opcode:#06x}.log")))?;
            self.files.insert((subdir, opcode), file);
        }
        let mut line = String::with_capacity(32 + data.len() * 2);
        line.push_str(&chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true));
        line.push(' ');
        for byte in data {
            let _ = write!(line, "{byte:02x}");
        }
        line.push(' ');
        line.push(if encrypted { 'E' } else { 'P' });
        line.push('\n');
        self.files
            .get_mut(&(subdir, opcode))
            .expect("inserted above")
            .write_all(line.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `0x2001` (and the login opcodes) travel in both directions, so a shared
    /// per-opcode file would interleave request and response bodies and make
    /// the log useless. Sent packets must land under `c2s/`, received ones
    /// stay at the root.
    #[test]
    fn the_two_directions_never_share_a_file() {
        let root = std::env::temp_dir().join(format!("openroad-dump-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut dump = PacketDump {
            root: root.clone(),
            ..Default::default()
        };

        dump.dump(0x2001, &[0xAA], false);
        dump.dump_sent(0x2001, &[0xBB]);

        let received = std::fs::read_to_string(root.join("0x2001.log")).expect("s2c log");
        let sent = std::fs::read_to_string(root.join("c2s").join("0x2001.log")).expect("c2s log");
        assert!(received.trim_end().ends_with(" aa P"), "got {received:?}");
        assert!(sent.trim_end().ends_with(" bb P"), "got {sent:?}");

        std::fs::remove_dir_all(&root).expect("clean up");
    }

    /// The ring must answer with the *same* payload hex the log line carries and
    /// in newest-first order, since a probe asks "what came back just now"; and
    /// it must be a ring, not a growing log, because the bot process is
    /// long-lived.
    #[test]
    fn the_recent_ring_mirrors_the_log_newest_first() {
        let root = std::env::temp_dir().join(format!("openroad-dump-ring-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut dump = PacketDump {
            root: root.clone(),
            ..Default::default()
        };

        dump.dump(0x3020, &[0xB5, 0xA8], true);
        dump.dump(0xB045, &[0x01], false);
        // c2s frames stay out of the ring: it reports what was *received*,
        // and mixing directions is the mistake `c2s/` exists to stop.
        dump.dump_sent(0x7045, &[0x02]);

        let all = dump.recent(10, &[]);
        assert_eq!(all.len(), 2, "sent frames must not enter the ring");
        assert_eq!(all[0].opcode, 0xB045, "newest first");
        assert_eq!(all[0].hex, "01");
        assert!(!all[0].encrypted);
        assert_eq!(all[1].opcode, 0x3020);
        assert_eq!(all[1].hex, "b5a8");
        assert!(all[1].encrypted);
        // …the very hex the log line carries.
        let log = std::fs::read_to_string(root.join("0x3020.log")).expect("s2c log");
        assert_eq!(log.lines().next().unwrap().split(' ').nth(1), Some("b5a8"));

        let filtered = dump.recent(10, &[0x3020]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].opcode, 0x3020);
        assert!(dump.recent(1, &[]).len() == 1, "limit is honoured");

        // The ring is bounded: RECENT_FRAMES + 5 frames leave RECENT_FRAMES.
        for _ in 0..RECENT_FRAMES + 5 {
            dump.dump(0x3057, &[0xFF], false);
        }
        assert_eq!(dump.recent(usize::MAX, &[]).len(), RECENT_FRAMES);

        // A body longer than the per-frame cap keeps its true length and says so.
        dump.dump(0x3013, &[0xAB; RECENT_PAYLOAD_BYTES + 16], false);
        let big = &dump.recent(1, &[0x3013])[0];
        assert_eq!(big.len, RECENT_PAYLOAD_BYTES + 16);
        assert_eq!(big.hex.len(), RECENT_PAYLOAD_BYTES * 2);
        assert!(big.truncated);

        std::fs::remove_dir_all(&root).expect("clean up");
    }

    /// #459: a dump line used to be `<ts> <hex>`, so a padded encrypted body
    /// and a plaintext one of the same length were indistinguishable after the
    /// fact. The flag is the third column — deliberately a suffix, so the
    /// documented `cut -d' ' -f2 | xxd -r -p` recipe and every pre-existing
    /// log line keep working.
    #[test]
    fn each_line_records_the_frames_encryption_bit() {
        let root = std::env::temp_dir().join(format!("openroad-dump-enc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut dump = PacketDump {
            root: root.clone(),
            ..Default::default()
        };

        dump.dump(0x3020, &[0xB5, 0xA8], true);
        dump.dump(0x3020, &[0xB5, 0xA8], false);

        let log = std::fs::read_to_string(root.join("0x3020.log")).expect("s2c log");
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 2, "got {log:?}");
        assert!(lines[0].ends_with(" b5a8 E"), "got {:?}", lines[0]);
        assert!(lines[1].ends_with(" b5a8 P"), "got {:?}", lines[1]);
        // The payload stays field 2 for `cut -d' ' -f2`.
        for line in lines {
            assert_eq!(line.split(' ').nth(1), Some("b5a8"), "got {line:?}");
        }

        std::fs::remove_dir_all(&root).expect("clean up");
    }
}
