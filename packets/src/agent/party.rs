//! Party wire opcodes: party data/update (0x3065, 0x3864), the create/leave/kick
//! requests (0x7060/0x7061/0x7063) and the party-match family (0x706x / 0xB06x).
//!
//! **0x3065 and 0x3864 are confirmed on the wire; the rest is still spec-derived.**
//! Rosters and deltas parse with zero leftover bytes, and the delta bodies below
//! (types 1/2/3/6/9) are read off the original's own parser and handler.
//! Everything else here (the match family, most acks) comes from statically
//! reading the original client's parser/builder (xBot `PacketParser.cs`/
//! `PacketBuilder.cs`), byte-level. Three
//! opcodes (0x706A, 0xB069, 0xB06A) have **no** original-client code at all and
//! are go-sro-shaped only — flagged per struct.
//!
//! # The presence-mask correction
//!
//! The one place that is **not** spec-derived is the record framing, and it is
//! the load-bearing one. The original's own handlers — 0x3065, 0x3864, 0x706D
//! and the shared member reader — agree on a shape neither xBot nor go-sro states outright: **both the roster
//! header and every member record begin with a presence bitmask, and only the
//! fields whose bit is set are on the wire.**
//!
//! A record with *all* bits set is byte-for-byte what the older fixed-layout
//! model read, which is exactly why the bug hid: the full record is the common
//! case, so the fixed reading survived every desk check. A partial record —
//! which is the normal shape of a 0x3864 delta — mis-slices from the first
//! absent field onward and desynchronises the rest of the packet.
//!
//! Three consequences worth stating because they contradict older notes:
//!
//! - the "opaque 9 bytes" of [`PartyData`] are resolved, and with them the
//!   **party leader** (`master_join_id`), which the client-side roster used to
//!   declare UNKNOWN;
//! - the extra byte xBot's parser reads in the 0x3864 flavour of the record
//!   (no licence; facts only) **does not exist** — the binary has no such read,
//!   so the two record flavours
//!   collapse into the single [`PartyMemberCore`];
//! - [`PartyMemberUpdate`]'s `kind` is a **bitmask**, not an enum, so combined
//!   masks like `0x24` (level *and* hp/mp) are ordinary traffic.
//!
//! The invite path (0x7062 / 0xB060 / 0x3080) is deliberately not here; see
//! `docs/net-invite-0x3080.md`.
//!
//! ⚠️ Free-text caveat: the derive decodes `String` with `from_utf8`, which
//! **fails the whole packet** on an invalid byte, whereas the original reads
//! cp1252 and never fails. `title`, `name` and `guild_name` here are all
//! user-authored, so a non-UTF-8 byte drops the event. That is a pre-existing
//! tree-wide property of the derived string path (the hand-written paths use a
//! lossy reader for exactly this reason — see `character_data::read_string`),
//! not something this module introduces; it is called out because party titles
//! are a likelier place to meet it than most.

use std::io::Cursor;

use bevy::prelude::Message;
use bytes::{BufMut, Bytes, BytesMut};

use crate::agent::character_data::{ItemClass, ItemClassResolver};

use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

/// `SRParty.Purpose`: what the party advertises itself for.
pub const PARTY_PURPOSE_HUNTING: u8 = 0;
pub const PARTY_PURPOSE_QUEST: u8 = 1;
pub const PARTY_PURPOSE_TRADER: u8 = 2;
pub const PARTY_PURPOSE_THIEF: u8 = 3;

/// `SRParty.Setup` — a `[Flags]` bitfield, not an enum, so it is modelled as a
/// newtype rather than a derived enum (an unknown value must not fail the
/// packet). Corroborated by go-sro `model/party_setting.go`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PartySetup(pub u8);

impl PartySetup {
    pub const EXP_SHARED: u8 = 0x01;
    pub const ITEM_SHARED: u8 = 0x02;
    pub const ANYONE_CAN_INVITE: u8 = 0x04;

    pub fn is_exp_shared(&self) -> bool {
        self.0 & Self::EXP_SHARED != 0
    }

    pub fn is_item_shared(&self) -> bool {
        self.0 & Self::ITEM_SHARED != 0
    }

    pub fn anyone_can_invite(&self) -> bool {
        self.0 & Self::ANYONE_CAN_INVITE != 0
    }

    /// Max members: sharing EXP raises the cap from 4 to 8 (`SRParty.cs:12`).
    pub fn capacity(&self) -> u8 {
        if self.is_exp_shared() {
            8
        } else {
            4
        }
    }
}

/// A bar's fill as the exact fraction the wire carries, so the rounding happens
/// once, at the pixel/text that shows it — the roster gauges are a crop of fixed
/// art, and quantising twice is how a full bar ends up one texel short.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PartyBarFill {
    pub numerator: u8,
    pub denominator: u8,
}

impl PartyBarFill {
    /// `0.0 ..= 1.0`. Clamped at the top because the HP scale can exceed full:
    /// nibble 11 is `10/9`, and the original's gauge clamps it the same way
    /// (whether 11 is *exactly* 100 % or the top of a saturating scale is open —
    /// every vitals frame seen so far had a full-HP subject).
    pub fn as_f32(&self) -> f32 {
        if self.denominator == 0 {
            return 0.0;
        }
        (f32::from(self.numerator) / f32::from(self.denominator)).clamp(0.0, 1.0)
    }

    /// For text only. Rounds the fraction, never a pre-rounded percent.
    pub fn percent_rounded(&self) -> u8 {
        (self.as_f32() * 100.0).round() as u8
    }
}

/// One byte packing both bars — **asymmetrically**, which is the whole point of
/// this type.
///
/// Origin: the original's vitals setter splits the byte
/// (`movzx eax,[esp+0x96]`; `and eax,0xf` becomes arg2, `shr ecx,4` becomes
/// arg4, both paired with the literal max `10`), so **low nibble = HP, high
/// nibble = MP** from the argument order rather than by convention. Its setter
/// then stores `hp == 0` as a *dead* flag at `node+0x60` and
/// otherwise `hp-1` over `hp_max-1`, while MP is stored unchanged over 10:
/// HP fills `(n-1)/9`, MP fills `m/10`.
///
/// The naive `nibble * 10` this type used to return is wrong on the real wire:
/// a full-HP character carries a low nibble of `0x0B` = 11, which that reading
/// turns into **110 %**.
///
/// **One reading for both paths — a deliberate simplification** (ADR-0009). The
/// original is inconsistent with itself: the full-roster/join path
/// stores the low nibble *raw*, without
/// the `-1`, while the delta path applies it. The byte is the same server field
/// in both, so its scale cannot differ; the two only agree at "full" (raw `10/10`
/// vs `(11-1)/(10-1)`), which is why the asymmetry survived — the roster byte
/// is `0xAA`, i.e. full, in ordinary traffic. We use the 1-based reading everywhere,
/// because it is the one that can express "dead" and the one the original's own
/// setter derives from the wire's own range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PartyHpMp(pub u8);

impl PartyHpMp {
    /// Raw low nibble, `0 ..= 11` on the wire.
    pub fn hp_nibble(&self) -> u8 {
        self.0 & 0x0F
    }

    /// Raw high nibble, `0 ..= 10` on the wire.
    pub fn mp_nibble(&self) -> u8 {
        self.0 >> 4
    }

    /// `hp` nibble 0 is not "0 % HP", it is the original's dead flag
    ///: `if (hp == 0) node+0x60 = 1`).
    pub fn is_dead(&self) -> bool {
        self.hp_nibble() == 0
    }

    /// `(n-1)/9`; empty (and [`Self::is_dead`]) at `n == 0`.
    pub fn hp_fill(&self) -> PartyBarFill {
        PartyBarFill {
            numerator: self.hp_nibble().saturating_sub(1),
            denominator: 9,
        }
    }

    /// `m/10` — 0-based, no `-1`. Type-6 frames paired against the `0x3057` MP
    /// values solve to `floor(mp*10/max)` and rule out every `+1`/ceil variant.
    pub fn mp_fill(&self) -> PartyBarFill {
        PartyBarFill {
            numerator: self.mp_nibble(),
            denominator: 10,
        }
    }
}

