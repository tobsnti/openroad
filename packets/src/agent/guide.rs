//! Game-guide flag packets (`0x70EA`), the client's own "I have seen this
//! topic" marks.
//!
//! Named for what the client's handler does with the word, not for a
//! subsystem we guessed: the `0xB0EA` handler (`00881690`, registered at
//! `00870973`) takes the position of the highest set bit, adds 50000, and hands
//! that to `009d4c80`, which subtracts the 50000 again (`add eax, 0xffff3cb0`)
//! before indexing a **UI object**. The 50000 is a namespace marker, not a
//! value. A bit therefore selects one entry of a UI topic list — which is why
//! the older name `JobFlagRequest` is gone: nothing in this binary or in our
//! captures ever tied it to the job system.

use bevy::prelude::Message;

use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

/// 0x70EA — client → server, eight bytes written as one blob, carrying a
/// 64-bit flag word in which exactly one bit is ever set.
///
/// Idea of the shape: the sender at `0081eda0` forwards its caller's eight
/// bytes verbatim with a single `FUN_00508fe0(&stack0x00000004, 8)`, so the
/// *grouping* cannot be read off the write sizes the way it can elsewhere in
/// this family (`0x704F`'s sender writes `1`, which is how that one-byte
/// action was confirmed). Two things settle it as **one 64-bit word** rather
/// than two dwords: the `0xB0EA` handler reads it with a single 8-byte read at
/// `008816c3` and then shifts it with `__allshr` (`00b47710`), the 64-bit shift
/// helper — a pair of `u32`s would not be shifted as one value; and across 177
/// frames the upper half is zero every time while the lower half carries
/// **exactly one bit**.
///
/// ```text
///                              independent  frames
/// 01 00 00 00 00 00 00 00   2^0      13        13
/// 02 00 00 00 00 00 00 00   2^1      11        11
/// 10 00 00 00 00 00 00 00   2^4       2         2
/// 40 00 00 00 00 00 00 00   2^6      33        33
/// 00 01 00 00 00 00 00 00   2^8      22        28
/// 00 02 00 00 00 00 00 00   2^9      41        41
/// 00 00 01 00 00 00 00 00   2^16      3         5
/// 00 00 20 00 00 00 00 00   2^21     11        11
/// 00 00 00 01 00 00 00 00   2^24     33        33
/// ```
///
/// Counted as **independent observations** rather than frames: a duplicate is
/// the same `(timestamp, body)` in two directories, and bits 8 and 16 lose 6
/// and 2 that way. 169 independent observations out of 177 frames — counting
/// frames would overstate exactly the bits whose evidence is thinnest.
///
/// **The four low bits are new, and they corrected a claim this file used to
/// make.** Until 2026-10-29 it said byte 0 was zero in all 118 frames, so "no
/// bit below 8 has ever been observed", and treated that as a property of the
/// field. It was a property of the *sample*: every one of those sessions used
/// an established character. Runs with a freshly created character produce bits
/// 0, 1, 4 and 6 — 59 independent observations — because the low marks are the
/// first topics a new character is shown. An absence is only a finding when the
/// population can contain the thing.
///
/// **It wants an answer.** Measured 2026-10-29 with the same walk sequence in
/// three runs against our own stub, which models `0xB0EA` as a result byte plus
/// the echoed word:
///
/// ```text
///   no answer    0x70EA 17   0xB0EA 0    bits 0:1 1:2 6:14
///   no answer    0x70EA 14   0xB0EA 0    bits 0:1 1:6 4:2 6:5
///   answered     0x70EA  3   0xB0EA 3    bits 0:1 1:1 6:1
/// ```
///
/// Answered, every bit goes exactly **once**; unanswered, the client repeats it
/// — bit 6 fourteen times in one session. The repetition we had recorded as a
/// trait of the opcode was a retry against our own silence.
///
/// **Which bit means which topic is `[U]`.** The caller decides the mask, and
/// `0081eda0` has **no caller in this binary** at all: zero direct `E8` calls,
/// the address appears nowhere as a 4-byte constant (so no vtable), and the one
/// `E9` candidate lands mid-instruction — measured against a positive control
/// of 345 and 816 callers for two neighbours in the same body. That route is
/// closed, not merely incomplete. Three outside projects do name the bits, but
/// they are licence-classed read-only for us and no bit is named here on their
/// word alone.
///
/// The server in the older capture sessions is **skrillax**, and in the 2026-10
/// ones our own stub — neither is vanilla, so only the **c2s** half proves
/// anything about the original client. This opcode is c2s; the `0xB0EA` answer
/// shape is read off the client's own handler rather than off captured server
/// frames for exactly that reason.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GameGuideFlagRequest {
    /// One 64-bit word with exactly one bit set in every captured frame; nine
    /// bits observed and the upper half zero in all 177. Raw rather than a
    /// named enum because a name would have to name the bits.
    pub flags: u64,
}

