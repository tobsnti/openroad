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

use bytes::Bytes;
use packets::agent::character_data::{EntityMovement, EntityState};
use packets::agent::entity_spawn::{PlayerBytes, SingleTail, SpawnKind, SpawnedEntity};
use packets::agent::prelude::SingleEntitySpawn;
use packets::agent::spawn_writer::{write_group_spawn, write_single_spawn};
use packets::region::{is_dungeon, region_id, sector};
use packets::Packet;

use crate::movement::in_region;
use crate::world::{Phase, Player, SessionId, World};
use crate::{monster, npc};

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
        .map(|(other, p)| (other, p.character.position.region))
        .collect();
    for (other, region) in others {
        let near = in_sight(me.character.position.region, region);
        if near == world.knows(me.session, other) {
            continue;
        }
        let Some(player) = world.player(other).cloned() else {
            continue;
        };
        if near {
            let (theirs, mine) = (spawn(world, other, &player), spawn(world, uid, &me));
            world.show(me.session, other, theirs);
            world.show(player.session, uid, mine);
        } else {
            world.hide(me.session, other);
            world.hide(player.session, uid);
        }
    }
    // NPCs and monsters are only seen, and come and go in groups, as the vSRO
    // server sends them [V capture, peer vSRO: every NPC and monster record
    // in 0x3019].
    let near: BTreeSet<u32> = around(me.character.position.region)
        .into_iter()
        .flat_map(|region| world.placed_in(region).to_vec())
        .collect();
    let gone: Vec<u32> = world
        .shown_placed(me.session)
        .into_iter()
        .filter(|uid| !near.contains(uid))
        .collect();
    world.hide_group(me.session, gone);
    let new: Vec<(u32, Bytes)> = near
        .into_iter()
        .filter(|uid| !world.knows(me.session, *uid))
        .map(|uid| (uid, placed_record(world, uid)))
        .collect();
    world.show_group(me.session, new);
}

/// Monster `uid` walked from region `from` into another: the clients in the
/// game that now see it are shown it, and those that no longer do have it
/// taken back, each as a group of one, as for NPCs.
pub(crate) fn monster_crossed(world: &mut World, uid: u32, from: u16) {
    let Some(region) = world.monster(uid).map(|m| m.position.region) else {
        return;
    };
    world.move_placed(uid, from, region);
    let in_game = |s: SessionId| matches!(world.phase(s), Some(Phase::Game { .. }));
    let players: Vec<(SessionId, u16)> = world
        .players()
        .filter(|(_, p)| in_game(p.session))
        .map(|(_, p)| (p.session, p.character.position.region))
        .collect();
    for (session, at) in players {
        let near = in_sight(at, region);
        if near && !world.knows(session, uid) {
            let record = placed_record(world, uid);
            world.show_group(session, vec![(uid, record)]);
        } else if !near && world.knows(session, uid) {
            world.hide_group(session, vec![uid]);
        }
    }
}

/// The group spawn record of NPC or monster `uid`.
fn placed_record(world: &World, uid: u32) -> Bytes {
    let entity = match (world.npc(uid), world.monster(uid)) {
        (Some(npc), _) => npc::entity(uid, npc),
        (None, Some(m)) => {
            let data = world.refdata().monster(m.ref_id);
            monster::entity(
                uid,
                m,
                data.expect("a placed monster has a monster's model"),
            )
        }
        (None, None) => unreachable!("only NPCs and monsters are placed"),
    };
    let record = write_group_spawn(&[entity], &world.refdata().spawn_tables());
    record
        .expect("a placed NPC or monster has its model's record (World::new)")
        .into()
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

/// The movement block of a character standing, facing `heading`, as the vSRO
/// server sends it [V capture, peer vSRO].
pub(crate) fn standing(heading: u16) -> EntityMovement {
    EntityMovement {
        has_destination: false,
        move_type: 1,
        source: Some(0),
        angle: Some(heading),
        ..Default::default()
    }
}

/// The single spawn (0x3015) of a character, with the items it wears as
/// [`crate::store::Character::shown_equipment`] lists them. What the server
/// holds no state for yet is empty (mask, mount, guild, stall) or carries what
/// the vSRO server sends for an ordinary player [V capture, peer vSRO: 0x3015
/// player records]: alive, Hwan speed 100, 5 avatar slots; before the
/// equipment the model's scale, three bytes 0 and the inventory size; behind
/// the name 0, 1 and 0; PK flag 0xFF; last byte 1.
fn spawn(world: &World, uid: u32, p: &Player) -> Packet {
    let c = &p.character;
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
        None => standing(c.position.heading),
    };
    let entity = SpawnedEntity {
        ref_id: c.ref_id,
        unique_id: uid,
        position: c.position.into(),
        name: Some(c.name.clone()),
        guild: None,
        movement: Some(movement),
        state: Some(EntityState {
            life_state: 1,
            unk: 0,
            motion_state: 0,
            body_state: 0,
            walk_speed: p.walk_speed,
            run_speed: p.run_speed,
            hwan_speed: 100.0,
            buffs: Vec::new(),
        }),
        spawn_rarity: None,
        talk_options: Vec::new(),
        interaction: None,
        kind: SpawnKind::Player {
            equipment: c.shown_equipment(),
            avatar: c.shown_avatar(),
            riding: 0,
            riding_uid: None,
            mask: None,
            interact_mode: 0,
            stall: None,
            guild_after_name: 0,
            guild_tail: [0; 14],
            bytes: PlayerBytes {
                head: [c.scale, 0, 0, 0, c.inventory_size],
                between_lists: 5,
                after_name: [0, 1, 0],
                after_riding: 0,
                around_mode: [0, 0],
                tail: [0, 0xFF],
            },
        },
    };
    let raw = write_single_spawn(
        &entity,
        &SingleTail::Figure(1),
        &world.refdata().spawn_tables(),
    )
    .expect("a character in the world has a player's model (join)");
    Packet::SingleEntitySpawn(SingleEntitySpawn { raw: raw.into() })
}