/// Member position in a dungeon region: full 32-bit coordinates.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyPositionDungeon {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// Member position in an overworld region: 16-bit region-local coordinates.
///
/// `y` is the height and is **signed here on purpose — a deliberate deviation**
/// (ADR-0009). Live 0x3864 updates for members standing in the Jangan field
/// carry `y = 0xFFF5`/`0xFFF1`, i.e. `-11`/`-15` read as `i16`. The original reads all three
/// as **u16 into pre-zeroed u32 slots**, so it turns every sub-zero height into a ~65 000
/// spike; nothing in its party window displays `y`, so that never surfaced
/// there. We read it signed because our consumers (world-map markers, a future
/// party window) would have to undo the spike anyway. Wire-compatible either
/// way: the two readings are the same two bytes.
/// `x`/`z` stay unsigned — they are region-local and never leave `0..=1920`.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyPositionWorld {
    pub x: u16,
    pub y: i16,
    pub z: u16,
}

/// The presence bits of a member record, and of the field
/// groups a 0x3864 type-6 update selects with the *same* numbering.
///
/// Modelled as a newtype rather than a derived enum for the same reason
/// [`PartySetup`] is: bits combine, and an unknown bit must not fail the packet.
///
/// Two independent readings agree here: the record reader
/// tests the bits in exactly this order — `0x10` u32 jid · `0x01`
/// name + u32 model · `0x02` u8 level · `0x04` u8 hp/mp · `0x20` u16 region +
/// position + u32 at `+0x54` · `0x40` guild name · `0x80` u8 at `+0x41` · `0x08`
/// two u32 masteries — and the type-6 handler dispatches
/// `test bl,1 / 2 / 4 / 0x20 / 0x40 / 8` to six separate setters
/// — i.e. it tests each bit **independently**.
/// The wire confirms it: frames carry `0x04` alone and frames carry `0x20`
/// alone, which no equality model can read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PartyMemberMask(pub u8);

impl PartyMemberMask {
    /// `name` *and* `model_id` — one bit gates both reads.
    pub const NAME: u8 = 0x01;
    pub const LEVEL: u8 = 0x02;
    /// The [`PartyHpMp`] byte.
    pub const HP_MP: u8 = 0x04;
    /// Both mastery ids.
    pub const MASTERIES: u8 = 0x08;
    /// The member jid *inside* the record. A type-6 delta carries its own jid
    /// ahead of the record, which is why this bit is clear in a delta.
    pub const MEMBER_ID: u8 = 0x10;
    /// `region`, the coordinates *and* the trailing `u32` — one bit gates all of
    /// it, which is why `position_tail` is not its own concept.
    pub const POSITION: u8 = 0x20;
    pub const GUILD: u8 = 0x40;
    /// The byte the original stores at `node+0x41`.
    pub const FLAG: u8 = 0x80;
    /// Every field present — the shape a full roster push and a join use, and
    /// the one the pre-mask fixed-layout model happened to read correctly.
    pub const ALL: u8 = 0xFF;

    pub fn has(&self, bit: u8) -> bool {
        self.0 & bit != 0
    }
}

/// One party-member record, as read by the original's shared —
/// the parser behind 0x3065's roster, 0x3864's type-2/type-6 deltas *and*
/// 0x706D's applicant block.
///
/// Every field is optional because every field is gated by `presence`; a delta
/// carries only what changed. Consumers fold these onto a stored member rather
/// than replacing it (see the client's `PartyRoster::apply_update`).
///
/// The position is region-gated exactly as elsewhere in the protocol: the high
/// bit of `region` marks a dungeon, which widens the coordinates from u16 to i32.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, Default, PartialEq)]
pub struct PartyMemberCore {
    /// See [`PartyMemberMask`]. Read first; every field below hangs off it.
    pub presence: u8,
    /// Account/character id (JID).
    #[sro_packet(when = "presence & 0x10 != 0")]
    pub member_id: Option<u32>,
    #[sro_packet(when = "presence & 0x01 != 0")]
    pub name: Option<String>,
    #[sro_packet(when = "presence & 0x01 != 0")]
    pub model_id: Option<u32>,
    #[sro_packet(when = "presence & 0x02 != 0")]
    pub level: Option<u8>,
    /// See [`PartyHpMp`] for the nibble packing.
    #[sro_packet(when = "presence & 0x04 != 0")]
    pub hp_mp: Option<u8>,
    #[sro_packet(when = "presence & 0x20 != 0")]
    pub region: Option<u16>,
    #[sro_packet(when = "presence & 0x20 != 0 && region.is_some_and(|r| r & 0x8000 != 0)")]
    pub position_dungeon: Option<PartyPositionDungeon>,
    #[sro_packet(when = "presence & 0x20 != 0 && region.is_some_and(|r| r & 0x8000 == 0)")]
    pub position_world: Option<PartyPositionWorld>,
    /// The `u32` at record+0x54, read under the *same* bit as the position. Its
    /// meaning has no reachable name in the binary — UNKNOWN, kept because
    /// dropping it would desynchronise everything after it.
    #[sro_packet(when = "presence & 0x20 != 0")]
    pub position_tail: Option<u32>,
    #[sro_packet(when = "presence & 0x40 != 0")]
    pub guild_name: Option<String>,
    /// The `u8` at record+0x41 — also UNKNOWN, also load-bearing for framing.
    #[sro_packet(when = "presence & 0x80 != 0")]
    pub flag: Option<u8>,
    #[sro_packet(when = "presence & 0x08 != 0")]
    pub mastery_primary: Option<u32>,
    #[sro_packet(when = "presence & 0x08 != 0")]
    pub mastery_secondary: Option<u32>,
}

impl PartyMemberCore {
    pub fn mask(&self) -> PartyMemberMask {
        PartyMemberMask(self.presence)
    }

    pub fn hp_mp(&self) -> Option<PartyHpMp> {
        self.hp_mp.map(PartyHpMp)
    }
}

/// 0x3065 — server → client full party roster.
///
/// The header used to be nine opaque bytes here, because xBot reads it
/// `[u32][u32][u8 purpose]` while go-sro writes `[u8 0xFF][u32 party_number]
/// [u32 master_jid]` and naming it either way would have encoded a guess. The
/// original's own handler settles it in go-sro's favour, with
/// the twist that the leading byte is a **presence flag** rather than the
/// constant `0xFF` go-sro happens to send: bit 0 gates the leader id and the
/// setup byte, bit 1 gates the count and the roster. go-sro's `0xFF` sets both,
/// which is why the byte counts coincided and xBot's misalignment never showed
/// (its "purpose" is really the top byte of `master_join_id`, hence always 0).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyData {
    /// Bit 0 — party info follows. Bit 1 — the roster follows.
    pub presence: u8,
    pub party_number: u32,
    /// The leader's JID. The original compares it against the local player's own
    /// JID to decide whether to show the "I am leader" state.
    #[sro_packet(when = "presence & 0x01 != 0")]
    pub master_join_id: Option<u32>,
    /// See [`PartySetup`].
    #[sro_packet(when = "presence & 0x01 != 0")]
    pub setup: Option<u8>,
    #[sro_packet(when = "presence & 0x02 != 0")]
    pub member_count: Option<u8>,
    #[sro_packet(list_type = "by-size-field", size_field = "member_count.unwrap_or(0)")]
    pub members: Vec<PartyMemberCore>,
}

impl PartyData {
    /// The sharing flags, or the empty set when this push carries no party info
    /// (`presence` bit 0 clear) — a roster-only update must not be read as
    /// "sharing turned off".
    pub fn setup(&self) -> PartySetup {
        PartySetup(self.setup.unwrap_or(0))
    }

    /// Whether bit 0 named the party's own attributes.
    pub fn has_party_info(&self) -> bool {
        self.presence & 0x01 != 0
    }

    /// Whether bit 1 named the roster. Distinguishes "the party has no members"
    /// (impossible) from "this push did not carry the roster" (routine).
    pub fn has_roster(&self) -> bool {
        self.presence & 0x02 != 0
    }
}

