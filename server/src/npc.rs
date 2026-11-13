//! NPCs stand where the user's data puts them: the server places one per NPC
//! row of `npcpos.txt` when it starts. The vSRO server does the same; every NPC
//! measured in its captures stands at its `npcpos.txt` row
//! [V capture, peer vSRO: 0x3019, refs 2003-2014 in region 25000].
//!
//! The file places more than NPCs: of its 12,367 rows in the user's data,
//! 12,110 are monsters and 12 name models that are not in the client's data
//! (the user's characterdata). The original client reads a monster's record
//! with more bytes [V original handler], and what is not in the client's data
//! is not sent to it. So only NPC models (`RefData::is_npc`) are placed; the
//! other rows are counted and skipped, and monsters come with their own
//! system.

use bytes::{BufMut, Bytes, BytesMut};
use packets::agent::character_data::{EntityMovement, EntityState, SpawnPosition};
use sro_macro::Serialize;

use crate::store::Position;

#[derive(Clone, Debug, PartialEq)]
pub struct Npc {
    pub ref_id: u32,
    pub position: Position,
}

/// What a click on an NPC offers, as the vSRO server sends it in the NPC's
/// spawn and in its selection (0xB045) [V capture, peer vSRO]. Where the
/// original takes it from is open [U]: the shop groups of `refshopgroup.txt`
/// fit option 02 for five of these but also name the warehouse, which has
/// none. An NPC without a row offers nothing.
const OPTIONS: [(u32, &[u8]); 10] = [
    (2003, &[0x01, 0x02, 0x04, 0x20]), // NPC_CH_SMITH
    (2004, &[0x01, 0x02, 0x04]),       // NPC_CH_ARMOR
    (2005, &[0x01, 0x02]),             // NPC_CH_POTION
    (2008, &[0x01, 0x02]),             // NPC_CH_ACCESSORY
    (2009, &[0x01, 0x02, 0x0B]),       // NPC_CH_HORSE
    (2013, &[0x01, 0x03]),             // NPC_CH_WAREHOUSE_M
    (2014, &[0x01, 0x03]),             // NPC_CH_WAREHOUSE_W
    (2021, &[0x01, 0x02, 0x0F]),       // NPC_CH_GENARAL_SP, selection only
    (2037, &[0x08]),                   // NPC_CH_SOLDIER_SO1, selection only
    (3861, &[0x02]),                   // NPC_CH_EVENT_KISAENG1, selection only
];

pub(crate) fn options(ref_id: u32) -> &'static [u8] {
    OPTIONS
        .iter()
        .find(|(id, _)| *id == ref_id)
        .map_or(&[], |(_, options)| options)
}

/// The NPC's record in a group spawn, in the order the original client reads
/// it [V original handler]. A standing NPC as the vSRO server sends it
/// [V capture, peer vSRO]: alive, speeds 0, 0 and 100, no buffs, and when it
/// offers something, interaction flag 2 with its options. `npcpos.txt` has no
/// heading; it is 0 [U].
pub(crate) fn record(uid: u32, npc: &Npc) -> Bytes {
    let at = npc.position;
    let mut b = BytesMut::new();
    b.put_u32_le(npc.ref_id);
    b.put_u32_le(uid);
    let position = SpawnPosition {
        region: at.region,
        x: at.x,
        y: at.y,
        z: at.z,
        heading: at.heading,
    };
    position.serialize_to(&mut b);
    let standing = EntityMovement {
        has_destination: false,
        move_type: 1,
        source: Some(0),
        angle: Some(at.heading),
        ..Default::default()
    };
    standing
        .write_with_region(&mut b, at.region)
        .expect("a standing block names its source and angle");
    let state = EntityState {
        life_state: 1,
        unk: 0,
        motion_state: 0,
        body_state: 0,
        walk_speed: 0.0,
        run_speed: 0.0,
        hwan_speed: 100.0,
        buffs: Vec::new(),
    };
    state.serialize_to(&mut b);
    match options(npc.ref_id) {
        [] => b.put_u8(0),
        options => {
            b.put_u8(2);
            b.put_u8(u8::try_from(options.len()).expect("a handful of options"));
            b.put_slice(options);
        }
    }
    b.freeze()
}
