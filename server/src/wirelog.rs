//! The session log a test writes for the client's reading contract
//! (decision 0011, session log v1): with `OPENROAD_WIRE_LOG=<dir>`, every frame
//! a session receives or sends becomes a line `seq<TAB>c2s|s2c<TAB>0xOPCODE<TAB>hex`
//! in `<dir>/session-<id>.sitzung`, in the order the session saw them, below a
//! header naming the format and the connection.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

#[derive(Clone, Copy)]
pub enum Dir {
    C2s,
    S2c,
}

pub struct WireLog {
    file: Option<File>,
    seq: u64,
}

impl WireLog {
    /// The log for agent session `id`; a no-op unless `OPENROAD_WIRE_LOG` is set.
    pub fn open(id: u32) -> WireLog {
        let file = std::env::var_os("OPENROAD_WIRE_LOG").map(|dir| {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir).expect("OPENROAD_WIRE_LOG is not a writable directory");
            let mut file = File::create(dir.join(format!("session-{id}.sitzung")))
                .expect("OPENROAD_WIRE_LOG is not a writable directory");
            writeln!(
                file,
                "# sitzung v1\n# server: openroad server {}\n# verbindung: agent",
                env!("CARGO_PKG_VERSION")
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
