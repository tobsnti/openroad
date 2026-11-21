//! Items on the ground. For now that is gold, which a normal monster drops
//! when it dies (decision 0021 E2). A pile lies there until its DecayTime is
//! up, and then it goes (0x3016).

use std::f32::consts::TAU;
use std::ops::RangeInclusive;

use packets::agent::entity_spawn::{SpawnKind, SpawnedEntity};

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

/// How many piles one kill drops.
fn piles(chance: &mut fastrand::Rng) -> usize {
    (0..THROWS).filter(|_| chance.f32() < PILE_CHANCE).count()
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
    for _ in 0..piles(world.chance()) {
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
            count[piles(&mut chance)] += 1;
        }
        let near = |n: u32, share: u32| n.abs_diff(share * 100) <= 100;
        assert!(
            near(count[0], 25) && near(count[1], 50) && near(count[2], 25),
            "{count:?}"
        );
    }
}
