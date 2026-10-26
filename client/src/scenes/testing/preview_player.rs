//! The preview player the window previews share.
//!
//! IDEA. A preview that needs a character must not assume a neighbour spawned
//! one — `UI_ONLY=<name>` runs exactly one preview, and the world map opened
//! with no player sits on the north-west corner of its sheet, which reads like
//! a map that failed to load. So the player is owned here and registered once,
//! by `add_ui_previews`, for any combination of previews that needs it.
//!
//! It used to be a helper each preview called with an `existing` query as the
//! idempotence check, and that check cannot work: the previews' spawns are
//! sibling `OnEnter` systems, so their commands are only applied at the
//! schedule's sync point and none of them can see another's player. Every
//! caller therefore found an empty query and spawned its own. Measured in the
//! 2026-10-25 gallery run (all previews at once): `Camera order ambiguities
//! detected for active cameras with the following priorities: {(-1,
//! Some(Image(..)))}` — several portrait cameras, one portrait image — plus
//! two `warning[B0004]` for the one preview player that was spawned without a
//! transform. One owner removes the class, where a second query could not.

use bevy::prelude::*;

use crate::plugins::player::{spawn_player_character, PlayerConfig};
use crate::scenes::SceneState;

/// Spawns the one preview player. Registered by `add_ui_previews` when any
/// preview that needs a character is active — never by the previews
/// themselves, so there is exactly one spawner however many are registered.
pub struct PreviewPlayerPlugin;

impl Plugin for PreviewPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(SceneState::UiTesting), spawn_preview_player);
    }
}

fn spawn_preview_player(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    config: Res<PlayerConfig>,
) {
    spawn_player_character(&mut commands, &asset_server, &config, Transform::IDENTITY);
}
