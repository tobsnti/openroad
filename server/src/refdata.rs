//! Reference data from the game's own tables, so that values like a
//! character's speeds come from the user's data rather than from code. The
//! rows are `sro_textdata`'s, the type the client reads them with, so each
//! column is named in one place; this module only finds and splits the files.
//! Only what the server uses so far is read.

use std::collections::HashMap;
use std::path::Path;

use sro_textdata::characterdata::CharacterDataRow;
use sro_textdata::npcpos::{self, NpcPos};
use sro_textdata::siegefortress::FortressTable;

/// What the server takes from a characterdata row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterRef {
    /// Walk and run speed in world units per second.
    pub walk_speed: f32,
    pub run_speed: f32,
}

#[derive(Default)]
pub struct RefData {
    characters: HashMap<u32, CharacterDataRow>,
    npcs: Vec<NpcPos>,
    fortresses: FortressTable,
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

    /// The rows of one characterdata table. As in the client's loader, a row
    /// has more than ten columns and its id in the second; the index file
    /// `characterdata.txt` only names the others.
    pub fn add_characters(&mut self, table: &str) {
        for line in table.lines() {
            let cols: Vec<String> = line.split('\t').map(String::from).collect();
            let id = cols.get(1).and_then(|id| id.trim().parse().ok());
            if let (true, Some(id)) = (cols.len() > 10, id) {
                self.characters.insert(id, CharacterDataRow(cols));
            }
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
        let row = self.characters.get(&ref_id);
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
        let Some(row) = self.characters.get(&ref_id) else {
            return false;
        };
        let typed = matches!(row.type_ids(), Some((1, 2, 2, _) | (4, _, _, _)));
        let listed = |name: &str| self.fortresses.0.values().any(|f| f.codename == name);
        typed && row.fortress().is_some_and(listed)
    }

    /// The characterdata row `ref_id`, as far as the server uses it.
    pub fn character(&self, ref_id: u32) -> Option<CharacterRef> {
        let row = self.characters.get(&ref_id)?;
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
