//! Post-join in-game packets (self-spawn foundation).
//!
//! After the `CharacterJoinRequest`/`CharacterJoinResponse` handshake
//! (`lobby.rs`), the agent server streams the character into the world:
//! `CharacterDataBegin` → `CharacterData` (the big blob) → `CharacterDataEnd`,
//! plus a `CelestialPosition` carrying the server time and the player's unique
//! id. Once the client has loaded, it answers with `GameReady`.
//!
//! Two shapes here don't fit the derive-based (de)serialization, so both
//! hand-write the `TryFrom<Bytes>` / `From<_> for Bytes` conversions the
//! `packets!` macro relies on:
//!   * `CharacterDataBody` carries its body **unparsed** (`Bytes`) — the vSRO
//!     layout has type-dependent inventory items whose sizes need itemdata,
//!     which lives in the client. The body's wire structs and staged parser
//!     live in `agent::character_data`; the client calls it with its itemdata
//!     lookup (`ItemClassResolver`).
//!   * the framing/ready packets have **empty bodies**.
//!   * the movement packets (`MovementRequest`/`MovementResponse`) have a
//!     coordinate width that depends on the region flag, which the derive can't
//!     express — hand-written below.
//!
//! Also here: the logout flow (`LogoutRequest`/`LogoutResponse`/cancel/success,
//! 0x7005/0xB005/0x7006/0xB006/0x300A) driving the Esc system window's
//! Quit/Restart buttons.

use bevy::prelude::Message;
use bytes::{BufMut, Bytes, BytesMut};

use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

/// 0x34A5 — marks the start of the character-data stream. Empty body.
#[derive(Message, Clone, Debug, Default)]
pub struct CharacterDataBegin;

/// 0x3013 — the character-data blob (between `CharacterDataBegin` and
/// `CharacterDataEnd`). Carried unparsed because decoding needs the client's
/// itemdata tables; see `agent::character_data::parse_character_info`. Named
/// `...Body` to avoid the collision with the lobby's `CharacterData` (the
/// char-list container).
#[derive(Message, Clone, Debug)]
pub struct CharacterDataBody {
    pub raw: Bytes,
}

/// 0x34A6 — marks the end of the character-data stream. Empty body. The client
/// treats this as "self-spawn complete" and answers with [`GameReady`].
#[derive(Message, Clone, Debug, Default)]
pub struct CharacterDataEnd;

/// 0x3020 — per-character celestial position sent on entering the world. The
/// `unique_id` is the **local player's** in-world id.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct CelestialPosition {
    pub unique_id: u32,
    /// `m_wDay`, the in-game **day counter** — not a moon phase. The original's
    /// handler feeds this `u16` straight into
    /// `m_LocalTime.InitTimer(pM->dwRealTime, pM->m_wDay, pM->m_byHour,
    /// `pM->m_byMin, 0)`. It is a counter, not a phase index.
    pub day: u16,
    pub hour: u8,
    pub minute: u8,
}

/// 0x3027 — periodic celestial (time-of-day) update. No unique id.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct CelestialUpdate {
    /// `m_wDay`, the in-game day counter — see [`CelestialPosition::day`]. The
    /// `0x3027` handler reads the same three fields without the leading
    /// `u32`.
    pub day: u16,
    pub hour: u8,
    pub minute: u8,
}

/// 0x34BE — the **real-world** server clock, pushed every ~10 minutes, packed
/// into one `u32`. Not to be confused with the in-game time-of-day clock
/// (`0x3020`/`0x3027`): this is the wall clock the original feeds into a C
/// `tm` and `mktime`.
///
/// The bit layout is read straight off the handler, which is a single 4-byte
/// read followed by:
///
/// ```text
/// tm_year = (v & 0x3F) + 100      // years since 1900 -> 2000 + (v & 0x3F)
/// tm_mon  = ((v >> 6) & 0x0F) - 1 // wire month is 1-based
/// tm_mday = (v >> 10) & 0x1F
/// tm_hour = (v >> 15) & 0x1F
/// tm_min  = (v >> 20) & 0x3F
/// tm_sec  = v >> 26               // top 6 bits
/// ```
///
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct ServerTime {
    pub packed: u32,
}

impl ServerTime {
    /// Full year (the original adds 100 to get `tm_year`, i.e. 1900 + 100 + n).
    pub fn year(&self) -> u16 {
        2000 + (self.packed & 0x3F) as u16
    }

    /// 1-based month, as on the wire (the original subtracts 1 for `tm_mon`).
    pub fn month(&self) -> u8 {
        ((self.packed >> 6) & 0x0F) as u8
    }

    pub fn day(&self) -> u8 {
        ((self.packed >> 10) & 0x1F) as u8
    }

    pub fn hour(&self) -> u8 {
        ((self.packed >> 15) & 0x1F) as u8
    }

    pub fn minute(&self) -> u8 {
        ((self.packed >> 20) & 0x3F) as u8
    }

    pub fn second(&self) -> u8 {
        (self.packed >> 26) as u8
    }
}

/// 0x3012 — client → server "loading finished / game ready". Empty body; the
/// server continues the spawn sequence once it arrives.
#[derive(Message, Clone, Debug, Default)]
pub struct GameReady;

/// 0x34B5 — server → client SERVER_AGENT_GAME_RESET: tear the world down and
/// reload (sent after a teleport commits). The body is the destination region
/// the client lands in. The server then goes COMPLETELY
/// silent (even HP ticks stop) until the client answers with
/// [`GameResetComplete`], after which it replays the CHARACTER_DATA stream
/// and waits for a second [`GameReady`].
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct GameReset {
    /// Destination region id.
    pub region: u16,
}

/// 0x34B6 — client → server CLIENT_AGENT_GAME_RESET_COMPLETE: the client
/// finished resetting and is ready for the post-teleport replay. Empty body.
#[derive(Message, Clone, Debug, Default)]
pub struct GameResetComplete;

// --- Hand-written wire conversions for the raw / empty bodies ---------------
//
// The `packets!` macro only needs `TryFrom<Bytes>` (decode) and
// `From<Self> for Bytes` (encode) per type; the derive macros generate exactly
// those. Empty and raw bodies provide them directly.

macro_rules! empty_packet {
    ($name:ident) => {
        impl TryFrom<Bytes> for $name {
            type Error = SerializationError;
            fn try_from(_: Bytes) -> Result<Self, Self::Error> {
                Ok($name)
            }
        }
        impl From<$name> for Bytes {
            fn from(_: $name) -> Self {
                Bytes::new()
            }
        }
    };
}

empty_packet!(CharacterDataBegin);
empty_packet!(CharacterDataEnd);
empty_packet!(GameReady);
empty_packet!(GameResetComplete);

impl TryFrom<Bytes> for CharacterDataBody {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        Ok(CharacterDataBody { raw: value })
    }
}

impl From<CharacterDataBody> for Bytes {
    fn from(value: CharacterDataBody) -> Self {
        value.raw
    }
}

// --- Movement (0x7021 request / 0xB021 response) ---------------------------
//
// Coordinates are raw region-local units; on the wire their width depends on the
// region: overworld regions (`region & 0x8000 == 0`) use 2-byte shorts, dungeon
// regions use 4-byte ints. The derive macro can't switch a field's width on a
// prior field, so these are hand-written.

/// Whether a region id denotes a dungeon (4-byte coords) vs the overworld
/// (2-byte coords). Bit 15 is the dungeon flag.
pub fn is_dungeon(region: u16) -> bool {
    region & 0x8000 != 0
}

fn short_packet() -> SerializationError {
    SerializationError::IoError(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "packet too short",
    ))
}

/// Bounds-checked little-endian reader — the `bytes::Buf` getters panic on
/// underflow, which we must not do on a malformed packet.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], SerializationError> {
        let end = self.pos.checked_add(n).ok_or_else(short_packet)?;
        let slice = self.buf.get(self.pos..end).ok_or_else(short_packet)?;
        self.pos = end;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8, SerializationError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, SerializationError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, SerializationError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, SerializationError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i16(&mut self) -> Result<i16, SerializationError> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, SerializationError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, SerializationError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, SerializationError> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    /// One coordinate component, widened to i32 (short for overworld regions).
    fn coord(&mut self, region: u16) -> Result<i32, SerializationError> {
        if is_dungeon(region) {
            self.i32()
        } else {
            Ok(self.i16()? as i32)
        }
    }
    /// An optional trailing byte (the movement source is not always present).
    fn opt_u8(&mut self) -> Option<u8> {
        let b = self.buf.get(self.pos).copied();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }
}

fn put_coords(buf: &mut BytesMut, region: u16, x: i32, y: i32, z: i32) {
    if is_dungeon(region) {
        buf.put_i32_le(x);
        buf.put_i32_le(y);
        buf.put_i32_le(z);
    } else {
        buf.put_i16_le(x as i16);
        buf.put_i16_le(y as i16);
        buf.put_i16_le(z as i16);
    }
}

/// 0x7021 — client → server move order to a location (click-to-move). `x/y/z`
/// are raw region-local units: scaling them by ten makes the server wrap the
/// destination several regions over.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct MovementRequest {
    pub region: u16,
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl From<MovementRequest> for Bytes {
    fn from(p: MovementRequest) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u8(1); // 1 = move to a location (vs 0 = turn to an angle)
        buf.put_u16_le(p.region);
        put_coords(&mut buf, p.region, p.x, p.y, p.z);
        buf.freeze()
    }
}

/// 0x7158 kind 1 — persist one quickslot of the under-bar.
///
/// `0x7158` is kind-discriminated: a leading `u8` picks between the quickslot
/// save (kind 1, built by the original's under-bar code) and the auto-potion
/// settings (kind 2). The kind-1 body is `u8 slot_index, u8 content_kind,
/// u32 value`.
///
/// Without this packet a quickslot assignment stays in client memory only, so
/// every relog hands the player a stale bar. `content_kind` is the one field
/// the original names but does not decode; the client sends what it knows the
/// slot holds — see `QuickSlotContent`.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct QuickSlotSaveRequest {
    pub slot_index: u8,
    pub content: QuickSlotContent,
    /// Skill or item ref id; `0` clears the slot.
    pub value: u32,
}

/// What a quickslot holds. The numeric values are the *client's* reading of
/// `content_kind` and are unconfirmed: they are what this client sends, and a
/// server's answer is what would confirm or correct them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickSlotContent {
    Empty = 0,
    Skill = 1,
    Item = 2,
}

impl From<QuickSlotSaveRequest> for Bytes {
    fn from(p: QuickSlotSaveRequest) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u8(1); // kind 1 = quickslot-bar save
        buf.put_u8(p.slot_index);
        buf.put_u8(p.content as u8);
        buf.put_u32_le(p.value);
        buf.freeze()
    }
}

impl TryFrom<Bytes> for QuickSlotSaveRequest {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut r = Reader::new(&value);
        let kind = r.u8()?;
        if kind != 1 {
            return Err(SerializationError::UnknownVariation(
                kind as usize,
                "0x7158 kind (1 = quickslot save)",
            ));
        }
        let slot_index = r.u8()?;
        let content = match r.u8()? {
            0 => QuickSlotContent::Empty,
            1 => QuickSlotContent::Skill,
            2 => QuickSlotContent::Item,
            other => {
                return Err(SerializationError::UnknownVariation(
                    other as usize,
                    "0x7158 content_kind",
                ));
            }
        };
        Ok(QuickSlotSaveRequest {
            slot_index,
            content,
            value: r.u32()?,
        })
    }
}

impl TryFrom<Bytes> for MovementRequest {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut r = Reader::new(&value);
        let _kind = r.u8()?; // 1 = location (the only form we build)
        let region = r.u16()?;
        Ok(MovementRequest {
            region,
            x: r.coord(region)?,
            y: r.coord(region)?,
            z: r.coord(region)?,
        })
    }
}

/// Where an entity *is* when a movement starts — the optional tail of 0xB021.
///
/// This is the only packet that states an entity's current position between
/// spawns: the destination says where it is going, and everything in between is
/// interpolation. A consumer that tracks positions (the headless bot walks by
/// them, and the GUI interpolates from them) needs the start point, so it is
/// surfaced rather than parsed and dropped.
#[derive(Serialize, Deserialize, ByteSize, Clone, Copy, Debug, PartialEq)]
pub struct MovementSource {
    pub region: u16,
    pub x: i32,
    /// Height is a plain `f32` here, not a region-scaled coordinate.
    pub y: f32,
    pub z: i32,
}

/// 0xB021 — server → client movement update. We surface `unique_id`, the
/// destination (or `angle` when it's a turn-in-place) and the optional source
/// position the update started from.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct MovementResponse {
    pub unique_id: u32,
    pub has_destination: bool,
    /// Valid when `has_destination`: region + raw region-local coords.
    pub region: u16,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// Valid when `!has_destination`: heading.
    pub angle: u16,
    /// Where the entity stood when this update was issued, when the server
    /// included it.
    pub source: Option<MovementSource>,
}

impl TryFrom<Bytes> for MovementResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut r = Reader::new(&value);
        let unique_id = r.u32()?;
        let has_destination = r.u8()? != 0;
        let mut out = MovementResponse {
            unique_id,
            has_destination,
            region: 0,
            x: 0,
            y: 0,
            z: 0,
            angle: 0,
            source: None,
        };
        if has_destination {
            out.region = r.u16()?;
            out.x = r.coord(out.region)?;
            out.y = r.coord(out.region)?;
            out.z = r.coord(out.region)?;
        } else {
            let _moving = r.u8()?;
            out.angle = r.u16()?;
        }
        // Optional source position: region, X (short/int), Y (always f32), Z.
        if matches!(r.opt_u8(), Some(1)) {
            let region = r.u16()?;
            let x = r.coord(region)?;
            let y = r.f32()?;
            let z = r.coord(region)?;
            out.source = Some(MovementSource { region, x, y, z });
        }
        Ok(out)
    }
}

impl From<MovementResponse> for Bytes {
    fn from(p: MovementResponse) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u32_le(p.unique_id);
        buf.put_u8(p.has_destination as u8);
        if p.has_destination {
            buf.put_u16_le(p.region);
            put_coords(&mut buf, p.region, p.x, p.y, p.z);
        } else {
            buf.put_u8(1); // moving
            buf.put_u16_le(p.angle);
        }
        match p.source {
            Some(src) => {
                buf.put_u8(1);
                buf.put_u16_le(src.region);
                // X and Z follow the region's coordinate rule; Y is always f32
                // here, which is why this cannot go through `put_coords`.
                if is_dungeon(src.region) {
                    buf.put_i32_le(src.x);
                    buf.put_f32_le(src.y);
                    buf.put_i32_le(src.z);
                } else {
                    buf.put_i16_le(src.x as i16);
                    buf.put_f32_le(src.y);
                    buf.put_i16_le(src.z as i16);
                }
            }
            None => buf.put_u8(0),
        }
        buf.freeze()
    }
}

/// 0xB023 — server → client absolute position sync for one entity: snap the
/// addressed entity to `region` + region-local float coords + heading. Unlike
/// [`MovementResponse`] (a move *order* whose coordinate width depends on the
/// region), this carries the entity's exact current position as floats — used for
/// teleports/knockback/corrections. Same position shape as the CHARACTER_DATA
/// block, so it needs no hand-written width handling.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct MovementPositionUpdate {
    pub unique_id: u32,
    pub region: u16,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub heading: u16,
}

/// 0xB024 — server → client: an entity turned **in place**. Without this an
/// entity that rotates without moving (an idle turn, or facing an NPC or a
/// target) keeps its old facing until its next move order.
///
/// 6-byte body, taken from the original's parser, which reads a `u32` then a
/// `u16` and stops. `angle` is the same `0..=u16::MAX → 0..2π` heading encoding
/// used everywhere else on the wire, as in [`MovementPositionUpdate`].
///
/// The layout rests on that parser alone and is unconfirmed on the wire.
///
/// The C→S half (0x7024 `CLIENT_CHARACTER_MOVEMENT_ANGLE`) is deliberately
/// **not** modelled: the original has no builder for it and its dispatch arm
/// is an empty stub, so any body would be invented.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct MovementAngleResponse {
    pub unique_id: u32,
    pub angle: u16,
}

/// 0x30D0 — server → client movement-speed change for one entity (buffs, GM
/// speed command, mounts): unique id + walk/run speeds in game units per
/// second.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct EntitySpeedUpdate {
    pub unique_id: u32,
    pub walk_speed: f32,
    pub run_speed: f32,
}

// --- Vitals / points / stats updates (0x3057 / 0x304E / 0x303D) -------------
//
// These three feed the player mini-info HUD: 0x3057 moves the HP/MP bars,
// 0x304E fills the hwan/berserk pips, and 0x303D is the only packet on the
// wire that carries max HP/MP (neither CHARACTER_DATA nor the bar update do).
// Layouts target vSRO 1.188.

/// [`EntityBarsUpdate::source`] display hints.
pub const BARS_SOURCE_DAMAGE: u16 = 0x01;
pub const BARS_SOURCE_REGEN: u16 = 0x10;
pub const BARS_SOURCE_LEVEL_UP: u16 = 0x80;

/// [`EntityBarsUpdate::flag`] bits — the body carries one block per set bit.
pub const BARS_FLAG_HP: u8 = 0x01;
pub const BARS_FLAG_MP: u8 = 0x02;
pub const BARS_FLAG_BAD_STATUS: u8 = 0x04;
/// A `u16` of UNKNOWN meaning; carried uninterpreted so the tail stays aligned.
pub const BARS_FLAG_UNKNOWN16: u8 = 0x08;

/// Which [`EntityBarsUpdate::bad_status`] bits carry a trailing `u8` level.
///
/// The original reads one level byte per set bit that also lies in this mask.
/// The level-less bits are exactly the six elemental/DoT states plus Petrify,
/// which independently corroborates the bit order in [`BadStatus`].
pub const BAD_STATUS_LEVELED: u32 = 0x017F_EFC0;

/// 0x3057 — server → client vitals update for one entity. Values are absolute,
/// not deltas.
///
/// **`flag` is a BITMASK, not an enum.** The original runs four independent
/// `if ((flag & bit) != 0)` blocks in this order: `0x01` HP u32, `0x02` MP u32,
/// `0x08` u16, `0x04` bad-status `u32` mask followed by
/// `popcount(mask & `[`BAD_STATUS_LEVELED`]`)` level bytes.
///
/// Body lengths follow the bitmask: 11 bytes for flags 1/2/4 (one block),
/// 15 for 3/5 (two blocks).
///
/// Decoded by hand rather than by the derive: the level tail's length is a
/// popcount of a value read earlier in the same body, which `#[sro_packet]`
/// has no way to express.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct EntityBarsUpdate {
    pub unique_id: u32,
    /// [`BARS_SOURCE_DAMAGE`] / [`BARS_SOURCE_REGEN`] / [`BARS_SOURCE_LEVEL_UP`].
    pub source: u16,
    pub flag: u8,
    pub hp: Option<u32>,
    pub mp: Option<u32>,
    /// The `0x08` block — meaning UNKNOWN, carried so the tail stays aligned.
    pub unknown16: Option<u16>,
    /// Abnormal-state bitmask ([`BadStatus`]), when `flag & 0x04`.
    pub bad_status: Option<u32>,
    /// One level per set [`BAD_STATUS_LEVELED`] bit, in bit order.
    pub bad_status_levels: Vec<u8>,
}

impl EntityBarsUpdate {
    /// The ailment mask, or 0 when this update carries no bad-status block.
    ///
    /// `None` means "this packet said nothing about ailments", which is NOT the
    /// same as "no ailments" — only a `flag & 0x04` body clears them.
    pub fn bad_status(&self) -> Option<BadStatus> {
        self.bad_status.map(BadStatus)
    }
}

impl TryFrom<Bytes> for EntityBarsUpdate {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let mut r = Reader::new(&value);
        let unique_id = r.u32()?;
        let source = r.u16()?;
        let flag = r.u8()?;
        // Order is the binary's, not the bit order: 0x08 is read BEFORE 0x04.
        let hp = (flag & BARS_FLAG_HP != 0).then(|| r.u32()).transpose()?;
        let mp = (flag & BARS_FLAG_MP != 0).then(|| r.u32()).transpose()?;
        let unknown16 = (flag & BARS_FLAG_UNKNOWN16 != 0)
            .then(|| r.u16())
            .transpose()?;
        let bad_status = (flag & BARS_FLAG_BAD_STATUS != 0)
            .then(|| r.u32())
            .transpose()?;
        let mut bad_status_levels = Vec::new();
        if let Some(mask) = bad_status {
            // A short/absent tail is tolerated rather than fatal: only the
            // no-level case of the level rule is confirmed, so a body that
            // ends early yields fewer levels instead of losing the mask that
            // names the ailments.
            for _ in 0..(mask & BAD_STATUS_LEVELED).count_ones() {
                match r.u8() {
                    Ok(level) => bad_status_levels.push(level),
                    Err(_) => break,
                }
            }
        }
        Ok(EntityBarsUpdate {
            unique_id,
            source,
            flag,
            hp,
            mp,
            unknown16,
            bad_status,
            bad_status_levels,
        })
    }
}

impl From<EntityBarsUpdate> for Bytes {
    fn from(p: EntityBarsUpdate) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u32_le(p.unique_id);
        buf.put_u16_le(p.source);
        buf.put_u8(p.flag);
        if let Some(hp) = p.hp {
            buf.put_u32_le(hp);
        }
        if let Some(mp) = p.mp {
            buf.put_u32_le(mp);
        }
        if let Some(unknown) = p.unknown16 {
            buf.put_u16_le(unknown);
        }
        if let Some(mask) = p.bad_status {
            buf.put_u32_le(mask);
        }
        for level in &p.bad_status_levels {
            buf.put_u8(*level);
        }
        buf.freeze()
    }
}

impl ByteSize for EntityBarsUpdate {
    fn byte_size(&self) -> usize {
        7 + self.hp.map_or(0, |_| 4)
            + self.mp.map_or(0, |_| 4)
            + self.unknown16.map_or(0, |_| 2)
            + self.bad_status.map_or(0, |_| 4)
            + self.bad_status_levels.len()
    }
}

/// The 0x3057 abnormal-state bitmask.
///
/// **The bit → ailment mapping is unconfirmed.** Only **Burn (`0x8`)** is
/// certain — see the [`EntityBarsUpdate`] doc. The ordering is independently
/// corroborated by [`BAD_STATUS_LEVELED`], whose level-less bits land exactly
/// on the six elemental/DoT states plus Petrify; that agreement is why the
/// order is trusted enough to act on, but no individual name below except Burn
/// should be called confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct BadStatus(pub u32);

/// The ailments [`BadStatus`] can name, in bit order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ailment {
    Freezing,
    Frostbite,
    ElectricShock,
    Burn,
    Poison,
    Zombie,
    Sleep,
    Bind,
    Dull,
    Fear,
    ShortSight,
    Bleed,
    Petrify,
    Darkness,
    Stun,
    Disease,
    Confusion,
    Decay,
    Weaken,
}

impl Ailment {
    /// Every ailment, in bit order — index `n` is bit `1 << n`.
    pub const ALL: [Ailment; 19] = [
        Ailment::Freezing,
        Ailment::Frostbite,
        Ailment::ElectricShock,
        Ailment::Burn,
        Ailment::Poison,
        Ailment::Zombie,
        Ailment::Sleep,
        Ailment::Bind,
        Ailment::Dull,
        Ailment::Fear,
        Ailment::ShortSight,
        Ailment::Bleed,
        Ailment::Petrify,
        Ailment::Darkness,
        Ailment::Stun,
        Ailment::Disease,
        Ailment::Confusion,
        Ailment::Decay,
        Ailment::Weaken,
    ];

    /// This ailment's mask bit.
    pub fn bit(self) -> u32 {
        1 << Ailment::ALL.iter().position(|a| *a == self).unwrap_or(0)
    }

    /// The archive's own name for this ailment's authored debuff effect, as
    /// `battle/status_bad_<name>.efp`.
    ///
    /// **Every name below is a real file** in `Particles.pk2` (26
    /// `battle/status_bad_*` entries). Six have a matching `status_cure_*`
    /// entry (`blind`, `burn`, `eshock`, `frostbite`, `poison`, `zombie`).
    ///
    /// The bit → file pairing is an inference over two independent naming
    /// schemes (the enum's English names vs. the artists' filenames) and
    /// is **unconfirmed for every bit except Burn**. Where the pairing
    /// is not obvious it returns `None` and that ailment plays no effect —
    /// borrowing a neighbouring file's art would be exactly the unsourced
    /// invention ADR-0009 forbids. Unpaired files: `control`, `hide`,
    /// `temptation`,
    /// `dark_blaze`, `dark_toxin`, the `_off` counterparts (`icing_off`,
    /// `stone_off` — almost certainly the *removal* animations) and the `_b`/
    /// `_m` icing size variants. A parallel `monster/status_bad_*` set exists
    /// for 8 of these; which set the original picks per entity kind is UNKNOWN,
    /// so only `battle/` is used.
    pub fn archive_name(self) -> Option<&'static str> {
        Some(match self {
            // "icing" is the archive's freeze family; frostbite is its own
            // file, matching the enum's two distinct cold states.
            Ailment::Freezing => "icing_on",
            Ailment::Frostbite => "frostbite",
            Ailment::ElectricShock => "eshock",
            Ailment::Burn => "burn",
            Ailment::Poison => "poison",
            Ailment::Zombie => "zombie",
            Ailment::Sleep => "sleep",
            Ailment::Bind => "root",
            Ailment::Dull => "blunt",
            Ailment::ShortSight => "myopia",
            Ailment::Bleed => "bleeding",
            Ailment::Petrify => "stone_on",
            Ailment::Darkness => "blind",
            Ailment::Stun => "stun",
            Ailment::Disease => "disease",
            Ailment::Confusion => "confusion",
            // No `status_bad_` file reads as fear, decay or weaken. The
            // unclaimed behavioural files (`panic`, `control`, `temptation`)
            // most likely belong to the ailments ABOVE Weaken that this enum
            // does not carry yet — note the icon set has a separate
            // `s_fear_icon` AND `s_panic_icon`, so pairing Fear to
            // `status_bad_panic.efp` would almost certainly be wrong.
            Ailment::Fear | Ailment::Decay | Ailment::Weaken => return None,
        })
    }

    /// The authored HUD icon for this ailment, under `icon/StateOdd/`.
    ///
    /// **Every name below is a real file** in `Media.pk2`, and this set is
    /// what raises confidence in the bit ORDER from "plausible" to "well
    /// corroborated": all nineteen bits pair to a distinct authored icon, and
    /// the last two land on `s_decay_icon` and `s_weakness_icon` at exactly
    /// the positions the enum puts Decay and Weaken. Three independent
    /// artifacts — the enum, the [`BAD_STATUS_LEVELED`] partition, and this
    /// icon set — agree.
    ///
    /// Names are given lowercase because `bevy_pk2` lowercases every path as
    /// it indexes the archive, so that is what a lookup must use — the
    /// archive's own mixed casing (`S_Burn_Icon.ddj` beside
    /// `s_bleeding_icon.ddj`) never reaches us.
    ///
    /// Unclaimed icons (`s_combustion_icon`, `s_dissociation_icon`,
    /// `s_incubation_icon`, `s_panic_icon`, `s_powerless_icon`) line up with
    /// the ailments above Weaken that this enum does not carry yet.
    pub fn icon_name(self) -> &'static str {
        match self {
            Ailment::Freezing => "s_freeze_icon",
            Ailment::Frostbite => "s_frostbite_icon",
            Ailment::ElectricShock => "s_electricshock_icon",
            Ailment::Burn => "s_burn_icon",
            Ailment::Poison => "s_poisoning_icon",
            Ailment::Zombie => "s_zombi_icon",
            Ailment::Sleep => "s_sleep_icon",
            Ailment::Bind => "s_root_icon",
            Ailment::Dull => "s_blunting_icon",
            Ailment::Fear => "s_fear_icon",
            Ailment::ShortSight => "s_myopia_icon",
            Ailment::Bleed => "s_bleeding_icon",
            Ailment::Petrify => "s_stonecurse_icon",
            Ailment::Darkness => "s_dark_icon",
            Ailment::Stun => "s_stun_icon",
            Ailment::Disease => "s_disease_icon",
            Ailment::Confusion => "s_confusion_icon",
            Ailment::Decay => "s_decay_icon",
            Ailment::Weaken => "s_weakness_icon",
        }
    }

    /// Asset path of [`Self::icon_name`].
    pub fn icon_path(self) -> String {
        format!("media://icon/stateodd/{}.ddj", self.icon_name())
    }
}

