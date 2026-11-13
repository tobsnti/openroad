//! Non-blocking stream plumbing both roles share: take what the socket holds,
//! cut logical packets off the front of the buffer (reassembling massive
//! packets), and write out what the socket takes. It works on any `Read` and
//! `Write` and on byte buffers; the sockets belong to each role.

use std::io::{ErrorKind, Read, Write};
use std::sync::{Arc, RwLock};

use bytes::{Bytes, BytesMut};
use tracing::{debug, trace};

use crate::frame::SilkroadFrame;
use crate::security::SilkroadSecurityState;

/// One step of draining the inbound byte accumulator. The receive loop pulls
/// logical packets off the front of the buffer with [`next_packet`]; a frame
/// whose bytes have not all arrived yet reports [`FrameStep::Incomplete`] and
/// stays buffered for the next read, rather than being dropped (which used to
/// corrupt every packet after the split).
pub enum FrameStep {
    /// A complete `Packet`, or a massive packet reassembled from its payloads.
    Ready {
        opcode: u16,
        data: Bytes,
        consumed: usize,
        /// The frame's wire `0x8000` bit, read from the plaintext size prefix
        /// before decryption. Carried out so `packet_dump` can record it
        /// (#459); for a massive packet it is the *header* frame's bit.
        encrypted: bool,
    },
    /// A frame parsed but carrying nothing to dispatch (a stray massive payload
    /// or a future frame kind) — advance past it.
    Skip { consumed: usize },
    /// Not enough bytes buffered yet for the next frame; wait for more.
    Incomplete,
    /// The length prefix is impossible or a frame failed to parse: the stream is
    /// desynchronised and the buffered bytes can no longer be trusted.
    Corrupt,
}

/// Pull the next logical packet off the front of `buf`. `buf` is decrypted in
/// place by [`SilkroadFrame::parse`]; the returned `data` is copied out, so the
/// caller can drop the consumed prefix afterwards.
pub fn next_packet(buf: &mut [u8], security: &Arc<RwLock<SilkroadSecurityState>>) -> FrameStep {
    let Some(frame_len) = SilkroadFrame::wire_len(buf) else {
        return FrameStep::Incomplete; // fewer than 2 bytes: length prefix not here yet
    };
    // Read before `parse`, which decrypts `buf` in place and drops the flag for
    // every frame kind but `Packet`.
    let encrypted = SilkroadFrame::wire_is_encrypted(buf).unwrap_or(false);
    // Smallest real frame is size(2) + opcode(2) + count(1) + crc(1); a shorter
    // length field means the stream is out of sync.
    if frame_len < 6 {
        return FrameStep::Corrupt;
    }
    if frame_len > buf.len() {
        return FrameStep::Incomplete; // partial tail — keep it for the next read
    }

    let frame = match SilkroadFrame::parse(&mut buf[..frame_len], security.clone()) {
        Ok((_, frame)) => frame,
        Err(e) => {
            debug!("network: failed to parse frame: {}", e);
            return FrameStep::Corrupt;
        }
    };
    // Full inbound-frame dump for the headless net-check client (filtered out at
    // info level, so free in the GUI build).
    trace!(target: "packets_in", "recv {}", frame);

    match frame {
        SilkroadFrame::Packet { opcode, data, .. } => FrameStep::Ready {
            opcode,
            data,
            consumed: frame_len,
            encrypted,
        },
        SilkroadFrame::MassiveHeader { opcode, amount, .. } => {
            // Reassemble the logical packet from the `amount` MassivePayload
            // frames that trail the header. If they have not all arrived, report
            // Incomplete so the header stays buffered and is retried next read.
            let mut body = BytesMut::new();
            let mut offset = frame_len;
            for _ in 0..amount {
                let Some(payload_len) = SilkroadFrame::wire_len(&buf[offset..]) else {
                    return FrameStep::Incomplete;
                };
                if offset + payload_len > buf.len() {
                    return FrameStep::Incomplete;
                }
                match SilkroadFrame::parse(&mut buf[offset..offset + payload_len], security.clone())
                {
                    Ok((_, payload @ SilkroadFrame::MassivePayload { .. })) => {
                        trace!(target: "packets_in", "recv {}", payload);
                        if let SilkroadFrame::MassivePayload { inner, .. } = payload {
                            body.extend_from_slice(&inner);
                        }
                        offset += payload_len;
                    }
                    _ => return FrameStep::Corrupt,
                }
            }
            FrameStep::Ready {
                opcode,
                data: body.freeze(),
                consumed: offset,
                encrypted,
            }
        }
        // A stray payload without a header, or any future frame kind.
        _ => FrameStep::Skip {
            consumed: frame_len,
        },
    }
}

