//! Status effects: incapacitating debuffs (stun/freeze) and timed
//! self-buffs with their looping aura effects.
//!
//! Two sources, and they are not equally good. **Online the wire wins**: the
//! 0x3057 bad-status block is an absolute per-entity ailment bitmask, decoded
//! into `net::entities::EntityAilments` and presented by
//! [`sync_ailment_effects`] at the bottom of this file. That is the
//! authoritative path for monsters and the local player alike.
//!
//! The skilldata path below is the OFFLINE simulation's, and it rests on
//! weaker evidence: only `'st' <duration_ms> <prob%> <level>` (stun) is
//! corpus-verified. `'fz'/'fb'/'es'/'bu'` — freeze / frostbite / electric
//! shock / burn — have **inferred names and UNKNOWN argument counts**
//! (`docs/re/gamedata/textdata-effect-option-tables.md`), `'fb'` does not even
//! appear in the tag census, and they are read with `param_after`, a linear
//! scan that can match an argument value rather than a tag. Treat what it
//! produces as a demo, not as fact. The local simulation applies them
//! deterministically (no
//! probability roll — documented simplification, so a stun skill always
//! demonstrably stuns). Incapacitated entities stop moving, stop facing,
//! and get their in-flight one-shot cast cancelled by the player plugin;
//! the cast pipeline refuses new casts from an incapacitated caster.
//! Self-buffs hold an [`AppliedBuff`] with the skill's `'dura'` duration
//! and own their ACT_S burst / ACT_L loop emissions, despawned on expiry.

use std::collections::HashMap;

use bevy::prelude::*;
use packets::agent::prelude::BuffRemainTime;

use crate::assets::textdata::skilldata::SkillDataRow;
use crate::assets::textdata::skilleffect::{EffectPhase, SkillEntry};
use crate::commands::SkeletonBinding;
use crate::plugins::combat::{FaceTarget, TimedEffect};
use crate::plugins::effects::spawn::DelayedEffect;
use crate::plugins::effects::OneShotEffect;
use crate::plugins::net::entities::RemoteMovement;
use crate::plugins::player::{Player, PlayerCommands};
use crate::plugins::skills::cast::SkillSwingStarted;
use crate::plugins::textdata::{ClientSkillData, ClientSkillEffects};

/// Stunned: no movement, no casting, current cast cancelled.
#[derive(Component)]
pub struct Stunned(pub Timer);

/// Frozen: same incapacitation as stun (visual distinction comes later).
#[derive(Component)]
pub struct Frozen(pub Timer);

/// Query filter matching any incapacitating status.
pub type IncapacitatedFilter = Or<(With<Stunned>, With<Frozen>)>;

/// Fallback duration for status params that carry no duration argument
/// (freeze & co.), and for buffs without a `'dura'` tag.
const STATUS_FALLBACK_SECS: f32 = 5.0;
const BUFF_FALLBACK_SECS: f32 = 10.0;

/// One active self-buff on an entity, owning its aura effect wrappers.
pub struct BuffInstance {
    pub codename: String,
    pub timer: Timer,
    /// Spawned ACT_L loop wrappers, despawned when the buff expires
    /// (ACT_S bursts clean themselves up via [`TimedEffect`]).
    pub loop_effects: Vec<Entity>,
    /// The skill's icon (`media://icon/...`), shown on the magic state
    /// board next to the player mini info.
    pub icon: Option<String>,
    /// The server's buff-instance id (0xB0BD), what the assumed 0xB072
    /// removal references; `None` for purely local applications.
    pub instance_id: Option<u32>,
}

/// The entity's active self-buffs (the local player today).
#[derive(Component, Default)]
pub struct ActiveBuffs(pub Vec<BuffInstance>);

