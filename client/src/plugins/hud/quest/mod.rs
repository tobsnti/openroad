//! Quest HUD: the journal's state and its two on-screen surfaces.
//!
//! Idea: this tree has carried the quest **wire** for a while — the
//! active-quest record inside `CHARACTER_DATA` and the `0x30D5` progress
//! packet ([`packets::agent::quest`]) — and it landed the record on the local
//! player as `CharacterInfo::active_quests` with "no consumer yet" written in
//! its own doc comment. That is the Dead-Wire shape this module closes, in the
//! order the tree insists on: **reception -> state -> visible effect**.
//!
//! * [`model`] — fills [`QuestJournal`] from the login record, folds `0x30D5`,
//!   and owns [`model::TrackedQuests`].
//! * [`mini_list`] — draws the tracked rows as the original's
//!   `questminilist.2dt` strip: the only quest surface the original keeps on
//!   screen while every window is closed.
//! * [`journal`] — the journal **window** (`questlist.2dt`, ten rows with the
//!   per-row track checkbox), opened by the original's own `KeyQuest` binding
//!   (id 3006, Q). It is the operator the tracked set would otherwise lack:
//!   without it `TrackedQuests` could only be seeded and emptied, and six of
//!   the nine quests a live login carries would have no surface at all. Wiring
//!   it takes 3006 out of `settings/keymap.rs`'s `NOT_YET_WIRED` list — the Key
//!   Map tab had been advertising a Q that did nothing.
//!
//! There is no **send** here, deliberately: accept, abandon and hand-in have
//! builders in the original but no evidenced client->server body, and an
//! invented request is worse than a missing one (ADR-0009).
//!
//! Registration note: [`QuestJournal`](crate::plugins::net::quest::QuestJournal)
//! is defined in the net tree but registered **here**, by the only plugin that
//! consumes it. The net side deliberately registers nothing (its systems must
//! run in the headless netcheck harness, AGENTS.md), and an unregistered
//! resource behind a non-optional `Res` panics the schedule on entering the
//! world rather than at compile time.

pub mod journal;
pub mod mini_list;
pub mod model;

use bevy::prelude::*;

use crate::plugins::net::quest::QuestJournal;
use crate::scenes::SceneState;

/// The quest journal's state and its on-screen tracker.
pub struct QuestPlugin;

impl Plugin for QuestPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<QuestJournal>()
            .init_resource::<model::TrackedQuests>()
            .init_resource::<journal::QuestJournalWindow>()
            // Gated on the HUD scenes because `sync_quest_mini_list` takes
            // `Res<FontAssets>`, which exists only there — an ungated HUD
            // system with a scene resource fails Bevy's parameter validation
            // and panics the schedule during the loading screen, which neither
            // `cargo test` nor `make ci` can see (AGENTS.md: `make ci` never
            // starts the app). The two model systems are gated with it because
            // there is no journal to keep outside the world either.
            .add_systems(
                Update,
                (
                    model::fill_journal_at_login,
                    model::apply_quest_updates,
                    mini_list::sync_quest_mini_list,
                    // The hotkey runs before the sync so a press is visible in
                    // the same frame; it takes `Res<ChatState>` and
                    // `Res<GameOptions>`, so it is gated with the rest.
                    journal::quest_journal_hotkey,
                    journal::sync_quest_journal_window,
                )
                    .chain()
                    .run_if(super::hud_scenes),
            )
            .add_systems(
                OnExit(SceneState::GameWorld),
                (
                    mini_list::cleanup_quest_mini_list,
                    journal::cleanup_quest_journal,
                    reset_quest_state,
                ),
            );
    }
}

/// Leaving the world drops the character's quests with it — the next login
/// sends its own `CHARACTER_DATA`, and a stale journal would decorate the next
/// character with the previous one's quests.
fn reset_quest_state(mut journal: ResMut<QuestJournal>, mut tracked: ResMut<model::TrackedQuests>) {
    *journal = QuestJournal::default();
    *tracked = model::TrackedQuests::default();
}

#[cfg(test)]
mod test {
    /// The start-trap assertion: the registration text itself must show the
    /// gate, because no test that does not boot the app can observe a
    /// missing-resource panic.
    #[test]
    fn the_quest_hud_is_gated_on_the_hud_scenes() {
        let registration = include_str!("mod.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("split always yields a first part");
        assert!(
            registration.contains("run_if(super::hud_scenes)"),
            "a HUD system taking `Res<FontAssets>` must be scene-gated"
        );
        assert!(
            registration.contains("init_resource::<QuestJournal>()"),
            "the journal resource must be registered by the plugin that reads it"
        );
        assert!(
            registration.contains("OnExit(SceneState::GameWorld)"),
            "the strip and the journal must not survive leaving the world"
        );
        // The journal window takes `Res<FontAssets>` and `Res<ClientUiStrings>`
        // as well, so it has to be inside the *same* gated tuple — not merely
        // somewhere in a file that also contains the gate.
        let gated = registration
            .split("run_if(super::hud_scenes)")
            .next()
            .expect("split always yields a first part");
        assert!(
            gated.contains("journal::sync_quest_journal_window"),
            "the journal window's system must be inside the gated tuple"
        );
        assert!(
            registration.contains("journal::cleanup_quest_journal"),
            "the journal window must be despawned and closed on leaving"
        );
    }
}
