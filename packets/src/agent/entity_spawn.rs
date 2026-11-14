//! Decodes the vSRO 1.88 group-spawn payload (0x3019) into entities to spawn or
//! despawn.
//!
//! Idea: the payload is a `count`-long list (count from the preceding
//! `GroupEntitySpawnBegin`) of variable-length, type-dependent records — a
//! player record embeds an equipment ref-id list whose per-item width depends on
//! whether each item is equipment, which needs itemdata. That is the same reason
//! `CharacterDataBody` is a raw passthrough, so the whole payload is carried
//! unparsed and decoded here, against the characterdata/itemdata tables the
//! caller holds ([`RefResolver`]). Each record starts with a `ref_id` (u32); its
//! type (player / NPC / monster / dropped item) is resolved from that id against
//! those tables, which selects the per-record layout. Because records are variable-length and
//! back-to-back, a mis-parse can't recover the next record boundary, so on any
//! unknown type or short read we stop the batch and return what parsed so far
//! (fail-safe, mirroring `character_data.rs`) rather than desync.
//!
//! Layout is calibrated against a live vSRO 1.88 capture: the player and
//! NPC records (incl. the `tag + u8 count + option bytes` interaction list) are
//! verified; the monster rarity tail and the dropped-item branch are still
//! cross-referenced with skrillax and pending a capture.
//!
//! COS (pet/mount) records and the mounted-player conditional are [S]
//! spec-derived from xBot `PacketParser.cs:735-807` (no capture yet — promoted
//! to [V] by CAPTURE_LIST F3/F7/F8): a COS record is the NPC record plus a
//! tid4-dependent owner tail, and a riding player inserts a `u32` mount uid
//! between its state flags — both used to desync the whole batch.

use std::collections::HashMap;

use std::fmt::{self, Display, Formatter};

use crate::agent::character_data::EntityState;
use crate::hexdump;

use crate::agent::pet::CosKind;
use crate::agent::spawn_reader::{GuildTag, Reader};

pub use crate::agent::character_data::SpawnPosition;

/// The kind of a spawned entity, with the per-kind extras the spawn system uses.
#[derive(Debug, Clone, PartialEq)]
pub enum SpawnKind {
    /// A remote player character; `equipment` are the equipped/avatar item
    /// `(ref id, +N opt level)` pairs to attach. `riding_uid` is the unique id
    /// of the COS the player is mounted on (`None` when on foot).
    Player {
        equipment: Vec<(u32, u8)>,
        riding_uid: Option<u32>,
    },
    Npc,
    /// A monster; `extra` is the byte the original client reads after the
    /// rarity of a thief or hunter monster (TypeID4 2 and 3)
    /// [V original handler], its meaning [U].
    Monster {
        extra: Option<u8>,
    },
    /// A COS (pet/mount/transport). `pet_name` is the owner-given name
    /// (attack/pick pets only); `owner_uid` links back to the summoner
    /// (absent on ride-only horses, which carry no owner tail at all).
    Cos {
        kind: CosKind,
        pet_name: Option<String>,
        owner_name: Option<String>,
        owner_uid: Option<u32>,
    },
    /// A dropped item; `amount` is the gold pile size (gold only). `name` is
    /// the name an item of TypeID 3/3/8 or 3/3/9 carries
    /// [V original handler]; the captured ones carry what looks like a
    /// character's name [U].
    Item {
        amount: Option<u32>,
        name: Option<String>,
    },
    /// A world structure (teleport gate building) — spawns as an invisible
    /// click anchor.
    Structure,
}

/// One entity to spawn.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnedEntity {
    pub ref_id: u32,
    pub unique_id: u32,
    pub position: SpawnPosition,
    /// The character's name (players only; `None` otherwise).
    pub name: Option<String>,
    /// The player's guild, or `None` for a guildless player and every
    /// non-player kind. The block is always on the wire for players — a
    /// guildless one just sends an empty name.
    pub guild: Option<GuildTag>,
    /// Life/motion/body states and movement speeds (characters only; `None`
    /// for dropped items, which carry no state block).
    pub state: Option<EntityState>,
    /// The per-instance rarity byte at the end of a monster record (`None` for
    /// every other kind). go-sro fills it from characterdata's rarity column;
    /// the spawn system logs a mismatch but keys the badge off characterdata.
    pub spawn_rarity: Option<u8>,
    /// Interaction option ids of a talkable NPC (empty for everything else) —
    /// the entries of its talk dialog (talk/store/storage/teleport…, see
    /// `plugins::cursor::interactions::npcs::TalkOption`). They are the
    /// interaction block's list, read for its flag bit 2.
    pub talk_options: Vec<u8>,
    /// The rest of the interaction block of an NPC, monster or COS record
    /// (`None` for every other kind).
    pub interaction: Option<Interaction>,
    pub kind: SpawnKind,
}

/// The interaction block as the original client reads it
/// [V original handler]: a flag byte, then a text for bit 1, the talk options
/// for bit 2 (kept in [`SpawnedEntity::talk_options`], the one place of that
/// list) and for bit 4 a type byte (1 or 2) and a u32. The meanings of the
/// text and of the typed value are [U].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Interaction {
    pub flags: u8,
    pub text: Option<String>,
    pub typed_value: Option<(u8, u32)>,
}

/// The outcome of decoding a group-spawn payload.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupSpawnParse {
    pub spawns: Vec<SpawnedEntity>,
    pub despawns: Vec<u32>,
    /// Ref ids the batch had to skip: present in the payload, in no client
    /// table, so unrenderable. Kept for diagnostics — the entities *around*
    /// them are in `spawns` (#426).
    pub unresolved: Vec<u32>,
    /// What the parse could not act on, in order, for the caller to log.
    pub notes: Vec<SpawnNote>,
}

/// A batch the parser did not read to its end, or a record it skipped. The
/// parser holds no logger, so it hands these back; `Display` is the line the
/// caller logs.
#[derive(Debug, Clone, PartialEq)]
pub enum SpawnNote {
    /// The despawn list ended after `at` of its `count` ids.
    DespawnTruncated { at: u16, count: u16 },
    /// Record `index` is an NPC or a monster and nothing says which; the rest
    /// of the batch is not read.
    Undecided { index: u16, count: u16 },
    /// Record `index` has a ref id in no client table; its `width` bytes,
    /// derived from the payload, were skipped.
    Skipped {
        index: u16,
        count: u16,
        ref_id: u32,
        width: usize,
    },
    /// Record `index` could not be read from `start` on (the parse stopped at
    /// `stop`); the rest of the batch is not read. `record` is the payload from
    /// `start`.
    Stopped {
        index: u16,
        count: u16,
        ref_id: Option<u32>,
        ref_type: Option<RefType>,
        start: usize,
        stop: usize,
        record: Vec<u8>,
    },
}

impl Display for SpawnNote {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            SpawnNote::DespawnTruncated { at, count } => {
                write!(
                    f,
                    "entity_spawn: despawn list truncated at {}/{}",
                    at, count
                )
            }
            SpawnNote::Undecided { index, count } => write!(
                f,
                "entity_spawn: record {}/{} is an NPC or a monster and no characterdata \
                 says which; the batch ends on its last byte either way, or neither \
                 way, so the rest of it is not read",
                index, count,
            ),
            SpawnNote::Skipped {
                index,
                count,
                ref_id,
                width,
            } => write!(
                f,
                "entity_spawn: record {}/{} ref_id={} is in no client table; \
                 skipped its {} bytes (width derived from the payload) and kept \
                 the rest of the batch",
                index, count, ref_id, width,
            ),
            SpawnNote::Stopped {
                index,
                count,
                ref_id,
                ref_type,
                start,
                stop,
                record,
            } => write!(
                f,
                "entity_spawn: could not parse spawn record {}/{} ref_id={:?} type={:?}; \
                 record started at {}, parse stopped at {}. record bytes: {}",
                index,
                count,
                ref_id,
                ref_type,
                start,
                stop,
                hexdump(record, 192),
            ),
        }
    }
}

