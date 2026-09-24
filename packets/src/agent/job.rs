//! Job / trade wire family (merchant, hunter, thief) — the bodies that are
//! decoded, and nothing else.
//!
//! Idea: the family stayed unwired for a long time because its bodies were not
//! confirmed anywhere. It is wired now because both sides agree: the original
//! client's handlers and a vSRO game server's writers describe the same
//! fields, and three of the bodies also appear on real frames. Every field
//! below names where it comes from. Where the source does not name a field,
//! the field keeps a neutral name and says so instead of guessing.
//!
//! Reading primitives, for the width claims below: the client reads and writes
//! with its own fixed-width helpers and strings as `u16 len` + ASCII — which is
//! exactly what this crate's `String` derive does. All widths are 1/2/4 and the
//! whole family is little-endian like the rest of Silkroad.
//!
//! ## What is deliberately NOT here
//!
//! - `0xB0E3` ALIAS: `u8 result` then a string, but the field order after that
//!   is not readable in the original. Not guessed.
//! - `0xB0E5` OUTCOME: the error arm carries an *extra* byte after the `u16`
//!   code, which no other member of the family does; that reading is single
//!   and unconfirmed, so the opcode stays unwired.
//! - `0xB0E1`/`0xB0E2` JOIN/LEAVE acks: their body is the generic
//!   `u8 result [, u16 error]` shape and is already published, but the tail of
//!   the success arm is unknown — the request halves are wired, the acks are not.
//! - `0x30E6` UPDATE_EXP: its semantics are per-level rather than cumulative
//!   and it wants its own change.
//! - `0xB034` sub-ops 19/20 (specialty goods on and off a transport): no frame
//!   of that shape has been seen, so nothing is decoded for them.

use std::io::Read;

use bevy::prelude::Message;
use byteorder::ReadBytesExt;

use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

// --- shared result / error vocabulary --------------------------------------

/// `result == 1` — the request succeeded and a body follows.
///
/// A server writes this byte directly in front of the body of the `0xB0E6`
/// completion.
pub const JOB_RESULT_SUCCESS: u8 = 1;

/// `result == 2` — the request was refused and a `u16` error code follows.
///
/// The generic refusal writes exactly `u8 result = 2` then a 2-byte code, and
/// that three-byte shape is what a real server answers: `0xB0E6 -> 02 03 00`
/// and `0xB0E2 -> 02 03 00`.
pub const JOB_RESULT_ERROR: u8 = 2;

/// The shared NPC pre-check refused the target.
///
/// Sending `0x70E6` with `npc_gid = 0` answers `02 03 00`. It is not a job code
/// at all — the same value comes back from `0xB0E2` and even from `0xB250`
/// (guild), because every `0x70Ex` runs the same pre-check.
pub const JOB_ERROR_INVALID_NPC_TARGET: u16 = 3;

/// The asynchronous database query could not be posted.
pub const JOB_ERROR_QUERY_NOT_POSTED: u16 = 2;

/// `0x4829` — "the old-job-data query returned nothing".
///
/// Not "you have no job": the server compares the stored procedure's first
/// output against 0 and takes this arm when it is `<= 0`.
pub const JOB_ERROR_NO_OLD_JOB_DATA: u16 = 0x4829;

/// Human-readable name for a job error code, for logs and chat lines.
///
/// A lookup rather than a wire enum, for the same reason
/// [`crate::agent::describe_agent_auth_error`] is one: an unknown code must
/// degrade to "unknown", never fail decoding. Only the three codes whose
/// *meaning* is actually sourced are named. The rest of the value space is
/// known — `0x4807, 0x480F, 0x4819, 0x4820, 0x4822..=0x4826, 0x4828..=0x482A,
/// 0x482D, 0x482E` — but what each one means is unknown until the client's own
/// text table is read.
pub fn describe_job_error(code: u16) -> &'static str {
    match code {
        JOB_ERROR_QUERY_NOT_POSTED => "database query could not be posted",
        JOB_ERROR_INVALID_NPC_TARGET => "invalid or out-of-range NPC target",
        JOB_ERROR_NO_OLD_JOB_DATA => "no archived job data",
        _ => "unknown job error",
    }
}

/// Wire values of the job type byte.
///
/// The numbering is the client's own enum (`JobType {Trader=1, Thief=2,
/// Hunter=3}`), and the `0xB0E6` pair order rests on it.
pub const JOB_TYPE_TRADER: u8 = 1;
pub const JOB_TYPE_THIEF: u8 = 2;
pub const JOB_TYPE_HUNTER: u8 = 3;

// --- C→S requests ----------------------------------------------------------

