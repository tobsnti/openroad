//! `teleportbuilding.txt`: the gate buildings, keyed by their ref id. These
//! refs have no characterdata row, so a spawn record of one is told apart by
//! this table alone (the Jangan dimensional gate, ref 2094).

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateBuilding {
    /// `STORE_CH_GATE`-style codename — the npcchat/speech lookup key.
    pub codename: String,
    /// `SN_NPC_*_GATE` display key.
    pub name_key: String,
}

/// `service|ref id|codename|kr|xxx|name_strid|…`; rows of a disabled service
/// and rows too short to hold a name are skipped.
pub fn parse(content: &str) -> HashMap<i32, GateBuilding> {
    let mut buildings = HashMap::new();
    for line in content.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 6 || fields[0].trim_start_matches('\u{feff}') != "1" {
            continue;
        }
        if let Ok(owner) = fields[1].parse::<i32>() {
            let building = GateBuilding {
                codename: fields[2].to_string(),
                name_key: fields[5].to_string(),
            };
            buildings.insert(owner, building);
        }
    }
    buildings
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn reads_a_gate_building_and_skips_a_disabled_row() {
        let rows = "1\t2094\tSTORE_CH_GATE\t????\txxx\tSN_NPC_CH_GATE\txxx\t0\n\
                    0\t2095\tSTORE_OFF\t????\txxx\tSN_OFF\txxx\t0\n";
        let gate = GateBuilding {
            codename: "STORE_CH_GATE".to_owned(),
            name_key: "SN_NPC_CH_GATE".to_owned(),
        };
        assert_eq!(parse(rows), HashMap::from([(2094, gate)]));
    }
}
