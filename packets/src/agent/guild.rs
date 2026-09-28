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

use crate::agent::cursor::{put_string, Cursor};

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
    /// The last byte of the member record. A live record with the founder as
    /// the only member ends `01 01 00`, which reads as `is_master = 1`,
    /// `is_offline = 1`, and the `00` is the head of the list behind the
    /// roster — not a third member field. Why an online character arrives with
    /// this flag set is an open question; it is the same one byte either way.
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
    /// How many [`GuildVoteEntry`] close the record.
    pub election_count: u8,
    #[sro_packet(list_type = "by-size-field", size_field = "election_count")]
    pub elections: Vec<GuildVoteEntry>,
}

/// One entry of the counted list that closes the guild record. The three
/// fields are read verbatim; their names are [U]. The first two have the same
/// widths as [`super::guild_leadership::GuildElectionEntry`]; whether the two
/// lists carry the same thing is [U].
#[derive(Serialize, Deserialize, ByteSize, Clone, Copy, Debug, PartialEq)]
pub struct GuildVoteEntry {
    /// [U] — read verbatim, semantics unknown.
    pub unk_u32_00: u32,
    /// [U] — read verbatim, semantics unknown.
    pub unk_u8_00: u8,
    /// [U] — read verbatim, semantics unknown.
    pub unk_u32_01: u32,
}

impl GuildData {
    /// Parse an assembled record. The roster is followed by one more counted
    /// list, and only after that list does the record end — so a well-formed
    /// buffer leaves no tail.
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
/// A `u8` sub-command family: the original dispatches the leading byte through
/// a jump table, and most arms read a body of their own. What is decoded here
/// is width, order and the arm a field belongs to — not meaning, hence the
/// `unk_*` names.
///
/// Arms that hand the packet on to a helper and read there are **not** typed;
/// they keep their bytes in [`GuildUpdate::Other`], together with every
/// sub-command the jump table sends to its default arm. An unknown
/// sub-command is not an error.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum GuildUpdate {
    /// Sub 0 — empty body.
    Sub00,
    /// Sub 1 — empty body.
    Sub01,
    Sub02 {
        unk_u32_00: u32,
        unk_str_00: String,
        unk_u8_00: u8,
        unk_u8_01: u8,
        unk_u32_01: u32,
        unk_u32_02: u32,
        unk_u32_03: u32,
        unk_u32_04: u32,
        unk_u32_05: u32,
        unk_str_01: String,
        unk_u32_06: u32,
        unk_u8_02: u8,
        unk_u8_03: u8,
    },
    Sub03 {
        unk_u32_00: u32,
        unk_u8_00: u8,
    },
    /// Sub 13.
    Sub0D {
        unk_u32_00: u32,
        unk_str_00: String,
        unk_u8_00: u8,
        unk_str_01: String,
        unk_u32_01: u32,
        unk_u8_01: u8,
    },
    /// Sub 14 — the arm carries a mask byte of its own, and four fields hang
    /// off its bits. Bit 4 gates **two** fields, a string and a `u32`.
    Sub0E {
        mask: u8,
        unk_u32_00: u32,
        unk_str_00: Option<String>,
        unk_u8_00: Option<u8>,
        unk_str_01: Option<String>,
        unk_u32_01: Option<u32>,
        unk_u8_01: Option<u8>,
    },
    /// Sub 18 — `kind` selects the single `u32`; any other kind reads nothing.
    Sub12 {
        kind: u8,
        unk_u32_00: Option<u32>,
    },
    /// Sub 20 — a counted list of pairs, the only list in this family.
    Sub14List {
        entries: Vec<GuildUpdateEntry>,
    },
    /// Sub 25 — a record shared with other call sites, then a string.
    Sub19 {
        record: GuildUpdateRecord,
        unk_str_00: String,
    },
    /// Sub 26 and 27 share one arm. The sub-command is carried so the two stay
    /// distinguishable on the way back out.
    Sub1A1B {
        sub: u8,
        unk_u32_00: u32,
    },
    /// Sub 28.
    Sub1C {
        unk_u32_00: u32,
        unk_u32_01: u32,
    },
    /// Sub 29.
    Sub1D {
        unk_u8_00: u8,
        unk_u32_00: u32,
        unk_u32_01: u32,
        unk_u32_02: u32,
        unk_str_00: String,
        unk_str_01: String,
    },
    /// Sub 31.
    Sub1F {
        unk_u32_00: u32,
        unk_u32_01: u32,
    },
    /// Sub 35.
    Sub23 {
        unk_u32_00: u32,
        unk_str_00: String,
    },
    /// Sub 50 — `kind` selects the single string; any other kind reads nothing.
    Sub32 {
        kind: u8,
        unk_str_00: Option<String>,
    },
    /// Every arm that is not decoded, and every sub-command the jump table
    /// sends to its default arm.
    Other {
        sub: u8,
        tail: Bytes,
    },
}

