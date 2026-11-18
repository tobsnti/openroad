//! Opt-in census of the spawn reader and writer against the user's own
//! captures: every captured single spawn (0x3015) and every group (0x3017
//! begin + 0x3019 data) that can be paired with its begin must read to its
//! last byte with the user's own tables, and write back to the same bytes.
//! Needs `OPENROAD_TEXTDATA_DIR` (the textdata folder) and
//! `OPENROAD_PACKET_DUMP_DIR` (a `packet_dump` folder; every `0x3015.log`
//! and `0x3019.log` below it counts):
//!
//! `cargo test -p sro_textdata --test spawn_census -- --ignored --nocapture`
//!
//! The dump keeps one file per opcode, each line `<time> <hex> [E|P]`; a
//! group's begin, data and end sit in three files. Groups do not interleave,
//! so where a folder holds as many begins as data frames and ends, and each
//! begin, data frame and end of the same index follow each other in time, the
//! data frame of index i is the data of begin i. A folder where that does not
//! hold is counted, not read.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use packets::agent::entity_spawn::{
    parse_group_spawn, parse_single_spawn, RefResolver, RefType, SpawnKind, SpawnNote,
    SpawnedEntity,
};
use packets::agent::spawn_writer::{write_group_despawn, write_group_spawn, write_single_spawn};
use sro_textdata::characterdata::CharacterData;
use sro_textdata::itemdata::ItemData;
use sro_textdata::spawn::SpawnTables;
use sro_textdata::teleportbuilding;