/// Read the row's status params and insert the matching debuff components
/// on `target`. Deterministic: any listed status applies.
pub fn apply_status_effects(commands: &mut Commands, row: &SkillDataRow, target: Entity) {
    if let Some(duration) = row.param_after("st") {
        let secs = (duration as f32 / 1000.0).max(0.1);
        info!("skills: stunning target for {secs:.1}s");
        commands
            .entity(target)
            .insert(Stunned(Timer::from_seconds(secs, TimerMode::Once)));
    }
    // freeze/frostbite share the frozen presentation; shock stuns briefly
    if row.param_after("fz").is_some() || row.param_after("fb").is_some() {
        info!("skills: freezing target for {STATUS_FALLBACK_SECS}s");
        commands.entity(target).insert(Frozen(Timer::from_seconds(
            STATUS_FALLBACK_SECS,
            TimerMode::Once,
        )));
    }
    if row.param_after("es").is_some() {
        info!("skills: shocking target");
        commands
            .entity(target)
            .insert(Stunned(Timer::from_seconds(2.0, TimerMode::Once)));
    }
}

/// While stunned/frozen: stop server-style movement and target facing; stop
/// the local player's click-to-move. (One-shot cast cancellation lives in
/// the player plugin, which owns the wrapper animation state.)
pub fn enforce_incapacitation(
    mut afflicted: Query<(Entity, Option<&mut RemoteMovement>, Has<Player>), IncapacitatedFilter>,
    mut player_commands: ResMut<PlayerCommands>,
    mut commands: Commands,
) {
    for (entity, movement, is_player) in afflicted.iter_mut() {
        if let Some(mut movement) = movement {
            movement.target = None;
        }
        if is_player {
            player_commands.stop();
        }
        commands.entity(entity).remove::<FaceTarget>();
    }
}

/// Tick status timers and drop expired components.
pub fn expire_status_effects(
    time: Res<Time>,
    mut stunned: Query<(Entity, &mut Stunned)>,
    mut frozen: Query<(Entity, &mut Frozen)>,
    mut commands: Commands,
) {
    for (entity, mut status) in stunned.iter_mut() {
        if status.0.tick(time.delta()).just_finished() {
            commands.entity(entity).remove::<Stunned>();
        }
    }
    for (entity, mut status) in frozen.iter_mut() {
        if status.0.tick(time.delta()).just_finished() {
            commands.entity(entity).remove::<Frozen>();
        }
    }
}

/// Spawn a buff's ACT_S burst + ACT_L loop emissions on `wrapper`, anchored
/// to their authored bones (pelvis/waist/hand) with the raw offset,
/// inheriting the single character mirror — the same recipe as
/// `cast::spawn_cast_effects` and the always-on effect path; falls back to
/// the wrapper when the bone is unnamed. Returns the loop wrappers (the
/// caller despawns them when the buff ends); ACT_S bursts clean themselves
/// up via [`TimedEffect`] and play once ([`OneShotEffect`]).
fn spawn_buff_auras(
    entry: &SkillEntry,
    wrapper: Entity,
    skeletons: &Query<&SkeletonBinding>,
    asset_server: &AssetServer,
    commands: &mut Commands,
) -> Vec<Entity> {
    let mut loop_effects = Vec::new();
    for emission in &entry.emissions {
        let looping = match emission.phase {
            EffectPhase::ActL => true,
            EffectPhase::ActS => false,
            _ => continue,
        };
        let anchor = skeletons
            .get(wrapper)
            .ok()
            .and_then(|binding| {
                emission
                    .start_bone
                    .as_ref()
                    .and_then(|bone| binding.bones.get(bone).copied())
            })
            .unwrap_or(wrapper);
        let mut spawned = commands.spawn((
            DelayedEffect {
                handle: asset_server.load(format!("particles://{}", emission.efp_path)),
                timer: Timer::from_seconds(0.0, TimerMode::Once),
            },
            Transform::from_translation(emission.start_offset),
            Visibility::Inherited,
            Name::new(format!("buff effect: {}", emission.efp_path)),
            ChildOf(anchor),
        ));
        if looping {
            loop_effects.push(spawned.id());
        } else {
            spawned.insert((TimedEffect::new(6.0), OneShotEffect));
        }
    }
    loop_effects
}

