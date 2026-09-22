//! The lobby's sit pose: a character whose deletion is reserved **sits** in the
//! selection screen, folds down when the player deletes it and gets up when the
//! player restores it.
//!
//! # The idea
//!
//! The line-up itself does not show that a character is about to be deleted.
//! The screen already says it in words (the red `UIO_STT_CHAR_DEL_CONFIRM`
//! notice and the countdown window), but only *after* the figure is clicked.
//!
//! **Deliberate deviation with a stated rationale (ADR-0009), and the
//! original's silence here is a known absence rather than an unknown.** The
//! original v1.188 does *not* pose its lobby figures:
//!
//! * No UI scene plays a character animation at all; every animation start in
//!   the original sits outside the selection scene.
//! * The seated pose is a *state*, `EnterState(6)`, and only the world object
//!   code ever enters it — never the selection screen.
//! * No `resinfo`/`textdata` row declares a pose either; the screen's authored
//!   answer to a reserved deletion is text plus the countdown window.
//!
//! So this is an openroad improvement, not a reproduction — and it is built out
//! of the original's own parts: the clips, their ids and their order are the
//! original's sit chain ([`crate::plugins::animation_sit`], `SIT_DOWN` 13 ->
//! `SIT` 14 -> `STAND_UP` 15), driven here for a lobby figure instead of the
//! player's body.
//!
//! # Why a second driver and not `player::drive_sit_chain`
//!
//! That one is the *world* half: it wants `Player`, `Movement` (whose `sit`
//! triple is built by the player's own graph builder) and `OneShotAttack`, none
//! of which a lobby preview has — it is a plain `spawn_resource` body with an
//! `AnimationPlayer` and an [`AnimationLibrary`]. What is shared is everything
//! that carries the original's behaviour: the phase machine, the ids and the loop rule all
//! come from `animation_sit`; only the ~40 lines that look the clips up in the
//! library and start them are new.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use crate::commands::AnimationLibrary;
use crate::plugins::animation_sit as sit;
use crate::plugins::dynamic_resource_loader::PreferredAnimationGroup;

use super::character_select::{CharacterInfoV2, SelectableCharacterV2};

/// What the lobby last saw for a character name: `true` = its deletion was
/// reserved.
///
/// The transition is read off *this*, not off the delete/restore request, on
/// purpose: a successful `0xB007` despawns the whole line-up and rebuilds it
/// from a fresh `0x7007` list (`character_select::on_character_delete_response`),
/// so there is no surviving entity to animate and no response field naming the
/// character either. Comparing the rebuilt list against what stood there before
/// gives all three cases from data alone:
///
/// | before | now | figure |
/// |---|---|---|
/// | not reserved | reserved | folds down (`SIT_DOWN` -> `SIT`) |
/// | reserved | not reserved | gets up (`STAND_UP` -> stand) |
/// | unknown (first list of the session) | reserved | **already seated** |
#[derive(Resource, Default, Debug)]
pub struct LobbyDeletionMemory(pub HashMap<String, bool>);

/// The phase a rebuilt figure starts in, as a total function of "what stood
/// here before" and "is its deletion reserved now" — the three cases of
/// [`LobbyDeletionMemory`] plus the ordinary character, which gets nothing.
pub fn phase_for(before: Option<bool>, deleting: bool) -> Option<sit::SitPhase> {
    match (before, deleting) {
        // just reserved: let the player watch it sit down
        (Some(false), true) => Some(sit::SitPhase::SittingDown),
        // just restored: let the player watch it get up
        (Some(true), false) => Some(sit::SitPhase::StandingUp),
        // already reserved when the session's first list arrived: seated, no
        // transition — the fold-down happened long before this login
        (_, true) => Some(sit::SitPhase::Sitting),
        (_, false) => None,
    }
}

/// Preview hook, `PREVIEW_DELETING=sit|down|up`: pose **every** lobby figure as
/// if its deletion were reserved.
///
/// Why it exists, and why it borrows the name the 2D preview already uses
/// (`scenes::testing::char_select_ui`, same variable, same meaning "show me the
/// deletion-pending variant"): a reserved character only exists after a real
/// `0xB007` against a live server, so on a server where nothing is scheduled
/// the pose — and both its transitions, which are one-shots — cannot be
/// seen at all. `sit` is the seated pose, `down` plays the fold-down,
/// `up` plays the get-up; inert unless set, and no counterpart in the original.
pub fn preview_phase(value: &str) -> Option<sit::SitPhase> {
    match value {
        "down" => Some(sit::SitPhase::SittingDown),
        "up" => Some(sit::SitPhase::StandingUp),
        _ => Some(sit::SitPhase::Sitting),
    }
}