impl BadStatus {
    /// Whether any ailment is set.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether `ailment` is set.
    pub fn has(self, ailment: Ailment) -> bool {
        self.0 & ailment.bit() != 0
    }

    /// The ailments this mask names. Bits beyond [`Ailment::ALL`] are ignored
    /// rather than guessed at — the SPEC enum runs out before `u32` does.
    pub fn ailments(self) -> impl Iterator<Item = Ailment> {
        Ailment::ALL.into_iter().filter(move |a| self.has(*a))
    }
}

/// 0x304E — server → client points update for the local player (no unique id).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub enum CharacterPointsUpdate {
    #[sro_packet(value = 1)]
    Gold { amount: u64, display: u8 },
    #[sro_packet(value = 2)]
    Sp { amount: u32, display: u8 },
    #[sro_packet(value = 3)]
    StatPoints { amount: u16 },
    /// `amount` is the hwan/berserk gauge fill (0–5).
    #[sro_packet(value = 4)]
    Berserk { amount: u8, source: u32 },
}

/// 0x303D — server → client recomputed combat stats for the local player (no
/// unique id), sent after the self-spawn stream and on any stat change. The
/// only source of max HP/MP.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct CharacterStatsUpdate {
    pub phys_attack_min: u32,
    pub phys_attack_max: u32,
    pub mag_attack_min: u32,
    pub mag_attack_max: u32,
    pub phys_defense: u16,
    pub mag_defense: u16,
    pub hit_rate: u16,
    pub parry_rate: u16,
    pub max_hp: u32,
    pub max_mp: u32,
    pub strength: u16,
    pub intelligence: u16,
}

// --- Misc world-join server pushes (EXPERIMENTAL) ---------------------------
//
// Four small S→C pushes a vSRO 1.188 server sends at world join.
// Only the empty-list branch of the two roster/cooldown packets is known, so
// their entry shapes are UNVERIFIED and kept as best-effort targets — the
// empty case round-trips exactly.

/// 0x3153 — SERVER_AGENT_SILK_UPDATE: account silk balances (Joymax premium
/// currency). Three u32 balances, little-endian. Confirmed: the 12-byte body
/// `F4 CB 9A 3B 50 C3 00 00 00 00 00 00` decodes to own 1,000,000,500 / gift
/// 50,000 / point 0. The own/gift/point label order is a convention; the wire
/// does not label them.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct SilkUpdate {
    /// Regular (purchased) silk.
    pub own: u32,
    /// Gift / premium silk.
    pub gift: u32,
    /// Silk points.
    pub point: u32,
}

/// 0x3809 — SERVER_AGENT_ENVIRONMENT_WEATHER_UPDATE: the zone weather sent on
/// world-enter. The body is exactly 2 bytes (`01 B4` → type 1 = clear,
/// intensity 180 in the Jangan start zone). The `weather_type` enum and the
/// `intensity` scale are unconfirmed; relevant to EP-27 (environment /
/// weather).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct WeatherUpdate {
    /// 1 = clear/fine (the only value known).
    pub weather_type: u8,
    /// Severity / particle amount (only 180 is known).
    pub intensity: u8,
}

/// 0x3305 — SERVER_AGENT_COMMUNITY_FRIEND_INFO: the join-time friend roster.
/// Only the empty roster (a single `00` count byte) has ever been seen, so only
/// the count-prefixed shell is confirmed. The per-entry [`FriendEntry`] record
/// is read off the original's own parser and is unconfirmed; the empty
/// case round-trips exactly.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct FriendListInfo {
    /// Number of roster entries that follow.
    pub count: u8,
    #[sro_packet(list_type = "by-size-field", size_field = "count")]
    pub friends: Vec<FriendEntry>,
}

/// One friend roster entry: `u32, u16 len + ASCII, u32, u8` — **four** fields.
///
/// Read straight off the original's parser: it reads the id, the name length,
/// the name bytes, then a `u32` and a `u8`, and hands exactly those four to the
/// roster constructor. The roster node it builds has slots for precisely those
/// four, and no fifth.
///
/// UNKNOWN: the *names* of the two trailing scalars. A model/ref-object id and
/// an online flag are the obvious reading, but that is inference, so the layout
/// is closed while the semantics are not.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct FriendEntry {
    pub char_id: u32,
    pub name: String,
    /// Trailing `u32` — very likely the character's ref-object id. Unknown.
    pub char_model: u32,
    /// Trailing `u8` — nonzero = online, per the roster node's status slot.
    /// Unknown.
    pub is_online: u8,
}

/// 0x3077 — CharacterFinished: the join-time cooldown replay. Two
/// count-prefixed lists — item cooldowns then skill cooldowns, keyed by ref-id.
/// Only the both-lists-empty body (`00 00`) has ever been seen, so only the
/// two-empty-list shell is confirmed; the per-entry [`Cooldown`] record is
/// UNVERIFIED. Relevant to EP-07 (item use / cooldowns).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct CharacterFinished {
    pub item_cooldown_count: u8,
    #[sro_packet(list_type = "by-size-field", size_field = "item_cooldown_count")]
    pub item_cooldowns: Vec<Cooldown>,
    pub skill_cooldown_count: u8,
    #[sro_packet(list_type = "by-size-field", size_field = "skill_cooldown_count")]
    pub skill_cooldowns: Vec<Cooldown>,
}

/// One cooldown entry — UNVERIFIED `{ ref_id, cooldown }`; a non-empty body
/// has never been seen.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct Cooldown {
    pub ref_id: u32,
    pub cooldown: u32,
}

// --- Entity selection (0x7045/0xB045) ---------------------------------------

/// 0x7045 — client → server "select this entity" (the click target of the
/// world-scene selection). The server answers with [`SelectEntityResponse`].
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct SelectEntityRequest {
    pub unique_id: u32,
}

/// 0xB045 — server → client answer to [`SelectEntityRequest`].
///
/// After the `result`/`unique_id` header the body depends on the *target's*
/// type, which is not encoded in the packet — so the tail stays raw and the
/// consumer interprets it against what it knows the entity to be. The shapes
/// are unconfirmed against vSRO:
///   * monster: `u8` (=1), `u32` current HP, `u8`, `u8`
///   * player:  `u32`, `u8` trader lvl, `u8` hunter lvl, `u8` thief lvl, `u8`
///   * NPC:     the reference server never sends the response at all, so the
///     client must treat this packet as optional enrichment, never a gate.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct SelectEntityResponse {
    /// 1 = ok, anything else = failure (tail then carries an error byte).
    pub result: u8,
    /// Echo of the selected unique id (0 on failure).
    pub unique_id: u32,
    /// Raw type-dependent remainder, see above.
    pub tail: Bytes,
}

impl SelectEntityResponse {
    /// Interpret the tail as the monster shape and return the current HP, if
    /// the shape matches and the server filled it in (0 means unknown).
    pub fn monster_hp(&self) -> Option<u32> {
        if self.result != 1 || self.tail.len() < 5 || self.tail[0] != 1 {
            return None;
        }
        let hp = u32::from_le_bytes(self.tail[1..5].try_into().unwrap());
        (hp > 0).then_some(hp)
    }
}

impl TryFrom<Bytes> for SelectEntityResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut r = Reader::new(&value);
        let result = r.u8()?;
        let unique_id = if result == 1 { r.u32()? } else { 0 };
        Ok(SelectEntityResponse {
            result,
            unique_id,
            tail: value.slice(r.pos..),
        })
    }
}

impl From<SelectEntityResponse> for Bytes {
    fn from(p: SelectEntityResponse) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u8(p.result);
        if p.result == 1 {
            buf.put_u32_le(p.unique_id);
        }
        buf.extend_from_slice(&p.tail);
        buf.freeze()
    }
}

// --- Object action (0x7074/0xB074, 0xB070/0xB071) ---------------------------
//
// The attack / skill-cast exchange. The request starts a *server-driven*
// action loop (the server paths the character into range and repeats basic
// attacks until a Cancel or the target dies); each swing/cast arrives as a
// 0xB070 [`ObjectActionUpdate`] carrying the per-target damage list — the
// 0xB074 ack itself is only accept/reject. On this wire a 0xB070 body is
// 20 bytes with no extra u32 before `target`, and the 0xB074 ack is the
// 2-byte `phase code` shape documented on [`ObjectActionResponse`]. Both
// still decode tolerantly into an `Unknown { result, tail }` fallback for
// unrecognized shapes — consumers must treat those as log-only.

/// 0x7074 — client → server object action: a leading flag byte selects
/// execute (1, followed by an [`ActionCommand`]) or cancel (2, empty).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub enum ObjectActionRequest {
    #[sro_packet(value = 1)]
    Execute(ActionCommand),
    #[sro_packet(value = 2)]
    Cancel,
}

/// The action selector inside [`ObjectActionRequest::Execute`].
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub enum ActionCommand {
    #[sro_packet(value = 1)]
    Attack(ActionTarget),
    #[sro_packet(value = 2)]
    Pickup(ActionTarget),
    /// Follow the target ("auto trace"). The action window's `CommandID` 1003
    /// arm opens 0x7074 and writes `01 03 01 <u32 uid>` — the execute flag,
    /// this discriminant, the entity-target flag and the current target's
    /// unique id. It is the only 0x7074 arm in that dispatcher.
    #[sro_packet(value = 3)]
    Trace(ActionTarget),
    #[sro_packet(value = 4)]
    CastSkill {
        ref_skill_id: u32,
        target: ActionTarget,
    },
}

/// Target selector: a flag byte, then the unique id when targeting an entity.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub enum ActionTarget {
    #[sro_packet(value = 0)]
    None,
    #[sro_packet(value = 1)]
    Entity { unique_id: u32 },
}

// Action error codes seen in [`ObjectActionResponse`] / [`ObjectActionUpdate`]
// failures — the original's **family-4 notice ids**, not small ordinals.
//
// The failure field is a `u16` that the original hands to its notice
// dispatcher as `(family = 4, code)`, and that dispatcher's code→text-id table
// covers exactly `0x3003 ..= 0x3048`, so every legal code is `0x30xx`. The two
// constants that used to stand here — skrillax's `PerformActionError` values
// `0x06`/`0x07` — were these same two errors with the family nibble sheared
// off and could therefore never match a live code.
//
// The text ids are the original's own `UIIT_SKILL_USE_FAIL_*` keys
// (textuisystem.txt L1598-1610, L1722-1726). Codes such as `0x3006`, `0x300F`
// and `0x3010` occur in normal play.
//
// Only the codes whose meaning is established are named here; anything else is
// passed through as a number by [`action_error_text_key`] rather than guessed
// at.
/// skill-id lookup failed (`UIIT_SKILL_USE_FAIL_NOTLEARN`).
pub const ACTION_ERROR_NOT_LEARNED: u16 = 0x3003;
/// MP cost above current MP (`..._NOTENOUGHMP`).
pub const ACTION_ERROR_NOT_ENOUGH_MP: u16 = 0x3004;
/// reuse delay still running (`..._TIMEDELAY_COOL_TIME`).
pub const ACTION_ERROR_COOLDOWN: u16 = 0x3005;
/// target legality / line-of-sight test (`..._WRONGTARGET`).
pub const ACTION_ERROR_INVALID_TARGET: u16 = 0x3006;
/// The original's range check — out of range (`..._WRONGDISTANCE`).
pub const ACTION_ERROR_INVALID_DISTANCE: u16 = 0x3007;
/// character level below the skill's requirement
/// (`..._NOTENOUGHLEVEL`).
pub const ACTION_ERROR_NOT_ENOUGH_LEVEL: u16 = 0x3008;
/// buff overlap (`..._OVERLAP`).
pub const ACTION_ERROR_OVERLAP: u16 = 0x300C;
/// equipped weapon class ≠ required (`..._WRONGWEAPON`).
pub const ACTION_ERROR_WRONG_WEAPON: u16 = 0x300D;
/// ammo missing or of the wrong kind (`..._RUNOUT_AMMO`).
pub const ACTION_ERROR_NO_AMMO: u16 = 0x300E;
/// weapon durability 0 / broken (`..._BROKEN_WEAPON`).
///
/// This is the code (12303) a 0x7074 attack comes back with against a living,
/// selectable target when the attacker carries no usable weapon. It is the
/// broken- or missing-weapon gate, which fits an unequipped clientless bot and
/// does *not* mean "the target expired".
pub const ACTION_ERROR_BROKEN_WEAPON: u16 = 0x300F;
/// navmesh ray caster→target fails (`..._PATH_INTERRUPTED`).
pub const ACTION_ERROR_PATH_INTERRUPTED: u16 = 0x3010;
/// The original's resurrection check — resurrection skill level below the target's level
/// (`..._RESURRECT`).
pub const ACTION_ERROR_RESURRECT: u16 = 0x3012;
/// HP cost above current HP (`..._NOTENOUGHHP`).
pub const ACTION_ERROR_NOT_ENOUGH_HP: u16 = 0x3013;

/// The original's `UIIT_SKILL_USE_FAIL_*` textuisystem key for a family-4
/// action error, or `None` for a code whose meaning is not established — the
/// caller then shows the raw number instead of inventing a meaning.
pub fn action_error_text_key(code: u16) -> Option<&'static str> {
    Some(match code {
        ACTION_ERROR_NOT_LEARNED => "UIIT_SKILL_USE_FAIL_NOTLEARN",
        ACTION_ERROR_NOT_ENOUGH_MP => "UIIT_SKILL_USE_FAIL_NOTENOUGHMP",
        ACTION_ERROR_COOLDOWN => "UIIT_SKILL_USE_FAIL_TIMEDELAY_COOL_TIME",
        ACTION_ERROR_INVALID_TARGET => "UIIT_SKILL_USE_FAIL_WRONGTARGET",
        ACTION_ERROR_INVALID_DISTANCE => "UIIT_SKILL_USE_FAIL_WRONGDISTANCE",
        ACTION_ERROR_NOT_ENOUGH_LEVEL => "UIIT_SKILL_USE_FAIL_NOTENOUGHLEVEL",
        ACTION_ERROR_OVERLAP => "UIIT_SKILL_USE_FAIL_OVERLAP",
        ACTION_ERROR_WRONG_WEAPON => "UIIT_SKILL_USE_FAIL_WRONGWEAPON",
        ACTION_ERROR_NO_AMMO => "UIIT_SKILL_USE_FAIL_RUNOUT_AMMO",
        ACTION_ERROR_BROKEN_WEAPON => "UIIT_SKILL_USE_FAIL_BROKEN_WEAPON",
        ACTION_ERROR_PATH_INTERRUPTED => "UIIT_SKILL_USE_FAIL_PATH_INTERRUPTED",
        ACTION_ERROR_RESURRECT => "UIIT_SKILL_USE_FAIL_RESURRECT",
        ACTION_ERROR_NOT_ENOUGH_HP => "UIIT_SKILL_USE_FAIL_NOTENOUGHHP",
        _ => return None,
    })
}

/// `wResult` of a successful [`ObjectActionUpdate`]: a skill/self action.
/// Only `0x3000` and `0x3002` are legal — anything else trips the original's
/// own assert. Both occur in normal play: `0x3000` on a join-time self-buff,
/// `0x3002` on a physical hit.
pub const ACTION_RESULT_SKILL: u16 = 0x3000;
/// `wResult` of a successful [`ObjectActionUpdate`]: a physical weapon hit.
pub const ACTION_RESULT_ATTACK: u16 = 0x3002;

// --- GM commands (0x7010 / 0xB010) -----------------------------------------

/// 0x7010 — client → server GM command: a **u16 LE sub-command selector**,
/// then per-command **typed** args. Only the account-privileged commands the
/// server honours do anything; a normal account gets a failure [`GmResponse`].
///
/// The original binds each command *name* to its own builder in a registry, and
/// each builder writes its fields individually. There is no generic
/// "`u16` + ASCII message" envelope: all 34 builders write
/// `{u16 sub_id, …typed args}`.
///
/// The two toggles carry no args; the server flips the state and echoes it back
/// as a 0x30BF body-state update rather than in the ack.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum GmCommand {
    /// `/loadmonster` (0x06) — spawn `count` of `ref_id` at the caller.
    /// 8-byte body `{u16, u32, u8, u8}`.
    LoadMonster { ref_id: u32, count: u8, rarity: u8 },
    /// `/makeitem` (0x07) — create an item in the GM's inventory. 7-byte body
    /// `{u16, u32, u8}`.
    ///
    /// `value` stays a neutral name because the one byte means two different
    /// things by item class. The original parses argument 2 with
    /// `swscanf("%d")`, truncates it to its low byte and sends it for both —
    /// clamped to `[1, MaxStack]` when the row is stackable (`TypeID2 == 3`),
    /// passed through untouched for equipment.
    ///
    /// The server reads it two ways: for **equipment it is the enchantment
    /// level**, for a stackable it is the quantity — which is what the
    /// client's own two-branch
    /// clamp already implied. It is a *request*, not a guarantee: the server
    /// clamps to the item's own ceiling.
    ///
    /// `ref_id` is resolved client-side: the original looks argument 1 up in
    /// the item ref-object table by codename and puts the resulting u32 on the
    /// wire, so a codename never reaches the server.
    MakeItem { ref_id: u32, value: u8 },
    /// `/zoe` **and** `/zoe2` (0x0C) — spawn `count` of a monster. 7-byte body
    /// `{u16,u32,u8}`.
    ///
    /// **Both commands emit this one sub-id.** `Zoe2` is not a distinct packet:
    /// it is a client-side batching wrapper that splits a large count into
    /// chunks of 200 and paces them, and every chunk is an ordinary `0x000C`.
    /// So the 34 builders cover only 33 distinct sub-ids.
    ///
    /// The monster is named by codename and resolved client-side against the
    /// same unified ref-object map `MakeItem` uses; the gate is `TypeID 1/2/1`
    /// (character / NPC / monster) where `MakeItem`'s is `TypeID1 == 3`.
    ///
    /// Whether the server then *kills* what it spawned is unknown — the client
    /// only builds this body.
    Zoe { ref_id: u32, count: u8 },
    /// `/invisible` (0x0E) — toggle GM invisibility. 2-byte body.
    Invisible,
    /// `/invincible` (0x0F) — toggle GM invincibility. 2-byte body.
    Invincible,
    /// Any other sub-command, kept verbatim so an unmodelled code survives a
    /// decode instead of failing the packet. This arm is needed: the body
    /// `0b 00 <u8> <u16>` occurs and is refused with `02 05 00`, and a hard
    /// error there would lose the frame that documents it.
    Other { code: u16, args: Bytes },
}

impl GmCommand {
    /// The u16 sub-command selector, as the original's own command registry
    /// defines it.
    pub fn code(&self) -> u16 {
        match self {
            GmCommand::LoadMonster { .. } => 0x06,
            GmCommand::MakeItem { .. } => 0x07,
            GmCommand::Zoe { .. } => 0x0C,
            GmCommand::Invisible => 0x0E,
            GmCommand::Invincible => 0x0F,
            GmCommand::Other { code, .. } => *code,
        }
    }
}

impl TryFrom<Bytes> for GmCommand {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let mut r = Reader::new(&value);
        let code = r.u16()?;
        match code {
            0x06 => Ok(GmCommand::LoadMonster {
                ref_id: r.u32()?,
                count: r.u8()?,
                rarity: r.u8()?,
            }),
            0x07 => Ok(GmCommand::MakeItem {
                ref_id: r.u32()?,
                value: r.u8()?,
            }),
            0x0C => Ok(GmCommand::Zoe {
                ref_id: r.u32()?,
                count: r.u8()?,
            }),
            0x0E => Ok(GmCommand::Invisible),
            0x0F => Ok(GmCommand::Invincible),
            _ => Ok(GmCommand::Other {
                code,
                args: value.slice(2..),
            }),
        }
    }
}

impl From<GmCommand> for Bytes {
    fn from(p: GmCommand) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u16_le(p.code());
        match p {
            GmCommand::LoadMonster {
                ref_id,
                count,
                rarity,
            } => {
                buf.put_u32_le(ref_id);
                buf.put_u8(count);
                buf.put_u8(rarity);
            }
            GmCommand::MakeItem { ref_id, value } => {
                buf.put_u32_le(ref_id);
                buf.put_u8(value);
            }
            GmCommand::Zoe { ref_id, count } => {
                buf.put_u32_le(ref_id);
                buf.put_u8(count);
            }
            GmCommand::Other { args, .. } => buf.put_slice(&args),
            GmCommand::Invisible | GmCommand::Invincible => {}
        }
        buf.freeze()
    }
}

/// 0xB010 — server → client GM command result.
///
/// `result` is 1 = ok / 2 = fail, and the `u16` after it is an **echo of the
/// request's sub-command**, read on *both* arms — not an error code. The proof
/// is that command 0x20 appears in both and yields "SiegeManager MSG Result -
/// Ok." / "…- Fail." from the same value. That echo is what makes this packet
/// a usable probe: it says which [`GmCommand`] the server just judged.
///
/// The per-command payload after the echo is still kept raw. Only a handful of
/// ids carry one (0x01 and 0x19/0x1a a string, 0x04 three u32s), and neither
/// [`GmCommand::MakeItem`] nor [`GmCommand::LoadMonster`] is among them — for
/// those the whole body is the 3-byte head.
///
/// On the wire a body reads e.g. `01 0e 00` — ok, echoing the `/invisible`
/// that was sent.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct GmResponse {
    pub result: u8,
    /// Echo of the [`GmCommand::code`] this answers.
    pub gm_command_id: u16,
    pub tail: Bytes,
}

impl GmResponse {
    pub fn is_success(&self) -> bool {
        self.result == GM_RESULT_OK
    }
}

/// `result` values the original branches on; anything else reads nothing.
pub const GM_RESULT_OK: u8 = 1;
pub const GM_RESULT_FAIL: u8 = 2;

impl TryFrom<Bytes> for GmResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let mut r = Reader::new(&value);
        let result = r.u8()?;
        // The original reads the echo on the ok and fail arms alike, and reads
        // nothing at all for any other `result` — so a body that stops here is
        // legal rather than malformed.
        let gm_command_id = r.u16().unwrap_or(0);
        Ok(GmResponse {
            result,
            gm_command_id,
            tail: value.slice(r.pos.min(value.len())..),
        })
    }
}

impl From<GmResponse> for Bytes {
    fn from(p: GmResponse) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u8(p.result);
        buf.put_u16_le(p.gm_command_id);
        buf.extend_from_slice(&p.tail);
        buf.freeze()
    }
}

/// 0xB074 — server → client ack for [`ObjectActionRequest`].
///
/// The shape is two bytes, `phase code`:
/// - `01 <code>` — action start ack: code 0 = skill cast accepted, code 1 =
///   attack accepted, code 2 = **rejected** (server action slot busy — a cast
///   sent mid-auto-attack; no 0xB070 follows).
/// - `02 <code>` — the running action ended (code 0 or 1; the meaning of the
///   code is not pinned).
/// - `03 <code> <error u16>` — the request was **refused with a message**: the
///   handler reads the same `code` byte, then a `u16` it hands to the
///   message-box helper, e.g. `03 00 04 40` = code 0, error `0x4004`.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum ObjectActionResponse {
    /// `01 <code>` — start ack; see [`Self::is_rejected`].
    Started { code: u8 },
    /// `02 <code>` — the server-side action loop ended.
    Ended { code: u8 },
    /// `03 <code> <error u16>` — refused with a message box; `error` is the
    /// original's notice id, not a small ordinal (see `ACTION_ERROR_*`).
    Failed { code: u8, error: u16 },
    /// Anything else — kept raw, log-only.
    Unknown { result: u8, tail: Bytes },
}

/// [`ObjectActionResponse::Started`] code: skill cast accepted.
pub const ACTION_START_CAST: u8 = 0;
/// [`ObjectActionResponse::Started`] code: attack accepted.
pub const ACTION_START_ATTACK: u8 = 1;
/// [`ObjectActionResponse::Started`] code: rejected — another action owns the
/// server's action slot.
pub const ACTION_START_REJECTED: u8 = 2;

impl ObjectActionResponse {
    /// A start ack that refused the request (nothing will follow on 0xB070).
    pub fn is_rejected(&self) -> bool {
        matches!(
            self,
            ObjectActionResponse::Started {
                code: ACTION_START_REJECTED
            }
        )
    }
}

impl TryFrom<Bytes> for ObjectActionResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let mut r = Reader::new(&value);
        let result = r.u8()?;
        let parsed = (|| -> Option<ObjectActionResponse> {
            match result {
                1 => Some(ObjectActionResponse::Started { code: r.u8().ok()? }),
                2 => Some(ObjectActionResponse::Ended { code: r.u8().ok()? }),
                3 => Some(ObjectActionResponse::Failed {
                    code: r.u8().ok()?,
                    error: r.u16().ok()?,
                }),
                _ => None,
            }
        })()
        // A typed read that leaves bytes over means we guessed the wrong
        // shape — fall back to raw rather than silently dropping data.
        .filter(|_| r.pos == value.len());
        Ok(parsed.unwrap_or_else(|| ObjectActionResponse::Unknown {
            result,
            tail: value.slice(1..),
        }))
    }
}

impl From<ObjectActionResponse> for Bytes {
    fn from(p: ObjectActionResponse) -> Self {
        let mut buf = BytesMut::new();
        match p {
            ObjectActionResponse::Started { code } => {
                buf.put_u8(1);
                buf.put_u8(code);
            }
            ObjectActionResponse::Ended { code } => {
                buf.put_u8(2);
                buf.put_u8(code);
            }
            ObjectActionResponse::Failed { code, error } => {
                buf.put_u8(3);
                buf.put_u8(code);
                buf.put_u16_le(error);
            }
            ObjectActionResponse::Unknown { result, tail } => {
                buf.put_u8(result);
                buf.extend_from_slice(&tail);
            }
        }
        buf.freeze()
    }
}

