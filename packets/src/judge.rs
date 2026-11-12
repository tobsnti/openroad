//! How much of a received body our decoder accounts for — the one judgement
//! the client, the offline census and (later) a server share.
//!
//! Idea: a decoder that reads a prefix of a body and stops is what an
//! unfinished or wrong layout looks like, and it does not fail. Re-encoding
//! what was decoded and comparing it with the bytes that arrived turns that
//! silence into a verdict. The client reads leniently and only reports a
//! short read; the census counts every verdict over the captures.

use bytes::Bytes;

use crate::{Packet, PacketError};

/// Server→client acks whose `From<_> for Bytes` is `unreachable!`: the
/// registry binds both directions per opcode, and building these is nothing a
/// client does (`agent::alchemy`). Judged without re-encoding.
const NO_ENCODER: &[u16] = &[0xB150, 0xB151];

/// What our decoder made of one body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Decoded, and re-encoding reproduces the received bytes.
    Exact,
    /// Decoded, and re-encoding is a strict prefix: the tail was never read.
    Short(usize),
    /// Decoded, and re-encoding is **shorter** than the body without being a
    /// prefix of it: the decoder read a tail it does not keep, and the encoder
    /// writes the flag that says the tail is absent. `MovementResponse`
    /// (`0xB021`) does exactly that by design.
    Lossy,
    /// Decoded, and re-encoding has the same length or is longer, but differs:
    /// the bytes themselves disagree. This is the shape that needs a look.
    Differs,
    /// The decoder refused this body.
    Error,
    /// Decoded, but the type cannot be encoded, so the round trip says nothing.
    NoEncoder,
    /// No type is bound to this opcode.
    Unwired,
}

/// Decode `body` and judge it.
pub fn judge(opcode: u16, body: &[u8]) -> Verdict {
    match Packet::deserialize(opcode, Bytes::copy_from_slice(body)) {
        Ok(packet) => judge_decoded(opcode, packet, body),
        Err(PacketError::UnknownOpcode(_)) => Verdict::Unwired,
        Err(_) => Verdict::Error,
    }
}

/// Decode `body` only if our form accounts for every byte of it — the strict
/// reading a server owes its clients (decision 0008): a prefix read of a
/// foreign request is refused rather than acted on. `Err` names why.
pub fn decode_exact(opcode: u16, body: &[u8]) -> Result<Packet, Verdict> {
    let packet = match Packet::deserialize(opcode, Bytes::copy_from_slice(body)) {
        Ok(packet) => packet,
        Err(PacketError::UnknownOpcode(_)) => return Err(Verdict::Unwired),
        Err(_) => return Err(Verdict::Error),
    };
    match judge_decoded(opcode, packet.clone(), body) {
        Verdict::Exact => Ok(packet),
        other => Err(other),
    }
}

/// Judge a body that already decoded into `packet`.
pub fn judge_decoded(opcode: u16, packet: Packet, body: &[u8]) -> Verdict {
    if NO_ENCODER.contains(&opcode) {
        return Verdict::NoEncoder;
    }
    let (_, encoded) = packet.into_serialize();
    if encoded.as_ref() == body {
        Verdict::Exact
    } else if encoded.len() < body.len() {
        if body.starts_with(encoded.as_ref()) {
            Verdict::Short(body.len() - encoded.len())
        } else {
            Verdict::Lossy
        }
    } else {
        Verdict::Differs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body with bytes behind the decoded form is short by exactly those
    /// bytes; the same body without them is exact. `0x3086` (the partner's
    /// confirm) is an empty body.
    #[test]
    fn bytes_behind_the_decoded_form_are_counted() {
        assert_eq!(judge(0x3086, &[]), Verdict::Exact);
        assert_eq!(judge(0x3086, &[7, 7]), Verdict::Short(2));
        assert_eq!(judge(0xFFFF, &[]), Verdict::Unwired);
    }

    /// The strict reading takes the exact body and refuses the one with bytes
    /// our form does not read.
    #[test]
    fn decode_exact_refuses_unread_bytes() {
        assert!(decode_exact(0x3086, &[]).is_ok());
        assert_eq!(decode_exact(0x3086, &[7]).err(), Some(Verdict::Short(1)));
        assert_eq!(decode_exact(0xFFFF, &[]).err(), Some(Verdict::Unwired));
    }
}