/// How a pass over the socket ended.
pub enum ReadEnd {
    /// Nothing more to read right now (`WouldBlock`).
    Drained,
    /// The server closed the connection (`Ok(0)`).
    Closed,
}

/// Append everything the non-blocking socket holds to `buf`.
///
/// One 4 KiB read per frame left a burst queued in the kernel for many frames
/// — the post-join stream, or a thousand small frames a server packs into a
/// few milliseconds — and a read of zero bytes, the peer closing, went
/// unnoticed while the connection looked alive.
pub fn read_available(stream: &mut impl Read, buf: &mut Vec<u8>) -> std::io::Result<ReadEnd> {
    let mut scratch = [0u8; 4096];
    loop {
        match stream.read(&mut scratch) {
            Ok(0) => return Ok(ReadEnd::Closed),
            Ok(n) => buf.extend_from_slice(&scratch[..n]),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(ReadEnd::Drained),
            Err(e) => return Err(e),
        }
    }
}

/// Outbound bytes a connection may hold before it is closed. Far above any
/// burst of ours — the largest frame we send is ~1 KB — so only a server that
/// stopped reading gets there, and then the disconnect says so instead of the
/// buffer growing without bound.
const WRITE_BUF_LIMIT: usize = 1 << 20;

/// Write as much of `pending` as the socket takes now and drop what it took.
///
/// A non-blocking write may take part of a frame, or nothing (`WouldBlock`).
/// The rest stays queued for the next frame: dropping it, as a bare `write`
/// did, cuts a frame short, and the server — which splits the stream by length
/// prefix — reads every later frame from the wrong offset. Retrying the frame
/// instead is no option either: serializing consumes the security sequence
/// byte, so each frame is serialized once and its bytes are what waits here.
pub fn flush_pending(stream: &mut impl Write, pending: &mut Vec<u8>) -> std::io::Result<()> {
    while !pending.is_empty() {
        match stream.write(pending) {
            Ok(0) => return Err(ErrorKind::WriteZero.into()),
            Ok(n) => {
                pending.drain(..n);
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(e) => return Err(e),
        }
    }
    if pending.len() > WRITE_BUF_LIMIT {
        return Err(std::io::Error::other(format!(
            "{} bytes the server has not taken — it stopped reading",
            pending.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blowfish::Blowfish;
    use crate::security::SilkroadSecurity;

    fn security() -> Arc<RwLock<SilkroadSecurityState>> {
        // `new()` leaves the state uninitialized, so `parse` takes the
        // unencrypted path and never needs a blowfish instance.
        Arc::new(RwLock::new(SilkroadSecurityState::new()))
    }

    /// Build an unencrypted `Packet` frame: size | opcode | count | crc | data.
    fn packet_frame(opcode: u16, data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(data.len() as u16).to_le_bytes());
        v.extend_from_slice(&opcode.to_le_bytes());
        v.push(0); // count
        v.push(0); // crc
        v.extend_from_slice(data);
        v
    }

    fn massive_header_frame(inner_opcode: u16, amount: u16) -> Vec<u8> {
        let mut content = vec![1u8]; // header flag
        content.extend_from_slice(&amount.to_le_bytes());
        content.extend_from_slice(&inner_opcode.to_le_bytes());
        packet_frame(0x600D, &content)
    }

    fn massive_payload_frame(inner: &[u8]) -> Vec<u8> {
        let mut content = vec![0u8]; // payload flag
        content.extend_from_slice(inner);
        packet_frame(0x600D, &content)
    }

    /// Mirror of the `receive_packets` drain loop over an in-memory buffer.
    fn drain(
        buf: &mut Vec<u8>,
        security: &Arc<RwLock<SilkroadSecurityState>>,
    ) -> Vec<(u16, Vec<u8>)> {
        let mut out = Vec::new();
        let mut consumed = 0;
        loop {
            match next_packet(&mut buf[consumed..], security) {
                FrameStep::Ready {
                    opcode,
                    data,
                    consumed: n,
                    ..
                } => {
                    out.push((opcode, data.to_vec()));
                    consumed += n;
                }
                FrameStep::Skip { consumed: n } => consumed += n,
                FrameStep::Incomplete => break,
                FrameStep::Corrupt => {
                    consumed = buf.len();
                    break;
                }
            }
        }
        buf.drain(..consumed);
        out
    }

    /// #459: `packet_dump` recorded `<ts> <hex>` and nothing else, so a padded
    /// encrypted body and a plaintext one of the same length were
    /// indistinguishable when re-reading an old log. `next_packet` now carries
    /// the wire `0x8000` bit out to the dumper. Payload fixture is the first
    /// captured line of `packet_dump/0x3020.log`.
    #[test]
    fn next_packet_reports_the_wire_encryption_bit() {
        let payload = [0xb5u8, 0xa8, 0x01, 0x00, 0xd7, 0x01, 0x07, 0x11];

        let plain_security = security();
        let mut plain = packet_frame(0x3020, &payload);
        match next_packet(&mut plain, &plain_security) {
            FrameStep::Ready {
                opcode, encrypted, ..
            } => {
                assert_eq!(opcode, 0x3020);
                assert!(!encrypted, "plaintext frame reported as encrypted");
            }
            _ => panic!("expected a ready frame"),
        }

        // Same payload, but on the wire with the 0x8000 bit: round-trip it
        // through `serialize` so the bytes are a real encrypted frame.
        let established = {
            let mut state = SilkroadSecurityState::new();
            state.state = SilkroadSecurity::Established;
            state.context.blowfish = Some(Blowfish::new(&[0u8; 8]).expect("test key"));
            Arc::new(RwLock::new(state))
        };
        let mut wire = SilkroadFrame::Packet {
            count: 0,
            crc: 0,
            opcode: 0x3020,
            encrypted: 1,
            data: Bytes::copy_from_slice(&payload),
        }
        .serialize(established.clone())
        .expect("serialize")
        .to_vec();
        match next_packet(&mut wire, &established) {
            FrameStep::Ready {
                opcode,
                data,
                encrypted,
                ..
            } => {
                assert_eq!(opcode, 0x3020);
                assert!(encrypted, "encrypted frame reported as plaintext");
                // The body is block-padded, which is exactly why the flag has
                // to be recorded: its length alone does not identify it.
                assert_eq!(&data[..payload.len()], &payload);
            }
            _ => panic!("expected a ready frame"),
        }
    }

    /// A non-blocking socket whose send buffer holds `room` more bytes: it
    /// takes up to that much, then reports `WouldBlock`.
    struct FullSocket {
        taken: Vec<u8>,
        room: usize,
    }

    impl Write for FullSocket {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.room == 0 {
                return Err(ErrorKind::WouldBlock.into());
            }
            let n = buf.len().min(self.room);
            self.taken.extend_from_slice(&buf[..n]);
            self.room -= n;
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A write the socket takes only in part used to lose the rest, and the
    /// server then read every later frame from the wrong offset. What it does
    /// not take now waits for the next frame, in order.
    #[test]
    fn bytes_the_socket_did_not_take_wait_for_the_next_frame() {
        let frames = [
            packet_frame(0x7021, &[1, 2, 3]),
            packet_frame(0x7045, &[4, 5, 6, 7]),
        ]
        .concat();
        let mut pending = frames.clone();
        let mut socket = FullSocket {
            taken: Vec::new(),
            room: 5,
        };

        flush_pending(&mut socket, &mut pending).expect("WouldBlock is not an error");
        assert_eq!(socket.taken.len(), 5);
        assert_eq!(pending.len(), frames.len() - 5);

        socket.room = usize::MAX;
        flush_pending(&mut socket, &mut pending).expect("flush");
        assert!(pending.is_empty());
        assert_eq!(socket.taken, frames);
    }

    /// A server that stops reading must end the connection visibly rather
    /// than let the queue grow without bound.
    #[test]
    fn a_server_that_stops_reading_is_disconnected() {
        let mut socket = FullSocket {
            taken: Vec::new(),
            room: 0,
        };
        let mut pending = vec![0u8; WRITE_BUF_LIMIT];
        assert!(flush_pending(&mut socket, &mut pending).is_ok());
        pending.push(0);
        assert!(flush_pending(&mut socket, &mut pending).is_err());
    }

    /// A non-blocking socket holding `data`: it hands it out in reads of at
    /// most `chunk` bytes, then reports `WouldBlock` — or the end of the
    /// stream once the peer has `closed` it.
    struct HeldSocket {
        data: Vec<u8>,
        at: usize,
        chunk: usize,
        closed: bool,
    }

    impl Read for HeldSocket {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.at == self.data.len() {
                return if self.closed {
                    Ok(0)
                } else {
                    Err(ErrorKind::WouldBlock.into())
                };
            }
            let n = out.len().min(self.chunk).min(self.data.len() - self.at);
            out[..n].copy_from_slice(&self.data[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
    }

    /// A thousand small frames a server packs into a few milliseconds are read
    /// in one pass, not one 4 KiB read per frame.
    #[test]
    fn a_burst_is_read_in_one_pass() {
        let burst: Vec<u8> = (0..1032u16)
            .flat_map(|i| packet_frame(0x3016, &i.to_le_bytes()))
            .collect();
        let mut socket = HeldSocket {
            data: burst.clone(),
            at: 0,
            chunk: 4096,
            closed: false,
        };
        let mut buf = Vec::new();
        assert!(matches!(
            read_available(&mut socket, &mut buf),
            Ok(ReadEnd::Drained)
        ));
        assert_eq!(buf, burst);
        assert_eq!(drain(&mut buf, &security()).len(), 1032);
    }

    /// A read of zero bytes is the server closing; what it sent before is
    /// still there to dispatch.
    #[test]
    fn frames_sent_before_a_close_are_kept() {
        let frames = [packet_frame(0x3013, &[1, 2]), packet_frame(0xB070, &[3])].concat();
        let mut socket = HeldSocket {
            data: frames,
            at: 0,
            chunk: 4096,
            closed: true,
        };
        let mut buf = Vec::new();
        assert!(matches!(
            read_available(&mut socket, &mut buf),
            Ok(ReadEnd::Closed)
        ));
        assert_eq!(
            drain(&mut buf, &security()),
            vec![(0x3013, vec![1, 2]), (0xB070, vec![3])]
        );
    }

    #[test]
    fn drains_multiple_frames_in_one_pass() {
        let sec = security();
        let mut buf = packet_frame(0x3013, &[1, 2, 3]);
        buf.extend(packet_frame(0xB001, &[9]));

        let out = drain(&mut buf, &sec);

        assert_eq!(out, vec![(0x3013, vec![1, 2, 3]), (0xB001, vec![9])]);
        assert!(buf.is_empty(), "fully drained buffer must be empty");
    }

    #[test]
    fn frame_split_across_reads_is_not_dropped() {
        let sec = security();
        let mut full = packet_frame(0x3013, &[1, 2, 3, 4, 5]);
        full.extend(packet_frame(0xB070, &[7, 7]));

        // First read ends inside the first frame's body.
        let split = 4;
        let mut buf: Vec<u8> = full[..split].to_vec();
        let first = drain(&mut buf, &sec);
        assert!(first.is_empty(), "no whole frame yet");
        assert_eq!(buf.len(), split, "partial tail must be retained");

        // Rest of the stream arrives.
        buf.extend_from_slice(&full[split..]);
        let rest = drain(&mut buf, &sec);

        assert_eq!(
            rest,
            vec![(0x3013, vec![1, 2, 3, 4, 5]), (0xB070, vec![7, 7])]
        );
        assert!(buf.is_empty());
    }

    #[test]
    fn split_between_frames_keeps_the_second() {
        let sec = security();
        let first = packet_frame(0x3013, &[1, 2, 3]);
        let second = packet_frame(0xB070, &[8, 9]);
        let mut full = first.clone();
        full.extend(second.clone());

        // First read: whole first frame + half of the second.
        let split = first.len() + 3;
        let mut buf: Vec<u8> = full[..split].to_vec();
        let out = drain(&mut buf, &sec);
        assert_eq!(out, vec![(0x3013, vec![1, 2, 3])]);
        assert_eq!(
            buf.len(),
            split - first.len(),
            "only the second frame's head remains"
        );

        buf.extend_from_slice(&full[split..]);
        let out = drain(&mut buf, &sec);
        assert_eq!(out, vec![(0xB070, vec![8, 9])]);
        assert!(buf.is_empty());
    }

    #[test]
    fn massive_packet_reassembled_across_reads() {
        let sec = security();
        let mut full = massive_header_frame(0x34A5, 2);
        full.extend(massive_payload_frame(&[1, 2, 3]));
        full.extend(massive_payload_frame(&[4, 5]));

        // First read: header + only the first of two payloads.
        let after_first_payload =
            massive_header_frame(0x34A5, 2).len() + massive_payload_frame(&[1, 2, 3]).len();
        let mut buf: Vec<u8> = full[..after_first_payload].to_vec();
        let out = drain(&mut buf, &sec);
        assert!(
            out.is_empty(),
            "massive packet incomplete — nothing dispatched yet"
        );
        assert_eq!(
            buf.len(),
            after_first_payload,
            "header + partial payloads retained"
        );

        // Second payload arrives; the logical packet reassembles.
        buf.extend_from_slice(&full[after_first_payload..]);
        let out = drain(&mut buf, &sec);
        assert_eq!(out, vec![(0x34A5, vec![1, 2, 3, 4, 5])]);
        assert!(buf.is_empty());
    }

    /// #130: `next_packet` catches parse errors so a desynchronised stream can
    /// resynchronise, but the 0x600D arm read its flag and header fields off
    /// the body with no length check — and `Buf::get_*` panics on underflow.
    /// Random bytes that happen to read as 0x600D with a short body therefore
    /// aborted the process instead of dropping the buffer. All three shapes
    /// clear the only structural pre-filter (`frame_len >= 6`).
    #[test]
    fn truncated_massive_frames_are_corrupt_not_a_panic() {
        for (name, body) in [
            ("empty body - no flag byte", vec![]),
            ("header flag, no amount/opcode", vec![1u8]),
            ("header flag, amount only", vec![1u8, 2, 0]),
        ] {
            let sec = security();
            let mut buf = packet_frame(0x600D, &body);
            let out = drain(&mut buf, &sec);
            assert!(out.is_empty(), "{name}: nothing may be dispatched");
            assert!(buf.is_empty(), "{name}: corrupt frame drops the buffer");
        }
    }

    /// The guard must not reject a legitimately empty massive chunk: a payload
    /// flag with no inner bytes carries the flag byte and nothing else.
    #[test]
    fn massive_payload_with_empty_inner_still_parses() {
        let sec = security();
        let mut buf = massive_header_frame(0x34A5, 1);
        buf.extend(massive_payload_frame(&[]));
        let out = drain(&mut buf, &sec);
        assert_eq!(out, vec![(0x34A5, vec![])]);
        assert!(buf.is_empty());
    }
}