/// One damage value inside a [`SkillPartDamage`] hit.
///
/// The `kind` byte is kept **raw**. The server resolves an outcome enum
/// internally (1 hit / 2 miss / 4 crit / 5 block / 6 parry / 8 defense,
/// `docs/combat-math-server-spec.md` §2) whose numbering is NOT the numbering
/// of this wire byte — on the wire, `2` is the critical marker. Only `1` and
/// `2` have ever been seen, so any other value is an unknown outcome:
/// collapsing it to a bool would rewrite it as an ordinary hit on the way back
/// out.
///
/// Both fields come out of a single packed little-endian `u32`: the low byte is
/// this `kind` (the original's "damage state"), the upper **24 bits** are the
/// amount. Reading the byte and then a full `u32` agrees only while the
/// following unnamed field is zero, which it always is so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageValue {
    /// Wire `kind` byte: 1 = standard, 2 = critical, other = UNKNOWN outcome.
    pub kind: u8,
    /// 24-bit on the wire; values above `0xFF_FFFF` cannot be encoded.
    pub amount: u32,
}

impl DamageValue {
    /// Wire kind for an ordinary hit.
    pub const KIND_NORMAL: u8 = 1;
    /// Wire kind for a critical hit — the only non-1 value seen so far.
    pub const KIND_CRITICAL: u8 = 2;

    pub fn is_critical(&self) -> bool {
        self.kind == Self::KIND_CRITICAL
    }

    /// True for a `kind` that is neither of the two known ones — an outcome
    /// the presentation layer must treat as log-only rather than as a plain
    /// hit.
    pub fn is_unknown_kind(&self) -> bool {
        !matches!(self.kind, Self::KIND_NORMAL | Self::KIND_CRITICAL)
    }
}

/// The 14-byte position tail carried by hit arms 4 and 5: a region id plus
/// three `i32` coordinates the original converts to floats at read time. Kept
/// as the wire's integers, because the scaling is unconfirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitPosition {
    pub region: u16,
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// The payload of one hit record, selected by `flags & 0x7F`.
///
/// The original is a `switch (flags & 0x7f)` over four listed arms plus a
/// catch-all, not a set of bit tests: arms 0 and 7 are 9 bytes, arms 4 and 5
/// are 23, and every other value reads nothing at all. A bit test on `0x08`
/// agrees with the catch-all only by luck and reads arms 4/5 fourteen bytes
/// short, desynchronising the rest of the packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitEffect {
    /// Arm 0 — plain damage: the packed damage word plus one unnamed `u32`
    /// (always zero so far, kept so a non-zero value survives a round trip).
    Damage { value: DamageValue, unknown: u32 },
    /// Arms 4 and 5 — arm 0 plus a position tail (knockback / knockdown).
    /// The original stores the two arms in *different* record slots
    /// (`rec+0x1c` vs `rec+0x2c`), so which arm it was is preserved; what
    /// distinguishes them is UNKNOWN.
    Displaced {
        arm: u8,
        value: DamageValue,
        unknown: u32,
        pos: HitPosition,
    },
    /// Arm 7 — the packed damage word plus two unnamed `u16`s. The original
    /// force-clears its killing-blow flag on this arm.
    Arm7 {
        value: DamageValue,
        unknown_a: u16,
        unknown_b: u16,
    },
    /// Any other arm: the record is the flag byte alone, no damage is read
    /// and the original zeroes its damage field (`:32-34`).
    NoPayload { arm: u8 },
}

/// One hit against one entity: the `0x80` bit of the flag byte plus the arm
/// payload it selects. The original keeps exactly this split — `0x80` goes to
/// a separate bool while `flags & 0x7F` drives the switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkillPartDamage {
    /// Wire bit `0x80`. Kept verbatim even on arm 7, where the original
    /// discards it after reading, so the byte round-trips.
    pub killing_blow: bool,
    pub effect: HitEffect,
}

impl SkillPartDamage {
    /// An ordinary arm-0 hit — the shape practically every record takes.
    pub fn hit(value: DamageValue) -> Self {
        SkillPartDamage {
            killing_blow: false,
            effect: HitEffect::Damage { value, unknown: 0 },
        }
    }

    /// An arm-0 hit that killed the target.
    pub fn killing_blow(value: DamageValue) -> Self {
        SkillPartDamage {
            killing_blow: true,
            effect: HitEffect::Damage { value, unknown: 0 },
        }
    }

    pub fn value(&self) -> Option<DamageValue> {
        match self.effect {
            HitEffect::Damage { value, .. }
            | HitEffect::Displaced { value, .. }
            | HitEffect::Arm7 { value, .. } => Some(value),
            HitEffect::NoPayload { .. } => None,
        }
    }

    /// This hit was **avoided by the defender**: [`HIT_ARM_AVOIDED`], the arm
    /// that carries no damage word at all.
    pub fn is_avoided(&self) -> bool {
        matches!(
            self.effect,
            HitEffect::NoPayload {
                arm: HIT_ARM_AVOIDED
            }
        )
    }
}

/// Hit arm 2 — **the defender took no damage**. The record is the flag byte
/// alone; the original reads no damage word and zeroes its damage field.
///
/// It lands on player-class defenders only, never on a monster, and appears
/// independently in either instance slot of a two-instance skill.
/// Contrast [`HIT_ARM_DEAD_TARGET`].
///
/// **Which** avoidance it is stays unknown. The original's internal resolver
/// numbers `2 = MISS`, `5 = BLOCK`, `6 = PARRY`, `8 = defense`, but that is not
/// this wire byte's numbering — the wire carries no discriminator and no
/// amount, so miss, parry and block are indistinguishable here. The client
/// presents it as BLOCK (`docs/combat-math-server-spec.md` §5).
pub const HIT_ARM_AVOIDED: u8 = 2;

/// Hit arm 8 — **no hit: the target was already dead**. Also payload-less.
///
/// It only ever appears as the second instance of a two-instance basic attack
/// whose *first* instance carried the `0x80` killing blow. Unlike
/// [`HIT_ARM_AVOIDED`] it never appears in the first slot and
/// never on a contested roll, so it is a filler, not an outcome, and it
/// correctly produces no popup.
pub const HIT_ARM_DEAD_TARGET: u8 = 8;

/// Damage dealt to a single target entity by one action instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerEntityDamage {
    /// Unique id of the entity taking the damage.
    pub target: u32,
    /// One entry per damage instance (count = [`DamageContent::instance_count`],
    /// no per-entity prefix on the wire).
    pub hits: Vec<SkillPartDamage>,
}

/// The damage block of an attack-kind [`ObjectActionUpdate`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DamageContent {
    /// Hits per entity (multi-hit skills; 1 for basic attacks).
    pub instance_count: u8,
    /// u8-count-prefixed list of damaged entities.
    pub entities: Vec<PerEntityDamage>,
}

/// What kind of action a 0xB070 update describes (wire byte 0 / 1 / 8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionKind {
    /// Self-casts / buffs — no damage payload.
    None,
    /// A swing/cast that dealt damage; `None` damage = swing without payload.
    Attack {
        damage: Option<DamageContent>,
    },
    Teleport,
}

/// 0xB070 — server → client: one action instance executes (a basic-attack
/// swing or skill cast). This is where per-hit damage arrives; the matching
/// [`SkillEnd`] later echoes `instance`.
///
/// The body is 20 bytes, e.g.
/// `01 0030 c6980000 90890500 27060000 00000000 00` = self-buff, target 0,
/// kind none. A variant with an extra `u32` before `target` exists in some
/// descriptions but does not fit here; an all-zero tail cannot fully
/// disambiguate the two, so re-adding that `u32` is the first thing to try if
/// a real attack body lands in [`Self::Unknown`].
#[derive(Message, Clone, Debug, PartialEq)]
pub enum ObjectActionUpdate {
    Success {
        /// Seen as 0x3000; some sources describe it as `0x3002 | 0x3000`.
        unknown: u16,
        /// Ref skill id (basic attacks use the weapon's base-attack skill).
        skill_id: u32,
        /// Unique id of the acting entity.
        source: u32,
        /// Cast-instance counter, echoed by [`SkillEnd`].
        instance: u32,
        /// Unique id of the primary target (0 for self-casts).
        target: u32,
        kind: ActionKind,
    },
    /// `02 <error u16>` — the action failed and the original pops a message
    /// box. The handler's non-success branch is a single 2-byte read followed
    /// by the message-box call, so the tail is exactly one `u16`.
    Failure { error: u16 },
    /// Any shape the typed parse doesn't fit — kept raw, log-only.
    Unknown { result: u8, tail: Bytes },
}

impl ObjectActionUpdate {
    /// Parse the post-`result` body of the success shape; any error or
    /// leftover bytes reject the whole typed read (→ `Unknown`).
    fn parse_success(value: &Bytes) -> Option<ObjectActionUpdate> {
        let mut r = Reader::new(value);
        let _ = r.u8().ok()?; // result, already known == 1
        let unknown = r.u16().ok()?;
        let skill_id = r.u32().ok()?;
        let source = r.u32().ok()?;
        let instance = r.u32().ok()?;
        let target = r.u32().ok()?;
        let kind = match r.u8().ok()? {
            0 => ActionKind::None,
            1 => {
                let damage = if r.pos == value.len() {
                    None
                } else {
                    Some(Self::parse_damage(&mut r)?)
                };
                ActionKind::Attack { damage }
            }
            8 => ActionKind::Teleport,
            _ => return None,
        };
        (r.pos == value.len()).then_some(ObjectActionUpdate::Success {
            unknown,
            skill_id,
            source,
            instance,
            target,
            kind,
        })
    }

    fn parse_damage(r: &mut Reader) -> Option<DamageContent> {
        let instance_count = r.u8().ok()?;
        let entity_count = r.u8().ok()?;
        let mut entities = Vec::with_capacity(entity_count as usize);
        for _ in 0..entity_count {
            let target = r.u32().ok()?;
            let mut hits = Vec::with_capacity(instance_count as usize);
            for _ in 0..instance_count {
                hits.push(Self::parse_hit(r)?);
            }
            entities.push(PerEntityDamage { target, hits });
        }
        Some(DamageContent {
            instance_count,
            entities,
        })
    }

    /// One hit record. The flag byte splits into a killing-blow bit (`0x80`)
    /// and an arm selector (`flags & 0x7F`) whose
    /// arms have four different lengths.
    fn parse_hit(r: &mut Reader) -> Option<SkillPartDamage> {
        let flags = r.u8().ok()?;
        let arm = flags & 0x7F;
        let effect = match arm {
            0 => HitEffect::Damage {
                value: Self::parse_damage_word(r)?,
                unknown: r.u32().ok()?,
            },
            4 | 5 => {
                let value = Self::parse_damage_word(r)?;
                let unknown = r.u32().ok()?;
                let pos = HitPosition {
                    region: r.u16().ok()?,
                    x: r.i32().ok()?,
                    y: r.i32().ok()?,
                    z: r.i32().ok()?,
                };
                HitEffect::Displaced {
                    arm,
                    value,
                    unknown,
                    pos,
                }
            }
            7 => HitEffect::Arm7 {
                value: Self::parse_damage_word(r)?,
                unknown_a: r.u16().ok()?,
                unknown_b: r.u16().ok()?,
            },
            _ => HitEffect::NoPayload { arm },
        };
        Some(SkillPartDamage {
            killing_blow: flags & 0x80 != 0,
            effect,
        })
    }

    /// The packed damage word: `u8 damage state | u24 amount`, LE.
    fn parse_damage_word(r: &mut Reader) -> Option<DamageValue> {
        let word = r.u32().ok()?;
        Some(DamageValue {
            kind: (word & 0xFF) as u8,
            amount: word >> 8,
        })
    }
}

/// Encode one hit record — the inverse of `ObjectActionUpdate::parse_hit`.
fn write_hit(buf: &mut BytesMut, hit: &SkillPartDamage) {
    let arm = match hit.effect {
        HitEffect::Damage { .. } => 0,
        HitEffect::Displaced { arm, .. } | HitEffect::NoPayload { arm } => arm & 0x7F,
        HitEffect::Arm7 { .. } => 7,
    };
    buf.put_u8(arm | if hit.killing_blow { 0x80 } else { 0 });
    let word = |v: &DamageValue| (v.amount & 0x00FF_FFFF) << 8 | v.kind as u32;
    match &hit.effect {
        HitEffect::Damage { value, unknown } => {
            buf.put_u32_le(word(value));
            buf.put_u32_le(*unknown);
        }
        HitEffect::Displaced {
            value,
            unknown,
            pos,
            ..
        } => {
            buf.put_u32_le(word(value));
            buf.put_u32_le(*unknown);
            buf.put_u16_le(pos.region);
            buf.put_i32_le(pos.x);
            buf.put_i32_le(pos.y);
            buf.put_i32_le(pos.z);
        }
        HitEffect::Arm7 {
            value,
            unknown_a,
            unknown_b,
        } => {
            buf.put_u32_le(word(value));
            buf.put_u16_le(*unknown_a);
            buf.put_u16_le(*unknown_b);
        }
        HitEffect::NoPayload { .. } => {}
    }
}

/// Encode an [`ActionKind`] + optional damage block — the shared tail of
/// [`ObjectActionUpdate`] and [`SkillEnd`].
fn write_action_kind(buf: &mut BytesMut, kind: &ActionKind) {
    match kind {
        ActionKind::None => buf.put_u8(0),
        ActionKind::Teleport => buf.put_u8(8),
        ActionKind::Attack { damage } => {
            buf.put_u8(1);
            if let Some(damage) = damage {
                buf.put_u8(damage.instance_count);
                buf.put_u8(damage.entities.len() as u8);
                for entity in &damage.entities {
                    buf.put_u32_le(entity.target);
                    for hit in &entity.hits {
                        write_hit(buf, hit);
                    }
                }
            }
        }
    }
}

impl TryFrom<Bytes> for ObjectActionUpdate {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut r = Reader::new(&value);
        let result = r.u8()?;
        let parsed = match result {
            1 => ObjectActionUpdate::parse_success(&value),
            2 => match r.u16() {
                Ok(error) if r.pos == value.len() => Some(ObjectActionUpdate::Failure { error }),
                _ => None,
            },
            _ => None,
        };
        Ok(parsed.unwrap_or_else(|| ObjectActionUpdate::Unknown {
            result,
            tail: value.slice(1..),
        }))
    }
}

impl From<ObjectActionUpdate> for Bytes {
    fn from(p: ObjectActionUpdate) -> Self {
        let mut buf = BytesMut::new();
        match p {
            ObjectActionUpdate::Success {
                unknown,
                skill_id,
                source,
                instance,
                target,
                kind,
            } => {
                buf.put_u8(1);
                buf.put_u16_le(unknown);
                buf.put_u32_le(skill_id);
                buf.put_u32_le(source);
                buf.put_u32_le(instance);
                buf.put_u32_le(target);
                write_action_kind(&mut buf, &kind);
            }
            ObjectActionUpdate::Failure { error } => {
                buf.put_u8(2);
                buf.put_u16_le(error);
            }
            ObjectActionUpdate::Unknown { result, tail } => {
                buf.put_u8(result);
                buf.extend_from_slice(&tail);
            }
        }
        buf.freeze()
    }
}

/// 0xB071 — server → client: the cast instance ends. Confirmed on vSRO
/// 1.188: `01 <instance u32> <target u32> <kind u8> [DamageContent]` — the
/// same trailing layout as [`ObjectActionUpdate`]. This server delivers
/// TARGETED SKILL DAMAGE here (~0.5 s after the kind-None 0xB070 cast
/// start, roughly the visual hit moment); the 10-byte majority shape is the
/// same layout with target 0, kind 0 (self-buffs / auto-attack ends).
/// Anything else — error shapes, other server builds — lands in
/// [`Self::Unknown`] raw. No Failure variant: none has ever been seen.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum SkillEnd {
    Success {
        /// Echoes the [`ObjectActionUpdate`] cast-instance counter.
        instance: u32,
        /// Unique id of the damaged/affected entity, 0 when none.
        target: u32,
        kind: ActionKind,
    },
    Unknown {
        result: u8,
        tail: Bytes,
    },
}

impl TryFrom<Bytes> for SkillEnd {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut r = Reader::new(&value);
        let result = r.u8()?;
        let parsed = (|| -> Option<SkillEnd> {
            if result != 1 {
                return None;
            }
            let instance = r.u32().ok()?;
            let target = r.u32().ok()?;
            let kind = match r.u8().ok()? {
                0 => ActionKind::None,
                1 => {
                    let damage = if r.pos == value.len() {
                        None
                    } else {
                        Some(ObjectActionUpdate::parse_damage(&mut r)?)
                    };
                    ActionKind::Attack { damage }
                }
                8 => ActionKind::Teleport,
                _ => return None,
            };
            Some(SkillEnd::Success {
                instance,
                target,
                kind,
            })
        })()
        // leftover bytes = wrong shape guess; keep raw rather than misread
        .filter(|_| r.pos == value.len());
        Ok(parsed.unwrap_or_else(|| SkillEnd::Unknown {
            result,
            tail: value.slice(1..),
        }))
    }
}

impl From<SkillEnd> for Bytes {
    fn from(p: SkillEnd) -> Self {
        let mut buf = BytesMut::new();
        match p {
            SkillEnd::Success {
                instance,
                target,
                kind,
            } => {
                buf.put_u8(1);
                buf.put_u32_le(instance);
                buf.put_u32_le(target);
                write_action_kind(&mut buf, &kind);
            }
            SkillEnd::Unknown { result, tail } => {
                buf.put_u8(result);
                buf.extend_from_slice(&tail);
            }
        }
        buf.freeze()
    }
}

// --- Skill / mastery learning (0x70A1/0x70A2) -------------------------------
//
// The two requests are read off the original's builders and both acks are
// confirmed. Only the ERROR shapes stay assumed — a non-`01` result has never
// been seen, so those paths keep their bytes raw and warn.

/// 0x70A1 — client → server "learn this skill" (AGENT_SKILL_LEARN) (the skilldata ref id of the
/// next rung of the ladder).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct SkillLearnRequest {
    pub ref_skill_id: u32,
}

/// 0xB0A1 — server → client ack for [`SkillLearnRequest`].
///
/// Confirmed on vSRO 1.188: `01 <ref_skill_id u32>` — the ack is the ONLY
/// learn notification (no character-data refresh follows), so the client
/// applies it to the skill book directly. The error shape is assumed to be
/// `02 <code u16>` by analogy with other acks; anything else lands in
/// [`Self::Unknown`] raw.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum SkillLearnResponse {
    Success { ref_skill_id: u32 },
    Failure(u16),
    Unknown { result: u8, tail: Bytes },
}

impl TryFrom<Bytes> for SkillLearnResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut r = Reader::new(&value);
        let result = r.u8()?;
        let parsed = (|| -> Option<SkillLearnResponse> {
            match result {
                1 => Some(SkillLearnResponse::Success {
                    ref_skill_id: r.u32().ok()?,
                }),
                2 => Some(SkillLearnResponse::Failure(r.u16().ok()?)),
                _ => None,
            }
        })()
        // leftover bytes = wrong shape guess; keep raw rather than misread
        .filter(|_| r.pos == value.len());
        Ok(parsed.unwrap_or_else(|| SkillLearnResponse::Unknown {
            result,
            tail: value.slice(1..),
        }))
    }
}

impl From<SkillLearnResponse> for Bytes {
    fn from(p: SkillLearnResponse) -> Self {
        let mut buf = BytesMut::new();
        match p {
            SkillLearnResponse::Success { ref_skill_id } => {
                buf.put_u8(1);
                buf.put_u32_le(ref_skill_id);
            }
            SkillLearnResponse::Failure(code) => {
                buf.put_u8(2);
                buf.put_u16_le(code);
            }
            SkillLearnResponse::Unknown { result, tail } => {
                buf.put_u8(result);
                buf.extend_from_slice(&tail);
            }
        }
        buf.freeze()
    }
}

/// 0x70A2 — client → server "raise this mastery" (AGENT_SKILL_MASTERY_LEARN)
/// by `amount` levels (the vanilla client always sends 1).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct MasteryLearnRequest {
    pub mastery_id: u32,
    pub amount: u8,
}

/// 0xB0A2 — server → client ack for [`MasteryLearnRequest`].
///
/// Confirmed on vSRO 1.188: `01 <mastery_id u32> <new_level u8>`
/// (e.g. `01 01010000 05` = Bicheon raised to 5). The error shape is assumed
/// to be `02 <code u16>`; anything else lands in [`Self::Unknown`].
#[derive(Message, Clone, Debug, PartialEq)]
pub enum MasteryLearnResponse {
    Success { mastery_id: u32, new_level: u8 },
    Failure(u16),
    Unknown { result: u8, tail: Bytes },
}

impl TryFrom<Bytes> for MasteryLearnResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut r = Reader::new(&value);
        let result = r.u8()?;
        let parsed = (|| -> Option<MasteryLearnResponse> {
            match result {
                1 => Some(MasteryLearnResponse::Success {
                    mastery_id: r.u32().ok()?,
                    new_level: r.u8().ok()?,
                }),
                2 => Some(MasteryLearnResponse::Failure(r.u16().ok()?)),
                _ => None,
            }
        })()
        .filter(|_| r.pos == value.len());
        Ok(parsed.unwrap_or_else(|| MasteryLearnResponse::Unknown {
            result,
            tail: value.slice(1..),
        }))
    }
}

impl From<MasteryLearnResponse> for Bytes {
    fn from(p: MasteryLearnResponse) -> Self {
        let mut buf = BytesMut::new();
        match p {
            MasteryLearnResponse::Success {
                mastery_id,
                new_level,
            } => {
                buf.put_u8(1);
                buf.put_u32_le(mastery_id);
                buf.put_u8(new_level);
            }
            MasteryLearnResponse::Failure(code) => {
                buf.put_u8(2);
                buf.put_u16_le(code);
            }
            MasteryLearnResponse::Unknown { result, tail } => {
                buf.put_u8(result);
                buf.extend_from_slice(&tail);
            }
        }
        buf.freeze()
    }
}

// --- Server notice push (0x300C) --------------------------------------------
//
// The discriminator is **one u16**, not `u8 type` + `u8 unk01`. The original
// does a single read and then `switch (code & 0xffff)` over eighteen `0x0Cxx`
// labels; a second "unk01" byte is simply that u16's high byte, constant `0x0C`
// everywhere — which is why it looked like a stable but meaningless field. The
// wire settles it: `05 0c 43 95 00 00` is six bytes, so u16 `0x0C05` + u32 ref
// id, and a u32 discriminator would leave two bytes and read the id as 0.
//
// Only the two codes with a recorded meaning are modelled — `0x0C05` (unique
// appeared) and `0x0C06` (unique killed) — plus `0x0C18`, whose two leading
// bytes are known. The other fifteen codes decode to `Raw`: the original's
// `default:` arm reads nothing either, so raw is the faithful behaviour and
// inventing widths for them is exactly the defect ADR-0009 names. See
// docs/net-misc-0x2113.md §0x300C.
//
// The `pos == len` guard is the same one the learn/level acks use: a code whose
// assumed shape does not consume the body exactly falls through to `Raw` rather
// than being misread.

/// `0x0C05` — a unique monster appeared. Confirmed.
pub const NOTICE_UNIQUE_APPEARED: u16 = 0x0C05;
/// `0x0C06` — a unique monster was killed. The original's parser reads one
/// scalar and one string; unconfirmed on the wire.
pub const NOTICE_UNIQUE_KILLED: u16 = 0x0C06;
/// `0x0C18` — meaning unknown; only its two leading bytes are known.
pub const NOTICE_0C18: u16 = 0x0C18;

/// 0x300C — server → client notice push.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum NoticeUpdate {
    /// [`NOTICE_UNIQUE_APPEARED`] — `ref_id` is a ref-data object id, not a
    /// model id: the original hands it to the ref-data lookup. Confirmed:
    /// `05 0c 43 95 00 00` → 38211.
    UniqueAppeared { ref_id: u32 },
    /// [`NOTICE_UNIQUE_KILLED`] — same lookup plus the killer's name.
    UniqueKilled { ref_id: u32, player: String },
    /// [`NOTICE_0C18`] — two known bytes (`18 0c 02 03` → 2, 3) and then, on
    /// an unknown condition, eight more. Since that gate is unknown, whatever
    /// follows is kept verbatim rather than gated on a guess.
    Code0C18 { a: u8, b: u8, tail: Bytes },
    /// Any other code — kept whole, code included, because no source records
    /// its field widths.
    Raw { code: u16, tail: Bytes },
}

impl NoticeUpdate {
    /// The wire code this notice carries.
    pub fn code(&self) -> u16 {
        match self {
            NoticeUpdate::UniqueAppeared { .. } => NOTICE_UNIQUE_APPEARED,
            NoticeUpdate::UniqueKilled { .. } => NOTICE_UNIQUE_KILLED,
            NoticeUpdate::Code0C18 { .. } => NOTICE_0C18,
            NoticeUpdate::Raw { code, .. } => *code,
        }
    }
}

impl TryFrom<Bytes> for NoticeUpdate {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let mut r = Reader::new(&value);
        let code = r.u16()?;
        let parsed = (|| -> Option<NoticeUpdate> {
            match code {
                NOTICE_UNIQUE_APPEARED => Some(NoticeUpdate::UniqueAppeared {
                    ref_id: r.u32().ok()?,
                }),
                NOTICE_UNIQUE_KILLED => {
                    let ref_id = r.u32().ok()?;
                    let len = r.u16().ok()? as usize;
                    let name = r.take(len).ok()?;
                    Some(NoticeUpdate::UniqueKilled {
                        ref_id,
                        player: String::from_utf8_lossy(name).into_owned(),
                    })
                }
                NOTICE_0C18 => {
                    let a = r.u8().ok()?;
                    let b = r.u8().ok()?;
                    let tail = value.slice(r.pos..);
                    r.pos = value.len();
                    Some(NoticeUpdate::Code0C18 { a, b, tail })
                }
                _ => None,
            }
        })()
        .filter(|_| r.pos == value.len());
        Ok(parsed.unwrap_or_else(|| NoticeUpdate::Raw {
            code,
            tail: value.slice(2.min(value.len())..),
        }))
    }
}

impl From<NoticeUpdate> for Bytes {
    fn from(p: NoticeUpdate) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u16_le(p.code());
        match p {
            NoticeUpdate::UniqueAppeared { ref_id } => buf.put_u32_le(ref_id),
            NoticeUpdate::UniqueKilled { ref_id, player } => {
                buf.put_u32_le(ref_id);
                buf.put_u16_le(player.len() as u16);
                buf.extend_from_slice(player.as_bytes());
            }
            NoticeUpdate::Code0C18 { a, b, tail } => {
                buf.put_u8(a);
                buf.put_u8(b);
                buf.extend_from_slice(&tail);
            }
            NoticeUpdate::Raw { tail, .. } => buf.extend_from_slice(&tail),
        }
        buf.freeze()
    }
}

// --- Mastery / skill level-DOWN (0x7202/0x7203, 0xB202/0xB203) --------------
//
// The mirror of the level-UP flow above. Both responses are read from the
// original's parsers, but neither *request* has a builder there, so their
// bodies are mirrored from the level-UP siblings and stay unconfirmed.
//
// The response enums reuse the level-UP shapes verbatim, including the
// `pos == len` guard. That guard matters more here than it does above: the
// original reads no error code on the failure branch, so whether a failure is a
// lone `02` or `02 <code u16>` is unresolved — and the guard makes both safe,
// because a shape that does not consume the body exactly falls through to
// `Unknown` raw instead of being misread.

