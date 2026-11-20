//! Attacking a monster (0x7074). As on the vSRO server, the server runs the
//! swings: one request starts an attack, and the server swings at the target
//! until it dies or the client cancels [V capture, peer vSRO: 47 swings for one
//! request]. A swing lasts its basic attack's Action_ActionDuration and three
//! steps more, and the next one starts when it ends [V capture, peer vSRO: the
//! median time between two swings is that duration plus 94 to 118 ms, for the
//! fist, the sword and the spear].
//!
//! A swing waits until the attacker is in reach. The monster stands while it
//! is attacked and does not hit back yet (decision 0020, E4); what a hit deals
//! is [`RefData::basic_attack`]'s.
//!
//! [`RefData::basic_attack`]: crate::refdata::RefData::basic_attack

use packets::agent::ingame::{
    ActionCommand, ActionKind, ActionTarget, DamageContent, DamageValue, EntityBarsUpdate,
    EntityStateUpdate, HitEffect, ObjectActionRequest, ObjectActionResponse, ObjectActionUpdate,
    PerEntityDamage, SkillPartDamage, BARS_FLAG_BAD_STATUS, BARS_FLAG_HP, BARS_SOURCE_DAMAGE,
    LIFE_STATE_DEAD, STATE_KIND_LIFE,
};
use packets::Packet;

use crate::movement::in_world;
use crate::refdata::BasicAttack;
use crate::store::Position;
use crate::tick::STEPS_PER_SECOND;
use crate::world::{Phase, Point, Reason, SessionId, World};

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
/// and runs; a cancel ends the attack with `02 00`, twice while a swing runs
/// [V capture, peer vSRO: 4 of 4 cancels during a swing, 3 of 3 outside one].
/// A request while an attack runs, and other actions, are not handled yet.
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
            if player.attack.is_some() {
                return Err(Reason::NotHandled);
            }
            player.attack = Some(Attack {
                target,
                swing_ends: None,
            });
            vec![ObjectActionResponse::Started { queue_len: 1 }]
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

/// Each step: every attack whose swing has ended swings again when in reach,
/// or ends with its target's death.
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
    let player = world.player(uid).expect("an attacker is a player");
    let (session, here, Some(attack)) = (player.session, player.character.position, player.attack)
    else {
        return;
    };
    if attack.swing_ends.is_some_and(|end| now < end) {
        return;
    }
    let at = world
        .monster(attack.target)
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
    if distance(here, at) <= basic.reach + BODIES {
        swing(world, uid, attack.target, basic);
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

/// The distance between two positions on the ground, across regions.
fn distance(a: Position, b: Position) -> f32 {
    let point = |p: Position| {
        in_world(
            p.region,
            Point {
                x: p.x,
                y: p.y,
                z: p.z,
            },
        )
    };
    let (a, b) = (point(a), point(b));
    (b.x - a.x).hypot(b.z - a.z)
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