/// 0x70E1 — join a job union at the NPC we are talking to.
///
/// The body is `{u32 npc_gid, u8 union_type}`. The union type is what the
/// server maps to an internal job id (`1 -> 0x14, 2 -> 0x15, 3 -> 0x16`), with
/// "1 = trader" taken from the client's own enum.
///
/// The join fee is **not** derived from that id, as one might expect: it is
/// **(character level − 20) × 5000 gold** (level 20 -> 0, level 30 -> 50 000,
/// level 140 -> 600 000), checked against the character's gold; too little
/// gives error `0x4807`. Below level 20 the refusal `0x4819` is a level gate,
/// not a rejection of the job type — the client's own
/// `UIIT_MSG_JOBGUILD_JOIN_ERR_LEVEL` ("Can join the league from level 20 or
/// above") says the same thing. 140 is this game's level cap.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobJoinRequest {
    pub npc_gid: u32,
    pub enroll_job_union_type: u8,
}

/// 0x70E2 — leave the job, at the NPC we are talking to.
///
/// The body is a single `u32`. Sending `00000000` answers `0xB0E2 02 03 00`.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobLeaveRequest {
    pub npc_gid: u32,
}

/// 0x70E3 — create or change the job alias.
///
/// The body is `{u32, u8, string}`, the string as `u16 len` + ASCII. Position
/// and width of the middle byte are certain; that it carries the job type comes
/// from the original's own code.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobAliasRequest {
    pub npc_gid: u32,
    /// A `u8` sits here; the name comes from the original's own code.
    pub job_type: u8,
    pub alias: String,
}

/// 0x70E4 — ask for a job ranking page.
///
/// The body is `{u32, u8, u8}`. The client's NPC menu only ever offers the
/// contribution ranking for hunters ("contribute rank") and traders ("donation
/// rank") — there is no `THIEFMENU_CONTRIBUTERANK` key — so `rank_kind == 1`
/// with `job_type == 2` is a combination the original never sends.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobRankingRequest {
    pub npc_gid: u32,
    pub job_type: u8,
    pub rank_kind: u8,
}

/// 0x70E5 — ask for the hunter "outcome" figure.
///
/// The body is `{u32, u8}`. A hunter-only menu action (`HUNTERMENU_OUTCOME`
/// exists, `TRADERMENU_OUTCOME` and `THIEFMENU_OUTCOME` do not). The width of
/// the trailing byte is certain, its *name* comes from the original's own code.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobOutcomeRequest {
    pub npc_gid: u32,
    /// Name from the original's own code — see the type comment.
    pub job_type: u8,
}

/// 0x70E6 — ask for the archived ("previous") job figures.
///
/// The body is a single `u32`. A server reads it, runs the shared NPC
/// pre-check and posts the archived-job database query (`_GetOldTrijobData`).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobPrevInfoRequest {
    pub npc_gid: u32,
}

/// 0x74D4 — ask for the goods carried by a transport / export listing.
///
/// The body is a single `u32`. This is the one request of the family
/// **without** an NPC pre-check: a nonsense id answers with an empty list
/// rather than an error.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobExportDetailRequest {
    pub selected_ref_id: u32,
}

// --- 0x30E0 UPDATE_PRICE ----------------------------------------------------

/// One row of the specialty price list.
///
/// Width and order of the two `u32`s are certain; the *names* come from the
/// original's own code, which walks a `map<RefItemID, price>` and writes the
/// key before the value. Which of the two the UI shows as the price is not
/// confirmed.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobPriceEntry {
    /// Map key — most likely the ref item id.
    pub ref_item_id: u32,
    /// Map value — most likely the current price.
    pub price: u32,
}

/// 0x30E0 — the specialty price list.
///
/// `u8 count`, then `count × { u32, u32 }` — body length `1 + 8*count`, and
/// both sides of the original agree on it.
///
/// Prices really do come over the wire: all 43 `ITEM_ETC_TRADE_*` rows in
/// `itemdata*.txt` carry the same placeholder price 383.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobPriceUpdate {
    /// `u8`-counted list — exactly what the derive's default list mode emits.
    pub entries: Vec<JobPriceEntry>,
}

// --- 0x30E7 COS_DISTANCE ----------------------------------------------------

/// `reason == 1` — too far from the trade transport.
pub const COS_DISTANCE_REASON_TRANSPORT: u8 = 1;
/// `reason == 2` — too far from the quest monster.
pub const COS_DISTANCE_REASON_MONSTER: u8 = 2;

/// The transport leash, in game units. **Not in the packet** — the original
/// carries it next to the message key
/// `UIIT_MSG_COSERR_TOO_FAR_FROM_TRADECART`.
pub const COS_LEASH_TRANSPORT: u32 = 100;

/// The quest-monster leash, carried next to
/// `UIIT_MSG_QUEST_ERR_TOO_FAR_FROM_MONSTER`.
pub const COS_LEASH_MONSTER: u32 = 30;

