// The transport seam: what `SilkroadConnection` needs from a byte pipe.
//
// Idea: the handshake and the two ECS loops are the only code in this client that
// touches a socket -- 14 call sites in 3 files, measured at 746c22edf. Everything
// else reaches the connection through its crossbeam channels (44 of the 50 files
// that name `SilkroadConnection`), and `conn.stream` itself is read at exactly two
// places (`plugins/net/plugin.rs`). So the socket type can move behind a trait
// without touching any of them -- and the handshake gains a second, socket-free
// caller to be tested with, which is the point of doing it now.
//
// `io::Read + io::Write` already covers 8 of those 14 sites (`write_all` is a
// provided method of `Write`). The two verbs below are everything else that is
// called THROUGH the seam, and they are here because a browser `WebSocket` has
// no equivalent for them: it has no socket deadline at all, so both become
// no-ops there. Named `*_deadline` rather than `*_timeout` so the trait method
// stays apart from `TcpStream`'s inherent one.
//
// `set_nonblocking` is deliberately NOT here, although it is one of the 14
// sites: its single caller switches the socket before it ever becomes a
// transport, so it is a concern of whatever constructs the connection, not of
// the pipe. Having it in the trait only produced a dead method, which is how
// this was found (`clippy --all-targets`: "method `set_nonblocking` is never
// used").
//
// This does NOT make the handshake browser-ready: `handshake::read_frame` still
// blocks, and a blocking read on a browser's main thread can never finish --
// the bytes arrive in a callback that cannot run while the thread is blocked.
// That is a separate, larger change (the handshake becomes poll-shaped); this
// trait is its precondition, not its substitute.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// A bidirectional, ordered byte pipe. Ordered is not a detail: every frame
/// carries a sequence byte and a CRC chained off the handshake seeds
/// (`frame.rs`), so a transport that may drop or reorder bytes desynchronises
/// every later frame.
pub trait SilkroadTransport: Read + Write + Send + 'static {
    /// The current bound on how long a read may wait, so a caller that sets one
    /// can put the previous value back.
    fn read_deadline(&mut self) -> std::io::Result<Option<Duration>>;

    fn set_read_deadline(&mut self, deadline: Option<Duration>) -> std::io::Result<()>;
}

impl SilkroadTransport for TcpStream {
    fn read_deadline(&mut self) -> std::io::Result<Option<Duration>> {
        self.read_timeout()
    }

    fn set_read_deadline(&mut self, deadline: Option<Duration>) -> std::io::Result<()> {
        self.set_read_timeout(deadline)
    }
}
