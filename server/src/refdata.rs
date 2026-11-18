//! Reference data from the game's own tables, so that values like a
//! character's speeds come from the user's data rather than from code. The
//! tables are `sro_textdata`'s, the types the client reads them with, so each
//! column is named in one place and a spawn record's layout is decided by the
//! same mapping as in the client ([`SpawnTables`]); this module only finds
//! the files. Only what the server uses so far is read.

use std::collections::HashMap;
use std::path::Path;

use sro_textdata::characterdata::{CharacterData, CharacterDataRow};
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

pub struct RefData {
    characters: CharacterData,
    npcs: Vec<NpcPos>,
    fortresses: FortressTable,
}

impl Default for RefData {
    fn default() -> Self {
        RefData {
            characters: CharacterData(HashMap::new()),
            npcs: Vec::new(),
            fortresses: FortressTable::default(),
        }
    }
}

impl RefData {
    /// Every `characterdata*.txt` under `textdata`, `npcpos.txt` and
    /// `siegefortress.txt`.
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

    /// The tables a spawn record's layout depends on, for the spawn writer.
    /// The server sends no dropped items and no gate buildings yet, so their
    /// tables are not read.
    pub fn spawn_tables(&self) -> SpawnTables<'_> {
        SpawnTables {
            characters: Some(&self.characters),
            items: None,
            gates: None,
        }
    }

    /// The rows of `npcpos.txt`: where the data puts its NPCs.
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
