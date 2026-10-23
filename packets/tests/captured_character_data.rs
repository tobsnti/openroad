//! Replay every captured CHARACTER_DATA (0x3013) body through the staged parser.
//!
//! IDEA. Two live bodies used to sit in `character_data.rs` as hex constants to
//! prove the item resync. A `0x3013` is the whole character state — name,
//! position, unique id, inventory — exactly the class of data `packet_dump/` is
//! gitignored for, so the width proofs moved to synthetic bodies in the unit
//! tests and the real bytes are replayed from the user's own tree, the way
//! `captured_party_frames.rs` does.
//!
//! What is provable without the user's itemdata: the class map lives there, not
//! in the dump, so no ref resolves in a test run and every captured body takes
//! the unroutable path (measured on 122 bodies: 17–107 items each, none empty).
//! That is the half worth guarding — #425, a record whose width we cannot
//! derive must not cost us the sections behind it, up to and including the job
//! and extras blocks at the very end of the record. The anchor needs the unique
//! id, which a hintless scan cannot infer from these bodies (28–51 plausible
//! positions each), so it comes from the `CelestialPosition` (0x3020) bodies
//! captured alongside — a different opcode, never the body under test.
//!
//! Names are asserted non-empty and never printed.

use std::path::Path;

use packets::agent::character_data::{
    parse_character_data, parse_character_info, ItemClass, ItemClassResolver,
};

mod dump;
use dump::{bodies, dump_root, logs};

/// Routes nothing — the state of every test run, which has no itemdata.
struct Unroutable;

impl ItemClassResolver for Unroutable {
    fn item_class(&self, _ref_id: u32) -> ItemClass {
        ItemClass::Unknown
    }
}

/// The unique ids the captured `CelestialPosition` frames carried (`u32` at
/// offset 0 of an 8-byte body).
fn celestial_ids(root: &Path) -> Vec<u32> {
    logs(root, "0x3020.log")
        .iter()
        .flat_map(|log| bodies(log))
        .filter_map(|body| {
            let head: [u8; 4] = body.get(..4)?.try_into().ok()?;
            Some(u32::from_le_bytes(head))
        })
        .collect()
}

#[test]
fn captured_character_bodies_keep_the_tail_behind_an_unroutable_record() {
    let Some(root) = dump_root() else {
        eprintln!(
            "skipped: set OPENROAD_PACKET_DUMP_DIR to a packet_dump tree to replay \
             captured character data"
        );
        return;
    };

    let ids = celestial_ids(&root);
    let mut replayed = 0usize;
    let mut unpaired = 0usize;

    for log in logs(&root, "0x3013.log") {
        for (index, body) in bodies(&log).into_iter().enumerate() {
            let at = format!("{}:{index}", log.display());
            let Some(id) = ids
                .iter()
                .copied()
                .find(|&id| parse_character_data(&body, Some(id)).is_some())
            else {
                unpaired += 1;
                continue;
            };

            let info = parse_character_info(&body, Some(id), &Unroutable);

            // The fixed-offset head reads whatever the items do.
            let stats = info.stats.unwrap_or_else(|| panic!("{at}: stats lost"));
            assert!(stats.hp > 0, "{at}: hp {}", stats.hp);
            assert!(
                stats.max_level >= stats.level,
                "{at}: level {} above max {}",
                stats.level,
                stats.max_level
            );

            // The record it cannot route is reported, never guessed.
            let stop = info
                .item_stop
                .unwrap_or_else(|| panic!("{at}: no stop recorded"));
            assert_eq!(
                info.recovered_body, None,
                "{at}: a width was claimed without itemdata"
            );
            assert_eq!(
                info.inventory.as_deref(),
                Some(&[][..]),
                "{at}: an unroutable record was decoded"
            );
            assert!(info.inventory_size.is_some(), "{at}: section size lost");
            let offender = info
                .item_trace
                .last()
                .unwrap_or_else(|| panic!("{at}: offender not traced"));
            assert_eq!(
                offender.start_offset, stop.offset,
                "{at}: trace off the stop"
            );
            assert_eq!(offender.ref_id, stop.ref_id, "{at}: trace off the stop");
            assert_eq!(offender.class, ItemClass::Unknown, "{at}: routed after all");

            // #425: the sections behind the offender still arrive.
            let spawn = info.spawn.unwrap_or_else(|| panic!("{at}: tail lost"));
            assert_eq!(spawn.unique_id, id, "{at}: tail id");
            assert_ne!(spawn.region, 0, "{at}: region 0");
            assert!(info.state.is_some(), "{at}: state lost");
            assert!(
                info.name.as_deref().is_some_and(|name| !name.is_empty()),
                "{at}: name lost"
            );
            // Behind the name sit the job block and the account/session extras
            // — the wire source for the hotkey bar, the auto-potion settings,
            // the GM flag and the ridden transport. They need no item width,
            // only the anchor, and are accepted solely when they land on the
            // blob's last byte: measured 243/243 on this tree, carrying the
            // same values the forward pass reads once the widths are known
            // (local mirror `fixes/messbank_cdtail.py`, 2026-10-23).
            assert!(info.job.is_some(), "{at}: job block lost");
            assert!(info.extras.is_some(), "{at}: extras block lost");

            replayed += 1;
        }
    }

    eprintln!("replayed {replayed}x 0x3013, {unpaired} without a CelestialPosition id");
    assert!(
        replayed > 0,
        "OPENROAD_PACKET_DUMP_DIR is set but holds no character body with a \
         CelestialPosition id: {}",
        root.display()
    );
}