/// What a record's leading `ref_id` resolves to, which selects the record parser.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RefType {
    Player,
    /// A monster. `extra`: the original client reads one byte more after its
    /// rarity for thief and hunter monsters (TypeID4 2 and 3)
    /// [V original handler].
    Monster {
        extra: bool,
    },
    Npc,
    /// An NPC or a monster, undecided: a resolver without characterdata (the
    /// headless net check) cannot tell them apart. The two records differ only
    /// by the monster's trailing rarity byte, so the rest of the batch has to
    /// settle it (see `settle_character`). A thief or hunter monster, one byte
    /// longer still, is not among the readings: it needs characterdata.
    Character,
    /// A COS (characterdata `1/2/3/tid4`) — NPC record plus an owner tail.
    Cos {
        kind: CosKind,
    },
    /// A dropped item, with the sub-shape needed to parse its record: its
    /// head carries an opt level (equipment), a name (`named`, TypeID
    /// 3/3/8 and 3/3/9) or a gold amount (gold), never two of them.
    Item {
        equipment: bool,
        gold: bool,
        named: bool,
    },
    /// A gate building (teleportbuilding.txt ref) — a fixed structure record,
    /// not a character body.
    Structure,
    /// Not found in any table — record layout unknown, so the batch must stop.
    Unknown,
}

/// Resolves entity/item ref ids to their type. The tables live with the
/// caller (the client's `TextdataResolver`), so the parser takes this lookup
/// as a trait, and the tests a small one of their own.
pub trait RefResolver {
    /// The type of an *entity* ref id (the record's leading id).
    fn resolve(&self, ref_id: u32) -> RefType;
    /// Whether an *item* ref id (inside a player's equipment list) is equipment,
    /// which decides its extra optimization-level byte.
    fn item_is_equipment(&self, ref_id: u32) -> bool;
}

/// Decode a group-spawn payload. `spawning` is the `kind` from
/// `GroupEntitySpawnBegin` (spawn vs despawn); `count` is its entity count.
/// Never panics: a short or unrecognized record stops the batch and returns the
/// records parsed so far.
pub fn parse_group_spawn(
    raw: &[u8],
    spawning: bool,
    count: u16,
    resolver: &impl RefResolver,
) -> GroupSpawnParse {
    let mut out = GroupSpawnParse::default();
    let mut r = Reader::new(raw);

    if !spawning {
        for i in 0..count {
            match r.u32() {
                Some(id) => out.despawns.push(id),
                None => {
                    out.notes.push(SpawnNote::DespawnTruncated { at: i, count });
                    break;
                }
            }
        }
        return out;
    }

    let mut fit = BatchFit::new(raw, resolver);
    let mut i = 0;
    while i < count {
        let record_start = r.pos();
        match parse_spawn_record(&mut r, resolver) {
            Some(entity) => match settle_character(entity, &mut r, count - i - 1, &mut fit) {
                Some(entity) => out.spawns.push(entity),
                None => {
                    out.notes.push(SpawnNote::Undecided { index: i, count });
                    break;
                }
            },
            None => {
                // An unclassifiable ref id has no derivable record width, so
                // the whole batch used to end here — losing every entity
                // behind it (#426). Try to derive the width from the payload
                // instead and carry on without that one entity (it has no
                // characterdata row, so it could not be rendered anyway).
                let unresolved = raw
                    .get(record_start..record_start + 4)
                    .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                    .filter(|id| resolver.resolve(*id) == RefType::Unknown);
                if let Some(ref_id) = unresolved {
                    if let Some(resume) =
                        recover_unknown_record(raw, record_start, count - i - 1, &mut fit)
                            .filter(|resume| r.seek(*resume).is_some())
                    {
                        out.notes.push(SpawnNote::Skipped {
                            index: i,
                            count,
                            ref_id,
                            width: resume - record_start,
                        });
                        out.unresolved.push(ref_id);
                        i += 1;
                        continue;
                    }
                }
                // Rich diagnostics so a live capture pins the record layout: the
                // ref id, what it resolved to, where the record began, how far the
                // parse got, and the record's own bytes (not just the head).
                let ref_id = raw
                    .get(record_start..record_start + 4)
                    .map(|b| u32::from_le_bytes(b.try_into().unwrap()));
                let ref_type = ref_id.map(|id| resolver.resolve(id));
                out.notes.push(SpawnNote::Stopped {
                    index: i,
                    count,
                    ref_id,
                    ref_type,
                    start: record_start,
                    stop: r.pos(),
                    record: raw.get(record_start..).unwrap_or(&[]).to_vec(),
                });
                break;
            }
        }
        i += 1;
    }
    out
}

/// Derive the width of a record whose ref id is in no client table, so the
/// batch can continue behind it.
///
/// Idea: the ref id cannot be classified (the server's characterdata has rows
/// this client's tables lack — live refs 9251/9252/36030 sit in `npcpos.txt`
/// but in no `characterdata_*.txt`), so the record's layout is unknown. Its
/// *width* is still decidable from the payload: assume the record's body is
/// `n` bytes, parse the records that must follow it, and keep `n` only if they
/// all parse and land exactly on the end of the payload — a group-spawn body
/// holds whole records and nothing else (verified over
/// `packet_dump/0x3019.log`: every spawn payload is consumed to its last byte
/// by its `GroupEntitySpawnBegin` count). A wrong `n` shifts every following
/// record, so it practically never survives that. Accepted only when exactly
/// one `n` does; anything else keeps the batch-abort behaviour.
fn recover_unknown_record<R: RefResolver>(
    raw: &[u8],
    record_start: usize,
    remaining: u16,
    fit: &mut BatchFit<'_, R>,
) -> Option<usize> {
    let body_start = record_start.checked_add(4)?;
    let max_body = raw.len().checked_sub(body_start)?;
    let mut found = None;
    for body_len in 0..=max_body {
        let resume = body_start + body_len;
        if !fit.fits(resume, remaining) {
            continue;
        }
        if found.is_some() {
            // Two widths both explain the payload: no basis to pick one.
            return None;
        }
        found = Some(resume);
    }
    found
}

/// Whether the records from an offset read to exactly the end of the payload —
/// the property [`recover_unknown_record`] and [`settle_character`] decide by.
/// Memoised per `(offset, records left)`: an undecided record offers two
/// readings, and without the memo a batch of them would be searched once per
/// combination.
struct BatchFit<'a, R> {
    raw: &'a [u8],
    resolver: &'a R,
    memo: HashMap<(usize, u16), bool>,
}

impl<'a, R: RefResolver> BatchFit<'a, R> {
    fn new(raw: &'a [u8], resolver: &'a R) -> Self {
        Self {
            raw,
            resolver,
            memo: HashMap::new(),
        }
    }

    /// Whether `remaining` records starting at `pos` end on the payload's last
    /// byte.
    fn fits(&mut self, pos: usize, remaining: u16) -> bool {
        if remaining == 0 {
            return pos == self.raw.len();
        }
        if let Some(&known) = self.memo.get(&(pos, remaining)) {
            return known;
        }
        let ok = self
            .record_ends(pos)
            .into_iter()
            .any(|end| self.fits(end, remaining - 1));
        self.memo.insert((pos, remaining), ok);
        ok
    }

