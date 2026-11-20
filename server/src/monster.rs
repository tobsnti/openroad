//! Monsters stand where the user's data puts them: one per monster row of
//! `npcpos.txt` (characterdata type ids 1/2/1), placed when the server starts.
//! One row is one monster, a deliberate simplification (decision 0019): the
//! file holds a row's model and position only, while the nest's size, radius
//! and respawn time are in the original's server data, which we do not have
//! [U].

use bytes::{BufMut, BytesMut};
use packets::agent::character_data::{EntityMovement, EntityState};
use packets::agent::entity_spawn::{Interaction, SpawnKind, SpawnedEntity};
use packets::agent::prelude::SelectEntityResponse;

use crate::movement::in_region;
use crate::refdata::MonsterRef;
use crate::sight;
use crate::store::Position;
use crate::world::Walk;

#[derive(Clone, Debug, PartialEq)]
pub struct Monster {
    pub ref_id: u32,
    pub position: Position,
    pub hp: u32,
    pub walk: Option<Walk>,
}

/// The talk option every monster offers in its record and its selection, as
/// the vSRO server sends it [V capture, peer vSRO: 4323 of 4323 monster
/// records, 73 of 73 selections]. Its meaning and source are open [U].
const OPTIONS: [u8; 1] = [0x05];

/// The monster as its record in a group spawn shows it, for the spawn writer,
/// as the vSRO server sends one [V capture, peer vSRO: 4323 monster records]:
/// alive, the walk and run speeds and the rarity of its characterdata row (all
/// records but the champions, which the server makes from normal ones at run
/// time), Hwan speed 100, no buffs, no name, and interaction flag 2 with its
/// option. Standing, it faces its heading; `npcpos.txt` has none, so it is 0
/// [U]. Walking, it carries its destination with movement type 0, as 3405 of
/// the 3443 walking records do.
pub(crate) fn entity(uid: u32, monster: &Monster, data: MonsterRef) -> SpawnedEntity {
    let movement = match monster.walk.map(|walk| in_region(walk.to)) {
        Some((region, to)) => EntityMovement {
            has_destination: true,
            move_type: 0,
            dest_region: Some(region),
            dest_x: Some(to.x as i32),
            dest_y: Some(to.y as i32),
            dest_z: Some(to.z as i32),
            ..Default::default()
        },
        None => sight::standing(monster.position.heading),
    };
    SpawnedEntity {
        ref_id: monster.ref_id,
        unique_id: uid,
        position: monster.position.into(),
        name: None,
        guild: None,
        movement: Some(movement),
        state: Some(EntityState {
            life_state: 1,
            unk: 0,
            motion_state: 0,
            body_state: 0,
            walk_speed: data.walk_speed,
            run_speed: data.run_speed,
            hwan_speed: 100.0,
            buffs: Vec::new(),
        }),
        spawn_rarity: Some(data.rarity),
        talk_options: OPTIONS.to_vec(),
        interaction: Some(Interaction {
            flags: 2,
            ..Interaction::default()
        }),
        kind: SpawnKind::Monster { extra: None },
    }
}

/// The answer to selecting the monster (0x7045), as the vSRO server sends it
/// [V capture, peer vSRO: 73 of 73]: the vital mask 1 and the monster's HP,
/// then its option. A monster at full HP showed its characterdata maximum in
/// every capture (66 of 66, e.g. MOB_CH_MANGNYANG 54, MOB_CH_YEOHA 249).
pub(crate) fn selection(uid: u32, monster: &Monster) -> SelectEntityResponse {
    let mut tail = BytesMut::new();
    tail.put_u8(1);
    tail.put_u32_le(monster.hp);
    tail.put_u8(u8::try_from(OPTIONS.len()).expect("one option"));
    tail.put_slice(&OPTIONS);
    SelectEntityResponse {
        result: 1,
        unique_id: uid,
        tail: tail.freeze(),
    }
}
