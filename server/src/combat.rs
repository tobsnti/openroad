//! Attacking a monster (0x7074). As on the vSRO server, the server runs the
//! swings: one request starts an attack, and the server swings at the target
//! until it dies or the client cancels [V capture, peer vSRO: 47 swings for one
//! request]. A swing lasts its basic attack's Action_ActionDuration and three
//! steps more, and the next one starts when it ends [V capture, peer vSRO: the
//! median time between two swings is that duration plus 94 to 118 ms, for the
//! fist, the sword and the spear]. A request while a swing runs waits behind
//! it: 0xB074 `01 02`, then `02 01` when the swing ends [V capture, peer vSRO:
//! `02 01` 1.07 to 1.57 s after the last sword swing, 65 times].
//!
//! The attacker first walks into reach. The monster stands while it is
//! attacked and does not hit back yet (decision 0020, E4); what a hit deals is
//! [`RefData::basic_attack`]'s.
//!
//! [`RefData::basic_attack`]: crate::refdata::RefData::basic_attack

use packets::agent::ingame::{
    ActionCommand, ActionKind, ActionTarget, DamageContent, DamageValue, EntityBarsUpdate,
    EntityStateUpdate, HitEffect, MovementResponse, ObjectActionRequest, ObjectActionResponse,
    ObjectActionUpdate, PerEntityDamage, SkillPartDamage, BARS_FLAG_BAD_STATUS, BARS_FLAG_HP,
    BARS_SOURCE_DAMAGE, LIFE_STATE_DEAD, STATE_KIND_LIFE,
};
use packets::Packet;

use crate::monster;
use crate::movement::{in_region, in_world};
use crate::refdata::BasicAttack;
use crate::store::Position;
use crate::tick::STEPS_PER_SECOND;
use crate::world::{Phase, Point, Reason, SessionId, Walk, World};

/// A character's attack on a monster.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attack {
    pub target: u32,
    /// The step its running swing ends; `None` before the first swing.
    pub swing_ends: Option<u64>,
}

/// The inventory slot of the weapon, where the vSRO server's 0x3013 lists it
/// [V capture, peer vSRO: 24 characters with a weapon, all in slot 6].
const WEAPON_SLOT: u8 = 6;

/// The pause after a swing's duration (100 ms, see the module).
const SWING_PAUSE: u64 = 3;

/// What reach adds for the two bodies: itemdata's reach is measured past them
/// (a sword reaches 6), positions are their centres [U]. vSRO attackers with a
/// sword stopped 16 to 19 units from where their target was last told to be
/// (p10 to p25 of 48 approaches).
const BODIES: f32 = 10.0;

/// The `unknown` word of every captured basic-attack swing [V capture, peer
/// vSRO: 4754 of 4754]; its meaning is open [U].
const SWING_WORD: u16 = 0x3002;

/// 0x7074. Attacking a living monster the client knows is answered `01 01`
/// and runs, or waits as `01 02` behind a running attack; a cancel ends the
/// attack with `02 00`, twice while a swing runs [V capture, peer vSRO: 4 of
/// 4 cancels during a swing, 3 of 3 outside one]. Other actions are not
/// handled yet.
pub(crate) fn request(world: &mut World, session: SessionId, packet: Packet) -> Result<(), Reason> {
    let Packet::ObjectActionRequest(request) = packet else {
        unreachable!("0x7074 decodes to ObjectActionRequest");
    };
    let Some(Phase::Game { uid, .. }) = world.phase(session) else {
        return Err(Reason::NotHandled);
    };
    let now = world.steps();
    let answers = match request {
        ObjectActionRequest::Execute(ActionCommand::Attack(ActionTarget::Entity {
            unique_id: target,
        })) => {
            let alive = world.monster(target).is_some_and(|m| m.hp > 0);
            if !alive || !world.knows(session, target) {
                return Err(Reason::NotHandled);
            }
            let player = world
                .player_mut(uid)
                .expect("a character in the game has a player");
            let queue_len = if player.attack.is_some() {
                player.queued = Some(target);
                2
            } else {
                player.attack = Some(Attack {
                    target,
                    swing_ends: None,
                });
                1
            };
            vec![ObjectActionResponse::Started { queue_len }]
        }
        ObjectActionRequest::Cancel => {
            let player = world
                .player_mut(uid)
                .expect("a character in the game has a player");
            let swinging = player
                .attack
                .take()
                .and_then(|a| a.swing_ends)
                .is_some_and(|end| now < end);
            player.queued = None;
            let ended = ObjectActionResponse::Ended { queue_len: 0 };
            vec![ended; 1 + usize::from(swinging)]
        }
        _ => return Err(Reason::NotHandled),
    };
    for answer in answers {
        world.send(session, Packet::ObjectActionResponse(answer));
    }
    Ok(())
}

