//! Job-league wire family: the one request/ack pair our own captures decode.
//!
//! Idea: the whole `0x_0E_` block belongs to the job league (merchant, hunter,
//! thief), and the client dispatches twelve of its opcodes — but a dispatcher
//! entry says only that a frame would be *read*, not what is in it. This module
//! types the pair that real frames cover and leaves the rest named rather than
//! guessed, the way `agent::academy` does for its undecoded record block.
//!
//! **What the captures hold** (`packet_dump/`, our own sessions; the line
//! format is `<ts> <hex body> <P|E>`, and `c2s/` is what we sent):
//!
//! | opcode | dir | frames | bodies |
//! |---|---|---|---|
//! | `0x70E6` | c2s | 2 | `00000000`, `57010000` |
//! | `0xB0E6` | s2c | 2 | `020300`, `020300` |
//! | `0x70E2` | c2s | 1 | `00000000` |
//! | `0xB0E2` | s2c | 1 | `020300` |
//! | `0x70EA` | c2s | 118 | five forms, see [`JobFlagRequest`] |
//! | `0x34D5` | s2c | 92 | `0040` (all 92, across five sessions) |
//!
//! **The pairing is measured, not assumed.** In `admin6/` the ack follows its
//! request by 43–51 ms, three times: `0x70E6` 07:40:34.312 -> `0xB0E6`
//! 07:40:34.355, `0x70E2` 07:41:03.399 -> `0xB0E2` 07:41:03.448, `0x70E6`
//! 08:24:02.177 -> `0xB0E6` 08:24:02.228.
//!
//! **The `u32` is an entity gid.** `57010000` is 343, and the same four bytes
//! are the whole body of `c2s/0x7045.log` (the selection request) three times
//! in the same session; 343 also appears in `0x3019` as the `unique_id` of the
//! record whose `ref_id` is 2037 — `NPC_CH_SOLDIER_SO1`, "Soldier Jingyo
//! [Teleport]" (`characterdata_5000.txt:116`, `textdata_object.txt:9035`).
//!
//! **What the refusal code is not.** Both `0x70E6` attempts were refused with
//! the same `02 03 00`, one with gid 0 and one with a real NPC's gid, so the
//! code does not mean "no target" — and `0xB045` in the same session answers
//! `020300` and `020400` for ordinary selections, which makes 3 a code of the
//! shared precheck vocabulary rather than a job code. Its meaning stays `[U]`:
//! the player's own `textuisystem.txt` carries the league's error sentences
//! (`UIIT_MSG_JOBGUILD_JOIN_ERR_LEVEL` `:3091`, `_OVERLAP` `:3093`, `_EXPIRE`
//! `:3094`, `_EXPENSE` `:3095`), but nothing in the data numbers them, and no
//! capture shows a refusal next to the line the original printed.
//!
//! ## Deliberately not here
//!
//! - `0x70E2`/`0xB0E2` carry exactly the shapes below — one `u32`, then
//!   `result` + code — but which league action they are is unknown: the only
//!   captured body is a zero gid, and the player's data gives two candidate
//!   actions (join, withdraw) without binding either to an opcode. Typing them
//!   would mean naming the action, so they wait for a capture that shows one.
//! - `0x70EA` **is now split** — see [`JobFlagRequest`]. The older note here
//!   ("eight bytes, identical on both frames, no field split readable") was
//!   true of the two frames it had and false as a statement: a sweep over
//!   every dump tree found **118** frames in **five** forms, three of them in
//!   captures of the original client, and the five together make the split
//!   readable.
//! - `0x34D5` is 92 frames of one constant `0040`. That it belongs to this
//!   family at all is somebody else's claim, not a measurement of ours.
//! - `0x30E0` (specialty price list) and `0x30E6` (job experience) have no
//!   frame in any of our dumps; in this repo they exist only as comments.

use bevy::prelude::Message;

use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

/// `result == 1` — the request was accepted.
///
/// Not seen on a job frame of ours; it is the family's own vocabulary, shared
/// with `0xB045`, whose success arm in the same session begins `01`.
pub const JOB_RESULT_SUCCESS: u8 = 1;

/// `result == 2` — refused, and a `u16` code follows.
///
/// Measured on all three job acks we hold (`02 03 00`).
pub const JOB_RESULT_ERROR: u8 = 2;