impl GameGuideFlagRequest {
    /// Every bit our captures contain, counted as **independent original-client
    /// observations** rather than frames. Bits 0, 1, 4 and 6 only appear with a
    /// freshly created character; see the type's note.
    pub const OBSERVED_BITS: [u32; 9] = [0, 1, 4, 6, 8, 9, 16, 21, 24];

    /// The bit this request carries, if it is a single-bit word as every
    /// captured frame was. `None` for zero or for more than one bit — both
    /// unobserved, so neither is rejected, only reported.
    pub fn single_bit(&self) -> Option<u32> {
        (self.flags != 0 && self.flags.is_power_of_two()).then(|| self.flags.trailing_zeros())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use bytes::Bytes;

    /// Every captured single-bit form, decoded and re-encoded. Pinned as a
    /// decode *and* an encode, because the sender writes the eight bytes as one
    /// blob and a wrong field order would still be eight bytes.
    #[test]
    fn game_guide_flag_request_decodes_every_captured_form() {
        let captured: [(&[u8], u32); 9] = [
            (&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], 0),
            (&[0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], 1),
            (&[0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], 4),
            (&[0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], 6),
            (&[0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], 8),
            (&[0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], 9),
            (&[0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00], 16),
            (&[0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00], 21),
            (&[0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00], 24),
        ];

        for (wire, bit) in captured {
            let mut cursor = Cursor::new(wire);
            let decoded = GameGuideFlagRequest::read_from(&mut cursor).expect("decodes");
            assert_eq!(cursor.position() as usize, wire.len());
            assert_eq!(decoded.byte_size(), wire.len());
            assert_eq!(decoded.single_bit(), Some(bit), "body {wire:02x?}");
            assert_eq!(decoded.flags >> 32, 0, "upper half is zero in all captures");
            assert!(GameGuideFlagRequest::OBSERVED_BITS.contains(&bit));

            let bytes: Bytes = decoded.into();
            assert_eq!(bytes.as_ref(), wire, "re-encode must be byte-identical");
        }
    }

    /// The four low bits are the 2026-10-29 addition, and they are the ones a
    /// fresh character produces. Pinned separately so a future narrowing of the
    /// field to "byte 0 is always zero" fails here rather than silently.
    #[test]
    fn the_low_bits_a_fresh_character_sends_are_observed() {
        for bit in [0, 1, 4, 6] {
            assert!(
                GameGuideFlagRequest::OBSERVED_BITS.contains(&bit),
                "bit {bit} is measured, 59 independent observations across the four"
            );
        }
    }

    /// `single_bit` reports rather than rejects: neither shape below appears in
    /// any capture, so the type must not pretend to know they are invalid.
    #[test]
    fn game_guide_flag_request_reports_unobserved_shapes_instead_of_refusing_them() {
        let zero = GameGuideFlagRequest { flags: 0 };
        assert_eq!(zero.single_bit(), None);
        let two_bits = GameGuideFlagRequest {
            flags: (1 << 8) | (1 << 9),
        };
        assert_eq!(two_bits.single_bit(), None);
        assert_eq!(two_bits.byte_size(), 8);
    }
}
