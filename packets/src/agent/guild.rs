//! Guild wire opcodes: the chunked guild-record transfer (0x34B3 / 0x3101 /
//! 0x34B4), the create-guild echo (0xB0F0), the notice edit request (0x70F9)
//! and two capture-gated pushes (0x30FF, 0x38F5).
//!
//! **Spec-derived, not capture-verified.** No `packet_dump/` sample exists for
//! any of these; layouts come from statically reading the original client's
//! parser/builder. Byte-level notes, per-field [V]/[S]/[U] tags and the
//! resolving capture for each unknown live in `docs/net-guild-0x3101.md`.
//!
//! The record arrives **chunked**: BEGIN, then one or more DATA bodies whose
//! payloads concatenate, then END, at which point the original parses the
//! assembled buffer in one go. So [`GuildDataBody`] is a raw passthrough and
//! [`GuildData`] is the parsed shape — the same split the character-data blob
//! uses ([`CharacterDataBody`](crate::agent::ingame::CharacterDataBody)),
//! except that guild records need no itemdata resolver, so [`GuildData`] is a
//! plain derive and [`GuildData::parse`] is all the staging that is required.
//!
//! Guild storage (0x7250/0xB250), guild chat and alliance/union live in their
//! own docs and are not here.

use bevy::prelude::Message;
use bytes::{BufMut, Bytes, BytesMut};

use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

/// `SRGuildMember.Permissions` — a `[Flags] uint`, so it is a newtype rather
/// than a derived enum: real servers can set bits this list does not name, and
/// an unknown discriminator would otherwise fail the whole packet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GuildPermissions(pub u32);

impl GuildPermissions {
    pub const JOIN: u32 = 0x0000_0001;
    pub const KICK: u32 = 0x0000_0002;
    pub const UNION_CHAT: u32 = 0x0000_0004;
    pub const STORAGE: u32 = 0x0000_0008;
    pub const NOTICE: u32 = 0x0000_0010;
    /// Everything a non-master can hold.
    pub const ALL: u32 = 0x0000_001F;
    /// The guild master's sentinel — every bit set, not just [`Self::ALL`].
    pub const MASTER: u32 = 0xFFFF_FFFF;

    pub fn can_invite(&self) -> bool {
        self.0 & Self::JOIN != 0
    }

    pub fn can_kick(&self) -> bool {
        self.0 & Self::KICK != 0
    }

    pub fn can_union_chat(&self) -> bool {
        self.0 & Self::UNION_CHAT != 0
    }

    pub fn can_use_storage(&self) -> bool {
        self.0 & Self::STORAGE != 0
    }

    pub fn can_edit_notice(&self) -> bool {
        self.0 & Self::NOTICE != 0
    }

    /// The master sentinel, distinct from merely holding every named right.
    pub fn is_master_sentinel(&self) -> bool {
        self.0 == Self::MASTER
    }
}

/// One roster entry inside the assembled guild record.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildMember {
    pub member_id: u32,
    pub name: String,
    /// [U] — read verbatim, semantics unknown.
    pub unk_u8_01: u8,
    pub level: u8,
    pub guild_points: u32,
    /// See [`GuildPermissions`].
    pub permissions: u32,
    /// Read verbatim ×3, semantics unknown.
    pub unk_u32_01: u32,
    pub unk_u32_02: u32,
    pub unk_u32_03: u32,
    pub nickname: String,
    pub model_id: u32,
    pub is_master: bool,
    /// Not present in the spec-derived layout this struct came from. A live
    /// record with the founder as the only member ends `01 01 00`: with the old two-flag tail that read
    /// `is_master = 1, is_offline = 1`, which contradicts the fact that the
    /// character was standing in the world at that moment. Reading it as
    /// `is_master = 1`, this byte `= 1`, `is_offline = 0` fits both facts, so the
    /// byte is real and sits here. Its meaning stays open: a record with an
    /// offline member decides whether it is a second flag, a rank, or padding.
    pub unk_u8_02: u8,
    pub is_offline: bool,
}

impl GuildMember {
    pub fn permissions(&self) -> GuildPermissions {
        GuildPermissions(self.permissions)
    }
}

/// The assembled guild record — the concatenation of every [`GuildDataBody`]
/// between BEGIN and END, and also the tail of a successful [`GuildCreatedData`].
///
/// Not a wire type of its own: nothing carries this as a single packet body,
/// which is why it is parsed via [`Self::parse`] rather than registered.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildData {
    pub guild_id: u32,
    pub name: String,
    pub level: u8,
    pub guild_points: u32,
    pub notice: String,
    pub message: String,
    /// [U] — read verbatim, semantics unknown.
    pub unk_u32_00: u32,
    /// [U] — read verbatim, semantics unknown.
    pub unk_u8_00: u8,
    pub member_count: u8,
    #[sro_packet(list_type = "by-size-field", size_field = "member_count")]
    pub members: Vec<GuildMember>,
}

