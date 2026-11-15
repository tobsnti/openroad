//! The session log for the client's reading contract (session log v1): with
//! `OPENROAD_WIRE_LOG=<dir>`, every frame a session receives or sends becomes a
//! line `seq<TAB>c2s|s2c<TAB>0xOPCODE<TAB>hex<TAB>t_ms` in a file of its own
//! under `<dir>`, in the order the session saw them, below a header naming the
//! format, the server build and the connection (`agent` or `gateway`). The
//! frames are the logical packets; the transport's framing is not in it. The
//! two login requests carry the password in clear text, so their bodies are
//! written as `-`, the format's mark for a body that was not recorded. `t_ms`
//! is the Unix time in milliseconds, as in the bench's events; readers of the
//! first four columns are not affected by it. Each line is one write, so a
//! reader during the run never sees half of one.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy)]
pub enum Dir {
    C2s,
    S2c,
}

pub struct WireLog {
    file: Option<File>,
    seq: u64,
}

/// The gateway login (0x6102) and the agent login (0x6103).
const CARRIES_PASSWORD: [u16; 2] = [0x6102, 0x6103];

/// Logs opened by this process, so that two worlds in one test run never
/// write the same file.
static OPENED: AtomicU64 = AtomicU64::new(0);

impl WireLog {
    /// The log of one `connection`; a no-op unless `OPENROAD_WIRE_LOG` is set.
    pub fn open(connection: &str) -> WireLog {
        let file = std::env::var_os("OPENROAD_WIRE_LOG").map(|dir| {
            let n = OPENED.fetch_add(1, Ordering::Relaxed) + 1;
            let name = format!("{connection}-{}-{n}.sitzung", std::process::id());
            let path = PathBuf::from(dir).join(name);
            let mut file =
                std::fs::create_dir_all(path.parent().expect("a log file has a directory"))
                    .and_then(|_| File::create(&path))
                    .expect("OPENROAD_WIRE_LOG is not a writable directory");
            let header = format!(
                "# sitzung v1\n# server: openroad server {} {}\n# verbindung: {connection}\n\
                 # spalten: seq, richtung (s2c|c2s), opcode, rumpf hex (- = nicht mitgeschrieben), t_ms\n",
                env!("CARGO_PKG_VERSION"),
                env!("OPENROAD_BUILD_COMMIT"),
            );
            file.write_all(header.as_bytes())
                .expect("wire log write failed");
            file
        });
        WireLog { file, seq: 0 }
    }

    pub fn record(&mut self, dir: Dir, opcode: u16, body: &[u8]) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        self.seq += 1;
        let (dir, withheld) = match dir {
            Dir::C2s => ("c2s", CARRIES_PASSWORD.contains(&opcode)),
            Dir::S2c => ("s2c", false),
        };
        let hex: String = match withheld {
            true => "-".to_owned(),
            false => body.iter().map(|b| format!("{b:02x}")).collect(),
        };
        let t_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let line = format!("{}\t{dir}\t0x{opcode:04X}\t{hex}\t{t_ms}\n", self.seq);
        file.write_all(line.as_bytes())
            .expect("wire log write failed");
    }
}