/// 0x3864 — server → client party delta, discriminated by a leading update type.
///
/// Modelled with `when`-conditional fields rather than a derived enum so an
/// unrecognised update type decodes to "no payload" instead of failing the
/// packet. The five types the original's handler branches on
/// (`local_2b4[0] == 1/2/3/6/9`) are all realised below, each with the body the
/// handler actually reads — so "type 9 has no known body" was wrong, not empty.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyUpdate {
    /// 1 dismissed · 2 joined · 3 left/kicked · 6 member update · 9 new master.
    pub update_type: u8,
    /// Type 1's tail: one `u16` the handler reads before tearing the party down
    /// (the `'\x01'` branch). A 3-byte body carries `0x000B`, which is why the
    /// field exists at all — xBot has the same read commented out as
    /// `ushort errCode`. What 11 *means* is open.
    #[sro_packet(when = "update_type == 1")]
    pub dismiss_code: Option<u16>,
    /// The joining member, as the shared mask record. There is **no** extra
    /// byte here: xBot reads one more byte in this flavour, the original has no
    /// such read, and neither does the wire — which is what collapsed the two
    /// record flavours into one type.
    #[sro_packet(when = "update_type == 2")]
    pub joined: Option<PartyMemberCore>,
    #[sro_packet(when = "update_type == 3 || update_type == 6")]
    pub member_id: Option<u32>,
    /// Type 3's tail: the handler does **two** reads in the `'\x03'` branch, the
    /// jid and one more byte, before branching on whether the departing jid is
    /// our own. A 6-byte frame carries `1` or `2` — leave versus kick is the
    /// obvious pairing but is not confirmed.
    #[sro_packet(when = "update_type == 3")]
    pub leave_reason: Option<u8>,
    /// Type 6's payload is the **same** mask record as everything else in this
    /// family — the original calls here too. It used to be its
    /// own `PartyMemberUpdate` type testing `kind` for equality, which read
    /// nothing at all for a combined mask such as `0x24` (level *and* hp/mp).
    #[sro_packet(when = "update_type == 6")]
    pub member_update: Option<PartyMemberCore>,
    /// Type 9's body: the jid of the new master. The handler reads one `u32`
    /// and `_swprintf_s`es it into a notice line. The shape is the handler's,
    /// the *width* is what the single read gives.
    #[sro_packet(when = "update_type == 9")]
    pub new_master_id: Option<u32>,
}

/// 0x306E — client → server answer to an incoming 0x706D join request. Sits in
/// the 0x3xxx range but is genuinely client → server.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyMatchJoinResponse {
    pub request_id: u32,
    pub join_id: u32,
    /// 1 accept, 0 decline.
    pub accept: u8,
}

/// 0x7060 — client → server: start a party. Whether `unique_id` is the invitee
/// or the sender is open (go-sro's handler is an empty stub).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyCreationRequest {
    pub unique_id: u32,
    /// See [`PartySetup`].
    pub setup: u8,
}

/// 0x7061 — client → server: leave the party. Empty body.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyLeave;

/// 0x7063 — client → server: kick a member by JID.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyKickRequest {
    pub join_id: u32,
}

/// 0x7069 — client → server: advertise a party in the match list. The second
/// u32 is written 0 by both the original client and go-sro; its purpose is open.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyMatchCreationRequest {
    /// 0 when creating.
    pub party_number: u32,
    pub unknown: u32,
    pub setup: u8,
    pub purpose: u8,
    pub level_min: u8,
    pub level_max: u8,
    pub title: String,
}

/// 0x706A — client → server: edit an existing match entry.
///
/// **SPEC, unverified**: the original client has an enum entry but no builder,
/// so this shape comes from go-sro's `matching_update_handler` alone. It is the
/// 0x7069 body with a real party number.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyMatchEditedRequest {
    pub party_number: u32,
    pub unknown: u32,
    pub setup: u8,
    pub purpose: u8,
    pub level_min: u8,
    pub level_max: u8,
    pub title: String,
}

/// 0x706B — client → server: withdraw a match entry.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyMatchDeleteRequest {
    pub number: u32,
}

/// 0x706C — client → server: request one page of the match list.
///
/// The original client reuses this number for `CLIENT_PET_DESTROY`; both are
/// C→S and only the match-list sender is realised in its builder, so we take the
/// party-match meaning. Open if a pet-destroy body ever contradicts it.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyMatchListRequest {
    pub page_index: u8,
}

/// 0x706D S→C — somebody asks to join our advertised party.
///
/// **Corrected** twice and independently: from the original client itself and
/// by upstream's `5520e3f7 fix(party): decode party records by their presence
/// mask` — one finding, which is why the mask model below is not a preference.
///
/// The old shape ended in a flat `u8, u32, String` taken from go-sro's builder,
/// and the original disagrees. The handler reads five `u32`s and one `u8`, and
/// then hands the rest to the shared party-member record
/// parser** — the very same function 0x3065 and 0x3864 use, which is why the
/// tail here is [`PartyMemberCore`] rather than a private copy of three
/// fields. The old flat decode was only correct for `mask == 0x11`, and even
/// then it dropped the `u32` that follows the name (bit `0x01` reads `+0x3c`);
/// any other mask mis-sliced the body from that point on.
///
/// The five leading `u32`s and the `u8` are **unnamed in the binary**. The first
/// three keep go-sro's names because the 0x306E answer is built from the first
/// two, so they are load-bearing; `D`/`E` do not keep the names
/// `mastery_primary`/`_secondary` they used to have — the masteries are the pair
/// under the record's `0x08` bit, not fixed offsets 12/16, so those names were
/// pointing at the wrong bytes.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyMatchJoinNotify {
    /// `A` — the id the 0x306E answer echoes back.
    pub request_id: u32,
    /// `B` — the applicant's jid, as the answer echoes it.
    pub join_id: u32,
    /// `C` — go-sro's match number.
    pub match_number: u32,
    /// `D` — unnamed in the binary (`+12`).
    pub unk_dword03: u32,
    /// `E` — unnamed in the binary (`+16`).
    pub unk_dword04: u32,
    /// `F` — unnamed in the binary (`+20`).
    pub unk_byte00: u8,
    /// The applicant, as the shared masked member record.
    pub applicant: PartyMemberCore,
}

impl PartyMatchJoinNotify {
    /// The applicant's name, when the record's `IDENTITY` bit carried one.
    ///
    /// An accessor rather than a field, because whether the name is on the wire
    /// is a property of the mask: a record without bit `0x01` has no name, and
    /// the dialog then has nothing to print but the jid.
    pub fn name(&self) -> Option<&str> {
        self.applicant.name.as_deref()
    }

    /// The jid *inside* the record (mask bit `0x10`), which is not the same
    /// field as [`Self::join_id`] — the header's `B` is what 0x306E answers
    /// with, this one is what the record describes. They agree in go-sro's
    /// builder; a body that disagrees would be the interesting one.
    pub fn record_join_id(&self) -> Option<u32> {
        self.applicant.member_id
    }
}

/// 0x706D — **both directions**, one codec.
///
/// The opcode is bidirectional with two unrelated bodies: S→C is
/// [`PartyMatchJoinNotify`], C→S is a bare `u32` match number. `packets!` maps
/// one type per opcode, so the two arms live in one enum that decodes the
/// inbound form and encodes the outbound one — the same construction
/// [`crate::agent::ingame::GameInvite`] uses for 0x3080.
///
/// **Why a second hand-written codec instead of a direction axis in `packets!`:**
/// there are exactly *two* opcodes in this tree that genuinely travel both ways
/// with different bodies. Of the ten numbers that appear in both the inbound and
/// the outbound verdict table, four are not C→S at all (local self-injections
/// that never reach the sender, `0x3019`/`0xB034`/`0xB04C`/`0xB082`), three are
/// unregistered with an unnamed inbound half (`0x7302`/`0x747E`/`0x751A`), and
/// `0x7110`'s registered type *is* the outbound one. That leaves `0x3080`,
/// already solved this way, and this one. Teaching `packets!` a direction axis
/// would touch all 274 registry lines, `scripts/check_opcode_ledger.py` and the
/// ledger docs for a second user.
/// **The threshold, so this does not become a habit: at the THIRD genuine
/// two-way opcode, `packets!` gets the direction axis and both hand-written
/// codecs move onto it.**
#[derive(Message, Clone, Debug, PartialEq)]
pub enum PartyMatchJoin {
    /// S→C — the applicant knocking on our advertised party.
    Notify(PartyMatchJoinNotify),
    /// C→S — *we* apply to the advertised party with this match number.
    Request { number: u32 },
}

impl TryFrom<Bytes> for PartyMatchJoin {
    type Error = SerializationError;

    /// Decoding is always the inbound arm: the client never receives its own
    /// request. A 4-byte body is therefore *not* read as a `Request` — that
    /// would silently accept a truncated notify.
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        Ok(PartyMatchJoin::Notify(PartyMatchJoinNotify::try_from(
            value,
        )?))
    }
}

impl From<PartyMatchJoin> for Bytes {
    fn from(packet: PartyMatchJoin) -> Self {
        match packet {
            PartyMatchJoin::Request { number } => {
                let mut buf = BytesMut::new();
                buf.put_u32_le(number);
                buf.freeze()
            }
            // Round-trip only: the client never sends a notify. Kept so the
            // codec is symmetric and the decode side is testable.
            PartyMatchJoin::Notify(notify) => notify.into(),
        }
    }
}