/// One entry of the [`GuildUpdate::Sub14List`] list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuildUpdateEntry {
    pub unk_u32_00: u32,
    pub unk_u32_01: u32,
}

/// The record [`GuildUpdate::Sub19`] reads. Its first field gates the rest, and
/// the same record is read from other call sites, so it is a type of its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuildUpdateRecord {
    pub unk_u32_00: u32,
    /// Read only when [`Self::unk_u32_00`] is non-zero.
    pub rest: Option<GuildUpdateRecordRest>,
}

/// The part of [`GuildUpdateRecord`] a zero head omits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuildUpdateRecordRest {
    pub unk_u32_01: u32,
    pub unk_u8_00: u8,
    pub unk_u32_02: u32,
    pub unk_u32_03: u32,
    pub unk_u32_04: u32,
    pub unk_u32_05: u32,
    pub unk_u32_06: u32,
}

impl TryFrom<Bytes> for GuildUpdate {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let mut c = Cursor::new(&value, "short 0x38F5 body");
        let sub = c.u8()?;
        let parsed = (|| -> Option<GuildUpdate> {
            let arm = match sub {
                0 => GuildUpdate::Sub00,
                1 => GuildUpdate::Sub01,
                2 => GuildUpdate::Sub02 {
                    unk_u32_00: c.u32().ok()?,
                    unk_str_00: c.string().ok()?,
                    unk_u8_00: c.u8().ok()?,
                    unk_u8_01: c.u8().ok()?,
                    unk_u32_01: c.u32().ok()?,
                    unk_u32_02: c.u32().ok()?,
                    unk_u32_03: c.u32().ok()?,
                    unk_u32_04: c.u32().ok()?,
                    unk_u32_05: c.u32().ok()?,
                    unk_str_01: c.string().ok()?,
                    unk_u32_06: c.u32().ok()?,
                    unk_u8_02: c.u8().ok()?,
                    unk_u8_03: c.u8().ok()?,
                },
                3 => GuildUpdate::Sub03 {
                    unk_u32_00: c.u32().ok()?,
                    unk_u8_00: c.u8().ok()?,
                },
                13 => GuildUpdate::Sub0D {
                    unk_u32_00: c.u32().ok()?,
                    unk_str_00: c.string().ok()?,
                    unk_u8_00: c.u8().ok()?,
                    unk_str_01: c.string().ok()?,
                    unk_u32_01: c.u32().ok()?,
                    unk_u8_01: c.u8().ok()?,
                },
                14 => {
                    let mask = c.u8().ok()?;
                    let unk_u32_00 = c.u32().ok()?;
                    let unk_str_00 = (mask & 1 != 0).then(|| c.string().ok()).flatten();
                    let unk_u8_00 = (mask & 2 != 0).then(|| c.u8().ok()).flatten();
                    // One bit, two fields.
                    let unk_str_01 = (mask & 4 != 0).then(|| c.string().ok()).flatten();
                    let unk_u32_01 = (mask & 4 != 0).then(|| c.u32().ok()).flatten();
                    let unk_u8_01 = (mask & 8 != 0).then(|| c.u8().ok()).flatten();
                    GuildUpdate::Sub0E {
                        mask,
                        unk_u32_00,
                        unk_str_00,
                        unk_u8_00,
                        unk_str_01,
                        unk_u32_01,
                        unk_u8_01,
                    }
                }
                18 => {
                    let kind = c.u8().ok()?;
                    let unk_u32_00 = matches!(kind, 1 | 2).then(|| c.u32().ok()).flatten();
                    GuildUpdate::Sub12 { kind, unk_u32_00 }
                }
                20 => {
                    let count = c.u8().ok()?;
                    let mut entries = Vec::with_capacity(count as usize);
                    for _ in 0..count {
                        entries.push(GuildUpdateEntry {
                            unk_u32_00: c.u32().ok()?,
                            unk_u32_01: c.u32().ok()?,
                        });
                    }
                    GuildUpdate::Sub14List { entries }
                }
                25 => {
                    let unk_u32_00 = c.u32().ok()?;
                    let rest = if unk_u32_00 != 0 {
                        Some(GuildUpdateRecordRest {
                            unk_u32_01: c.u32().ok()?,
                            unk_u8_00: c.u8().ok()?,
                            unk_u32_02: c.u32().ok()?,
                            unk_u32_03: c.u32().ok()?,
                            unk_u32_04: c.u32().ok()?,
                            unk_u32_05: c.u32().ok()?,
                            unk_u32_06: c.u32().ok()?,
                        })
                    } else {
                        None
                    };
                    GuildUpdate::Sub19 {
                        record: GuildUpdateRecord { unk_u32_00, rest },
                        unk_str_00: c.string().ok()?,
                    }
                }
                26 | 27 => GuildUpdate::Sub1A1B {
                    sub,
                    unk_u32_00: c.u32().ok()?,
                },
                28 => GuildUpdate::Sub1C {
                    unk_u32_00: c.u32().ok()?,
                    unk_u32_01: c.u32().ok()?,
                },
                29 => GuildUpdate::Sub1D {
                    unk_u8_00: c.u8().ok()?,
                    unk_u32_00: c.u32().ok()?,
                    unk_u32_01: c.u32().ok()?,
                    unk_u32_02: c.u32().ok()?,
                    unk_str_00: c.string().ok()?,
                    unk_str_01: c.string().ok()?,
                },
                31 => GuildUpdate::Sub1F {
                    unk_u32_00: c.u32().ok()?,
                    unk_u32_01: c.u32().ok()?,
                },
                35 => GuildUpdate::Sub23 {
                    unk_u32_00: c.u32().ok()?,
                    unk_str_00: c.string().ok()?,
                },
                50 => {
                    let kind = c.u8().ok()?;
                    let unk_str_00 = matches!(kind, 0 | 2).then(|| c.string().ok()).flatten();
                    GuildUpdate::Sub32 { kind, unk_str_00 }
                }
                _ => return None,
            };
            Some(arm)
        })()
        // A body that does not close on its last byte is a layout surprise:
        // keeping it whole beats reporting a half-read arm.
        .filter(|_| c.at_end());
        Ok(parsed.unwrap_or(GuildUpdate::Other {
            sub,
            tail: value.slice(1..),
        }))
    }
}

