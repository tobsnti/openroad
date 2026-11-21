//! Items on the ground. For now that is gold, which a normal monster drops
//! when it dies (decision 0021 E2). A pile lies there until a character picks
//! it up or its DecayTime is up. Any character may pick it up, a deliberate
//! simplification (decision 0021 E4): how long a pile is kept for its owner
//! is decided with the party.

use std::f32::consts::TAU;
use std::ops::RangeInclusive;

use bytes::Bytes;
use packets::agent::entity_spawn::{SpawnKind, SpawnedEntity};
use packets::agent::ingame::{CharacterPointsUpdate, ObjectActionResponse};
use packets::agent::inventory::{
    InventoryOperationResponse, InventoryOperationResult, PlayerPickupAnimation,
};
use packets::Packet;

use crate::combat;
use crate::monster::Monster;
use crate::movement::{in_region, in_world};
use crate::sight;
use crate::store::Position;
use crate::tick::STEPS_PER_SECOND;
use crate::world::{Point, World};

/// An item on the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundItem {
    pub ref_id: u32,
    /// The gold it holds.
    pub amount: u32,
    pub position: Position,
    /// The id of the character it was dropped for (decision 0021 E3).
    pub owner: u32,
    /// The monster that dropped it.
    pub dropper: u32,
    /// The step it goes at.
    pub decays: u64,
}

/// A normal monster drops gold in two throws, each a pile with chance one
/// half (decision 0021 E2) [S]. That follows the vSRO server over 164 kills of
/// the Mangnyang, the Yeoha and the Chakji [V capture, peer vSRO]: no pile in
/// 53 (32 %), one in 68 (41 %), two in 43 (26 %), where two throws at one half
/// give 25, 50 and 25 %. By monster it differed, which this rule does not
/// follow [U]: the Mangnyang (level 1) dropped two piles in none of its 55
/// kills, and the Chakji (level 20) dropped none in 3 of its 42.
const THROWS: usize = 2;
const PILE_CHANCE: f32 = 0.5;

/// A pile lands 10 to 30 units from the monster, in any direction and with any
/// heading, as 2189 of 2248 drops of the vSRO server did (median 19.4 units)
/// [V capture, peer vSRO].
const SCATTER: RangeInclusive<f32> = 10.0..=30.0;

/// How many piles one kill drops. The bench may fix the chance
/// (`BENCH_GOLD_DROP_PERMILLE`, bench contract); then one throw at that chance
/// replaces the two, so that a test kill drops at most one pile (lead,
/// 2026-11-21).
fn piles(chance: &mut fastrand::Rng, fixed: Option<u16>) -> usize {
    match fixed {
        Some(permille) => usize::from(chance.u16(0..1000) < permille),
        None => (0..THROWS).filter(|_| chance.f32() < PILE_CHANCE).count(),
    }
}

/// Monster `uid` died, killed for the character with the id `owner`: a normal
/// monster drops its piles of gold, each holding an amount from
/// `levelgold.txt` at the monster's level, and the clients that see where a
/// pile lands are shown it. A unique drops by another rule on the vSRO server
/// (the Tiger Woman 1 to 8 piles of 2850 to 5910 gold in each of 61 kills);
/// that rule is not built, so only normal monsters drop gold yet.
pub(crate) fn monster_died(world: &mut World, uid: u32, owner: u32) {
    let Some(&Monster {
        ref_id,
        position: at,
        ..
    }) = world.monster(uid)
    else {
        return;
    };
    let data = world.refdata().monster(ref_id);
    let Some(data) = data.filter(|m| m.rarity == 0) else {
        return;
    };
    let Some(gold) = world.refdata().gold_at(data.level) else {
        return;
    };
    let here = in_world(
        at.region,
        Point {
            x: at.x,
            y: at.y,
            z: at.z,
        },
    );
    let fixed = world.gold_drop();
    for _ in 0..piles(world.chance(), fixed) {
        let amount = world.chance().u32(gold.clone());
        let Some((ref_id, decay_ms)) = world.refdata().gold_pile(amount) else {
            continue;
        };
        let chance = world.chance();
        let (angle, distance) = (chance.f32() * TAU, scatter(chance.f32()));
        let spot = Point {
            x: here.x + distance * angle.cos(),
            y: here.y,
            z: here.z + distance * angle.sin(),
        };
        let (region, local) = in_region(spot);
        let item = GroundItem {
            ref_id,
            amount,
            position: Position {
                region,
                x: local.x,
                y: local.y,
                z: local.z,
                heading: world.chance().u16(..),
            },
            owner,
            dropper: uid,
            decays: world.steps() + u64::from(decay_ms) * STEPS_PER_SECOND / 1000,
        };
        let pile = world.place_ground_item(item);
        sight::item_dropped(world, pile);
    }
}

/// The distance of a pile from its monster for a roll in 0..1.
fn scatter(roll: f32) -> f32 {
    SCATTER.start() + roll * (SCATTER.end() - SCATTER.start())
}

/// Each step: the items whose time is up go, and the clients that know them
/// are told (0x3016).
pub(crate) fn tend(world: &mut World) {
    let now = world.steps();
    let due: Vec<u32> = world
        .ground_items()
        .filter(|(_, item)| item.decays <= now)
        .map(|(uid, _)| uid)
        .collect();
    for uid in due {
        for session in world.observers(uid) {
            world.hide(session, uid);
        }
        world.remove_ground_item(uid);
    }
}

/// A character picks an item up where it stands when it is within 10 units
/// of it, and else first walks until it is 1.9 units from it [S, from 54
/// walks of vSRO characters to a pile: each began 10.7 units from it or
/// further, and ended a median 1.87 units from it, 48 of them 0.5 to 2.5].
const PICK_REACH: f32 = 10.0;
const PICK_STOP: f32 = 1.9;