fn forced_phase() -> Option<sit::SitPhase> {
    preview_phase(std::env::var("PREVIEW_DELETING").ok()?.as_str())
}

/// Gives every freshly spawned lobby figure its phase, and remembers the state
/// the next rebuild will be compared against.
pub fn begin_lobby_sit(
    added: Query<(Entity, &CharacterInfoV2), Added<CharacterInfoV2>>,
    mut memory: ResMut<LobbyDeletionMemory>,
    mut commands: Commands,
) {
    for (entity, info) in added.iter() {
        let name = info.0.name.as_str();
        let deleting = info.0.is_deleting;
        if let Some(phase) =
            forced_phase().or_else(|| phase_for(memory.0.get(name).copied(), deleting))
        {
            debug!("lobby char '{name}': sit phase {phase:?} (reserved: {deleting})");
            commands.entity(entity).insert(phase);
        }
        memory.0.insert(name.to_string(), deleting);
    }
}

/// Starts and advances the chain on the figure's body.
///
/// The body spawns a few frames after its root (the resource loads
/// asynchronously), so this runs every frame and is a no-op until the
/// `AnimationPlayer`/[`AnimationLibrary`] exist. "Has the clip been started
/// yet?" is answered by the player itself — an unknown node means *start it*, a
/// finished one means *advance* — so no second bookkeeping component is needed.
///
/// A body without the three clips (`animation_sit`: they ship together on 78
/// `.bsr`, among them every playable body) simply keeps standing: the lookup
/// returns `None` and nothing here fires.
pub fn drive_lobby_sit(
    roots: Query<
        (
            Entity,
            &sit::SitPhase,
            Option<&PreferredAnimationGroup>,
            &Children,
        ),
        With<SelectableCharacterV2>,
    >,
    children: Query<&Children>,
    mut bodies: Query<(&AnimationLibrary, &mut AnimationPlayer)>,
    mut commands: Commands,
) {
    for (root, &phase, group, root_children) in roots.iter() {
        let Some(body) = find_body(root, root_children, &children, &bodies) else {
            continue;
        };
        let Ok((library, mut player)) = bodies.get_mut(body) else {
            continue;
        };
        let group = group.map(|g| g.0.as_str());
        let Some(node) = clip(library, group, sit::phase_anim_type(phase)) else {
            // no sit clips on this body: drop the phase so the frame after a
            // restore does not keep asking for a clip that does not exist
            commands.entity(root).remove::<sit::SitPhase>();
            continue;
        };
        match player.animation(node) {
            // not started yet: this is the frame it begins
            None => {
                debug!(
                    "lobby sit: {phase:?} -> {:?}",
                    library
                        .entries
                        .iter()
                        .find(|e| e.node == node)
                        .map(|e| e.label.as_str())
                );
                play(&mut player, node, sit::phase_loops(phase));
            }
            Some(active) if active.is_finished() => match sit::phase_after_clip(phase) {
                Some(next) if next != phase => {
                    if let Some(next_node) = clip(library, group, sit::phase_anim_type(next)) {
                        play(&mut player, next_node, sit::phase_loops(next));
                        commands.entity(root).insert(next);
                    }
                }
                Some(_) => {}
                // the chain is over: back to the stand the lobby spawns with
                None => {
                    if let Some(stand) = clip(
                        library,
                        group,
                        crate::assets::bsr::resource::ANIM_TYPE_STAND,
                    ) {
                        play(&mut player, stand, true);
                    }
                    commands.entity(root).remove::<sit::SitPhase>();
                }
            },
            Some(_) => {}
        }
    }
}

/// The spawned body under a lobby root: the descendant that carries the
/// animation library (`spawn_resource` puts player + library on the resource
/// root it spawns under the char entity).
fn find_body(
    root: Entity,
    root_children: &Children,
    children: &Query<&Children>,
    bodies: &Query<(&AnimationLibrary, &mut AnimationPlayer)>,
) -> Option<Entity> {
    if bodies.contains(root) {
        return Some(root);
    }
    root_children
        .iter()
        .find(|child| bodies.contains(*child))
        .or_else(|| {
            children
                .iter_descendants(root)
                .find(|e| bodies.contains(*e))
        })
}