impl GuildData {
    /// Parse an assembled record. The original reads exactly to the end of the
    /// last member, so a well-formed buffer leaves no tail.
    pub fn parse(assembled: Bytes) -> Result<Self, SerializationError> {
        Self::try_from(assembled)
    }

    /// Parse a record that is **followed by** other bytes, returning how many it
    /// consumed. Needed because `0xB0F0` carries the record inline and then one
    /// more byte, so the whole-buffer form cannot be used
    /// there. Uses the same generated reader as [`Self::parse`] — no second
    /// parser.
    pub fn parse_prefix(buf: Bytes) -> Result<(Self, usize), SerializationError> {
        let mut cursor = std::io::Cursor::new(buf.as_ref());
        let data = <Self as sro_macro::Deserialize>::read_from(&mut cursor)?;
        Ok((data, cursor.position() as usize))
    }
}

/// 0x34B3 — server → client: the guild record starts. Marker, empty body.
///
/// The original reads no bytes here. Whether a length or id prefix exists is
/// [U] — resolving capture `packet_dump/0x34B3.log`.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildDataBegin;

/// 0x3101 — server → client: one chunk of the guild record.
///
/// Carried unparsed: a single chunk is not a complete record, so decoding it as
/// [`GuildData`] would fail on every packet but the last. Accumulate the bodies
/// between BEGIN and END, then call [`GuildData::parse`].
#[derive(Message, Clone, Debug, PartialEq)]
pub struct GuildDataBody {
    pub data: Bytes,
}

impl TryFrom<Bytes> for GuildDataBody {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        Ok(GuildDataBody { data: value })
    }
}

impl From<GuildDataBody> for Bytes {
    fn from(p: GuildDataBody) -> Self {
        p.data
    }
}

/// 0x34B4 — server → client: the guild record is complete. Marker, empty body.
///
/// Same [U] as [`GuildDataBegin`] — the original reads no bytes and takes no
/// packet argument; it only closes the accumulator.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildDataEnd;

/// 0x30FF — server → client: an entity's guild affiliation changed.
///
/// **Renamed** from `GuildPlayerLog`: that name (and the "no parser at all"
/// note that stood here) came from xBot, and reading the original's handler
/// disproves both. It has a parser, and what it carries is the
/// spawn record's guild block for one already-spawned entity — the live update
/// of the same state, pairing with [`EntityGuildRemove`] (0x3100) which clears
/// it.
///
/// The tail is genuinely conditional, not merely empty: the handler gates every
/// read after the name on `if (name_len != 0)`, so a guildless subject
/// is `10 + len(name)` bytes and stops there.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct EntityGuildUpdate {
    /// The subject entity's unique id resolves it in the
    /// world-object map — the same accessor family the other entity pushes use).
    pub entity_id: u32,
    pub guild_id: u32,
    /// Empty means "no guild": the client then reads nothing further.
    pub guild_name: String,
    #[sro_packet(when = "!guild_name.is_empty()")]
    pub affiliation: Option<EntityGuildAffiliation>,
}

/// The part of [`EntityGuildUpdate`] a guildless subject omits.
///
/// Field *meaning* here rests on the callees, not on a name table:
/// builds `"G%u_%u_%u.crb"` from (shard, guild id, crest rev)
/// and `"A%u_%u_%u.crb"` from (shard, union id, union crest rev), which is what
/// pins the two revision fields to their ids; writes the
/// fortress byte to `this+0x81c` and switches it as a **bit-valued** enum
/// (1 commander, 2 sub-commander, 4 battle-manager, 8 product-manager,
/// 0x10 trainer-manager, 0x20 engineer).
///
/// Note the order of the two trailing bytes: this is the first-hand read
/// (position, then relation). The SilkroadDoc layout lists them the other way
/// round (`isFriendly`, then `siegeAuthority`). Neither byte has a consumer that
/// keys colour off it yet, so the disagreement is recorded rather than silently
/// resolved — a record from a guild-war member settles it.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct EntityGuildAffiliation {
    /// The guild-granted nickname.
    pub granted_nick: String,
    /// `-1` means "leave the crest unchanged", `0` means "clear it".
    pub crest_rev: u32,
    pub union_id: u32,
    pub union_crest_rev: u32,
    /// Fortress-guild position, bit-valued (see the type docs).
    pub fortress_position: u8,
    /// 0/1; the polarity is as open as 0x30EF's relation byte.
    pub relation_flag: u8,
}