/// 0x30E7 — "you have walked too far away".
///
/// One byte. The distances belonging to the two reasons live in the client,
/// see [`COS_LEASH_TRANSPORT`] / [`COS_LEASH_MONSTER`].
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobCosDistance {
    pub reason: u8,
}

// --- 0x30E8 UPDATE_TRADESCALE ----------------------------------------------

/// 0x30E8 — the current trade scale.
///
/// Exactly one byte on both sides. The scale table
/// `textdata/maxtradescaledata.txt` has six rows — `0, 510, 918, 1428, 2142,
/// 2856` — and the client really loads it. What those six numbers *mean* is
/// not confirmed; with five `UIIT_STT_TRADE_TRADESCALE1..5` labels for six
/// rows, row 0 reads as the "no scale" row.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobTradeScaleUpdate {
    pub trade_scale: u8,
}

// --- 0x34D5 UPDATE_SAFETRADE ------------------------------------------------

/// `'@'` — the neutral code. **Not distinguishable on the wire** from the codes
/// `0x42..=0x4F`: a sender forces those to `state = 0, code = 0x40`, so a
/// `00 40` body means "one of 15 codes".
pub const SAFE_TRADE_CODE_NEUTRAL: u8 = 0x40;
/// `'A'` — `UIIT_MSG_SAFETRADE_NUMBER_ERR_LIMIT`.
pub const SAFE_TRADE_CODE_LIMIT: u8 = 0x41;
/// `'P'` — `UIIT_MSG_SAFETRADE_PROGRESS` = "Safe trade will begin.[%d]/[%d]",
/// which is where the two counter bytes are consumed.
pub const SAFE_TRADE_CODE_PROGRESS: u8 = 0x50;
/// `'Q'` — a fourth body shape of four `u8`s that the original can send; what
/// it means is not confirmed.
pub const SAFE_TRADE_CODE_UNKNOWN_Q: u8 = 0x51;

/// The HUD state index the handler toggles on **both** arms, which is what
/// makes `0x34D5` the safe-mode switch.
pub const SAFE_TRADE_HUD_STATE: u8 = 0xB;

/// Does this `(state, code)` pair carry the two trailing counter bytes?
///
/// The two ends of the original frame this packet differently and the
/// predicate has to satisfy both. The reader branches on the **first** byte:
/// `state == 1` reads three more bytes, `state == 0` reads one. The writer
/// branches on `code - 0x40` and emits four bytes for `'P'`/`'Q'` and two for
/// everything else. Both agree as long as `'P'`/`'Q'` only ever ride on
/// `state == 1`; the union is used so that neither reading can under-read the
/// body if they ever do not.
pub fn safe_trade_carries_counters(state: u8, code: u8) -> bool {
    state == 1 || code == SAFE_TRADE_CODE_PROGRESS || code == SAFE_TRADE_CODE_UNKNOWN_Q
}

/// 0x34D5 — safe-trade state. Two or four bytes, see
/// [`safe_trade_carries_counters`].
///
/// In practice a server sends the two-byte arm `00 40` almost exclusively, and
/// it arrives inside the periodic world-state bundle, usually right behind a
/// `0x3206`. The spacing between frames ranges from seconds to hours, so there
/// is no fixed period to rely on.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobSafeTradeUpdate {
    /// The safe-mode on/off state pushed into HUD state [`SAFE_TRADE_HUD_STATE`].
    pub state: u8,
    /// `'@'`, `'A'`, `'P'`, `'Q'` — the value space is `0x40..=0x51`.
    pub code: u8,
    /// Present exactly on the four-byte arm; "current" per the `[%d]/[%d]`
    /// message.
    #[sro_packet(when = "safe_trade_carries_counters(state, code)")]
    pub current: Option<u8>,
    /// Present exactly on the four-byte arm; "max" per the message.
    #[sro_packet(when = "safe_trade_carries_counters(state, code)")]
    pub max: Option<u8>,
}

// --- 0xB0E4 JOB_RANKING -----------------------------------------------------

/// `rank_kind == 0` — the activity ranking; its rows carry one extra byte.
pub const JOB_RANK_KIND_ACTIVITY: u8 = 0;
/// `rank_kind == 1` — the contribution ("donation") ranking.
pub const JOB_RANK_KIND_CONTRIBUTION: u8 = 1;

/// One ranking row.
///
/// The field order is the original's, and it lines up with the descriptor
/// `resinfo/ifjobrankslot.txt`, whose slots are `GDR_JOB_RANK_RANK, _ALIAS,
/// _GRADE, _GRADENAME, _EXP`.
///
/// `trailing` exists **only** on the activity branch: the contribution branch
/// reads one byte less. What it means is unknown, so it is carried verbatim
/// instead of named.
#[derive(Clone, Debug, PartialEq)]
pub struct JobRankEntry {
    pub rank: u8,
    /// The job alias, not the character name (`_ALIAS` in the descriptor).
    pub name: String,
    pub job_level: u8,
    pub points: u32,
    /// Activity rows only; meaning unknown.
    pub trailing: Option<u8>,
}

