//! A bounded, in-memory record of the frames that crossed the wire, readable
//! over BRP while the game runs.
//!
//! `packet_dump` already writes every payload to `packet_dump/<opcode>.log`,
//! but answering "what did the server reply to that" from it means stopping,
//! finding the files, and reading them in two directions at once — the on-disk
//! dump is built for offline analysis, not for a question asked mid-session.
//! This keeps the last N frames of both directions in order, with a timestamp,
//! so `openroad/packet_tail` can hand back a filtered window.
//!
//! Two deliberate properties:
//! - **Bounded.** The buffer has a fixed capacity and counts what it dropped,
//!   so a long session cannot grow it and a caller is told when the answer is
//!   incomplete rather than being shown a gap.
//! - **Credential frames keep no body.** The two login opcodes carry the
//!   account password in clear text; the trace line already summarises them
//!   instead of dumping them, and a buffer that any loopback caller can read
//!   must not undo that.

use std::collections::VecDeque;

use bevy::prelude::{In, Res, Resource};
use bevy::remote::error_codes::{INTERNAL_ERROR, INVALID_PARAMS};
use bevy::remote::{BrpError, BrpResult};
use bytes::Bytes;
use serde_json::{json, Value};

use crate::plugins::net::plugin::carries_credentials;

/// Frames kept when nothing asks for more; a few seconds of a busy stream.
pub const DEFAULT_CAPACITY: usize = 256;

/// Which way a frame went.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// Server to client.
    In,
    /// Client to server.
    Out,
}

impl Direction {
    /// The wire name used by the BRP payload and the `--dir` argument.
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::In => "in",
            Direction::Out => "out",
        }
    }

    /// Parses the same two names, so request and response speak one spelling.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "in" | "s2c" => Some(Direction::In),
            "out" | "c2s" => Some(Direction::Out),
            _ => None,
        }
    }
}

/// One recorded frame.
#[derive(Clone, Debug)]
pub struct TappedFrame {
    /// Seconds since app start, from `Time::elapsed_secs_f64`.
    pub at_secs: f64,
    pub direction: Direction,
    pub opcode: u16,
    /// `None` for a frame whose body is withheld (see the module docs).
    pub body: Option<Bytes>,
    /// The frame's wire `0x8000` bit; always false outbound, where the body is
    /// recorded before encryption is applied.
    pub encrypted: bool,
}

impl TappedFrame {
    /// Body length, which stays known even when the body itself is withheld.
    pub fn len(&self) -> usize {
        self.body.as_ref().map_or(0, Bytes::len)
    }
}

/// What a caller wants out of the buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TapFilter {
    pub opcode: Option<u16>,
    pub direction: Option<Direction>,
    /// Newest-first cut-off; 0 means "everything that matches".
    pub limit: usize,
}

#[derive(Resource)]
pub struct PacketTap {
    capacity: usize,
    frames: VecDeque<TappedFrame>,
    /// Frames pushed out by newer ones. The count is the difference between
    /// "nothing matched" and "it scrolled past", which are different answers.
    dropped: u64,
    recorded: u64,
}

impl Default for PacketTap {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
}

impl PacketTap {
    /// A capacity of 0 is raised to 1: a buffer that records nothing would
    /// report an empty window for a stream that is in fact flowing.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            frames: VecDeque::new(),
            dropped: 0,
            recorded: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn recorded(&self) -> u64 {
        self.recorded
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Nothing reads this yet; it exists because a public `len` without an
    /// `is_empty` is a clippy lint, and the pair is cheaper than an exception.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Record one frame, oldest dropped first once the buffer is full.
    pub fn record(
        &mut self,
        at_secs: f64,
        direction: Direction,
        opcode: u16,
        body: &[u8],
        encrypted: bool,
    ) {
        let body = if carries_credentials(opcode) {
            None
        } else {
            Some(Bytes::copy_from_slice(body))
        };
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
            self.dropped += 1;
        }
        self.frames.push_back(TappedFrame {
            at_secs,
            direction,
            opcode,
            body,
            encrypted,
        });
        self.recorded += 1;
    }