/// 0x7202 — client → server "lower this skill by one level".
///
/// **Unknown body.** The original has no builder for this opcode; the single
/// `u32` is mirrored from the level-UP sibling [`SkillLearnRequest`]. Confirm
/// it before anything sends it.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct SkillLevelDownRequest {
    pub ref_skill_id: u32,
}

/// 0xB202 — server → client ack for [`SkillLevelDownRequest`].
///
/// `01 <new_skill_id u32>` on success — a level-down returns the id of the skill at
/// its new, lower level. The failure shape is unconfirmed; see the section
/// comment.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum MasterySkillLevelDownResponse {
    Success { new_skill_id: u32 },
    Failure(u16),
    Unknown { result: u8, tail: Bytes },
}

impl TryFrom<Bytes> for MasterySkillLevelDownResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut r = Reader::new(&value);
        let result = r.u8()?;
        let parsed = (|| -> Option<MasterySkillLevelDownResponse> {
            match result {
                1 => Some(MasterySkillLevelDownResponse::Success {
                    new_skill_id: r.u32().ok()?,
                }),
                2 => Some(MasterySkillLevelDownResponse::Failure(r.u16().ok()?)),
                _ => None,
            }
        })()
        .filter(|_| r.pos == value.len());
        Ok(
            parsed.unwrap_or_else(|| MasterySkillLevelDownResponse::Unknown {
                result,
                tail: value.slice(1..),
            }),
        )
    }
}

impl From<MasterySkillLevelDownResponse> for Bytes {
    fn from(p: MasterySkillLevelDownResponse) -> Self {
        let mut buf = BytesMut::new();
        match p {
            MasterySkillLevelDownResponse::Success { new_skill_id } => {
                buf.put_u8(1);
                buf.put_u32_le(new_skill_id);
            }
            MasterySkillLevelDownResponse::Failure(code) => {
                buf.put_u8(2);
                buf.put_u16_le(code);
            }
            MasterySkillLevelDownResponse::Unknown { result, tail } => {
                buf.put_u8(result);
                buf.extend_from_slice(&tail);
            }
        }
        buf.freeze()
    }
}

/// 0x7203 — client → server "lower this mastery by one level".
///
/// **Unknown body.** No builder in the original. The level-UP sibling
/// [`MasteryLearnRequest`] carries a trailing `amount: u8`; whether the DOWN
/// request does too is unresolved, so it is **not** included here rather than
/// assumed.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct MasteryLevelDownRequest {
    pub mastery_id: u32,
}

/// 0xB203 — server → client ack for [`MasteryLevelDownRequest`].
///
/// `01 <mastery_id u32> <new_level u8>` on success — the exact mirror of
/// [`MasteryLearnResponse`]. The failure shape is unconfirmed; see the section
/// comment.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum MasteryLevelDownResponse {
    Success { mastery_id: u32, new_level: u8 },
    Failure(u16),
    Unknown { result: u8, tail: Bytes },
}

impl TryFrom<Bytes> for MasteryLevelDownResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut r = Reader::new(&value);
        let result = r.u8()?;
        let parsed = (|| -> Option<MasteryLevelDownResponse> {
            match result {
                1 => Some(MasteryLevelDownResponse::Success {
                    mastery_id: r.u32().ok()?,
                    new_level: r.u8().ok()?,
                }),
                2 => Some(MasteryLevelDownResponse::Failure(r.u16().ok()?)),
                _ => None,
            }
        })()
        .filter(|_| r.pos == value.len());
        Ok(parsed.unwrap_or_else(|| MasteryLevelDownResponse::Unknown {
            result,
            tail: value.slice(1..),
        }))
    }
}

impl From<MasteryLevelDownResponse> for Bytes {
    fn from(p: MasteryLevelDownResponse) -> Self {
        let mut buf = BytesMut::new();
        match p {
            MasteryLevelDownResponse::Success {
                mastery_id,
                new_level,
            } => {
                buf.put_u8(1);
                buf.put_u32_le(mastery_id);
                buf.put_u8(new_level);
            }
            MasteryLevelDownResponse::Failure(code) => {
                buf.put_u8(2);
                buf.put_u16_le(code);
            }
            MasteryLevelDownResponse::Unknown { result, tail } => {
                buf.put_u8(result);
                buf.extend_from_slice(&tail);
            }
        }
        buf.freeze()
    }
}

// --- Stat point allocation (0x7050/0x7051) ----------------------------------
//
// Confirmed on vSRO 1.188: empty 0x7050 requests ack `01` on success and
// `02 7406` when the wallet is empty. The
// stat-point balance and the new STR/INT arrive separately (0x304E StatPoints,
// 0x303D stats refresh), so the acks carry no payload on success. 0x7051 (INT)
// shares the shape but is unconfirmed.

/// 0x7050 — client → server "spend one stat point on strength".
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct IncreaseStrRequest;

/// 0x7051 — client → server "spend one stat point on intelligence".
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct IncreaseIntRequest;

/// 0xB050 — server → client ack for [`IncreaseStrRequest`]. Assumed `01` on
/// success and `02 <code u16>` on error by analogy with the other acks;
/// anything else lands in [`Self::Unknown`] raw.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum IncreaseStrResponse {
    Success,
    Failure(u16),
    Unknown { result: u8, tail: Bytes },
}

/// 0xB051 — server → client ack for [`IncreaseIntRequest`] (same shape as
/// [`IncreaseStrResponse`]).
#[derive(Message, Clone, Debug, PartialEq)]
pub enum IncreaseIntResponse {
    Success,
    Failure(u16),
    Unknown { result: u8, tail: Bytes },
}

macro_rules! stat_ack_wire {
    ($name:ident) => {
        impl TryFrom<Bytes> for $name {
            type Error = SerializationError;
            fn try_from(value: Bytes) -> Result<Self, Self::Error> {
                let mut r = Reader::new(&value);
                let result = r.u8()?;
                let parsed = (|| -> Option<$name> {
                    match result {
                        1 => Some($name::Success),
                        2 => Some($name::Failure(r.u16().ok()?)),
                        _ => None,
                    }
                })()
                // leftover bytes = wrong shape guess; keep raw rather than misread
                .filter(|_| r.pos == value.len());
                Ok(parsed.unwrap_or_else(|| $name::Unknown {
                    result,
                    tail: value.slice(1..),
                }))
            }
        }

        impl From<$name> for Bytes {
            fn from(p: $name) -> Self {
                let mut buf = BytesMut::new();
                match p {
                    $name::Success => buf.put_u8(1),
                    $name::Failure(code) => {
                        buf.put_u8(2);
                        buf.put_u16_le(code);
                    }
                    $name::Unknown { result, tail } => {
                        buf.put_u8(result);
                        buf.extend_from_slice(&tail);
                    }
                }
                buf.freeze()
            }
        }
    };
}

stat_ack_wire!(IncreaseStrResponse);
stat_ack_wire!(IncreaseIntResponse);

// --- NPC talk (0x7046/0xB046, 0x704B/0xB04B) --------------------------------
//
// 0x7046 and 0x704B are confirmed on vSRO 1.188: the talk
// acks `01` plus the echoed talk flag, with errors `02 0500` (out of range /
// unselected) and `02 0b1c` (session already open — cleared by sending the
// close first); the close acks a bare `01` (docs/net-npc-talk-0x7046.md).

/// 0x7046 — client → server "start talking to this NPC" (flag 1 = open the
/// talk dialog).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct TalkRequest {
    pub unique_id: u32,
    pub talk_flag: u8,
}

/// 0xB046 — server → client ack for [`TalkRequest`]. Only the result byte is
/// interpreted; a success' payload (if any) stays raw.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum TalkResponse {
    Success { tail: Bytes },
    Failure(u16),
    Unknown { result: u8, tail: Bytes },
}

impl TryFrom<Bytes> for TalkResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut r = Reader::new(&value);
        let result = r.u8()?;
        Ok(match result {
            1 => TalkResponse::Success {
                tail: value.slice(1..),
            },
            2 => match r.u16() {
                Ok(code) if r.pos == value.len() => TalkResponse::Failure(code),
                _ => TalkResponse::Unknown {
                    result,
                    tail: value.slice(1..),
                },
            },
            _ => TalkResponse::Unknown {
                result,
                tail: value.slice(1..),
            },
        })
    }
}

impl From<TalkResponse> for Bytes {
    fn from(p: TalkResponse) -> Self {
        let mut buf = BytesMut::new();
        match p {
            TalkResponse::Success { tail } => {
                buf.put_u8(1);
                buf.extend_from_slice(&tail);
            }
            TalkResponse::Failure(code) => {
                buf.put_u8(2);
                buf.put_u16_le(code);
            }
            TalkResponse::Unknown { result, tail } => {
                buf.put_u8(result);
                buf.extend_from_slice(&tail);
            }
        }
        buf.freeze()
    }
}

/// 0x7059 — client → server "make this teleporter my recall point".
///
/// The original has no builder for this opcode, so the single `u32` below — one
/// teleporter id — is **unconfirmed**.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct TeleportRecallRequest {
    pub teleport_unique_id: u32,
}

/// 0xB059 — server → client ack for [`TeleportRecallRequest`].
///
/// **Carried whole.** The original parses this ack: it reads a `u8 result`,
/// shows `UIIT_MSG_STATE_REBIRTH_POINT_APPOINT` on `result == 1` and otherwise
/// reads a `u16` error code it never displays — so one byte on success, three
/// on failure, and a vSRO server writes a bare `01`.
///
/// What is still open: the error-code *values*. The body stays a raw
/// passthrough for now — typing it is a wire change, not a comment fix.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct TeleportRecallResponse {
    pub raw: Bytes,
}

impl TryFrom<Bytes> for TeleportRecallResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        Ok(TeleportRecallResponse { raw: value })
    }
}

impl From<TeleportRecallResponse> for Bytes {
    fn from(p: TeleportRecallResponse) -> Self {
        p.raw
    }
}

/// 0x705A — client → server "teleport me via this teleporter"
/// (AGENT_TELEPORT_USE). The `kind` discriminator is 2 for a
/// designated-destination teleport; the destination is the teleportdata id as
/// a **u32**, not a u16: with a u16 the server reads 2 bytes past the 7-byte
/// body and resets the connection without any 0xB05A.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct TeleportRequest {
    pub npc_unique_id: u32,
    pub kind: u8,
    pub destination_id: u32,
}

/// 0xB05A — server → client ack for [`TeleportRequest`]. Confirmed on vSRO
/// 1.188: a TWO-PHASE ack — `02 01 00` (begin, with the zone-teardown despawns
/// sandwiched after it) then `01` about 40 ms later (committed, right before
/// the 0x34B5 GameReset). Logging-only either way.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum TeleportResponse {
    /// `02 <code u16>` — teleport accepted, teardown starting (code 1 is the
    /// known value; error codes may share this shape).
    Begin { code: u16 },
    /// `01` — teleport committed; 0x34B5 GameReset follows.
    Committed,
    /// Anything else — kept raw, log-only.
    Unknown { result: u8, tail: Bytes },
}

impl TryFrom<Bytes> for TeleportResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let mut r = Reader::new(&value);
        let result = r.u8()?;
        let parsed = (|| -> Option<TeleportResponse> {
            match result {
                1 => Some(TeleportResponse::Committed),
                2 => Some(TeleportResponse::Begin {
                    code: r.u16().ok()?,
                }),
                _ => None,
            }
        })()
        .filter(|_| r.pos == value.len());
        Ok(parsed.unwrap_or_else(|| TeleportResponse::Unknown {
            result,
            tail: value.slice(1..),
        }))
    }
}

impl From<TeleportResponse> for Bytes {
    fn from(p: TeleportResponse) -> Self {
        let mut buf = BytesMut::new();
        match p {
            TeleportResponse::Begin { code } => {
                buf.put_u8(2);
                buf.put_u16_le(code);
            }
            TeleportResponse::Committed => buf.put_u8(1),
            TeleportResponse::Unknown { result, tail } => {
                buf.put_u8(result);
                buf.extend_from_slice(&tail);
            }
        }
        buf.freeze()
    }
}

/// 0x705B — client → server "abort the cast I am in the middle of". Empty body.
///
/// The cancel behind `GDR_DI_CANCEL`, the button on the cast/delay gauge. The
/// original builds it with no payload at all; the surrounding UI strings are
/// `UIIT_STT_TRANSITION_CANCEL` (the button) and
/// `UIIT_MSG_TRANSITION_CANCEL_RESULT` (the ack's message).
///
/// ⚠️ **vSRO never writes the [`TransitionCastingCancelResponse`]**, so a
/// server is expected to ignore this. Do not read silence as a
/// malformed request — read it as an unimplemented one. Distinct from
/// [`ObjectActionRequest::Cancel`] (`0x7074`, byte `02`), which aborts the
/// object-action loop and has no bearing on a `0x704C` item cast.
#[derive(Message, Clone, Debug, Default)]
pub struct TransitionCastingCancelRequest;

empty_packet!(TransitionCastingCancelRequest);

/// 0xB05B — server → client: result of [`TransitionCastingCancelRequest`].
///
/// Per the original's handler: `result == 1` is bare and shows
/// `UIIT_MSG_TRANSITION_CANCEL_RESULT`; `result == 2` carries a `u16` the
/// original reads but never displays. Same shape as [`LogoutCancelResponse`],
/// which is the other "cancel a pending countdown" pair.
///
/// Never seen on the wire — see the request's note.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct TransitionCastingCancelResponse {
    pub result: u8,
    #[sro_packet(when = "result == 2")]
    pub error: Option<u16>,
}

/// 0x704B — client → server "close the talk session with this NPC".
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct CloseTalkRequest {
    pub unique_id: u32,
}

/// 0xB04B — server → client ack for [`CloseTalkRequest`] (assumed the common
/// `01` / `02 <code u16>` shape).
#[derive(Message, Clone, Debug, PartialEq)]
pub enum CloseTalkResponse {
    Success,
    Failure(u16),
    Unknown { result: u8, tail: Bytes },
}

stat_ack_wire!(CloseTalkResponse);

/// [`EntityStateUpdate::kind`]: the entity's life state changed.
pub const STATE_KIND_LIFE: u8 = 0;
/// [`EntityStateUpdate::kind`]: the entity's motion state changed.
pub const STATE_KIND_MOTION: u8 = 1;
/// [`EntityStateUpdate::kind`]: the entity's body state changed (invisibility,
/// invincibility, stealth, berserk).
pub const STATE_KIND_BODY: u8 = 4;
/// Life-state values (kind 0).
pub const LIFE_STATE_ALIVE: u8 = 1;
pub const LIFE_STATE_DEAD: u8 = 2;

/// Motion-state values (kind 1): a monster toggling its wander gait sends `2`
/// when it starts strolling and `3` when it runs. The same byte is the
/// `motion_state` of every spawn/state block.
pub const MOTION_STATE_WALK: u8 = 2;
pub const MOTION_STATE_RUN: u8 = 3;

/// A state kind that is on the wire but that no source names: a clean 1/0
/// toggle that appears **only on the local player's uid**, goes `1` when a
/// fight starts, and whose `→ 0` transition lands in the same millisecond as a
/// death. Read as the in-combat flag the spawn record also carries; this is an
/// inference.
pub const STATE_KIND_COMBAT: u8 = 8;

/// Body-state values (kind 4). The three that hide the entity are grouped in
/// [`body_state_is_invisible`].
pub const BODY_STATE_NONE: u8 = 0;
/// Post-resurrect invulnerability. A revive sets it in the same millisecond as
/// life→alive and clears it after about 6.2 s; it occurs nowhere else
/// (`docs/net-death-resurrect.md` §2).
pub const BODY_STATE_UNTOUCHABLE: u8 = 2;
pub const BODY_STATE_GM_INVINCIBLE: u8 = 3;
pub const BODY_STATE_GM_INVISIBLE: u8 = 4;
pub const BODY_STATE_STEALTH: u8 = 6;
pub const BODY_STATE_INVISIBLE: u8 = 7;

/// How a hidden entity should be drawn for *somebody else's* character.
///
/// [`body_state_is_invisible`] answers "is this entity hidden", which is the
/// right question for your **own** body — all three values mean you are hidden
/// — but the wrong one for drawing another character, because GM invisibility
/// and stealth are not the same secret. A GM is meant to see other GMs as
/// translucent ghosts; nobody is meant to see a stealthed player at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HiddenRender {
    /// Draw normally.
    Visible,
    /// Draw translucent — GM invisibility, seen by a GM.
    Ghost,
    /// Do not draw.
    Hidden,
}

/// Resolve [`HiddenRender`] for another character's body state.
///
/// **Caveat, and the reason callers must scope this to players.** Kind 4 with
/// value 4 arrives for hundreds of distinct unique ids, almost none of which
/// ever emit a life state, so most of them are not monsters in combat. If
/// value 4 really meant GM-invisible for all of them, applying this to every
/// entity would hide hundreds of them from a non-GM. What value 4 means for a
/// **non-player** entity is therefore **UNKNOWN**, so this must be asked only
/// about characters.
pub fn hidden_render(body_state: u8, viewer_is_gm: bool) -> HiddenRender {
    match body_state {
        // A GM's own invisibility: fellow GMs see the ghost, nobody else sees
        // anything.
        BODY_STATE_GM_INVISIBLE if viewer_is_gm => HiddenRender::Ghost,
        BODY_STATE_GM_INVISIBLE => HiddenRender::Hidden,
        // Stealth and player invisibility are gameplay, not moderation: being
        // a GM does not entitle you to see through them here.
        BODY_STATE_STEALTH | BODY_STATE_INVISIBLE => HiddenRender::Hidden,
        _ => HiddenRender::Visible,
    }
}

/// 0xB0BD — server → client: a buff landed on an entity. Confirmed on vSRO
/// 1.188 (fixed 12-byte payloads; the server self-casts skill 39110 on every
/// join and announces it here). No duration
/// on the wire — the client derives it from skilldata's `'dura'` param.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct BuffAdd {
    /// Unique id of the entity the buff applies to.
    pub unique_id: u32,
    pub ref_skill_id: u32,
    /// Instance id echoed by the (assumed) 0xB072 removal.
    pub buff_instance_id: u32,
}

/// 0xB0BD's counterpart, 0xB072 — buffs removed by instance id. The leading
/// byte is a **COUNT, not a result**: the body is a list.
///
/// The original's handler takes one byte and then consumes exactly that many
/// u32 ids, resolving each to its ref skill id locally. The server writer
/// agrees: it derives the byte from a vector size and writes one u32 per
/// element, while its five single-removal writers hard-code it to 1.
///
/// Every body seen so far is a one-element list (e.g. `01 8c030000`), and a
/// single removal cannot tell `{result:1, id}` from `{count:1, [id]}`.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct BuffRemove {
    /// Instance ids to drop, in wire order (count-prefixed, never empty in practice).
    pub buff_instance_ids: Vec<u32>,
}

impl TryFrom<Bytes> for BuffRemove {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut r = Reader::new(&value);
        let count = r.u8()?;
        let mut buff_instance_ids = Vec::with_capacity(count as usize);
        for _ in 0..count {
            buff_instance_ids.push(r.u32()?);
        }
        Ok(BuffRemove { buff_instance_ids })
    }
}

impl From<BuffRemove> for Bytes {
    fn from(p: BuffRemove) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u8(p.buff_instance_ids.len() as u8);
        for id in &p.buff_instance_ids {
            buf.put_u32_le(*id);
        }
        buf.freeze()
    }
}

/// 0x704F — client → server: the character's own motion/posture command, the
/// verb behind three of the action window's character-control slots.
///
/// One byte, and its value space comes from the original itself: the action
/// window's command dispatcher over `CommandID` 1000..=1017 routes `1000`
/// (sit/stand) and `1001` (walk/run) into the same builder, which opens
/// `0x704F` and writes a single byte. The callers supply the value:
///
/// * walk/run: `2` when the character is walking and `3` otherwise — the
///   *other* gait, i.e. a toggle,
/// * sit/stand: `4`, guarded by two state checks.
///
/// The 2/3 pair is the same encoding [`MOTION_STATE_WALK`]/[`MOTION_STATE_RUN`]
/// carry inbound on 0x30BF, which is an independent confirmation of both.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct CharacterActionRequest {
    pub action: u8,
}

/// Switch to walking — the same value 0x30BF reports as [`MOTION_STATE_WALK`].
pub const CHARACTER_ACTION_WALK: u8 = MOTION_STATE_WALK;
/// Switch to running — [`MOTION_STATE_RUN`].
pub const CHARACTER_ACTION_RUN: u8 = MOTION_STATE_RUN;
/// Sit down / stand up. The original sends one value for both directions: it
/// is a toggle the server resolves, not a state we assert.
pub const CHARACTER_ACTION_SIT_STAND: u8 = 4;

impl CharacterActionRequest {
    pub fn sit_stand() -> Self {
        Self {
            action: CHARACTER_ACTION_SIT_STAND,
        }
    }

    /// The gait we want *next*: `walking = true` sends 2, else 3.
    pub fn gait(walking: bool) -> Self {
        Self {
            action: if walking {
                CHARACTER_ACTION_WALK
            } else {
                CHARACTER_ACTION_RUN
            },
        }
    }
}

/// 0x3091 — client → server: play an emote. **One byte, the emote code.**
///
/// Direction caveat, the same one [`GetUpRequest`] carries: 0x3091 sits in the
/// 0x3xxx range this repo otherwise treats as S→C, and the opcode is aliased in
/// both directions (the original has no *parser* for it, only a builder).
///
/// Both the opcode and the code table come from the action window's
/// dispatcher: `CommandID` 4000 lands in a builder that opens `0x3091` and
/// writes exactly one byte, the constant that arm holds. `CommandID`
/// 4001..=4006 run through a second table with one arm each, built
/// identically. That is where the mapping in [`EMOTE_CODES`] comes from, and
/// why it is *not* the `4000 + n` order the icons suggest.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct EmoteRequest {
    pub emote: u8,
}

/// `actionwnddata.txt` command id → 0x3091 emote code, one arm at a time. The
/// wire order is not the slot order: laugh is 6, pokun is 1, joy is 3.
pub const EMOTE_CODES: [(u32, u8); 7] = [
    (4000, 0), // 인사 greeting
    (4001, 6), // 웃음 laugh
    (4002, 1), // 포권 pokun
    (4003, 5), // 네 yes
    (4004, 2), // 돌진 rush
    (4005, 3), // 아자 joy
    (4006, 4), // 아니오 no
];

impl EmoteRequest {
    /// The emote a `actionwnddata.txt` command id plays, if it is an emote.
    pub fn for_command(command_id: u32) -> Option<Self> {
        EMOTE_CODES
            .iter()
            .find(|(id, _)| *id == command_id)
            .map(|(_, emote)| Self { emote: *emote })
    }
}

/// 0x70A7 — client → server: the hwan (jahwan / berserk) activation request.
///
/// One byte. The original's builder writes exactly one, handed to it by its
/// caller. **What that byte enumerates is unknown** — the value space is
/// unbounded, and `1 = berserk` is the only value we send.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct HwanActionRequest {
    pub action: u8,
}

/// The only known 0x70A7 action value: activate.
pub const HWAN_ACTION_BERSERK: u8 = 1;

impl HwanActionRequest {
    pub fn berserk() -> Self {
        HwanActionRequest {
            action: HWAN_ACTION_BERSERK,
        }
    }
}

/// 0xB0A7 — the server's answer to 0x70A7.
///
/// The handler reads one byte, and **only when it is not `1`** reads a `u16`
/// and hands it to the message box. So the error code is conditional, not a
/// fixed trailer: a success body is a single byte.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct HwanActionResponse {
    pub result: u8,
    #[sro_packet(when = "result != 1")]
    pub error_code: Option<u16>,
}

impl HwanActionResponse {
    pub fn is_success(&self) -> bool {
        self.result == 1
    }
}

/// 0x30DF — server → client HWANLEVEL: an entity's hwan level changed.
///
/// The original reads a `u32` and a `u8`, then resolves the `u32` through the
/// object registry before applying the byte — so the `u32` is a unique id and
/// the `u8` is the level. What the level drives visually (the hwan aura tier)
/// is not modelled
/// on our side yet.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct HwanLevelUpdate {
    pub unique_id: u32,
    pub level: u8,
}

/// Whether a body-state value renders the entity invisible (GM invisible,
/// player invisible, or stealth).
pub fn body_state_is_invisible(body_state: u8) -> bool {
    matches!(
        body_state,
        BODY_STATE_GM_INVISIBLE | BODY_STATE_INVISIBLE | BODY_STATE_STEALTH
    )
}

/// 0x30BF — server → client entity state change. Confirmed on vSRO
/// 1.188: `kind` 0 = life state (1 alive, **2 dead — arrives when a
/// monster dies, before its despawn**), `kind` 1 = motion state (2 walk,
/// 3 run — monsters toggling their wander gait). `kind` 4 = body state
/// (invisibility/invincibility/stealth), which the GM `/invisible` toggle
/// produces for the acting entity.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct EntityStateUpdate {
    pub unique_id: u32,
    pub kind: u8,
    pub value: u8,
}

impl EntityStateUpdate {
    /// The entity just died (life state → dead).
    pub fn is_death(&self) -> bool {
        self.kind == STATE_KIND_LIFE && self.value == LIFE_STATE_DEAD
    }

    /// The entity just revived (life state → alive) — the local player's
    /// respawn/get-up signal. Unconfirmed: whether the server sends
    /// this to the reviving player itself is not settled.
    pub fn is_revive(&self) -> bool {
        self.kind == STATE_KIND_LIFE && self.value == LIFE_STATE_ALIVE
    }

    /// `Some(is_invisible)` when this is a body-state update, else `None`.
    pub fn body_invisibility(&self) -> Option<bool> {
        (self.kind == STATE_KIND_BODY).then(|| body_state_is_invisible(self.value))
    }

    /// The raw body-state value when this is a body-state update, else `None`.
    ///
    /// [`Self::body_invisibility`] collapses the value to a yes/no, which is
    /// all your own body needs. Drawing *another* character needs the value
    /// itself, because GM invisibility and stealth are shown differently — see
    /// [`hidden_render`].
    pub fn body_state_value(&self) -> Option<u8> {
        (self.kind == STATE_KIND_BODY).then_some(self.value)
    }

    /// `Some(is_walking)` when this is a motion-state update naming a gait,
    /// else `None`. Only the two known gaits answer: an unknown motion
    /// value must not be guessed into "running" and silently pick an
    /// animation.
    pub fn motion_walking(&self) -> Option<bool> {
        if self.kind != STATE_KIND_MOTION {
            return None;
        }
        match self.value {
            MOTION_STATE_WALK => Some(true),
            MOTION_STATE_RUN => Some(false),
            _ => None,
        }
    }
}