/// The name length is a `u16` on the wire, so a longer name is cut to what the
/// length field can name — and cut on a char boundary, because the reader
/// parses the bytes back as UTF-8.
fn clamped_name(name: &str) -> &str {
    const MAX: usize = u16::MAX as usize;
    if name.len() <= MAX {
        return name;
    }
    let mut end = MAX;
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}

impl JobRankEntry {
    /// Reads one row. The row width depends on `rank_kind`, which is why this is
    /// not a plain `Deserialize` impl: the discriminating byte sits in the
    /// enclosing packet, not in the row.
    pub fn read_with_kind<T: Read + ReadBytesExt>(
        reader: &mut T,
        rank_kind: u8,
    ) -> Result<Self, SerializationError> {
        let rank = u8::read_from(reader)?;
        let len = u16::read_from(reader)?;
        let mut bytes = Vec::with_capacity(len as usize);
        for _ in 0..len {
            bytes.push(u8::read_from(reader)?);
        }
        let name = String::from_utf8(bytes)?;
        let job_level = u8::read_from(reader)?;
        let points = u32::read_from(reader)?;
        let trailing = if rank_kind == JOB_RANK_KIND_ACTIVITY {
            Some(u8::read_from(reader)?)
        } else {
            None
        };
        Ok(Self {
            rank,
            name,
            job_level,
            points,
            trailing,
        })
    }

    /// The mirror of [`Self::read_with_kind`], and it needs the same
    /// `rank_kind`: the trailing byte is on the wire iff the *packet's* kind is
    /// [`JOB_RANK_KIND_ACTIVITY`], so writing it off `trailing.is_some()`
    /// produced rows the reader could not read back.
    fn serialize_with_kind(&self, buf: &mut bytes::BytesMut, rank_kind: u8) {
        self.rank.serialize_to(buf);
        let name = clamped_name(&self.name);
        (name.len() as u16).serialize_to(buf);
        for byte in name.as_bytes() {
            byte.serialize_to(buf);
        }
        self.job_level.serialize_to(buf);
        self.points.serialize_to(buf);
        if rank_kind == JOB_RANK_KIND_ACTIVITY {
            self.trailing.unwrap_or_default().serialize_to(buf);
        }
    }

    fn byte_size_with_kind(&self, rank_kind: u8) -> usize {
        1 + 2
            + clamped_name(&self.name).len()
            + 1
            + 4
            + usize::from(rank_kind == JOB_RANK_KIND_ACTIVITY)
    }
}

/// 0xB0E4 — a job ranking page.
///
/// ```text
/// u8 result
/// result == 1: u8 job_type ; u8 rank_kind ; u8 count ; count × row
/// result == 2: u16 error_code            (the generic refusal)
/// ```
/// Hand-written rather than derived for two reasons: the row width depends on
/// `rank_kind`, which no derive list mode can express, and a *server push* must
/// not be a derived enum — an unmatched tag would fail decoding instead of
/// degrading (the same argument `agent::stall` records).
#[derive(Message, Clone, Debug, PartialEq)]
pub struct JobRankingResponse {
    pub result: u8,
    /// Present on `result == 1`.
    pub job_type: Option<u8>,
    /// Present on `result == 1`; [`JOB_RANK_KIND_ACTIVITY`] /
    /// [`JOB_RANK_KIND_CONTRIBUTION`].
    pub rank_kind: Option<u8>,
    pub entries: Vec<JobRankEntry>,
    /// Present on `result == 2`; see [`describe_job_error`].
    pub error_code: Option<u16>,
}

impl JobRankingResponse {
    pub fn is_success(&self) -> bool {
        self.result == JOB_RESULT_SUCCESS
    }
}

impl Deserialize for JobRankingResponse {
    fn read_from<T: Read + ReadBytesExt>(reader: &mut T) -> Result<Self, SerializationError> {
        let result = u8::read_from(reader)?;
        let mut packet = Self {
            result,
            job_type: None,
            rank_kind: None,
            entries: Vec::new(),
            error_code: None,
        };
        match result {
            JOB_RESULT_SUCCESS => {
                let job_type = u8::read_from(reader)?;
                let rank_kind = u8::read_from(reader)?;
                let count = u8::read_from(reader)?;
                let mut entries = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    entries.push(JobRankEntry::read_with_kind(reader, rank_kind)?);
                }
                packet.job_type = Some(job_type);
                packet.rank_kind = Some(rank_kind);
                packet.entries = entries;
            }
            JOB_RESULT_ERROR => packet.error_code = Some(u16::read_from(reader)?),
            // Any other leading byte carries no known body. Returning the bare
            // result beats failing: an unhandled push must not kill the reader.
            _ => {}
        }
        Ok(packet)
    }
}