/// Self-buff application, piggybacking on the swing-start report so the
/// aura effects anchor on the caster's loaded body wrapper: skills without
/// an enemy target hold an [`AppliedBuff`]-style entry for their `'dura'`
/// and spawn their ACT_S burst + ACT_L loop emissions. Re-casting an
/// active buff refreshes its timer.
pub fn apply_self_buffs(
    mut started: MessageReader<SkillSwingStarted>,
    skill_data: Res<ClientSkillData>,
    skill_effects: Res<ClientSkillEffects>,
    mut buffs: Query<&mut ActiveBuffs>,
    skeletons: Query<&SkeletonBinding>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    for swing in started.read() {
        let Some(entry) = skill_effects.get(&swing.codename) else {
            continue;
        };
        // enemy-targeted swings were damage casts; buffs are the self casts
        // anything non-offensive that got this far is a self-buff — imbues
        // (activity 1) carry all-zero target columns, so a positive
        // `targets_self` requirement would miss them. The EXACT row (via the
        // swing's skill id) wins: the codename scan lands on an arbitrary
        // level of the group, giving the wrong 'dura'/icon for leveled buffs.
        let row = swing
            .skill_id
            .and_then(|id| skill_data.get(&(id as i32)))
            .or_else(|| {
                skill_data.data().and_then(|data| {
                    data.values()
                        .find(|r| r.basic_group() == Some(swing.codename.as_str()))
                })
            });
        let is_buff = row.is_some_and(|r| !r.targets_enemy());
        if !is_buff {
            continue;
        }
        let duration = row
            .and_then(|r| r.param_after("dura"))
            .map(|ms| ms as f32 / 1000.0)
            .unwrap_or(BUFF_FALLBACK_SECS);

        // re-cast of an active buff just refreshes its timer (only the
        // timer: an adopted 0xB0BD instance_id must survive the refresh)
        if let Ok(mut active) = buffs.get_mut(swing.owner) {
            if let Some(existing) = active
                .0
                .iter_mut()
                .find(|buff| buff.codename == swing.codename)
            {
                existing.timer = Timer::from_seconds(duration, TimerMode::Once);
                continue;
            }
        }

        let loop_effects = spawn_buff_auras(
            entry,
            swing.wrapper,
            &skeletons,
            &asset_server,
            &mut commands,
        );
        info!(
            "skills: buff {} active for {duration:.1}s ({} loop effects)",
            swing.codename,
            loop_effects.len()
        );
        let instance = BuffInstance {
            codename: swing.codename.clone(),
            timer: Timer::from_seconds(duration, TimerMode::Once),
            loop_effects,
            icon: row.and_then(|r| r.icon_path()),
            instance_id: None,
        };
        push_buff_instance(&mut buffs, &mut commands, swing.owner, instance);
    }
}

/// Push or refresh (by codename) a buff on `owner`. A re-application only
/// rewrites the timer and adopts a wire instance id — existing loop effects
/// and icon stay, so a 0xB0BD following our own cast's [`apply_self_buffs`]
/// entry MERGES instead of duplicating (the incoming duplicate's loop
/// effects are despawned).
pub(crate) fn push_buff_instance(
    buffs: &mut Query<&mut ActiveBuffs>,
    commands: &mut Commands,
    owner: Entity,
    instance: BuffInstance,
) {
    if let Ok(mut active) = buffs.get_mut(owner) {
        if let Some(existing) = active
            .0
            .iter_mut()
            .find(|buff| buff.codename == instance.codename)
        {
            existing.timer = instance.timer;
            if instance.instance_id.is_some() {
                existing.instance_id = instance.instance_id;
            }
            for effect in instance.loop_effects {
                if let Ok(mut e) = commands.get_entity(effect) {
                    e.despawn();
                }
            }
            return;
        }
        active.0.push(instance);
    } else {
        commands.entity(owner).insert(ActiveBuffs(vec![instance]));
    }
}