/// 0x3053 — client → server CLIENT_CHARACTER_AUTORESURRECTION: the death
/// window's resurrect request. Carries a single option byte selecting the
/// resurrect mode — the body is one byte, not empty.
///
/// Two option values are known:
///
/// - `2` present resurrection point. vSRO gates this one (level/scroll), and a
///   server that refuses it answers with *nothing at all*.
/// - `1` return to the designated resurrection point (town). Unconfirmed, but
///   it is the only always-available option.
///
/// The "wait for other player's help" case sends no packet at all (it is the
/// server's default: stay dead), so it has no option byte.
///
/// Direction caveat: 0x3053 sits in the 0x3xxx range this repo otherwise treats
/// as S→C, but death/resurrect is the documented C→S exception.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, Default, PartialEq)]
pub struct GetUpRequest {
    pub option: u8,
}

impl GetUpRequest {
    /// Return to the designated resurrection point / town (value `1`).
    pub const RETURN_TO_TOWN: u8 = 1;

    /// Free resurrect at the present resurrection point (value `2`).
    pub const PRESENT_POINT: u8 = 2;

    /// Resurrect at the designated resurrection point (the town return): the
    /// option a dead character always has, whatever its level.
    pub fn return_to_town() -> Self {
        Self {
            option: Self::RETURN_TO_TOWN,
        }
    }

    /// The free resurrect at the present resurrection point.
    pub fn present_point() -> Self {
        Self {
            option: Self::PRESENT_POINT,
        }
    }
}

/// 0x3054 — server → client: the entity leveled up (play the level-up
/// effect on it). Confirmed: a bare unique id.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct EntityLevelUp {
    pub unique_id: u32,
}

/// 0x3011 — server → client: the local character died (drives the death
/// window / respawn UI). One byte: the original reads exactly one byte and
/// stops, and every real body is a single `04`.
///
/// UNKNOWN: the value space of [`Self::death_cause`]. Only `0x04` has ever
/// been seen, and the original's own comment merely guesses at it
/// ("4 = Dead by mob?"), so the raw byte is exposed rather than branched on.
/// It is **not** the `LifeState` discriminator carried by spawn/state-update
/// — do not conflate the two.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct CharacterDied {
    pub death_cause: u8,
}

/// 0x304D — server → client: a dropped item's owner-lock has expired, so
/// anyone may pick it up now. Body is the drop entity's unique id.
///
/// Layout is supported but not confirmed: the original has no parser for this
/// opcode, only an enum entry. A real body such as `aa600100` → `0x000160AA`
/// reads cleanly as the `u32` unique id that drop spawns and their owner field
/// both use. Trailing fields cannot be ruled out; the generated decode ignores
/// a tail, so a longer body degrades to "unique id only" rather than failing.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct DropUnlocked {
    pub unique_id: u32,
}

/// 0x3056 — server → client experience delta: a gain from a kill, or the
/// negative EXP penalty charged on death (see [`Self::experience`]). Confirmed
/// on vSRO 1.188 (21-byte bodies, empty tail); a level-up appends a trailing
/// u16 that is the character's **total stat points**, NOT the new level — the
/// values seen (3, 6, 9, 12) are 3×(level-1), i.e. the 3 stat-points-per-level
/// award, which some sources mislabel `new_level`. The level
/// itself is derived from the exp curve (leveldata), so this is exposed only
/// as the stat-point total.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct ReceiveExperience {
    /// Unique id of the entity that provided the experience.
    pub exp_origin: u32,
    /// Experience delta — **signed**: a kill grants a positive value, death
    /// charges the EXP penalty as a negative one (e.g. `5df3ffffffffffff` =
    /// −3235, with `exp_origin` set to the dying player's own uid).
    pub experience: i64,
    /// Skill-experience points gained (400 sp-exp = 1 SP).
    pub sp_exp: u64,
    /// Flag for extra trailing data (always 0 so far).
    pub unknown: u8,
    /// Raw remainder, see [`Self::stat_points`].
    pub tail: Bytes,
}

impl ReceiveExperience {
    /// The character's total stat points, present only when this gain caused
    /// a level-up (its presence therefore also flags "a level-up happened").
    /// The client applies this to the stat-point wallet in
    /// `character_info::model::on_experience_stat_points` — required because
    /// some servers (e.g. the reference vSRO) never send a 0x304E `StatPoints`
    /// refresh, so without it the wallet would freeze at the login value.
    pub fn stat_points(&self) -> Option<u16> {
        (self.tail.len() >= 2).then(|| u16::from_le_bytes(self.tail[0..2].try_into().unwrap()))
    }
}

impl TryFrom<Bytes> for ReceiveExperience {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut r = Reader::new(&value);
        Ok(ReceiveExperience {
            exp_origin: r.u32()?,
            experience: r.i64()?,
            sp_exp: r.u64()?,
            unknown: r.u8()?,
            tail: value.slice(r.pos..),
        })
    }
}

impl From<ReceiveExperience> for Bytes {
    fn from(p: ReceiveExperience) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u32_le(p.exp_origin);
        buf.put_i64_le(p.experience);
        buf.put_u64_le(p.sp_exp);
        buf.put_u8(p.unknown);
        buf.extend_from_slice(&p.tail);
        buf.freeze()
    }
}

// --- Invite / petition popup (0x3080) --------------------------------------

/// `SRTypes.PlayerPetition` — which invite the 0x3080 popup is for.
pub const PETITION_EXCHANGE: u8 = 1;
pub const PETITION_PARTY_CREATION: u8 = 2;
pub const PETITION_PARTY_INVITATION: u8 = 3;
pub const PETITION_RESURRECTION: u8 = 4;
pub const PETITION_GUILD: u8 = 5;
pub const PETITION_UNION: u8 = 6;
pub const PETITION_ACADEMY: u8 = 9;

/// `SRParty.Setup` flags carried by the two party petitions.
pub const PARTY_SETUP_EXP_SHARED: u8 = 1;
pub const PARTY_SETUP_ITEM_SHARED: u8 = 2;
pub const PARTY_SETUP_ANYONE_CAN_INVITE: u8 = 4;

/// The petition body the server pushes to the invitee.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvitePetition {
    /// One of the `PETITION_*` constants.
    pub petition: u8,
    /// UNKNOWN whether this is a spawn id or a petition token
    /// (docs/net-invite-0x3080.md §7) — it is echoed by nothing, so the
    /// server correlates the answer by session either way.
    pub unique_id: u32,
    /// Party petitions only (`PARTY_CREATION`/`PARTY_INVITATION`): the
    /// `PARTY_SETUP_*` flags. Capacity is 8 when `EXP_SHARED` is set, else 4.
    pub setup: Option<u8>,
}

impl InvitePetition {
    pub fn is_party(&self) -> bool {
        matches!(
            self.petition,
            PETITION_PARTY_CREATION | PETITION_PARTY_INVITATION
        )
    }
}

/// The invitee's answer. The wire carries no echoed id or type — the server
/// correlates by session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteResponse {
    /// `01 01`
    Accept,
    /// `01 00`
    Decline,
    /// `02 <reason:u16le>` — the party-specific decline.
    ///
    /// **The reason is per-arm, not a constant.** The two senders the original
    /// uses for a party refusal write different words: `0x2C0C` for a creation
    /// petition (type 2, `PartyCreation`) against `0x2C17` for an invitation
    /// into an existing party (type 3, `PartyInvitation`). Both then emit `02`
    /// plus that word. Use [`PARTY_DECLINE_CREATION`] /
    /// [`PARTY_DECLINE_INVITATION`].
    DeclineParty(u16),
}

/// Reason word for declining a **party creation** (petition type 2): `0x2C0C`.
pub const PARTY_DECLINE_CREATION: u16 = 0x2C0C;
/// Reason word for declining an **invitation into an existing party**
/// (petition type 3): `0x2C17`.
pub const PARTY_DECLINE_INVITATION: u16 = 0x2C17;

/// 0x3080 — the shared invite/petition popup.
///
/// The opcode is **dual-mapped**: server → client it is
/// `SERVER_PLAYER_PETITION_REQUEST` (the popup on the invitee), client →
/// server it is `CLIENT_PLAYER_INVITATION_RESPONSE` (accept/decline). One
/// opcode services party / exchange / guild / academy / resurrection invites,
/// keyed by the leading `type` byte.
///
/// Because the two directions carry different bodies under one opcode, this
/// enum is the codec for both: decoding always yields [`Self::Petition`] and
/// encoding a [`Self::Response`] emits the answer bytes.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum GameInvite {
    /// S→C.
    Petition(InvitePetition),
    /// C→S.
    Response(InviteResponse),
}

impl TryFrom<Bytes> for GameInvite {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut r = Reader::new(&value);
        let petition = r.u8()?;
        let unique_id = r.u32()?;
        let mut invite = InvitePetition {
            petition,
            unique_id,
            setup: None,
        };
        // the setup byte is party-only; tolerate its absence rather than fail
        if invite.is_party() {
            invite.setup = r.u8().ok();
        }
        Ok(GameInvite::Petition(invite))
    }
}

impl From<GameInvite> for Bytes {
    fn from(p: GameInvite) -> Self {
        let mut buf = BytesMut::new();
        match p {
            GameInvite::Response(InviteResponse::Accept) => buf.extend_from_slice(&[0x01, 0x01]),
            GameInvite::Response(InviteResponse::Decline) => buf.extend_from_slice(&[0x01, 0x00]),
            GameInvite::Response(InviteResponse::DeclineParty(reason)) => {
                buf.put_u8(0x02);
                buf.put_u16_le(reason);
            }
            // Round-trip only: the client never sends a petition.
            GameInvite::Petition(invite) => {
                buf.put_u8(invite.petition);
                buf.put_u32_le(invite.unique_id);
                if let Some(setup) = invite.setup {
                    buf.put_u8(setup);
                }
            }
        }
        buf.freeze()
    }
}

/// 0x7081 — ask to exchange with an entity; the server answers the target with
/// a 0x3080 petition. The party (0x7062), guild (0x70F3) and academy (0x7472)
/// funnels carry the same single-`u32` body.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct ExchangeInviteRequest {
    pub unique_id: u32,
}

/// 0x7062 — invite an entity to the party.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyInviteRequest {
    pub unique_id: u32,
}

/// 0x70F3 — invite an entity to the guild.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildInviteRequest {
    pub unique_id: u32,
}

/// 0x7472 — invite an entity to the academy.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct AcademyInviteRequest {
    pub unique_id: u32,
}

/// 0xB081 — inviter-side ack that the exchange petition was raised.
///
/// The tail is **selected by the result byte**, which is why this is not a
/// plain derive: `result == 1` carries the target's `u32 uniqueID`, anything
/// else carries a `u16` error code.
/// Reading both as one fixed 5-byte body made
/// every *refused* invite fail to decode — a 3-byte body is short, not
/// malformed.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum ExchangeInviteResponse {
    /// The petition reached the target; the window opens on the following
    /// 0x3085.
    Accepted { unique_id: u32 },
    /// The server refused. The code is carried verbatim: no source names its
    /// value space, so interpreting it here would be an invented table.
    Refused { error: u16 },
}

impl TryFrom<Bytes> for ExchangeInviteResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let short = || {
            SerializationError::IoError(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "0xB081 body too short",
            ))
        };
        let result = *value.first().ok_or_else(short)?;
        if result == EXCHANGE_INVITE_RAISED {
            let uid = value.get(1..5).ok_or_else(short)?;
            Ok(ExchangeInviteResponse::Accepted {
                unique_id: u32::from_le_bytes(uid.try_into().unwrap()),
            })
        } else {
            let code = value.get(1..3).ok_or_else(short)?;
            Ok(ExchangeInviteResponse::Refused {
                error: u16::from_le_bytes(code.try_into().unwrap()),
            })
        }
    }
}

impl From<ExchangeInviteResponse> for Bytes {
    fn from(p: ExchangeInviteResponse) -> Self {
        let mut buf = bytes::BytesMut::new();
        match p {
            ExchangeInviteResponse::Accepted { unique_id } => {
                bytes::BufMut::put_u8(&mut buf, EXCHANGE_INVITE_RAISED);
                bytes::BufMut::put_u32_le(&mut buf, unique_id);
            }
            ExchangeInviteResponse::Refused { error } => {
                bytes::BufMut::put_u8(&mut buf, EXCHANGE_INVITE_REFUSED);
                bytes::BufMut::put_u16_le(&mut buf, error);
            }
        }
        buf.freeze()
    }
}

/// `result` values of [`ExchangeInviteResponse`]. Only 1 is named by a source;
/// 2 is the family's usual "failed" and is what we write when re-encoding a
/// refusal, never something we require on the wire.
pub const EXCHANGE_INVITE_RAISED: u8 = 1;
pub const EXCHANGE_INVITE_REFUSED: u8 = 2;

// --- Logout (0x7005/0xB005/0x7006/0xB006/0x300A) ---------------------------

/// Logout mode sent in [`LogoutRequest`].
pub const LOGOUT_MODE_EXIT: u8 = 1;
pub const LOGOUT_MODE_RESTART: u8 = 2;
/// Logout error codes (in [`LogoutResponse::error`]).
pub const LOGOUT_ERROR_IN_BATTLE: u16 = 0x801;
pub const LOGOUT_ERROR_IN_TELEPORT: u16 = 0x802;

/// 0x7005 — client → server: begin logout. `mode` is [`LOGOUT_MODE_EXIT`] or
/// [`LOGOUT_MODE_RESTART`].
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct LogoutRequest {
    pub mode: u8,
}

/// 0xB005 — server → client: logout accepted (result 1, with a countdown and
/// the echoed mode) or rejected (result 2, with an error code).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct LogoutResponse {
    pub result: u8,
    #[sro_packet(when = "result == 1")]
    pub countdown: Option<u8>,
    #[sro_packet(when = "result == 1")]
    pub mode: Option<u8>,
    #[sro_packet(when = "result == 2")]
    pub error: Option<u16>,
}

/// 0x7006 — client → server: cancel a pending logout. Empty body.
#[derive(Message, Clone, Debug, Default)]
pub struct LogoutCancelRequest;

/// 0xB006 — server → client: result of a cancel request.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct LogoutCancelResponse {
    pub result: u8,
    #[sro_packet(when = "result == 2")]
    pub error: Option<u16>,
}

/// 0x300A — server → client: the logout countdown elapsed; perform the
/// exit/restart now. Empty body.
#[derive(Message, Clone, Debug, Default)]
pub struct LogoutSuccess;

empty_packet!(LogoutCancelRequest);
empty_packet!(LogoutSuccess);

// --- Group entity spawn/despawn (0x3017 begin / 0x3019 data / 0x3018 end) ----
//
// After the self-spawn stream, the agent server streams every *other* nearby
// entity (remote players, NPCs, monsters, item drops) as a batch: a
// `GroupEntitySpawnBegin` marker (spawn vs despawn + entity count), one
// `GroupEntitySpawnData` packet carrying all `count` records, then a
// `GroupEntitySpawnEnd` marker. (0x3018 is the empty end and 0x3019 the data,
// not the other way round.) The per-record layout is version- and
// itemdata-dependent (a player's equipment list writes an extra byte only for
// equipment items), the same reason `CharacterDataBody` is a raw passthrough, so
// the data packet is carried unparsed and decoded in the client where itemdata
// lives (see `client/src/net/entity_spawn.rs`).

/// [`GroupEntitySpawnBegin::kind`]: this batch adds entities.
pub const GROUP_SPAWN: u8 = 1;
/// [`GroupEntitySpawnBegin::kind`]: this batch removes entities.
pub const GROUP_DESPAWN: u8 = 2;

/// 0x3017 — start of a group spawn/despawn batch. `kind` is [`GROUP_SPAWN`] or
/// [`GROUP_DESPAWN`]; `count` is the number of records in the following
/// [`GroupEntitySpawnData`]. (Some servers append trailing "unknown" fields;
/// they are ignored on decode.)
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct GroupEntitySpawnBegin {
    pub kind: u8,
    pub count: u16,
}

/// 0x3019 — the group spawn/despawn payload: `count` records (per
/// [`GroupEntitySpawnBegin`]). For a spawn each record is a full entity; for a
/// despawn each record is a bare `u32` unique id. Carried unparsed; the client
/// decodes it with itemdata (see `client/src/net/entity_spawn.rs`).
#[derive(Message, Clone, Debug)]
pub struct GroupEntitySpawnData {
    pub raw: Bytes,
}

/// 0x3018 — end of a group spawn/despawn batch. Empty body.
#[derive(Message, Clone, Debug, Default)]
pub struct GroupEntitySpawnEnd;

/// 0x3015 — a single entity entering view outside a group batch (monster
/// respawn, a player walking into range). The body is exactly one
/// [`GroupEntitySpawnData`] spawn record (ref id + type-dependent data), so it
/// is carried unparsed for the same reason: decoding needs the client's
/// itemdata tables.
#[derive(Message, Clone, Debug)]
pub struct SingleEntitySpawn {
    pub raw: Bytes,
}

/// 0x3016 — a single entity leaving view.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct SingleEntityDespawn {
    pub unique_id: u32,
}

impl TryFrom<Bytes> for SingleEntitySpawn {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        Ok(SingleEntitySpawn { raw: value })
    }
}

impl From<SingleEntitySpawn> for Bytes {
    fn from(value: SingleEntitySpawn) -> Self {
        value.raw
    }
}

impl TryFrom<Bytes> for GroupEntitySpawnData {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        Ok(GroupEntitySpawnData { raw: value })
    }
}

impl From<GroupEntitySpawnData> for Bytes {
    fn from(value: GroupEntitySpawnData) -> Self {
        value.raw
    }
}

empty_packet!(GroupEntitySpawnEnd);

#[cfg(test)]
mod test {

    /// The quickslot save is `0x7158` *kind 1* — the leading discriminator is
    /// what separates it from the auto-potion settings on the same opcode, and
    /// dropping it would silently write an auto-potion packet.
    #[test]
    fn quickslot_save_writes_its_kind_discriminator_first() {
        let bytes: Bytes = QuickSlotSaveRequest {
            slot_index: 3,
            content: QuickSlotContent::Item,
            value: 24457,
        }
        .into();

        assert_eq!(bytes[0], 1, "kind 1 = quickslot-bar save");
        assert_eq!(bytes[1], 3, "slot index");
        assert_eq!(bytes[2], QuickSlotContent::Item as u8);
        assert_eq!(&bytes[3..7], &24457u32.to_le_bytes());
        assert_eq!(bytes.len(), 7, "u8 kind + u8 slot + u8 content + u32");

        // round-trips, and a kind-2 body is rejected rather than misread
        let parsed = QuickSlotSaveRequest::try_from(bytes).expect("round-trips");
        assert_eq!(parsed.value, 24457);
        assert!(
            QuickSlotSaveRequest::try_from(Bytes::from_static(&[2, 0, 0, 0, 0, 0, 0])).is_err()
        );
    }
    use super::*;

    #[test]
    fn celestial_position_roundtrips() {
        let packet = CelestialPosition {
            unique_id: 0x0A0B0C0D,
            day: 15,
            hour: 13,
            minute: 42,
        };
        let bytes: Bytes = packet.clone().into();
        // u32 LE + u16 LE + u8 + u8
        assert_eq!(&bytes[..], &[0x0D, 0x0C, 0x0B, 0x0A, 0x0F, 0x00, 13, 42]);
        let decoded: CelestialPosition = bytes.try_into().unwrap();
        assert_eq!(decoded.unique_id, packet.unique_id);
        assert_eq!(decoded.day, 15);
        assert_eq!(decoded.hour, 13);
        assert_eq!(decoded.minute, 42);
    }