/// 0x70EA — client → server, eight bytes written as one blob, carrying a
/// 32-bit flag word in which exactly one bit is ever set.
///
/// Idea of the shape: the sender at `0081eda0` forwards its caller's eight
/// bytes verbatim with a single `FUN_00508fe0(&stack0x00000004, 8)`, so the
/// *grouping* cannot be read off the write sizes the way it can elsewhere in
/// this family (`0x704F`'s sender writes `1`, which is how that one-byte
/// action was confirmed). Two things settle it as **one 64-bit word** rather
/// than two dwords: the `0xB0EA` handler (`00881690`, registered at
/// `00870973`) reads it with a single 8-byte read at `008816c3` and then
/// shifts it with `__allshr` (`00b47710`), the 64-bit shift helper — a pair of
/// `u32`s would not be shifted as one value; and across 118 frames in 22 dump
/// trees the upper half is zero every time while the lower half carries
/// **exactly one bit**.
///
/// The handler also says what the word is *for*: it counts the position of the
/// highest set bit, adds 50000, and hands that to `009d4c80`, which subtracts
/// the 50000 again (`add eax, 0xffff3cb0`) before indexing a UI object. The
/// 50000 is a namespace marker, not a value — reading it as one means looking
/// for a meaning that is not there.
///
/// ```text
///                           total   from the original client
/// 00 02 00 00 00 00 00 00    41x    40    2^9
/// 00 00 00 01 00 00 00 00    33x    33    2^24
/// 00 01 00 00 00 00 00 00    28x    22    2^8
/// 00 00 20 00 00 00 00 00    11x    11    2^21
/// 00 00 01 00 00 00 00 00     5x     4    2^16
/// ```
///
/// Byte 0 is zero in all 118, so no bit below 8 has ever been observed. The
/// word stays a full `u64` rather than being narrowed to what varied: the
/// handler reads and shifts 64 bits whatever the frames happen to carry.
///
/// **Which bit means what is `[U]`; that all five are vanilla's is `[V]`.**
/// The caller decides the mask, and `0081eda0` has **no caller in this binary**
/// at all: zero direct `E8` calls, the address appears nowhere as a 4-byte
/// constant (so no vtable), and the one `E9` candidate lands mid-instruction —
/// measured against a positive control of 345 and 816 callers for two
/// neighbours in the same body. That route is closed, not merely incomplete,
/// which leaves the server handler (`00516870`, answering `0xB0EA`) as the way
/// to the names.
///
/// The provenance took three passes and is worth stating once, because two of
/// them were wrong in opposite directions. Every *session* root holding these
/// frames records the **original** client's own chain
/// (`capture-orig/STATE.md:2-4`: `sro_client (Win) -> relay -> sro_proxy ->
/// skrillax`), `cap-src/packet_dump/proxy-local` included — that directory
/// carries 73 of the 118 frames and every frame with bit 24. A census pass had
/// read it as ours through a path-matching artefact.
///
/// Bit 16 is the one to read carefully, and it took three readings to settle.
/// Its five frames sit in two proxy directories and one replay set rather than
/// in a session root named in the census header — which is why a grep over
/// those `# root` lines reported zero for it. The header names **roots**;
/// `discover_roots` stops at the topmost directory holding dumps while the
/// counting itself recurses, so both directories were fully counted and merely
/// unnamed. Four of the five are the original client's.
///
/// The remaining caveat, measured rather than assumed: one of those sets is
/// `or-loginfilm-dreh/fixtures`, and that harness replays recorded bytes rather
/// than recording live ones (`lauf.py`: "The peer only replays bytes we
/// recorded"). Replayed original bytes are still original bytes, but the chain
/// is a copy of a recording instead of a session documented in
/// `capture-orig/STATE.md` — so bit 16 rests on a longer chain than the other
/// four, even though it rests on the original client just the same.
///
/// Cross-checked the other way as well: a root containing a stub invention of
/// ours (`"Stub Server"`, `"Sword Slash"`, `"Gilgamesh"`) is demonstrably ours,
/// and none of the 22 roots does — with our own live session root as the
/// positive control for that search, because the dumps store frames as hex
/// text and a naive byte search finds nothing in any of them. That test is
/// one-way: a hit proves ours, a miss proves nothing, so what carries the four
/// bits is the session note and not the miss.
///
/// The server in those sessions is **skrillax, not vanilla**, so only the
/// **c2s** half proves anything about the original client. This opcode is c2s;
/// the `0xB0EA` answer shape above is read off the client's handler rather than
/// off captured server frames for exactly that reason.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobFlagRequest {
    /// One 64-bit word with exactly one bit set in every captured frame; bits
    /// 8, 9, 16, 21 and 24 observed, and the upper half zero in all 118. Raw
    /// rather than a named enum because a name would have to name the bits.
    pub flags: u64,
}

impl JobFlagRequest {
    /// Every bit our captures contain, all five of them with original-client
    /// frames behind them — 22, 40, 4, 11 and 33 respectively.
    pub const OBSERVED_BITS: [u32; 5] = [8, 9, 16, 21, 24];

