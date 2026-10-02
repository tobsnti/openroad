//! Print a `.2dt` window descriptor as text or JSON.
//!
//! The rects in these files are the authored layout a drawn window is checked
//! against, and reading them has meant reading bytes by hand: the parser lives
//! in the client and its entry point was private, so there was no way to ask a
//! file what it says without building a client around it. Two findings came out
//! of such hand reads — a duplicated `Id` and a file order that is not id order
//! — so both are reported here rather than left to the next reader.
//!
//! Coordinates: a descriptor's rects are absolute positions in one flat design
//! space, and a child is neither clipped to nor based on its parent. The table
//! therefore prints the local rect (`child.xy - root.xy`) next to the absolute
//! one, because positioning anything needs the first and comparing against the
//! file needs the second.

use std::collections::BTreeMap;
use std::path::Path;

use clap::{App, Arg};
use client::assets::twodt::JMXV2DT;
use serde_json::{json, Value};

/// One row of the table, pulled out of the asset so the shaping is testable
/// without a file on disk.
struct Row {
    id: u32,
    parent_id: u32,
    is_root: bool,
    name: String,
    ni_type: u32,
    abs: [f32; 4],
    local: [f32; 4],
    image: String,
    text: String,
}

fn rows(descriptor: &JMXV2DT) -> Vec<Row> {
    descriptor
        .entries()
        .iter()
        .map(|entry| {
            let abs = entry.rect();
            let local = descriptor.local_rect(entry);
            Row {
                id: entry.id(),
                parent_id: entry.parent_id(),
                is_root: entry.is_root(),
                name: entry.name().to_string(),
                ni_type: entry.raw_ni_type(),
                abs: [abs.min.x, abs.min.y, abs.width(), abs.height()],
                local: [local.min.x, local.min.y, local.width(), local.height()],
                image: entry.image().to_string(),
                text: entry.text().to_string(),
            }
        })
        .collect()
}

/// Ids that more than one entry carries, with how many entries carry them.
///
/// This matters because every lookup by id answers with the *first* match:
/// a duplicate id silently hides an entry, which is exactly what one hand read
/// found. Reporting it is cheap; finding it twice is not.
fn duplicate_ids(descriptor: &JMXV2DT) -> Vec<(u32, usize)> {
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for entry in descriptor.entries() {
        *counts.entry(entry.id()).or_default() += 1;
    }
    counts.into_iter().filter(|(_, count)| *count > 1).collect()
}

/// How many entries the header promised against how many were read. A short
/// tail means a truncated file, not an unknown layout.
fn missing_entries(descriptor: &JMXV2DT) -> u32 {
    descriptor
        .declared_entry_count()
        .saturating_sub(descriptor.entries().len() as u32)
}

/// True when the entries are not in ascending id order — one hand read found
/// `66, 67, 68, 65`, and anyone comparing a table against the file needs to
/// know that file order is its own thing.
fn file_order_differs_from_id_order(descriptor: &JMXV2DT) -> bool {
    descriptor
        .entries()
        .windows(2)
        .any(|pair| pair[0].id() > pair[1].id())
}

fn as_json(descriptor: &JMXV2DT, path: &Path) -> Value {
    let entries: Vec<Value> = rows(descriptor)
        .into_iter()
        .map(|row| {
            json!({
                "id": row.id,
                "parent_id": row.parent_id,
                "is_root": row.is_root,
                "name": row.name,
                "ni_type": row.ni_type,
                "abs": { "x": row.abs[0], "y": row.abs[1], "w": row.abs[2], "h": row.abs[3] },
                "local": { "x": row.local[0], "y": row.local[1], "w": row.local[2], "h": row.local[3] },
                "image": row.image,
                "text": row.text,
            })
        })
        .collect();
    json!({
        "file": path.display().to_string(),
        "declared_entry_count": descriptor.declared_entry_count(),
        "read_entries": descriptor.entries().len(),
        "missing_entries": missing_entries(descriptor),
        "duplicate_ids": duplicate_ids(descriptor)
            .into_iter()
            .map(|(id, count)| json!({ "id": id, "count": count }))
            .collect::<Vec<Value>>(),
        "file_order_differs_from_id_order": file_order_differs_from_id_order(descriptor),
        "entries": entries,
    })
}