/// A textdata file: UTF-16LE behind a byte-order mark, or UTF-8.
fn text(bytes: &[u8]) -> String {
    match bytes.strip_prefix(&[0xFF, 0xFE]) {
        Some(utf16) => {
            let units: Vec<u16> = utf16
                .chunks_exact(2)
                .map(|p| u16::from_le_bytes([p[0], p[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        None => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// The frames of one dump file: timestamp and body.
fn frames(path: &Path) -> Vec<(String, Vec<u8>)> {
    let Ok(log) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    log.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let ts = fields.next()?.to_owned();
            let hex = fields
                .next()
                .filter(|h| !matches!(*h, "E" | "P"))
                .unwrap_or("");
            let body = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
                .collect::<Option<Vec<u8>>>()?;
            Some((ts, body))
        })
        .collect()
}

/// Every folder below `dir` (itself included) that holds `file`.
fn folders_with(dir: &Path, file: &str, out: &mut Vec<PathBuf>) {
    if dir.join(file).is_file() {
        out.push(dir.to_path_buf());
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.path().is_dir() {
            folders_with(&entry.path(), file, out);
        }
    }
}

/// Players written back to the same bytes, those of them with equipment, and
/// those with avatar items.
#[derive(Default)]
struct Players(usize, usize, usize);

impl Players {
    fn count(&mut self, entity: &SpawnedEntity) {
        if let SpawnKind::Player {
            equipment, avatar, ..
        } = &entity.kind
        {
            self.0 += 1;
            self.1 += usize::from(!equipment.is_empty());
            self.2 += usize::from(!avatar.is_empty());
        }
    }
}

#[test]
#[ignore = "needs OPENROAD_TEXTDATA_DIR and OPENROAD_PACKET_DUMP_DIR"]
fn every_captured_spawn_reads_to_its_last_byte_and_writes_back() {
    let textdata = std::env::var("OPENROAD_TEXTDATA_DIR").expect("OPENROAD_TEXTDATA_DIR");
    let dumps =
        PathBuf::from(std::env::var("OPENROAD_PACKET_DUMP_DIR").expect("OPENROAD_PACKET_DUMP_DIR"));
    let mut characters = CharacterData(HashMap::new());
    let mut items = ItemData(HashMap::new());
    let mut gates = HashMap::new();
    for entry in std::fs::read_dir(&textdata)
        .expect("the textdata folder")
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        let read = || text(&std::fs::read(entry.path()).expect("a textdata file"));
        if name.starts_with("characterdata_") && name.ends_with(".txt") {
            characters.0.extend(CharacterData::parse_shard(&read()).0);
        } else if name.starts_with("itemdata_") && name.ends_with(".txt") {
            items.0.extend(ItemData::parse_shard(&read()).0);
        } else if name == "teleportbuilding.txt" {
            gates = teleportbuilding::parse(&read());
        }
    }
    let tables = SpawnTables {
        characters: Some(&characters),
        items: Some(&items),
        gates: Some(&gates),
    };
    let mut failed = Vec::new();
    let (mut single_players, mut group_players) = (Players::default(), Players::default());

    let mut singles = HashSet::new();
    let (mut unknown_ref, mut single_read, mut single_written) = (0, 0, 0);
    let mut folders = Vec::new();
    folders_with(&dumps, "0x3015.log", &mut folders);
    for folder in &folders {
        for (ts, body) in frames(&folder.join("0x3015.log")) {
            if !singles.insert((ts.clone(), body.clone())) {
                continue;
            }
            let ref_id = body
                .get(..4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()));
            if ref_id.map(|id| tables.resolve(id)) == Some(RefType::Unknown) {
                unknown_ref += 1;
            } else if let Some((entity, tail)) = parse_single_spawn(&body, &tables) {
                single_read += 1;
                if write_single_spawn(&entity, &tail, &tables).as_ref() == Some(&body) {
                    single_written += 1;
                    single_players.count(&entity);
                } else {
                    failed.push(format!("0x3015 written back {} {ts}", folder.display()));
                }
            } else {
                failed.push(format!("0x3015 {} {ts}", folder.display()));
            }
        }
    }

    let mut groups = HashSet::new();
    let (mut paired, mut unpaired, mut group_skipped) = (0, 0, 0);
    let (mut group_read, mut group_written) = (0, 0);
    let mut folders = Vec::new();
    folders_with(&dumps, "0x3019.log", &mut folders);
    for folder in &folders {
        let begins = frames(&folder.join("0x3017.log"));
        let datas = frames(&folder.join("0x3019.log"));
        let ends = frames(&folder.join("0x3018.log"));
        let in_order = begins.len() == datas.len()
            && datas.len() == ends.len()
            && (0..datas.len()).all(|i| begins[i].0 <= datas[i].0 && datas[i].0 <= ends[i].0);
        for (i, (ts, body)) in datas.iter().enumerate() {
            if !groups.insert((ts.clone(), body.clone())) {
                continue;
            }
            let begin = in_order.then(|| &begins[i].1).and_then(|b| match b[..] {
                [kind, lo, hi, ..] => Some((kind == 1, u16::from_le_bytes([lo, hi]))),
                _ => None,
            });
            let Some((spawning, count)) = begin else {
                unpaired += 1;
                continue;
            };
            paired += 1;
            let parsed = parse_group_spawn(body, spawning, count, &tables);
            let read = match spawning {
                true => parsed.spawns.len() + parsed.unresolved.len(),
                false => parsed.despawns.len(),
            };
            let skipped = parsed
                .notes
                .iter()
                .all(|n| matches!(n, SpawnNote::Skipped { .. }));
            if read != count as usize || parsed.end != body.len() || !skipped {
                failed.push(format!("0x3019 {} {ts}", folder.display()));
            } else if parsed.notes.is_empty() {
                group_read += 1;
                let written = match spawning {
                    true => write_group_spawn(&parsed.spawns, &tables),
                    false => Some(write_group_despawn(&parsed.despawns)),
                };
                if written.as_ref() == Some(body) {
                    group_written += 1;
                    parsed.spawns.iter().for_each(|e| group_players.count(e));
                } else {
                    failed.push(format!("0x3019 written back {} {ts}", folder.display()));
                }
            } else {
                group_skipped += 1;
            }
        }
    }

    println!(
        "0x3015: {} distinct frames, {single_read} read to their last byte, {single_written} of \
         them written back to the same bytes, {unknown_ref} with a ref id in no table",
        singles.len()
    );
    println!(
        "0x3019: {} distinct frames, {paired} paired with their begin, {group_read} of them read to \
         their last byte, {group_written} of these written back to the same bytes, \
         {group_skipped} to their last byte past a record whose ref id is in no table (no layout \
         to write), {unpaired} not paired",
        groups.len()
    );
    for (source, Players(read, equipped, avatar)) in [
        ("in 0x3015", &single_players),
        ("in 0x3019", &group_players),
    ] {
        println!(
            "players {source}: {read} written back, {equipped} with equipment, \
             {avatar} with avatar items"
        );
    }
    assert!(
        single_read + group_read > 0,
        "no frame read below {}",
        dumps.display()
    );
    assert!(
        failed.is_empty(),
        "{} not read to their last byte or not written back:\n{}",
        failed.len(),
        failed.join("\n")
    );
    // Nothing else runs a player's two item lists end to end (the test
    // bench's players carry none), so the round trip has to cover them.
    assert!(
        group_players.1 > 0,
        "no captured group holds a player with equipment: the round trip misses it"
    );
    assert!(
        group_players.2 > 0,
        "no captured group holds a player with avatar items: the round trip misses them"
    );
}
