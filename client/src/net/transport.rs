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

/// A scripted byte pipe with no socket behind it, used to run the real handshake
/// over a second transport.
///
/// It hands out the peer's bytes in the chunks the script lists, so a frame the
/// peer split across two segments is seen split here too -- the case that used
/// to fail the login (#107). Everything the client writes is kept for the
/// comparison, and the byte counts are exposed so a test can assert that the
/// pipe really carried something: a fixture that silently carried nothing looks
/// exactly like a pass.
///
/// Exhausting the script does NOT mean end of stream. A peer that stops in the
/// middle of a frame without closing is what the handshake's 5-second
/// continuation bound exists for, and `std::net` reports that bound firing as
/// `WouldBlock` on unix -- so that is what an open, empty pipe answers. Reporting
/// `Ok(0)` instead would blame a peer that is still there for an "unexpected end
/// of file", which is the mistake the bound was added to avoid.
///
/// An **empty chunk** in the script is a gap: the peer is still there and said
/// nothing in this poll. That is the one thing a callback-fed transport does
/// between two polls, so it has to be expressible here -- and it answers like an
/// exhausted-but-open script rather than handing out zero bytes, which every
/// caller reads as end of stream.
#[cfg(test)]
pub(crate) struct MemPipe {
    inbound: std::collections::VecDeque<Vec<u8>>,
    outbound: Vec<u8>,
    handed_out: usize,
    reads: usize,
    empty_reads: usize,
    closed: bool,
    /// Red control only: answer an exhausted-but-open script -- or a gap --
    /// with `Ok(0)`, so a test can show that the distinction between a closed
    /// peer and a stalled one is really being measured.
    report_eof_when_empty: bool,
}

#[cfg(test)]
impl MemPipe {
    /// `chunks` is what the peer sends, one `read` per chunk.
    pub(crate) fn new(chunks: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self {
            inbound: chunks.into_iter().collect(),
            outbound: Vec::new(),
            handed_out: 0,
            reads: 0,
            empty_reads: 0,
            closed: false,
            report_eof_when_empty: false,
        }
    }

    /// The peer hangs up once its script runs out.
    pub(crate) fn closing(mut self) -> Self {
        self.closed = true;
        self
    }

    pub(crate) fn reporting_eof_when_empty(mut self) -> Self {
        self.report_eof_when_empty = true;
        self
    }

    /// Everything the client wrote, in order.
    pub(crate) fn written(&self) -> &[u8] {
        &self.outbound
    }

    /// How many bytes the pipe handed to the client. Zero means the fixture
    /// never ran, whatever the rest of the test says.
    pub(crate) fn handed_out(&self) -> usize {
        self.handed_out
    }

    /// How many reads the client needed. This, not the byte count, is what says
    /// a split really happened: a pipe that merged its script into one chunk
    /// hands out the same bytes and would pass a byte-count assertion.
    pub(crate) fn reads(&self) -> usize {
        self.reads
    }

    /// How many polls carried no bytes at all -- a gap, or an exhausted script.
    /// Kept apart from [`Self::reads`] because that one counts only reads that
    /// delivered something, so a test asserting on it cannot tell whether the
    /// gap it scripted was ever reached.
    pub(crate) fn empty_reads(&self) -> usize {
        self.empty_reads
    }
}

#[cfg(test)]
impl Read for MemPipe {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // A gap and an exhausted script answer the same thing, so they share one
        // branch: an empty chunk is consumed here, and from there on this is the
        // "nothing arrived, the peer is still there" case. Copying an empty
        // chunk instead would return `Ok(0)` -- end of stream -- in the middle
        // of a peer that is still sending.
        let gap = self.inbound.front().is_some_and(Vec::is_empty);
        if gap {
            self.inbound.pop_front();
        }
        if gap || self.inbound.is_empty() {
            self.empty_reads += 1;
            return if self.closed || self.report_eof_when_empty {
                Ok(0)
            } else {
                Err(std::io::Error::from(std::io::ErrorKind::WouldBlock))
            };
        }
        let Some(chunk) = self.inbound.front_mut() else {
            unreachable!("the exhausted script and the gap both returned above");
        };
        let n = chunk.len().min(buf.len());
        buf[..n].copy_from_slice(&chunk[..n]);
        chunk.drain(..n);
        if chunk.is_empty() {
            self.inbound.pop_front();
        }
        self.handed_out += n;
        self.reads += 1;
        Ok(n)
    }
}

