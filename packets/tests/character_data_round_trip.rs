//! Replay captured CHARACTER_DATA (0x3013) bodies through the reader and the
//! writer: every body the reader parses fully must come back byte for byte.
//!
//! It needs the user's own data, so it is ignored by default:
//! `OPENROAD_PACKET_DUMP_DIR` is a packet_dump tree (as for
//! `captured_character_data.rs`), `OPENROAD_ITEMDATA_DIR` the folder with the
//! unpacked `itemdata*.txt`. Item classes follow the client's mapping
//! (`ClientItemData` in the client), so the reader routes what the game would.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use packets::agent::character_data::{
    parse_character_data, parse_character_info, write_character_info, ItemClass, ItemClassResolver,
};

mod dump;
use dump::{bodies, dump_root, logs};

/// Item classes from the itemdata rows: column 1 is the id, 9 to 12 the four
/// type ids.
struct Itemdata(HashMap<u32, ItemClass>);

impl Itemdata {
    fn load(dir: &Path) -> Itemdata {
        let mut classes = HashMap::new();
        for entry in std::fs::read_dir(dir).expect("OPENROAD_ITEMDATA_DIR is readable") {
            let path = entry.expect("the folder is readable").path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let name = name.to_ascii_lowercase();
            if !(name.starts_with("itemdata") && name.ends_with(".txt")) {
                continue;
            }
            for line in text(&std::fs::read(&path).expect("the table is readable")).lines() {
                let cols: Vec<&str> = line.split('\t').collect();
                let field = |i: usize| cols.get(i)?.trim().parse::<u32>().ok();
                let (Some(id), Some(t1), Some(t2), Some(t3), Some(t4)) =
                    (field(1), field(9), field(10), field(11), field(12))
                else {
                    continue;
                };
                let class = match (t1, t2) {
                    (3, 1) => ItemClass::Equipment,
                    (3, 2) => ItemClass::Container { tid3: t3, tid4: t4 },
                    (3, 3) => ItemClass::Expendable { tid3: t3, tid4: t4 },
                    _ => continue,
                };
                classes.insert(id, class);
            }
        }
        Itemdata(classes)
    }
}

/// UTF-16LE behind its byte-order mark, as the game ships the tables, or UTF-8.
fn text(bytes: &[u8]) -> String {
    match bytes.strip_prefix(&[0xFF, 0xFE]) {
        Some(utf16) => {
            let (pairs, _) = utf16.as_chunks::<2>();
            let units: Vec<u16> = pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
            String::from_utf16_lossy(&units)
        }
        None => String::from_utf8_lossy(bytes).into_owned(),
    }
}

impl ItemClassResolver for Itemdata {
    fn item_class(&self, ref_id: u32) -> ItemClass {
        self.0.get(&ref_id).copied().unwrap_or(ItemClass::Unknown)
    }
}

#[test]
#[ignore = "needs the user's data: OPENROAD_PACKET_DUMP_DIR=<packet_dump> \
            OPENROAD_ITEMDATA_DIR=<textdata> cargo test -p packets \
            --test character_data_round_trip -- --ignored"]
fn every_fully_read_captured_body_is_written_back_byte_for_byte() {
    let root = dump_root().expect("OPENROAD_PACKET_DUMP_DIR names a packet_dump tree");
    let root = root.as_path();
    let itemdata = std::env::var("OPENROAD_ITEMDATA_DIR").expect("OPENROAD_ITEMDATA_DIR is set");
    let items = Itemdata::load(Path::new(&itemdata));
    let ids: Vec<u32> = logs(root, "0x3020.log")
        .iter()
        .flat_map(|log| bodies(log))
        .filter_map(|body| Some(u32::from_le_bytes(body.get(..4)?.try_into().ok()?)))
        .collect();
    let (mut total, mut written_back) = (0, 0);
    let mut not_read: BTreeMap<&str, usize> = BTreeMap::new();
    for log in logs(root, "0x3013.log") {
        for (index, body) in bodies(&log).into_iter().enumerate() {
            total += 1;
            let id = ids
                .iter()
                .copied()
                .find(|&id| parse_character_data(&body, Some(id)).is_some());
            let info = parse_character_info(&body, id, &items);
            if !info.fully_parsed {
                let stage = info.failed_stage.unwrap_or("bytes left over");
                *not_read.entry(stage).or_default() += 1;
                continue;
            }
            // A width the resync only inferred is not a record the writer can
            // reproduce; the writer refuses it by design.
            if info.recovered_body.is_some() {
                *not_read.entry("item width only recovered").or_default() += 1;
                continue;
            }
            let at = format!("{}:{index}", log.display());
            let written =
                write_character_info(&info, &items).unwrap_or_else(|e| panic!("{at}: {e}"));
            assert!(written == body, "{at}: the written body differs");
            written_back += 1;
        }
    }
    eprintln!("{written_back} of {total} bodies written back; the rest: {not_read:?}");
    assert!(written_back > 0, "no captured body was read fully");
}
