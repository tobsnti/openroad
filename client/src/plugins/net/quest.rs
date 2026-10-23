//! Quest marks (`0x30D6` / `0x30D7`) — the map-pin half of the quest family.
//!
//! Idea: the wire never names a *quest*. It describes the mark's **target
//! spawn record** — region, the target's position truncated to whole units,
//! and the target entity's `unique_id` (the capture join that proves this is
//! in [`QuestMarkAdd`]'s own docs). So this keeps exactly what the server
//! said, keyed by the handle the remove packet names, and leaves projecting it
//! onto a map to the HUD (`hud::world_map::model::quest_markers`). That split
//! is the one `PartyRoster` already uses, and it is why this plugin touches no
//! HUD resource: net systems must run headless (AGENTS.md).
//!
//! The icon state (`QUEST_MARK_*`) is deliberately not kept. The world map and
//! minimap each have exactly one quest sign in the user's PK2
//! (`wmap_sign_questnpc.ddj`, `mm_sign_questnpc.ddj`), so nothing could select
//! a variant from it without inventing the mapping; a journal consumer that
//! needs the state reads the packet.

use std::collections::HashMap;

use bevy::prelude::*;
use packets::agent::quest::{QuestMarkAdd, QuestMarkRemove};

/// Where one server-placed mark points, in the server's own frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuestMarkTarget {
    pub region: u16,
    pub x: u32,
    /// Truncated toward zero, so a target below zero arrives wrapped — read it
    /// back through `as i32` (see [`QuestMarkAdd`]).
    pub y: u32,
    pub z: u32,
    /// The target entity's unique id, the key
    /// [`NetworkEntities`](super::entities::NetworkEntities) indexes spawns by.
    pub target_unique_id: u32,
}

/// The marks the server currently has placed, keyed by mark handle.
#[derive(Resource, Default, Debug)]
pub struct QuestMarks(pub HashMap<u32, QuestMarkTarget>);

/// `0x30D6` — place a mark under its handle.
///
/// Insert, not push: the server re-adds the same target under a *new* handle
/// (114 distinct handles for one target across the capture), so the handle is
/// the identity and only its own `0x30D7` retires it.
pub fn on_quest_mark_add(mut reader: MessageReader<QuestMarkAdd>, mut marks: ResMut<QuestMarks>) {
    for add in reader.read() {
        marks.0.insert(
            add.mark_id,
            QuestMarkTarget {
                region: add.region,
                x: add.x,
                y: add.y,
                z: add.z,
                target_unique_id: add.target_unique_id,
            },
        );
    }
}

/// `0x30D7` — drop the mark with this handle.
pub fn on_quest_mark_remove(
    mut reader: MessageReader<QuestMarkRemove>,
    mut marks: ResMut<QuestMarks>,
) {
    for remove in reader.read() {
        marks.0.remove(&remove.mark_id);
    }
}

/// Owns [`QuestMarks`]; part of the networking core so the marks exist
/// wherever quest-mark packets can arrive — they land in the same frame as the
/// login spawns, before any HUD is up.
pub struct QuestMarkPlugin;

impl Plugin for QuestMarkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<QuestMarks>()
            .add_systems(Update, (on_quest_mark_add, on_quest_mark_remove).chain());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use packets::agent::quest::QUEST_MARK_NEW;

    /// The literal `0x30D6` body of 2026-08-10T18:19:22.552Z and the
    /// `0x30D7` body that retires the same handle.
    fn captured_add() -> QuestMarkAdd {
        QuestMarkAdd {
            mark_id: 0x52e647e8,
            unknown0: 0x82,
            mark: QUEST_MARK_NEW,
            region: 25000,
            x: 1584,
            y: 0,
            z: 1407,
            target_unique_id: 243,
        }
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_message::<QuestMarkAdd>()
            .add_message::<QuestMarkRemove>()
            .add_plugins(QuestMarkPlugin);
        app
    }

    #[test]
    fn an_added_mark_is_kept_under_its_handle_and_a_remove_retires_it() {
        let mut app = app();

        app.world_mut().write_message(captured_add());
        app.update();
        assert_eq!(
            app.world().resource::<QuestMarks>().0.get(&0x52e647e8),
            Some(&QuestMarkTarget {
                region: 25000,
                x: 1584,
                y: 0,
                z: 1407,
                target_unique_id: 243,
            })
        );

        app.world_mut().write_message(QuestMarkRemove {
            mark_id: 0x52e647e8,
        });
        app.update();
        assert!(app.world().resource::<QuestMarks>().0.is_empty());
    }

    /// A remove for a handle we never saw is not an error and must not touch
    /// the live marks — the capture has 47 removes and every one named an
    /// earlier add, but the stream can start mid-session.
    #[test]
    fn a_remove_for_an_unknown_handle_leaves_the_other_marks_alone() {
        let mut app = app();
        app.world_mut().write_message(captured_add());
        app.update();

        app.world_mut()
            .write_message(QuestMarkRemove { mark_id: 0xdead });
        app.update();
        assert_eq!(app.world().resource::<QuestMarks>().0.len(), 1);
    }
}