/// 0x38F5 — server → client: an incremental guild update.
///
/// The original reads `update_type` and then switches on it with an **empty**
/// body for every arm, so only the discriminator is known: 5 = notice,
/// 6 = permissions, 15 = ?. The rest is kept raw rather than guessed — one
/// capture per type (`packet_dump/0x38F5.log`) is what resolves it.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct GuildUpdate {
    pub update_type: u8,
    /// [U] — the per-type payload, unparsed.
    pub tail: Bytes,
}

impl TryFrom<Bytes> for GuildUpdate {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let update_type = *value.first().ok_or_else(|| {
            SerializationError::IoError(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "empty 0x38F5 body",
            ))
        })?;
        Ok(GuildUpdate {
            update_type,
            tail: value.slice(1..),
        })
    }
}

impl From<GuildUpdate> for Bytes {
    fn from(p: GuildUpdate) -> Self {
        let mut buf = BytesMut::with_capacity(1 + p.tail.len());
        buf.put_u8(p.update_type);
        buf.extend_from_slice(&p.tail);
        buf.freeze()
    }
}

/// 0xB0F0 — server → client: the answer to creating a guild. On success it
/// carries the same record as the chunked transfer, inline.
///
/// **Corrected against the wire.** It was modelled as `success: bool` plus the
/// record, which cannot decode a refusal: the wire uses the guild family's
/// standard ack shape — `u8 result`, and on `result == 2` a `u16` error. A cold
/// `0x70F0` (character standing on the NPC, no dialogue open) answers
/// `02 03 00`, i.e. error `0x0003`, which is **not** from the `0x4Cxx` GUILDERR
/// space; the NPC talk chain (`0x7045` / `0x7046`) first makes the same request
/// succeed. So the creation needs an open NPC session, and `0x0003` reads as
/// "no dialogue" rather than anything about the guild.
///
/// The success arm confirmed [`GuildData`] field-for-field against real bytes
/// for the first time — the record had been spec-derived until now. From a
/// freshly founded guild with one member: `level = 5` — a fresh guild does not
/// start at 1 — `member_id = 5`, `level = 68` (the character's real level),
/// `permissions = 0xFFFFFFFF` (the master sentinel [`GuildPermissions`] already
/// knew), `model_id = 1931` (her character ref id), `is_master = 1`.
///
/// The member tail turned out to be **three** bytes, not two: read as two it
/// made `is_offline = 1` for a character that was demonstrably standing in the
/// world. The extra byte is [`GuildMember::unk_u8_02`], and with it the record
/// leaves [`Self::tail`] empty — which is what shows the whole record is
/// aligned rather than merely plausible.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct GuildCreatedData {
    pub result: u8,
    /// Present when `result == 1`.
    pub data: Option<GuildData>,
    /// Present when `result != 1`. The refusal code; `0x0003` = no NPC dialogue
    /// open.
    pub error: Option<u16>,
    /// Anything after the record. On the wire this is **empty**
    /// once the member tail is read correctly — the byte that looked like a
    /// trailing one belongs to [`GuildMember::unk_u8_02`]. Kept so a future
    /// server that appends something does not lose it silently.
    pub tail: Bytes,
}

impl TryFrom<Bytes> for GuildCreatedData {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let result = *value.first().ok_or_else(|| {
            SerializationError::IoError(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "empty 0xB0F0 body",
            ))
        })?;
        if result != 1 {
            let error = (value.len() >= 3).then(|| u16::from_le_bytes([value[1], value[2]]));
            return Ok(GuildCreatedData {
                result,
                data: None,
                error,
                tail: value.slice(value.len().min(3)..),
            });
        }
        // The record is length-prefixed only by its own member count, so parse it
        // and treat whatever is left as the trailing byte(s) rather than assuming
        // the body ends exactly there.
        let (data, consumed) = GuildData::parse_prefix(value.slice(1..))?;
        Ok(GuildCreatedData {
            result,
            data: Some(data),
            error: None,
            tail: value.slice(1 + consumed..),
        })
    }
}

impl From<GuildCreatedData> for Bytes {
    fn from(p: GuildCreatedData) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u8(p.result);
        if let Some(data) = p.data {
            buf.extend_from_slice(&Bytes::from(data));
        }
        if let Some(error) = p.error {
            buf.put_u16_le(error);
        }
        buf.extend_from_slice(&p.tail);
        buf.freeze()
    }
}

/// 0x70F9 — client → server: edit the guild notice.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildNoticeEditRequest {
    pub title: String,
    pub message: String,
}

