//! Monsters stand where the user's data puts them: one per monster row of
//! `npcpos.txt` (characterdata type ids 1/2/1), placed when the server starts.
//! One row is one monster, a deliberate simplification (decision 0019): the
//! file holds a row's model and position only, while the nest's size, radius
//! and respawn time are in the original's server data, which we do not have
//! [U].

use std::collections::BTreeSet;
use std::ops::RangeInclusive;

use bytes::{BufMut, BytesMut};
use packets::agent::character_data::{EntityMovement, EntityState};
use packets::agent::entity_spawn::{Interaction, SpawnKind, SpawnedEntity};
use packets::agent::prelude::SelectEntityResponse;
use packets::region::is_dungeon;

use crate::movement::{in_region, in_world};
use crate::refdata::MonsterRef;
use crate::sight;
use crate::store::Position;
use crate::world::{Phase, Point, Walk, World};

#[derive(Clone, Debug, PartialEq)]
pub struct Monster {
    pub ref_id: u32,
    pub position: Position,
    pub hp: u32,
    pub walk: Option<Walk>,
    /// Where it was placed; it wanders around it.
    pub home: Position,
    /// The step from which it may start its next wander step.
    pub rest_until: u64,
}

/// A calm monster's wander step and the time from one step's start to the
/// next, as the vSRO server moves them [V capture, peer vSRO: 1947 calm moves
/// of monsters near their `npcpos.txt` row, the middle half of each; studio
/// note 2026-11-20-monster-wander-messung]: 63 to 111 units, every 9.2 to
/// 16.3 s.
const STEP_LENGTH: RangeInclusive<f32> = 63.0..=111.0;
const REST_STEPS: RangeInclusive<u64> = 276..=489;

/// How far a monster strays from its row. The original keeps a nest's
/// monsters within a radius from its server data, which we do not have [U]. A
/// quarter of the captured calm moves end within 252 units of the nearest row
/// of their model (same note); one monster per row stays within that.
const HOME_RADIUS: f32 = 250.0;

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

/// A dead monster's body lies 5 s, then it is taken away (0x3016), as on the
/// vSRO server [V capture, peer vSRO: 4.89 to 5.67 s after the killing swing,
/// median 5.008, for 173 of 173 kills].
const BODY_STEPS: u64 = 150;

/// The monster of the row comes back 14 to 17 s after the death, under a new
/// uid, standing at the row (decision 0020, E3) [S: the middle half of 20
/// natural returns in the captures, median 16 s; 38 that seemed to be returns
/// were GM spawns].
const RETURN_STEPS: RangeInclusive<u64> = 420..=510;

/// Monster `uid` died this step: its body is taken away, and its row's
/// monster comes back, each at its time.
pub(crate) fn fell(world: &mut World, uid: u32) {
    let now = world.steps();
    let Some(&Monster { ref_id, home, .. }) = world.monster(uid) else {
        return;
    };
    let back = now + world.chance().u64(RETURN_STEPS);
    world.bodies.push_back((now + BODY_STEPS, uid));
    world.returning.push((back, ref_id, home));
}

/// Each step: the bodies whose time is up are taken away from the clients that
/// know them, and the monsters whose time is up come back.
pub(crate) fn tend(world: &mut World) {
    let now = world.steps();
    while let Some(&(_, uid)) = world.bodies.front().filter(|(due, _)| *due <= now) {
        world.bodies.pop_front();
        for session in world.observers(uid) {
            world.hide(session, uid);
        }
        world.remove_monster(uid);
    }
    let (due, later) = world.returning.iter().partition(|(at, ..)| *at <= now);
    world.returning = later;
    for (_, ref_id, home) in due {
        if let Some(uid) = world.place_monster(ref_id, home) {
            sight::monster_returned(world, uid);
        }
    }
}

/// The AI timer (A10): every monster within a region of what a character in
/// the game sees, standing and rested long enough, starts a wander step. A
/// monster strays at most [`HOME_RADIUS`] from its row, less than a region,
/// so one whose row is in sight walks back in after it walked out, instead of
/// standing just beyond. Monsters further away do not think, so the cost
/// follows the players, not the 12,000 monsters of the user's data; a monster
/// in a dungeon or without a walk speed stays where it is.
pub(crate) fn think(world: &mut World) {
    let now = world.steps();
    let regions: BTreeSet<u16> = world
        .players()
        .filter(|(_, p)| matches!(world.phase(p.session), Some(Phase::Game { .. })))
        .flat_map(|(_, p)| sight::around(p.character.position.region))
        .flat_map(sight::around)
        .collect();
    let near: Vec<u32> = regions
        .into_iter()
        .flat_map(|region| world.placed_in(region).to_vec())
        .collect();
    for uid in near {
        let Some(monster) = world.monster(uid) else {
            continue;
        };
        let speed = world
            .refdata()
            .monster(monster.ref_id)
            .map_or(0.0, |d| d.walk_speed);
        let idle = monster.walk.is_none() && monster.rest_until <= now;
        let still = monster.hp == 0 || world.attacked(uid);
        if !idle || still || speed <= 0.0 || is_dungeon(monster.position.region) {
            continue;
        }
        let to = wander_step(monster.position, monster.home, world.chance());
        world.walk_monster(uid, to);
        let rest = rest(world.chance());
        if let Some(monster) = world.monster_mut(uid) {
            monster.rest_until = now + rest;
        }
    }
}

/// A time of [`REST_STEPS`] until the next wander step, for a monster just
/// placed or starting a step.
pub(crate) fn rest(chance: &mut fastrand::Rng) -> u64 {
    chance.u64(REST_STEPS)
}

/// Where a monster standing `at` takes its next wander step, in world
/// coordinates: [`STEP_LENGTH`] in a random direction, or toward its `home`
/// when that step would leave [`HOME_RADIUS`].
fn wander_step(at: Position, home: Position, chance: &mut fastrand::Rng) -> Point {
    let point = |at: Position| {
        in_world(
            at.region,
            Point {
                x: at.x,
                y: at.y,
                z: at.z,
            },
        )
    };
    let (here, home) = (point(at), point(home));
    let length = STEP_LENGTH.start() + chance.f32() * (STEP_LENGTH.end() - STEP_LENGTH.start());
    let angle = chance.f32() * std::f32::consts::TAU;
    let mut to = Point {
        x: here.x + length * angle.cos(),
        y: here.y,
        z: here.z + length * angle.sin(),
    };
    if (to.x - home.x).hypot(to.z - home.z) > HOME_RADIUS {
        let (dx, dz) = (home.x - here.x, home.z - here.z);
        let back = dx.hypot(dz).min(length);
        let share = back / dx.hypot(dz).max(f32::EPSILON);
        to = Point {
            x: here.x + dx * share,
            y: here.y,
            z: here.z + dz * share,
        };
    }
    to
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every step drawn for a monster at the edge of its home radius, 245
    /// units east of its row, ends within 250 units of the row.
    #[test]
    fn a_wander_step_stays_within_the_home_radius() {
        let home = Position {
            region: 25000,
            x: 900.0,
            y: 0.0,
            z: 1400.0,
            heading: 0,
        };
        let edge = Position {
            x: home.x + 245.0,
            ..home
        };
        let row = in_world(
            home.region,
            Point {
                x: 900.0,
                y: 0.0,
                z: 1400.0,
            },
        );
        let mut chance = fastrand::Rng::with_seed(19);
        for _ in 0..100 {
            let to = wander_step(edge, home, &mut chance);
            assert!((to.x - row.x).hypot(to.z - row.z) <= 250.0, "{to:?}");
        }
    }
}
