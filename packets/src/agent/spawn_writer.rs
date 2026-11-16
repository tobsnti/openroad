//! The spawn records written back, the mirror of the reader in
//! `entity_spawn.rs`: a record is written from what the reader keeps, and the
//! same tables decide its layout. Wherever the record's shape depends on data
//! (an item's head, a monster's extra byte, the opt byte of an item in a
//! player's lists, a mask that is a player model), the writer asks the
//! resolver, as the reader does; what a field holds comes from the entity.
//! An entity written and read again is the same entity, and a record read and
//! written again is the same bytes.

use bytes::{BufMut, BytesMut};
use sro_macro::Serialize;

use crate::agent::entity_spawn::{
    Interaction, Mask, RefResolver, RefType, SingleTail, SpawnKind, SpawnedEntity,
};
use crate::agent::pet::CosKind;

/// Little-endian writes into a growing body. The blocks the spawn records
/// share with CHARACTER_DATA (position, movement, state) are written by their
/// own serializers, the same ones the 0x3013 writer uses.
struct Writer(BytesMut);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.put_u8(v);
    }

    fn u16(&mut self, v: u16) {
        self.0.put_u16_le(v);
    }

    fn u32(&mut self, v: u32) {
        self.0.put_u32_le(v);
    }

    fn bytes(&mut self, v: &[u8]) {
        self.0.put_slice(v);
    }

    /// A u16-length-prefixed string.
    fn string(&mut self, s: &str) {
        self.u16(s.len() as u16);
        self.bytes(s.as_bytes());
    }

    /// The unique id, position, movement and state every character starts
    /// with. `None` when the entity has no movement or state block, or its
    /// movement block lacks a field its flags call for.
    fn character_head(&mut self, e: &SpawnedEntity) -> Option<()> {
        self.u32(e.unique_id);
        e.position.serialize_to(&mut self.0);
        e.movement?.write(&mut self.0)?;
        e.state.as_ref()?.serialize_to(&mut self.0);
        Some(())
    }

    /// The interaction block. Its flag bits announce its parts, so a part is
    /// written only when its bit is set: the options of `talk_options` only
    /// for bit 2. `None` when a part the flags announce is missing.
    fn interaction(&mut self, i: &Interaction, talk_options: &[u8]) -> Option<()> {
        self.u8(i.flags);
        if i.flags & 1 != 0 {
            self.string(i.text.as_deref()?);
        }
        if i.flags & 2 != 0 {
            self.u8(talk_options.len() as u8);
            self.bytes(talk_options);
        }
        if i.flags & 4 != 0 {
            let (kind, value) = i.typed_value?;
            self.u8(kind);
            self.u32(value);
        }
        Some(())
    }

    /// A player's item list: each item's opt byte only where the item is
    /// equipment, as the reader reads it.
    fn items(&mut self, items: &[(u32, u8)], resolver: &impl RefResolver) {
        self.u8(items.len() as u8);
        for &(ref_id, opt_level) in items {
            self.u32(ref_id);
            if resolver.item_is_equipment(ref_id) {
                self.u8(opt_level);
            }
        }
    }

    /// `None` when a mask that is a player model lacks its model.
    fn mask(&mut self, mask: Option<&Mask>, resolver: &impl RefResolver) -> Option<()> {
        let Some(mask) = mask else {
            self.u8(0);
            return Some(());
        };
        self.u8(1);
        self.u32(mask.ref_id);
        if resolver.resolve(mask.ref_id) == RefType::Player {
            let (byte, values) = mask.model.as_ref()?;
            self.u8(*byte);
            self.u8(values.len() as u8);
            for v in values {
                self.u32(*v);
            }
        }
        Some(())
    }
}

