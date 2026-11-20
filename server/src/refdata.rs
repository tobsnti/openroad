//! Reference data from the game's own tables, so that values like a
//! character's speeds come from the user's data rather than from code. The
//! tables are `sro_textdata`'s, the types the client reads them with, so each
//! column is named in one place and a spawn record's layout is decided by the
//! same mapping as in the client ([`SpawnTables`]); this module only finds
//! the files. Only what the server uses so far is read.

use std::collections::HashMap;
use std::path::Path;

use sro_textdata::characterdata::{CharacterData, CharacterDataRow};
use sro_textdata::itemdata::{ItemData, StatRange};
use sro_textdata::npcpos::{self, NpcPos};
use sro_textdata::siegefortress::FortressTable;
use sro_textdata::spawn::SpawnTables;

/// What the server takes from a characterdata row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterRef {
    /// Walk and run speed in world units per second.
    pub walk_speed: f32,
    pub run_speed: f32,
}

/// What a monster's record and selection take from its characterdata row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonsterRef {
    pub max_hp: u32,
    pub rarity: u8,
    /// Walk and run speed in world units per second, 0 for a monster that
    /// does not move.
    pub walk_speed: f32,
    pub run_speed: f32,
}

pub struct RefData {
    characters: CharacterData,
    items: ItemData,
    npcs: Vec<NpcPos>,
    fortresses: FortressTable,
}

impl Default for RefData {
    fn default() -> Self {
        RefData {
            characters: CharacterData(HashMap::new()),
            items: ItemData(HashMap::new()),
            npcs: Vec::new(),
            fortresses: FortressTable::default(),
        }
    }
}

impl RefData {
    /// Every `characterdata*.txt` and `itemdata*.txt` under `textdata`,
    /// `npcpos.txt` and `siegefortress.txt`.
    pub fn load(textdata: &Path) -> std::io::Result<RefData> {
        let mut data = RefData::default();
        for entry in std::fs::read_dir(textdata)? {
            let path = entry?.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let name = name.to_ascii_lowercase();
            if name.starts_with("characterdata") && name.ends_with(".txt") {
                data.add_characters(&text(&std::fs::read(&path)?));
            } else if name.starts_with("itemdata") && name.ends_with(".txt") {
                data.add_items(&text(&std::fs::read(&path)?));
            } else if name == "npcpos.txt" {
                data.add_npcs(&text(&std::fs::read(&path)?));
            } else if name == "siegefortress.txt" {
                data.add_fortresses(&text(&std::fs::read(&path)?));
            }
        }
        Ok(data)
    }

    /// The rows of one characterdata table, read by the client's loader: a
    /// row has more than ten columns and its id in the second; the index file
    /// `characterdata.txt` only names the others.
    pub fn add_characters(&mut self, table: &str) {
        self.characters
            .0
            .extend(CharacterData::parse_shard(table).0);
    }

    fn row(&self, ref_id: u32) -> Option<&CharacterDataRow> {
        self.characters.0.get(&i32::try_from(ref_id).ok()?)
    }

    /// The rows of one itemdata table, read like a characterdata table.
    pub fn add_items(&mut self, table: &str) {
        self.items.0.extend(ItemData::parse_shard(table).0);
    }

    /// The ref id of the item whose code name is `code`.
    pub fn item_named(&self, code: &str) -> Option<u32> {
        let mut rows = self.items.0.iter();
        let (id, _) = rows.find(|(_, row)| row.code_name() == code)?;
        u32::try_from(*id).ok()
    }

    /// The durability a new item of `ref_id` carries: the lower bound of its
    /// itemdata range, 0 where it has none, as the vSRO server sends an item
    /// without variance [V capture, peer vSRO: 2205 of 2358 such worn items
    /// in 240 records, the rest worn below it].
    pub fn new_durability(&self, ref_id: u32) -> u32 {
        let row = i32::try_from(ref_id)
            .ok()
            .and_then(|id| self.items.0.get(&id));
        let range = row.and_then(|row| row.stat_range(StatRange::Durability));
        range.map_or(0, |(lower, _)| lower as u32)
    }

    /// The item table, for the item records' classes (0x3013).
    pub fn items(&self) -> &ItemData {
        &self.items
    }

    /// The tables a spawn record's layout depends on, for the spawn writer.
    /// The server sends no gate buildings yet, so their table is not read.
    pub fn spawn_tables(&self) -> SpawnTables<'_> {
        SpawnTables {
            characters: Some(&self.characters),
            items: Some(&self.items),
            gates: None,
        }
    }

    /// The rows of `npcpos.txt`: where the data puts its NPCs and monsters.
    pub fn add_npcs(&mut self, table: &str) {
        self.npcs.extend(npcpos::parse(table));
    }

    pub fn npcs(&self) -> &[NpcPos] {
        &self.npcs
    }

    /// Whether `ref_id` is an NPC's model: a characterdata row with the type
    /// ids 1/2/2.
    pub fn is_npc(&self, ref_id: u32) -> bool {
        let row = self.row(ref_id);
        row.is_some_and(|row| matches!(row.type_ids(), Some((1, 2, 2, _))))
    }

    /// The monster model `ref_id` (characterdata type ids 1/2/1); `None` for
    /// any other model and for a monster row without HP.
    pub fn monster(&self, ref_id: u32) -> Option<MonsterRef> {
        let row = self.row(ref_id).filter(|row| row.is_monster())?;
        Some(MonsterRef {
            max_hp: row.max_hp()?,
            rarity: row.rarity(),
            walk_speed: row.walk_speed().unwrap_or(0.0),
            run_speed: row.run_speed().unwrap_or(0.0),
        })
    }

    /// The rows of `siegefortress.txt`.
    pub fn add_fortresses(&mut self, table: &str) {
        self.fortresses.0.extend(FortressTable::parse(table).0);
    }

    /// Whether the original client reads a fortress's two bytes when an
    /// entity of model `ref_id` is selected: for an NPC row and a TID1 4 row
    /// whose fortress `siegefortress.txt` lists [V original handler].
    pub fn in_listed_fortress(&self, ref_id: u32) -> bool {
        let Some(row) = self.row(ref_id) else {
            return false;
        };
        let typed = matches!(row.type_ids(), Some((1, 2, 2, _) | (4, _, _, _)));
        let listed = |name: &str| self.fortresses.0.values().any(|f| f.codename == name);
        typed && row.fortress().is_some_and(listed)
    }

    /// The characterdata row `ref_id` of a player's model, as far as the
    /// server uses it; `None` for a row that is no player's (type ids 1/1),
    /// whose spawn record a player's would not be.
    pub fn character(&self, ref_id: u32) -> Option<CharacterRef> {
        let row = self.row(ref_id).filter(|row| row.is_player())?;
        let (walk_speed, run_speed) = (row.walk_speed()?, row.run_speed()?);
        Some(CharacterRef {
            walk_speed,
            run_speed,
        })
    }
}

/// UTF-16LE behind its byte-order mark, as the game ships its tables, or UTF-8.
fn text(bytes: &[u8]) -> String {
    match bytes.strip_prefix(&[0xFF, 0xFE]) {
        Some(utf16) => {
            let (pairs, _) = utf16.as_chunks::<2>();
            let units: Vec<u16> = pairs.iter().map(|p| u16::from_le_bytes(*p)).collect();
            String::from_utf16_lossy(&units)
        }
        None => String::from_utf8_lossy(bytes).into_owned(),
    }
}