    /// Where the record at `pos` can end: once for a resolved type, twice for
    /// an undecided character (as an NPC, or one byte later as a monster),
    /// never for an unknown or short record.
    fn record_ends(&self, pos: usize) -> Vec<usize> {
        let mut r = Reader::new(self.raw);
        let Some(entity) = r
            .seek(pos)
            .and_then(|()| parse_spawn_record(&mut r, self.resolver))
        else {
            return Vec::new();
        };
        if self.resolver.resolve(entity.ref_id) == RefType::Character {
            vec![r.pos(), r.pos() + 1]
        } else {
            vec![r.pos()]
        }
    }
}

/// An undecided character record, read as an NPC by [`parse_spawn_record`],
/// is settled by the rest of the batch: an NPC when only that reading lets the
/// batch end on its last byte, a monster (rarity byte included) when only the
/// one-byte-longer reading does. Anything else leaves it undecided (`None`):
/// measured on our stub's batches (2026-11-11), seven NPCs in a row fit 64
/// readings, and without characterdata nothing picks the true one.
/// A thief or hunter monster, two bytes past the NPC reading, is not tried:
/// the batch stops at it, undecided.
fn settle_character<R: RefResolver>(
    mut entity: SpawnedEntity,
    r: &mut Reader,
    remaining: u16,
    fit: &mut BatchFit<'_, R>,
) -> Option<SpawnedEntity> {
    if fit.resolver.resolve(entity.ref_id) != RefType::Character {
        return Some(entity);
    }
    let end = r.pos();
    match (fit.fits(end, remaining), fit.fits(end + 1, remaining)) {
        (true, false) => Some(entity),
        (false, true) => {
            entity.spawn_rarity = Some(r.u8()?);
            entity.kind = SpawnKind::Monster { extra: None };
            Some(entity)
        }
        _ => None,
    }
}

fn parse_spawn_record(r: &mut Reader, resolver: &impl RefResolver) -> Option<SpawnedEntity> {
    let ref_id = r.u32()?;
    match resolver.resolve(ref_id) {
        RefType::Player => parse_player(r, ref_id, resolver),
        RefType::Monster { extra } => parse_character(r, ref_id, Some(extra)),
        // An undecided record is read as an NPC; whether a monster's rarity
        // byte follows is the batch's call (`settle_character`).
        RefType::Npc | RefType::Character => parse_character(r, ref_id, None),
        RefType::Cos { kind } => parse_cos(r, ref_id, kind),
        RefType::Item {
            equipment,
            gold,
            named,
        } => parse_item(r, ref_id, equipment, gold, named),
        RefType::Structure => parse_structure(r, ref_id),
        RefType::Unknown => None,
    }
}

/// Gate-building record — 36 bytes, captured 40× in `packet_dump/0x3019.log`
/// (Jangan dimensional gate, always its own count-1 batch):
/// `2e080000 0c000000 a861 00c09c44 0000c0c0 00c0ab44 0000` + 12-byte tail
/// `01 00 00 01 00 00 00 00 00 00 00 00` = ref 2094, uid 12, region 25000,
/// (1254, -6, 1374), heading 0 — exactly the teleportbuilding.txt row's
/// position. The tail's layout is unknown (no movement/state block fits in
/// 12 bytes) and is skipped whole; the constant `01 00 00 01` prefix will
/// flag any variant via the batch-abort diagnostics if other gates differ.
fn parse_structure(r: &mut Reader, ref_id: u32) -> Option<SpawnedEntity> {
    let unique_id = r.u32()?;
    let position = r.position()?;
    r.skip(12)?;
    Some(SpawnedEntity {
        ref_id,
        unique_id,
        position,
        name: None,
        guild: None,
        state: None,
        spawn_rarity: None,
        talk_options: Vec::new(),
        interaction: None,
        kind: SpawnKind::Structure,
    })
}

fn parse_player(r: &mut Reader, ref_id: u32, resolver: &impl RefResolver) -> Option<SpawnedEntity> {
    // scale, hwan_level, pvp_cape, auto_invest_xp, base_inventory_size
    r.skip(5)?;
    let mut equipment = Vec::new();
    read_item_list(r, resolver, &mut equipment)?; // inventory (equipped gear)
    let _avatar_slots = r.u8()?;
    read_item_list(r, resolver, &mut equipment)?; // avatar items
    let _has_mask = r.u8()?;

    let unique_id = r.u32()?;
    let position = r.position()?;
    r.skip_movement(position.region)?;
    let state = r.character_state()?;
    let name = r.string()?;
    // job_type, job_level, pk_state
    r.skip(3)?;
    // A mounted player inserts its COS's unique id between the riding and
    // scroll flags ([S] xBot PacketParser.cs:744-747; the old flat skip(8)
    // desynced the batch by 4 bytes whenever a rider was in view).
    let riding = r.u8()? != 0;
    let _in_combat = r.u8()?;
    let riding_uid = if riding { Some(r.u32()?) } else { None };
    // scroll, interact, unk
    r.skip(3)?;
    // guildless players send an empty name here, not an absent block
    let guild = r.guild()?;
    // equipment_cooldown, pk_flag(0xFF)
    r.skip(2)?;

    Some(SpawnedEntity {
        ref_id,
        unique_id,
        position,
        name: Some(name),
        guild: (!guild.name.is_empty()).then_some(guild),
        state: Some(state),
        spawn_rarity: None,
        talk_options: Vec::new(),
        interaction: None,
        kind: SpawnKind::Player {
            equipment,
            riding_uid,
        },
    })
}

/// Read a `u8`-count-prefixed equipment list, appending each item's
/// `(ref id, opt level)` to `out`. Equipment items carry a trailing
/// enhancement (+N opt) byte; non-equipment (avatar attachments without
/// one) records 0.
fn read_item_list(
    r: &mut Reader,
    resolver: &impl RefResolver,
    out: &mut Vec<(u32, u8)>,
) -> Option<()> {
    let count = r.u8()?;
    for _ in 0..count {
        let item_ref = r.u32()?;
        let opt_level = if resolver.item_is_equipment(item_ref) {
            r.u8()?
        } else {
            0
        };
        out.push((item_ref, opt_level));
    }
    Some(())
}

/// The interaction block ([`Interaction`]); its talk options go to
/// `talk_options`.
fn read_interaction(r: &mut Reader, talk_options: &mut Vec<u8>) -> Option<Interaction> {
    let flags = r.u8()?;
    let text = if flags & 1 != 0 {
        Some(r.string()?)
    } else {
        None
    };
    if flags & 2 != 0 {
        let count = r.u8()?;
        for _ in 0..count {
            talk_options.push(r.u8()?);
        }
    }
    let typed_value = if flags & 4 != 0 {
        Some((r.u8()?, r.u32()?))
    } else {
        None
    };
    Some(Interaction {
        flags,
        text,
        typed_value,
    })
}

/// An NPC (`monster` None) or a monster: the NPC record plus the rarity, and
/// for a thief or hunter monster one byte more.
fn parse_character(r: &mut Reader, ref_id: u32, monster: Option<bool>) -> Option<SpawnedEntity> {
    let unique_id = r.u32()?;
    let position = r.position()?;
    r.skip_movement(position.region)?;
    let state = r.character_state()?;
    let mut talk_options = Vec::new();
    let interaction = read_interaction(r, &mut talk_options)?;
    let (spawn_rarity, kind) = match monster {
        Some(extra) => {
            let rarity = r.u8()?;
            let extra = if extra { Some(r.u8()?) } else { None };
            (Some(rarity), SpawnKind::Monster { extra })
        }
        None => (None, SpawnKind::Npc),
    };
    Some(SpawnedEntity {
        ref_id,
        unique_id,
        position,
        name: None,
        guild: None,
        state: Some(state),
        spawn_rarity,
        talk_options,
        interaction: Some(interaction),
        kind,
    })
}