/// Buffs the server never timed and skilldata gives no `'dura'` for are
/// treated as permanent-until-removed: a day-long Timer keeps the gauge/expiry
/// machinery untouched (infinite `Duration`s panic, and an `Option<Timer>`
/// would ripple through expire_buffs for no visible gain — the board gauge
/// simply reads full).
///
/// A placeholder, not an estimate, and 0x3206 is why it is no longer the answer
/// for the buffs that have one: of the 114 captured remaining times
/// (`packet_dump/0x3206.log`, 20898000…256516000 ms = 5.80 h…71.25 h) exactly
/// one sits within 1% of 24 h. It now only covers the gap until a refresh
/// arrives.
const PERMANENT_BUFF_SECS: f32 = 24.0 * 3600.0;

/// A 0x3206 remaining time as Timer seconds.
///
/// No conversion of the time base is needed and that is the point: the wire
/// value counts down with time spent in the game, not with the clock
/// (`BuffRemainTime::Own::remaining_play_ms`), and a [`Timer`] ticked from
/// `Res<Time>` counts the same thing. Clamped away from zero the way
/// [`apply_status_effects`] clamps a stun duration — a zero-length Timer makes
/// `fraction_remaining()` divide by zero, and the board would paint a NaN gauge.
fn buff_timer_secs(remaining_ms: u32) -> f32 {
    (remaining_ms as f32 / 1000.0).max(0.1)
}

