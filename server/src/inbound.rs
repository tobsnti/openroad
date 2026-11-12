//! Strict reading of client frames (ADR 0013 §5).
//!
//! A frame is accepted only when it decodes into a packet that encodes back to
//! the very same bytes. A shorter, longer or otherwise different body is
//! rejected with its reason, never read as a prefix: the lenient reading let
//! the stub peer act on requests it had turned down. The round trip moves into
//! `packets` (`judge::decode_exact`) once that lands; the rule stays here.

use bytes::Bytes;
use packets::{Packet, PacketError};

/// Why a client frame was not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// No packet type is registered for the opcode.
    UnknownOpcode,
    /// The body does not decode.
    Undecodable,
    /// The body decodes, but not to what it says: it re-encodes differently.
    NotExact,
}

/// The packet `body` carries, if it carries exactly one.
pub fn decode_exact(opcode: u16, body: &Bytes) -> Result<Packet, Rejected> {
    let decode = || {
        Packet::deserialize(opcode, body.clone()).map_err(|e| match e {
            PacketError::UnknownOpcode(_) => Rejected::UnknownOpcode,
            PacketError::SerializationError(_) => Rejected::Undecodable,
        })
    };
    let (_, written) = decode()?.into_serialize();
    if written != *body {
        return Err(Rejected::NotExact);
    }
    decode()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_body_is_accepted() {
        // 0x7046: uid u32, talk flag u8.
        let talk = Bytes::from_static(&[0x02, 0x10, 0x00, 0x00, 0x01]);
        assert!(matches!(
            decode_exact(0x7046, &talk),
            Ok(Packet::TalkRequest(_))
        ));
        let long = Bytes::from_static(&[0x02, 0x10, 0x00, 0x00, 0x01, 0xFF]);
        assert_eq!(decode_exact(0x7046, &long).err(), Some(Rejected::NotExact));
        let short = Bytes::from_static(&[0x02, 0x10, 0x00, 0x00]);
        assert_eq!(
            decode_exact(0x7046, &short).err(),
            Some(Rejected::Undecodable)
        );
        assert_eq!(
            decode_exact(0x0FFF, &talk).err(),
            Some(Rejected::UnknownOpcode)
        );
    }
}