    /// The matching frames in the order they crossed the wire, cut to the
    /// newest `limit`. Oldest-first output keeps a request next to the reply
    /// it caused, which is the thing being read.
    pub fn tail(&self, filter: TapFilter) -> Vec<&TappedFrame> {
        let mut matching: Vec<&TappedFrame> = self
            .frames
            .iter()
            .filter(|frame| filter.opcode.is_none_or(|op| frame.opcode == op))
            .filter(|frame| filter.direction.is_none_or(|dir| frame.direction == dir))
            .collect();
        if filter.limit > 0 && matching.len() > filter.limit {
            matching.drain(..matching.len() - filter.limit);
        }
        matching
    }
}

/// Lower-case hex of a body, for the BRP payload and the log line.
pub fn hex_body(body: &[u8]) -> String {
    let mut out = String::with_capacity(body.len() * 2);
    for byte in body {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

// ---------------------------------------------------------------------------
// The BRP side. Parameter reading is a pure function so the shape of a request
// is covered by tests: a misread field would silently widen the window instead
// of failing.
// ---------------------------------------------------------------------------

/// Reads `{opcode, dir, limit}`. An opcode may be decimal or `0x`-prefixed
/// hex, because both spellings are in the notes this answers questions from.
pub fn tap_filter_from_params(params: Option<&Value>) -> Result<TapFilter, String> {
    let mut filter = TapFilter::default();
    let Some(params) = params else {
        return Ok(filter);
    };
    if let Some(raw) = params.get("opcode") {
        filter.opcode = Some(match raw {
            Value::String(text) => parse_opcode(text)?,
            Value::Number(number) => number
                .as_u64()
                .and_then(|value| u16::try_from(value).ok())
                .ok_or_else(|| format!("opcode out of range: {number}"))?,
            other => return Err(format!("opcode must be a string or a number, got {other}")),
        });
    }
    if let Some(raw) = params.get("dir") {
        let text = raw
            .as_str()
            .ok_or_else(|| format!("dir must be a string, got {raw}"))?;
        filter.direction =
            Some(Direction::parse(text).ok_or_else(|| format!("unknown dir {text:?}"))?);
    }
    if let Some(raw) = params.get("limit") {
        let limit = raw
            .as_u64()
            .ok_or_else(|| format!("limit must be a non-negative number, got {raw}"))?;
        filter.limit = usize::try_from(limit).unwrap_or(usize::MAX);
    }
    Ok(filter)
}

/// `0x3026`, `3026` as hex, or a plain decimal number — all three appear in
/// use, so all three are accepted and the hex form wins on ambiguity.
fn parse_opcode(text: &str) -> Result<u16, String> {
    let trimmed = text.trim();
    let stripped = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"));
    match stripped {
        Some(hex) => u16::from_str_radix(hex, 16).map_err(|e| format!("bad opcode {text:?}: {e}")),
        None => u16::from_str_radix(trimmed, 16)
            .or_else(|_| trimmed.parse::<u16>())
            .map_err(|e| format!("bad opcode {text:?}: {e}")),
    }
}

/// The response body: the window plus what the window does *not* contain.
pub fn tail_payload(tap: &PacketTap, filter: TapFilter) -> Value {
    let frames: Vec<Value> = tap
        .tail(filter)
        .into_iter()
        .map(|frame| {
            json!({
                "at_secs": frame.at_secs,
                "dir": frame.direction.as_str(),
                "opcode": format!("{:#06x}", frame.opcode),
                "len": frame.len(),
                "encrypted": frame.encrypted,
                // A withheld body is reported as such instead of as an empty
                // one, which would read like a frame with no payload.
                "body": match &frame.body {
                    Some(body) => Value::String(hex_body(body)),
                    None => Value::Null,
                },
            })
        })
        .collect();
    json!({
        "frames": frames,
        "buffered": tap.len(),
        "capacity": tap.capacity(),
        "recorded": tap.recorded(),
        "dropped": tap.dropped(),
    })
}

/// `openroad/packet_tail`: the last frames that crossed the wire.
pub fn brp_packet_tail(In(params): In<Option<Value>>, tap: Option<Res<PacketTap>>) -> BrpResult {
    let Some(tap) = tap else {
        return Err(BrpError {
            code: INTERNAL_ERROR,
            message: "no packet tap in this run: it is part of the diagnostics tier".to_string(),
            data: None,
        });
    };
    let filter = tap_filter_from_params(params.as_ref()).map_err(|message| BrpError {
        code: INVALID_PARAMS,
        message,
        data: None,
    })?;
    Ok(tail_payload(&tap, filter))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gateway login; `carries_credentials` names it and the agent login.
    const LOGIN_OPCODE: u16 = 0x6102;

    fn record_n(tap: &mut PacketTap, n: usize) {
        for i in 0..n {
            tap.record(
                i as f64,
                Direction::In,
                0x3000 + i as u16,
                &[i as u8],
                false,
            );
        }
    }

    #[test]
    fn the_buffer_keeps_the_newest_frames_and_counts_what_it_dropped() {
        let mut tap = PacketTap::with_capacity(3);
        record_n(&mut tap, 5);
        assert_eq!(tap.len(), 3);
        assert_eq!(tap.dropped(), 2);
        assert_eq!(tap.recorded(), 5);
        let opcodes: Vec<u16> = tap
            .tail(TapFilter::default())
            .iter()
            .map(|frame| frame.opcode)
            .collect();
        assert_eq!(opcodes, vec![0x3002, 0x3003, 0x3004]);
    }

    /// A capacity of 0 would report an empty window for a flowing stream.
    #[test]
    fn a_zero_capacity_still_records_one_frame() {
        let mut tap = PacketTap::with_capacity(0);
        record_n(&mut tap, 2);
        assert_eq!(tap.capacity(), 1);
        assert_eq!(tap.len(), 1);
    }

    #[test]
    fn a_filter_selects_by_opcode_and_by_direction() {
        let mut tap = PacketTap::default();
        tap.record(0.0, Direction::Out, 0x7021, &[1], false);
        tap.record(1.0, Direction::In, 0x3026, &[2], true);
        tap.record(2.0, Direction::In, 0x7021, &[3], false);

        let by_opcode = tap.tail(TapFilter {
            opcode: Some(0x7021),
            ..TapFilter::default()
        });
        assert_eq!(by_opcode.len(), 2);

        let by_both = tap.tail(TapFilter {
            opcode: Some(0x7021),
            direction: Some(Direction::Out),
            limit: 0,
        });
        assert_eq!(by_both.len(), 1);
        assert_eq!(by_both[0].at_secs, 0.0);

        let inbound = tap.tail(TapFilter {
            direction: Some(Direction::In),
            ..TapFilter::default()
        });
        assert!(inbound.iter().all(|frame| frame.direction == Direction::In));
        assert!(inbound[0].encrypted, "the 0x8000 bit is carried through");
    }

    /// The cut keeps the *newest* matches but hands them back oldest-first, so
    /// a reply still reads after its request.
    #[test]
    fn a_limit_keeps_the_newest_matches_in_wire_order() {
        let mut tap = PacketTap::default();
        record_n(&mut tap, 10);
        let window = tap.tail(TapFilter {
            limit: 3,
            ..TapFilter::default()
        });
        let opcodes: Vec<u16> = window.iter().map(|frame| frame.opcode).collect();
        assert_eq!(opcodes, vec![0x3007, 0x3008, 0x3009]);
    }

    /// The property the module exists to keep: a login frame is recorded —
    /// its opcode, time and direction are the interesting part — but its body
    /// is not, because the body is a password.
    #[test]
    fn a_credential_frame_is_recorded_without_its_body() {
        let mut tap = PacketTap::default();
        tap.record(0.0, Direction::Out, LOGIN_OPCODE, b"secret", false);
        let frames = tap.tail(TapFilter::default());
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].opcode, LOGIN_OPCODE);
        assert!(
            frames[0].body.is_none(),
            "a login body must not be readable from the buffer"
        );
        assert_eq!(frames[0].len(), 0);
    }

    /// Reading a request wrong would widen the window instead of failing, so
    /// both spellings and every refusal are pinned.
    #[test]
    fn a_request_may_name_its_opcode_in_hex_or_decimal() {
        let filter = tap_filter_from_params(Some(&json!({ "opcode": "0x3026" }))).expect("hex");
        assert_eq!(filter.opcode, Some(0x3026));
        // Bare digits are read as hex first: the notes write opcodes that way.
        let filter = tap_filter_from_params(Some(&json!({ "opcode": "3026" }))).expect("bare hex");
        assert_eq!(filter.opcode, Some(0x3026));
        let filter = tap_filter_from_params(Some(&json!({ "opcode": 12326 }))).expect("number");
        assert_eq!(filter.opcode, Some(0x3026));
        assert!(tap_filter_from_params(Some(&json!({ "opcode": "zzz" }))).is_err());
        assert!(tap_filter_from_params(Some(&json!({ "opcode": 70000 }))).is_err());
        assert!(tap_filter_from_params(Some(&json!({ "opcode": true }))).is_err());
    }

    #[test]
    fn a_request_without_params_asks_for_everything() {
        let filter = tap_filter_from_params(None).expect("no params is valid");
        assert_eq!(filter, TapFilter::default());
        assert_eq!(filter.limit, 0);
        assert!(filter.opcode.is_none() && filter.direction.is_none());
    }

    #[test]
    fn a_bad_direction_or_limit_is_refused_instead_of_ignored() {
        assert!(tap_filter_from_params(Some(&json!({ "dir": "sideways" }))).is_err());
        assert!(tap_filter_from_params(Some(&json!({ "dir": 1 }))).is_err());
        assert!(tap_filter_from_params(Some(&json!({ "limit": -2 }))).is_err());
        let filter =
            tap_filter_from_params(Some(&json!({ "dir": "out", "limit": 5 }))).expect("both valid");
        assert_eq!(filter.direction, Some(Direction::Out));
        assert_eq!(filter.limit, 5);
    }

    /// The payload reports what it does not contain: a withheld body is
    /// `null`, not an empty string, and the counters say whether anything
    /// scrolled past.
    #[test]
    fn the_payload_separates_an_empty_body_from_a_withheld_one() {
        let mut tap = PacketTap::with_capacity(2);
        tap.record(0.5, Direction::Out, LOGIN_OPCODE, b"secret", false);
        tap.record(1.5, Direction::In, 0x3026, &[], false);
        tap.record(2.5, Direction::In, 0x3014, &[0xab], true);

        let payload = tail_payload(&tap, TapFilter::default());
        assert_eq!(payload["buffered"], json!(2));
        assert_eq!(payload["capacity"], json!(2));
        assert_eq!(payload["recorded"], json!(3));
        assert_eq!(payload["dropped"], json!(1));

        let frames = payload["frames"].as_array().expect("an array of frames");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["opcode"], json!("0x3026"));
        assert_eq!(
            frames[0]["body"],
            json!(""),
            "an empty body is an empty hex string"
        );
        assert_eq!(frames[1]["body"], json!("ab"));
        assert_eq!(frames[1]["encrypted"], json!(true));
        assert_eq!(frames[1]["len"], json!(1));
        assert_eq!(frames[1]["dir"], json!("in"));
    }

    #[test]
    fn a_withheld_body_is_null_in_the_payload() {
        let mut tap = PacketTap::default();
        tap.record(0.0, Direction::Out, LOGIN_OPCODE, b"secret", false);
        let payload = tail_payload(&tap, TapFilter::default());
        assert_eq!(payload["frames"][0]["body"], Value::Null);
        assert!(
            !payload.to_string().contains("secret"),
            "the payload must not carry a login body in any form"
        );
    }

    #[test]
    fn a_body_is_rendered_as_lower_case_hex() {
        assert_eq!(hex_body(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
        assert_eq!(hex_body(&[]), "");
    }

    #[test]
    fn both_direction_spellings_round_trip() {
        assert_eq!(Direction::parse("in"), Some(Direction::In));
        assert_eq!(Direction::parse("S2C"), Some(Direction::In));
        assert_eq!(Direction::parse(" out "), Some(Direction::Out));
        assert_eq!(Direction::parse("c2s"), Some(Direction::Out));
        assert_eq!(Direction::parse("both"), None);
        assert_eq!(Direction::In.as_str(), "in");
        assert_eq!(Direction::Out.as_str(), "out");
    }
}