/// The slot 0xB034 names for gold [V capture, peer vSRO: 81 of 81 gold
/// pickups].
const GOLD_SLOT: u8 = 0xFE;

/// Each step: a character that is to pick an item up and does not attack
/// walks to it or picks it up; if the item went meanwhile, the request ends
/// (`02 00`).
pub(crate) fn step(world: &mut World) {
    let pickers: Vec<(u32, u32)> = world
        .players()
        .filter(|(_, p)| p.attack.is_none())
        .filter_map(|(uid, p)| Some((uid, p.pickup?)))
        .collect();
    for (uid, item) in pickers {
        fetch(world, uid, item);
    }
}

fn fetch(world: &mut World, uid: u32, item: u32) {
    let player = world.player(uid).expect("a picker is a player");
    let (session, here) = (player.session, player.character.position);
    if player.walk.is_some() {
        return;
    }
    let Some(&pile) = world.ground_item(item) else {
        world.player_mut(uid).expect("a picker is a player").pickup = None;
        let ended = ObjectActionResponse::Ended { queue_len: 0 };
        world.send(session, Packet::ObjectActionResponse(ended));
        return;
    };
    if combat::distance(here, pile.position) <= PICK_REACH {
        pick_up(world, uid, item, pile);
    } else {
        let (from, to) = (combat::ground(here), combat::ground(pile.position));
        combat::approach(world, uid, from, to, PICK_STOP);
    }
}

/// Character `uid` picks up the gold of pile `item`. As from the vSRO server
/// [V capture, peer vSRO: 73 gold pickups], its client is told the new total
/// (0x304E 1) first; the clients that know the pile see it go (0x3016), those
/// that know the character see it bend down (0x3036, facing the pile), and
/// its client is told the pickup (0xB034 `01 06 FE <amount>`) and the end of
/// the action (`02 00`).
fn pick_up(world: &mut World, uid: u32, item: u32, pile: GroundItem) {
    let player = world.player_mut(uid).expect("a picker is a player");
    player.pickup = None;
    player.character.gold += u64::from(pile.amount);
    let character = &player.character;
    let (session, gold, here) = (player.session, character.gold, character.position);
    let total = CharacterPointsUpdate::Gold {
        amount: gold,
        display: 0,
    };
    world.send(session, Packet::CharacterPointsUpdate(total));
    for session in world.observers(item) {
        world.hide(session, item);
    }
    world.remove_ground_item(item);
    let bend = PlayerPickupAnimation {
        unique_id: uid,
        rotation: facing(here, pile.position),
    };
    let observers = world.observers(uid);
    world.broadcast(&observers, Packet::PlayerPickupAnimation(bend));
    let picked = InventoryOperationResponse {
        result: 1,
        operation: Some(InventoryOperationResult::Pickup {
            slot: GOLD_SLOT,
            tail: Bytes::copy_from_slice(&pile.amount.to_le_bytes()),
        }),
        error: None,
    };
    world.send(session, Packet::InventoryOperationResponse(picked));
    let ended = ObjectActionResponse::Ended { queue_len: 0 };
    world.send(session, Packet::ObjectActionResponse(ended));
}

/// Which way a character at `from` faces `to`, in 256ths of a turn from +x
/// toward +z, as 0x3036 carries it [S: in the vSRO captures the byte follows
/// the direction of the walk to the pile].
fn facing(from: Position, to: Position) -> u8 {
    let (a, b) = (combat::ground(from), combat::ground(to));
    let turns = (b.z - a.z).atan2(b.x - a.x) / TAU;
    ((turns.rem_euclid(1.0) * 256.0).round() as u32 % 256) as u8
}

/// The spawn record of item `uid`: the owner set, rarity 0, as on all 461
/// gold piles of the vSRO server [V capture, peer vSRO].
pub(crate) fn entity(uid: u32, item: &GroundItem) -> SpawnedEntity {
    SpawnedEntity {
        ref_id: item.ref_id,
        unique_id: uid,
        position: item.position.into(),
        name: None,
        guild: None,
        movement: None,
        state: None,
        spawn_rarity: Some(0),
        talk_options: Vec::new(),
        interaction: None,
        kind: SpawnKind::Item {
            amount: Some(item.amount),
            name: None,
            opt_level: None,
            owner: Some(item.owner),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Over 10,000 kills the share of none, one and two piles is a quarter, a
    /// half and a quarter, within one percent.
    #[test]
    fn two_throws_at_one_half() {
        let mut chance = fastrand::Rng::with_seed(19);
        let mut count = [0u32; 3];
        for _ in 0..10_000 {
            count[piles(&mut chance, None)] += 1;
        }
        let near = |n: u32, share: u32| n.abs_diff(share * 100) <= 100;
        assert!(
            near(count[0], 25) && near(count[1], 50) && near(count[2], 25),
            "{count:?}"
        );
    }

    /// With the bench's chance fixed, one throw: at 1000 a pile every time, at
    /// 0 never, at 250 in a quarter of 10,000 kills, within one percent; never
    /// two.
    #[test]
    fn one_throw_at_a_fixed_chance() {
        let mut chance = fastrand::Rng::with_seed(19);
        let mut count = |permille| {
            let mut count = [0u32; 3];
            for _ in 0..10_000 {
                count[piles(&mut chance, Some(permille))] += 1;
            }
            count
        };
        assert_eq!((count(1000), count(0)), ([0, 10_000, 0], [10_000, 0, 0]));
        let quarter = count(250);
        assert!(
            quarter[1].abs_diff(2500) <= 100 && quarter[2] == 0,
            "{quarter:?}"
        );
    }
}