impl Serialize for JobRankingResponse {
    fn serialize_to(&self, buf: &mut bytes::BytesMut) {
        self.result.serialize_to(buf);
        if self.result == JOB_RESULT_SUCCESS {
            self.job_type.unwrap_or_default().serialize_to(buf);
            let rank_kind = self.rank_kind.unwrap_or_default();
            rank_kind.serialize_to(buf);
            // the count is one byte; a longer list cannot be named on the wire,
            // so the rows are cut with it rather than written past it
            let count = self.entries.len().min(u8::MAX as usize);
            (count as u8).serialize_to(buf);
            for entry in self.entries.iter().take(count) {
                entry.serialize_with_kind(buf, rank_kind);
            }
        } else if self.result == JOB_RESULT_ERROR {
            // only the error arm carries the code; writing it on a success
            // frame appended two bytes the reader never consumes
            self.error_code.unwrap_or_default().serialize_to(buf);
        }
    }
}

impl ByteSize for JobRankingResponse {
    fn byte_size(&self) -> usize {
        let mut size = 1;
        if self.result == JOB_RESULT_SUCCESS {
            let rank_kind = self.rank_kind.unwrap_or_default();
            let count = self.entries.len().min(u8::MAX as usize);
            size += 3 + self
                .entries
                .iter()
                .take(count)
                .map(|entry| entry.byte_size_with_kind(rank_kind))
                .sum::<usize>();
        } else if self.result == JOB_RESULT_ERROR {
            size += 2;
        }
        size
    }
}

/// Mirrors what the derives generate, so the hand-written packet is used like
/// every other one (same pattern as `agent::quest::ActiveQuest`).
impl TryFrom<bytes::Bytes> for JobRankingResponse {
    type Error = SerializationError;

    fn try_from(data: bytes::Bytes) -> Result<Self, Self::Error> {
        use bytes::Buf;
        let mut reader = data.reader();
        JobRankingResponse::read_from(&mut reader)
    }
}

impl From<JobRankingResponse> for bytes::Bytes {
    fn from(packet: JobRankingResponse) -> bytes::Bytes {
        let mut buffer = bytes::BytesMut::with_capacity(packet.byte_size());
        packet.serialize_to(&mut buffer);
        buffer.freeze()
    }
}

// --- 0xB0E6 PREV_JOB_INFO ---------------------------------------------------

/// 0xB0E6 — the archived level/exp of the three jobs.
///
/// The one body in this family that client and server describe identically:
/// `u8 result` and then three `(u8, u32)` pairs, i.e. 16 bytes.
///
/// **The pair order is the reading of the original's own code, not a confirmed
/// fact.** The window writer fetches its widgets in the order MERCHANT,
/// HUNTER, THIEF (`resinfo/ifprevjobinfo.txt` ids `25,30,50,45 · 26,31,51,46 ·
/// 27,32,52,47`) while the handler hands its pairs over as `(1st, 3rd, 2nd)` —
/// read backwards that is wire order MERCHANT, THIEF, HUNTER, matching
/// `JobType {Trader=1, Thief=2, Hunter=3}`. Consistent, but a character
/// without a job at a valid NPC only ever gets `02 29 48`
/// ([`JOB_ERROR_NO_OLD_JOB_DATA`]), so settling it needs a character with exp
/// in exactly one job.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobPrevInfoResponse {
    pub result: u8,
    /// First pair — see the type comment for why the order is not confirmed.
    #[sro_packet(when = "result == JOB_RESULT_SUCCESS")]
    pub merchant_level: Option<u8>,
    #[sro_packet(when = "result == JOB_RESULT_SUCCESS")]
    pub merchant_exp: Option<u32>,
    /// Second pair.
    #[sro_packet(when = "result == JOB_RESULT_SUCCESS")]
    pub thief_level: Option<u8>,
    #[sro_packet(when = "result == JOB_RESULT_SUCCESS")]
    pub thief_exp: Option<u32>,
    /// Third pair.
    #[sro_packet(when = "result == JOB_RESULT_SUCCESS")]
    pub hunter_level: Option<u8>,
    #[sro_packet(when = "result == JOB_RESULT_SUCCESS")]
    pub hunter_exp: Option<u32>,
    /// `02 03 00` comes back for `npc_gid = 0`.
    #[sro_packet(when = "result == JOB_RESULT_ERROR")]
    pub error_code: Option<u16>,
}

// --- 0xB4D4 EXPORT_DETAIL ---------------------------------------------------

/// One carried-goods row: `u32 ref_item_id ; u16 quantity`.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobExportItem {
    pub ref_item_id: u32,
    pub quantity: u16,
}