/// The library node of `anim_type`, with the same group fallback the spawner
/// uses for the initial stand (`commands::SpawnResource`): the preferred group
/// first, then `default`, then whatever group ships the clip.
fn clip(
    library: &AnimationLibrary,
    group: Option<&str>,
    anim_type: u32,
) -> Option<AnimationNodeIndex> {
    let of_group = |wanted: Option<&str>| {
        library
            .entries
            .iter()
            .find(|e| Some(e.group.as_str()) == wanted && e.anim_type == anim_type)
    };
    of_group(group)
        .or_else(|| of_group(Some("default")))
        .or_else(|| library.entries.iter().find(|e| e.anim_type == anim_type))
        .map(|e| e.node)
}

/// Hard cut, like every other clip start in the lobby: these bodies get a plain
/// `AnimationPlayer` from `spawn_resource` (no `AnimationTransitions` — that
/// component is inserted by `plugins::player` for the world's wrappers only), so
/// there is nothing here to cross-fade *through*. Stated rather than hidden: the
/// original's `SIT_DOWN`/`STAND_UP` cut in too (zero blend-in), only its
/// `SIT` fades over 200 ms.
///
/// **`stop_all` first, and this is not tidiness — it is the bug this function
/// exists to avoid.** `spawn_resource` starts the looping stand
/// (`commands/mod.rs`), and Bevy *blends* every active animation by weight: a
/// second clip played on top at weight 1.0 does not replace the stand, it
/// averages with it. The symptom: figures with `default/14 chinaman_a_sitbreath`
/// playing next to the stand look like they are standing with slightly bent
/// knees — the seated pose is there, at half weight. The same trap is written up in
/// [`crate::plugins::animation_sit`]'s module doc for the world's wrappers.
fn play(player: &mut AnimationPlayer, node: AnimationNodeIndex, looping: bool) {
    player.stop_all();
    let active = player.play(node);
    if looping {
        active.repeat();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::animation_sit::{ANIM_TYPE_SIT, ANIM_TYPE_SIT_DOWN, ANIM_TYPE_STAND_UP};

    /// The three cases this driver must cover, and the negative control that
    /// makes the other three mean something: an ordinary character gets **no**
    /// phase at all, so nothing in the line-up moves that should not.
    #[test]
    fn the_three_cases_the_owner_named_and_the_one_that_must_stay_standing() {
        // Delete: it was standing, now it is reserved -> folds down
        assert_eq!(
            phase_for(Some(false), true),
            Some(sit::SitPhase::SittingDown)
        );
        // Restore: it was reserved, now it is not -> gets up
        assert_eq!(
            phase_for(Some(true), false),
            Some(sit::SitPhase::StandingUp)
        );
        // Fresh login on a reserved character: seated from the first frame
        assert_eq!(phase_for(None, true), Some(sit::SitPhase::Sitting));
        // ... and it stays seated across a rebuild that changes nothing
        assert_eq!(phase_for(Some(true), true), Some(sit::SitPhase::Sitting));
        // Negative control: an ordinary character is never posed
        assert_eq!(phase_for(None, false), None);
        assert_eq!(phase_for(Some(false), false), None);
    }

    /// The preview hook's three values, and the one that is not a pose.
    #[test]
    fn the_preview_hook_maps_its_three_values() {
        assert_eq!(preview_phase("sit"), Some(sit::SitPhase::Sitting));
        assert_eq!(preview_phase("down"), Some(sit::SitPhase::SittingDown));
        assert_eq!(preview_phase("up"), Some(sit::SitPhase::StandingUp));
        // any other value still previews the pose rather than doing nothing:
        // the variable is only ever set deliberately
        assert_eq!(preview_phase("1"), Some(sit::SitPhase::Sitting));
    }

    /// The chain and its ids stay the original's, not a copy that can drift:
    /// the clips come from `animation_sit`, which sources them in the binary.
    #[test]
    fn the_chain_is_the_original_one() {
        assert_eq!(
            [ANIM_TYPE_SIT_DOWN, ANIM_TYPE_SIT, ANIM_TYPE_STAND_UP],
            sit::SIT_CHAIN
        );
        assert_eq!(sit::phase_anim_type(sit::SitPhase::Sitting), ANIM_TYPE_SIT);
        // only the seated pose holds; both transitions are one-shots that end
        assert!(sit::phase_loops(sit::SitPhase::Sitting));
        assert!(!sit::phase_loops(sit::SitPhase::SittingDown));
        assert!(!sit::phase_loops(sit::SitPhase::StandingUp));
        // and the chain's exits are the ones this driver relies on
        assert_eq!(
            sit::phase_after_clip(sit::SitPhase::SittingDown),
            Some(sit::SitPhase::Sitting)
        );
        assert_eq!(sit::phase_after_clip(sit::SitPhase::StandingUp), None);
    }
}
