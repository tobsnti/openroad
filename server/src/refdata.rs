//! Reference data from the game's own tables, so that values like a
//! character's speeds come from the user's data rather than from code. The
//! rows are `sro_textdata`'s, the type the client reads them with, so each
//! column is named in one place; this module only finds and splits the files.
//! Only what the server uses so far is read.

use std::collections::HashMap;
use std::path::Path;

use sro_textdata::characterdata::CharacterDataRow;

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
}

impl RefData {
    /// Every `characterdata*.txt` under `textdata`.
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