/// 0xB4D4 — the goods a transport is carrying.
///
/// `u16 count`, then `count × { u32, u16 }`; body `2 + 6*count`.
///
/// **No `result` byte**: `0x74D4` with `selected_ref_id = 0` answers the
/// two-byte body `00 00`, i.e. an empty list, where every other member of the
/// family would answer `02 ..`. It is the only request of the family without
/// the NPC pre-check.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct JobExportDetailResponse {
    /// The `u16` count is explicit because the derive's default list mode counts
    /// with a `u8`.
    pub count: u16,
    #[sro_packet(list_type = "by-size-field", size_field = "count")]
    pub goods: Vec<JobExportItem>,
}

impl JobExportDetailResponse {
    /// Builds a response, keeping the count in step with the rows.
    pub fn new(goods: Vec<JobExportItem>) -> Self {
        Self {
            count: goods.len() as u16,
            goods,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn roundtrip<T>(packet: T, expected: &[u8])
    where
        T: Clone + PartialEq + std::fmt::Debug + Into<Bytes> + TryFrom<Bytes>,
        <T as TryFrom<Bytes>>::Error: std::fmt::Debug,
    {
        let bytes: Bytes = packet.clone().into();
        assert_eq!(bytes.as_ref(), expected);
        assert_eq!(T::try_from(bytes).unwrap(), packet);
    }

    /// `u32 npc_gid ; u8 enroll_job_union_type`, as the original sends it.
    #[test]
    fn join_request_is_a_gid_and_a_union_type() {
        roundtrip(
            JobJoinRequest {
                npc_gid: 0x0000_0157,
                enroll_job_union_type: JOB_TYPE_TRADER,
            },
            &[0x57, 0x01, 0x00, 0x00, 0x01],
        );
    }

    /// A real `0x70E2` frame with the body `00000000`.
    #[test]
    fn leave_request_matches_the_real_frame() {
        roundtrip(JobLeaveRequest { npc_gid: 0 }, &[0x00, 0x00, 0x00, 0x00]);
    }

    /// The alias string is `u16 len` + ASCII bytes.
    #[test]
    fn alias_request_writes_a_length_prefixed_ascii_alias() {
        roundtrip(
            JobAliasRequest {
                npc_gid: 1,
                job_type: JOB_TYPE_HUNTER,
                alias: "Ida".to_owned(),
            },
            &[0x01, 0x00, 0x00, 0x00, 0x03, 0x03, 0x00, b'I', b'd', b'a'],
        );
    }

    #[test]
    fn ranking_and_outcome_and_prev_info_requests_have_their_widths() {
        roundtrip(
            JobRankingRequest {
                npc_gid: 0x1234,
                job_type: JOB_TYPE_HUNTER,
                rank_kind: JOB_RANK_KIND_CONTRIBUTION,
            },
            &[0x34, 0x12, 0x00, 0x00, 0x03, 0x01],
        );
        roundtrip(
            JobOutcomeRequest {
                npc_gid: 2,
                job_type: JOB_TYPE_HUNTER,
            },
            &[0x02, 0x00, 0x00, 0x00, 0x03],
        );
        // The frame that answers with the shared NPC refusal.
        roundtrip(JobPrevInfoRequest { npc_gid: 0 }, &[0x00, 0x00, 0x00, 0x00]);
        // The one request without that pre-check.
        roundtrip(
            JobExportDetailRequest { selected_ref_id: 0 },
            &[0x00, 0x00, 0x00, 0x00],
        );
    }

    /// `1 + 8*count`, pairs in (key, value) order.
    #[test]
    fn price_update_is_a_u8_counted_pair_list() {
        roundtrip(
            JobPriceUpdate {
                entries: vec![
                    JobPriceEntry {
                        ref_item_id: 2147,
                        price: 383,
                    },
                    JobPriceEntry {
                        ref_item_id: 2148,
                        price: 400,
                    },
                ],
            },
            &[
                0x02, 0x63, 0x08, 0x00, 0x00, 0x7F, 0x01, 0x00, 0x00, 0x64, 0x08, 0x00, 0x00, 0x90,
                0x01, 0x00, 0x00,
            ],
        );
        // Body length rule from the handler, on the degenerate case too.
        let empty: Bytes = JobPriceUpdate {
            entries: Vec::new(),
        }
        .into();
        assert_eq!(empty.len(), 1);
    }

    /// One byte, and the two leash distances live in the client, not here.
    #[test]
    fn cos_distance_is_one_reason_byte() {
        roundtrip(
            JobCosDistance {
                reason: COS_DISTANCE_REASON_TRANSPORT,
            },
            &[0x01],
        );
        assert_eq!(COS_LEASH_TRANSPORT, 100);
        assert_eq!(COS_LEASH_MONSTER, 30);
    }

    /// Exactly one byte, on both sides of the original.
    #[test]
    fn trade_scale_update_is_one_byte() {
        roundtrip(JobTradeScaleUpdate { trade_scale: 3 }, &[0x03]);
    }

    /// The neutral form a server really sends: the two-byte body `00 40`,
    /// which must round-trip unchanged.
    #[test]
    fn safe_trade_update_reads_the_two_byte_neutral_form() {
        roundtrip(
            JobSafeTradeUpdate {
                state: 0,
                code: SAFE_TRADE_CODE_NEUTRAL,
                current: None,
                max: None,
            },
            &[0x00, 0x40],
        );
    }

    /// The four-byte arm: `state ; code ; cur ; max`.
    #[test]
    fn safe_trade_update_reads_the_four_byte_progress_arm() {
        roundtrip(
            JobSafeTradeUpdate {
                state: 1,
                code: SAFE_TRADE_CODE_PROGRESS,
                current: Some(2),
                max: Some(5),
            },
            &[0x01, 0x50, 0x02, 0x05],
        );
        // The `'Q'` form is four bytes too — shape known, meaning unknown.
        assert!(safe_trade_carries_counters(0, SAFE_TRADE_CODE_UNKNOWN_Q));
        // …and a plain `'A'` refusal is two bytes.
        assert!(!safe_trade_carries_counters(0, SAFE_TRADE_CODE_LIMIT));
    }

    /// The contribution branch reads one byte less than the activity one.
    #[test]
    fn job_ranking_rows_are_one_byte_shorter_on_the_contribution_branch() {
        let activity = JobRankingResponse {
            result: JOB_RESULT_SUCCESS,
            job_type: Some(JOB_TYPE_HUNTER),
            rank_kind: Some(JOB_RANK_KIND_ACTIVITY),
            entries: vec![JobRankEntry {
                rank: 1,
                name: "Ida".to_owned(),
                job_level: 7,
                points: 4242,
                trailing: Some(0),
            }],
            error_code: None,
        };
        roundtrip(
            activity,
            &[
                0x01, 0x03, 0x00, 0x01, 0x01, 0x03, 0x00, b'I', b'd', b'a', 0x07, 0x92, 0x10, 0x00,
                0x00, 0x00,
            ],
        );

        let contribution = JobRankingResponse {
            result: JOB_RESULT_SUCCESS,
            job_type: Some(JOB_TYPE_TRADER),
            rank_kind: Some(JOB_RANK_KIND_CONTRIBUTION),
            entries: vec![JobRankEntry {
                rank: 1,
                name: "Ida".to_owned(),
                job_level: 7,
                points: 4242,
                trailing: None,
            }],
            error_code: None,
        };
        roundtrip(
            contribution,
            &[
                0x01, 0x01, 0x01, 0x01, 0x01, 0x03, 0x00, b'I', b'd', b'a', 0x07, 0x92, 0x10, 0x00,
                0x00,
            ],
        );
    }

    /// Three frames the serializer used to build and its own reader could not
    /// read back: a `trailing` byte that disagrees with `rank_kind` in either
    /// direction, and an `error_code` written onto a success frame.
    #[test]
    fn job_ranking_never_serialises_a_frame_its_reader_rejects() {
        let row = |trailing| JobRankEntry {
            rank: 1,
            name: "Ida".to_owned(),
            job_level: 7,
            points: 4242,
            trailing,
        };

        // the kind says contribution, so the row has no trailing byte on the
        // wire no matter what the struct holds
        let stray = JobRankingResponse {
            result: JOB_RESULT_SUCCESS,
            job_type: Some(JOB_TYPE_TRADER),
            rank_kind: Some(JOB_RANK_KIND_CONTRIBUTION),
            entries: vec![row(Some(9))],
            error_code: None,
        };
        let bytes: Bytes = stray.clone().into();
        assert_eq!(bytes.len(), stray.byte_size(), "byte_size must agree");
        let back = JobRankingResponse::try_from(bytes.clone()).unwrap();
        assert_eq!(back.entries[0].trailing, None);
        assert_eq!(Bytes::from(back), bytes, "re-encoding is stable");

        // the kind says activity, so the byte is written even when the struct
        // has none — the reader demands it, and used to hit end-of-frame
        let missing = JobRankingResponse {
            result: JOB_RESULT_SUCCESS,
            job_type: Some(JOB_TYPE_HUNTER),
            rank_kind: Some(JOB_RANK_KIND_ACTIVITY),
            entries: vec![row(None)],
            error_code: None,
        };
        let bytes: Bytes = missing.clone().into();
        assert_eq!(bytes.len(), missing.byte_size());
        let back = JobRankingResponse::try_from(bytes).unwrap();
        assert_eq!(back.entries[0].trailing, Some(0));

        // an error code belongs to the refusal arm only
        let success_with_code = JobRankingResponse {
            result: JOB_RESULT_SUCCESS,
            job_type: Some(JOB_TYPE_TRADER),
            rank_kind: Some(JOB_RANK_KIND_CONTRIBUTION),
            entries: Vec::new(),
            error_code: Some(JOB_ERROR_INVALID_NPC_TARGET),
        };
        let bytes: Bytes = success_with_code.clone().into();
        assert_eq!(
            bytes.as_ref(),
            &[
                JOB_RESULT_SUCCESS,
                JOB_TYPE_TRADER,
                JOB_RANK_KIND_CONTRIBUTION,
                0x00
            ]
        );
        assert_eq!(bytes.len(), success_with_code.byte_size());
        assert_eq!(
            JobRankingResponse::try_from(bytes).unwrap().error_code,
            None
        );
    }

    /// The refusal arm, and the rule that an unknown leading byte must not fail.
    #[test]
    fn job_ranking_carries_an_error_code_on_result_two() {
        let refused =
            JobRankingResponse::try_from(Bytes::from_static(&[0x02, 0x03, 0x00])).unwrap();
        assert!(!refused.is_success());
        assert_eq!(refused.error_code, Some(JOB_ERROR_INVALID_NPC_TARGET));
        assert_eq!(
            describe_job_error(JOB_ERROR_INVALID_NPC_TARGET),
            "invalid or out-of-range NPC target"
        );

        let odd = JobRankingResponse::try_from(Bytes::from_static(&[0x07])).unwrap();
        assert_eq!(odd.result, 7);
        assert!(odd.entries.is_empty());
    }

    /// `01` + 3 × `(u8, u32)` = 16 bytes, pair order MERCHANT, THIEF, HUNTER.
    #[test]
    fn prev_job_info_success_body_is_sixteen_bytes() {
        let packet = JobPrevInfoResponse {
            result: JOB_RESULT_SUCCESS,
            merchant_level: Some(3),
            merchant_exp: Some(1000),
            thief_level: Some(0),
            thief_exp: Some(0),
            hunter_level: Some(1),
            hunter_exp: Some(7),
            error_code: None,
        };
        let bytes: Bytes = packet.clone().into();
        assert_eq!(bytes.len(), 16);
        assert_eq!(
            bytes.as_ref(),
            &[
                0x01, 0x03, 0xE8, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x07, 0x00,
                0x00, 0x00
            ]
        );
        assert_eq!(JobPrevInfoResponse::try_from(bytes).unwrap(), packet);
    }

    /// The refusal `02 03 00` and the "no archived data" code `0x4829`.
    #[test]
    fn prev_job_info_refusal_is_a_three_byte_frame() {
        let refused =
            JobPrevInfoResponse::try_from(Bytes::from_static(&[0x02, 0x03, 0x00])).unwrap();
        assert_eq!(refused.result, JOB_RESULT_ERROR);
        assert_eq!(refused.error_code, Some(JOB_ERROR_INVALID_NPC_TARGET));
        assert_eq!(refused.merchant_level, None);

        let no_data =
            JobPrevInfoResponse::try_from(Bytes::from_static(&[0x02, 0x29, 0x48])).unwrap();
        assert_eq!(no_data.error_code, Some(JOB_ERROR_NO_OLD_JOB_DATA));
        assert_eq!(
            describe_job_error(JOB_ERROR_NO_OLD_JOB_DATA),
            "no archived job data"
        );
    }

    /// The empty answer is exactly `00 00` — no result byte.
    #[test]
    fn export_detail_empty_answer_is_two_zero_bytes() {
        let empty = JobExportDetailResponse::try_from(Bytes::from_static(&[0x00, 0x00])).unwrap();
        assert_eq!(empty.count, 0);
        assert!(empty.goods.is_empty());
        let bytes: Bytes = empty.into();
        assert_eq!(bytes.as_ref(), &[0x00, 0x00]);
    }

    /// `2 + 6*count`.
    #[test]
    fn export_detail_rows_are_six_bytes_each() {
        let packet = JobExportDetailResponse::new(vec![
            JobExportItem {
                ref_item_id: 2147,
                quantity: 5,
            },
            JobExportItem {
                ref_item_id: 10394,
                quantity: 1,
            },
        ]);
        assert_eq!(packet.count, 2);
        let bytes: Bytes = packet.clone().into();
        assert_eq!(bytes.len(), 2 + 6 * 2);
        assert_eq!(
            bytes.as_ref(),
            &[0x02, 0x00, 0x63, 0x08, 0x00, 0x00, 0x05, 0x00, 0x9A, 0x28, 0x00, 0x00, 0x01, 0x00]
        );
        assert_eq!(JobExportDetailResponse::try_from(bytes).unwrap(), packet);
    }
}
