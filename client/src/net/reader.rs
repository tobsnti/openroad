//! Bounds-checked little-endian reader for the client-side decoding of the
//! group-spawn payload (0x3019), whose records need the itemdata/characterdata
//! tables to select their layout (see `entity_spawn.rs`). The wire-block
//! shapes themselves (position, character state) are defined in
//! `packets::agent::character_data`; this reader just decodes them
//! `Option`-fashion so a malformed or truncated record fails safe (and keeps a
//! byte offset for diagnostics) instead of erroring through `std::io`.

use bevy::prelude::Component;
use packets::agent::character_data::{ActiveBuff, EntityState, SpawnPosition};

/// A player's guild affiliation, from the spawn record's guild block. Attached
/// to the spawned entity so the nameplate can draw the guild line under the
/// name.
///
/// Only constructed for a **non-empty** guild name: guildless players are on
/// the wire as an empty string, not as an absent block.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct GuildTag {
    pub name: String,
    /// The guild-granted nickname. Parsed and carried because the record
    /// provides it; no consumer yet.
    pub granted_nick: String,
}

/// The six numeric fields behind the guild tag (`GuildID`, crest revisions,
/// union, hostility, siege authority). Separate from [`GuildTag`] because the
/// tag is the nameplate's text component while these drive hostile-guild
/// styling and the fortress-position display — and because a job-suited
/// player's record carries the tag's name but **not** this sub-block (see
/// [`Reader::guild`]).
///
/// Field order and widths: `id:u32`, `crest_rev:u32`, `union_id:u32`,
/// `union_crest_rev:u32`, `is_friendly:u8`, `siege_authority:u8`.
/// Little-endian like the whole wire.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GuildAffiliation {
    pub id: u32,
    /// Crest revision the *sender* holds; a client with an older one refetches
    /// the emblem out of band.
    pub crest_rev: u32,
    pub union_id: u32,
    pub union_crest_rev: u32,
    /// War hostility: false = at war with the observer's guild. The byte is a
    /// bool on the wire (`!= 0`); which side sets it is unconfirmed, so no
    /// consumer may key colour off it yet.
    pub is_friendly: bool,
    /// Siege/member authority. Only `0xFF` ("none") is known; every other
    /// value is unconfirmed.
    pub siege_authority: u8,
}

/// A player record's guild block: the tag plus the sub-block that a job-suited
/// player omits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildBlock {
    pub tag: GuildTag,
    /// `None` when the record was read in job mode, i.e. the sub-block was not
    /// on the wire at all — *not* "all zeroes".
    pub affiliation: Option<GuildAffiliation>,
}