/// 0xB0BD/0xB072/0x3206 — server-driven buff add, remove and remaining time
/// (the join-time auto-buffs and every networked cast's authoritative buff
/// state). Network adds also
/// spawn the ACT_S/ACT_L aura effects UNLESS the entity already shows the
/// buff (our own cast's [`apply_self_buffs`] entry — the following 0xB0BD
/// then merges by codename without re-spawning): without this, item/scroll
/// buffs got their board icon but no visual (the fourth-playtest report).
/// The aura anchors on the owner's body wrapper (the child carrying the
/// [`SkeletonBinding`]); if the body hasn't streamed in yet, the effects
/// fall back to the entity root.
///
/// The remaining time is read here, in the same system as the adds, rather
/// than in one of its own. 0xB0BD carries no duration and the server sends the
/// 0x3206 refresh for it in the same millisecond (measured over all 114 frames
/// of `packet_dump/0x3206.log`), so both land in one batch —
/// but [`push_buff_instance`] creates `ActiveBuffs` through `Commands`, so a
/// buff added in this run is invisible to every query until the next sync
/// point. A separate system would therefore drop exactly the first refresh,
/// the one that replaces [`PERMANENT_BUFF_SECS`]. Collected before the add
/// loop, the wire time becomes the buff's first duration instead.
pub fn apply_network_buffs(
    mut adds: MessageReader<packets::agent::prelude::BuffAdd>,
    mut removes: MessageReader<packets::agent::prelude::BuffRemove>,
    mut refreshes: MessageReader<BuffRemainTime>,
    net: Res<crate::plugins::net::entities::NetworkEntities>,
    skill_data: Res<ClientSkillData>,
    skill_effects: Res<ClientSkillEffects>,
    mut buffs: Query<&mut ActiveBuffs>,
    children: Query<&Children>,
    skeletons: Query<&SkeletonBinding>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    // Both arms are keyed by instance id alone: 0xB072 removes buffs by
    // instance id with no uid beside it, so those ids are unique across
    // entities and the `Entity` arm's `unique_id` adds nothing a lookup needs.
    let mut wire_remaining: HashMap<u32, u32> = HashMap::new();
    for refresh in refreshes.read() {
        match refresh {
            BuffRemainTime::Own {
                buff_instance_id,
                remaining_play_ms,
            } => {
                wire_remaining.insert(*buff_instance_id, *remaining_play_ms);
            }
            BuffRemainTime::Entity {
                buff_instance_id,
                remaining_ms,
                ..
            } => {
                wire_remaining.insert(*buff_instance_id, *remaining_ms);
            }
            BuffRemainTime::Unknown { raw } => {
                debug!(
                    "skills: 0x3206 kept whole, {} bytes (no modelled sub-type)",
                    raw.len()
                );
            }
        }
    }

    for add in adds.read() {
        let Some(owner) = net.get(add.unique_id) else {
            // join-order race: the buff may beat the entity registration —
            // logged so playtests can tell whether a retry buffer is needed
            debug!("skills: buff add for unknown uid {}", add.unique_id);
            continue;
        };
        let Some(row) = skill_data.get(&(add.ref_skill_id as i32)) else {
            debug!("skills: buff add for unknown skill {}", add.ref_skill_id);
            continue;
        };
        // 0xB0BD carries no duration of its own, so: the server's remaining
        // time when its 0x3206 arrived with the add, then skilldata's 'dura',
        // else permanent-until-0xB072. NOT the 10s BUFF_FALLBACK — that would
        // silently expire real permanent buffs.
        let duration = wire_remaining
            .remove(&add.buff_instance_id)
            .map(buff_timer_secs)
            .or_else(|| row.param_after("dura").map(|ms| ms as f32 / 1000.0))
            .unwrap_or(PERMANENT_BUFF_SECS);
        let codename = row.basic_group().unwrap_or(row.code_name()).to_string();
        info!(
            "skills: server buff {codename} on uid {} ({duration:.0}s, instance {})",
            add.unique_id, add.buff_instance_id
        );
        let already_shown = buffs
            .get(owner)
            .is_ok_and(|active| active.0.iter().any(|buff| buff.codename == codename));
        let loop_effects = if already_shown {
            Vec::new()
        } else {
            skill_effects
                .get(&codename)
                .map(|entry| {
                    let wrapper = children
                        .get(owner)
                        .into_iter()
                        .flat_map(|c| c.iter())
                        .find(|&child| skeletons.contains(child))
                        .unwrap_or(owner);
                    spawn_buff_auras(entry, wrapper, &skeletons, &asset_server, &mut commands)
                })
                .unwrap_or_default()
        };
        push_buff_instance(
            &mut buffs,
            &mut commands,
            owner,
            BuffInstance {
                codename,
                timer: Timer::from_seconds(duration, TimerMode::Once),
                loop_effects,
                icon: row.icon_path(),
                instance_id: Some(add.buff_instance_id),
            },
        );
    }
    // Whatever is left names a buff that was added in an earlier frame. No
    // captured frame does that — across 129 frames in six dump trees every
    // instance id appears exactly once, always beside its add — so this is
    // the unproven branch, kept because the refresh carries an instance id
    // and nothing in it says "only valid with an add". An id nothing holds
    // has nothing to refresh, so it is logged, not guessed at.
    for (instance_id, remaining_ms) in wire_remaining {
        let secs = buff_timer_secs(remaining_ms);
        let mut refreshed = false;
        for mut active in buffs.iter_mut() {
            let Some(buff) = active
                .0
                .iter_mut()
                .find(|buff| buff.instance_id == Some(instance_id))
            else {
                continue;
            };
            buff.timer = Timer::from_seconds(secs, TimerMode::Once);
            info!(
                "skills: buff {} has {secs:.0}s of play time left (0x3206 instance {instance_id})",
                buff.codename
            );
            refreshed = true;
        }
        if !refreshed {
            debug!("skills: 0x3206 remaining time for unheld buff instance {instance_id}");
        }
    }
    // 0xB072 is a LIST (count-prefixed) — see BuffRemove. Captures only ever
    // showed one id, but the original loops, so drop every id it names.
    for remove in removes.read() {
        for instance_id in &remove.buff_instance_ids {
            info!("skills: buff remove instance {instance_id} (0xB072)");
            for mut active in buffs.iter_mut() {
                let Some(pos) = active
                    .0
                    .iter()
                    .position(|buff| buff.instance_id == Some(*instance_id))
                else {
                    continue;
                };
                let buff = active.0.remove(pos);
                for effect in buff.loop_effects {
                    if let Ok(mut e) = commands.get_entity(effect) {
                        e.despawn();
                    }
                }
                info!("skills: buff {} removed by server", buff.codename);
            }
        }
    }
}