    /// The bit this request carries, if it is a single-bit word as every
    /// captured frame was. `None` for zero or for more than one bit — both
    /// unobserved, so neither is rejected, only reported.
    pub fn single_bit(&self) -> Option<u32> {
        (self.flags != 0 && self.flags.is_power_of_two()).then(|| self.flags.trailing_zeros())
    }
}

/// 0x70E6 — client → server: a job-league request about one NPC.
///
/// One `u32`, captured twice (`00000000` and `57010000`). The field name is the
/// gid's own provenance: the same four bytes are the entire body of the
/// selection request `0x7045`, and the value resolves through `0x3019` to an
/// NPC's `unique_id` (see the module note). What the request *asks* of the NPC
/// is `[U]` — both captures were refused.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobNpcRequest {
    pub npc_gid: u32,
}

/// 0xB0E6 — server → client: the answer to [`JobNpcRequest`].
///
/// `u8 result`, then a `u16` code when the result is [`JOB_RESULT_ERROR`].
/// Both captured frames are `02 03 00`; the success arm's tail is unknown,
/// which is why nothing is decoded after the code.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobNpcResponse {
    pub result: u8,
    /// `None` on the success arm. Decoded as a raw `u16` rather than an enum so
    /// an unknown code degrades instead of failing the frame — the same reason
    /// `AgentLoginResponse` keeps its code numeric.
    #[sro_packet(when = "result == JOB_RESULT_ERROR")]
    pub error_code: Option<u16>,
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use bytes::Bytes;

    /// The two captured `c2s/0x70e6.log` bodies, byte for byte, and what the
    /// session says the second one is.
    #[test]
    fn the_request_is_the_gid_the_session_selected() {
        let zero = JobNpcRequest::try_from(Bytes::from_static(&[0x00, 0x00, 0x00, 0x00]))
            .expect("the 07:40:34 frame");
        assert_eq!(zero.npc_gid, 0);
        let npc = JobNpcRequest::try_from(Bytes::from_static(&[0x57, 0x01, 0x00, 0x00]))
            .expect("the 08:24:02 frame");
        assert_eq!(npc.npc_gid, 343);
    }

    /// `admin6/0xb0e6.log`, both frames — and the success arm, which reads no
    /// code and stops there because its tail is in no capture of ours.
    #[test]
    fn the_refusal_carries_its_code_and_success_carries_none() {
        let refused =
            JobNpcResponse::try_from(Bytes::from_static(&[0x02, 0x03, 0x00])).expect("refusal");
        assert_eq!(refused.result, JOB_RESULT_ERROR);
        assert_eq!(refused.error_code, Some(3));

        let ok = JobNpcResponse::try_from(Bytes::from_static(&[JOB_RESULT_SUCCESS]))
            .expect("success arm");
        assert_eq!(ok.error_code, None);
    }

    /// The five captured `0x70EA` bodies, and the property that makes the split
    /// readable: one bit set in the first word, zero in the second. Pinned as a
    /// decode *and* an encode, because the sender writes the eight bytes as one
    /// blob and a wrong field order would still be eight bytes.
    #[test]
    fn job_flag_request_decodes_every_captured_form() {
        let captured: [(&[u8], u32); 5] = [
            (&[0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], 9),
            (&[0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00], 24),
            (&[0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], 8),
            (&[0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00], 21),
            (&[0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00], 16),
        ];

        for (wire, bit) in captured {
            let mut cursor = Cursor::new(wire);
            let decoded = JobFlagRequest::read_from(&mut cursor).expect("decodes");
            assert_eq!(cursor.position() as usize, wire.len());
            assert_eq!(decoded.byte_size(), wire.len());
            assert_eq!(decoded.single_bit(), Some(bit), "body {wire:02x?}");
            assert_eq!(decoded.flags >> 32, 0, "upper half is zero in all captures");
            assert!(JobFlagRequest::OBSERVED_BITS.contains(&bit));

            let bytes: Bytes = decoded.into();
            assert_eq!(bytes.as_ref(), wire, "re-encode must be byte-identical");
        }
    }

    /// `single_bit` reports rather than rejects: neither shape below appears in
    /// any capture, so the type must not pretend to know they are invalid.
    #[test]
    fn job_flag_request_reports_unobserved_shapes_instead_of_refusing_them() {
        let zero = JobFlagRequest { flags: 0 };
        assert_eq!(zero.single_bit(), None);
        let two_bits = JobFlagRequest {
            flags: (1 << 8) | (1 << 9),
        };
        assert_eq!(two_bits.single_bit(), None);
        assert_eq!(two_bits.byte_size(), 8);
    }
}
