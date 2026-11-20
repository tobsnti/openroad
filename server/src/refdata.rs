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
use sro_textdata::skilldata::SkillData;
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

/// A weapon's basic attack, as skilldata and itemdata hold it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BasicAttack {
    pub skill: u32,
    /// How long one swing lasts (skilldata Action_ActionDuration).
    pub duration_ms: u32,
    pub hits: u8,
    /// The lowest and the highest damage of one hit.
    pub damage: (u32, u32),
    /// How far the weapon reaches past the bodies, in world units.
    pub reach: f32,
}

pub struct RefData {
    characters: CharacterData,
    items: ItemData,
    npcs: Vec<NpcPos>,
    fortresses: FortressTable,
    skills: SkillData,
}

impl Default for RefData {
    fn default() -> Self {
        RefData {
            characters: CharacterData(HashMap::new()),
            items: ItemData(HashMap::new()),
            npcs: Vec::new(),
            fortresses: FortressTable::default(),
            skills: SkillData(HashMap::new()),
        }
    }
}

impl RefData {
    /// Every `characterdata*.txt`, `itemdata*.txt` and plain
    /// `skilldata_*.txt` under `textdata`, `npcpos.txt` and
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
            } else if name.starts_with("itemdata") && name.ends_with(".txt") {
                data.add_items(&text(&std::fs::read(&path)?));
            } else if name == "npcpos.txt" {
                data.add_npcs(&text(&std::fs::read(&path)?));
            } else if name == "siegefortress.txt" {
                data.add_fortresses(&text(&std::fs::read(&path)?));
            } else if name.starts_with("skilldata_") && !name.ends_with("enc.txt") {
                data.add_skills(&text(&std::fs::read(&path)?));
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

    /// The rows of one skilldata table.
    pub fn add_skills(&mut self, table: &str) {
        self.skills.0.extend(SkillData::parse_shard(table).0);
    }

    /// The basic attack of a character holding `weapon` (an itemdata ref), or
    /// bare-handed. It is the player skill (`SKILL_`) of level 1 that is cast,
    /// stays out of the skill book (tab 255), needs no mastery level, and
    /// names the weapon's type (TypeID4) among its cast weapons; bare hands
    /// are type 1, which only the punch names. The user's data holds exactly
    /// one such skill for each of the 15 types [S].
    ///
    /// Its damage is a deliberate simplification [U], decision 0020: the
    /// weapon's white attack (the lower bounds of its itemdata ranges) times
    /// the skill's attack share, because the character's own values and the
    /// target's defence are not built yet; bare hands deal 1. On the vSRO
    /// server a Lv 21 sword 03_C hit a Mangnyang for 43 to 46, where this
    /// gives 46 to 52.
    pub fn basic_attack(&self, weapon: Option<u32>) -> Option<BasicAttack> {
        let weapon = weapon.and_then(|id| self.items.0.get(&i32::try_from(id).ok()?));
        let kind = match weapon.map(|w| w.type_ids()) {
            None => 1,
            Some(Some((3, 1, 6, kind))) => u8::try_from(kind).ok()?,
            Some(_) => return None,
        };
        let basic = |row: &&sro_textdata::skilldata::SkillDataRow| {
            let names = row
                .required_weapons()
                .is_some_and(|(a, b)| a == kind || b == kind);
            row.code_name().starts_with("SKILL_")
                && row.basic_level() == 1
                && row.activity() == 2
                && row.ui_grid().is_none()
                && row.mastery_req().is_none_or(|(_, level)| level == 0)
                && names
        };
        let (&skill, row) = self.skills.0.iter().find(|(_, row)| basic(row))?;
        let share = row.attack_percent()? as f32 / 100.0;
        let hit = |attack: f32| (attack * share).round().max(1.0) as u32;
        let white = |stat| weapon?.stat_range(stat).map(|(lower, _)| lower);
        let damage = match (white(StatRange::PhyAtkMin), white(StatRange::PhyAtkMax)) {
            (Some(min), Some(max)) => (hit(min), hit(max)),
            _ => (1, 1),
        };
        Some(BasicAttack {
            skill: u32::try_from(skill).ok()?,
            duration_ms: row.action_duration_ms(),
            hits: row.hit_count(),
            damage,
            reach: weapon.and_then(|w| w.attack_reach()).unwrap_or(0.0),
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A table row: `values` at their columns, 0 elsewhere.
    fn row(width: usize, values: &[(usize, &str)]) -> String {
        let mut row = vec!["0"; width];
        for (at, value) in values {
            row[*at] = value;
        }
        row.join("\t")
    }

    /// A basic attack's skilldata row: level 1, cast (activity 2), out of the
    /// skill book (tab 255), its duration, its cast weapons and its
    /// parameters from column 69 on.
    fn basic(id: &str, code: &str, duration: &str, weapons: [&str; 2], params: &[&str]) -> String {
        let mut values = vec![(1, id), (3, code), (7, "1"), (8, "2"), (13, duration)];
        values.extend([(50, weapons[0]), (51, weapons[1]), (57, "255")]);
        values.extend(params.iter().enumerate().map(|(at, p)| (69 + at, *p)));
        row(119, &values)
    }

    /// The sword of the bench and the basic attacks of the bare fist and the
    /// sword, with the values the user's data holds for them
    /// (itemdata_5000.txt:1683, skilldata_5000.txt:1 and :2).
    fn armoury() -> RefData {
        let mut data = RefData::default();
        let sword = [
            (1, "3632"),
            (9, "3"),
            (10, "1"),
            (11, "6"),
            (12, "2"),
            (94, "6"),
        ];
        let white = [(95, "15"), (96, "16"), (97, "16"), (98, "18")];
        data.add_items(&row(160, &[&sword[..], &white[..]].concat()));
        let att = |share| ["6386804", "5", share, "0", "0", share];
        data.add_skills(&basic(
            "1",
            "SKILL_PUNCH_01",
            "1500",
            ["1", "255"],
            &att("150"),
        ));
        let sword_base = [&att("60")[..], &["28003", "2", "2"]].concat();
        data.add_skills(&basic(
            "2",
            "SKILL_CH_SWORD_BASE_01",
            "1200",
            ["2", "3"],
            &sword_base,
        ));
        data
    }

    /// The sword strikes with the skill that names its type, for that skill's
    /// duration and hits; a hit deals the sword's white attack (the lower
    /// bounds) times the skill's share, as the rows above say. Bare hands
    /// punch for 1.
    #[test]
    fn a_weapons_basic_attack_comes_from_its_rows() {
        let data = armoury();
        let share = |attack: f32| (attack * 60.0 / 100.0).round() as u32;
        let sword = BasicAttack {
            skill: 2,
            duration_ms: 1200,
            hits: 2,
            damage: (share(15.0), share(16.0)),
            reach: 6.0,
        };
        let fist = BasicAttack {
            skill: 1,
            duration_ms: 1500,
            hits: 1,
            damage: (1, 1),
            reach: 0.0,
        };
        assert_eq!(data.basic_attack(Some(3632)), Some(sword));
        assert_eq!(data.basic_attack(None), Some(fist));
    }

    /// The bench's weapons in the user's data: the sword
    /// (itemdata_5000.txt:1683, white attack 15..16 to 16..18) strikes with
    /// SKILL_CH_SWORD_BASE_01 (skilldata_5000.txt:2: 1200 ms, share 60 %,
    /// 'mc' 2 2), the glaive (itemdata_5000.txt:1686, 16..17 to 20..22) with
    /// SKILL_CH_SPEAR_BASE_01 (skilldata_5000.txt:40: 1166 ms, 117 %, no
    /// 'mc'), bare hands with SKILL_PUNCH_01 (skilldata_5000.txt:1, 1500 ms).
    #[test]
    #[ignore = "needs OPENROAD_TEXTDATA_DIR, the user's textdata folder"]
    fn the_benchs_weapons_strike_as_the_users_data_says() {
        let dir = std::env::var("OPENROAD_TEXTDATA_DIR").unwrap();
        let data = RefData::load(Path::new(&dir)).unwrap();
        let share = |attack: f32, percent: f32| (attack * percent / 100.0).round() as u32;
        let sword = data.basic_attack(data.item_named("ITEM_CH_SWORD_01_A_DEF"));
        let expected = BasicAttack {
            skill: 2,
            duration_ms: 1200,
            hits: 2,
            damage: (share(15.0, 60.0), share(16.0, 60.0)),
            reach: 6.0,
        };
        assert_eq!(sword, Some(expected));
        let glaive = data.basic_attack(data.item_named("ITEM_CH_TBLADE_01_A_DEF"));
        let expected = BasicAttack {
            skill: 40,
            duration_ms: 1166,
            hits: 1,
            damage: (share(16.0, 117.0), share(20.0, 117.0)),
            reach: 18.0,
        };
        assert_eq!(glaive, Some(expected));
        let fist = data
            .basic_attack(None)
            .map(|a| (a.skill, a.duration_ms, a.hits));
        assert_eq!(fist, Some((1, 1500, 1)));
    }
}
