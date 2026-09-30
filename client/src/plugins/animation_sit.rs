//! The sit chain (`SIT_DOWN` -> `SIT` -> `STAND_UP`): the pure state half of
//! the original's seated state, shared by the lobby (a delete-scheduled figure
//! sits) and, later, the in-game toggle.
//!
//! Idea: sitting is not one pose. The original plays the looping `SIT`
//! (animation type 14) and, when the body was upright before, `SIT_DOWN` (13)
//! on top; leaving the state plays `STAND_UP` (15). Bevy's
//! `AnimationTransitions` manages one main clip, so the two are played **in
//! sequence** here — `SIT_DOWN` once, then a cross-fade into the looping
//! `SIT` (deliberate deviation, ADR-0009). Type ids 13/14/15 come from the
//! `.bsr` files: every playable body ships all three.
//!
//! Only data and transitions live here; the keyboard toggle belongs to the
//! in-game HUD and is not part of this file yet.

use bevy::ecs::component::Component;
use bevy::ecs::message::Message;

/// `ANI_SIT_DOWN` — the fold-down transition.
pub const ANIM_TYPE_SIT_DOWN: u32 = 13;
/// `ANI_SIT` — the looping seated pose.
pub const ANIM_TYPE_SIT: u32 = 14;
/// `ANI_STAND_UP` — the get-up transition.
pub const ANIM_TYPE_STAND_UP: u32 = 15;

/// The three clips a body needs before it may sit at all, in chain order.
pub const SIT_CHAIN: [u32; 3] = [ANIM_TYPE_SIT_DOWN, ANIM_TYPE_SIT, ANIM_TYPE_STAND_UP];

/// Where a sitting body is in the chain. Absent component = standing.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SitPhase {
    /// `SIT_DOWN` is playing once.
    SittingDown,
    /// `SIT` is looping.
    Sitting,
    /// `STAND_UP` is playing once; the component is removed when it ends.
    StandingUp,
}

/// "Toggle sit/stand" — the original's `UIIT_CTL_TOG_SIT_STAND`
///.
///
/// A message rather than a keypress read here on purpose: the action window
/// already owns the wire side of command 1000
/// (`hud::action` sends `CharacterActionRequest::sit_stand()`), and the key
/// binding `KeySitStand` (3014) lives in the settings key map. Both are owned
/// by other files; this is the seam they raise.
#[derive(Message)]
pub struct SitStandToggle;

/// What a toggle does, given the phase the body is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SitAction {
    /// Start the chain: play `SIT_DOWN`.
    SitDown,
    /// Leave it: play `STAND_UP`.
    StandUp,
    /// A transition is already running — the original refuses too
    /// (`forbidden 0x0040` on state 6, and state 3 is refused while dead).
    Ignore,
}

/// The original's toggle, as a total function of the current phase.
///
/// Only the two *stable* phases react: standing (no component) sits down,
/// looping `SIT` stands up. A body mid-fold or mid-rise ignores the toggle —
/// in the original that falls out of the transition table rather than an
/// `if`, because entering 6 while in 6 is forbidden and entering 3 requires
/// the sit state it is about to clear.
pub fn toggle_action(phase: Option<SitPhase>) -> SitAction {
    match phase {
        None => SitAction::SitDown,
        Some(SitPhase::Sitting) => SitAction::StandUp,
        Some(SitPhase::SittingDown) | Some(SitPhase::StandingUp) => SitAction::Ignore,
    }
}

/// The phase that follows when the currently playing clip has finished.
///
/// `SittingDown` -> `Sitting` (the loop), `StandingUp` -> `None` (the chain is
/// over and the movement path takes the body back). `Sitting` loops forever,
/// so it never "finishes" — it is only left by a toggle.
pub fn phase_after_clip(phase: SitPhase) -> Option<SitPhase> {
    match phase {
        SitPhase::SittingDown => Some(SitPhase::Sitting),
        SitPhase::Sitting => Some(SitPhase::Sitting),
        SitPhase::StandingUp => None,
    }
}

/// The phase's index into [`SIT_CHAIN`] (and into any array built from it).
///
/// Written out rather than `phase as usize`, so the order of the enum's
/// variants is not load-bearing.
pub fn phase_index(phase: SitPhase) -> usize {
    match phase {
        SitPhase::SittingDown => 0,
        SitPhase::Sitting => 1,
        SitPhase::StandingUp => 2,
    }
}

/// The animation type a phase plays.
pub fn phase_anim_type(phase: SitPhase) -> u32 {
    match phase {
        SitPhase::SittingDown => ANIM_TYPE_SIT_DOWN,
        SitPhase::Sitting => ANIM_TYPE_SIT,
        SitPhase::StandingUp => ANIM_TYPE_STAND_UP,
    }
}

/// Whether the phase's clip loops (only the seated pose does).
pub fn phase_loops(phase: SitPhase) -> bool {
    matches!(phase, SitPhase::Sitting)
}

// What ends the chain from outside, and why there is no `moving` case here:
//
// * **Dying** does (`clear 0xffcf` on state 1 clears bit 6), and the ECS half
//   removes the phase when the wrapper turns into a corpse.
// * **Moving** cannot reach a seated body at all in our build: a sitting
//   wrapper keeps `OneShotAttack` and the looping `SIT` never finishes, so
//   `player::drive_character_animation` never gets to switch the clip. That
//   matches the original's outcome (it stands up *before* it walks — entering
//   state 3 is what clears bit 6, `clear 0x8050`) but not its mechanism:
//   there, a movement order first enters `UPRIGHT`, which plays `STAND_UP`.
//   Ours simply refuses to move until the player stands up. Stated as a
//   deviation rather than hidden, per ADR-0009; the honest fix is a movement
//   order that raises the toggle, which needs the input seam the module doc
//   names.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_stable_phases_react_to_the_toggle() {
        assert_eq!(toggle_action(None), SitAction::SitDown);
        assert_eq!(toggle_action(Some(SitPhase::Sitting)), SitAction::StandUp);
        // mid-transition the original's table refuses the state change
        assert_eq!(
            toggle_action(Some(SitPhase::SittingDown)),
            SitAction::Ignore
        );
        assert_eq!(toggle_action(Some(SitPhase::StandingUp)), SitAction::Ignore);
    }

    #[test]
    fn the_chain_runs_down_up_and_ends() {
        assert_eq!(
            phase_after_clip(SitPhase::SittingDown),
            Some(SitPhase::Sitting)
        );
        // the seated pose loops: it is never finished by itself
        assert_eq!(phase_after_clip(SitPhase::Sitting), Some(SitPhase::Sitting));
        assert!(phase_loops(SitPhase::Sitting));
        assert!(!phase_loops(SitPhase::SittingDown));
        assert!(!phase_loops(SitPhase::StandingUp));
        // and standing up is the end of it
        assert_eq!(phase_after_clip(SitPhase::StandingUp), None);
    }

    /// The ids are the data's, not ours.
    #[test]
    fn the_chain_uses_the_original_ids() {
        assert_eq!(SIT_CHAIN, [13, 14, 15]);
        // the index and the type must not drift apart
        for phase in [
            SitPhase::SittingDown,
            SitPhase::Sitting,
            SitPhase::StandingUp,
        ] {
            assert_eq!(SIT_CHAIN[phase_index(phase)], phase_anim_type(phase));
        }
        assert_eq!(phase_anim_type(SitPhase::SittingDown), 13);
        assert_eq!(phase_anim_type(SitPhase::Sitting), 14);
        assert_eq!(phase_anim_type(SitPhase::StandingUp), 15);
    }
}