impl From<GuildUpdate> for Bytes {
    fn from(p: GuildUpdate) -> Self {
        let mut buf = BytesMut::new();
        match p {
            GuildUpdate::Sub00 => buf.put_u8(0),
            GuildUpdate::Sub01 => buf.put_u8(1),
            GuildUpdate::Sub02 {
                unk_u32_00,
                unk_str_00,
                unk_u8_00,
                unk_u8_01,
                unk_u32_01,
                unk_u32_02,
                unk_u32_03,
                unk_u32_04,
                unk_u32_05,
                unk_str_01,
                unk_u32_06,
                unk_u8_02,
                unk_u8_03,
            } => {
                buf.put_u8(2);
                buf.put_u32_le(unk_u32_00);
                put_string(&mut buf, &unk_str_00);
                buf.put_u8(unk_u8_00);
                buf.put_u8(unk_u8_01);
                buf.put_u32_le(unk_u32_01);
                buf.put_u32_le(unk_u32_02);
                buf.put_u32_le(unk_u32_03);
                buf.put_u32_le(unk_u32_04);
                buf.put_u32_le(unk_u32_05);
                put_string(&mut buf, &unk_str_01);
                buf.put_u32_le(unk_u32_06);
                buf.put_u8(unk_u8_02);
                buf.put_u8(unk_u8_03);
            }
            GuildUpdate::Sub03 {
                unk_u32_00,
                unk_u8_00,
            } => {
                buf.put_u8(3);
                buf.put_u32_le(unk_u32_00);
                buf.put_u8(unk_u8_00);
            }
            GuildUpdate::Sub0D {
                unk_u32_00,
                unk_str_00,
                unk_u8_00,
                unk_str_01,
                unk_u32_01,
                unk_u8_01,
            } => {
                buf.put_u8(13);
                buf.put_u32_le(unk_u32_00);
                put_string(&mut buf, &unk_str_00);
                buf.put_u8(unk_u8_00);
                put_string(&mut buf, &unk_str_01);
                buf.put_u32_le(unk_u32_01);
                buf.put_u8(unk_u8_01);
            }
            GuildUpdate::Sub0E {
                mask,
                unk_u32_00,
                unk_str_00,
                unk_u8_00,
                unk_str_01,
                unk_u32_01,
                unk_u8_01,
            } => {
                buf.put_u8(14);
                buf.put_u8(mask);
                buf.put_u32_le(unk_u32_00);
                if let Some(v) = unk_str_00 {
                    put_string(&mut buf, &v);
                }
                if let Some(v) = unk_u8_00 {
                    buf.put_u8(v);
                }
                if let Some(v) = unk_str_01 {
                    put_string(&mut buf, &v);
                }
                if let Some(v) = unk_u32_01 {
                    buf.put_u32_le(v);
                }
                if let Some(v) = unk_u8_01 {
                    buf.put_u8(v);
                }
            }
            GuildUpdate::Sub12 { kind, unk_u32_00 } => {
                buf.put_u8(18);
                buf.put_u8(kind);
                if let Some(v) = unk_u32_00 {
                    buf.put_u32_le(v);
                }
            }
            GuildUpdate::Sub14List { entries } => {
                buf.put_u8(20);
                buf.put_u8(entries.len() as u8);
                for e in entries {
                    buf.put_u32_le(e.unk_u32_00);
                    buf.put_u32_le(e.unk_u32_01);
                }
            }
            GuildUpdate::Sub19 { record, unk_str_00 } => {
                buf.put_u8(25);
                buf.put_u32_le(record.unk_u32_00);
                if let Some(r) = record.rest {
                    buf.put_u32_le(r.unk_u32_01);
                    buf.put_u8(r.unk_u8_00);
                    buf.put_u32_le(r.unk_u32_02);
                    buf.put_u32_le(r.unk_u32_03);
                    buf.put_u32_le(r.unk_u32_04);
                    buf.put_u32_le(r.unk_u32_05);
                    buf.put_u32_le(r.unk_u32_06);
                }
                put_string(&mut buf, &unk_str_00);
            }
            GuildUpdate::Sub1A1B { sub, unk_u32_00 } => {
                buf.put_u8(sub);
                buf.put_u32_le(unk_u32_00);
            }
            GuildUpdate::Sub1C {
                unk_u32_00,
                unk_u32_01,
            } => {
                buf.put_u8(28);
                buf.put_u32_le(unk_u32_00);
                buf.put_u32_le(unk_u32_01);
            }
            GuildUpdate::Sub1D {
                unk_u8_00,
                unk_u32_00,
                unk_u32_01,
                unk_u32_02,
                unk_str_00,
                unk_str_01,
            } => {
                buf.put_u8(29);
                buf.put_u8(unk_u8_00);
                buf.put_u32_le(unk_u32_00);
                buf.put_u32_le(unk_u32_01);
                buf.put_u32_le(unk_u32_02);
                put_string(&mut buf, &unk_str_00);
                put_string(&mut buf, &unk_str_01);
            }
            GuildUpdate::Sub1F {
                unk_u32_00,
                unk_u32_01,
            } => {
                buf.put_u8(31);
                buf.put_u32_le(unk_u32_00);
                buf.put_u32_le(unk_u32_01);
            }
            GuildUpdate::Sub23 {
                unk_u32_00,
                unk_str_00,
            } => {
                buf.put_u8(35);
                buf.put_u32_le(unk_u32_00);
                put_string(&mut buf, &unk_str_00);
            }
            GuildUpdate::Sub32 { kind, unk_str_00 } => {
                buf.put_u8(50);
                buf.put_u8(kind);
                if let Some(v) = unk_str_00 {
                    put_string(&mut buf, &v);
                }
            }
            GuildUpdate::Other { sub, tail } => {
                buf.put_u8(sub);
                buf.extend_from_slice(&tail);
            }
        }
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
/// The captured record ends `01 01 00`, and the last of those bytes is the
/// count of the list that closes the record rather than a member field — with
/// it the record leaves [`Self::tail`] empty, which is what shows the whole
/// record is aligned rather than merely plausible.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct GuildCreatedData {
    pub result: u8,
    /// Present when `result == 1`.
    pub data: Option<GuildData>,
    /// Present when `result != 1`. The refusal code; `0x0003` = no NPC dialogue
    /// open.
    pub error: Option<u16>,
    /// Present when `error` is [`GUILD_SECESSION_PENALTY`]: how long the
    /// character still has to wait, in seconds.
    pub secession_penalty_seconds: Option<u32>,
    /// Anything after the record, and after the penalty when there is one. On
    /// the wire this is empty in every other case. Kept so a future server that
    /// appends something does not lose it silently.
    pub tail: Bytes,
}

/// The one refusal code of this opcode that carries a body of its own: the
/// original reads a `u32` of seconds behind it and shows the remaining
/// secession lock-out as days, hours and minutes.
pub const GUILD_SECESSION_PENALTY: u16 = 0x4C3C;

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
            let mut read = value.len().min(3);
            let mut secession_penalty_seconds = None;
            if error == Some(GUILD_SECESSION_PENALTY) && value.len() >= 7 {
                secession_penalty_seconds =
                    Some(u32::from_le_bytes([value[3], value[4], value[5], value[6]]));
                read = 7;
            }
            return Ok(GuildCreatedData {
                result,
                data: None,
                error,
                secession_penalty_seconds,
                tail: value.slice(read..),
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
            secession_penalty_seconds: None,
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
        if let Some(seconds) = p.secession_penalty_seconds {
            buf.put_u32_le(seconds);
        }
        buf.extend_from_slice(&p.tail);
        buf.freeze()
    }
}

/// 0xB0F8 — server → client: an ack whose success arm carries the whole guild
/// record, the same one the chunked transfer assembles.
///
/// The request it answers is [U]. On `result == 2` it reads a `u16` code from
/// the guild error family and special-cases exactly one value (`0x4C10`), which
/// is what puts this opcode in the guild family rather than anywhere else.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct GuildRecordResponse {
    pub result: u8,
    /// Present when `result == 1`.
    pub data: Option<GuildData>,
    /// Present when `result == 2`.
    pub error: Option<u16>,
    /// Anything after the record. Empty on the wire; kept so a future server
    /// that appends something does not lose it silently.
    pub tail: Bytes,
}