fn print_table(descriptor: &JMXV2DT) {
    println!(
        "{:>5} {:>6} {:>4} {:>5} {:<34} {:<22} {:<22} {}",
        "id", "parent", "root", "type", "name", "local x,y,w,h", "abs x,y,w,h", "image"
    );
    for row in rows(descriptor) {
        let local = format!(
            "{},{},{},{}",
            row.local[0], row.local[1], row.local[2], row.local[3]
        );
        let abs = format!(
            "{},{},{},{}",
            row.abs[0], row.abs[1], row.abs[2], row.abs[3]
        );
        println!(
            "{:>5} {:>6} {:>4} {:>5} {:<34} {:<22} {:<22} {}",
            row.id,
            row.parent_id,
            if row.is_root { "yes" } else { "" },
            row.ni_type,
            row.name,
            local,
            abs,
            row.image
        );
    }
}

/// The lines a reader needs *before* trusting the table above.
fn print_notes(descriptor: &JMXV2DT) {
    let missing = missing_entries(descriptor);
    if missing > 0 {
        eprintln!(
            "note: the header declares {} entries and {} were read — {missing} missing, so the file is truncated",
            descriptor.declared_entry_count(),
            descriptor.entries().len()
        );
    }
    for (id, count) in duplicate_ids(descriptor) {
        eprintln!(
            "note: id {id} appears {count} times — every lookup by id answers with the first, so the others are invisible"
        );
    }
    if file_order_differs_from_id_order(descriptor) {
        eprintln!("note: file order is not ascending id order");
    }
    if descriptor.root().is_none() {
        eprintln!(
            "note: no entry is marked as the root, so the local rects equal the absolute ones"
        );
    }
}