/// COS record ([S] xBot `PacketParser.cs:791-807`): the shared NPC head
/// (uid, position, movement, state, talk options), then an owner tail selected
/// by the ref's characterdata tid4 — the wire carries **no** subtype byte.
/// Ride-only horses (tid4 1) have no tail; everything else ends in
/// `OwnerUniqueID`, with pets prefixing their given name, pick pets omitting
/// the PVP-state byte and guild guards inserting an `OwnerObjectID`. Routing
/// these rows through the plain NPC parse (the pre-COS behavior) left the tail
/// unconsumed and desynced every later record in the batch.
fn parse_cos(r: &mut Reader, ref_id: u32, kind: CosKind) -> Option<SpawnedEntity> {
    let unique_id = r.u32()?;
    let position = r.position()?;
    r.skip_movement(position.region)?;
    let state = r.character_state()?;
    // The interaction block is shared with NPCs (a summoned COS sends flag 0).
    let mut talk_options = Vec::new();
    let interaction = read_interaction(r, &mut talk_options)?;
    let mut pet_name = None;
    let mut owner_name = None;
    let mut owner_uid = None;
    if kind != CosKind::Vehicle {
        if matches!(kind, CosKind::GrowthPet | CosKind::GrabPet) {
            pet_name = Some(r.string()?).filter(|n| !n.is_empty());
        }
        owner_name = Some(r.string()?).filter(|n| !n.is_empty());
        let _job_type = r.u8()?;
        if kind != CosKind::GrabPet {
            let _pvp_state = r.u8()?;
        }
        if kind == CosKind::Fellow {
            let _owner_obj_id = r.u32()?;
        }
        owner_uid = Some(r.u32()?);
    }
    Some(SpawnedEntity {
        ref_id,
        unique_id,
        position,
        name: None,
        guild: None,
        state: Some(state),
        spawn_rarity: None,
        talk_options,
        interaction: Some(interaction),
        kind: SpawnKind::Cos {
            kind,
            pet_name,
            owner_name,
            owner_uid,
        },
    })
}