/// One record as a group payload carries it. `None` when the tables do not
/// know the entity's ref id or give it a type its kind does not have (then
/// its layout is not known), or when the entity lacks a value its record
/// calls for: a value is written as the entity holds it, never made up.
/// Absent names stand for empty ones, as the reader keeps them.
fn record(w: &mut Writer, e: &SpawnedEntity, resolver: &impl RefResolver) -> Option<()> {
    let ref_type = resolver.resolve(e.ref_id);
    w.u32(e.ref_id);
    match (&e.kind, ref_type) {
        (SpawnKind::Npc, RefType::Npc | RefType::Character) => {
            w.character_head(e)?;
            w.interaction(e.interaction.as_ref()?, &e.talk_options)?;
        }
        (SpawnKind::Monster { extra }, RefType::Monster { .. } | RefType::Character) => {
            w.character_head(e)?;
            w.interaction(e.interaction.as_ref()?, &e.talk_options)?;
            w.u8(e.spawn_rarity?);
            if ref_type == (RefType::Monster { extra: true }) {
                w.u8((*extra)?);
            }
        }
        (
            SpawnKind::Cos {
                kind,
                pet_name,
                owner_name,
                owner_bytes,
                owner_uid,
            },
            RefType::Cos { .. },
        ) => {
            w.character_head(e)?;
            w.interaction(e.interaction.as_ref()?, &e.talk_options)?;
            if *kind != CosKind::Vehicle {
                if matches!(kind, CosKind::GrowthPet | CosKind::GrabPet) {
                    w.string(pet_name.as_deref().unwrap_or_default());
                }
                w.string(owner_name.as_deref().unwrap_or_default());
                let (first, second, fellow) = (*owner_bytes)?;
                w.u8(first);
                if *kind != CosKind::GrabPet {
                    w.u8(second?);
                }
                if *kind == CosKind::Fellow {
                    w.u32(fellow?);
                }
                w.u32((*owner_uid)?);
            }
        }
        (
            SpawnKind::Player {
                equipment,
                avatar,
                riding,
                riding_uid,
                mask,
                interact_mode,
                stall,
                guild_after_name,
                guild_tail,
                bytes,
            },
            RefType::Player,
        ) => {
            w.bytes(&bytes.head);
            w.items(equipment, resolver);
            w.u8(bytes.between_lists);
            w.items(avatar, resolver);
            w.mask(mask.as_ref(), resolver)?;
            w.character_head(e)?;
            w.string(e.name.as_deref()?);
            w.bytes(&bytes.after_name);
            // The riding byte announces the mount's id, as the interaction's
            // flags announce its parts.
            w.u8(*riding);
            w.u8(bytes.after_riding);
            if *riding == 1 {
                w.u32((*riding_uid)?);
            }
            w.u8(bytes.around_mode[0]);
            w.u8(*interact_mode);
            w.u8(bytes.around_mode[1]);
            let guild = e.guild.as_ref();
            w.string(guild.map_or("", |g| g.name.as_str()));
            w.u32(*guild_after_name);
            w.string(guild.map_or("", |g| g.granted_nick.as_str()));
            w.bytes(guild_tail);
            if *interact_mode == 4 {
                let (title, value) = stall.as_ref()?;
                w.string(title);
                w.u32(*value);
            }
            w.bytes(&bytes.tail);
        }
        (
            SpawnKind::Item {
                amount,
                name,
                opt_level,
                owner,
            },
            RefType::Item {
                equipment,
                gold,
                named,
            },
        ) => {
            if equipment {
                w.u8((*opt_level)?);
            } else if named {
                w.string(name.as_deref()?);
            } else if gold {
                w.u32((*amount)?);
            }
            w.u32(e.unique_id);
            e.position.serialize_to(&mut w.0);
            match owner {
                Some(uid) => {
                    w.u8(1);
                    w.u32(*uid);
                }
                None => w.u8(0),
            }
            w.u8(e.spawn_rarity?);
        }
        (SpawnKind::Structure { tail }, RefType::Structure) => {
            w.u32(e.unique_id);
            e.position.serialize_to(&mut w.0);
            w.bytes(tail);
        }
        _ => return None,
    }
    Some(())
}

/// A group-spawn payload (0x3019) of `entities`, in order: `None` when one
/// of them has a layout the tables do not give (see [`record`]).
pub fn write_group_spawn(
    entities: &[SpawnedEntity],
    resolver: &impl RefResolver,
) -> Option<Vec<u8>> {
    let mut w = Writer(BytesMut::new());
    for e in entities {
        record(&mut w, e, resolver)?;
    }
    Some(w.0.to_vec())
}

/// A group-despawn payload (0x3019 under a despawn begin): the unique ids.
pub fn write_group_despawn(unique_ids: &[u32]) -> Vec<u8> {
    unique_ids.iter().flat_map(|id| id.to_le_bytes()).collect()
}

/// A single-spawn payload (0x3015): the record and its tail. `None` when the
/// record cannot be written or the tail is not the one its kind carries: a
/// byte behind a character, the drop source behind an item (and the dropper
/// for a source of 5 or 6, as the reader reads it), the rest behind a gate
/// building.
pub fn write_single_spawn(
    entity: &SpawnedEntity,
    tail: &SingleTail,
    resolver: &impl RefResolver,
) -> Option<Vec<u8>> {
    let mut w = Writer(BytesMut::new());
    record(&mut w, entity, resolver)?;
    match (tail, resolver.resolve(entity.ref_id)) {
        (SingleTail::Item { source, dropper }, RefType::Item { .. }) => {
            w.u8(*source);
            if matches!(source, 5 | 6) {
                w.u32((*dropper)?);
            }
        }
        (SingleTail::Rest(bytes), RefType::Structure) => w.bytes(bytes),
        (SingleTail::Figure(b), ref_type)
            if !matches!(ref_type, RefType::Item { .. } | RefType::Structure) =>
        {
            w.u8(*b)
        }
        _ => return None,
    }
    Some(w.0.to_vec())
}