fn main() {
    let matches = App::new("twodt_dump")
        .version("0.1.0")
        .about("Print a .2dt window descriptor: ids, parents, types, rects, art paths")
        .arg(
            Arg::with_name("file")
                .required(true)
                .help("Path to an extracted .2dt file"),
        )
        .arg(
            Arg::with_name("json")
                .long("json")
                .help("JSON instead of a table"),
        )
        .get_matches();

    let path = Path::new(matches.value_of("file").unwrap());
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("cannot read {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    if bytes.len() < 4 {
        eprintln!(
            "{} is {} bytes: a .2dt starts with a 4-byte entry count",
            path.display(),
            bytes.len()
        );
        std::process::exit(1);
    }
    let descriptor = JMXV2DT::from_bytes(&bytes);
    if matches.is_present("json") {
        let payload = as_json(&descriptor, path);
        println!("{}", serde_json::to_string_pretty(&payload).unwrap());
    } else {
        print_table(&descriptor);
    }
    print_notes(&descriptor);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte offsets inside one 976-byte entry, after the six fixed-size string
    /// fields (64 + 256 + 256 + 128 + 64 + 64 = 832).
    const STRINGS_LEN: usize = 832;
    const ENTRY_SIZE: usize = 976;
    const OFF_ID: usize = STRINGS_LEN + 4;
    const OFF_PARENT: usize = STRINGS_LEN + 8;
    const OFF_RECT_X: usize = STRINGS_LEN + 28;
    const OFF_IS_ROOT: usize = STRINGS_LEN + 84;

    /// One synthetic entry. Everything not named is zero, which parses as an
    /// empty string or a zero field — enough to exercise the shaping without
    /// shipping a corpus file into the test.
    fn entry(name: &str, id: u32, parent_id: u32, rect: [u32; 4], is_root: bool) -> Vec<u8> {
        let mut buffer = vec![0u8; ENTRY_SIZE];
        buffer[..name.len()].copy_from_slice(name.as_bytes());
        buffer[OFF_ID..OFF_ID + 4].copy_from_slice(&id.to_le_bytes());
        buffer[OFF_PARENT..OFF_PARENT + 4].copy_from_slice(&parent_id.to_le_bytes());
        for (index, value) in rect.iter().enumerate() {
            let at = OFF_RECT_X + index * 4;
            buffer[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        if is_root {
            buffer[OFF_IS_ROOT..OFF_IS_ROOT + 4].copy_from_slice(&1u32.to_le_bytes());
        }
        buffer
    }

    fn file(entries: Vec<Vec<u8>>, declared: u32) -> Vec<u8> {
        let mut bytes = declared.to_le_bytes().to_vec();
        for entry in entries {
            bytes.extend_from_slice(&entry);
        }
        bytes
    }

    /// The offsets this test file is built on are the ones a hand read uses
    /// (`id` at +0x344, `parent` at +0x348); if the layout ever moves, this
    /// fails rather than quietly reading zeros.
    #[test]
    fn a_synthetic_file_reads_back_its_ids_parents_and_rects() {
        let bytes = file(
            vec![
                entry("CNIFWnd", 65, 0, [100, 200, 300, 400], true),
                entry("CNIFButton", 66, 65, [120, 230, 40, 20], false),
            ],
            2,
        );
        assert_eq!(OFF_ID, 0x344);
        assert_eq!(OFF_PARENT, 0x348);

        let descriptor = JMXV2DT::from_bytes(&bytes);
        let rows = rows(&descriptor);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "CNIFWnd");
        assert!(rows[0].is_root);
        assert_eq!(rows[1].id, 66);
        assert_eq!(rows[1].parent_id, 65);
        assert_eq!(rows[1].abs, [120.0, 230.0, 40.0, 20.0]);
        // Local layout is child minus root, not child minus parent.
        assert_eq!(rows[1].local, [20.0, 30.0, 40.0, 20.0]);
        assert_eq!(missing_entries(&descriptor), 0);
        assert!(duplicate_ids(&descriptor).is_empty());
    }

    /// The trap a hand read found: two entries carrying one id. Every lookup
    /// by id answers with the first, so the second is invisible unless
    /// something counts.
    #[test]
    fn a_duplicated_id_is_reported_with_its_count() {
        let bytes = file(
            vec![
                entry("CNIFWnd", 28, 0, [0, 0, 10, 10], true),
                entry("CNIFFirst", 28, 28, [1, 1, 2, 2], false),
                entry("CNIFSecond", 28, 28, [3, 3, 2, 2], false),
                entry("CNIFOther", 29, 28, [4, 4, 2, 2], false),
            ],
            4,
        );
        let descriptor = JMXV2DT::from_bytes(&bytes);
        assert_eq!(duplicate_ids(&descriptor), vec![(28, 3)]);
        // The lookup really does hide the other two, which is why counting is
        // not optional.
        assert_eq!(descriptor.entry(28).map(|e| e.name()), Some("CNIFWnd"));
        let payload = as_json(&descriptor, Path::new("x.2dt"));
        assert_eq!(payload["duplicate_ids"][0]["id"], json!(28));
        assert_eq!(payload["duplicate_ids"][0]["count"], json!(3));
    }

    /// File order is its own order; the hand read that found `66, 67, 68, 65`
    /// is the reason this is reported instead of assumed.
    #[test]
    fn a_file_order_that_is_not_id_order_is_reported() {
        let ascending = file(
            vec![
                entry("A", 1, 0, [0, 0, 1, 1], true),
                entry("B", 2, 1, [0, 0, 1, 1], false),
            ],
            2,
        );
        assert!(!file_order_differs_from_id_order(&JMXV2DT::from_bytes(
            &ascending
        )));
        let shuffled = file(
            vec![
                entry("A", 66, 0, [0, 0, 1, 1], true),
                entry("B", 67, 66, [0, 0, 1, 1], false),
                entry("C", 68, 66, [0, 0, 1, 1], false),
                entry("D", 65, 66, [0, 0, 1, 1], false),
            ],
            4,
        );
        assert!(file_order_differs_from_id_order(&JMXV2DT::from_bytes(
            &shuffled
        )));
    }

    /// A short tail is a truncated file, and the difference between declared
    /// and read is the signal. Reporting 0 entries as "the file is empty"
    /// would be the wrong answer.
    #[test]
    fn a_truncated_file_reports_how_many_entries_are_missing() {
        let mut bytes = file(vec![entry("A", 1, 0, [0, 0, 1, 1], true)], 3);
        bytes.extend_from_slice(&vec![0u8; 500]); // half of a second entry
        let descriptor = JMXV2DT::from_bytes(&bytes);
        assert_eq!(descriptor.entries().len(), 1);
        assert_eq!(descriptor.declared_entry_count(), 3);
        assert_eq!(missing_entries(&descriptor), 2);
        let payload = as_json(&descriptor, Path::new("x.2dt"));
        assert_eq!(payload["read_entries"], json!(1));
        assert_eq!(payload["missing_entries"], json!(2));
    }

    /// Without a root there is nothing to subtract, so local equals absolute —
    /// stated here so a reader of the table is not told a local rect that is
    /// really an absolute one by accident.
    #[test]
    fn without_a_root_the_local_rect_equals_the_absolute_one() {
        let bytes = file(vec![entry("A", 1, 0, [50, 60, 10, 20], false)], 1);
        let descriptor = JMXV2DT::from_bytes(&bytes);
        assert!(descriptor.root().is_none());
        let rows = rows(&descriptor);
        assert_eq!(rows[0].local, rows[0].abs);
    }
}