/// The guild command acks share **one body form**: `{result:u8}` on success,
/// `{result:u8, error_code:u16}` on refusal. That is not an assumption — the
/// eleven handlers sit in one cluster, and every one
/// of them reads the same two fields in the same order, and the server writers
/// answer with the single byte `01` on success. Modelling them as one
/// generated form keeps that finding visible instead of copying a struct
/// eleven times.
///
/// Each ack still gets its own type, because the opcode table in
/// [`crate::Packet`] maps one opcode to one type.
macro_rules! guild_op_ack {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        ///
        /// Body: `result:u8` (1 = ok, 2 = error); on `result != 1` a `u16`
        /// error code follows. 1 byte on success, 3 on failure.
        #[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
        pub struct $name {
            pub result: u8,
            #[sro_packet(when = "result != 1")]
            pub error_code: Option<u16>,
        }

        impl $name {
            /// `result == 1`. The original tests for exactly this value and
            /// treats every other value as the error arm.
            pub fn is_success(&self) -> bool {
                self.result == 1
            }
        }
    };
}

/// Re-exported for the union family, whose acks live in the same handler
/// cluster ([`crate::agent::guild_union`]).
pub(crate) use guild_op_ack;

guild_op_ack! {
    /// 0xB0F1 — ack for the guild **disband** request (C→S `0x70F1`), handler
    ///.
    ///
    /// Which of 0xB0F1/0xB0F2 is disband and which is leave is not stated by
    /// either handler — both refresh the same guild window. Two independent
    /// sources decide it the same way: the C→S catalog pairs `0x70F1` with
    /// disband and `0x70F2` with leave/secede, and the server function that
    /// emits the *kick* ack `0xB0F4` emits `0xB0F2` on its other branch
    ///, "kick vs leave") — so `0xB0F2` is the
    /// member-side verb and `0xB0F1` is the one left over. Naming is [S],
    /// the layout is [V].
    GuildDisbandAck
}

guild_op_ack! {
    /// 0xB0F2 — ack for the guild **leave/secede** request (C→S `0x70F2`),
    /// handler. See [`GuildDisbandAck`] for why this one is
    /// leave rather than disband.
    GuildLeaveAck
}

guild_op_ack! {
    /// 0xB0F3 — ack for the guild **invite** (C→S `0x70F3`
    /// [`crate::agent::ingame::GuildInviteRequest`]), handler.
    ///
    /// The success arm is deliberately inert in the original: the inviter
    /// learns nothing here, the invitee gets the `0x3080` petition popup. The
    /// error arm additionally clears the client's "invitation pending" flag.
    GuildInviteAck
}

guild_op_ack! {
    /// 0xB0F4 — ack for **kicking** a member (C→S `0x70F4`); SilkroadDoc calls it `AGENT_GUILD_KICK`.
    ///
    /// The success arm does nothing: the roster change arrives out of band via
    /// `0x3100`/`0x38F5`. Server writer emits the single
    /// byte `01`.
    GuildKickAck
}

guild_op_ack! {
    /// 0xB0F9 — ack for the notice edit ([`GuildNoticeEditRequest`], C→S
    /// `0x70F9`), handler.
    ///
    /// On success the original shows `UIIT_MSG_GUILD_COMMON_KNOW_REMIND_UPDATE`
    /// and reads nothing further; the server writer
    /// agrees byte-for-byte.
    GuildNoticeEditAck
}

guild_op_ack! {
    /// 0xB0FA — ack for **promote/demote** (C→S `0x70FA`); SilkroadDoc calls it `AGENT_GUILD_PROMOTE`.
    ///
    /// Success refreshes the guild panel and carries no payload. The server
    /// side charges a per-grade cost with the grade
    /// clamped to 2..=5, which is where the C→S grade range comes from — that
    /// request is not modelled here, only its ack.
    GuildPromoteAck
}

guild_op_ack! {
    /// 0xB104 — ack for a member **permission update** (C→S `0x7104`); SilkroadDoc calls it `AGENT_GUILD_UPDATE_PERMISSION`.
    ///
    /// Success is empty; the new bitmask itself arrives inside the guild record
    /// as [`GuildMember::permissions`]. Server writer.
    GuildPermissionUpdateAck
}

/// 0xB0F6 — ack for the guild **GP donation** (C→S `0x70F6`); SilkroadDoc calls it `AGENT_GUILD_DONATE_OBSOLETE`.
///
/// The one member of the cluster whose success arm is not empty: it carries the
/// donated guild points, which the original formats into
/// `UIIT_MSG_GUILD_GP_SUBSCRIPION_RESULT` (the misspelling is the original's).
/// The string literal is resolved inside the decompiled handler, so the `u32`
/// is unambiguously the contributed amount rather than a running total.
/// 5 bytes on success, 3 on failure.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildDonateAck {
    pub result: u8,
    #[sro_packet(when = "result == 1")]
    pub donated_gp: Option<u32>,
    #[sro_packet(when = "result != 1")]
    pub error_code: Option<u16>,
}

impl GuildDonateAck {
    pub fn is_success(&self) -> bool {
        self.result == 1
    }
}

/// 0x3100 — server → client: this entity is no longer in a guild.
///
/// A bare entity id. The client drops the guild association on the spawned
/// object; the server emits one of these per member while disbanding a guild
/// writes exactly one `u32` per member, the handler
/// reads exactly one).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct EntityGuildRemove {
    pub entity_id: u32,
}