/// Each step: every attack whose swing has ended swings again, walks on into
/// reach, takes up the waiting request, or ends with its target's death.
pub(crate) fn step(world: &mut World) {
    let attackers: Vec<u32> = world
        .players()
        .filter(|(_, p)| p.attack.is_some())
        .map(|(uid, _)| uid)
        .collect();
    for uid in attackers {
        attack(world, uid);
    }
}

fn attack(world: &mut World, uid: u32) {
    let now = world.steps();
    let player = world.player_mut(uid).expect("an attacker is a player");
    let (session, Some(attack)) = (player.session, player.attack) else {
        return;
    };
    if attack.swing_ends.is_some_and(|end| now < end) {
        return;
    }
    // A request that waited behind the swing starts now (`02 01`).
    let target = match attack.swing_ends.and_then(|_| player.queued.take()) {
        Some(next) => {
            player.attack = Some(Attack {
                target: next,
                swing_ends: None,
            });
            let started = ObjectActionResponse::Ended { queue_len: 1 };
            world.send(session, Packet::ObjectActionResponse(started));
            next
        }
        None => attack.target,
    };
    let at = world
        .monster(target)
        .filter(|m| m.hp > 0)
        .map(|m| m.position);
    let basic = player_weapon(world, uid).and_then(|w| world.refdata().basic_attack(w));
    let (Some(at), Some(basic)) = (at, basic) else {
        world
            .player_mut(uid)
            .expect("an attacker is a player")
            .attack = None;
        let ended = ObjectActionResponse::Ended { queue_len: 0 };
        world.send(session, Packet::ObjectActionResponse(ended));
        return;
    };
    let player = world.player(uid).expect("an attacker is a player");
    let (here, walking) = (player.character.position, player.walk.is_some());
    let reach = basic.reach + BODIES;
    if distance(here, at) <= reach {
        swing(world, uid, target, basic);
    } else if !walking {
        approach(world, uid, ground(here), ground(at), reach);
    }
}

/// The weapon `uid` holds, `Some(None)` for bare hands.
fn player_weapon(world: &World, uid: u32) -> Option<Option<u32>> {
    let worn = &world.player(uid)?.character.equipment;
    Some(
        worn.iter()
            .find(|(slot, _)| *slot == WEAPON_SLOT)
            .map(|(_, id)| *id),
    )
}

/// A position in world coordinates.
fn ground(p: Position) -> Point {
    in_world(
        p.region,
        Point {
            x: p.x,
            y: p.y,
            z: p.z,
        },
    )
}

/// The distance between two positions on the ground, across regions.
fn distance(a: Position, b: Position) -> f32 {
    let (a, b) = (ground(a), ground(b));
    (b.x - a.x).hypot(b.z - a.z)
}