/// Bounds-checked little-endian reader.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// The current read offset into the buffer.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Move the read offset to `pos` (used to resume behind a record whose
    /// width was derived, see `entity_spawn::recover_unknown_record`).
    pub fn seek(&mut self, pos: usize) -> Option<()> {
        (pos <= self.buf.len()).then(|| self.pos = pos)
    }

    /// Bytes left in the buffer. Used where a record's *tail* is optional and
    /// only the payload length can say whether it is there.
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    pub fn skip(&mut self, n: usize) -> Option<()> {
        self.take(n).map(|_| ())
    }

    pub fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    /// The next `u32` without consuming it. Used where a record's optional
    /// tail can only be told apart from the *following* record's head
    /// (see `entity_spawn::parse_item`).
    pub fn peek_u32(&self) -> Option<u32> {
        let end = self.pos.checked_add(4)?;
        let bytes = self.buf.get(self.pos..end)?;
        Some(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    pub fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    /// A u16-length-prefixed string (lossy UTF-8; SRO names are single-byte).
    pub fn string(&mut self) -> Option<String> {
        let len = self.u16()? as usize;
        let bytes = self.take(len)?;
        Some(String::from_utf8_lossy(bytes).into_owned())
    }

    pub fn position(&mut self) -> Option<SpawnPosition> {
        Some(SpawnPosition {
            region: self.u16()?,
            x: self.f32()?,
            y: self.f32()?,
            z: self.f32()?,
            heading: self.u16()?,
        })
    }

    /// Consume the movement block: `has_dest:u8, move_type:u8`, then either a
    /// destination (region, and x/y/z when the destination region > 0) or a
    /// standing turn (`u8, heading:u16`). The destination coordinates are u16
    /// in the overworld and i32 in a dungeon, keyed by the entity's *current*
    /// position region — read immediately before this block — not the
    /// destination region.
    pub fn skip_movement(&mut self, current_region: u16) -> Option<()> {
        let has_dest = self.u8()? != 0;
        let _move_type = self.u8()?;
        if has_dest {
            let region = self.u16()?;
            if region > 0 {
                let coord_width = if current_region & 0x8000 != 0 { 4 } else { 2 };
                self.skip(3 * coord_width)?; // x, y, z
            }
        } else {
            self.skip(3)?; // unknown byte + heading:u16
        }
        Some(())
    }

    /// Read the character-state block: `life, unk, motion, body` (4×u8),
    /// walk/run/berserk speeds (3×f32), then a `u8` buff count and that many
    /// 8-byte active-buff entries.
    pub fn character_state(&mut self) -> Option<EntityState> {
        let life_state = self.u8()?;
        let unk = self.u8()?;
        let motion_state = self.u8()?;
        let body_state = self.u8()?;
        let walk_speed = self.f32()?;
        let run_speed = self.f32()?;
        let hwan_speed = self.f32()?;
        let buff_count = self.u8()?;
        let mut buffs = Vec::with_capacity(buff_count as usize);
        for _ in 0..buff_count {
            buffs.push(ActiveBuff {
                ref_skill_id: self.u32()?,
                duration: self.u32()?,
            });
        }
        Some(EntityState {
            life_state,
            unk,
            motion_state,
            body_state,
            walk_speed,
            run_speed,
            hwan_speed,
            buffs,
        })
    }

    /// Read a player record's guild block. `job_mode` is the record's own
    /// `job_type != 0` (read a few fields earlier by the caller) and decides
    /// **how much of the block is on the wire**:
    ///
    /// - always: the guild `name` string (empty for a guildless player — the
    ///   name field is never omitted, only the sub-block behind it);
    /// - only when *not* in job mode: `id:u32`, the granted-nick string,
    ///   `crest_rev:u32`, `union_id:u32`, `union_crest_rev:u32`,
    ///   `is_friendly:u8`, `siege_authority:u8`.
    ///
    /// **Why the branch, and where the old "always present" came from.** This
    /// used to consume `u32 + string + 14` unconditionally with a comment
    /// claiming the block is always there. That claim is traceable to go-sro's
    /// zero-writer `WriteGuild` (`model/packetutils_entity.go:331-342`) — the
    /// stub server our dumps were captured against, which has **no job mode at
    /// all** and therefore cannot ever omit it. The original client's own
    /// third-party parser branches: xBot reads the guild name and then skips
    /// the whole id…authority sub-block when its job-mode predicate holds
    /// (`PacketParser.cs:750-766`, `SRPlayer.cs:49-54`; job players render as
    /// `*Name`). One job-suited player in view desynced the entire spawn batch
    /// (the local RE notes).
    ///
    /// Confidence: `[S]` — the *branch* is spec-derived from xBot (no licence;
    /// facts and field layout only) and not yet seen on real bytes: probing all 868
    /// frames of `packet_dump/0x3019.log` for the little-endian ref id of every
    /// player row in `characterdata*.txt` (26 ids) turns up no player record
    /// (positive control on the identical probe: NPC ref 2013 = `dd070000`
    /// appears in 14 frames). A capture of a job-suited player spawn closes
    /// it; if it comes back the other way, delete the branch, not the fields.
    ///
    /// `None` (a short read) aborts the record like every other parse failure
    /// here.
    pub fn guild(&mut self, job_mode: bool) -> Option<GuildBlock> {
        let name = self.string()?;
        if job_mode {
            return Some(GuildBlock {
                tag: GuildTag {
                    name,
                    granted_nick: String::new(),
                },
                affiliation: None,
            });
        }
        let id = self.u32()?;
        let granted_nick = self.string()?;
        let affiliation = GuildAffiliation {
            id,
            crest_rev: self.u32()?,
            union_id: self.u32()?,
            union_crest_rev: self.u32()?,
            is_friendly: self.u8()? != 0,
            siege_authority: self.u8()?,
        };
        Some(GuildBlock {
            tag: GuildTag { name, granted_nick },
            affiliation: Some(affiliation),
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// A trailing sentinel byte proves exactly the right number of coordinate
    /// bytes was consumed for each region kind.
    #[test]
    fn skip_movement_widths() {
        // Overworld current region: 3 × u16 destination coords.
        let mut b: Vec<u8> = vec![1, 0];
        b.extend_from_slice(&0x60A8u16.to_le_bytes());
        b.extend_from_slice(&[0; 6]);
        b.push(0xEE);
        let mut r = Reader::new(&b);
        r.skip_movement(0x60A8).unwrap();
        assert_eq!(r.u8(), Some(0xEE));

        // Dungeon current region: 3 × i32 destination coords.
        let mut b: Vec<u8> = vec![1, 0];
        b.extend_from_slice(&0x8001u16.to_le_bytes());
        b.extend_from_slice(&[0; 12]);
        b.push(0xEE);
        let mut r = Reader::new(&b);
        r.skip_movement(0x8001).unwrap();
        assert_eq!(r.u8(), Some(0xEE));

        // Destination region 0: no coordinate triple at all.
        let mut b: Vec<u8> = vec![1, 0];
        b.extend_from_slice(&0u16.to_le_bytes());
        b.push(0xEE);
        let mut r = Reader::new(&b);
        r.skip_movement(0x8001).unwrap();
        assert_eq!(r.u8(), Some(0xEE));

        // Standing turn: source byte + heading, independent of region kind.
        let b: Vec<u8> = vec![0, 1, 1, 0xEC, 0x2F, 0xEE];
        let mut r = Reader::new(&b);
        r.skip_movement(0x8001).unwrap();
        assert_eq!(r.u8(), Some(0xEE));
    }

    fn guild_bytes(name: &str, nick: &str) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&(name.len() as u16).to_le_bytes());
        b.extend_from_slice(name.as_bytes());
        b.extend_from_slice(&7u32.to_le_bytes()); // guild id
        b.extend_from_slice(&(nick.len() as u16).to_le_bytes());
        b.extend_from_slice(nick.as_bytes());
        b.extend_from_slice(&3u32.to_le_bytes()); // guild crest rev
        b.extend_from_slice(&9u32.to_le_bytes()); // union id
        b.extend_from_slice(&4u32.to_le_bytes()); // union crest rev
        b.push(0); // is_friendly = false -> at war
        b.push(0xFF); // authority: None
        b
    }

    /// The six fields behind the tag were skipped as 14 anonymous bytes; a
    /// sentinel proves the width is unchanged and the values now land.
    #[test]
    fn guild_block_full_reads_every_field() {
        let mut b = guild_bytes("Ironclad", "Quartermaster");
        b.push(0xEE);
        let mut r = Reader::new(&b);
        let block = r.guild(false).unwrap();
        assert_eq!(block.tag.name, "Ironclad");
        assert_eq!(block.tag.granted_nick, "Quartermaster");
        let a = block
            .affiliation
            .expect("non-job record carries the sub-block");
        assert_eq!(a.id, 7);
        assert_eq!(a.crest_rev, 3);
        assert_eq!(a.union_id, 9);
        assert_eq!(a.union_crest_rev, 4);
        assert!(!a.is_friendly);
        assert_eq!(a.siege_authority, 0xFF);
        assert_eq!(r.u8(), Some(0xEE), "block width must be unchanged");
    }

    /// In job mode only the name string is on the wire: reading the
    /// sub-block anyway ate the following record's bytes.
    #[test]
    fn guild_block_in_job_mode_is_name_only() {
        let b: Vec<u8> = vec![3, 0, b'A', b'B', b'C', 0xEE];
        let mut r = Reader::new(&b);
        let block = r.guild(true).unwrap();
        assert_eq!(block.tag.name, "ABC");
        assert_eq!(block.tag.granted_nick, "");
        assert_eq!(block.affiliation, None);
        assert_eq!(r.u8(), Some(0xEE), "nothing behind the name may be eaten");
    }
}