/// 0x70F0 — client → server: create a guild.
///
/// Builder: one `b4` then an ASCII string. The leading `u32` is the guild
/// **NPC's** unique id — creation is an NPC dialogue in the original, the same
/// shape the storage opener [`crate::agent::guild_storage::GuildStorageOpenRequest`]
/// uses — but the original only shows the width, so the name is inferred and
/// the value's origin is the window's target slot.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildCreateRequest {
    pub npc_unique_id: u32,
    pub name: String,
}

/// 0x70F1 — client → server: disband the guild. Builder,
/// body `b4`.
///
/// The `u32` comes from the guild window's accessor the original's routine
/// (`this+0x628`) — the *same* slot 0x70F2 reads. Whether that slot holds the
/// guild id or the current selection is [U], so the field is not named after a
/// guess; a capture of either request decides it. See [`GuildDisbandAck`] for
/// why this opcode is disband and 0x70F2 is leave.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildDisbandRequest {
    /// [U] — the guild window's `+0x628` slot, verbatim.
    pub unk_u32_00: u32,
}

/// 0x70F2 — client → server: leave/secede from the guild. Builder
///, body `b4` from the same accessor as
/// [`GuildDisbandRequest`].
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildLeaveRequest {
    /// [U] — the guild window's `+0x628` slot, verbatim.
    pub unk_u32_00: u32,
}

/// 0x70F4 — client → server: kick a member, addressed **by name**, not by id.
/// Builder, body `strA`.
///
/// The name-addressed form is the notable part: every other membership op in
/// the family carries a `u32`, so a consumer must pass the roster row's name
/// ([`GuildMember::name`]) rather than its `member_id`.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildKickRequest {
    pub member_name: String,
}

/// 0x70FA — client → server: promote/demote a member. Builder
///, body `b4`.
///
/// The ack's server side charges a cost indexed by the new grade with the grade
/// clamped to `2..=5`, so grades 2..5 are the promotable range;
/// whether this `u32` is that grade or the member id is [U] — one `b4` is all
/// the builder shows.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildPromoteRequest {
    /// [U] — one `u32`, either the target member or the target grade.
    pub unk_u32_00: u32,
}

/// 0x7104 — client → server: the guild permission update.
///
/// **Corrected**: this used to be modelled as a single `u8` called a
/// "selector", on the reading that one byte cannot be a 32-bit mask. The byte
/// is a **count**, and the pairs follow it — which the ack's own server read
/// loop already said (`u8 count` + `count x {u32,u32}`) and which the *client's*
/// builder confirms independently: it writes `count = (end-start)>>3`, then two
/// `u32` per pair. Two sources, opposite ends of the wire.
///
/// It is a **delta**: the dialog keeps the member ids and their values in two
/// parallel vectors and pushes a pair only where the value differs from the
/// member's current one, so an unchanged row is not on the wire. `count = 0` is
/// wire-legal (the server loop simply runs zero times), but the original never
/// sends it — it enters the builder only with at least one pair.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildPermissionUpdateRequest {
    pub count: u8,
    #[sro_packet(list_type = "by-size-field", size_field = "count")]
    pub changes: Vec<GuildPermissionChange>,
}