    /// `0x34BE` is the real-world server clock packed into one `u32`. The
    /// decoded minute/second land ten minutes apart, across an hour rollover.
    #[test]
    fn server_time_decodes_the_captured_clock_pushes() {
        /// bytes -> (year, month, day, hour, minute, second)
        type ClockCase = (&'static [u8; 4], (u16, u8, u8, u8, u8, u8));
        let cases: [ClockCase; 6] = [
            (b"\x1a\xaa\xd5\x59", (2026, 8, 10, 11, 29, 22)),
            (b"\x1a\xaa\x75\x5a", (2026, 8, 10, 11, 39, 22)),
            (b"\x1a\xaa\x15\x5b", (2026, 8, 10, 11, 49, 22)),
            (b"\x1a\xaa\xb5\x5b", (2026, 8, 10, 11, 59, 22)),
            (b"\x1a\x2a\x96\x58", (2026, 8, 10, 12, 9, 22)),
            (b"\x1a\x2a\x36\x59", (2026, 8, 10, 12, 19, 22)),
        ];

        for (wire, (year, month, day, hour, minute, second)) in cases {
            let bytes = Bytes::copy_from_slice(wire);
            let decoded: ServerTime = bytes.clone().try_into().unwrap();
            assert_eq!(
                (
                    decoded.year(),
                    decoded.month(),
                    decoded.day(),
                    decoded.hour(),
                    decoded.minute(),
                    decoded.second()
                ),
                (year, month, day, hour, minute, second),
                "sample {wire:02x?}"
            );
            let back: Bytes = decoded.into();
            assert_eq!(back, bytes, "round-trip {wire:02x?}");
        }
    }

    #[test]
    fn select_entity_response_roundtrips_and_reads_mob_hp() {
        // mob shape: result=1, unique id, then u8(1) u32(hp) u8 u8.
        let wire = Bytes::from_static(&[1, 0x39, 0x30, 0, 0, 1, 0xA0, 0x0F, 0, 0, 1, 5]);
        let decoded: SelectEntityResponse = wire.clone().try_into().unwrap();
        assert_eq!(decoded.result, 1);
        assert_eq!(decoded.unique_id, 12345);
        assert_eq!(decoded.monster_hp(), Some(4000));
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        // a server that sends hp=0 means "unknown", not "dead".
        let stub = Bytes::from_static(&[1, 1, 0, 0, 0, 1, 0, 0, 0, 0, 1, 5]);
        let decoded: SelectEntityResponse = stub.try_into().unwrap();
        assert_eq!(decoded.monster_hp(), None);

        // failure shape: result=0 + error byte, no unique id.
        let fail = Bytes::from_static(&[0, 0]);
        let decoded: SelectEntityResponse = fail.try_into().unwrap();
        assert_eq!(decoded.result, 0);
        assert_eq!(decoded.unique_id, 0);
        assert_eq!(decoded.monster_hp(), None);
    }

    #[test]
    fn object_action_request_roundtrips() {
        // cast with an entity target: 01 (execute) 04 (cast) id LE 01 uid LE
        let packet = ObjectActionRequest::Execute(ActionCommand::CastSkill {
            ref_skill_id: 0x0102,
            target: ActionTarget::Entity {
                unique_id: 0x0A0B0C0D,
            },
        });
        let bytes: Bytes = packet.clone().into();
        assert_eq!(
            &bytes[..],
            &[1, 4, 0x02, 0x01, 0, 0, 1, 0x0D, 0x0C, 0x0B, 0x0A]
        );
        let decoded: ObjectActionRequest = bytes.try_into().unwrap();
        assert_eq!(decoded, packet);

        // untargeted cast: 01 04 id LE 00
        let packet = ObjectActionRequest::Execute(ActionCommand::CastSkill {
            ref_skill_id: 3,
            target: ActionTarget::None,
        });
        let bytes: Bytes = packet.clone().into();
        assert_eq!(&bytes[..], &[1, 4, 3, 0, 0, 0, 0]);
        let decoded: ObjectActionRequest = bytes.try_into().unwrap();
        assert_eq!(decoded, packet);

        // cancel: bare 02
        let bytes: Bytes = ObjectActionRequest::Cancel.into();
        assert_eq!(&bytes[..], &[2]);
        let decoded: ObjectActionRequest = bytes.try_into().unwrap();
        assert_eq!(decoded, ObjectActionRequest::Cancel);
    }

    #[test]
    fn object_action_response_decodes_captured_shapes() {
        // the four 2-byte bodies that make up 99 % of this ack
        for (wire, expected) in [
            (
                Bytes::from_static(&[1, 0]),
                ObjectActionResponse::Started {
                    code: ACTION_START_CAST,
                },
            ),
            (
                Bytes::from_static(&[1, 1]),
                ObjectActionResponse::Started {
                    code: ACTION_START_ATTACK,
                },
            ),
            (
                Bytes::from_static(&[1, 2]),
                ObjectActionResponse::Started {
                    code: ACTION_START_REJECTED,
                },
            ),
            (
                Bytes::from_static(&[2, 0]),
                ObjectActionResponse::Ended { code: 0 },
            ),
        ] {
            let decoded: ObjectActionResponse = wire.clone().try_into().unwrap();
            assert_eq!(decoded, expected);
            assert_eq!(
                decoded.is_rejected(),
                wire.as_ref() == [1, 2],
                "{wire:?} rejection flag"
            );
            let back: Bytes = decoded.into();
            assert_eq!(back, wire);
        }
    }

    #[test]
    fn object_action_response_unknown_shapes_keep_raw_tail() {
        // over-long or unrecognized phases must land in Unknown and roundtrip
        // untouched. `03 xx 04 40` is not among them: the handler reads it as
        // `Failed { code, error }`, so `03 09 04 40` is typed below, not raw.
        for wire in [
            Bytes::from_static(&[2, 0x04, 0x30]),
            Bytes::from_static(&[1]),
            Bytes::from_static(&[4, 9, 4, 0x40]),
        ] {
            let decoded: ObjectActionResponse = wire.clone().try_into().unwrap();
            assert!(
                matches!(decoded, ObjectActionResponse::Unknown { .. }),
                "{wire:?}"
            );
            let back: Bytes = decoded.into();
            assert_eq!(back, wire);
        }

        // the `03` family varies only in its code byte (`03 xx 04 40`), and
        // every one of them is typed
        let decoded: ObjectActionResponse =
            Bytes::from_static(&[3, 9, 4, 0x40]).try_into().unwrap();
        assert_eq!(
            decoded,
            ObjectActionResponse::Failed {
                code: 9,
                error: 0x4004
            }
        );
    }

    #[test]
    fn teleport_ack_and_game_reset_decode_captured_lines() {
        // real lines of a successful teleport: phase 1 `02 01 00`,
        // phase 2 `01`
        let begin = Bytes::from_static(&[0x02, 0x01, 0x00]);
        let decoded: TeleportResponse = begin.clone().try_into().unwrap();
        assert_eq!(decoded, TeleportResponse::Begin { code: 1 });
        let back: Bytes = decoded.into();
        assert_eq!(back, begin);

        let committed = Bytes::from_static(&[0x01]);
        let decoded: TeleportResponse = committed.clone().try_into().unwrap();
        assert_eq!(decoded, TeleportResponse::Committed);
        let back: Bytes = decoded.into();
        assert_eq!(back, committed);

        // a real game-reset line: destination region 0x61A7
        let reset = Bytes::from_static(&[0xa7, 0x61]);
        let decoded: GameReset = reset.try_into().unwrap();
        assert_eq!(decoded.region, 0x61A7);

        let complete: Bytes = GameResetComplete.into();
        assert!(complete.is_empty());
    }

    /// 0x704F is one byte, and the two gait values are the *same* encoding
    /// 0x30BF uses inbound, so 2/3 rests on both directions agreeing.
    #[test]
    fn character_action_request_is_one_byte_and_shares_the_motion_encoding() {
        let wire: Bytes = CharacterActionRequest::sit_stand().into();
        assert_eq!(&wire[..], &[4u8]);

        let walk: Bytes = CharacterActionRequest::gait(true).into();
        let run: Bytes = CharacterActionRequest::gait(false).into();
        assert_eq!(&walk[..], &[MOTION_STATE_WALK]);
        assert_eq!(&run[..], &[MOTION_STATE_RUN]);

        assert_eq!(
            CharacterActionRequest::try_from(wire).unwrap(),
            CharacterActionRequest { action: 4 }
        );
    }

    /// The emote codes are NOT the slot order: the dispatcher's arms give
    /// 4001 -> 6 and 4002 -> 1, so a `command_id - 4000` shortcut would send
    /// the wrong emote for five of the seven.
    #[test]
    fn emote_request_maps_command_ids_to_their_codes() {
        let greeting = EmoteRequest::for_command(4000).expect("4000 is an emote");
        let wire: Bytes = greeting.into();
        assert_eq!(&wire[..], &[0u8]);

        assert_eq!(EmoteRequest::for_command(4001).unwrap().emote, 6);
        assert_eq!(EmoteRequest::for_command(4002).unwrap().emote, 1);
        assert_eq!(EmoteRequest::for_command(4005).unwrap().emote, 3);
        // the COS charm is not an emote — it is a 0x70C5 pet command
        assert!(EmoteRequest::for_command(5000).is_none());

        // every code is distinct, and 0..=6 is covered exactly once
        let mut codes: Vec<u8> = EMOTE_CODES.iter().map(|(_, code)| *code).collect();
        codes.sort_unstable();
        assert_eq!(codes, (0..=6).collect::<Vec<u8>>());
    }

    /// `Trace` is discriminant 3 of the same 0x7074 envelope Attack and Pickup
    /// use, and its wire form is the dispatcher's `01 03 01 <uid>`.
    #[test]
    fn object_action_request_carries_the_trace_arm() {
        let trace = ObjectActionRequest::Execute(ActionCommand::Trace(ActionTarget::Entity {
            unique_id: 0x0002_98df,
        }));
        let wire: Bytes = trace.clone().into();
        assert_eq!(&wire[..], &[0x01, 0x03, 0x01, 0xdf, 0x98, 0x02, 0x00]);
        assert_eq!(ObjectActionRequest::try_from(wire).unwrap(), trace);
    }

    #[test]
    fn get_up_request_carries_the_option_byte() {
        // 0x3053: one option byte. Present-point (free resurrect) = 2, the
        // only confirmed value.
        let wire: Bytes = GetUpRequest::present_point().into();
        assert_eq!(&wire[..], &[2u8]);
        let decoded: GetUpRequest = wire.try_into().unwrap();
        assert_eq!(decoded, GetUpRequest::present_point());
        assert_eq!(decoded.option, GetUpRequest::PRESENT_POINT);
    }

    /// The town return is the option a dead character always has; its byte is
    /// `1`.
    #[test]
    fn get_up_request_town_return_is_option_one() {
        let wire: Bytes = GetUpRequest::return_to_town().into();
        assert_eq!(&wire[..], &[1u8]);
        let decoded: GetUpRequest = wire.try_into().unwrap();
        assert_eq!(decoded.option, GetUpRequest::RETURN_TO_TOWN);
        assert_ne!(GetUpRequest::RETURN_TO_TOWN, GetUpRequest::PRESENT_POINT);
    }

    #[test]
    fn buff_remove_decodes_captured_line() {
        // a real buff-remove line: count 01, instance 0x38c
        let wire = Bytes::from_static(&[0x01, 0x8c, 0x03, 0x00, 0x00]);
        let decoded: BuffRemove = wire.clone().try_into().unwrap();
        assert_eq!(decoded.buff_instance_ids, vec![0x38c]);
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// A single-removal body cannot separate `{result, id}` from
    /// `{count, [id]}`. The original's handler can: it loops the leading byte.
    /// Pin the multi-element case.
    #[test]
    fn buff_remove_reads_the_leading_byte_as_a_count() {
        let wire = Bytes::from_static(&[
            0x03, 0x8c, 0x03, 0x00, 0x00, 0x27, 0x06, 0x00, 0x00, 0x4c, 0x00, 0x00, 0x00,
        ]);
        let decoded = BuffRemove::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.buff_instance_ids, vec![0x38c, 0x627, 0x4c]);
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
        // A count that outruns the body is a decode error, not a silent short read.
        assert!(BuffRemove::try_from(Bytes::from_static(&[0x02, 0x8c, 0x03, 0x00, 0x00])).is_err());
    }

    /// A refused attack is the three bytes `02 0f 30`. Pinned here because the
    /// whole failure arm used to be a single `u8` (#232).
    #[test]
    fn object_action_update_decodes_the_live_refusal_frame() {
        let wire = Bytes::from_static(&[0x02, 0x0f, 0x30]);
        let decoded: ObjectActionUpdate = wire.clone().try_into().unwrap();
        assert_eq!(decoded, ObjectActionUpdate::Failure { error: 0x300F });
        assert_eq!(
            decoded,
            ObjectActionUpdate::Failure {
                error: ACTION_ERROR_BROKEN_WEAPON
            }
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// The named codes are the family-4 notice ids, not skrillax's small
    /// ordinals: `0x06`/`0x07` would decode as `Unknown`, and every named code
    /// resolves to one of the original's own text keys while an unnamed one
    /// stays a number.
    #[test]
    fn action_error_codes_are_family_four_notice_ids() {
        assert_eq!(ACTION_ERROR_INVALID_TARGET, 0x3006);
        assert_eq!(ACTION_ERROR_INVALID_DISTANCE, 0x3007);
        assert_eq!(
            action_error_text_key(ACTION_ERROR_BROKEN_WEAPON),
            Some("UIIT_SKILL_USE_FAIL_BROKEN_WEAPON")
        );
        assert_eq!(
            action_error_text_key(ACTION_ERROR_INVALID_DISTANCE),
            Some("UIIT_SKILL_USE_FAIL_WRONGDISTANCE")
        );
        // positive control on the same lookup: a code inside the family range
        // that the static read did not decode has no key and must stay a number
        assert_eq!(action_error_text_key(0x3048), None);
        assert_eq!(action_error_text_key(0x06), None);
    }

    #[test]
    fn object_action_update_decodes_live_self_cast() {
        // a real 0xB070 line (self-buff, no target, no damage)
        let wire = Bytes::from_static(&[
            0x01, 0x00, 0x30, 0xC6, 0x98, 0x00, 0x00, 0x90, 0x89, 0x05, 0x00, 0x27, 0x06, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]);
        let decoded: ObjectActionUpdate = wire.clone().try_into().unwrap();
        assert_eq!(
            decoded,
            ObjectActionUpdate::Success {
                unknown: 0x3000,
                skill_id: 0x98C6,
                source: 0x58990,
                instance: 0x627,
                target: 0,
                kind: ActionKind::None,
            }
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        // the matching 0xB071 body: same instance, no target, kind none
        let wire = Bytes::from_static(&[0x01, 0x27, 0x06, 0, 0, 0, 0, 0, 0, 0]);
        let decoded: SkillEnd = wire.clone().try_into().unwrap();
        assert_eq!(
            decoded,
            SkillEnd::Success {
                instance: 0x627,
                target: 0,
                kind: ActionKind::None,
            }
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    #[test]
    fn skill_end_decodes_live_damage_capture() {
        // a real 0xB071 line: the skill cast's damage — killing blow,
        // 151 damage on target 0xc40c
        let wire = Bytes::from_static(&[
            0x01, 0x3B, 0x00, 0x00, 0x00, 0x0C, 0xC4, 0x00, 0x00, 0x01, 0x01, 0x01, 0x0C, 0xC4,
            0x00, 0x00, 0x80, 0x01, 0x97, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]);
        let decoded: SkillEnd = wire.clone().try_into().unwrap();
        assert_eq!(
            decoded,
            SkillEnd::Success {
                instance: 0x3B,
                target: 0xC40C,
                kind: ActionKind::Attack {
                    damage: Some(DamageContent {
                        instance_count: 1,
                        entities: vec![PerEntityDamage {
                            target: 0xC40C,
                            hits: vec![SkillPartDamage::killing_blow(DamageValue {
                                kind: DamageValue::KIND_NORMAL,
                                amount: 151,
                            })],
                        }],
                    }),
                },
            }
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        // junk / truncated bodies stay raw
        for wire in [
            Bytes::from_static(&[0x02, 0x06, 0x30]),
            Bytes::from_static(&[0x01, 0x3B, 0x00]),
        ] {
            let decoded: SkillEnd = wire.clone().try_into().unwrap();
            assert!(matches!(decoded, SkillEnd::Unknown { .. }), "{wire:?}");
            let back: Bytes = decoded.into();
            assert_eq!(back, wire);
        }
    }

    /// Real 0xB070 lines (vSRO 1.188), one critical killing blow and one
    /// ordinary hit, plus a synthetic line carrying an outcome byte that has
    /// never been seen. The reasoning behind "only 1 and 2 exist so far" is in
    /// `docs/combat-math-server-spec.md` §5.
    #[test]
    fn damage_kind_byte_is_preserved_from_captured_lines() {
        // 01 3002 <skill 40> <src> <inst 0x3ad> <tgt> 01 | 01 01 <tgt>
        // 80 02 1c010000 0000 00  → killing blow, kind 2 (critical), 284
        let wire = Bytes::from_static(&[
            0x01, 0x02, 0x30, 0x28, 0x00, 0x00, 0x00, 0x80, 0xab, 0x01, 0x00, 0xad, 0x03, 0x00,
            0x00, 0xa0, 0x60, 0x01, 0x00, 0x01, 0x01, 0x01, 0xa0, 0x60, 0x01, 0x00, 0x80, 0x02,
            0x1c, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]);
        let decoded: ObjectActionUpdate = wire.clone().try_into().unwrap();
        let ObjectActionUpdate::Success {
            kind: ActionKind::Attack {
                damage: Some(damage),
            },
            ..
        } = decoded.clone()
        else {
            panic!("captured attack line did not decode as an attack: {decoded:?}");
        };
        let hit = damage.entities[0].hits[0];
        assert_eq!(
            hit,
            SkillPartDamage::killing_blow(DamageValue {
                kind: DamageValue::KIND_CRITICAL,
                amount: 284,
            })
        );
        assert!(hit.value().unwrap().is_critical());
        assert!(!hit.value().unwrap().is_unknown_kind());
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        // same line with the outcome byte the resolver would call PARRY
        // (never seen): it must survive the round trip as itself, not be
        // rewritten into an ordinary hit
        let mut unknown_kind = wire.to_vec();
        unknown_kind[27] = 0x06;
        let unknown_kind = Bytes::from(unknown_kind);
        let decoded: ObjectActionUpdate = unknown_kind.clone().try_into().unwrap();
        let ObjectActionUpdate::Success {
            kind: ActionKind::Attack {
                damage: Some(damage),
            },
            ..
        } = decoded.clone()
        else {
            panic!("unknown-outcome line did not decode as an attack");
        };
        let value = damage.entities[0].hits[0].value().unwrap();
        assert_eq!(value.kind, 0x06);
        assert!(value.is_unknown_kind());
        assert!(!value.is_critical());
        let back: Bytes = decoded.into();
        assert_eq!(back, unknown_kind);
    }

    /// The per-hit record is a tagged record whose damage is a `u24`
    /// packed behind a state byte, and whose arm is `flags & 0x7F`, not a bit
    /// test.
    #[test]
    fn hit_record_damage_is_a_u24_behind_a_state_byte() {
        // a real 0xB071 line: one hit, state 1, damage 0x5F = 95
        let wire = Bytes::from_static(&[
            0x01, 0xb9, 0x03, 0x00, 0x00, 0x80, 0xab, 0x01, 0x00, 0x01, 0x01, 0x01, 0x80, 0xab,
            0x01, 0x00, 0x00, 0x01, 0x5f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]);
        let decoded: SkillEnd = wire.clone().try_into().unwrap();
        let SkillEnd::Success {
            kind: ActionKind::Attack {
                damage: Some(damage),
            },
            ..
        } = decoded.clone()
        else {
            panic!("captured damage line did not decode as an attack: {decoded:?}");
        };
        assert_eq!(
            damage.entities[0].hits[0],
            SkillPartDamage::hit(DamageValue {
                kind: DamageValue::KIND_NORMAL,
                amount: 95,
            })
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        // the byte after the packed word belongs to the *unnamed* u32, not to
        // the amount: reading a bare u32 for the amount would report
        // 0xFF00005F here. Same line with that byte set.
        let mut with_trailer = wire.to_vec();
        with_trailer[21] = 0xFF;
        let with_trailer = Bytes::from(with_trailer);
        let decoded: SkillEnd = with_trailer.clone().try_into().unwrap();
        let SkillEnd::Success {
            kind: ActionKind::Attack {
                damage: Some(damage),
            },
            ..
        } = decoded.clone()
        else {
            panic!("trailer variant did not decode as an attack");
        };
        assert_eq!(
            damage.entities[0].hits[0],
            SkillPartDamage {
                killing_blow: false,
                effect: HitEffect::Damage {
                    value: DamageValue {
                        kind: DamageValue::KIND_NORMAL,
                        amount: 95,
                    },
                    unknown: 0xFF,
                },
            }
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, with_trailer);
    }

    /// Arms 4 and 5 are 23-byte records; a `flag & 0x08` bit test reads them
    /// as 9 and desynchronises everything after them.
    #[test]
    fn hit_record_arms_4_and_5_carry_a_position_tail() {
        // the 0xB071 line above with its single hit rewritten to arm 4
        // (killing blow) + a position tail, and a second 9-byte arm-0 hit
        // appended: a short read of the first record would swallow the second.
        let mut wire = vec![
            0x01, 0xb9, 0x03, 0x00, 0x00, 0x80, 0xab, 0x01, 0x00, 0x01, 0x02, 0x01, 0x80, 0xab,
            0x01, 0x00,
        ];
        wire.extend_from_slice(&[0x84, 0x01, 0x5f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        wire.extend_from_slice(&[0x2f, 0x00]); // region
        wire.extend_from_slice(&(-125i32).to_le_bytes());
        wire.extend_from_slice(&300i32.to_le_bytes());
        wire.extend_from_slice(&17i32.to_le_bytes());
        wire.extend_from_slice(&[0x00, 0x02, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        let wire = Bytes::from(wire);

        let decoded: SkillEnd = wire.clone().try_into().unwrap();
        let SkillEnd::Success {
            kind: ActionKind::Attack {
                damage: Some(damage),
            },
            ..
        } = decoded.clone()
        else {
            panic!("arm-4 line did not decode as an attack: {decoded:?}");
        };
        assert_eq!(
            damage.entities[0].hits,
            vec![
                SkillPartDamage {
                    killing_blow: true,
                    effect: HitEffect::Displaced {
                        arm: 4,
                        value: DamageValue {
                            kind: DamageValue::KIND_NORMAL,
                            amount: 95,
                        },
                        unknown: 0,
                        pos: HitPosition {
                            region: 0x2f,
                            x: -125,
                            y: 300,
                            z: 17,
                        },
                    },
                },
                SkillPartDamage::hit(DamageValue {
                    kind: DamageValue::KIND_CRITICAL,
                    amount: 44,
                }),
            ]
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        // arm 8 (and every other unlisted arm) is the flag byte alone
        let abort = Bytes::from_static(&[
            0x01, 0xb9, 0x03, 0x00, 0x00, 0x80, 0xab, 0x01, 0x00, 0x01, 0x01, 0x01, 0x80, 0xab,
            0x01, 0x00, 0x08,
        ]);
        let decoded: SkillEnd = abort.clone().try_into().unwrap();
        let SkillEnd::Success {
            kind: ActionKind::Attack {
                damage: Some(damage),
            },
            ..
        } = decoded.clone()
        else {
            panic!("arm-8 line did not decode as an attack");
        };
        assert_eq!(
            damage.entities[0].hits[0],
            SkillPartDamage {
                killing_blow: false,
                effect: HitEffect::NoPayload { arm: 8 },
            }
        );
        assert!(damage.entities[0].hits[0].value().is_none());
        assert!(!damage.entities[0].hits[0].is_avoided());
        let back: Bytes = decoded.into();
        assert_eq!(back, abort);
    }

    /// A real 0xB070: a monster's two-instance skill on the local character
    /// where instance 1 landed 6 damage and instance 2 was **avoided** — the
    /// arm the client shows as BLOCK. See [`HIT_ARM_AVOIDED`] for why the
    /// arm is a defender outcome and not a filler.
    #[test]
    fn a_captured_avoided_hit_decodes_as_arm_2() {
        let line = Bytes::from_static(&[
            0x01, 0x02, 0x30, 0xc3, 0x00, 0x00, 0x00, 0xa7, 0x62, 0x02, 0x00, 0xa2, 0x1b, 0x00,
            0x00, 0xc7, 0x40, 0x03, 0x00, 0x01, 0x02, 0x01, 0xc7, 0x40, 0x03, 0x00, 0x00, 0x01,
            0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
        ]);
        let decoded: ObjectActionUpdate = line.clone().try_into().unwrap();
        let ObjectActionUpdate::Success {
            kind: ActionKind::Attack {
                damage: Some(damage),
            },
            ..
        } = decoded.clone()
        else {
            panic!("captured line did not decode as an attack");
        };
        assert_eq!(damage.instance_count, 2);
        let hits = &damage.entities[0].hits;
        // instance 1: an ordinary 6-damage hit
        assert_eq!(hits[0].value().unwrap().amount, 6);
        assert!(!hits[0].is_avoided());
        // instance 2: avoided — no damage word on the wire at all
        assert_eq!(hits[1].effect, HitEffect::NoPayload { arm: 2 });
        assert!(hits[1].is_avoided());
        assert!(hits[1].value().is_none());
        let back: Bytes = decoded.into();
        assert_eq!(back, line);
    }

    #[test]
    fn object_action_update_attack_damage_roundtrips() {
        // synthetic basic-attack swing: two damage
        // instances on one target, a critical hit + a killing blow.
        let packet = ObjectActionUpdate::Success {
            unknown: 0x3002,
            skill_id: 70,
            source: 0x58990,
            instance: 0x629,
            target: 0x9001,
            kind: ActionKind::Attack {
                damage: Some(DamageContent {
                    instance_count: 2,
                    entities: vec![PerEntityDamage {
                        target: 0x9001,
                        hits: vec![
                            SkillPartDamage::hit(DamageValue {
                                kind: DamageValue::KIND_CRITICAL,
                                amount: 123,
                            }),
                            SkillPartDamage::killing_blow(DamageValue {
                                kind: DamageValue::KIND_NORMAL,
                                amount: 45,
                            }),
                        ],
                    }],
                }),
            },
        };
        let bytes: Bytes = packet.clone().into();
        let decoded: ObjectActionUpdate = bytes.try_into().unwrap();
        assert_eq!(decoded, packet);

        // swing without a damage block (whiff) and a failure code
        let whiff = ObjectActionUpdate::Success {
            unknown: 0x3002,
            skill_id: 70,
            source: 1,
            instance: 2,
            target: 3,
            kind: ActionKind::Attack { damage: None },
        };
        let bytes: Bytes = whiff.clone().into();
        let decoded: ObjectActionUpdate = bytes.try_into().unwrap();
        assert_eq!(decoded, whiff);

        // the failure tail is a u16, not a u8; both bodies below are real
        // failure bodies.
        for (wire, error) in [([2u8, 0x06, 0x30], 0x3006u16), ([2, 0x10, 0x30], 0x3010)] {
            let bytes = Bytes::copy_from_slice(&wire);
            let decoded: ObjectActionUpdate = bytes.clone().try_into().unwrap();
            assert_eq!(decoded, ObjectActionUpdate::Failure { error });
            let back: Bytes = decoded.into();
            assert_eq!(back, bytes);
        }

        // A one-byte tail is the wrong shape and must stay raw rather than
        // be read as a truncated error.
        let short = Bytes::from_static(&[2, 0x07]);
        let decoded: ObjectActionUpdate = short.try_into().unwrap();
        assert!(matches!(
            decoded,
            ObjectActionUpdate::Unknown { result: 2, .. }
        ));
    }

    /// `0xB074 result=3` is `code u8 + error u16` — the handler reads the
    /// same code byte as results 1/2 and then a `u16` for its message box.
    /// The fixture is the real refusal body.
    #[test]
    fn object_action_response_decodes_the_captured_refusal() {
        let wire = Bytes::from_static(&[0x03, 0x00, 0x04, 0x40]);
        let decoded: ObjectActionResponse = wire.clone().try_into().unwrap();
        assert_eq!(
            decoded,
            ObjectActionResponse::Failed {
                code: 0,
                error: 0x4004
            }
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        // both modelled shapes decode
        let started: ObjectActionResponse = Bytes::from_static(&[1, 1]).try_into().unwrap();
        assert_eq!(started, ObjectActionResponse::Started { code: 1 });
        let ended: ObjectActionResponse = Bytes::from_static(&[2, 0]).try_into().unwrap();
        assert_eq!(ended, ObjectActionResponse::Ended { code: 0 });

        // a result=3 body of the wrong length stays raw
        let short: ObjectActionResponse = Bytes::from_static(&[3, 0, 4]).try_into().unwrap();
        assert!(matches!(
            short,
            ObjectActionResponse::Unknown { result: 3, .. }
        ));
    }

    #[test]
    fn buff_add_decodes_live_capture() {
        // a real buff-add line: join-time auto-buff, skill 39110
        let wire = Bytes::from_static(&[
            0x90, 0x89, 0x05, 0x00, 0xC6, 0x98, 0x00, 0x00, 0x27, 0x06, 0x00, 0x00,
        ]);
        let decoded = BuffAdd::try_from(wire.clone()).unwrap();
        assert_eq!(
            decoded,
            BuffAdd {
                unique_id: 0x058990,
                ref_skill_id: 39110,
                buff_instance_id: 0x627,
            }
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        // the matching 0xB072 drops that same instance, as a one-element list
        let wire = Bytes::from_static(&[0x01, 0x27, 0x06, 0x00, 0x00]);
        let decoded = BuffRemove::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.buff_instance_ids, vec![0x627]);
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    #[test]
    fn learn_acks_decode_live_captures() {
        // a real 0xB0A1 line: skill 3 (SWORD_SMASH_A_01) learned
        let wire = Bytes::from_static(&[0x01, 0x03, 0x00, 0x00, 0x00]);
        let decoded: SkillLearnResponse = wire.clone().try_into().unwrap();
        assert_eq!(decoded, SkillLearnResponse::Success { ref_skill_id: 3 });
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        // a real 0xB0A2 line: Bicheon (257) raised to level 5
        let wire = Bytes::from_static(&[0x01, 0x01, 0x01, 0x00, 0x00, 0x05]);
        let decoded: MasteryLearnResponse = wire.clone().try_into().unwrap();
        assert_eq!(
            decoded,
            MasteryLearnResponse::Success {
                mastery_id: 257,
                new_level: 5,
            }
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    #[test]
    fn learn_acks_keep_unexpected_shapes_raw() {
        // assumed error shape decodes; truncated/overlong bodies stay raw
        let decoded: SkillLearnResponse =
            Bytes::from_static(&[0x02, 0x34, 0x12]).try_into().unwrap();
        assert_eq!(decoded, SkillLearnResponse::Failure(0x1234));
        for wire in [
            Bytes::from_static(&[0x01, 0x03, 0x00]),
            Bytes::from_static(&[0x01, 0x03, 0x00, 0x00, 0x00, 0xFF]),
            Bytes::from_static(&[0x03, 0x01]),
        ] {
            let decoded: SkillLearnResponse = wire.clone().try_into().unwrap();
            assert!(
                matches!(decoded, SkillLearnResponse::Unknown { .. }),
                "{wire:?}"
            );
            let back: Bytes = decoded.into();
            assert_eq!(back, wire);
        }
        let decoded: MasteryLearnResponse = Bytes::from_static(&[0x01, 0x01, 0x01, 0x00, 0x00])
            .try_into()
            .unwrap();
        assert!(matches!(decoded, MasteryLearnResponse::Unknown { .. }));
    }

    #[test]
    fn gm_command_invisible_roundtrips() {
        // 0x7010 body = 2-byte LE sub-command; /invisible = 0x000E.
        let bytes: Bytes = GmCommand::Invisible.into();
        assert_eq!(&bytes[..], &[0x0E, 0x00]);
        assert_eq!(GmCommand::try_from(bytes).unwrap(), GmCommand::Invisible);

        let bytes: Bytes = GmCommand::Invincible.into();
        assert_eq!(&bytes[..], &[0x0F, 0x00]);

        // an unmodelled sub-command survives as raw args and round-trips, so
        // nothing is lost — `0b 00 04 9c 02 00` is the frame that made this arm
        // necessary.
        let wire = Bytes::from_static(&[0x0B, 0x00, 0x04, 0x9C, 0x02, 0x00]);
        let decoded = GmCommand::try_from(wire.clone()).unwrap();
        assert_eq!(
            decoded,
            GmCommand::Other {
                code: 0x0B,
                args: Bytes::from_static(&[0x04, 0x9C, 0x02, 0x00]),
            }
        );
        assert_eq!(Bytes::from(decoded), wire);
    }

    /// `0x7010` splits by sub-command: 6 is the monster spawner and carries
    /// *two* trailing bytes, 7 is the item maker and carries one. The bodies
    /// and their acks below are real frames.
    #[test]
    fn gm_spawn_and_make_item_match_their_bodies() {
        // 06 00 8d 07 00 00 01 01 -> ack 01 06 00 (success)
        let wire = Bytes::from_static(&[0x06, 0x00, 0x8D, 0x07, 0x00, 0x00, 0x01, 0x01]);
        let decoded = GmCommand::try_from(wire.clone()).unwrap();
        assert_eq!(
            decoded,
            GmCommand::LoadMonster {
                ref_id: 1933,
                count: 1,
                rarity: 1,
            }
        );
        assert_eq!(Bytes::from(decoded), wire);

        // 07 00 06 00 00 00 01 -> ack 01 07 00 (success)
        let wire = Bytes::from_static(&[0x07, 0x00, 0x06, 0x00, 0x00, 0x00, 0x01]);
        let decoded = GmCommand::try_from(wire.clone()).unwrap();
        assert_eq!(
            decoded,
            GmCommand::MakeItem {
                ref_id: 6,
                value: 1,
            }
        );
        assert_eq!(Bytes::from(decoded), wire);
    }

    /// Pins the sub-id: `MakeItem` is 0x07, and 0x06 is **`LoadMonster`**. The
    /// bytes below are the real `/Makeitem ITEM_EU_STAFF_11_SET_A_RARE 255`
    /// (ref 25627, equipment, so 255 is not clamped).
    #[test]
    fn gm_make_item_is_sub_command_seven() {
        let bytes: Bytes = GmCommand::MakeItem {
            ref_id: 25627,
            value: 255,
        }
        .into();
        assert_eq!(&bytes[..], &[0x07, 0x00, 0x1B, 0x64, 0x00, 0x00, 0xFF]);
        assert_eq!(
            GmCommand::try_from(bytes).unwrap(),
            GmCommand::MakeItem {
                ref_id: 25627,
                value: 255,
            }
        );
    }

    /// 0x06 is LoadMonster, and its body is 8 bytes — one more than MakeItem's.
    #[test]
    fn gm_load_monster_owns_sub_command_six() {
        let command = GmCommand::LoadMonster {
            ref_id: 1907,
            count: 3,
            rarity: 1,
        };
        let bytes: Bytes = command.clone().into();
        assert_eq!(
            &bytes[..],
            &[0x06, 0x00, 0x73, 0x07, 0x00, 0x00, 0x03, 0x01]
        );
        assert_eq!(bytes.len(), 8);
        assert_eq!(GmCommand::try_from(bytes).unwrap(), command);
    }

    /// `Zoe` and `Zoe2` are two *commands* over one sub-id: `Zoe2` is a
    /// client-side batching wrapper, so on the wire there is nothing to tell
    /// them apart. 34 recovered builders, 33 distinct sub-ids.
    #[test]
    fn gm_zoe_is_sub_command_twelve() {
        let command = GmCommand::Zoe {
            ref_id: 1907,
            count: 200,
        };
        let bytes: Bytes = command.clone().into();
        assert_eq!(&bytes[..], &[0x0C, 0x00, 0x73, 0x07, 0x00, 0x00, 0xC8]);
        assert_eq!(bytes.len(), 7);
        assert_eq!(GmCommand::try_from(bytes).unwrap(), command);
    }

    /// The u16 after `result` is an **echo of the sub-command**, read on both
    /// the ok and fail arms — not an error code.
    #[test]
    fn gm_response_decodes_the_command_echo() {
        // a real 0xB010 body: ok, echoing /invisible
        let captured = Bytes::from_static(&[0x01, 0x0E, 0x00]);
        let decoded: GmResponse = captured.clone().try_into().unwrap();
        assert!(decoded.is_success());
        assert_eq!(decoded.gm_command_id, 0x000E);
        assert!(decoded.tail.is_empty());
        assert_eq!(Bytes::from(decoded), captured);

        // a refused /makeitem is the same shape with result 2 — which is what
        // an unprivileged account is expected to answer
        let refused = Bytes::from_static(&[0x02, 0x07, 0x00]);
        let decoded: GmResponse = refused.clone().try_into().unwrap();
        assert!(!decoded.is_success());
        assert_eq!(decoded.result, GM_RESULT_FAIL);
        assert_eq!(decoded.gm_command_id, 0x0007);
        assert_eq!(Bytes::from(decoded), refused);

        // a per-command payload after the echo is kept raw
        let with_tail = Bytes::from_static(&[0x01, 0x01, 0x00, 0x68, 0x69]);
        let decoded: GmResponse = with_tail.clone().try_into().unwrap();
        assert_eq!(decoded.gm_command_id, 0x0001);
        assert_eq!(decoded.tail.len(), 2);
        assert_eq!(Bytes::from(decoded), with_tail);

        // `result` alone is legal: the original reads nothing further for a
        // result it does not branch on
        let bare: GmResponse = Bytes::from_static(&[0x09]).try_into().unwrap();
        assert_eq!(bare.gm_command_id, 0);
    }

    #[test]
    fn entity_state_update_body_invisibility() {
        // kind 4 (body), value 4 = GM invisible
        let inv = EntityStateUpdate {
            unique_id: 1,
            kind: STATE_KIND_BODY,
            value: BODY_STATE_GM_INVISIBLE,
        };
        assert_eq!(inv.body_invisibility(), Some(true));
        // kind 4, value 0 = none (visible)
        let vis = EntityStateUpdate {
            unique_id: 1,
            kind: STATE_KIND_BODY,
            value: BODY_STATE_NONE,
        };
        assert_eq!(vis.body_invisibility(), Some(false));
        // non-body update
        let life = EntityStateUpdate {
            unique_id: 1,
            kind: STATE_KIND_LIFE,
            value: LIFE_STATE_DEAD,
        };
        assert_eq!(life.body_invisibility(), None);
        // ...and the raw value, which is what drawing somebody ELSE needs
        assert_eq!(inv.body_state_value(), Some(BODY_STATE_GM_INVISIBLE));
        assert_eq!(vis.body_state_value(), Some(BODY_STATE_NONE));
        assert_eq!(life.body_state_value(), None);
    }

    /// GM invisibility and stealth are not the same secret, which is why the
    /// render needs the value and not `body_state_is_invisible`'s yes/no: a GM
    /// is meant to see another GM's ghost, and nobody is meant to see a
    /// stealthed player.
    #[test]
    fn only_gm_invisibility_is_a_ghost_and_only_for_a_gm() {
        use HiddenRender::*;
        assert_eq!(hidden_render(BODY_STATE_GM_INVISIBLE, true), Ghost);
        assert_eq!(hidden_render(BODY_STATE_GM_INVISIBLE, false), Hidden);

        // being a GM buys no sight of stealth or player invisibility
        for state in [BODY_STATE_STEALTH, BODY_STATE_INVISIBLE] {
            assert_eq!(hidden_render(state, true), Hidden);
            assert_eq!(hidden_render(state, false), Hidden);
        }

        // everything else draws normally, GM or not — including the two states
        // that are about damage rather than sight
        for state in [
            BODY_STATE_NONE,
            BODY_STATE_UNTOUCHABLE,
            BODY_STATE_GM_INVINCIBLE,
        ] {
            assert_eq!(hidden_render(state, true), Visible);
            assert_eq!(hidden_render(state, false), Visible);
        }
    }

    /// The three "you are hidden" values stay grouped for your OWN body, where
    /// they really do all mean the same thing. Pins that the finer split did
    /// not quietly change the coarse one.
    #[test]
    fn your_own_body_still_treats_all_three_alike() {
        for state in [
            BODY_STATE_GM_INVISIBLE,
            BODY_STATE_STEALTH,
            BODY_STATE_INVISIBLE,
        ] {
            assert!(body_state_is_invisible(state));
        }
        for state in [
            BODY_STATE_NONE,
            BODY_STATE_UNTOUCHABLE,
            BODY_STATE_GM_INVINCIBLE,
        ] {
            assert!(!body_state_is_invisible(state));
        }
    }

    /// 0x3057's flag is a BITMASK. Every body below is a real 0x3057 line.
    /// See the [`EntityBarsUpdate`] doc for what settles it: `flag=0x04`
    /// appears, and its uid is a monster whose trailing u32 tracks a burn.
    ///
    /// Note flags 3 and 5 are byte-identical in LENGTH (two u32s), so length
    /// can never distinguish them — only meaning can.
    #[test]
    fn entity_bars_update_flag_is_a_bitmask() {
        // flag=1 HP only: monster 0x1a8b5 regenerating to 350
        let wire =
            Bytes::from_static(&[0xB5, 0xA8, 0x01, 0x00, 0x10, 0x00, 0x01, 0x5E, 0x01, 0, 0]);
        let decoded = EntityBarsUpdate::try_from(wire).unwrap();
        assert_eq!((decoded.hp, decoded.mp), (Some(350), None));
        assert_eq!(decoded.bad_status, None);

        // flag=2 MP only: the local player at 1039 MP
        let wire =
            Bytes::from_static(&[0x80, 0xAB, 0x01, 0x00, 0x10, 0x00, 0x02, 0x0F, 0x04, 0, 0]);
        let decoded = EntityBarsUpdate::try_from(wire).unwrap();
        assert_eq!((decoded.hp, decoded.mp), (None, Some(1039)));

        // flag=3 = HP|MP: both present, MP after HP
        let wire = Bytes::from_static(&[
            0xB5, 0xA8, 0x01, 0x00, 0x10, 0x00, 0x03, 0xCE, 0, 0, 0, 0xEF, 0x02, 0, 0,
        ]);
        let decoded = EntityBarsUpdate::try_from(wire).unwrap();
        assert_eq!((decoded.hp, decoded.mp), (Some(206), Some(751)));

        // flag=5 = HP|BAD_STATUS: the trailing u32 is the ailment mask, NOT
        // MP. A healthy monster's is 0.
        let wire = Bytes::from_static(&[
            0x30, 0x61, 0x01, 0x00, 0x01, 0x00, 0x05, 0xBA, 0, 0, 0, 0, 0, 0, 0,
        ]);
        let decoded = EntityBarsUpdate::try_from(wire).unwrap();
        assert_eq!(decoded.unique_id, 0x16130);
        assert_eq!((decoded.hp, decoded.mp), (Some(186), None));
        assert_eq!(decoded.bad_status(), Some(BadStatus(0)));
        assert!(decoded.bad_status().unwrap().is_empty());
    }

    /// The body that settles it: a monster catching fire. 11 bytes,
    /// `flag=0x04`, mask `0x8` = Burn, and — because bit 3 is NOT in
    /// `BAD_STATUS_LEVELED` — no trailing level byte, which confirms the level
    /// rule.
    #[test]
    fn a_burning_monster_decodes_as_burn_not_as_mp() {
        let wire = Bytes::from_static(&[0xB1, 0x64, 0x02, 0x00, 0x03, 0x01, 0x04, 0x08, 0, 0, 0]);
        let decoded = EntityBarsUpdate::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.unique_id, 0x000264B1);
        assert_eq!(decoded.hp, None);
        assert_eq!(decoded.mp, None, "the mask must not be read as MP");
        let status = decoded.bad_status().expect("bad-status block");
        assert!(status.has(Ailment::Burn));
        assert_eq!(status.ailments().collect::<Vec<_>>(), vec![Ailment::Burn]);
        assert!(
            decoded.bad_status_levels.is_empty(),
            "Burn carries no level byte"
        );
        // and it round-trips byte-for-byte
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// A level-carrying bit pulls one `u8` per set `BAD_STATUS_LEVELED` bit.
    /// Synthetic — no real body with such a bit set is known, so this pins the
    /// described structure rather than a real one.
    #[test]
    fn leveled_bad_status_bits_pull_one_level_byte_each() {
        // Stun (bit 14) and Bleed (bit 11) are both in the leveled mask; Burn
        // (bit 3) is not, so a mask of all three yields exactly two levels.
        let mask = Ailment::Stun.bit() | Ailment::Bleed.bit() | Ailment::Burn.bit();
        assert_eq!((mask & BAD_STATUS_LEVELED).count_ones(), 2);
        let mut body = vec![0xB1, 0x64, 0x02, 0x00, 0x03, 0x01, 0x04];
        body.extend_from_slice(&mask.to_le_bytes());
        body.extend_from_slice(&[7, 9]);
        let wire = Bytes::from(body);
        let decoded = EntityBarsUpdate::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.bad_status_levels, vec![7, 9]);
        assert_eq!(
            decoded.bad_status().unwrap().ailments().collect::<Vec<_>>(),
            vec![Ailment::Burn, Ailment::Bleed, Ailment::Stun],
            "ailments come back in bit order"
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// The level-less bits are exactly the six elemental/DoT states plus
    /// Petrify. That partition is what corroborates the SPEC bit ORDER, so if
    /// either the mask constant or the enum order drifts, this catches it.
    #[test]
    fn the_leveled_mask_matches_the_spec_bit_order() {
        let level_less: Vec<Ailment> = Ailment::ALL
            .into_iter()
            .filter(|a| a.bit() & BAD_STATUS_LEVELED == 0)
            .collect();
        assert_eq!(
            level_less,
            vec![
                Ailment::Freezing,
                Ailment::Frostbite,
                Ailment::ElectricShock,
                Ailment::Burn,
                Ailment::Poison,
                Ailment::Zombie,
                Ailment::Petrify,
            ]
        );
    }

    #[test]
    fn motion_updates_name_the_gait() {
        // Real 0x30BF lines: uid 0x1ab9c starts walking, uid 0x16130 starts
        // running (kind 1, values 2 and 3).
        let walk = Bytes::from_static(&[0x9c, 0xab, 0x01, 0x00, 0x01, 0x02]);
        let decoded: EntityStateUpdate = walk.try_into().unwrap();
        assert_eq!(decoded.unique_id, 0x1ab9c);
        assert_eq!(decoded.motion_walking(), Some(true));

        let run = Bytes::from_static(&[0x30, 0x61, 0x01, 0x00, 0x01, 0x03]);
        let decoded: EntityStateUpdate = run.try_into().unwrap();
        assert_eq!(decoded.motion_walking(), Some(false));

        // A life-state update is not a gait, and an unknown motion value
        // stays unanswered rather than being guessed into a gait.
        let dead = Bytes::from_static(&[0x30, 0x61, 0x01, 0x00, 0x00, 0x02]);
        let decoded: EntityStateUpdate = dead.try_into().unwrap();
        assert_eq!(decoded.motion_walking(), None);
        let unknown = Bytes::from_static(&[0x30, 0x61, 0x01, 0x00, 0x01, 0x09]);
        let decoded: EntityStateUpdate = unknown.try_into().unwrap();
        assert_eq!(decoded.motion_walking(), None);
    }

    #[test]
    fn entity_state_update_decodes_live_captures() {
        // live 0x30bf lines: monster 0x13656 running, then dying
        let run = Bytes::from_static(&[0x56, 0x36, 0x01, 0x00, 0x01, 0x03]);
        let decoded: EntityStateUpdate = run.try_into().unwrap();
        assert_eq!(decoded.unique_id, 0x13656);
        assert_eq!(decoded.kind, STATE_KIND_MOTION);
        assert!(!decoded.is_death());

        let dead = Bytes::from_static(&[0x56, 0x36, 0x01, 0x00, 0x00, 0x02]);
        let decoded: EntityStateUpdate = dead.try_into().unwrap();
        assert!(decoded.is_death());
        assert!(!decoded.is_revive());

        // life → alive (revive): kind 0, value 1
        let alive = Bytes::from_static(&[0x56, 0x36, 0x01, 0x00, 0x00, 0x01]);
        let decoded: EntityStateUpdate = alive.try_into().unwrap();
        assert!(decoded.is_revive());
        assert!(!decoded.is_death());

        // live 0x3054 line: bare uid of the leveling entity
        let levelup = Bytes::from_static(&[0x56, 0xAF, 0x05, 0x00]);
        let decoded: EntityLevelUp = levelup.try_into().unwrap();
        assert_eq!(decoded.unique_id, 0x5AF56);
    }

    /// 0xB024 is a 6-byte body: u32 uid then u16 angle, exactly what the
    /// original's parser reads before it stops.
    ///
    /// This fixture is built from that parser layout rather than from real
    /// bytes — the widths and their order are what it pins.
    #[test]
    fn movement_angle_decodes_the_parser_layout() {
        let body = Bytes::from_static(&[0x56, 0xAF, 0x05, 0x00, 0x00, 0x40]);
        let decoded: MovementAngleResponse = body.try_into().unwrap();
        assert_eq!(decoded.unique_id, 0x5AF56);
        // 0x4000 = a quarter turn in the 0..=u16::MAX -> 0..2pi encoding
        assert_eq!(decoded.angle, 0x4000);
    }

    /// The uid must not swallow the angle's low byte: a 6-byte body split
    /// 4+2, not 2+4 or 5+1.
    #[test]
    fn movement_angle_field_widths_do_not_overlap() {
        let body = Bytes::from_static(&[0xFF, 0xFF, 0xFF, 0xFF, 0x34, 0x12]);
        let decoded: MovementAngleResponse = body.try_into().unwrap();
        assert_eq!(decoded.unique_id, u32::MAX);
        assert_eq!(decoded.angle, 0x1234);
    }

    /// 0x3080 S->C: `{type, uid}`, plus a party-only `setup` byte. The
    /// discriminator is `SRTypes.PlayerPetition`.
    #[test]
    fn game_invite_decodes_a_petition() {
        // exchange (1), uid 0x0001_60AA, no setup byte
        let body = Bytes::from_static(&[0x01, 0xAA, 0x60, 0x01, 0x00]);
        let GameInvite::Petition(p) = GameInvite::try_from(body).unwrap() else {
            panic!("decoding always yields a petition")
        };
        assert_eq!(p.petition, PETITION_EXCHANGE);
        assert_eq!(p.unique_id, 0x0001_60AA);
        assert_eq!(p.setup, None, "only party petitions carry setup");
        assert!(!p.is_party());
    }

    /// The party arms carry the extra `setup` byte; capacity is 8 when
    /// EXP_SHARED is set, else 4.
    #[test]
    fn a_party_petition_carries_its_setup_flags() {
        let body = Bytes::from_static(&[0x03, 0x01, 0x00, 0x00, 0x00, 0x05]);
        let GameInvite::Petition(p) = GameInvite::try_from(body).unwrap() else {
            panic!("petition")
        };
        assert_eq!(p.petition, PETITION_PARTY_INVITATION);
        assert!(p.is_party());
        let setup = p.setup.expect("party petitions carry setup");
        assert_eq!(setup & PARTY_SETUP_EXP_SHARED, PARTY_SETUP_EXP_SHARED);
        assert_eq!(
            setup & PARTY_SETUP_ANYONE_CAN_INVITE,
            PARTY_SETUP_ANYONE_CAN_INVITE
        );
        assert_eq!(setup & PARTY_SETUP_ITEM_SHARED, 0);
    }

    /// A truncated party tail must not fail the whole decode - the petition
    /// type and uid are still usable.
    #[test]
    fn a_party_petition_without_its_setup_byte_still_decodes() {
        let body = Bytes::from_static(&[0x02, 0x01, 0x00, 0x00, 0x00]);
        let GameInvite::Petition(p) = GameInvite::try_from(body).unwrap() else {
            panic!("petition")
        };
        assert_eq!(p.petition, PETITION_PARTY_CREATION);
        assert_eq!(p.setup, None);
    }

    /// C->S: the answer carries no echoed id or type; the server correlates by
    /// session. Party decline has its own three-byte form.
    #[test]
    fn invite_responses_serialize_to_the_original_bytes() {
        let accept: Bytes = GameInvite::Response(InviteResponse::Accept).into();
        assert_eq!(&accept[..], &[0x01, 0x01]);
        let decline: Bytes = GameInvite::Response(InviteResponse::Decline).into();
        assert_eq!(&decline[..], &[0x01, 0x00]);
        let party: Bytes =
            GameInvite::Response(InviteResponse::DeclineParty(PARTY_DECLINE_CREATION)).into();
        assert_eq!(&party[..], &[0x02, 0x0C, 0x2C]);
    }

    /// The two party arms decline with **different** reason words — the whole
    /// point of parameterising `DeclineParty`. Nail both down so a future
    /// "simplification" back to one constant fails here instead of on a live
    /// server: `0x2C0C` is what the original writes for a party *creation*
    /// (type 2), `0x2C17` what it writes for an *invitation* into an existing
    /// party (type 3).
    #[test]
    fn the_two_party_arms_decline_with_different_reasons() {
        assert_ne!(PARTY_DECLINE_CREATION, PARTY_DECLINE_INVITATION);
        let creation: Bytes =
            GameInvite::Response(InviteResponse::DeclineParty(PARTY_DECLINE_CREATION)).into();
        assert_eq!(&creation[..], &[0x02, 0x0C, 0x2C]);
        let invitation: Bytes =
            GameInvite::Response(InviteResponse::DeclineParty(PARTY_DECLINE_INVITATION)).into();
        assert_eq!(&invitation[..], &[0x02, 0x17, 0x2C]);
    }

    /// The four funnel requests are a bare target uid.
    #[test]
    fn the_invite_funnels_are_a_bare_uid() {
        let uid = 0x0001_60AA;
        let party: Bytes = PartyInviteRequest { unique_id: uid }.into();
        assert_eq!(&party[..], &[0xAA, 0x60, 0x01, 0x00]);
        let exchange: Bytes = ExchangeInviteRequest { unique_id: uid }.into();
        assert_eq!(&exchange[..], &party[..]);
        let guild: Bytes = GuildInviteRequest { unique_id: uid }.into();
        assert_eq!(&guild[..], &party[..]);
        let academy: Bytes = AcademyInviteRequest { unique_id: uid }.into();
        assert_eq!(&academy[..], &party[..]);
    }

    /// 0xB081 inviter-side ack: the result byte selects the tail, so the
    /// refused case is three bytes and must decode as such — reading it as the
    /// success layout would make a refused invite look malformed.
    #[test]
    fn the_exchange_ack_decodes_both_tails() {
        let raised = Bytes::from_static(&[0x01, 0xAA, 0x60, 0x01, 0x00]);
        let ack: ExchangeInviteResponse = raised.clone().try_into().unwrap();
        assert_eq!(
            ack,
            ExchangeInviteResponse::Accepted {
                unique_id: 0x0001_60AA
            }
        );
        assert_eq!(Bytes::from(ack), raised);

        let refused = Bytes::from_static(&[0x02, 0x0C, 0x2C]);
        let ack: ExchangeInviteResponse = refused.clone().try_into().unwrap();
        assert_eq!(ack, ExchangeInviteResponse::Refused { error: 0x2C0C });
        assert_eq!(Bytes::from(ack), refused);

        // a body that stops inside its tail is an error, not a silent zero
        assert!(ExchangeInviteResponse::try_from(Bytes::from_static(&[0x01, 0x00])).is_err());
        assert!(ExchangeInviteResponse::try_from(Bytes::new()).is_err());
    }

    /// Every real 0x3011 body is the same single byte `04`, and the original's
    /// parser reads exactly one byte.
    #[test]
    fn character_died_decodes_live_capture() {
        let body = Bytes::from_static(&[0x04]);
        let decoded: CharacterDied = body.try_into().unwrap();
        assert_eq!(decoded.death_cause, 0x04);
        // the byte is passed through, not interpreted - its value space is
        // UNKNOWN, so any other cause must survive the round trip too
        for cause in [0x00u8, 0x01, 0x7F, 0xFF] {
            let raw = Bytes::copy_from_slice(&[cause]);
            let decoded: CharacterDied = raw.try_into().unwrap();
            assert_eq!(decoded.death_cause, cause);
        }
    }

    /// A real 0x304D body: `aa600100`.
    #[test]
    fn drop_unlocked_decodes_live_capture() {
        let body = Bytes::from_static(&[0xAA, 0x60, 0x01, 0x00]);
        let decoded: DropUnlocked = body.try_into().unwrap();
        assert_eq!(decoded.unique_id, 0x0001_60AA);
        assert_eq!(decoded.unique_id, 90282);
    }

    /// The 0x304D body is unconfirmed, so a longer real one is possible. The
    /// decode must ignore a tail rather than fail, leaving the unique id
    /// usable.
    #[test]
    fn drop_unlocked_tolerates_an_unknown_tail() {
        let body = Bytes::from_static(&[0xAA, 0x60, 0x01, 0x00, 0xDE, 0xAD]);
        let decoded: DropUnlocked = body.try_into().unwrap();
        assert_eq!(decoded.unique_id, 0x0001_60AA);
    }

    #[test]
    fn receive_experience_decodes_live_capture() {
        // a real 0x3056 line: kill grants 23 exp, 119 sp-exp
        let wire = Bytes::from_static(&[
            0xA8, 0x47, 0x01, 0x00, 0x17, 0, 0, 0, 0, 0, 0, 0, 0x77, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        let decoded: ReceiveExperience = wire.clone().try_into().unwrap();
        assert_eq!(decoded.exp_origin, 0x147A8);
        assert_eq!(decoded.experience, 23);
        assert_eq!(decoded.sp_exp, 119);
        assert_eq!(decoded.stat_points(), None);
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        // level-up: trailing u16 is the total stat points (here 12 = level 5)
        let wire = Bytes::from_static(&[
            1, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 12, 0,
        ]);
        let decoded: ReceiveExperience = wire.try_into().unwrap();
        assert_eq!(decoded.stat_points(), Some(12));
    }

    #[test]
    fn receive_experience_death_penalty_is_negative() {
        // a real 0x3056 death line: the EXP penalty arrives as a negative
        // i64 on the player's own uid. Read as u64 it is
        // 18446744073709548381.
        let wire = Bytes::from_static(&[
            0x80, 0xAB, 0x01, 0x00, 0x5D, 0xF3, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0,
            0, 0, 0, 0,
        ]);
        let decoded: ReceiveExperience = wire.clone().try_into().unwrap();
        assert_eq!(decoded.exp_origin, 0x1AB80);
        assert_eq!(decoded.experience, -3235);
        assert_eq!(decoded.sp_exp, 0);
        assert_eq!(decoded.stat_points(), None);
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    #[test]
    fn object_action_update_unknown_shapes_keep_raw_tail() {
        for wire in [
            // truncated success body
            Bytes::from_static(&[1, 0, 0x30, 5]),
            // success with trailing garbage after kind=none
            Bytes::from_static(&[
                1, 0, 0x30, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, 0, 0xAA,
            ]),
            // unknown result byte
            Bytes::from_static(&[9, 1, 2, 3]),
        ] {
            let decoded: ObjectActionUpdate = wire.clone().try_into().unwrap();
            assert!(
                matches!(decoded, ObjectActionUpdate::Unknown { .. }),
                "{wire:?}"
            );
            let back: Bytes = decoded.into();
            assert_eq!(back, wire);
        }
    }

    #[test]
    fn character_data_is_raw_passthrough() {
        let body = Bytes::from_static(&[1, 2, 3, 4, 5]);
        let packet: CharacterDataBody = body.clone().try_into().unwrap();
        assert_eq!(packet.raw, body);
        let back: Bytes = packet.into();
        assert_eq!(back, body);
    }

    #[test]
    fn empty_packets_serialize_empty() {
        let bytes: Bytes = GameReady.into();
        assert!(bytes.is_empty());
        let _decoded: GameReady = Bytes::new().try_into().unwrap();
    }

    #[test]
    fn movement_request_overworld_uses_shorts() {
        let req = MovementRequest {
            region: 0x60A8,
            x: 10580,
            y: -77,
            z: 14260,
        };
        let bytes: Bytes = req.clone().into();
        // kind(1) + region(u16) + 3× i16
        assert_eq!(bytes.len(), 1 + 2 + 6);
        assert_eq!(bytes[0], 1);
        let decoded: MovementRequest = bytes.try_into().unwrap();
        assert_eq!(decoded, req);
    }

    #[test]
    fn movement_request_dungeon_uses_ints() {
        let req = MovementRequest {
            region: 0x8001,
            x: 123456,
            y: -7,
            z: 654321,
        };
        let bytes: Bytes = req.clone().into();
        // kind(1) + region(u16) + 3× i32
        assert_eq!(bytes.len(), 1 + 2 + 12);
        let decoded: MovementRequest = bytes.try_into().unwrap();
        assert_eq!(decoded, req);
    }

    #[test]
    fn movement_response_destination_roundtrips() {
        let resp = MovementResponse {
            unique_id: 352808,
            has_destination: true,
            region: 0x60A8,
            x: 10580,
            y: -77,
            z: 14260,
            angle: 0,
            source: None,
        };
        let bytes: Bytes = resp.clone().into();
        let decoded: MovementResponse = bytes.try_into().unwrap();
        assert_eq!(decoded, resp);
    }

    /// The source tail is where the entity *is*; a live 0xB021 carries it, and
    /// dropping it left every consumer with only the destination — which is why
    /// a bot that walked by the echoed destination believed it had arrived
    /// while the character was still walking.
    #[test]
    fn movement_response_keeps_the_source_position() {
        let resp = MovementResponse {
            unique_id: 352808,
            has_destination: true,
            region: 0x60A8,
            x: 10580,
            y: -77,
            z: 14260,
            angle: 0,
            source: Some(MovementSource {
                region: 0x60A8,
                x: 10000,
                y: -77.0,
                z: 14000,
            }),
        };
        let bytes: Bytes = resp.clone().into();
        let decoded: MovementResponse = bytes.try_into().unwrap();
        assert_eq!(decoded, resp);
    }

    #[test]
    fn movement_response_short_body_fails_safe() {
        for len in 0..4 {
            let bytes = Bytes::copy_from_slice(&[0u8; 4][..len]);
            assert!(MovementResponse::try_from(bytes).is_err());
        }
    }

    #[test]
    fn logout_response_success_carries_countdown_and_mode() {
        let ok = LogoutResponse {
            result: 1,
            countdown: Some(5),
            mode: Some(LOGOUT_MODE_RESTART),
            error: None,
        };
        let bytes: Bytes = ok.clone().into();
        // result + countdown + mode, no error, no presence flags
        assert_eq!(&bytes[..], &[1, 5, LOGOUT_MODE_RESTART]);
        let decoded: LogoutResponse = bytes.try_into().unwrap();
        assert_eq!(decoded.countdown, Some(5));
        assert_eq!(decoded.mode, Some(LOGOUT_MODE_RESTART));
        assert_eq!(decoded.error, None);
    }

    #[test]
    fn movement_position_update_roundtrips() {
        let p = MovementPositionUpdate {
            unique_id: 352808,
            region: 0x60A8,
            x: 1058.0,
            y: -7.68,
            z: 1426.0,
            heading: 12268,
        };
        let bytes: Bytes = p.clone().into();
        // u32 + u16 + 3×f32 + u16
        assert_eq!(bytes.len(), 4 + 2 + 12 + 2);
        let decoded: MovementPositionUpdate = bytes.try_into().unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn entity_speed_update_roundtrips() {
        let p = EntitySpeedUpdate {
            unique_id: 352808,
            walk_speed: 16.0,
            run_speed: 50.0,
        };
        let bytes: Bytes = p.clone().into();
        // u32 + 2×f32
        assert_eq!(bytes.len(), 4 + 8);
        let decoded: EntitySpeedUpdate = bytes.try_into().unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn group_spawn_begin_roundtrips() {
        let begin = GroupEntitySpawnBegin {
            kind: GROUP_SPAWN,
            count: 3,
        };
        let bytes: Bytes = begin.clone().into();
        // kind(u8) + count(u16 LE)
        assert_eq!(&bytes[..], &[GROUP_SPAWN, 0x03, 0x00]);
        let decoded: GroupEntitySpawnBegin = bytes.try_into().unwrap();
        assert_eq!(decoded.kind, GROUP_SPAWN);
        assert_eq!(decoded.count, 3);
    }

    #[test]
    fn group_spawn_begin_ignores_trailing_bytes() {
        // Some servers append "unknown" fields after count; decode must not fail.
        let body = Bytes::from_static(&[GROUP_DESPAWN, 0x02, 0x00, 0xAA, 0xBB, 0xCC]);
        let decoded: GroupEntitySpawnBegin = body.try_into().unwrap();
        assert_eq!(decoded.kind, GROUP_DESPAWN);
        assert_eq!(decoded.count, 2);
    }

    #[test]
    fn group_spawn_data_is_raw_passthrough() {
        let body = Bytes::from_static(&[9, 8, 7, 6, 5, 4]);
        let packet: GroupEntitySpawnData = body.clone().try_into().unwrap();
        assert_eq!(packet.raw, body);
        let back: Bytes = packet.into();
        assert_eq!(back, body);
    }

    #[test]
    fn single_spawn_is_raw_passthrough() {
        let body = Bytes::from_static(&[1, 2, 3, 4, 5, 6, 7]);
        let packet: SingleEntitySpawn = body.clone().try_into().unwrap();
        assert_eq!(packet.raw, body);
        let back: Bytes = packet.into();
        assert_eq!(back, body);
    }

    #[test]
    fn single_despawn_roundtrips() {
        let p = SingleEntityDespawn { unique_id: 352808 };
        let bytes: Bytes = p.clone().into();
        assert_eq!(&bytes[..], &[0x28, 0x62, 0x05, 0x00]);
        let decoded: SingleEntityDespawn = bytes.try_into().unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn group_spawn_end_serializes_empty() {
        let bytes: Bytes = GroupEntitySpawnEnd.into();
        assert!(bytes.is_empty());
        let _decoded: GroupEntitySpawnEnd = Bytes::new().try_into().unwrap();
    }

    #[test]
    fn entity_bars_update_hp_only() {
        let p = EntityBarsUpdate {
            unique_id: 352808,
            source: BARS_SOURCE_DAMAGE,
            flag: BARS_FLAG_HP,
            hp: Some(231),
            mp: None,
            unknown16: None,
            bad_status: None,
            bad_status_levels: Vec::new(),
        };
        let bytes: Bytes = p.clone().into();
        // u32 + u16 + u8 + u32, no MP field on the wire
        assert_eq!(
            &bytes[..],
            &[0x28, 0x62, 0x05, 0x00, 0x01, 0x00, 0x01, 231, 0x00, 0x00, 0x00]
        );
        let decoded: EntityBarsUpdate = bytes.try_into().unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn entity_bars_update_both_orders_hp_before_mp() {
        let p = EntityBarsUpdate {
            unique_id: 1,
            source: BARS_SOURCE_REGEN,
            flag: BARS_FLAG_HP | BARS_FLAG_MP,
            hp: Some(0x11223344),
            mp: Some(0x55667788),
            unknown16: None,
            bad_status: None,
            bad_status_levels: Vec::new(),
        };
        let bytes: Bytes = p.clone().into();
        assert_eq!(bytes.len(), 4 + 2 + 1 + 4 + 4);
        assert_eq!(&bytes[7..11], &[0x44, 0x33, 0x22, 0x11]); // HP first
        let decoded: EntityBarsUpdate = bytes.try_into().unwrap();
        assert_eq!(decoded, p);
    }

    /// The `0x08` block is read BEFORE the `0x04` one — the original's order,
    /// not bit order. A body carrying both pins that, since swapping them
    /// would still consume the same byte count and silently mis-slice.
    #[test]
    fn entity_bars_update_reads_the_burn_body() {
        // A real body, verbatim: `e8 63 02 00 01 01 04 08 00 00 00`. Burn is
        // bit 3, which BAD_STATUS_LEVELED does not carry, so the body ends
        // after the mask — 11 bytes, no level tail.
        let body = Bytes::from_static(&[
            0xE8, 0x63, 0x02, 0x00, // unique_id 156136
            0x01, 0x01, // source
            0x04, // flag: bad status only
            0x08, 0x00, 0x00, 0x00, // mask: Burn
        ]);
        let decoded: EntityBarsUpdate = body.clone().try_into().unwrap();
        assert_eq!(decoded.hp, None);
        assert_eq!(decoded.mp, None);
        assert_eq!(decoded.unknown16, None);
        assert_eq!(decoded.bad_status(), Some(BadStatus(Ailment::Burn.bit())));
        assert!(decoded.bad_status_levels.is_empty());
        // and it goes back out byte-identically — the property
        // tools/src/bin/protocol_verify.rs replays
        let back: Bytes = decoded.into();
        assert_eq!(back, body);
    }

    /// The `0x08` block is read BEFORE the `0x04` one — the original's order,
    /// not bit order. A body carrying both pins that, since swapping them
    /// would still consume the same byte count and silently mis-slice.
    #[test]
    fn entity_bars_update_reads_the_unknown16_block_before_bad_status() {
        let p = EntityBarsUpdate {
            unique_id: 1,
            source: BARS_SOURCE_DAMAGE,
            flag: BARS_FLAG_HP | BARS_FLAG_BAD_STATUS | BARS_FLAG_UNKNOWN16,
            hp: Some(0x11223344),
            mp: None,
            unknown16: Some(0xBEEF),
            bad_status: Some(Ailment::Burn.bit()),
            bad_status_levels: Vec::new(),
        };
        let bytes: Bytes = p.clone().into();
        assert_eq!(&bytes[11..13], &[0xEF, 0xBE], "the u16 comes first");
        let decoded: EntityBarsUpdate = bytes.try_into().unwrap();
        assert_eq!(decoded, p);
    }

    /// A truncated level tail must not lose the mask that names the ailments:
    /// only the no-level case of the level rule is confirmed, so a short body
    /// yields fewer levels rather than failing the whole decode.
    #[test]
    fn a_short_level_tail_keeps_the_mask() {
        let mask = Ailment::Stun.bit() | Ailment::Bleed.bit();
        let mut body = vec![0x01, 0x00, 0x00, 0x00, 0x01, 0x00, BARS_FLAG_BAD_STATUS];
        body.extend_from_slice(&mask.to_le_bytes());
        body.push(3); // only one of the two expected level bytes
        let decoded: EntityBarsUpdate = Bytes::from(body).try_into().unwrap();
        assert_eq!(decoded.bad_status(), Some(BadStatus(mask)));
        assert_eq!(decoded.bad_status_levels, vec![3]);
    }

    #[test]
    fn character_points_berserk_roundtrips() {
        let p = CharacterPointsUpdate::Berserk {
            amount: 5,
            source: 352808,
        };
        let bytes: Bytes = p.clone().into();
        // discriminator u8 + amount u8 + source u32
        assert_eq!(&bytes[..], &[4, 5, 0x28, 0x62, 0x05, 0x00]);
        let decoded: CharacterPointsUpdate = bytes.try_into().unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn character_points_gold_roundtrips() {
        let p = CharacterPointsUpdate::Gold {
            amount: 123_456_789,
            display: 1,
        };
        let bytes: Bytes = p.clone().into();
        assert_eq!(bytes.len(), 1 + 8 + 1);
        let decoded: CharacterPointsUpdate = bytes.try_into().unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn character_stats_update_is_36_bytes() {
        let p = CharacterStatsUpdate {
            phys_attack_min: 10,
            phys_attack_max: 14,
            mag_attack_min: 20,
            mag_attack_max: 26,
            phys_defense: 8,
            mag_defense: 9,
            hit_rate: 25,
            parry_rate: 17,
            max_hp: 244,
            max_mp: 244,
            strength: 21,
            intelligence: 22,
        };
        let bytes: Bytes = p.clone().into();
        assert_eq!(bytes.len(), 36);
        let decoded: CharacterStatsUpdate = bytes.try_into().unwrap();
        assert_eq!(decoded.max_hp, 244);
        assert_eq!(decoded.max_mp, 244);
        assert_eq!(decoded, p);
    }

    #[test]
    fn logout_response_error_carries_code() {
        let err = LogoutResponse {
            result: 2,
            countdown: None,
            mode: None,
            error: Some(LOGOUT_ERROR_IN_BATTLE),
        };
        let bytes: Bytes = err.into();
        assert_eq!(&bytes[..], &[2, 0x01, 0x08]); // result + u16 LE
        let decoded: LogoutResponse = bytes.try_into().unwrap();
        assert_eq!(decoded.error, Some(LOGOUT_ERROR_IN_BATTLE));
        assert_eq!(decoded.countdown, None);
    }

    // --- World-join server pushes (vSRO 1.188) -------------------------------

    #[test]
    fn silk_update_decodes_captured_body() {
        // a real 0x3153 body: F4 CB 9A 3B 50 C3 00 00 00 00 00 00
        let body = Bytes::from_static(&[
            0xF4, 0xCB, 0x9A, 0x3B, // own = 1_000_000_500
            0x50, 0xC3, 0x00, 0x00, // gift = 50_000
            0x00, 0x00, 0x00, 0x00, // point = 0
        ]);
        let decoded: SilkUpdate = body.clone().try_into().unwrap();
        assert_eq!(
            decoded,
            SilkUpdate {
                own: 1_000_000_500,
                gift: 50_000,
                point: 0,
            }
        );
        let reencoded: Bytes = decoded.into();
        assert_eq!(reencoded, body);
    }

    #[test]
    fn weather_update_decodes_captured_body() {
        // a real 0x3809 body: 01 B4
        let body = Bytes::from_static(&[0x01, 0xB4]);
        let decoded: WeatherUpdate = body.clone().try_into().unwrap();
        assert_eq!(
            decoded,
            WeatherUpdate {
                weather_type: 1,
                intensity: 180,
            }
        );
        let reencoded: Bytes = decoded.into();
        assert_eq!(reencoded, body);
    }

    #[test]
    fn friend_list_info_decodes_empty_roster() {
        // a real 0x3305 body: 00 (empty roster)
        let body = Bytes::from_static(&[0x00]);
        let decoded: FriendListInfo = body.clone().try_into().unwrap();
        assert_eq!(decoded.count, 0);
        assert!(decoded.friends.is_empty());
        let reencoded: Bytes = decoded.into();
        assert_eq!(reencoded, body);
    }

    /// The record the original's parser reads is `u32, u16 len + ASCII, u32,
    /// `u8` — four fields, and no fifth. Two entries is the smallest roster
    /// that shows a two-byte field drift; the empty roster above cannot.
    ///
    /// Synthetic bytes: a real 0x3305 roster is always empty.
    #[test]
    fn friend_list_info_decodes_two_entries_without_drift() {
        let body = Bytes::from_static(&[
            0x02, // count
            // entry 1: id 0x00000101, "ab", model 7, online 1
            0x01, 0x01, 0x00, 0x00, //
            0x02, 0x00, b'a', b'b', //
            0x07, 0x00, 0x00, 0x00, //
            0x01, //
            // entry 2: id 0x00000202, "xyz", model 8, offline
            0x02, 0x02, 0x00, 0x00, //
            0x03, 0x00, b'x', b'y', b'z', //
            0x08, 0x00, 0x00, 0x00, //
            0x00,
        ]);
        let decoded: FriendListInfo = body.clone().try_into().unwrap();
        assert_eq!(decoded.count, 2);
        assert_eq!(
            decoded.friends,
            vec![
                FriendEntry {
                    char_id: 0x0101,
                    name: "ab".to_string(),
                    char_model: 7,
                    is_online: 1,
                },
                FriendEntry {
                    char_id: 0x0202,
                    name: "xyz".to_string(),
                    char_model: 8,
                    is_online: 0,
                },
            ]
        );
        // The record has no trailing padding: 1 + 2 * (11 + name_len).
        let reencoded: Bytes = decoded.into();
        assert_eq!(reencoded, body);
        assert_eq!(body.len(), 1 + (11 + 2) + (11 + 3));
    }

    #[test]
    fn character_finished_decodes_two_empty_lists() {
        // a real 0x3077 body: 00 00 (no item + no skill cooldowns)
        let body = Bytes::from_static(&[0x00, 0x00]);
        let decoded: CharacterFinished = body.clone().try_into().unwrap();
        assert_eq!(decoded.item_cooldown_count, 0);
        assert_eq!(decoded.skill_cooldown_count, 0);
        assert!(decoded.item_cooldowns.is_empty());
        assert!(decoded.skill_cooldowns.is_empty());
        let reencoded: Bytes = decoded.into();
        assert_eq!(reencoded, body);
    }

    // --- Server notice push (0x300C) — real wire bytes -----------------------

    /// A real 0x300C body: `05 0c 43 95 00 00` — code 0x0C05, ref id
    /// 0x9543 = 38211. Six bytes is what pins the discriminator as a u16: a
    /// second u8 field would have to be part of it.
    #[test]
    fn notice_update_decodes_a_captured_unique_spawn() {
        let body = Bytes::from_static(&[0x05, 0x0c, 0x43, 0x95, 0x00, 0x00]);

        let decoded = NoticeUpdate::try_from(body.clone()).unwrap();

        assert_eq!(decoded, NoticeUpdate::UniqueAppeared { ref_id: 38211 });
        assert_eq!(decoded.code(), NOTICE_UNIQUE_APPEARED);
        let back: Bytes = decoded.into();
        assert_eq!(back, body);
    }

    /// The next body differs only in the ref id (38212), which is what pins
    /// the field as a little-endian u32 rather than a wider or narrower one.
    #[test]
    fn notice_update_reads_the_second_captured_spawn_ref_id() {
        let body = Bytes::from_static(&[0x05, 0x0c, 0x44, 0x95, 0x00, 0x00]);

        assert_eq!(
            NoticeUpdate::try_from(body).unwrap(),
            NoticeUpdate::UniqueAppeared { ref_id: 38212 }
        );
    }

    /// A real 0x300C body: `18 0c 02 03` — code 0x0C18 with its two known
    /// bytes and no tail (both are >= 2, so the conditional 8-byte run is
    /// absent).
    #[test]
    fn notice_update_decodes_the_captured_0c18_code() {
        let body = Bytes::from_static(&[0x18, 0x0c, 0x02, 0x03]);

        let decoded = NoticeUpdate::try_from(body.clone()).unwrap();

        assert_eq!(
            decoded,
            NoticeUpdate::Code0C18 {
                a: 2,
                b: 3,
                tail: Bytes::new(),
            }
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, body);
    }

    /// A code no source decodes keeps its whole body — including the code — so
    /// nothing is invented and nothing is lost.
    #[test]
    fn an_unrecorded_notice_code_is_kept_raw() {
        let body = Bytes::from_static(&[0x16, 0x0c, 0xAA, 0xBB]);

        let decoded = NoticeUpdate::try_from(body.clone()).unwrap();

        assert_eq!(
            decoded,
            NoticeUpdate::Raw {
                code: 0x0C16,
                tail: Bytes::from_static(&[0xAA, 0xBB]),
            }
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, body);
    }

    #[test]
    fn notice_update_reads_the_killer_name_on_the_kill_code() {
        let mut body = vec![0x06, 0x0c];
        body.extend_from_slice(&38211u32.to_le_bytes());
        body.extend_from_slice(&5u16.to_le_bytes());
        body.extend_from_slice(b"Hunter");
        body.truncate(2 + 4 + 2 + 5); // length prefix says 5
        let body = Bytes::from(body);

        let decoded = NoticeUpdate::try_from(body.clone()).unwrap();

        assert_eq!(
            decoded,
            NoticeUpdate::UniqueKilled {
                ref_id: 38211,
                player: "Hunte".to_string(),
            }
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, body);
    }

    /// A spawn body of the wrong length must not be accepted as a spawn.
    #[test]
    fn a_short_notice_body_is_kept_raw_instead_of_misread() {
        let decoded = NoticeUpdate::try_from(Bytes::from_static(&[0x05, 0x0c, 0x43])).unwrap();

        assert!(matches!(
            decoded,
            NoticeUpdate::Raw {
                code: NOTICE_UNIQUE_APPEARED,
                ..
            }
        ));
    }

    // --- Mastery/skill level-down + teleport recall --------------------------

    #[test]
    fn skill_level_down_request_is_a_lone_skill_id() {
        let req = SkillLevelDownRequest {
            ref_skill_id: 0x0102_0304,
        };
        let wire: Bytes = req.clone().into();

        assert_eq!(&wire[..], &0x0102_0304u32.to_le_bytes());
        assert_eq!(SkillLevelDownRequest::try_from(wire).unwrap(), req);
    }

    /// The trailing `amount` byte the level-UP sibling carries is deliberately NOT
    /// mirrored onto the DOWN request — it is unresolved, so the body is 4 bytes.
    #[test]
    fn mastery_level_down_request_omits_the_unresolved_amount_byte() {
        let req = MasteryLevelDownRequest { mastery_id: 257 };
        let wire: Bytes = req.clone().into();

        assert_eq!(wire.len(), 4);
        assert_eq!(MasteryLevelDownRequest::try_from(wire).unwrap(), req);
    }

    #[test]
    fn skill_level_down_response_reads_the_new_skill_id() {
        let mut wire = vec![1u8];
        wire.extend_from_slice(&9001u32.to_le_bytes());

        let decoded = MasterySkillLevelDownResponse::try_from(Bytes::from(wire.clone())).unwrap();

        assert_eq!(
            decoded,
            MasterySkillLevelDownResponse::Success { new_skill_id: 9001 }
        );
        let back: Bytes = decoded.into();
        assert_eq!(&back[..], &wire[..]);
    }

    #[test]
    fn mastery_level_down_response_reads_the_new_level() {
        let mut wire = vec![1u8];
        wire.extend_from_slice(&257u32.to_le_bytes());
        wire.push(4);

        let decoded = MasteryLevelDownResponse::try_from(Bytes::from(wire.clone())).unwrap();

        assert_eq!(
            decoded,
            MasteryLevelDownResponse::Success {
                mastery_id: 257,
                new_level: 4,
            }
        );
        let back: Bytes = decoded.into();
        assert_eq!(&back[..], &wire[..]);
    }

    /// The failure branch is unconfirmed: the original reads no error code. Both
    /// candidate shapes must survive — `02 <code>` parses as `Failure`, while a
    /// lone `02` falls through to `Unknown` rather than being misread. That is the
    /// `pos == len` guard doing its job, and it is why cloning the level-UP shape
    /// is safe despite the unknown.
    #[test]
    fn level_down_failure_shapes_both_degrade_safely() {
        let mut with_code = vec![2u8];
        with_code.extend_from_slice(&0x7406u16.to_le_bytes());
        assert_eq!(
            MasteryLevelDownResponse::try_from(Bytes::from(with_code)).unwrap(),
            MasteryLevelDownResponse::Failure(0x7406)
        );

        let lone = Bytes::from_static(&[2]);
        assert_eq!(
            MasteryLevelDownResponse::try_from(lone).unwrap(),
            MasteryLevelDownResponse::Unknown {
                result: 2,
                tail: Bytes::new(),
            }
        );

        // Same for the skill half.
        assert_eq!(
            MasterySkillLevelDownResponse::try_from(Bytes::from_static(&[2])).unwrap(),
            MasterySkillLevelDownResponse::Unknown {
                result: 2,
                tail: Bytes::new(),
            }
        );
    }

    /// A success body of the wrong length must not be accepted as a success.
    #[test]
    fn a_short_level_down_success_body_is_not_read_as_success() {
        let decoded =
            MasteryLevelDownResponse::try_from(Bytes::from_static(&[1, 0x01, 0x01])).unwrap();

        assert!(matches!(
            decoded,
            MasteryLevelDownResponse::Unknown { result: 1, .. }
        ));
    }

    #[test]
    fn teleport_recall_request_is_a_lone_unique_id() {
        let req = TeleportRecallRequest {
            teleport_unique_id: 4242,
        };
        let wire: Bytes = req.clone().into();

        assert_eq!(&wire[..], &4242u32.to_le_bytes());
        assert_eq!(TeleportRecallRequest::try_from(wire).unwrap(), req);
    }

    /// 0xB059 is carried raw (see the struct's note: the original reads
    /// `u8 result` [+ `u16` error], but the error-code values are unknown), so
    /// any body — including an empty one — round-trips untouched instead of
    /// failing to decode.
    #[test]
    fn teleport_recall_response_keeps_any_body_whole() {
        for body in [
            Bytes::new(),
            Bytes::from_static(&[1]),
            Bytes::from_static(&[2, 0xAA, 0xBB]),
        ] {
            let decoded = TeleportRecallResponse::try_from(body.clone()).unwrap();
            assert_eq!(decoded.raw, body);
            let back: Bytes = decoded.into();
            assert_eq!(back, body);
        }
    }
    /// 0x70A7 is one byte — the original's builder writes exactly one
    /// (`0081e690:21,30`).
    #[test]
    fn hwan_action_request_is_a_single_byte() {
        let wire: Bytes = HwanActionRequest::berserk().into();
        assert_eq!(&wire[..], &[HWAN_ACTION_BERSERK]);
        assert_eq!(
            HwanActionRequest::try_from(wire).unwrap(),
            HwanActionRequest { action: 1 }
        );
    }

    /// 0xB0A7's error code is conditional: `008a7a20` reads the `u16` only when
    /// the result byte is not `1`, so a success body is a lone byte and a
    /// failure body carries the code.
    #[test]
    fn hwan_action_response_reads_the_error_code_only_on_failure() {
        let ok = HwanActionResponse::try_from(Bytes::from_static(&[0x01])).unwrap();
        assert!(ok.is_success());
        assert_eq!(ok.error_code, None);
        let back: Bytes = ok.into();
        assert_eq!(&back[..], &[0x01]);

        let failed = HwanActionResponse::try_from(Bytes::from_static(&[0x02, 0x34, 0x12])).unwrap();
        assert!(!failed.is_success());
        assert_eq!(failed.error_code, Some(0x1234));
        let back: Bytes = failed.into();
        assert_eq!(&back[..], &[0x02, 0x34, 0x12]);
    }

    /// 0x30DF is `{ u32 unique id, u8 level }` — `008a7630:8-9`, the u32 goes
    /// through the object registry before the byte is applied.
    #[test]
    fn hwan_level_update_is_an_id_and_a_level() {
        let wire = Bytes::from_static(&[0xBE, 0xAB, 0x01, 0x00, 0x03]);
        let decoded = HwanLevelUpdate::try_from(wire.clone()).unwrap();
        assert_eq!(
            decoded,
            HwanLevelUpdate {
                unique_id: 0x1ABBE,
                level: 3,
            }
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }
    /// Real 0x30BF bodies: the four state
    /// deltas a death and the following resurrection produce for the local
    /// player.
    #[test]
    fn death_and_resurrect_state_deltas_decode_from_the_capture() {
        let decode = |hex: &[u8; 6]| {
            EntityStateUpdate::try_from(Bytes::copy_from_slice(hex)).expect("decodes")
        };

        // combat flag drops in the same millisecond as the death
        let combat_off = decode(&[0x7b, 0xb3, 0x01, 0x00, 0x08, 0x00]);
        assert_eq!(combat_off.unique_id, 111_483);
        assert_eq!(combat_off.kind, STATE_KIND_COMBAT);
        assert_eq!(combat_off.value, 0);
        assert!(!combat_off.is_death(), "kind 8 is not the death signal");

        // the authoritative death
        let died = decode(&[0x7b, 0xb3, 0x01, 0x00, 0x00, 0x02]);
        assert!(died.is_death());

        // revive, plus the untouchable window
        let revived = decode(&[0x7b, 0xb3, 0x01, 0x00, 0x00, 0x01]);
        assert!(revived.is_revive());
        let untouchable = decode(&[0x7b, 0xb3, 0x01, 0x00, 0x04, 0x02]);
        assert_eq!(untouchable.value, BODY_STATE_UNTOUCHABLE);
        // it is a body state, but not one that hides the player
        assert_eq!(untouchable.body_invisibility(), Some(false));

        // cleared 6.29 s later
        let cleared = decode(&[0x7b, 0xb3, 0x01, 0x00, 0x04, 0x00]);
        assert_eq!(cleared.value, BODY_STATE_NONE);
    }

    /// The EXP penalty rides the same opcode as an EXP gain, with a negative
    /// value.
    #[test]
    fn death_charges_a_negative_experience_delta() {
        let body = Bytes::from_static(&[
            0x80, 0xab, 0x01, 0x00, // exp_origin = the dying player's own uid
            0x5d, 0xf3, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // experience = -3235
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // sp_exp
            0x00,
        ]);
        let decoded = ReceiveExperience::try_from(body).expect("decodes");
        assert_eq!(decoded.exp_origin, 109_440);
        assert_eq!(decoded.experience, -3235);
        assert_eq!(decoded.stat_points(), None, "no level-up tail on a death");
    }
}
