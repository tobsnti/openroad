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
//! //! | `0x34D5` | s2c | 92 | `0040` (all 92, across five sessions) |
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
//! - `0x70E2`/`0xB0E2` carry one `u32`, then `result` + code, without the
//!   success arm of 0xB0E6 `[V original handler]` — but which league action
//!   they are is unknown: the only captured body is a zero gid, and the
//!   player's data gives two candidate actions (join, withdraw) without
//!   binding either to an opcode. Typing them would mean naming the action,
//!   so they wait for a capture that shows one.
//! - `0x70EA` moved to [`crate::agent::guide`] — its handler indexes a UI
//!   ("eight bytes, identical on both frames, no field split readable") was
//!   true of the two frames it had and false as a statement: a sweep over
//!   every dump tree found **118** frames in **five** forms, three of them in
//!   captures of the original client, and the five together make the split
//!   readable.
//! - `0x34D5` is 92 frames of one constant `0040`. That it belongs to this
//!   family at all is somebody else's claim, not a measurement of ours.
//! - `0x30E0` (specialty price list) and `0x30E6` (job experience) have no
//!   frame in any of our dumps; in this repo they exist only as comments.

use crate::Message;

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
/// `u8 result`, then [`JobNpcAnswer`] on [`JOB_RESULT_SUCCESS`] and a `u16`
/// code on [`JOB_RESULT_ERROR`] `[V original handler]`. Both captured frames
/// are `02 03 00`, so no capture shows the success arm.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobNpcResponse {
    pub result: u8,
    #[sro_packet(when = "result == JOB_RESULT_SUCCESS")]
    pub answer: Option<JobNpcAnswer>,
    /// `None` on the success arm. Decoded as a raw `u16` rather than an enum so
    /// an unknown code degrades instead of failing the frame — the same reason
    /// `AgentLoginResponse` keeps its code numeric.
    #[sro_packet(when = "result == JOB_RESULT_ERROR")]
    pub error_code: Option<u16>,
}

/// The success arm of [`JobNpcResponse`]: three `u8, u32` pairs
/// `[V original handler]`. What they hold is [U].
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobNpcAnswer {
    pub unk_u8_00: u8,
    pub unk_u32_00: u32,
    pub unk_u8_01: u8,
    pub unk_u32_01: u32,
    pub unk_u8_02: u8,
    pub unk_u32_02: u32,
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

    /// `admin6/0xb0e6.log`, both frames — and the success arm, which reads its
    /// three pairs to the last byte and no code.
    #[test]
    fn the_refusal_carries_its_code_and_success_carries_none() {
        let refused =
            JobNpcResponse::try_from(Bytes::from_static(&[0x02, 0x03, 0x00])).expect("refusal");
        assert_eq!(refused.result, JOB_RESULT_ERROR);
        assert_eq!(refused.error_code, Some(3));

        let mut success = vec![JOB_RESULT_SUCCESS];
        for pair in 1u8..=3 {
            success.push(pair);
            success.extend(u32::from(pair).to_le_bytes());
        }
        assert_eq!(
            crate::judge::judge(0xB0E6, &success),
            crate::judge::Verdict::Exact
        );
        let ok = JobNpcResponse::try_from(Bytes::from(success)).expect("success arm");
        assert_eq!(ok.error_code, None);
        assert_eq!(ok.answer.map(|answer| answer.unk_u32_02), Some(3));
    }
}