/// Dropped item record: `ref u32, [upgrade u8 | name s16 | amount u32], uid
/// u32, pos, owner flag(+u32), rarity u8` — the owner a flagged optional, as
/// in skrillax's `ItemSpawnData`; the name as the original client reads it for
/// TypeID 3/3/8 and 3/3/9 [V original handler].
///
/// The record ends at the rarity byte. A single-entity spawn (0x3015) follows
/// it with a drop-source byte and, for sources 5 and 6 only, the dropper's uid
/// `[V original handler]`; the group frame (0x3019) has no such tail — a gold
/// pile then a monster arrive as 30 + 55 bytes `[V capture, peer vSRO]`, see
/// `a_group_drop_ends_at_its_rarity_byte`. The single frame's tail is left
/// unread; nothing behind it in that frame needs it.
fn parse_item(
    r: &mut Reader,
    ref_id: u32,
    equipment: bool,
    gold: bool,
    named: bool,
) -> Option<SpawnedEntity> {
    let (mut amount, mut name) = (None, None);
    if equipment {
        r.skip(1)?; // upgrade / opt level
    } else if named {
        name = Some(r.string()?);
    } else if gold {
        amount = Some(r.u32()?);
    }
    let unique_id = r.u32()?;
    let position = r.position()?;
    // owner: flag byte + optional owner id
    if r.u8()? != 0 {
        r.skip(4)?;
    }
    // The drop's rarity class, kept rather than skipped: it is the only thing
    // on the wire that says whether the pile is a plain, a "blue" (magic-option)
    // or a Seal-grade drop. A ground drop carries NO item body — no opt level,
    // no `mag_params` — so this byte is the sole source for the drop label's
    // colour. Values beyond 0/1 are UNKNOWN and are carried through unmapped
    // rather than guessed at.
    let rarity = r.u8()?;
    Some(SpawnedEntity {
        ref_id,
        unique_id,
        position,
        name: None,
        guild: None,
        state: None,
        spawn_rarity: Some(rarity),
        talk_options: Vec::new(),
        interaction: None,
        kind: SpawnKind::Item { amount, name },
    })
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::HashMap;

    /// A resolver driven by an explicit ref-id → type map (unknown ids resolve
    /// to `Unknown`).
    struct MockResolver {
        types: HashMap<u32, RefType>,
    }

    impl RefResolver for MockResolver {
        fn resolve(&self, ref_id: u32) -> RefType {
            self.types.get(&ref_id).copied().unwrap_or(RefType::Unknown)
        }
        fn item_is_equipment(&self, ref_id: u32) -> bool {
            matches!(
                self.types.get(&ref_id),
                Some(RefType::Item {
                    equipment: true,
                    ..
                })
            )
        }
    }

    /// A byte builder mirroring the server's little-endian writes.
    #[derive(Default)]
    struct Body(Vec<u8>);
    impl Body {
        fn u8(mut self, v: u8) -> Self {
            self.0.push(v);
            self
        }
        fn u16(mut self, v: u16) -> Self {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn u32(mut self, v: u32) -> Self {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn f32(mut self, v: f32) -> Self {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn string(mut self, s: &str) -> Self {
            self.0.extend_from_slice(&(s.len() as u16).to_le_bytes());
            self.0.extend_from_slice(s.as_bytes());
            self
        }
        fn raw(mut self, bytes: &[u8]) -> Self {
            self.0.extend_from_slice(bytes);
            self
        }
        /// A standing position + movement + character-state block with no buffs.
        fn pos_move_state(self, region: u16, x: f32, y: f32, z: f32, heading: u16) -> Self {
            self.u16(region)
                .f32(x)
                .f32(y)
                .f32(z)
                .u16(heading)
                // movement: standing (has_dest=0, move_type, unk, heading)
                .u8(0)
                .u8(1)
                .u8(0)
                .u16(heading)
                // character-state: life/unk/motion/body + 3 speeds + 0 buffs
                .u8(1)
                .u8(0)
                .u8(0)
                .u8(0)
                .f32(16.0)
                .f32(50.0)
                .f32(0.0)
                .u8(0)
        }
    }

    fn resolver(entries: &[(u32, RefType)]) -> MockResolver {
        MockResolver {
            types: entries.iter().copied().collect(),
        }
    }

    #[test]
    fn parses_player_with_mixed_equipment() {
        const PLAYER_REF: u32 = 1907;
        const SWORD: u32 = 100; // equipment -> carries opt byte
        const NECKLACE: u32 = 200; // non-equipment -> no opt byte
        let res = resolver(&[
            (PLAYER_REF, RefType::Player),
            (
                SWORD,
                RefType::Item {
                    equipment: true,
                    gold: false,
                    named: false,
                },
            ),
            (
                NECKLACE,
                RefType::Item {
                    equipment: false,
                    gold: false,
                    named: false,
                },
            ),
        ]);

        let body = Body::default()
            .u32(PLAYER_REF)
            // scale, hwan, pvp_cape, autoxp, base_inv_size
            .u8(0)
            .u8(0)
            .u8(0)
            .u8(1)
            .u8(45)
            // inventory: 2 items
            .u8(2)
            .u32(SWORD)
            .u8(3) // equipment: opt byte present
            .u32(NECKLACE) // non-equipment: no opt byte
            // avatar slots + 0 avatar items + has_mask
            .u8(5)
            .u8(0)
            .u8(0)
            // unique id + position/movement/state
            .u32(352808)
            .pos_move_state(0x60A8, 1058.0, -7.68, 1426.0, 12268)
            // name
            .string("Remote")
            // job_type, job_level, pk_state, transport, in_combat, scroll, interact, unk
            .raw(&[0, 1, 0xFF, 0, 0, 0, 0, 0])
            // guild: empty name, guild id, empty nick, 3×u32 + 2×u8
            .string("")
            .u32(0)
            .string("")
            .raw(&[0u8; 14])
            // equipment_cooldown, pk_flag
            .u8(0)
            .u8(0xFF)
            .0;

        let parsed = parse_group_spawn(&body, true, 1, &res);
        assert_eq!(parsed.despawns.len(), 0);
        assert_eq!(parsed.spawns.len(), 1, "player should parse");
        let e = &parsed.spawns[0];
        assert_eq!(e.ref_id, PLAYER_REF);
        assert_eq!(e.unique_id, 352808);
        assert_eq!(e.name.as_deref(), Some("Remote"));
        // guildless: the block is present but its name is empty
        assert_eq!(e.guild, None);
        assert_eq!(e.position.region, 0x60A8);
        assert_eq!(e.position.x, 1058.0);
        assert_eq!(
            e.kind,
            SpawnKind::Player {
                // the sword's +3 opt byte is captured; non-equipment has none
                equipment: vec![(SWORD, 3), (NECKLACE, 0)],
                riding_uid: None,
            }
        );
        let state = e.state.as_ref().expect("characters carry a state block");
        assert_eq!(state.walk_speed, 16.0);
        assert_eq!(state.run_speed, 50.0);
    }

    #[test]
    fn parses_npc_and_monster() {
        const NPC_REF: u32 = 1900;
        const MOB_REF: u32 = 2000;
        let res = resolver(&[
            (NPC_REF, RefType::Npc),
            (MOB_REF, RefType::Monster { extra: false }),
        ]);

        // NPC then monster in one batch, so the NPC's length must be exact for
        // the monster to parse.
        let body = Body::default()
            .u32(NPC_REF)
            .u32(11)
            .pos_move_state(0x60A8, 1.0, 2.0, 3.0, 0)
            .u8(0) // talk flag: none
            .u32(MOB_REF)
            .u32(22)
            .pos_move_state(0x60A8, 4.0, 5.0, 6.0, 0)
            .u8(0) // talk flag: none
            .u8(1) // rarity (monster only)
            .0;

        let parsed = parse_group_spawn(&body, true, 2, &res);
        assert_eq!(parsed.spawns.len(), 2);
        assert_eq!(parsed.spawns[0].unique_id, 11);
        assert_eq!(parsed.spawns[0].kind, SpawnKind::Npc);
        assert_eq!(parsed.spawns[0].spawn_rarity, None);
        assert_eq!(parsed.spawns[1].unique_id, 22);
        assert_eq!(parsed.spawns[1].kind, SpawnKind::Monster { extra: None });
        assert_eq!(parsed.spawns[1].spawn_rarity, Some(1));
    }

    /// The interaction block has three parts, one per flag bit; with only the
    /// options read, an NPC carrying the other two would shift the record
    /// behind it.
    #[test]
    fn an_interaction_block_reads_all_three_parts() {
        const NPC_REF: u32 = 1900;
        let res = resolver(&[(NPC_REF, RefType::Npc)]);
        let body = Body::default()
            .u32(NPC_REF)
            .u32(11)
            .pos_move_state(0x60A8, 1.0, 2.0, 3.0, 0)
            .u8(1 | 2 | 4)
            .string("Hi") // bit 1
            .u8(2) // bit 2: two options
            .u8(1)
            .u8(4)
            .u8(2) // bit 4: type and value
            .u32(0xBEEF)
            .u32(NPC_REF)
            .u32(12)
            .pos_move_state(0x60A8, 1.0, 2.0, 3.0, 0)
            .u8(0)
            .0;
        let parsed = parse_group_spawn(&body, true, 2, &res);
        assert!(parsed.notes.is_empty(), "{:?}", parsed.notes);
        let npc = &parsed.spawns[0];
        let interaction = Interaction {
            flags: 7,
            text: Some("Hi".to_owned()),
            typed_value: Some((2, 0xBEEF)),
        };
        assert_eq!(npc.interaction.as_ref(), Some(&interaction));
        assert_eq!(npc.talk_options, [1, 4]);
        assert_eq!(parsed.spawns[1].unique_id, 12);
    }

    /// A thief or hunter monster carries one byte more after its rarity; read
    /// without it, the next record would start one byte early.
    #[test]
    fn a_thief_monster_reads_its_extra_byte() {
        const THIEF_REF: u32 = 2001;
        const NPC_REF: u32 = 1900;
        let res = resolver(&[
            (THIEF_REF, RefType::Monster { extra: true }),
            (NPC_REF, RefType::Npc),
        ]);
        let body = Body::default()
            .u32(THIEF_REF)
            .u32(22)
            .pos_move_state(0x60A8, 4.0, 5.0, 6.0, 0)
            .u8(0) // talk flag: none
            .u8(1) // rarity
            .u8(7) // the thief's extra byte
            .u32(NPC_REF)
            .u32(11)
            .pos_move_state(0x60A8, 1.0, 2.0, 3.0, 0)
            .u8(0)
            .0;
        let parsed = parse_group_spawn(&body, true, 2, &res);
        assert!(parsed.notes.is_empty(), "{:?}", parsed.notes);
        let thief = SpawnKind::Monster { extra: Some(7) };
        assert_eq!(
            (&parsed.spawns[0].kind, parsed.spawns[1].unique_id),
            (&thief, 11)
        );
    }

    /// A resolver without characterdata answers [`RefType::Character`] for
    /// both: the rarity byte is settled by where the batch ends. NPC, monster,
    /// NPC — each reading other than the right one shifts the records behind
    /// it off the payload's end.
    #[test]
    fn undecided_characters_are_settled_by_the_batch_end() {
        const NPC_REF: u32 = 2003;
        const MOB_REF: u32 = 1933;
        let res = resolver(&[(NPC_REF, RefType::Character), (MOB_REF, RefType::Character)]);
        let body = Body::default()
            .u32(NPC_REF)
            .u32(11)
            .pos_move_state(0x61A8, 1.0, 0.0, 2.0, 0)
            .u8(2) // talk: one option
            .u8(1)
            .u8(2)
            .u32(MOB_REF)
            .u32(22)
            .pos_move_state(0x61A8, 3.0, 0.0, 4.0, 0)
            .u8(0) // no options
            .u8(1) // rarity
            .u32(NPC_REF)
            .u32(33)
            .pos_move_state(0x61A8, 5.0, 0.0, 6.0, 0)
            .u8(0) // no options
            .0;

        let parsed = parse_group_spawn(&body, true, 3, &res);
        let read: Vec<_> = parsed
            .spawns
            .iter()
            .map(|e| (e.unique_id, e.kind.clone(), e.spawn_rarity))
            .collect();
        assert_eq!(
            read,
            vec![
                (11, SpawnKind::Npc, None),
                (22, SpawnKind::Monster { extra: None }, Some(1)),
                (33, SpawnKind::Npc, None),
            ]
        );
    }

    #[test]
    fn parses_talkable_npc_with_options() {
        // A talkable NPC carries interaction options: tag=2, then a u8 count and
        // that many 1-byte option ids (values from a live capture). A second NPC
        // follows so the option list's length must be consumed exactly.
        const NPC_REF: u32 = 1900;
        let res = resolver(&[(NPC_REF, RefType::Npc)]);
        let body = Body::default()
            .u32(NPC_REF)
            .u32(99)
            .pos_move_state(0x60A8, 1.0, 2.0, 3.0, 0)
            .u8(2) // interact tag: talk
            .u8(4) // option count
            .raw(&[0x01, 0x02, 0x04, 0x20]) // 4 option ids
            .u32(NPC_REF)
            .u32(100)
            .pos_move_state(0x60A8, 4.0, 5.0, 6.0, 0)
            .u8(0) // second NPC: no options
            .0;

        let parsed = parse_group_spawn(&body, true, 2, &res);
        assert_eq!(
            parsed.spawns.len(),
            2,
            "the option list must be consumed exactly"
        );
        assert_eq!(parsed.spawns[0].unique_id, 99);
        assert_eq!(parsed.spawns[0].talk_options, vec![0x01, 0x02, 0x04, 0x20]);
        assert_eq!(parsed.spawns[1].unique_id, 100);
        assert!(parsed.spawns[1].talk_options.is_empty());
    }

    /// A live vSRO group frame — `0x3017` with count 2, then this 85-byte
    /// `0x3019` (packet_dump/0x3019.log, 2026-08-17T11:13:14.797Z): a gold pile
    /// of 30 bytes, then a monster of 55. Read with a drop-source tail, the
    /// pile took the monster's first byte (`8d`) for a source and four more for
    /// a dropper, and the monster was lost.
    #[test]
    fn a_group_drop_ends_at_its_rarity_byte() {
        const LINE: &str = "010000005f000000d0610100a8600f870a435cb834c2a32f33446c5500008d070000\
                            caa00100a860acf30743e11926c2d5812144475d0100a8605000d7ffc6020100\
                            0200000000410000b0410000c8420002010500";
        let body: Vec<u8> = (0..LINE.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&LINE[i..i + 2], 16).unwrap())
            .collect();
        let res = resolver(&[
            (
                1,
                RefType::Item {
                    equipment: false,
                    gold: true,
                    named: false,
                },
            ),
            (1933, RefType::Monster { extra: false }),
        ]);
        let parsed = parse_group_spawn(&body, true, 2, &res);
        let read: Vec<_> = parsed
            .spawns
            .iter()
            .map(|e| (e.unique_id, e.kind.clone()))
            .collect();
        assert_eq!(
            read,
            vec![
                (
                    0x161D0,
                    SpawnKind::Item {
                        amount: Some(95),
                        name: None
                    }
                ),
                (0x1A0CA, SpawnKind::Monster { extra: None })
            ]
        );
    }

    #[test]
    fn parses_gold_drop() {
        const GOLD_REF: u32 = 3000;
        let res = resolver(&[(
            GOLD_REF,
            RefType::Item {
                equipment: false,
                gold: true,
                named: false,
            },
        )]);
        // mirrors a live 0x3015 gold line, drop-source tail included; the
        // record ends at the rarity byte and the tail is left unread
        let body = Body::default()
            .u32(GOLD_REF)
            .u32(5000) // amount
            .u32(77) // unique id
            .u16(0x60A8)
            .f32(1.0)
            .f32(2.0)
            .f32(3.0)
            .u16(0) // position
            .u8(1) // owner flag: present
            .u32(4) // owner jid
            .u8(0) // rarity
            .u8(5) // drop source
            .u32(0x147A8) // dropper uid
            .0;
        let parsed = parse_group_spawn(&body, true, 1, &res);
        assert_eq!(parsed.spawns.len(), 1);
        assert_eq!(parsed.spawns[0].unique_id, 77);
        assert_eq!(
            parsed.spawns[0].kind,
            SpawnKind::Item {
                amount: Some(5000),
                name: None
            }
        );
        // the drop's rarity byte is kept, not skipped: it is the only source
        // for the drop label's colour (a ground drop carries no item body)
        assert_eq!(parsed.spawns[0].spawn_rarity, Some(0));
    }

    /// An item of TypeID 3/3/8 or 3/3/9 names someone between its ref and its
    /// unique id; read as gold or plain, the name's bytes would be the uid.
    /// A synthetic frame: the captured ones carry a real player's name.
    #[test]
    fn a_named_item_reads_its_name_before_the_unique_id() {
        const BOX_REF: u32 = 3862;
        let res = resolver(&[(
            BOX_REF,
            RefType::Item {
                equipment: false,
                gold: false,
                named: true,
            },
        )]);
        let body = Body::default()
            .u32(BOX_REF)
            .string("Someone")
            .u32(88) // unique id
            .u16(0x60A8)
            .f32(1.0)
            .f32(2.0)
            .f32(3.0)
            .u16(0) // position
            .u8(0) // no owner
            .u8(0) // rarity
            .0;
        let parsed = parse_group_spawn(&body, true, 1, &res);
        assert!(parsed.notes.is_empty(), "{:?}", parsed.notes);
        assert_eq!(parsed.spawns[0].unique_id, 88);
        let named = SpawnKind::Item {
            amount: None,
            name: Some("Someone".to_owned()),
        };
        assert_eq!(parsed.spawns[0].kind, named);
    }

    #[test]
    fn parses_despawn_list() {
        let res = resolver(&[]);
        let body = Body::default().u32(10).u32(20).u32(30).0;
        let parsed = parse_group_spawn(&body, false, 3, &res);
        assert_eq!(parsed.despawns, vec![10, 20, 30]);
        assert_eq!(parsed.spawns.len(), 0);
        assert!(parsed.notes.is_empty());
        // A list cut short keeps what it had and says where it ended.
        let parsed = parse_group_spawn(&body[..10], false, 3, &res);
        assert_eq!(parsed.despawns, vec![10, 20]);
        assert_eq!(
            parsed.notes,
            [SpawnNote::DespawnTruncated { at: 2, count: 3 }]
        );
    }

    #[test]
    fn unknown_ref_stops_batch_safely() {
        // First record is a good NPC; second ref id is unknown -> stop after one.
        const NPC_REF: u32 = 1900;
        let res = resolver(&[(NPC_REF, RefType::Npc)]);
        let body = Body::default()
            .u32(NPC_REF)
            .u32(11)
            .pos_move_state(0x60A8, 1.0, 2.0, 3.0, 0)
            .u8(0)
            .u32(999999) // unknown ref
            .raw(&[0u8; 40])
            .0;
        let parsed = parse_group_spawn(&body, true, 2, &res);
        assert_eq!(
            parsed.spawns.len(),
            1,
            "keeps the record parsed before the bad one"
        );
        assert_eq!(parsed.spawns[0].unique_id, 11);
        // The unknown record is the last one, so its width is simply the rest
        // of the payload: it is skipped rather than ending the batch.
        let skipped = SpawnNote::Skipped {
            index: 1,
            count: 2,
            ref_id: 999999,
            width: 44,
        };
        assert_eq!(parsed.notes, [skipped]);
    }

    /// Verbatim `packet_dump/0x3019.log` payload (2026-08-11 09:43:45.339Z,
    /// 98 bytes) whose `GroupEntitySpawnBegin` of the same instant is
    /// `01 02 00` — spawn, 2 records. Record 0 is ref 9252 (in `npcpos.txt`,
    /// in NO `characterdata_*.txt` of either of the user's Media.pk2 — the
    /// ids jump 8984 -> 9264), record 1 is ref 3861 NPC_CH_EVENT_KISAENG1.
    /// Each record is 49 bytes.
    const LIVE_UNKNOWN_NPC_BATCH: &[u8] = &[
        0x24, 0x24, 0x00, 0x00, 0x6b, 0x01, 0x00, 0x00, 0xa8, 0x61, 0xec, 0x51, 0x74, 0x44, 0x7f,
        0x6f, 0x02, 0xc2, 0x52, 0xe8, 0x35, 0x44, 0xb4, 0xc0, 0x00, 0x01, 0x00, 0xb4, 0xc0, 0x01,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc8, 0x42,
        0x00, 0x02, 0x01, 0x12, 0x15, 0x0f, 0x00, 0x00, 0x6d, 0x01, 0x00, 0x00, 0xa8, 0x61, 0xc3,
        0xb5, 0x89, 0x44, 0x7f, 0x6f, 0x02, 0xc2, 0x33, 0x93, 0x55, 0x44, 0xff, 0x3f, 0x00, 0x01,
        0x00, 0xff, 0x3f, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0xc8, 0x42, 0x00, 0x02, 0x01, 0x02,
    ];

    #[test]
    fn live_unknown_ref_keeps_the_rest_of_the_batch() {
        // #426: the unclassifiable record used to end the batch, so the NPC
        // *behind* it never spawned. Its width (49 bytes) is derivable from
        // the payload — it is the only one that lets the second record parse
        // and land exactly on the last byte — so the batch survives.
        let res = resolver(&[(3861, RefType::Npc)]);
        let parsed = parse_group_spawn(LIVE_UNKNOWN_NPC_BATCH, true, 2, &res);

        assert_eq!(parsed.unresolved, vec![9252]);
        assert!(matches!(
            parsed.notes[..],
            [SpawnNote::Skipped {
                index: 0,
                count: 2,
                ref_id: 9252,
                ..
            }]
        ));
        assert_eq!(parsed.spawns.len(), 1, "the record behind the unknown one");
        let npc = &parsed.spawns[0];
        assert_eq!(npc.ref_id, 3861);
        assert_eq!(npc.unique_id, 0x16d);
        assert_eq!(npc.position.region, 25000);
        assert_eq!(npc.kind, SpawnKind::Npc);
    }

    #[test]
    fn unknown_ref_still_stops_the_batch_when_no_width_fits() {
        // The skip is only allowed when a width makes the remaining records
        // parse to the payload's last byte. Cut the capture's last byte off
        // and none does, so the batch must abort as before instead of
        // inventing a boundary.
        let res = resolver(&[(3861, RefType::Npc)]);
        let truncated = &LIVE_UNKNOWN_NPC_BATCH[..LIVE_UNKNOWN_NPC_BATCH.len() - 1];
        let parsed = parse_group_spawn(truncated, true, 2, &res);

        assert!(parsed.unresolved.is_empty());
        assert!(parsed.spawns.is_empty());
    }

    #[test]
    fn truncated_body_fails_safe() {
        const NPC_REF: u32 = 1900;
        let res = resolver(&[(NPC_REF, RefType::Npc)]);
        // ref id present but the rest of the record is cut off.
        let body = Body::default().u32(NPC_REF).u16(5).0;
        let parsed = parse_group_spawn(&body, true, 1, &res);
        assert_eq!(parsed.spawns.len(), 0);
        assert_eq!(parsed.despawns.len(), 0);
        assert!(matches!(
            parsed.notes[..],
            [SpawnNote::Stopped {
                index: 0,
                ref_id: Some(NPC_REF),
                start: 0,
                ..
            }]
        ));
    }

    #[test]
    fn parses_captured_gate_building_record() {
        // verbatim packet_dump/0x3019.log payload (Jangan dimensional gate,
        // ref 2094 = STORE_CH_GATE, captured 40x as its own count-1 batch)
        const GATE_REF: u32 = 2094;
        let body: &[u8] = &[
            0x2e, 0x08, 0x00, 0x00, // ref 2094
            0x0c, 0x00, 0x00, 0x00, // uid 12
            0xa8, 0x61, // region 25000
            0x00, 0xc0, 0x9c, 0x44, // x 1254.0
            0x00, 0x00, 0xc0, 0xc0, // y -6.0
            0x00, 0xc0, 0xab, 0x44, // z 1374.0
            0x00, 0x00, // heading 0
            0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, // 12-byte unknown tail
        ];
        let res = resolver(&[(GATE_REF, RefType::Structure)]);
        let parsed = parse_group_spawn(body, true, 1, &res);
        assert_eq!(parsed.spawns.len(), 1);
        let gate = &parsed.spawns[0];
        assert_eq!(gate.ref_id, GATE_REF);
        assert_eq!(gate.unique_id, 12);
        assert_eq!(gate.position.region, 25000);
        assert_eq!(
            (gate.position.x, gate.position.y, gate.position.z),
            (1254.0, -6.0, 1374.0)
        );
        assert_eq!(gate.kind, SpawnKind::Structure);
    }

    /// The shared NPC head of a COS record (uid + position/movement/state +
    /// empty talk block), ready for the kind-specific owner tail.
    fn cos_head(ref_id: u32, uid: u32) -> Body {
        Body::default()
            .u32(ref_id)
            .u32(uid)
            .pos_move_state(0x60A8, 1.0, 2.0, 3.0, 0)
            .u8(0) // talk flag: none
    }

    /// A trailing NPC record used to prove the COS record before it ended
    /// exactly on its boundary.
    fn follower_npc(body: Body, ref_id: u32, uid: u32) -> Body {
        body.u32(ref_id)
            .u32(uid)
            .pos_move_state(0x60A8, 4.0, 5.0, 6.0, 0)
            .u8(0)
    }

    /// Ride-only horses (tid4 1) carry no owner tail at all — a follower
    /// record right after the talk block must still parse.
    #[test]
    fn parses_horse_cos_without_owner_tail() {
        const HORSE_REF: u32 = 2137; // COS_C_HORSE1
        const NPC_REF: u32 = 1900;
        let res = resolver(&[
            (
                HORSE_REF,
                RefType::Cos {
                    kind: CosKind::Vehicle,
                },
            ),
            (NPC_REF, RefType::Npc),
        ]);
        let body = follower_npc(cos_head(HORSE_REF, 41), NPC_REF, 42).0;
        let parsed = parse_group_spawn(&body, true, 2, &res);
        assert_eq!(parsed.spawns.len(), 2, "horse record must end after talk");
        assert_eq!(
            parsed.spawns[0].kind,
            SpawnKind::Cos {
                kind: CosKind::Vehicle,
                pet_name: None,
                owner_name: None,
                owner_uid: None,
            }
        );
        assert_eq!(parsed.spawns[1].kind, SpawnKind::Npc);
    }

    /// Transports (tid4 2): OwnerName, Job, PVPState, OwnerUniqueID.
    #[test]
    fn parses_transport_cos_owner_tail() {
        const TRANSPORT_REF: u32 = 2184; // COS_T_HORSE1
        const NPC_REF: u32 = 1900;
        let res = resolver(&[
            (
                TRANSPORT_REF,
                RefType::Cos {
                    kind: CosKind::Transport,
                },
            ),
            (NPC_REF, RefType::Npc),
        ]);
        let body = follower_npc(
            cos_head(TRANSPORT_REF, 51)
                .string("Owner") // owner name
                .u8(0) // job type
                .u8(0xFF) // pvp state
                .u32(0x0102_0304), // owner uid
            NPC_REF,
            52,
        )
        .0;
        let parsed = parse_group_spawn(&body, true, 2, &res);
        assert_eq!(parsed.spawns.len(), 2);
        assert_eq!(
            parsed.spawns[0].kind,
            SpawnKind::Cos {
                kind: CosKind::Transport,
                pet_name: None,
                owner_name: Some("Owner".into()),
                owner_uid: Some(0x0102_0304),
            }
        );
    }

    /// Attack pets (tid4 3) prefix their given name; pick pets (tid4 4) skip
    /// the PVP-state byte; guild guards (tid4 5) insert an OwnerObjectID.
    #[test]
    fn parses_pet_and_guard_cos_tails() {
        const ATTACK_REF: u32 = 6106; // COS_P_WOLF
        const PICK_REF: u32 = 10365; // COS_P_RABBIT
        const GUARD_REF: u32 = 2100;
        let res = resolver(&[
            (
                ATTACK_REF,
                RefType::Cos {
                    kind: CosKind::GrowthPet,
                },
            ),
            (
                PICK_REF,
                RefType::Cos {
                    kind: CosKind::GrabPet,
                },
            ),
            (
                GUARD_REF,
                RefType::Cos {
                    kind: CosKind::Fellow,
                },
            ),
        ]);
        let body = cos_head(ATTACK_REF, 61)
            .string("Rex") // pet name
            .string("Owner")
            .u8(0) // job
            .u8(0xFF) // pvp
            .u32(1001) // owner uid
            // pick pet: name + owner, job, NO pvp byte, owner uid
            .raw(&cos_head(PICK_REF, 62).0)
            .string("Hoppel")
            .string("Owner")
            .u8(0)
            .u32(1001)
            // guild guard: no pet name, owner, job, pvp, owner OBJECT id, owner uid
            .raw(&cos_head(GUARD_REF, 63).0)
            .string("GuildMate")
            .u8(0)
            .u8(0xFF)
            .u32(777) // owner object id
            .u32(1002)
            .0;
        let parsed = parse_group_spawn(&body, true, 3, &res);
        assert_eq!(parsed.spawns.len(), 3, "all three tails must be exact");
        assert_eq!(
            parsed.spawns[0].kind,
            SpawnKind::Cos {
                kind: CosKind::GrowthPet,
                pet_name: Some("Rex".into()),
                owner_name: Some("Owner".into()),
                owner_uid: Some(1001),
            }
        );
        assert_eq!(
            parsed.spawns[1].kind,
            SpawnKind::Cos {
                kind: CosKind::GrabPet,
                pet_name: Some("Hoppel".into()),
                owner_name: Some("Owner".into()),
                owner_uid: Some(1001),
            }
        );
        assert_eq!(
            parsed.spawns[2].kind,
            SpawnKind::Cos {
                kind: CosKind::Fellow,
                pet_name: None,
                owner_name: Some("GuildMate".into()),
                owner_uid: Some(1002),
            }
        );
    }

    /// A truncated COS owner tail fails safe instead of panicking.
    #[test]
    fn truncated_cos_tail_fails_safe() {
        const TRANSPORT_REF: u32 = 2184;
        let res = resolver(&[(
            TRANSPORT_REF,
            RefType::Cos {
                kind: CosKind::Transport,
            },
        )]);
        let body = cos_head(TRANSPORT_REF, 51).string("Owner").u8(0).0;
        let parsed = parse_group_spawn(&body, true, 1, &res);
        assert_eq!(parsed.spawns.len(), 0);
    }

    /// Build a one-player spawn body whose guild block carries `guild_name` and
    /// `nick`. Guildless players send the same block with an empty name — the
    /// block is never omitted (go-sro `WriteGuild`, corroborated by the vSRO
    /// client parser; see `Reader::guild`). `riding_uid` inserts the mounted
    /// player's conditional COS uid between the state flags.
    fn player_body(
        player_ref: u32,
        guild_name: &str,
        nick: &str,
        riding_uid: Option<u32>,
    ) -> Vec<u8> {
        let mut body = Body::default()
            .u32(player_ref)
            // scale, hwan, pvp_cape, autoxp, base_inv_size
            .raw(&[0u8; 5])
            // 0 inventory items, 0 avatar slots, 0 avatar items, has_mask
            .u8(0)
            .u8(0)
            .u8(0)
            .u8(0)
            .u32(352808)
            .pos_move_state(0x60A8, 1058.0, -7.68, 1426.0, 12268)
            .string("Remote")
            // job_type, job_level, pk_state
            .raw(&[0, 1, 0xFF])
            // riding flag, in_combat, [riding uid]
            .u8(riding_uid.is_some() as u8)
            .u8(0);
        if let Some(uid) = riding_uid {
            body = body.u32(uid);
        }
        body
            // scroll, interact, unk
            .raw(&[0, 0, 0])
            // guild block
            .string(guild_name)
            .u32(7)
            .string(nick)
            .raw(&[0u8; 14])
            // equipment_cooldown, pk_flag
            .u8(0)
            .u8(0xFF)
            .0
    }

    fn player_body_with_guild(player_ref: u32, guild_name: &str, nick: &str) -> Vec<u8> {
        player_body(player_ref, guild_name, nick, None)
    }

    /// A mounted player inserts a `u32` mount uid; the old flat skip desynced
    /// the batch, so a follower record proves the boundary ([S] until F7).
    #[test]
    fn parses_mounted_player_and_keeps_the_boundary() {
        const PLAYER_REF: u32 = 1907;
        const NPC_REF: u32 = 1900;
        let res = resolver(&[(PLAYER_REF, RefType::Player), (NPC_REF, RefType::Npc)]);
        let mut body = player_body(PLAYER_REF, "", "", Some(4711));
        body.extend_from_slice(&follower_npc(Body::default(), NPC_REF, 99).0);

        let parsed = parse_group_spawn(&body, true, 2, &res);
        assert_eq!(parsed.spawns.len(), 2, "rider + follower must both parse");
        assert_eq!(
            parsed.spawns[0].kind,
            SpawnKind::Player {
                equipment: vec![],
                riding_uid: Some(4711),
            }
        );
        assert_eq!(parsed.spawns[1].unique_id, 99);
    }

    /// The guild block used to be skipped wholesale (#30). A guilded player's
    /// name and granted nick now reach the spawn record.
    #[test]
    fn parses_the_guild_block_of_a_guilded_player() {
        const PLAYER_REF: u32 = 1907;
        let res = resolver(&[(PLAYER_REF, RefType::Player)]);
        let body = player_body_with_guild(PLAYER_REF, "Ironclad", "Quartermaster");

        let parsed = parse_group_spawn(&body, true, 1, &res);
        assert_eq!(parsed.spawns.len(), 1, "guilded player should parse");
        let guild = parsed.spawns[0]
            .guild
            .as_ref()
            .expect("guilded player carries a guild tag");
        assert_eq!(guild.name, "Ironclad");
        assert_eq!(guild.granted_nick, "Quartermaster");
        // the record must still end exactly where it did before
        assert_eq!(parsed.spawns[0].name.as_deref(), Some("Remote"));
        assert_eq!(parsed.spawns[0].unique_id, 352808);
    }

    /// An empty guild name is guildless, not a guild called "" — otherwise
    /// every player would draw a blank second nameplate line.
    #[test]
    fn empty_guild_name_parses_as_guildless() {
        const PLAYER_REF: u32 = 1907;
        let res = resolver(&[(PLAYER_REF, RefType::Player)]);
        let body = player_body_with_guild(PLAYER_REF, "", "");

        let parsed = parse_group_spawn(&body, true, 1, &res);
        assert_eq!(parsed.spawns.len(), 1);
        assert_eq!(parsed.spawns[0].guild, None);
    }

    /// The guild block is variable-length, so reading it must leave the record
    /// boundary intact: a second record after a guilded one still parses.
    #[test]
    fn guild_block_does_not_desync_the_next_record() {
        const PLAYER_REF: u32 = 1907;
        let res = resolver(&[(PLAYER_REF, RefType::Player)]);
        let mut body = player_body_with_guild(PLAYER_REF, "Ironclad", "Quartermaster");
        body.extend_from_slice(&player_body_with_guild(PLAYER_REF, "", ""));

        let parsed = parse_group_spawn(&body, true, 2, &res);
        assert_eq!(parsed.spawns.len(), 2, "both records should parse");
        assert_eq!(
            parsed.spawns[0].guild.as_ref().map(|g| g.name.as_str()),
            Some("Ironclad")
        );
        assert_eq!(parsed.spawns[1].guild, None);
    }
}
