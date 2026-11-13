//! Who sees whom. A character in the game sees the others in its region. The
//! original takes a leaving character back from the observers of its block
//! and of the neighbouring blocks [V original server]; how large a block is
//! is open [U], and neighbours need the region arithmetic the server does not
//! share with the client yet.
//!
//! The original client asserts on the spawn of an id it knows and on any
//! other frame about an id it does not [V original handler], so the world
//! keeps, per session, the ids its client was shown ([`World::show`]), and
//! takes them back with 0x3016 when the character leaves.

use bytes::{BufMut, BytesMut};
use packets::agent::character_data::{EntityMovement, EntityState, SpawnPosition};
use packets::agent::prelude::SingleEntitySpawn;
use packets::Packet;
use sro_macro::Serialize;

use crate::world::{Player, World};

/// The character `uid` has entered the game: it and the characters in its
/// region are shown to each other.
pub(crate) fn enter(world: &mut World, uid: u32) {
    let Some(me) = world.player(uid).cloned() else {
        return;
    };
    let near: Vec<(u32, Player)> = world
        .players()
        .filter(|(other, p)| *other != uid && p.position.region == me.position.region)
        .map(|(other, p)| (other, p.clone()))
        .collect();
    for (other, player) in near {
        world.show(me.session, other, spawn(other, &player));
        world.show(player.session, uid, spawn(uid, &me));
    }
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
    let movement = match p.walk {
        Some(walk) => EntityMovement {
            has_destination: true,
            move_type: 1,
            dest_region: Some(at.region),
            dest_x: Some(walk.to.x as i32),
            dest_y: Some(walk.to.y as i32),
            dest_z: Some(walk.to.z as i32),
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