/// Cancel an active buff by codename: despawn its aura wrappers and drop
/// the instance (the magic state board's right-click path). Returns whether
/// a buff was removed. Nothing goes to the server — SilkroadDoc documents
/// buff add/remove only as server→client (0xB0BD/0xB072); the client's
/// cancel-request opcode needs a vanilla capture before it can be wired.
pub fn cancel_buff(active: &mut ActiveBuffs, codename: &str, commands: &mut Commands) -> bool {
    let Some(pos) = active.0.iter().position(|buff| buff.codename == codename) else {
        return false;
    };
    let buff = active.0.remove(pos);
    for effect in buff.loop_effects {
        if let Ok(mut e) = commands.get_entity(effect) {
            e.despawn();
        }
    }
    info!("skills: buff {} cancelled", buff.codename);
    true
}

/// Tick buffs; despawn loop effects when one expires.
pub fn expire_buffs(time: Res<Time>, mut buffs: Query<&mut ActiveBuffs>, mut commands: Commands) {
    for mut active in buffs.iter_mut() {
        active.0.retain_mut(|buff| {
            if !buff.timer.tick(time.delta()).just_finished() {
                return true;
            }
            info!("skills: buff {} expired", buff.codename);
            for effect in buff.loop_effects.drain(..) {
                if let Ok(mut e) = commands.get_entity(effect) {
                    e.despawn();
                }
            }
            false
        });
    }
}

// --- Wire-driven ailments (0x3057 bad-status mask) --------------------------
//
// Idea: online the server owns ailments outright — `EntityBarsUpdate`'s
// `flag & 0x04` block carries an absolute per-entity `BadStatus` bitmask
// (`net::entities::EntityAilments`), for monsters and the local player alike.
// That is a far better source than `apply_status_effects` above, which infers
// stun/freeze from skilldata fourcc tags whose names and argument counts are
// still `[U]` — see this module's header. The offline sim keeps using the
// tags because it has no wire; online, this is what actually fires.
//
// Presentation is the archive's own art: each ailment names an authored
// `battle/status_bad_<name>.efp`, attached to the body wrapper for as long as
// the bit stays set. **No tint is applied** — no per-ailment colour exists in
// any SRO data we have (textdata, resinfo, or the RE corpus), so choosing one
// would be exactly the unsourced magic number ADR-0009 forbids. An ailment
// whose `.efp` is missing from the archive simply shows nothing rather than
// borrowing another's art.

use packets::agent::prelude::Ailment;

use crate::plugins::net::entities::EntityAilments;

/// The authored looping debuff effect for one ailment, when the archive has
/// one paired to it ([`Ailment::archive_name`] documents the pairing and which
/// ailments deliberately have none).
fn ailment_effect(ailment: Ailment) -> Option<String> {
    ailment
        .archive_name()
        .map(|name| format!("particles://battle/status_bad_{name}.efp"))
}

/// The ailment effect wrappers currently attached to an entity, keyed by the
/// mask bit that spawned them, so a bit going out despawns exactly its own.
#[derive(Component, Default)]
pub struct AilmentEffects {
    /// The mask these effects were spawned for — the diff base.
    shown: u32,
    effects: Vec<(u32, Entity)>,
}