/// Walks the attacker straight toward its target until it is a unit inside
/// reach; the clients that know it are told where, as for a requested walk.
fn approach(world: &mut World, uid: u32, here: Point, there: Point, reach: f32) {
    let (dx, dz) = (there.x - here.x, there.z - here.z);
    let share = 1.0 - (reach - 1.0) / dx.hypot(dz);
    let to = Point {
        x: here.x + dx * share,
        y: there.y,
        z: here.z + dz * share,
    };
    let player = world.player_mut(uid).expect("an attacker is a player");
    player.walk = Some(Walk {
        from: here,
        to,
        steps: 0,
    });
    let (region, local) = in_region(to);
    let told = MovementResponse {
        unique_id: uid,
        has_destination: true,
        region,
        x: local.x.round() as i32,
        y: local.y.round() as i32,
        z: local.z.round() as i32,
        angle: 0,
    };
    let observers = world.observers(uid);
    world.broadcast(&observers, Packet::MovementResponse(told));
}

/// One swing of `uid` at `target`: its hits are rolled and taken off the
/// monster's HP, the hit that kills is marked (0x80) and the hits after it
/// land on nothing (arm 8) [V capture, peer vSRO: 57 of 57 two-hit swings
/// that killed with the first]. Who sees both is told the swing (0xB070), who
/// has the monster selected its HP (0x3057), who sees it its death (0x30BF).
fn swing(world: &mut World, uid: u32, target: u32, basic: BasicAttack) {
    let now = world.steps();
    let instance = world.next_action();
    let (low, high) = basic.damage;
    let mut hits = Vec::new();
    for _ in 0..basic.hits {
        let amount = world.chance().u32(low..=high);
        let monster = world
            .monster_mut(target)
            .expect("a swing hits a placed monster");
        if monster.hp == 0 {
            let effect = HitEffect::NoPayload { arm: 8 };
            hits.push(SkillPartDamage {
                killing_blow: false,
                effect,
            });
            continue;
        }
        monster.hp = monster.hp.saturating_sub(amount);
        let value = DamageValue {
            kind: DamageValue::KIND_NORMAL,
            amount,
        };
        hits.push(SkillPartDamage {
            killing_blow: monster.hp == 0,
            effect: HitEffect::Damage { value, unknown: 0 },
        });
    }
    let monster = world
        .monster_mut(target)
        .expect("a swing hits a placed monster");
    let hp = monster.hp;
    if hp == 0 {
        monster.walk = None;
    }
    let damage = DamageContent {
        instance_count: basic.hits,
        entities: vec![PerEntityDamage { target, hits }],
    };
    let update = ObjectActionUpdate::Success {
        unknown: SWING_WORD,
        skill_id: basic.skill,
        source: uid,
        instance,
        target,
        kind: ActionKind::Attack {
            damage: Some(damage),
        },
    };
    let both: Vec<SessionId> = world
        .observers(uid)
        .into_iter()
        .filter(|s| world.knows(*s, target))
        .collect();
    world.broadcast(&both, Packet::ObjectActionUpdate(update));
    let bars = EntityBarsUpdate {
        unique_id: target,
        source: BARS_SOURCE_DAMAGE,
        flag: BARS_FLAG_HP | BARS_FLAG_BAD_STATUS,
        hp: Some(hp),
        mp: None,
        unknown16: None,
        bad_status: Some(0),
        bad_status_levels: Vec::new(),
    };
    let selecting = world.selecting(target);
    world.broadcast(&selecting, Packet::EntityBarsUpdate(bars));
    if hp == 0 {
        monster::fell(world, target);
        let dead = EntityStateUpdate {
            unique_id: target,
            kind: STATE_KIND_LIFE,
            value: LIFE_STATE_DEAD,
            unk_u8_00: None,
        };
        let observers = world.observers(target);
        world.broadcast(&observers, Packet::EntityStateUpdate(dead));
    }
    let lasts = u64::from(basic.duration_ms) * STEPS_PER_SECOND;
    let player = world.player_mut(uid).expect("an attacker is a player");
    player.attack = Some(Attack {
        target,
        swing_ends: Some(now + lasts.div_ceil(1000) + SWING_PAUSE),
    });
}
