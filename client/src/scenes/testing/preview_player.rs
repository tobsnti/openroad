//! The preview player every window preview can ask for.
//!
//! IDEA. Before `UI_ONLY` existed, `SCENE=ui_testing` registered every preview
//! at once, and three of them quietly shared one player: the mini-info preview
//! spawned it, and the minimap and world map read it. Registering a single
//! preview exposed that coupling — the world map opened with no player, so the
//! canvas never centred and sat on the north-west corner of the sheet, which
//! reads exactly like a map that failed to load.
//!
//! So a preview that needs a player asks for one here instead of assuming a
//! neighbour spawned it. The call is idempotent: with every preview registered
//! the first one wins and the rest are no-ops, which keeps the combined scene
//! behaving as it did.

use bevy::prelude::*;

use crate::plugins::player::{spawn_player_character, Player, PlayerConfig};

/// Spawn the preview player unless one already exists.
pub fn ensure_preview_player(
    commands: &mut Commands,
    asset_server: &AssetServer,
    config: &PlayerConfig,
    existing: &Query<(), With<Player>>,
) {
    if !existing.is_empty() {
        return;
    }
    spawn_player_character(commands, asset_server, config, Transform::IDENTITY);
}