/// The advertised entry echoed back by 0xB069 and 0xB06A on success.
///
/// One type for both because the original delegates both handlers to the same
/// reader, whose seven fields match go-sro's record exactly.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyMatchForm {
    pub party_number: u32,
    pub unknown: u32,
    pub setup: u8,
    pub purpose: u8,
    pub level_min: u8,
    pub level_max: u8,
    pub title: String,
}

/// 0xB069 — server → client: result of advertising a party.
///
/// `result` is the discriminator, not a spare byte: `1` means the record
/// follows, `2` a `u16` error code. Reading the record unconditionally made a
/// real failure (a 3-byte body) run the derive off the end, so the packet was
/// dropped and the error never surfaced.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyMatchCreationResponse {
    pub result: u8,
    #[sro_packet(when = "result == 1")]
    pub entry: Option<PartyMatchForm>,
    /// Branch on `!= 1`, not `== 2`: an unexpected result must read as an error
    /// rather than as a truncated record.
    #[sro_packet(when = "result != 1")]
    pub error_code: Option<u16>,
}

/// 0xB06A — server → client: result of editing a match entry. Same shape and
/// same reader as [`PartyMatchCreationResponse`]; only the completion message
/// the original shows differs (modify vs record).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyMatchEditedResponse {
    pub result: u8,
    #[sro_packet(when = "result == 1")]
    pub entry: Option<PartyMatchForm>,
    #[sro_packet(when = "result != 1")]
    pub error_code: Option<u16>,
}

/// 0xB06B — server → client: the withdrawn entry's number, or why it failed.
///
/// `result` was a `bool` here, which deserializes as `read_u8() == 1` — so a
/// real failure (`02 <u16 code>`) decoded as a bland "not successful" and threw
/// the code away.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyMatchDeleteResponse {
    pub result: u8,
    #[sro_packet(when = "result == 1")]
    pub number: Option<u32>,
    #[sro_packet(when = "result != 1")]
    pub error_code: Option<u16>,
}

/// One advertised party in the 0xB06C list. `race_type` is byte-exact against
/// go-sro's `countryType` but its semantics (China/Europe?) are open.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyMatchEntry {
    pub number: u32,
    /// When the entry was advertised — a **unix timestamp**, seconds.
    ///
    /// go-sro calls this field `masterJID` and openroad inherited the name, but
    /// the wire refutes it: two registrations 70 seconds apart come back in the
    /// listing as two values that decode as exactly those two instants, and
    /// differ by exactly those 70 seconds. A join id would not.
    ///
    /// This matters beyond the name: it is the field one reaches for to decide
    /// which listing is your own party, and it can never answer that.
    pub registered_at: u32,
    pub master_name: String,
    pub race_type: u8,
    pub member_count: u8,
    pub setup: u8,
    pub purpose: u8,
    pub level_min: u8,
    pub level_max: u8,
    pub title: String,
}

/// The populated half of a 0xB06C response. Nested rather than flattened
/// because the whole block hangs off one `has_data` flag — the original's parser
/// opens a single `if` around all of it — and because a count that drives a
/// sized list cannot itself be conditional.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyMatchListPage {
    pub page_count: u8,
    pub page_index: u8,
    pub party_count: u8,
    #[sro_packet(list_type = "by-size-field", size_field = "party_count")]
    pub parties: Vec<PartyMatchEntry>,
}

/// 0xB06C — server → client: one page of the party-match list. Everything after
/// `has_data` is absent when there is nothing to list.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyMatchListResponse {
    pub has_data: bool,
    #[sro_packet(when = "has_data")]
    pub page: Option<PartyMatchListPage>,
}

// --- The family remainder: the four acks/pushes the seed never carried -----
//
// Idea: openroad's party family was seeded from xBot's `Agent.cs` enum, which
// declares these four but dispatches none of them, so they arrived as opcode
// numbers with no bodies (#760, #451). Their layouts are not invented here:
// SilkroadDoc-wiki documents `0xB060`, `0xB062`, `0xB06D` and `0x3068` field by
// field, and go-sro's opcode table names three of them the same way
// (`network/opcode/party.go:7,20` — `PartyCreateResponse`,
// `PartyMemberCountResponse`). Only field *layouts* are taken from the wiki —
// it carries no licence, so it is read for facts and never for code or text
// (AGENTS.md, licence table).
//
// **Attribution correction, stated because it contradicts a merged doc.**
// `docs/net-invite-0x3080.md:100,146` files `0xB060` as the *party-invite* ack
// with an UNKNOWN body. It is the **create** ack: the wiki pairs `0x7060` with
// `0xB060` (`AGENT_PARTY_CREATE`) and `0x7062` with `0xB062`
// (`AGENT_PARTY_INVITE`), and go-sro's table agrees. The invite ack that doc
// wanted is `0xB062`, modelled below. The doc lives under `docs/` and this lane
// does not rewrite RE units (runbook §5), so the correction is written here and
// on the issue.

/// 0xB060 — ack for `0x7060` create.
///
/// `result` is a raw byte with two different tails, not a bool: success carries
/// the leader's **JID**, failure a `u16` error code (11276 request denied,
/// 11280 no response, 11288 already in a party, 11301 registered in party
/// matching). Modelling it as a success bool would truncate every real error,
/// the same trap as `StallCreateResponse`.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyCreateResponse {
    pub result: u8,
    #[sro_packet(when = "result == 1")]
    pub leader_join_id: Option<u32>,
    #[sro_packet(when = "result == 2")]
    pub error_code: Option<u16>,
}

/// 0xB067 — ack for joining a party that already **exists**.
///
/// Same two-armed shape as `PartyCreateResponse`, and the same meaning for the
/// success tail: it is **our own party jid**, not a member count. go-sro's
/// opcode table labels this `PartyMemberCountResponse`
/// (`network/opcode/party.go:20`) but no handler there builds or reads it, so
/// the name is a label without a layout. The wire decides it: the `u32` always
/// equals the jid our own name carries in the `0x3065` of the same instant,
/// while the party *number* and the member *count* of those same frames are
/// different values, so the `u32` is neither. The failure arm carries
/// `02 102C`, i.e. 11280 "no response", the code `party_error_text` already
/// maps.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyJoinResponse {
    pub result: u8,
    /// Our own member jid in the party we just joined.
    #[sro_packet(when = "result == 1")]
    pub local_join_id: Option<u32>,
    #[sro_packet(when = "result == 2")]
    pub error_code: Option<u16>,
}

/// 0xB062 — ack for `0x7062` invite. Success carries nothing: the invitation
/// itself travels as the separate `0x3080` popup, so this ack only says whether
/// the request was accepted for delivery.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyInviteResponse {
    pub result: u8,
    #[sro_packet(when = "result == 2")]
    pub error_code: Option<u16>,
}

/// 0xB06D — ack for `0x706D` party-match join.
///
/// Note the two tails are **both** `u16` and mean different things: on success
/// it is a `PartyMatchingJoinResult` (the same enum `PartyMatchJoinResponse`
/// answers), on failure an error code (11292 = no such party). The failure test
/// is `!= 1`, not `== 2` — that is how the source branches it, and the two
/// differ for every other result value.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PartyMatchJoinAck {
    pub result: u8,
    #[sro_packet(when = "result == 1")]
    pub join_result: Option<u16>,
    #[sro_packet(when = "result != 1")]
    pub error_code: Option<u16>,
}

/// 0x3068 — an item dropped by the party was distributed to a member.
///
/// Hand-written for the same reason as the stall rows: the tail's *width*
/// depends on the item's class in the client's own itemdata
/// (`TID2 == 1` → one `opt_level` byte, `TID2 == 2` → nothing at all,
/// `TID2 == 3` → a `u16` quantity), and `Deserialize` cannot take an
/// [`ItemClassResolver`]. The tail is therefore kept raw behind a
/// resolver-taking accessor, exactly like `StallEntityAction::rows`.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct PartyDistribution {
    /// The member who received it.
    pub join_id: u32,
    pub ref_item_id: u32,
    /// The class-dependent tail, undecoded — read with [`Self::detail`].
    pub raw_tail: Bytes,
}

/// The class-dependent tail of a 0x3068.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartyDistributionDetail {
    /// `TID2 == 1` (equipment): the item's plus level.
    OptLevel(u8),
    /// `TID2 == 2` (containers/COS): the original triggers no message and
    /// writes no tail.
    None,
    /// `TID2 == 3` (expendables): how many.
    Quantity(u16),
}

