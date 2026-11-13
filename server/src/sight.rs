//! Who sees whom. The original takes a leaving character back from the
//! observers of its block and of the neighbouring blocks [V original server];
//! how large a block is is open [U]. This server takes a region for a block,
//! so a character sees the others and the NPCs in its region and the eight
//! around it; in a dungeon, in its region.
//!
//! The original client asserts on the spawn of an id it knows and on any
//! other frame about an id it does not [V original handler], so the world
//! keeps, per session, the ids its client was shown ([`World::show`]), and
//! takes them back with 0x3016 when the character leaves its sight.

use std::collections::BTreeSet;

use bytes::{BufMut, Bytes, BytesMut};
use packets::agent::character_data::{EntityMovement, EntityState, SpawnPosition};
use packets::agent::prelude::SingleEntitySpawn;
use packets::region::{is_dungeon, region_id, sector};
use packets::Packet;
use sro_macro::Serialize;

use crate::movement::in_region;
use crate::npc;
use crate::world::{Player, World};

/// Brings who sees the character `uid`, and whom it sees, in line with where
/// everyone is: what came into sight is shown, what left it is taken back,
/// both ways. Run when it enters the game and when it crosses into another
/// region.
pub(crate) fn update(world: &mut World, uid: u32) {
    let Some(me) = world.player(uid).cloned() else {
        return;
    };
    let others: Vec<(u32, u16)> = world
        .players()
        .filter(|(other, _)| *other != uid)
        .map(|(other, p)| (other, p.position.region))
        .collect();
    for (other, region) in others {
        let near = in_sight(me.position.region, region);
        if near == world.knows(me.session, other) {
            continue;
        }
        let Some(player) = world.player(other).cloned() else {
            continue;
        };
        if near {
            world.show(me.session, other, spawn(other, &player));
            world.show(player.session, uid, spawn(uid, &me));
        } else {
            world.hide(me.session, other);
            world.hide(player.session, uid);
        }
    }
    // NPCs only are seen, and come and go in groups, as the vSRO server
    // sends them [V capture, peer vSRO: every NPC record in 0x3019].
    let near: BTreeSet<u32> = around(me.position.region)
        .into_iter()
        .flat_map(|region| world.npcs_in(region).to_vec())
        .collect();
    let gone: Vec<u32> = world
        .shown_npcs(me.session)
        .into_iter()
        .filter(|uid| !near.contains(uid))
        .collect();
    world.hide_group(me.session, gone);
    let new: Vec<(u32, Bytes)> = near
        .into_iter()
        .filter(|uid| !world.knows(me.session, *uid))
        .filter_map(|uid| world.npc(uid).map(|n| (uid, npc::record(uid, n))))
        .collect();
    world.show_group(me.session, new);
}

/// The regions a character in `region` sees: its own and, in the overworld,
/// the eight around it.
fn around(region: u16) -> Vec<u16> {
    if is_dungeon(region) {
        return vec![region];
    }
    let (x, z) = sector(region);
    let mut regions = Vec::with_capacity(9);
    for nz in z.saturating_sub(1)..=(z + 1).min(0x7F) {
        for nx in x.saturating_sub(1)..=x.saturating_add(1) {
            regions.push(region_id(nx, nz));
        }
    }
    regions
}

/// Whether a character in region `a` sees one in region `b`.
fn in_sight(a: u16, b: u16) -> bool {
    around(a).contains(&b)
}

/// The single spawn (0x3015) of a character, in the order the original
/// client reads it [V original handler]. What the server holds no state for
/// yet is empty (gear, mask, mount, guild, stall) or carries what the vSRO
/// server sends for an ordinary player [V capture, peer vSRO: 0x3015 player
/// records]: alive, 5 avatar slots, job level 1, Hwan speed 100, PK flag
/// 0xFF, last byte 1.
fn spawn(uid: u32, p: &Player) -> Packet {
    let at = p.position;
    let mut b = BytesMut::new();
    b.put_u32_le(p.ref_id);
    // Height and build, Hwan level, PVP cape, auto-invest, inventory size.
    b.put_slice(&[p.scale, 0, 0, 0, p.inventory_size]);
    // No gear; an avatar section of 5 slots with none worn; no mask.
    b.put_slice(&[0, 5, 0, 0]);
    b.put_u32_le(uid);
    let position = SpawnPosition {
        region: at.region,
        x: at.x,
        y: at.y,
        z: at.z,
        heading: at.heading,
    };
    position.serialize_to(&mut b);
    let movement = match p.walk.map(|walk| in_region(walk.to)) {
        Some((region, to)) => EntityMovement {
            has_destination: true,
            move_type: 1,
            dest_region: Some(region),
            dest_x: Some(to.x as i32),
            dest_y: Some(to.y as i32),
            dest_z: Some(to.z as i32),
            ..Default::default()
        },
        None => EntityMovement {
            has_destination: false,
            move_type: 1,
            source: Some(0),
            angle: Some(at.heading),
            ..Default::default()
        },
    };
    movement
        .write_with_region(&mut b, at.region)
        .expect("each form names the fields it writes");
    let state = EntityState {
        life_state: 1,
        unk: 0,
        motion_state: 0,
        body_state: 0,
        walk_speed: p.walk_speed,
        run_speed: p.run_speed,
        hwan_speed: 100.0,
        buffs: Vec::new(),
    };
    state.serialize_to(&mut b);
    let name = u16::try_from(p.name.len()).expect("a name fits its u16 length");
    b.put_u16_le(name);
    b.put_slice(p.name.as_bytes());
    // Job type and level, PK state; not riding, not fighting; no scroll,
    // interaction or the byte after it.
    b.put_slice(&[0, 1, 0, 0, 0, 0, 0, 0]);
    // The guild block of a character without one: name, id, nick, crest,
    // union id and crest, two flags.
    b.put_slice(&[0; 2 + 4 + 2 + 4 + 4 + 4 + 1 + 1]);
    // Equipment cooldown; PK flag 0xFF; the last byte of a single spawn.
    b.put_slice(&[0, 0xFF, 1]);
    Packet::SingleEntitySpawn(SingleEntitySpawn { raw: b.freeze() })
}