#[cfg(test)]
impl Write for MemPipe {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.outbound.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
impl SilkroadTransport for MemPipe {
    /// A scripted pipe has no deadline to keep: it never has to wait.
    fn read_deadline(&mut self) -> std::io::Result<Option<Duration>> {
        Ok(None)
    }

    fn set_read_deadline(&mut self, _deadline: Option<Duration>) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The pipe hands out the peer's chunks one read at a time -- that is what
    /// reproduces a split frame -- and keeps the tail of a chunk the caller's
    /// buffer was too small for.
    #[test]
    fn a_scripted_pipe_hands_out_one_chunk_per_read() {
        let mut pipe = MemPipe::new([vec![1, 2, 3], vec![4, 5]]);
        let mut buf = [0u8; 8];

        assert_eq!(pipe.read(&mut buf).expect("first chunk"), 3);
        assert_eq!(&buf[..3], &[1, 2, 3]);
        assert_eq!(pipe.read(&mut buf).expect("second chunk"), 2);
        assert_eq!(&buf[..2], &[4, 5]);
        assert_eq!(pipe.handed_out(), 5);

        let mut small = [0u8; 2];
        let mut pipe = MemPipe::new([vec![1, 2, 3]]);
        assert_eq!(pipe.read(&mut small).expect("short buffer"), 2);
        assert_eq!(pipe.read(&mut small).expect("the kept tail"), 1);
        assert_eq!(pipe.reads(), 2);
    }

    /// The distinction the handshake's continuation bound rests on: a peer that
    /// closed is end of stream, a peer that merely stopped is not.
    #[test]
    fn an_exhausted_script_is_a_stall_unless_the_peer_closed() {
        let mut buf = [0u8; 4];

        let mut stalled = MemPipe::new([]);
        let err = stalled.read(&mut buf).expect_err("a stall is not a read");
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);

        let mut hung_up = MemPipe::new([]).closing();
        assert_eq!(hung_up.read(&mut buf).expect("end of stream"), 0);
    }

    #[test]
    fn the_pipe_keeps_everything_the_client_wrote() {
        let mut pipe = MemPipe::new([]);
        pipe.write_all(&[7, 8]).expect("write");
        pipe.write_all(&[9]).expect("write");
        assert_eq!(pipe.written(), &[7, 8, 9]);
    }

    /// The case a poll-shaped reader has to survive, scripted: an empty chunk
    /// between two full ones. The gap answers like a stall, and -- the part that
    /// makes it a gap rather than an end -- what the peer sends afterwards still
    /// arrives.
    #[test]
    fn a_gap_in_the_script_stalls_and_the_rest_still_arrives() {
        let mut pipe = MemPipe::new([vec![1, 2], vec![], vec![3]]);
        let mut buf = [0u8; 4];

        assert_eq!(pipe.read(&mut buf).expect("the first chunk"), 2);
        let err = pipe.read(&mut buf).expect_err("a gap is not a read");
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
        assert_eq!(pipe.read(&mut buf).expect("the chunk behind the gap"), 1);
        assert_eq!(&buf[..1], &[3]);

        // Two reads carried bytes, one carried nothing. The empty count is the
        // only witness that the gap was reached at all: `reads` cannot show it,
        // and a pipe whose gap was silently skipped passes every assertion above
        // except this one.
        assert_eq!(pipe.reads(), 2);
        assert_eq!(pipe.empty_reads(), 1);
        assert_eq!(pipe.handed_out(), 3);
    }

    /// Red control for the gap, kept as a test rather than run once: with the
    /// eof mode on, the same script reports the gap as end of stream. That is
    /// what an empty chunk did before it meant anything -- `Ok(0)` in the middle
    /// of a peer that is still sending.
    #[test]
    fn a_gap_reported_as_eof_ends_the_stream_mid_script() {
        let mut pipe = MemPipe::new([vec![1, 2], vec![], vec![3]]).reporting_eof_when_empty();
        let mut buf = [0u8; 4];

        assert_eq!(pipe.read(&mut buf).expect("the first chunk"), 2);
        assert_eq!(
            pipe.read(&mut buf).expect("the gap, reported as eof"),
            0,
            "this is the wrong answer the WouldBlock contract avoids"
        );
        assert_eq!(pipe.empty_reads(), 1);
    }
}