impl PartyDistribution {
    /// Decode the tail against the client's itemdata. `None` when the class is
    /// unknown (the tail's width is then unknowable, and guessing it is what
    /// forged a fake record in `InventoryItem` once already) or the tail is
    /// short.
    pub fn detail(&self, resolver: &impl ItemClassResolver) -> Option<PartyDistributionDetail> {
        match resolver.item_class(self.ref_item_id) {
            ItemClass::Equipment => self
                .raw_tail
                .first()
                .copied()
                .map(PartyDistributionDetail::OptLevel),
            ItemClass::Container { .. } => Some(PartyDistributionDetail::None),
            ItemClass::Expendable { .. } => {
                let bytes: [u8; 2] = self.raw_tail.get(..2)?.try_into().ok()?;
                Some(PartyDistributionDetail::Quantity(u16::from_le_bytes(bytes)))
            }
            ItemClass::Unknown => None,
        }
    }
}

impl TryFrom<Bytes> for PartyDistribution {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let mut cursor = Cursor::new(&value[..]);
        let join_id = u32::read_from(&mut cursor)?;
        let ref_item_id = u32::read_from(&mut cursor)?;
        let read = cursor.position() as usize;
        Ok(PartyDistribution {
            join_id,
            ref_item_id,
            raw_tail: value.slice(read..),
        })
    }
}

impl From<PartyDistribution> for Bytes {
    fn from(p: PartyDistribution) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u32_le(p.join_id);
        buf.put_u32_le(p.ref_item_id);
        buf.extend_from_slice(&p.raw_tail);
        buf.freeze()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    /// A member record with every presence bit set, written in the original's
    /// read order. This is the shape the pre-mask fixed-layout
    /// model used to read, so it doubles as the compatibility anchor.
    fn full_member_bytes() -> Vec<u8> {
        let mut b: Vec<u8> = vec![PartyMemberMask::ALL];
        b.extend(0x1234u32.to_le_bytes()); // 0x10 member_id
        b.extend(2u16.to_le_bytes()); // 0x01 name
        b.extend(b"Ax");
        b.extend(1907u32.to_le_bytes()); // 0x01 model_id
        b.push(40); // 0x02 level
        b.push(0x3A); // 0x04 hp 100 %, mp 30 %
        b.extend(0x0100u16.to_le_bytes()); // 0x20 region, overworld
        b.extend(11u16.to_le_bytes());
        b.extend(22u16.to_le_bytes());
        b.extend(33u16.to_le_bytes());
        b.extend(0xDEADBEEFu32.to_le_bytes()); // 0x20 position tail
        b.extend(3u16.to_le_bytes()); // 0x40 guild name
        b.extend(b"Guo");
        b.push(6); // 0x80 flag
        b.extend(100u32.to_le_bytes()); // 0x08 masteries
        b.extend(200u32.to_le_bytes());
        b
    }

