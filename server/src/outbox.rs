//! The bounded queue of frames a session has yet to send.
//!
//! Bounded because a peer that stops reading must not make the server hold its
//! backlog forever: a session that falls [`CAPACITY`] frames behind is cut off.
//! The bound is sized from the largest burst measured from a vSRO server,
//! 1,032 despawns within 13 ms [V capture, peer vSRO], with fourfold headroom.

use std::collections::VecDeque;

use bytes::Bytes;

/// Frames a session may have queued before it is cut off.
pub const CAPACITY: usize = 4096;

/// One frame on its way out: opcode and body, before framing and encryption.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub opcode: u16,
    pub body: Bytes,
}

/// The session fell [`CAPACITY`] frames behind.
#[derive(Debug, PartialEq, Eq)]
pub struct Overflow;

#[derive(Debug, Default)]
pub struct Outbox {
    frames: VecDeque<Frame>,
}

impl Outbox {
    pub fn push(&mut self, frame: Frame) -> Result<(), Overflow> {
        if self.frames.len() >= CAPACITY {
            return Err(Overflow);
        }
        self.frames.push_back(frame);
        Ok(())
    }

    /// Everything queued, oldest first.
    pub fn take(&mut self) -> Vec<Frame> {
        self.frames.drain(..).collect()
    }
}
