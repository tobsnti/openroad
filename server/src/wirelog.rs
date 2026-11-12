//! The session log for the client's reading contract (session log v1): with
//! `OPENROAD_WIRE_LOG=<dir>`, every frame a session receives or sends becomes a
//! line `seq<TAB>c2s|s2c<TAB>0xOPCODE<TAB>hex` in a file of its own under
//! `<dir>`, in the order the session saw them, below a header naming the
//! format, the server build and the connection (`agent` or `gateway`). The
//! frames are the logical packets; the transport's framing is not in it.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy)]
pub enum Dir {
    C2s,
    S2c,
}

pub struct WireLog {
    file: Option<File>,
    seq: u64,
}

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
            writeln!(
                file,
                "# sitzung v1\n# server: openroad server {} {}\n# verbindung: {connection}",
                env!("CARGO_PKG_VERSION"),
                env!("OPENROAD_BUILD_COMMIT"),
            )
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
        let dir = match dir {
            Dir::C2s => "c2s",
            Dir::S2c => "s2c",
        };
        let hex: String = body.iter().map(|b| format!("{b:02x}")).collect();
        writeln!(file, "{}\t{dir}\t0x{opcode:04X}\t{hex}", self.seq)
            .expect("wire log write failed");
    }
}