    /// The compatibility anchor: a full-mask record decodes every field and
    /// re-serialises to the identical bytes. If the mask rewrite had changed the
    /// *order* of any read, this is what would catch it — the full record is the
    /// only shape both the old and the new model agree on.
    #[test]
    fn a_full_mask_record_round_trips_every_field() {
        let wire = Bytes::from(full_member_bytes());

        let decoded = PartyMemberCore::try_from(wire.clone()).unwrap();

        assert_eq!(decoded.member_id, Some(0x1234));
        assert_eq!(decoded.name.as_deref(), Some("Ax"));
        assert_eq!(decoded.model_id, Some(1907));
        assert_eq!(decoded.level, Some(40));
        // 0x3A: HP nibble 10 -> (10-1)/9 = full, MP nibble 3 -> 3/10
        assert_eq!(decoded.hp_mp().unwrap().hp_fill().as_f32(), 1.0);
        assert_eq!(decoded.hp_mp().unwrap().mp_fill().percent_rounded(), 30);
        assert!(!decoded.hp_mp().unwrap().is_dead());
        assert_eq!(
            decoded.position_world,
            Some(PartyPositionWorld {
                x: 11,
                y: 22,
                z: 33
            })
        );
        assert!(decoded.position_dungeon.is_none());
        assert_eq!(decoded.position_tail, Some(0xDEAD_BEEF));
        assert_eq!(decoded.guild_name.as_deref(), Some("Guo"));
        assert_eq!(decoded.flag, Some(6));
        assert_eq!(decoded.mastery_primary, Some(100));
        assert_eq!(decoded.mastery_secondary, Some(200));

        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// The whole point of the rewrite: a record carrying one field is two bytes
    /// on the wire, not a truncated full record. The old fixed-layout model read
    /// the mask as `unk_byte01` and then demanded a `u32` member id that is not
    /// there, so every partial delta desynchronised the rest of the packet.
    #[test]
    fn a_partial_mask_record_reads_only_what_the_mask_names() {
        let wire = Bytes::from_static(&[PartyMemberMask::HP_MP, 0x5A]);

        let decoded = PartyMemberCore::try_from(wire.clone()).unwrap();

        assert_eq!(decoded.hp_mp, Some(0x5A));
        assert!(decoded.member_id.is_none());
        assert!(decoded.name.is_none());
        assert!(decoded.level.is_none());
        assert!(decoded.position_world.is_none());
        assert!(decoded.mastery_primary.is_none());
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// Bits combine. `0x24` — level *and* hp/mp in one update — is ordinary
    /// traffic that the previous `kind == 2` / `kind == 4` equality tests read as
    /// nothing at all.
    #[test]
    fn a_combined_mask_reads_both_fields() {
        let wire = Bytes::from_static(&[PartyMemberMask::LEVEL | PartyMemberMask::HP_MP, 42, 0x5A]);

        let decoded = PartyMemberCore::try_from(wire.clone()).unwrap();

        assert_eq!(decoded.level, Some(42));
        assert_eq!(decoded.hp_mp, Some(0x5A));
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// The position bit gates the region, the coordinates *and* the trailing
    /// u32 — and the region's high bit widens the coordinates from u16 to i32.
    #[test]
    fn the_position_bit_widens_the_coordinates_inside_a_dungeon() {
        let mut b: Vec<u8> = vec![PartyMemberMask::POSITION];
        b.extend(0x8001u16.to_le_bytes()); // high bit set -> dungeon
        b.extend((-5i32).to_le_bytes());
        b.extend(6i32.to_le_bytes());
        b.extend(7i32.to_le_bytes());
        b.extend(0u32.to_le_bytes()); // the tail rides the same bit
        let wire = Bytes::from(b);

        let decoded = PartyMemberCore::try_from(wire.clone()).unwrap();

        assert_eq!(
            decoded.position_dungeon,
            Some(PartyPositionDungeon { x: -5, y: 6, z: 7 })
        );
        assert!(decoded.position_world.is_none());
        assert_eq!(decoded.position_tail, Some(0));
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// 0x3065's header is mask-gated too, and bit 0 is what carries the leader —
    /// the field the client-side roster used to declare UNKNOWN.
    #[test]
    fn party_data_reads_the_leader_and_the_roster_under_their_own_bits() {
        let mut b: Vec<u8> = vec![0x03]; // both bits
        b.extend(77u32.to_le_bytes()); // party number
        b.extend(0xABCDu32.to_le_bytes()); // master join id
        b.push(PartySetup::EXP_SHARED | PartySetup::ITEM_SHARED);
        b.push(1); // member count
        b.extend(full_member_bytes());
        let wire = Bytes::from(b);

        let decoded = PartyData::try_from(wire.clone()).unwrap();

        assert!(decoded.has_party_info() && decoded.has_roster());
        assert_eq!(decoded.party_number, 77);
        assert_eq!(decoded.master_join_id, Some(0xABCD));
        assert!(decoded.setup().is_exp_shared() && decoded.setup().is_item_shared());
        assert_eq!(decoded.setup().capacity(), 8);
        assert_eq!(decoded.members.len(), 1);
        assert_eq!(decoded.members[0].mastery_secondary, Some(200));
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// A roster-only push (bit 1 alone) has no leader and no setup byte. Under
    /// the old `[u8; 9]` header those five bytes came out of the roster instead,
    /// mis-framing the first member.
    #[test]
    fn party_data_without_the_info_bit_has_no_leader_or_setup() {
        let mut b: Vec<u8> = vec![0x02];
        b.extend(77u32.to_le_bytes());
        b.push(0); // member count
        let wire = Bytes::from(b);

        let decoded = PartyData::try_from(wire.clone()).unwrap();

        assert!(!decoded.has_party_info());
        assert_eq!(decoded.master_join_id, None);
        assert_eq!(decoded.setup, None);
        // and the empty flag set is not "sharing was turned off"
        assert_eq!(decoded.setup(), PartySetup(0));
        assert!(decoded.members.is_empty());
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// Info-only (bit 0 alone): no count byte, no records.
    #[test]
    fn party_data_without_the_roster_bit_carries_no_count() {
        let mut b: Vec<u8> = vec![0x01];
        b.extend(77u32.to_le_bytes());
        b.extend(0xABCDu32.to_le_bytes());
        b.push(PartySetup::EXP_SHARED);
        let wire = Bytes::from(b);

        let decoded = PartyData::try_from(wire.clone()).unwrap();

        assert!(!decoded.has_roster());
        assert_eq!(decoded.member_count, None);
        assert!(decoded.members.is_empty());
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// An update type the original's handler does not branch on at all must
    /// decode to "no payload" rather than failing the packet. `7` is such a
    /// type; `9` is NOT one any more — it has a body (see
    /// `a_new_master_update_carries_the_masters_jid`).
    #[test]
    fn an_unknown_party_update_type_is_not_fatal() {
        let decoded = PartyUpdate::try_from(Bytes::from_static(&[7])).unwrap();

        assert_eq!(decoded.update_type, 7);
        assert!(decoded.joined.is_none());
        assert!(decoded.member_id.is_none());
        assert!(decoded.member_update.is_none());
        assert!(decoded.new_master_id.is_none());
        assert!(decoded.dismiss_code.is_none());
    }

    /// Type 3 carries the leaving member's id **and a reason byte**; type 6 adds
    /// the *same* mask record every other party opcode uses — and no extra
    /// byte (the one xBot's parser reads), which the original's reader never reads.
    #[test]
    fn party_update_reads_the_payload_its_type_selects() {
        let mut body: Vec<u8> = vec![3];
        body.extend(0xABCDu32.to_le_bytes());
        body.push(2);
        let wire = Bytes::from(body);
        let decoded = PartyUpdate::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.member_id, Some(0xABCD));
        assert_eq!(decoded.leave_reason, Some(2));
        assert!(decoded.member_update.is_none());
        let back: Bytes = decoded.into();
        assert_eq!(back, wire, "no byte of a type 3 may be left unread");

        let mut body: Vec<u8> = vec![6];
        body.extend(0xABCDu32.to_le_bytes());
        body.push(PartyMemberMask::HP_MP);
        body.push(0x5A);
        let wire = Bytes::from(body);
        let decoded = PartyUpdate::try_from(wire.clone()).unwrap();
        let update = decoded.member_update.clone().unwrap();
        assert_eq!(update.hp_mp, Some(0x5A));
        assert!(update.level.is_none());
        assert!(update.region.is_none());
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        // type 2 is the full record, with nothing extra in front of the masteries
        let mut body: Vec<u8> = vec![2];
        body.extend(full_member_bytes());
        let wire = Bytes::from(body);
        let decoded = PartyUpdate::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.joined.clone().unwrap().name.as_deref(), Some("Ax"));
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    // ---------------------------------------------------------------------
    // Wire fixtures. Every `wire` below is a body a server really sent. The
    // round-trip assert is the point: it fails if we leave a single byte unread.
    // ---------------------------------------------------------------------

    /// The byte a live server actually sends:
    /// `06 02000000 04 8b` — jid 2, mask 0x04, vitals byte `0x8B`. The low
    /// nibble is **11**, which the old `nibble * 10` turned into 110 %; the
    /// original's `(n-1)/9` makes it a full bar, and the high nibble 8 is
    /// `8/10` (that frame pairs with a 0x3057 MP of 1021/1153 = 88.6 %).
    #[test]
    fn the_vitals_byte_is_not_one_hundred_and_ten_percent() {
        let wire = Bytes::from_static(&[0x06, 0x02, 0x00, 0x00, 0x00, 0x04, 0x8B]);
        let decoded = PartyUpdate::try_from(wire.clone()).unwrap();

        assert_eq!(decoded.member_id, Some(2));
        let delta = decoded.member_update.clone().unwrap();
        assert_eq!(delta.presence, PartyMemberMask::HP_MP);
        let vitals = delta.hp_mp().unwrap();
        assert_eq!(vitals.hp_nibble(), 11);
        assert!(!vitals.is_dead());
        assert_eq!(
            vitals.hp_fill(),
            PartyBarFill {
                numerator: 10,
                denominator: 9
            }
        );
        // the fraction overshoots; the *display* clamps, and never reports 110
        assert_eq!(vitals.hp_fill().percent_rounded(), 100);
        assert_eq!(
            vitals.mp_fill(),
            PartyBarFill {
                numerator: 8,
                denominator: 10
            }
        );
        assert_eq!(vitals.mp_fill().percent_rounded(), 80);

        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// Every DISTINCT vitals byte a live server sent in a type-6 mask-0x04
    /// frame: `0x4B 0x5B 0x6B 0x7B 0x8B 0x9B 0xAB`. The low nibble is `0x0B` =
    /// 11 in all of them, so `nibble * 10` would have reported 110 % for every
    /// one of them — and the MP decile is whatever the high nibble says.
    /// Nothing here may exceed 100 %.
    #[test]
    fn no_vitals_byte_can_report_more_than_a_full_bar() {
        for byte in [0x4Bu8, 0x5B, 0x6B, 0x7B, 0x8B, 0x9B, 0xAB] {
            let vitals = PartyHpMp(byte);
            assert_eq!(vitals.hp_nibble(), 11, "{byte:#04x}");
            assert!(!vitals.is_dead());
            // the raw fraction overshoots (10/9) — that is the wire, not a bug
            assert_eq!(
                vitals.hp_fill(),
                PartyBarFill {
                    numerator: 10,
                    denominator: 9
                }
            );
            // ...and every consumer-facing form is clamped
            assert_eq!(vitals.hp_fill().as_f32(), 1.0, "{byte:#04x}");
            assert_eq!(vitals.hp_fill().percent_rounded(), 100, "{byte:#04x}");
            let mp = vitals.mp_fill();
            assert_eq!(mp.numerator, byte >> 4);
            assert!(mp.percent_rounded() <= 100, "{byte:#04x}");
        }
        // the observed MP range is 4..=10 deciles, read as plain m/10
        assert_eq!(PartyHpMp(0x4B).mp_fill().percent_rounded(), 40);
        assert_eq!(PartyHpMp(0xAB).mp_fill().percent_rounded(), 100);
    }

    /// A dead member is nibble 0 — not "0 % HP" but the original's own dead
    /// flag: `if (hp == 0) node+0x60 = 1`). Synthetic byte, real branch: a dead
    /// member has not been seen on the wire.
    #[test]
    fn hp_nibble_zero_is_the_dead_flag_and_not_a_percentage() {
        let dead = PartyHpMp(0x50);
        assert!(dead.is_dead());
        assert_eq!(dead.hp_fill().as_f32(), 0.0);
        // ...while a *live* member at the bottom of the scale is nibble 1
        let alive = PartyHpMp(0x51);
        assert!(!alive.is_dead());
        assert_eq!(alive.hp_fill().as_f32(), 0.0);
        // MP is 0-based, so nibble 0 there really is empty and means nothing else
        assert_eq!(PartyHpMp(0x01).mp_fill().numerator, 0);
    }

    /// `06 05000000 20 a861 c103 0000 8600 01000100` — 18 bytes. The trailing
    /// `u32` after the position is the field we used to leave on the wire, so
    /// this test is a byte-count test as much as a value test.
    #[test]
    fn the_position_delta_leaves_no_trailing_bytes() {
        let wire = Bytes::from_static(&[
            0x06, 0x05, 0x00, 0x00, 0x00, 0x20, 0xa8, 0x61, 0xc1, 0x03, 0x00, 0x00, 0x86, 0x00,
            0x01, 0x00, 0x01, 0x00,
        ]);
        let decoded = PartyUpdate::try_from(wire.clone()).unwrap();

        assert_eq!(decoded.member_id, Some(5));
        let delta = decoded.member_update.clone().unwrap();
        assert_eq!(delta.presence, PartyMemberMask::POSITION);
        assert_eq!(delta.region, Some(0x61a8));
        assert_eq!(
            delta.position_world,
            Some(PartyPositionWorld {
                x: 0x03c1,
                y: 0,
                z: 0x0086
            })
        );
        assert_eq!(delta.position_tail, Some(0x0001_0001));
        assert!(delta.hp_mp.is_none());

        let back: Bytes = decoded.into();
        assert_eq!(back, wire, "the u32 after the position must be consumed");
    }

    /// The same shape with `y = 0xFFE1` — the negative height that makes our
    /// signed `y` a deliberate deviation from the original's u16 read.
    #[test]
    fn a_negative_height_reads_as_a_small_negative_number() {
        let wire = Bytes::from_static(&[
            0x06, 0x05, 0x00, 0x00, 0x00, 0x20, 0xa8, 0x61, 0x00, 0x05, 0xe1, 0xff, 0xc5, 0x01,
            0x01, 0x00, 0x01, 0x00,
        ]);
        let decoded = PartyUpdate::try_from(wire.clone()).unwrap();
        let delta = decoded.member_update.clone().unwrap();

        assert_eq!(
            delta.position_world,
            Some(PartyPositionWorld {
                x: 0x0500,
                y: -31,
                z: 0x01c5
            })
        );
        // the original would have read 65505 here; same two bytes on the wire
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// `mask` is tested bit by bit by the original
    /// (`test bl,1 / 2 / 4 / 0x20 / 0x40 / 8`), so a combined delta must decode
    /// **all** its parts. Under the old equality model this body decoded to
    /// nothing at all. Composed from the two real deltas above rather than
    /// invented: mask `0x26` = position + vitals + level, in the original's
    /// read order.
    #[test]
    fn a_combined_mask_decodes_every_part_it_announces() {
        let mut body: Vec<u8> = vec![6];
        body.extend(5u32.to_le_bytes());
        body.push(PartyMemberMask::LEVEL | PartyMemberMask::HP_MP | PartyMemberMask::POSITION);
        body.push(68); // level
        body.push(0x8B); // the vitals byte
        body.extend(0x61a8u16.to_le_bytes()); // region
        body.extend(0x03c1u16.to_le_bytes()); // x
        body.extend((-31i16).to_le_bytes()); // y
        body.extend(0x0086u16.to_le_bytes()); // z
        body.extend(0x0001_0001u32.to_le_bytes());
        let wire = Bytes::from(body);

        let decoded = PartyUpdate::try_from(wire.clone()).unwrap();
        let delta = decoded.member_update.clone().unwrap();

        assert!(delta.mask().has(PartyMemberMask::LEVEL));
        assert_eq!(delta.level, Some(68));
        assert_eq!(delta.hp_mp, Some(0x8B));
        assert_eq!(delta.region, Some(0x61a8));
        assert_eq!(delta.position_world.as_ref().map(|p| p.y), Some(-31));
        assert_eq!(delta.position_tail, Some(0x0001_0001));
        assert!(delta.name.is_none() && delta.guild_name.is_none());

        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// `01 0b00` — three bytes, so the `u16` is really read; what `11` means is
    /// open.
    #[test]
    fn the_dismiss_frame_carries_its_u16() {
        let wire = Bytes::from_static(&[0x01, 0x0b, 0x00]);
        let decoded = PartyUpdate::try_from(wire.clone()).unwrap();

        assert_eq!(decoded.update_type, 1);
        assert_eq!(decoded.dismiss_code, Some(0x000B));
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// `03 04000000 02` — jid 4 left, reason 2. Both `01` and `02` occur.
    #[test]
    fn the_leave_frame_carries_its_reason_byte() {
        let wire = Bytes::from_static(&[0x03, 0x04, 0x00, 0x00, 0x00, 0x02]);
        let decoded = PartyUpdate::try_from(wire.clone()).unwrap();

        assert_eq!(decoded.member_id, Some(4));
        assert_eq!(decoded.leave_reason, Some(2));
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// Type 9 is the new master, announced as text from one `u32`
    /// (`_swprintf_s`). The shape is the handler's single read — which is
    /// exactly why an empty type 9 must not silently pass any more.
    #[test]
    fn a_new_master_update_carries_the_masters_jid() {
        let wire = Bytes::from_static(&[0x09, 0x06, 0x00, 0x00, 0x00]);
        let decoded = PartyUpdate::try_from(wire.clone()).unwrap();

        assert_eq!(decoded.update_type, 9);
        assert_eq!(decoded.new_master_id, Some(6));
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// A full member record off the wire (`02ff05…`, 41 bytes, jid 5 level 68).
    /// Pins that the full record and the masked delta agree about the `u32`
    /// after the position: it is `unk_byte02..05` here and `position_extra`
    /// there, in both.
    #[test]
    fn a_join_record_round_trips_with_the_same_trailing_u32() {
        let wire = Bytes::from(hex_body(
            "02ff0500000004004d6972618b07000044aaa861c40200000000010001000000040101000012010000",
        ));
        let decoded = PartyUpdate::try_from(wire.clone()).unwrap();

        assert_eq!(decoded.update_type, 2);
        let joined = decoded.joined.clone().unwrap();
        assert_eq!(joined.name.as_deref(), Some("Mira"));
        assert_eq!(joined.level, Some(68));
        assert_eq!(joined.member_id, Some(5));
        // mask 0xFF: every field present
        assert_eq!(joined.presence, PartyMemberMask::ALL);
        // the roster path sends the nibbles RAW (both 10 = full), unlike a delta
        assert_eq!(joined.hp_mp, Some(0xAA));
        assert_eq!(joined.position_tail, Some(0x0001_0001));
        // and there is no extra byte between the flag and the masteries: the
        // extra byte xBot reads does not exist in this record.
        assert_eq!((joined.flag, joined.mastery_primary), (Some(4), Some(257)));

        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// A whole roster off the wire (54 bytes): party number 2, master jid 6,
    /// setup 0, one member. The round-trip is the zero-leftover-bytes proof for
    /// the named header.
    #[test]
    fn a_roster_round_trips_with_its_named_header() {
        let wire = Bytes::from(hex_body(
            "ff02000000060000000001ff060000000700547261646572368b07000015aaa860870300003306010001000000040000000000000000",
        ));
        let decoded = PartyData::try_from(wire.clone()).unwrap();

        assert_eq!(decoded.presence, 0xFF);
        assert!(decoded.has_party_info() && decoded.has_roster());
        assert_eq!(decoded.party_number, 2);
        assert_eq!(decoded.master_join_id, Some(6));
        assert_eq!(decoded.setup, Some(0));
        assert!(!decoded.setup().is_exp_shared());
        assert_eq!(decoded.members.len(), 1);
        let member = &decoded.members[0];
        assert_eq!(member.name.as_deref(), Some("Trader6"));
        assert_eq!(member.member_id, Some(6));
        assert_eq!(member.level, Some(21));
        // the roster byte is 0xAA — both nibbles full, no `-1` on this path
        assert_eq!(member.hp_mp, Some(0xAA));
        assert_eq!(member.hp_mp().unwrap().hp_fill().as_f32(), 1.0);
        assert_eq!(member.hp_mp().unwrap().mp_fill().percent_rounded(), 100);

        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    fn hex_body(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// 0xB067's success tail is our own party jid (see [`PartyJoinResponse`]).
    /// The failure arm carries the same `u16` code space the rest of the family
    /// uses: `02 102C` = 11280 "no response".
    #[test]
    fn the_join_ack_carries_our_own_jid_on_success_and_a_code_on_failure() {
        let wire = Bytes::from_static(&[1, 4, 0, 0, 0]);
        let decoded = PartyJoinResponse::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.local_join_id, Some(4));
        assert_eq!(decoded.error_code, None);
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        let wire = Bytes::from_static(&[2, 0x10, 0x2C]);
        let decoded = PartyJoinResponse::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.local_join_id, None);
        assert_eq!(decoded.error_code, Some(11280));
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// 0x706D's two directions through one codec: the outbound arm is four
    /// bytes, and a four-byte *inbound* body must not be mistaken for it — that
    /// would silently accept a truncated notify.
    #[test]
    fn the_join_opcode_encodes_outbound_and_decodes_inbound() {
        let out: Bytes = PartyMatchJoin::Request { number: 42 }.into();
        assert_eq!(out, Bytes::from_static(&[0x2A, 0, 0, 0]));

        assert!(PartyMatchJoin::try_from(Bytes::from_static(&[0x2A, 0, 0, 0])).is_err());

        let mut body: Vec<u8> = Vec::new();
        for value in [7u32, 8, 9, 100, 200] {
            body.extend(value.to_le_bytes());
        }
        body.push(0);
        body.extend(full_member_bytes());
        let decoded = PartyMatchJoin::try_from(Bytes::from(body)).unwrap();
        let PartyMatchJoin::Notify(notify) = decoded else {
            panic!("an inbound 0x706D is always the notify arm");
        };
        assert_eq!(notify.name(), Some("Ax"));
        assert_eq!(notify.record_join_id(), Some(0x1234));
    }

    /// The match list's whole body hangs off one flag.
    #[test]
    fn an_empty_match_list_is_just_the_flag() {
        let wire = Bytes::from_static(&[0]);
        let decoded = PartyMatchListResponse::try_from(wire.clone()).unwrap();

        assert!(!decoded.has_data);
        assert!(decoded.page.is_none());
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// ...and round-trips a populated page.
    #[test]
    fn a_populated_match_list_round_trips() {
        let mut body: Vec<u8> = vec![1, 2, 0, 1]; // has_data, page_count, page_index, party_count
        body.extend(77u32.to_le_bytes()); // number
        body.extend(0x1234u32.to_le_bytes()); // registered_at
        body.extend(3u16.to_le_bytes());
        body.extend(b"Bob");
        body.extend([0, 4, PartySetup::EXP_SHARED, PARTY_PURPOSE_QUEST, 10, 80]);
        body.extend(2u16.to_le_bytes());
        body.extend(b"hi");
        let wire = Bytes::from(body);

        let decoded = PartyMatchListResponse::try_from(wire.clone()).unwrap();

        let page = decoded.page.clone().unwrap();
        assert_eq!(page.parties.len(), 1);
        assert_eq!(page.parties[0].master_name, "Bob");
        assert_eq!(page.parties[0].title, "hi");
        assert!(PartySetup(page.parties[0].setup).is_exp_shared());
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// 0xB06B is a three-state result, not a bool: a failure carries a `u16`
    /// code that a `success: bool` silently discarded.
    #[test]
    fn the_delete_response_keeps_its_error_code() {
        let wire = Bytes::from_static(&[1, 0x2A, 0, 0, 0]);
        let decoded = PartyMatchDeleteResponse::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.number, Some(42));
        assert_eq!(decoded.error_code, None);
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        let wire = Bytes::from_static(&[2, 0x1C, 0x2C]);
        let decoded = PartyMatchDeleteResponse::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.number, None);
        assert_eq!(decoded.error_code, Some(11292));
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// 0xB069 / 0xB06A: a failure is three bytes. Reading the seven record
    /// fields unconditionally ran the derive off the end, so the packet was
    /// dropped and the player was told nothing.
    #[test]
    fn the_match_form_acks_decode_both_arms() {
        let mut body: Vec<u8> = vec![1];
        body.extend(77u32.to_le_bytes());
        body.extend(0u32.to_le_bytes());
        body.extend([PartySetup::EXP_SHARED, PARTY_PURPOSE_HUNTING, 10, 80]);
        body.extend(2u16.to_le_bytes());
        body.extend(b"hi");
        let wire = Bytes::from(body);
        let decoded = PartyMatchCreationResponse::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.entry.clone().unwrap().title, "hi");
        assert_eq!(decoded.error_code, None);
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        let wire = Bytes::from_static(&[2, 0x0C, 0x2C]);
        let decoded = PartyMatchEditedResponse::try_from(wire.clone()).unwrap();
        assert!(decoded.entry.is_none());
        assert_eq!(decoded.error_code, Some(11276));
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// Sharing EXP raises the member cap from 4 to 8.
    #[test]
    fn the_setup_bitfield_decodes_independently() {
        let both = PartySetup(PartySetup::EXP_SHARED | PartySetup::ITEM_SHARED);
        assert!(both.is_exp_shared() && both.is_item_shared());
        assert!(!both.anyone_can_invite());
        assert_eq!(both.capacity(), 8);
        assert_eq!(PartySetup(PartySetup::ITEM_SHARED).capacity(), 4);
    }

    /// 0xB060 is the CREATE ack, not the invite ack: success carries the
    /// leader's JID and failure a u16 code, so a bare success bool would eat
    /// every real error (and `docs/net-invite-0x3080.md` files it under the
    /// wrong request — see the module note).
    #[test]
    fn the_create_ack_carries_a_jid_on_success_and_a_code_on_failure() {
        let wire = Bytes::from_static(&[1, 0x2A, 0, 0, 0]);
        let decoded = PartyCreateResponse::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.leader_join_id, Some(42));
        assert_eq!(decoded.error_code, None);
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        // 11288 = already in another party
        let wire = Bytes::from_static(&[2, 0x18, 0x2C]);
        let decoded = PartyCreateResponse::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.error_code, Some(11288));
        assert_eq!(decoded.leader_join_id, None);
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// 0xB062's success case is empty on purpose: the invitation itself is the
    /// separate 0x3080 popup.
    #[test]
    fn the_invite_ack_is_empty_on_success() {
        let decoded = PartyInviteResponse::try_from(Bytes::from_static(&[1])).unwrap();
        assert_eq!(decoded.result, 1);
        assert_eq!(decoded.error_code, None);

        let wire = Bytes::from_static(&[2, 0x0C, 0x2C]);
        let decoded = PartyInviteResponse::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.error_code, Some(11276));
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// 0xB06D branches on `result == 1`, not `== 2`: both tails are u16 and
    /// they mean different things, so a result of 3 must read as an ERROR and
    /// not as a join result.
    #[test]
    fn the_match_join_ack_branches_on_result_one_not_two() {
        let wire = Bytes::from_static(&[1, 2, 0]);
        let decoded = PartyMatchJoinAck::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.join_result, Some(2));
        assert_eq!(decoded.error_code, None);
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);

        // 11292 = cannot find corresponding party
        let wire = Bytes::from_static(&[3, 0x1C, 0x2C]);
        let decoded = PartyMatchJoinAck::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.join_result, None);
        assert_eq!(decoded.error_code, Some(11292));
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// 0x706D's tail is the shared member record, so the applicant's guild and
    /// level arrive whenever the server's mask names them — the flat
    /// `unk_byte01, join_id_repeated, name` model was right only for mask 0x11
    /// and dropped everything after the name.
    #[test]
    fn the_join_notify_ends_in_a_member_record() {
        let mut body: Vec<u8> = Vec::new();
        for value in [7u32, 8, 9, 100, 200] {
            body.extend(value.to_le_bytes());
        }
        body.push(0); // unk_byte00
        body.extend(full_member_bytes());
        let wire = Bytes::from(body);

        let decoded = PartyMatchJoinNotify::try_from(wire.clone()).unwrap();

        assert_eq!(decoded.request_id, 7);
        assert_eq!(decoded.join_id, 8);
        assert_eq!(decoded.match_number, 9);
        assert_eq!(decoded.applicant.name.as_deref(), Some("Ax"));
        assert_eq!(decoded.applicant.guild_name.as_deref(), Some("Guo"));
        assert_eq!(decoded.applicant.level, Some(40));
        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }

    /// 0x3068's tail width is a property of the ITEM, not of the packet: one
    /// byte for equipment, nothing for a container, two bytes for an
    /// expendable — and unknown when the class is unknown, because a guessed
    /// width is how a fake record gets forged.
    #[test]
    fn the_distribution_tail_is_read_against_the_item_class() {
        struct Fixed(ItemClass);
        impl ItemClassResolver for Fixed {
            fn item_class(&self, _ref_id: u32) -> ItemClass {
                self.0
            }
        }

        let mut body = 7u32.to_le_bytes().to_vec();
        body.extend(4242u32.to_le_bytes());
        body.push(9); // opt level, or the low byte of a quantity
        body.push(0);
        let wire = Bytes::from(body);

        let decoded = PartyDistribution::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.join_id, 7);
        assert_eq!(decoded.ref_item_id, 4242);
        assert_eq!(
            decoded.detail(&Fixed(ItemClass::Equipment)),
            Some(PartyDistributionDetail::OptLevel(9))
        );
        assert_eq!(
            decoded.detail(&Fixed(ItemClass::Expendable { tid3: 0, tid4: 0 })),
            Some(PartyDistributionDetail::Quantity(9))
        );
        assert_eq!(
            decoded.detail(&Fixed(ItemClass::Container { tid3: 0, tid4: 0 })),
            Some(PartyDistributionDetail::None)
        );
        assert_eq!(
            decoded.detail(&Fixed(ItemClass::Unknown)),
            None,
            "an unknown class must not guess the tail's width"
        );

        let back: Bytes = decoded.into();
        assert_eq!(back, wire);
    }
}