/// One row of [`GuildPermissionUpdateRequest`].
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildPermissionChange {
    /// The key of the guild-member map `(mgr+0x148)+0x88`: it is the `0x3101`
    /// record's `member_id`, not the character's `unique_id`.
    ///
    /// The ack does **not** discriminate — even a garbage id comes back as
    /// `01`. What settles it is the *record that follows*: only the record's
    /// `member_id` actually changes the mask, while the character's unique id
    /// changes nothing although it is acknowledged the same way.
    pub member_id: u32,
    /// The value the dialog edits, `member+0x2C`. That it is the permission
    /// mask is an inference, not a confirmed reading: the strong indication is
    /// what is written there — the master sentinel [`GuildPermissions`] already
    /// knows it.
    pub permission_mask: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ascii(s: &str) -> Vec<u8> {
        let mut out = (s.len() as u16).to_le_bytes().to_vec();
        out.extend(s.as_bytes());
        out
    }

    fn guild_record(members: &[(&str, u32)]) -> Vec<u8> {
        let mut out = 0x2A_u32.to_le_bytes().to_vec(); // guild_id
        out.extend(ascii("Wanderers"));
        out.push(5); // level
        out.extend(1234u32.to_le_bytes()); // guild_points
        out.extend(ascii("notice text"));
        out.extend(ascii("motd"));
        out.extend(0u32.to_le_bytes()); // unk_u32_00
        out.push(0); // unk_u8_00
        out.push(members.len() as u8);
        for (name, perms) in members {
            out.extend(0x1234u32.to_le_bytes()); // member_id
            out.extend(ascii(name));
            out.push(0); // unk_u8_01
            out.push(40); // level
            out.extend(99u32.to_le_bytes()); // guild_points
            out.extend(perms.to_le_bytes());
            out.extend([0u8; 12]); // unk_u32_01..03
            out.extend(ascii("nick"));
            out.extend(1907u32.to_le_bytes()); // model_id
            out.push(u8::from(*perms == GuildPermissions::MASTER));
            // The tail is THREE bytes, not two: a live record ends `01 01 00`
            // for an online master. See `GuildMember::unk_u8_02`.
            out.push(1); // unk_u8_02 — unnamed, 1 on the wire
            out.push(0); // is_offline
        }
        out
    }

    /// The assembled record round-trips, and the roster is sized by the header
    /// count rather than read to EOF.
    #[test]
    fn an_assembled_guild_record_round_trips() {
        let wire = Bytes::from(guild_record(&[
            ("Master", GuildPermissions::MASTER),
            ("Grunt", GuildPermissions::JOIN | GuildPermissions::STORAGE),
        ]));

        let decoded = GuildData::parse(wire.clone()).unwrap();

        assert_eq!(decoded.guild_id, 0x2A);
        assert_eq!(decoded.name, "Wanderers");
        assert_eq!(decoded.notice, "notice text");
        assert_eq!(decoded.members.len(), 2);
        assert_eq!(decoded.members[1].nickname, "nick");
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// `permissions` is a bitfield, so the master sentinel is every bit set —
    /// not merely holding each named right — and a member can hold an
    /// arbitrary subset.
    #[test]
    fn permissions_decode_as_flags_not_an_enum() {
        let wire = Bytes::from(guild_record(&[
            ("Master", GuildPermissions::MASTER),
            ("Grunt", GuildPermissions::JOIN | GuildPermissions::STORAGE),
        ]));
        let decoded = GuildData::parse(wire).unwrap();

        let master = decoded.members[0].permissions();
        assert!(master.is_master_sentinel());
        assert!(master.can_kick() && master.can_edit_notice());

        let grunt = decoded.members[1].permissions();
        assert!(grunt.can_invite() && grunt.can_use_storage());
        assert!(!grunt.can_kick() && !grunt.can_edit_notice());
        assert!(!grunt.is_master_sentinel());

        // an unnamed bit must not fail decoding
        assert!(!GuildPermissions(0x8000_0000).can_invite());
    }

    /// A chunk is deliberately *not* a record: 0x3101 carries its payload raw
    /// so a partial buffer cannot fail the packet, and the pieces concatenate.
    #[test]
    fn chunks_pass_through_raw_and_concatenate_into_a_record() {
        let full = guild_record(&[("Solo", GuildPermissions::ALL)]);
        let (head, rest) = full.split_at(10);

        let a = GuildDataBody::try_from(Bytes::copy_from_slice(head)).unwrap();
        let b = GuildDataBody::try_from(Bytes::copy_from_slice(rest)).unwrap();

        // neither half is a record on its own
        assert!(GuildData::parse(a.data.clone()).is_err());

        let mut assembled = BytesMut::new();
        assembled.extend_from_slice(&a.data);
        assembled.extend_from_slice(&b.data);
        let decoded = GuildData::parse(assembled.freeze()).unwrap();
        assert_eq!(decoded.members[0].name, "Solo");
    }

    /// 0xB0F0 in both of its real shapes. The refusal is the guild family's
    /// standard ack —
    /// `u8 result` then a `u16` error — which the old `success: bool` model could
    /// not decode at all; and the success arm carries the record inline plus one
    /// trailing byte, which is why the record is parsed as a *prefix*.
    #[test]
    fn the_create_response_is_an_ack_with_the_record_inline() {
        // Refusal: a cold 0x70F0 with no NPC dialogue open.
        let refused = Bytes::from_static(&[0x02, 0x03, 0x00]);
        let decoded = GuildCreatedData::try_from(refused.clone()).unwrap();
        assert_eq!(decoded.result, 2);
        assert_eq!(decoded.error, Some(0x0003));
        assert!(decoded.data.is_none());
        assert_eq!(Bytes::from(decoded), refused);

        // Success: record inline, then exactly one byte left over.
        let mut body = vec![1u8];
        body.extend(guild_record(&[("Founder", GuildPermissions::MASTER)]));
        body.push(0x00);
        let wire = Bytes::from(body);
        let decoded = GuildCreatedData::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.result, 1);
        assert_eq!(decoded.data.as_ref().unwrap().name, "Wanderers");
        assert_eq!(decoded.error, None);
        assert_eq!(
            decoded.tail.len(),
            1,
            "the trailing byte is carried, not dropped"
        );
        assert_eq!(Bytes::from(decoded), wire);
    }

    /// 0x38F5 keeps its per-type payload raw — the original switches on the
    /// type with an empty body for every arm, so there is nothing to decode.
    #[test]
    fn a_guild_update_keeps_its_unknown_payload() {
        let wire = Bytes::from_static(&[5, 0xAA, 0xBB]);
        let decoded = GuildUpdate::try_from(wire.clone()).unwrap();

        assert_eq!(decoded.update_type, 5);
        assert_eq!(&decoded.tail[..], &[0xAA, 0xBB]);
        assert_eq!(Bytes::from(decoded), wire);

        assert!(GuildUpdate::try_from(Bytes::new()).is_err());
    }

    #[test]
    fn the_notice_edit_request_round_trips() {
        let mut body = ascii("Title");
        body.extend(ascii("Body text"));
        let wire = Bytes::from(body);

        let decoded = GuildNoticeEditRequest::try_from(wire.clone()).unwrap();

        assert_eq!(
            (decoded.title.as_str(), decoded.message.as_str()),
            ("Title", "Body text")
        );
        assert_eq!(Bytes::from(decoded), wire);
    }

    /// The 0xB0Fx cluster is one body form: 1 byte on success, `result` plus a
    /// `u16` code on refusal — and it round-trips in both arms.
    #[test]
    fn a_guild_op_ack_is_one_byte_on_success_and_three_on_refusal() {
        let ok = Bytes::from_static(&[0x01]);
        let decoded = GuildKickAck::try_from(ok.clone()).unwrap();
        assert!(decoded.is_success());
        assert_eq!(decoded.error_code, None);
        assert_eq!(Bytes::from(decoded), ok);

        let refused = Bytes::from_static(&[0x02, 0x33, 0x4C]);
        let decoded = GuildKickAck::try_from(refused.clone()).unwrap();
        assert!(!decoded.is_success());
        assert_eq!(decoded.error_code, Some(0x4C33));
        assert_eq!(Bytes::from(decoded), refused);
    }

    /// Every ack in the cluster reads the same two fields — the point of the
    /// shared form. Decoding the identical wire through each type must agree.
    #[test]
    fn every_ack_in_the_cluster_reads_the_same_two_fields() {
        let refused = Bytes::from_static(&[0x02, 0x0E, 0x1C]);

        macro_rules! same {
            ($($ty:ty),*) => {$({
                let d = <$ty>::try_from(refused.clone()).unwrap();
                assert_eq!((d.result, d.error_code), (0x02, Some(0x1C0E)));
                assert_eq!(Bytes::from(d), refused);
            })*};
        }

        same!(
            GuildDisbandAck,
            GuildLeaveAck,
            GuildInviteAck,
            GuildKickAck,
            GuildNoticeEditAck,
            GuildPromoteAck,
            GuildPermissionUpdateAck
        );
    }

    /// 0xB0F6 is the exception: its success arm carries the donated GP.
    #[test]
    fn the_donate_ack_carries_the_contributed_gp_only_on_success() {
        let mut body = vec![0x01u8];
        body.extend(500u32.to_le_bytes());
        let wire = Bytes::from(body);
        let decoded = GuildDonateAck::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.donated_gp, Some(500));
        assert_eq!(decoded.error_code, None);
        assert_eq!(Bytes::from(decoded), wire);

        let refused = Bytes::from_static(&[0x02, 0x0E, 0x1C]);
        let decoded = GuildDonateAck::try_from(refused.clone()).unwrap();
        assert_eq!(decoded.donated_gp, None);
        assert_eq!(decoded.error_code, Some(0x1C0E));
        assert_eq!(Bytes::from(decoded), refused);
    }

    /// 0x7104 is a counted list of pairs, not a byte. The single-pair case is
    /// the one the dialog actually produces; the empty one is wire-legal but
    /// never sent by the original, so it is asserted as a decode, not as
    /// something we may emit.
    #[test]
    fn the_permission_update_is_a_counted_list_of_id_mask_pairs() {
        let mut body = vec![0x02u8];
        body.extend(0x0100u32.to_le_bytes());
        body.extend(0x0000_00FFu32.to_le_bytes());
        body.extend(0x0101u32.to_le_bytes());
        body.extend(0xFFFF_FFFFu32.to_le_bytes());
        let wire = Bytes::from(body);

        let decoded = GuildPermissionUpdateRequest::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.count, 2);
        assert_eq!(
            decoded.changes,
            vec![
                GuildPermissionChange {
                    member_id: 0x0100,
                    permission_mask: 0xFF,
                },
                GuildPermissionChange {
                    member_id: 0x0101,
                    permission_mask: 0xFFFF_FFFF,
                },
            ]
        );
        assert_eq!(Bytes::from(decoded), wire);

        let empty = Bytes::from_static(&[0x00]);
        let decoded = GuildPermissionUpdateRequest::try_from(empty.clone()).unwrap();
        assert!(decoded.changes.is_empty());
        assert_eq!(Bytes::from(decoded), empty);
    }

    /// 0x30FF in both of its shapes. The guildless one is the point: the
    /// handler gates every read after the name on `name_len != 0`, so
    /// a body that stops after an empty name is well-formed, not truncated.
    #[test]
    fn the_entity_guild_update_tail_is_gated_on_a_non_empty_name() {
        let mut body = 0x018B50u32.to_le_bytes().to_vec();
        body.extend(7u32.to_le_bytes()); // guild id
        body.extend((3u16).to_le_bytes());
        body.extend(b"Sun");
        body.extend((4u16).to_le_bytes());
        body.extend(b"Star"); // granted nickname
        body.extend(11u32.to_le_bytes()); // guild crest rev
        body.extend(2u32.to_le_bytes()); // union id
        body.extend(3u32.to_le_bytes()); // union crest rev
        body.push(0x04); // fortress position: battle manager
        body.push(0x01); // relation flag
        let wire = Bytes::from(body);

        let decoded = EntityGuildUpdate::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.entity_id, 0x018B50);
        assert_eq!(decoded.guild_id, 7);
        assert_eq!(decoded.guild_name, "Sun");
        let tail = decoded.affiliation.clone().unwrap();
        assert_eq!(tail.granted_nick, "Star");
        assert_eq!(tail.crest_rev, 11);
        assert_eq!(tail.union_id, 2);
        assert_eq!(tail.union_crest_rev, 3);
        assert_eq!(tail.fortress_position, 0x04);
        assert_eq!(tail.relation_flag, 1);
        assert_eq!(Bytes::from(decoded), wire);

        // Guildless: 10 bytes + an empty name, and nothing after it.
        let mut body = 0x018B50u32.to_le_bytes().to_vec();
        body.extend(0u32.to_le_bytes());
        body.extend(0u16.to_le_bytes());
        let wire = Bytes::from(body);
        assert_eq!(wire.len(), 10);
        let decoded = EntityGuildUpdate::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.guild_name, "");
        assert_eq!(decoded.affiliation, None);
        assert_eq!(Bytes::from(decoded), wire);
    }

    /// 0x3100 is a bare entity id — four bytes, nothing else.
    #[test]
    fn the_guild_removal_push_is_a_bare_entity_id() {
        let wire = Bytes::from_static(&[0x2A, 0x00, 0x00, 0x00]);
        let decoded = EntityGuildRemove::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.entity_id, 0x2A);
        assert_eq!(Bytes::from(decoded), wire);
    }

    /// The two lifecycle requests are one `u32` each — the same window slot,
    /// so they must encode identically.
    #[test]
    fn the_lifecycle_requests_are_a_single_u32() {
        let wire = Bytes::from_static(&[0x2A, 0x00, 0x00, 0x00]);

        let disband = GuildDisbandRequest::try_from(wire.clone()).unwrap();
        assert_eq!(disband.unk_u32_00, 0x2A);
        assert_eq!(Bytes::from(disband), wire);

        let leave = GuildLeaveRequest::try_from(wire.clone()).unwrap();
        assert_eq!(leave.unk_u32_00, 0x2A);
        assert_eq!(Bytes::from(leave), wire);

        let promote = GuildPromoteRequest::try_from(wire.clone()).unwrap();
        assert_eq!(promote.unk_u32_00, 0x2A);
        assert_eq!(Bytes::from(promote), wire);
    }

    /// 0x70F0 leads with the NPC id and then the guild name.
    #[test]
    fn the_create_request_is_an_id_then_a_name() {
        let mut body = 0x1234u32.to_le_bytes().to_vec();
        body.extend(ascii("Wanderers"));
        let wire = Bytes::from(body);

        let decoded = GuildCreateRequest::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.npc_unique_id, 0x1234);
        assert_eq!(decoded.name, "Wanderers");
        assert_eq!(Bytes::from(decoded), wire);
    }

    /// The kick is addressed by name — no id anywhere in the body.
    #[test]
    fn the_kick_request_addresses_the_member_by_name() {
        let wire = Bytes::from(ascii("Grunt"));
        let decoded = GuildKickRequest::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.member_name, "Grunt");
        assert_eq!(Bytes::from(decoded), wire);
    }

    // Removed: `the_permission_update_request_is_a_single_byte` pinned the
    // reading "0x7104 is one byte, therefore a selector". Both ends of the wire
    // disagree — server loop and client builder: the byte is a count. Its
    // replacement is
    // `the_permission_update_is_a_counted_list_of_id_mask_pairs` above. A test
    // that fixes a wrong layout is worse than no test, because the next reader
    // takes it for the original's own layout.
}