/// Reconcile each entity's authored ailment effects with the mask 0x3057 last
/// reported for it: spawn a looping `.efp` for every bit that came on, despawn
/// the wrappers of every bit that went off.
///
/// Diffed against what is actually spawned rather than driven by change
/// detection, because the mask is re-sent unchanged on every vitals tick — a
/// burning monster's 0x3057 repeats `08000000` for the whole burn — and
/// re-spawning the aura each tick would stack dozens of copies.
pub fn sync_ailment_effects(
    changed: Query<(Entity, &EntityAilments, Option<&Children>)>,
    wrappers: Query<(), With<crate::commands::SpawnedFromResource>>,
    skeletons: Query<&SkeletonBinding>,
    mut shown: Query<&mut AilmentEffects>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    for (entity, ailments, children) in changed.iter() {
        let mask = ailments.0 .0;
        let current = shown.get(entity).map(|s| s.shown).unwrap_or(0);
        if mask == current {
            continue;
        }
        // The body wrapper is where the model (and its skeleton) lives; the
        // root is the fallback for the frames before it has streamed in.
        let anchor = children
            .into_iter()
            .flat_map(|c| c.iter())
            .find(|child| wrappers.contains(*child))
            .unwrap_or(entity);
        // Nothing to anchor to yet — leave `shown` untouched so this retries
        // next frame once the body arrives, rather than recording a mask whose
        // effects never spawned.
        if anchor == entity && skeletons.get(entity).is_err() && children.is_none() {
            continue;
        }
        let mut entry = match shown.get_mut(entity) {
            Ok(entry) => entry,
            Err(_) => {
                commands
                    .entity(entity)
                    .try_insert(AilmentEffects::default());
                continue; // pick it up next frame, once the component exists
            }
        };
        // bits that went out
        entry.effects.retain(|(bit, effect)| {
            if mask & bit != 0 {
                return true;
            }
            if let Ok(mut e) = commands.get_entity(*effect) {
                e.despawn();
            }
            false
        });
        // bits that came on
        for ailment in ailments.0.ailments() {
            let bit = ailment.bit();
            if current & bit != 0 {
                continue;
            }
            let Some(path) = ailment_effect(ailment) else {
                // No authored file pairs to this bit — show nothing rather
                // than borrow another ailment's art. Still recorded in
                // `shown`, so it is not retried every frame.
                debug!("status: {ailment:?} on {entity} has no authored effect — nothing played");
                continue;
            };
            debug!("status: {ailment:?} on {entity} — playing {path}");
            let effect = commands
                .spawn((
                    DelayedEffect {
                        handle: asset_server.load(path.clone()),
                        timer: Timer::from_seconds(0.0, TimerMode::Once),
                    },
                    Transform::IDENTITY,
                    Visibility::Inherited,
                    Name::new(format!("ailment effect: {path}")),
                    ChildOf(anchor),
                ))
                .id();
            entry.effects.push((bit, effect));
        }
        entry.shown = mask;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::net::entities::NetworkEntities;
    use packets::agent::prelude::{BuffAdd, BuffRemove};

    /// One buff already held, started on [`PERMANENT_BUFF_SECS`] — the value
    /// the board gauge used to read for every server-timed buff. The skill
    /// tables stay empty on purpose: these tests drive the refresh path, which
    /// never consults them.
    ///
    /// `TaskPoolPlugin` before `AssetPlugin` is the house fixture pairing
    /// (#665, same shape as `player_mini_info`'s param smoke) — the system asks
    /// for `Res<AssetServer>`.
    fn app_holding(instance_id: Option<u32>) -> App {
        let mut app = App::new();
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ))
        .init_resource::<NetworkEntities>()
        .init_resource::<ClientSkillData>()
        .init_resource::<ClientSkillEffects>()
        .add_message::<BuffAdd>()
        .add_message::<BuffRemove>()
        .add_message::<BuffRemainTime>()
        .add_systems(Update, apply_network_buffs);
        app.world_mut().spawn(ActiveBuffs(vec![BuffInstance {
            codename: "SKILL_SERVER_AUTO_BUFF".to_string(),
            timer: Timer::from_seconds(PERMANENT_BUFF_SECS, TimerMode::Once),
            loop_effects: Vec::new(),
            icon: None,
            instance_id,
        }]));
        app
    }

    fn held_timer(app: &mut App) -> Timer {
        let mut timers: Vec<Timer> = app
            .world_mut()
            .query::<&ActiveBuffs>()
            .iter(app.world())
            .flat_map(|active| active.0.iter().map(|buff| buff.timer.clone()))
            .collect();
        assert_eq!(timers.len(), 1, "the fixture holds exactly one buff");
        timers.remove(0)
    }

    /// Capture line `0x2557` from `packet_dump/0x3206.log` (the same body the
    /// packets-crate decode test pins): 134372000 ms = 37.3 h of play time.
    const CAPTURED_INSTANCE: u32 = 0x2557;
    const CAPTURED_REMAINING_MS: u32 = 134_372_000;

    #[test]
    fn a_refresh_replaces_the_permanent_placeholder_on_the_named_instance() {
        let mut app = app_holding(Some(CAPTURED_INSTANCE));
        app.world_mut().write_message(BuffRemainTime::Own {
            buff_instance_id: CAPTURED_INSTANCE,
            remaining_play_ms: CAPTURED_REMAINING_MS,
        });
        app.update();

        let timer = held_timer(&mut app);
        assert_eq!(
            timer.duration().as_secs(),
            u64::from(CAPTURED_REMAINING_MS / 1000),
            "the server's remaining time must land on the buff"
        );
        assert_ne!(timer.duration().as_secs(), PERMANENT_BUFF_SECS as u64);
        // What the board reads (`update_state_gauges` -> `gauge_fill_width`).
        assert_eq!(timer.fraction_remaining(), 1.0);
    }

    /// The instance id is the only key 0x3206 carries, so a refresh for an id
    /// nobody holds must leave every buff alone rather than pick one. (This
    /// drives the late-refresh branch, which no capture exercises.)
    #[test]
    fn a_refresh_for_an_unheld_instance_changes_nothing() {
        let mut app = app_holding(Some(CAPTURED_INSTANCE));
        app.world_mut().write_message(BuffRemainTime::Own {
            buff_instance_id: CAPTURED_INSTANCE + 1,
            remaining_play_ms: CAPTURED_REMAINING_MS,
        });
        app.update();
        assert_eq!(
            held_timer(&mut app).duration().as_secs(),
            PERMANENT_BUFF_SECS as u64
        );
    }

    /// A locally-applied buff (`instance_id: None`, our own cast before any
    /// 0xB0BD) must not be adopted by a refresh that names an id.
    #[test]
    fn a_local_buff_without_an_instance_id_is_never_refreshed() {
        let mut app = app_holding(None);
        app.world_mut().write_message(BuffRemainTime::Own {
            buff_instance_id: CAPTURED_INSTANCE,
            remaining_play_ms: CAPTURED_REMAINING_MS,
        });
        app.update();
        assert_eq!(
            held_timer(&mut app).duration().as_secs(),
            PERMANENT_BUFF_SECS as u64
        );
    }

    /// Why [`buff_timer_secs`] clamps: a zero-length Timer divides by zero in
    /// `fraction_remaining()`, and the board would multiply NaN into a gauge
    /// width. Asserted through the value the board reads, not the clamp.
    #[test]
    fn a_zero_remaining_time_still_gives_the_gauge_a_number() {
        let timer = Timer::from_seconds(buff_timer_secs(0), TimerMode::Once);
        assert!(
            timer.fraction_remaining().is_finite(),
            "a NaN fraction paints a NaN gauge width"
        );
    }
}
