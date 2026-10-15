//! A [`Pk2Source`] that fetches byte ranges over HTTP instead of reading a file.
//!
//! IDEA. `Pk2Source::read_exact_at(buf, offset)` *is* an HTTP `Range:` header —
//! "send me `buf.len()` bytes starting at `offset`" — so an archive on a web
//! server needs no new PK2 code at all, only this adapter. The index walk, the
//! cipher and the entry lookup stay exactly as they are; they just get their
//! bytes from a socket. That is what makes a 5 GiB archive usable from a client
//! that cannot hold it: nothing is downloaded until something is read.
//!
//! Two things are checked rather than assumed, because both fail silently:
//!
//! * **The server must honour the range.** A server without range support
//!   answers `200` with the *whole* body — for the user's `Data.pk2` that is
//!   3,3 GB pulled to satisfy a 256-byte header read. So a `200` is an error
//!   here, reported at open time by a one-byte probe and again on every read.
//! * **The range that comes back must be the range that was asked for.** A
//!   proxy or a buggy server answering a shifted range would feed the parser
//!   offset data; the cipher does not catch that inside a file body. The
//!   `Content-Range` is therefore compared against the request.
//!
//! NOT COMPILED FOR wasm32. `ureq` -> `rustls` -> `ring` has no
//! `wasm32-unknown-unknown` build, so `http-source` is inert there. A browser
//! path is not a port of this file: `fetch` is asynchronous, so it needs a
//! `Pk2Source` whose read is a future.

use std::io::{Error, ErrorKind, Read, Result};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::pk2::source::Pk2Source;

/// Enough for a slow link and a cold cache, short enough that a dead server
/// fails the client instead of hanging it.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// An archive served over HTTP, read one range at a time.
///
/// Cheap to share: `ureq`'s agent pools connections, and the counters are
/// atomics, so one source serves concurrent positioned reads without a lock —
/// the same property the file-backed source has.
#[derive(Debug)]
pub struct HttpSource {
    url: String,
    agent: ureq::Agent,
    size: u64,
    requests: AtomicU64,
    bytes: AtomicU64,
}

impl HttpSource {
    /// Probe `url` and keep it as an archive source.
    ///
    /// The probe is a one-byte ranged GET: it answers both questions at once —
    /// does the server honour `Range`, and how long is the archive — and costs
    /// one byte of body.
    pub fn open(url: impl Into<String>) -> Result<Self> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .build();
        Self::with_agent(url, agent)
    }

    /// As [`HttpSource::open`], with a caller-built agent — for a proxy, a
    /// custom TLS root store or different timeouts.
    pub fn with_agent(url: impl Into<String>, agent: ureq::Agent) -> Result<Self> {
        let url = url.into();
        let probe = Self {
            url,
            agent,
            size: u64::MAX, // not known yet; the probe below is what sets it
            requests: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        };
        let mut first = [0u8; 1];
        let (_, _, total) = probe.fetch_range(&mut first, 0)?;
        let size = total.ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidData,
                format!(
                    "{}: server answered a range but did not state the archive size \
                     (Content-Range total is `*`)",
                    probe.url
                ),
            )
        })?;
        Ok(Self { size, ..probe })
    }

    /// The archive's length, as the server stated it.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// How many HTTP requests this source has made, and how many body bytes it
    /// has taken. These exist to be reported: the point of reading a PK2 over
    /// the network is how *little* of it has to travel, and that claim is worth
    /// nothing unmeasured.
    pub fn requests(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    pub fn transferred_bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// One ranged GET into `buf`, returning the range the server said it sent.
    fn fetch_range(&self, buf: &mut [u8], offset: u64) -> Result<(u64, u64, Option<u64>)> {
        let want_last = offset
            .checked_add(buf.len() as u64 - 1)
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "read range overflows"))?;
        let response = self
            .agent
            .get(&self.url)
            .set("Range", &format!("bytes={offset}-{want_last}"))
            .call()
            .map_err(|e| Error::other(format!("{}: range request failed: {e}", self.url)))?;
        self.requests.fetch_add(1, Ordering::Relaxed);

        // The red case: no range support means the whole archive is on its way.
        // The body is deliberately not read — the response is dropped here.
        if response.status() != 206 {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!(
                    "{}: server answered {} instead of 206 Partial Content, so it ignores \
                     `Range` and would send the whole archive; serve the PK2 from a server \
                     with byte-range support",
                    self.url,
                    response.status()
                ),
            ));
        }
        let got = response
            .header("content-range")
            .and_then(parse_content_range)
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!("{}: 206 without a usable Content-Range header", self.url),
                )
            })?;
        if (got.0, got.1) != (offset, want_last) {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "{}: asked for bytes {offset}-{want_last}, server sent {}-{}",
                    self.url, got.0, got.1
                ),
            ));
        }
        response.into_reader().read_exact(buf)?;
        self.bytes.fetch_add(buf.len() as u64, Ordering::Relaxed);
        Ok(got)
    }
}