impl TryFrom<Bytes> for GuildRecordResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let result = *value.first().ok_or_else(|| {
            SerializationError::IoError(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "empty 0xB0F8 body",
            ))
        })?;
        match result {
            1 => {
                let (data, consumed) = GuildData::parse_prefix(value.slice(1..))?;
                Ok(GuildRecordResponse {
                    result,
                    data: Some(data),
                    error: None,
                    tail: value.slice(1 + consumed..),
                })
            }
            2 => Ok(GuildRecordResponse {
                result,
                data: None,
                error: (value.len() >= 3).then(|| u16::from_le_bytes([value[1], value[2]])),
                tail: value.slice(value.len().min(3)..),
            }),
            // Any other result reads nothing at all.
            _ => Ok(GuildRecordResponse {
                result,
                data: None,
                error: None,
                tail: value.slice(1..),
            }),
        }
    }
}

impl From<GuildRecordResponse> for Bytes {
    fn from(p: GuildRecordResponse) -> Self {
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
            /// `result == 1`. Every other value reads as the error arm, so an
            /// unexpected result surfaces as a failure and not as a success.
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
            out.push(1); // is_offline
        }
        out.push(0); // election_count
        out
    }

    /// 0xB0F8 carries the whole record on success and a code on refusal.
    #[test]
    fn the_record_ack_is_wired_and_decodes_both_arms() {
        let mut body = vec![1u8];
        body.extend(guild_record(&[("Founder", GuildPermissions::MASTER)]));
        let wire = Bytes::from(body);

        let packet = crate::Packet::deserialize(0xB0F8, wire.clone()).unwrap();
        let (opcode, back) = packet.into_serialize();
        assert_eq!(opcode, 0xB0F8);
        assert_eq!(back, wire);

        let decoded = GuildRecordResponse::try_from(wire).unwrap();
        assert_eq!(decoded.data.as_ref().unwrap().name, "Wanderers");
        assert!(decoded.tail.is_empty(), "the record ends the body");

        let refused = Bytes::from_static(&[0x02, 0x10, 0x4C]);
        let decoded = GuildRecordResponse::try_from(refused.clone()).unwrap();
        assert_eq!(decoded.error, Some(0x4C10));
        assert!(decoded.data.is_none());
        assert_eq!(Bytes::from(decoded), refused);
    }

    /// The record does not end with the last member: a second counted list
    /// follows it.
    #[test]
    fn the_record_ends_in_the_second_list_not_in_the_last_member() {
        let mut wire = guild_record(&[("Solo", GuildPermissions::ALL)]);
        // one entry in the second list
        let last = wire.len() - 1;
        wire[last] = 1;
        wire.extend(0xAABB_CCDDu32.to_le_bytes());
        wire.push(7);
        wire.extend(0x1122_3344u32.to_le_bytes());
        let wire = Bytes::from(wire);

        let decoded = GuildData::parse(wire.clone()).expect("record with a second list");
        assert_eq!(decoded.members.len(), 1);
        assert_eq!(decoded.election_count, 1);
        assert_eq!(
            decoded.elections,
            vec![GuildVoteEntry {
                unk_u32_00: 0xAABB_CCDD,
                unk_u8_00: 7,
                unk_u32_01: 0x1122_3344,
            }]
        );
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
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

    /// One refusal code carries four more bytes: the remaining secession
    /// lock-out in seconds. They used to sit unnamed in `tail`, so nothing told
    /// a caller they were a duration.
    #[test]
    fn the_secession_penalty_refusal_names_its_seconds() {
        let wire = Bytes::from_static(&[0x02, 0x3C, 0x4C, 0x80, 0x51, 0x01, 0x00]);

        let decoded = GuildCreatedData::try_from(wire.clone()).unwrap();

        assert_eq!(decoded.error, Some(GUILD_SECESSION_PENALTY));
        assert_eq!(decoded.secession_penalty_seconds, Some(86_400));
        assert!(decoded.tail.is_empty());
        assert_eq!(Bytes::from(decoded), wire);

        // Any other code keeps the plain three-byte shape.
        let plain = GuildCreatedData::try_from(Bytes::from_static(&[0x02, 0x03, 0x00])).unwrap();
        assert_eq!(plain.secession_penalty_seconds, None);
    }

    /// Body bytes for a 0x38F5 arm, decoded and written back: every arm must
    /// reproduce its own wire bytes exactly.
    fn guild_update(hex: &str) -> GuildUpdate {
        let clean: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
        let wire = Bytes::from(
            (0..clean.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
                .collect::<Vec<u8>>(),
        );
        let decoded = GuildUpdate::try_from(wire.clone()).unwrap();
        assert_eq!(Bytes::from(decoded.clone()), wire, "write-back differs");
        decoded
    }

    /// Two arms read nothing at all, which is not the same as "unknown".
    #[test]
    fn the_two_empty_guild_update_arms_are_named() {
        assert_eq!(guild_update("00"), GuildUpdate::Sub00);
        assert_eq!(guild_update("01"), GuildUpdate::Sub01);
        assert!(GuildUpdate::try_from(Bytes::new()).is_err());
    }

    /// The longest flat arm — thirteen fields, two of them strings.
    #[test]
    fn the_thirteen_field_guild_update_arm_decodes_in_order() {
        let decoded = guild_update(
            "02 00222222 020066 31 12 13 04222222 05222222 06222222 07222222 08222222 \
             020066 39 0A222222 1B 1C",
        );
        assert_eq!(
            decoded,
            GuildUpdate::Sub02 {
                unk_u32_00: 0x2222_2200,
                unk_str_00: "f1".into(),
                unk_u8_00: 0x12,
                unk_u8_01: 0x13,
                unk_u32_01: 0x2222_2204,
                unk_u32_02: 0x2222_2205,
                unk_u32_03: 0x2222_2206,
                unk_u32_04: 0x2222_2207,
                unk_u32_05: 0x2222_2208,
                unk_str_01: "f9".into(),
                unk_u32_06: 0x2222_220A,
                unk_u8_02: 0x1B,
                unk_u8_03: 0x1C,
            }
        );
    }

    #[test]
    fn the_short_guild_update_arms_decode() {
        assert_eq!(
            guild_update("03 00222222 11"),
            GuildUpdate::Sub03 {
                unk_u32_00: 0x2222_2200,
                unk_u8_00: 0x11,
            }
        );
        assert_eq!(
            guild_update("1C 00222222 01222222"),
            GuildUpdate::Sub1C {
                unk_u32_00: 0x2222_2200,
                unk_u32_01: 0x2222_2201,
            }
        );
        assert_eq!(
            guild_update("1F 00222222 01222222"),
            GuildUpdate::Sub1F {
                unk_u32_00: 0x2222_2200,
                unk_u32_01: 0x2222_2201,
            }
        );
        assert_eq!(
            guild_update("23 00222222 0200 6631"),
            GuildUpdate::Sub23 {
                unk_u32_00: 0x2222_2200,
                unk_str_00: "f1".into(),
            }
        );
    }

    /// Strings and numbers alternate here, so a shifted frame shows up at once.
    #[test]
    fn the_alternating_guild_update_arm_decodes() {
        assert_eq!(
            guild_update("0D 00222222 020066 31 12 020066 33 04222222 15"),
            GuildUpdate::Sub0D {
                unk_u32_00: 0x2222_2200,
                unk_str_00: "f1".into(),
                unk_u8_00: 0x12,
                unk_str_01: "f3".into(),
                unk_u32_01: 0x2222_2204,
                unk_u8_01: 0x15,
            }
        );
    }

    /// The mask arm. The second body is the interesting one: one bit carries
    /// two fields, so reading the bits as "one bit, one field" shifts the frame
    /// by four bytes.
    #[test]
    fn the_mask_arm_reads_two_fields_behind_one_bit() {
        assert_eq!(
            guild_update("0E 00 01000000"),
            GuildUpdate::Sub0E {
                mask: 0,
                unk_u32_00: 1,
                unk_str_00: None,
                unk_u8_00: None,
                unk_str_01: None,
                unk_u32_01: None,
                unk_u8_01: None,
            }
        );
        assert_eq!(
            guild_update("0E 04 01000000 0200 6162 09000000"),
            GuildUpdate::Sub0E {
                mask: 4,
                unk_u32_00: 1,
                unk_str_00: None,
                unk_u8_00: None,
                unk_str_01: Some("ab".into()),
                unk_u32_01: Some(9),
                unk_u8_01: None,
            }
        );
    }

    /// A leading byte selects the single field; any other value reads nothing.
    #[test]
    fn the_kind_selected_arms_read_one_field_or_none() {
        assert_eq!(
            guild_update("12 01 07000000"),
            GuildUpdate::Sub12 {
                kind: 1,
                unk_u32_00: Some(7),
            }
        );
        assert_eq!(
            guild_update("12 03"),
            GuildUpdate::Sub12 {
                kind: 3,
                unk_u32_00: None,
            }
        );
        assert_eq!(
            guild_update("32 00 0200 6162"),
            GuildUpdate::Sub32 {
                kind: 0,
                unk_str_00: Some("ab".into()),
            }
        );
        assert_eq!(
            guild_update("32 05"),
            GuildUpdate::Sub32 {
                kind: 5,
                unk_str_00: None,
            }
        );
    }

    #[test]
    fn the_counted_list_arm_handles_an_empty_and_a_filled_list() {
        assert_eq!(
            guild_update("14 00"),
            GuildUpdate::Sub14List { entries: vec![] }
        );
        assert_eq!(
            guild_update("14 02 01000000 02000000 03000000 04000000"),
            GuildUpdate::Sub14List {
                entries: vec![
                    GuildUpdateEntry {
                        unk_u32_00: 1,
                        unk_u32_01: 2,
                    },
                    GuildUpdateEntry {
                        unk_u32_00: 3,
                        unk_u32_01: 4,
                    },
                ],
            }
        );
    }

    /// The record's head gates its own tail, so a zero head is a five-byte
    /// record followed by the arm's string.
    #[test]
    fn the_record_arm_gates_its_tail_on_the_head_field() {
        assert_eq!(
            guild_update("19 00000000 0200 6162"),
            GuildUpdate::Sub19 {
                record: GuildUpdateRecord {
                    unk_u32_00: 0,
                    rest: None,
                },
                unk_str_00: "ab".into(),
            }
        );
        assert_eq!(
            guild_update(
                "19 01000000 02000000 03 04000000 05000000 06000000 07000000 08000000 0200 6162"
            ),
            GuildUpdate::Sub19 {
                record: GuildUpdateRecord {
                    unk_u32_00: 1,
                    rest: Some(GuildUpdateRecordRest {
                        unk_u32_01: 2,
                        unk_u8_00: 3,
                        unk_u32_02: 4,
                        unk_u32_03: 5,
                        unk_u32_04: 6,
                        unk_u32_05: 7,
                        unk_u32_06: 8,
                    }),
                },
                unk_str_00: "ab".into(),
            }
        );
    }

    /// Two sub-commands share one arm, so the variant carries the sub-command:
    /// without it the two could not be told apart on the way back out.
    #[test]
    fn the_shared_arm_carries_its_sub_command() {
        assert_eq!(
            guild_update("1A 09000000"),
            GuildUpdate::Sub1A1B {
                sub: 26,
                unk_u32_00: 9,
            }
        );
        assert_eq!(
            guild_update("1B 09000000"),
            GuildUpdate::Sub1A1B {
                sub: 27,
                unk_u32_00: 9,
            }
        );
    }

    #[test]
    fn the_six_field_guild_update_arm_decodes() {
        assert_eq!(
            guild_update("1D 10 01000000 02000000 03000000 0200 6162 0200 6364"),
            GuildUpdate::Sub1D {
                unk_u8_00: 0x10,
                unk_u32_00: 1,
                unk_u32_01: 2,
                unk_u32_02: 3,
                unk_str_00: "ab".into(),
                unk_str_01: "cd".into(),
            }
        );
    }

    /// Arms that read through a helper, and every sub-command the jump table
    /// sends to its default arm, keep their bytes. An unknown value is not an
    /// error.
    #[test]
    fn undecoded_guild_update_arms_keep_their_bytes() {
        for hex in ["05 AABB", "06 AABB", "16 AABB", "0F 0102"] {
            assert!(
                matches!(guild_update(hex), GuildUpdate::Other { .. }),
                "{hex} should stay raw"
            );
        }
    }

    /// A body that does not close on the last byte is kept whole rather than
    /// reported as a half-read arm.
    #[test]
    fn a_guild_update_arm_that_does_not_close_stays_raw() {
        assert!(matches!(
            guild_update("03 00222222 11 FF"),
            GuildUpdate::Other { sub: 3, .. }
        ));
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
