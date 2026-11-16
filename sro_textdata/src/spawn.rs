//! Which spawn record a ref id has, read from the reference tables: the one
//! [`RefResolver`] the spawn reader in `packets` asks, for the client and the
//! server alike. An item row comes first (a dropped item), then a
//! characterdata row (a player, a monster, a COS, or the NPC layout for every
//! other character), then a gate building, which has no characterdata row.

use std::collections::HashMap;

use packets::agent::entity_spawn::{RefResolver, RefType};

use crate::characterdata::CharacterData;
use crate::itemdata::ItemData;
use crate::teleportbuilding::GateBuilding;

/// The tables a spawn record's layout depends on; one not loaded yet is
/// `None`, and then nothing resolves through it.
#[derive(Clone, Copy, Default)]
pub struct SpawnTables<'a> {
    pub characters: Option<&'a CharacterData>,
    pub items: Option<&'a ItemData>,
    pub gates: Option<&'a HashMap<i32, GateBuilding>>,
}

impl RefResolver for SpawnTables<'_> {
    fn resolve(&self, ref_id: u32) -> RefType {
        let id = ref_id as i32;
        if let Some(item) = self.items.and_then(|items| items.get(&id)) {
            return RefType::Item {
                equipment: item.is_equipment(),
                gold: item.is_gold(),
                named: matches!(item.type_ids(), Some((3, 3, 8 | 9, _))),
            };
        }
        if let Some(ch) = self.characters.and_then(|characters| characters.get(&id)) {
            if ch.is_player() {
                return RefType::Player;
            }
            if ch.is_monster() {
                let extra = matches!(ch.type_ids(), Some((1, 2, 1, 2 | 3)));
                return RefType::Monster { extra };
            }
            if let Some(kind) = ch.cos_kind() {
                return RefType::Cos { kind };
            }
            // is_npc, plus any remaining uncategorized character-family row
            // (quest clones 1/2/3/6..8, …), parses with the NPC record layout.
            return RefType::Npc;
        }
        if self.gates.is_some_and(|gates| gates.contains_key(&id)) {
            return RefType::Structure;
        }
        RefType::Unknown
    }

    fn item_is_equipment(&self, ref_id: u32) -> bool {
        let item = self.items.and_then(|items| items.get(&(ref_id as i32)));
        item.is_some_and(|item| item.is_equipment())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::characterdata::CharacterDataRow;
    use crate::itemdata::ItemDataRow;
    use packets::agent::pet::CosKind;

    /// A row with `id` in column 1, `code` in column 2 and the type ids in
    /// columns 9-12, as both tables have them.
    fn row(id: i32, code: &str, tid: [u32; 4]) -> Vec<String> {
        let mut fields = vec![String::from("0"); 60];
        (fields[1], fields[2]) = (id.to_string(), code.to_owned());
        for (i, t) in tid.iter().enumerate() {
            fields[9 + i] = t.to_string();
        }
        fields
    }

    #[test]
    fn each_table_decides_its_records_and_an_item_row_comes_first() {
        let characters = CharacterData(HashMap::from([
            (
                1907,
                CharacterDataRow(row(1907, "CHAR_CH_MAN_ADVENTURER", [1, 1, 0, 0])),
            ),
            (
                1933,
                CharacterDataRow(row(1933, "MOB_CH_MANGNYANG", [1, 2, 1, 1])),
            ),
            (
                5000,
                CharacterDataRow(row(5000, "MOB_EU_THIEF_NPC_0001", [1, 2, 1, 2])),
            ),
            (
                2003,
                CharacterDataRow(row(2003, "NPC_CH_SMITH", [1, 2, 2, 0])),
            ),
            (3, CharacterDataRow(row(3, "COS_C_HORSE1", [1, 2, 3, 1]))),
            (
                8,
                CharacterDataRow(row(8, "SHADOWED_BY_AN_ITEM", [1, 2, 2, 0])),
            ),
        ]));
        let items = ItemData(HashMap::from([
            (8, ItemDataRow(row(8, "ITEM_CH_SWORD_01_A", [3, 1, 6, 2]))),
            (1, ItemDataRow(row(1, "ITEM_ETC_GOLD_01", [3, 3, 5, 0]))),
            (
                3862,
                ItemDataRow(row(3862, "ITEM_ETC_E050618_TREASUREBOX", [3, 3, 9, 0])),
            ),
        ]));
        let gate = GateBuilding {
            codename: "STORE_CH_GATE".to_owned(),
            name_key: "SN_NPC_CH_GATE".to_owned(),
        };
        let gates = HashMap::from([(2094, gate)]);
        let tables = SpawnTables {
            characters: Some(&characters),
            items: Some(&items),
            gates: Some(&gates),
        };
        let item = |equipment, gold, named| RefType::Item {
            equipment,
            gold,
            named,
        };
        assert_eq!(tables.resolve(1907), RefType::Player);
        assert_eq!(tables.resolve(1933), RefType::Monster { extra: false });
        assert_eq!(tables.resolve(5000), RefType::Monster { extra: true });
        assert_eq!(tables.resolve(2003), RefType::Npc);
        let horse = CosKind::from_type_id4(1).expect("tid4 1 is a COS");
        assert_eq!(tables.resolve(3), RefType::Cos { kind: horse });
        assert_eq!(tables.resolve(8), item(true, false, false));
        assert_eq!(tables.resolve(1), item(false, true, false));
        assert_eq!(tables.resolve(3862), item(false, false, true));
        assert_eq!(tables.resolve(2094), RefType::Structure);
        assert_eq!(tables.resolve(9999), RefType::Unknown);
        assert!(tables.item_is_equipment(8) && !tables.item_is_equipment(1));
        assert_eq!(SpawnTables::default().resolve(1907), RefType::Unknown);
    }
}