/// `bytes 0-255/5618049024` -> `(0, 255, Some(5618049024))`; a `*` total is
/// `None` (RFC 9110 allows it when the server does not know the length).
fn parse_content_range(header: &str) -> Option<(u64, u64, Option<u64>)> {
    let (range, total) = header.trim().strip_prefix("bytes ")?.split_once('/')?;
    let (start, last) = range.split_once('-')?;
    let total = match total.trim() {
        "*" => None,
        n => Some(n.parse().ok()?),
    };
    Some((start.trim().parse().ok()?, last.trim().parse().ok()?, total))
}

impl Pk2Source for HttpSource {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> Result<()> {
        if buf.is_empty() {
            return Ok(());
        }
        // Bounds-checked here rather than by a 416 from the server: a read past
        // the end is a malformed archive (or a hostile one), and it should cost
        // a message, not a round trip.
        let end = offset
            .checked_add(buf.len() as u64)
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "read length overflows"))?;
        if end > self.size {
            return Err(Error::new(
                ErrorKind::UnexpectedEof,
                "failed to fill whole buffer",
            ));
        }
        self.fetch_range(buf, offset).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pk2::archive::tests::{test_key, tiny_archive};
    use crate::pk2::blowfish::Blowfish;
    use crate::pk2::key::Pk2Key;
    use crate::prelude::Archive;
    use bytes::Bytes;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    use std::thread;

    /// The smallest HTTP server that can answer a byte range, so the tests
    /// exercise the real socket path — status line, headers, body — with no
    /// dependency and no archive on disk. `honour_ranges = false` is the red
    /// control: it answers `200` with the whole body, which is what a plain
    /// static server without range support does.
    fn serve(body: Vec<u8>, honour_ranges: bool) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let url = format!(
            "http://{}/archive.pk2",
            listener.local_addr().expect("local addr")
        );
        let hits = Arc::new(AtomicUsize::new(0));
        let served = Arc::clone(&hits);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut head = BufReader::new(stream.try_clone().expect("clone socket"));
                let mut asked: Option<(usize, usize)> = None;
                loop {
                    let mut line = String::new();
                    if head.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let line = line.trim_end().to_ascii_lowercase();
                    if line.is_empty() {
                        break;
                    }
                    if let Some(spec) = line.strip_prefix("range: bytes=") {
                        if let Some((a, b)) = spec.split_once('-') {
                            asked = Some((
                                a.trim().parse().unwrap_or(0),
                                b.trim().parse().unwrap_or(body.len() - 1),
                            ));
                        }
                    }
                }
                served.fetch_add(1, Ordering::SeqCst);
                let (head, slice) = match asked.filter(|_| honour_ranges) {
                    Some((start, last)) if start < body.len() => {
                        let last = last.min(body.len() - 1);
                        (
                            format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{last}/{}\r\n",
                                body.len()
                            ),
                            &body[start..=last],
                        )
                    }
                    _ => ("HTTP/1.1 200 OK\r\n".to_string(), &body[..]),
                };
                let _ = stream.write_all(
                    format!(
                        "{head}Content-Length: {}\r\nConnection: close\r\n\r\n",
                        slice.len()
                    )
                    .as_bytes(),
                );
                let _ = stream.write_all(slice);
            }
        });
        (url, hits)
    }

    /// Bytes with no structure of their own, so a wrong offset cannot pass by
    /// luck.
    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 + i / 7) as u8).collect()
    }

    /// Byte equality, offset by offset: the in-memory source and the HTTP
    /// source are handed the same reads and must answer identically.
    #[test]
    fn a_ranged_read_matches_the_same_bytes_in_memory() {
        let body = pattern(4096);
        let (url, hits) = serve(body.clone(), true);
        let memory = Bytes::from(body.clone());
        let http = HttpSource::open(&url).expect("server honours ranges");

        assert_eq!(http.size(), 4096, "size comes from Content-Range");
        for (offset, len) in [
            (0u64, 256usize),
            (255, 1),
            (1000, 1024),
            (4095, 1),
            (0, 4096),
        ] {
            let mut over_http = vec![0u8; len];
            let mut in_memory = vec![0u8; len];
            http.read_exact_at(&mut over_http, offset).expect("ranged");
            memory.read_exact_at(&mut in_memory, offset).expect("slice");
            assert_eq!(over_http, in_memory, "same bytes at {offset}+{len}");
        }
        assert_eq!(http.requests(), 6, "the probe plus one per read");
        assert_eq!(http.transferred_bytes(), 1 + 256 + 1 + 1024 + 1 + 4096);
        assert_eq!(hits.load(Ordering::SeqCst), 6);
    }

    /// Red control. A server that ignores `Range` must be named as such at open
    /// time — the failure mode it hides is pulling the whole archive.
    #[test]
    fn a_server_that_ignores_range_is_refused() {
        let (url, hits) = serve(pattern(4096), false);
        let err = HttpSource::open(&url).expect_err("200 is not usable");
        assert_eq!(err.kind(), ErrorKind::Unsupported);
        let message = err.to_string();
        assert!(message.contains("200"), "names the status: {message}");
        assert!(
            message.contains("whole archive"),
            "names the cost: {message}"
        );
        assert_eq!(hits.load(Ordering::SeqCst), 1, "it did not try again");
    }

    /// The point of the whole slice: an archive is indexed and read over the
    /// network through the same code that reads one on disk, and the bytes are
    /// the same ones.
    #[test]
    fn an_archive_is_indexed_and_read_over_http() {
        let key = test_key();
        let blowfish = Blowfish::from_key(&key).expect("fixture key is valid");
        let archive_bytes = tiny_archive(&blowfish, "a.txt", b"hello pk2");
        let (url, _) = serve(archive_bytes.to_vec(), true);

        let source = Arc::new(HttpSource::open(&url).expect("server honours ranges"));
        let archive = Archive::from_source(source.clone(), &key).expect("indexes over HTTP");

        assert_eq!(
            archive.read_file_bytes(Path::new("a.txt")).as_deref(),
            Some(&b"hello pk2"[..]),
            "same bytes as the in-memory archive"
        );
        // Measured, not predicted: five requests, because the index walk reads
        // the first block TWICE — once as the start of the block chain, once as
        // the body of the root `.` entry, which points at the same offset. On a
        // 2 825-byte fixture that is free; on the user's 3,3 GB archive it is
        // the argument for the block cache named as the next slice.
        assert_eq!(source.requests(), 5);
        assert_eq!(
            source.transferred_bytes(),
            1 + 256 + 2560 + 2560 + 9,
            "probe + header + two block reads + the file body"
        );
    }

    /// A read past the end is answered without a round trip.
    #[test]
    fn a_read_past_the_end_makes_no_request() {
        let (url, hits) = serve(pattern(512), true);
        let http = HttpSource::open(&url).expect("server honours ranges");
        let mut buf = [0u8; 16];
        let err = http.read_exact_at(&mut buf, 500).expect_err("past the end");
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
        assert_eq!(hits.load(Ordering::SeqCst), 1, "only the probe was sent");
    }

    #[test]
    fn a_content_range_header_is_parsed() {
        assert_eq!(
            parse_content_range("bytes 0-255/5618049024"),
            Some((0, 255, Some(5_618_049_024)))
        );
        assert_eq!(parse_content_range("bytes 7-7/*"), Some((7, 7, None)));
        assert_eq!(parse_content_range("items 0-1/2"), None, "not a byte range");
        assert_eq!(parse_content_range("bytes 0-255"), None, "no total");
    }

    /// Not a test but a measurement, kept here so it is reproducible: read one
    /// named file out of a real archive twice — once from disk, once over HTTP
    /// — and report the request count and the bytes that travelled.
    ///
    /// Serve the archive from a server that answers `206` to a `Range` request
    /// (nginx, caddy, `npx http-server`). A static server that answers `200`
    /// with the whole body is the red control, not a usable source — this test
    /// then fails at `HttpSource::open` with that message.
    ///
    /// ```text
    /// SRO_PK2_KEY=… SRO_PK2_SALT=… PK2_HTTP_URL=http://127.0.0.1:8099/Data.pk2 \
    ///   PK2_DISK_PATH=<dir>/Data.pk2 PK2_HTTP_ENTRY=<path inside the archive> \
    ///   cargo test -p bevy_pk2 --features http-source -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs a local server over the user's own archive"]
    fn measure_one_file_out_of_a_real_archive() {
        let url = std::env::var("PK2_HTTP_URL").expect("PK2_HTTP_URL");
        let entry = std::env::var("PK2_HTTP_ENTRY").expect("PK2_HTTP_ENTRY");
        let key = Pk2Key::resolve().expect("a key in the environment or config.yaml");

        let source = Arc::new(HttpSource::open(&url).expect("range-capable server"));
        let archive = Archive::from_source(source.clone(), &key).expect("indexes over HTTP");
        let (index_requests, index_bytes) = (source.requests(), source.transferred_bytes());
        let over_http = archive
            .read_file_bytes(Path::new(&entry))
            .expect("the entry is in the archive");
        println!(
            "archive {} bytes; index: {index_requests} requests / {index_bytes} bytes; \
             entry {entry}: {} bytes in {} requests / {} bytes transferred (total {:.4}% of the archive)",
            source.size(),
            over_http.len(),
            source.requests() - index_requests,
            source.transferred_bytes() - index_bytes,
            source.transferred_bytes() as f64 * 100.0 / source.size() as f64,
        );

        if let Ok(path) = std::env::var("PK2_DISK_PATH") {
            let from_disk = Archive::open(&path, &key)
                .expect("the same archive on disk")
                .read_file_bytes(Path::new(&entry))
                .expect("the same entry");
            println!(
                "disk {} bytes, fnv1a {:016x}",
                from_disk.len(),
                fnv1a(&from_disk)
            );
            println!(
                "http {} bytes, fnv1a {:016x}",
                over_http.len(),
                fnv1a(&over_http)
            );
            assert_eq!(
                from_disk, over_http,
                "disk and HTTP must agree byte for byte"
            );
        }
    }

    /// A hash to quote in a report; the assertion above compares the bytes
    /// themselves, which is stronger.
    fn fnv1a(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ *b as u64).wrapping_mul(0x100_0000_01b3)
        })
    }
}
