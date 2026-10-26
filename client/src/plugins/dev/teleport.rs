//! A dev egui window that jumps the player to a handful of hardcoded world
//! positions. Mirrors the other dev windows (see `player_config.rs`).
//!
//! Each target is a server-style (region, x/y/z) triple — the same form the
//! network spawn path uses — so it goes through [`server_position_to_sro`] and
//! gets the identical mirror/region handling as a real spawn.
//!
//! Unlike the network teleport, this re-anchors the floating world origin on
//! the target ([`set_world_origin`]) before placing the player. The targets are
//! tens of regions apart (Jangan to Takla), well past the ~8.5-region f32
//! precision budget the origin keeps small (see `world_origin`); without the
//! re-anchor the character would tremble and the render coordinates would lose
//! resolution. Re-anchoring shifts the already-loaded terrain to hold its world
//! placement, and the camera-driven streamer fills in around the new position.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

use crate::plugins::map::terrain::Terrain;
use crate::plugins::nav::NavLocation;
use crate::plugins::player::{Player, PlayerCommands};
use crate::plugins::textdata::{ClientOptionalTeleports, ClientTextNames};
use crate::plugins::world_origin::{set_world_origin, WorldOrigin};
use crate::scenes::game_scene::server_position_to_sro;
use crate::scenes::SceneState;

/// One teleport destination, in server (region + region-local x/y/z) form.
struct TeleportTarget {
    label: &'static str,
    region: u16,
    x: f32,
    y: f32,
    z: f32,
}

/// The game's own destinations, read from `refoptionalteleport.txt` through
/// [`ClientOptionalTeleports`] and labelled from the row's `SN_ZONE_*` key.
///
/// Measured over the user's own file (2026-10-26): **32 rows, 31 in service**,
/// each with a zone key that resolves in `textdata_object.txt`
/// (`SN_ZONE_22004` → "Constantinople"). A client of another version ships its
/// own table, which is the point of reading it instead of transcribing it.
///
/// A row whose key does not resolve keeps the key as its label rather than
/// disappearing: a missing string is a data gap worth seeing, not a reason to
/// hide a destination.
fn data_targets(
    teleports: &ClientOptionalTeleports,
    names: &ClientTextNames,
) -> Vec<(String, u16, f32, f32, f32)> {
    teleports
        .active()
        .into_iter()
        .map(|row| {
            let label = names
                .name(&row.zone_key)
                .unwrap_or(row.zone_key.as_str())
                .to_string();
            (label, row.region, row.x, row.y, row.z)
        })
        .collect()
}

/// Spots this project measured for itself — not in any table, and not the
/// game's. They stay because they are what a renderer check needs (the splat
/// codes, the water gate), and the data list cannot carry them.
const TARGETS: &[TeleportTarget] = &[
    TeleportTarget {
        label: "Jangan West",
        region: 24999,
        x: 435.0,
        y: 0.0,
        z: 1745.0,
    },
    TeleportTarget {
        label: "Takla Bridge Top",
        region: 25991,
        x: 1374.0,
        y: -28.0,
        z: 937.0,
    },
    TeleportTarget {
        label: "Talkla Bridge Bottom",
        region: 26246,
        x: 939.0,
        y: -522.0,
        z: 992.0,
    },
    TeleportTarget {
        // From the position readout at the Karakoram water gate (region
        // 129.69 x 92.39). `y` is the camera height off that readout, not the
        // player's feet, so the character drops to the water surface on its
        // first step (the local player only ground-snaps while moving).
        label: "Karakoram",
        region: 23681,
        x: 1331.6,
        y: 877.3,
        z: 748.0,
    },
    // Splat-scale verification spots (docs/formats/mapm-jmxvmapm.md): the
    // tiling codes 24/32 map to repeat factors 2.0/4.0 by the ×2-per-+8
    // progression, never visually confirmed against vanilla. These are the
    // cluster centroids from `probe_splat_scale_census` (mean terrain height
    // as `y`); compare the ground-texture tiling density here in both
    // clients — if ours is right, these patches tile noticeably FINER than
    // the surrounding code-16 ground (2× resp. 4×), not coarser.
    TeleportTarget {
        label: "Splat code 24 (west, biggest)",
        region: 24147,
        x: 973.0,
        y: -431.0,
        z: 914.0,
    },
    TeleportTarget {
        label: "Splat code 24 (east)",
        region: 26773,
        x: 1042.0,
        y: 44.0,
        z: 1024.0,
    },
    TeleportTarget {
        label: "Splat code 32 (a)",
        region: 25244,
        x: 772.0,
        y: 66.0,
        z: 616.0,
    },
    TeleportTarget {
        label: "Splat code 32 (b)",
        region: 25761,
        x: 1363.0,
        y: 82.0,
        z: 1297.0,
    },
];

pub struct TeleportPlugin;

impl Plugin for TeleportPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            // egui UI must build inside the multipass context pass — in Update
            // it would render but never receive input.
            EguiPrimaryContextPass,
            teleport_window
                .run_if(in_state(SceneState::WorldSandbox))
                .run_if(super::dev_windows_visible),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn teleport_window(
    mut contexts: EguiContexts,
    mut selected: Local<usize>,
    mut origin: ResMut<WorldOrigin>,
    mut player: Query<(&mut Transform, &mut NavLocation), With<Player>>,
    mut terrain: Query<&mut Transform, (With<Terrain>, Without<Player>)>,
    mut commands: ResMut<PlayerCommands>,
    teleports: Res<ClientOptionalTeleports>,
    names: Res<ClientTextNames>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    // The game's destinations first, our own measured spots after them, so the
    // list reads as "where the game can send you" plus a debug tail.
    let mut entries = data_targets(&teleports, &names);
    entries.extend(
        TARGETS
            .iter()
            .map(|t| (t.label.to_string(), t.region, t.x, t.y, t.z)),
    );

    egui::Window::new("Teleport")
        .default_open(false)
        .show(ctx, |ui| {
            *selected = (*selected).min(entries.len().saturating_sub(1));

            egui::ComboBox::from_label("Destination")
                .selected_text(
                    entries
                        .get(*selected)
                        .map(|e| e.0.as_str())
                        .unwrap_or_default(),
                )
                .show_ui(ui, |ui| {
                    for (i, entry) in entries.iter().enumerate() {
                        ui.selectable_value(&mut *selected, i, entry.0.as_str());
                    }
                });

            ui.separator();

            if ui.button("Teleport").clicked() {
                let Some(&(_, region, x, y, z)) = entries.get(*selected) else {
                    return;
                };
                let sro = server_position_to_sro(region, x, y, z);

                // Re-anchor first so the target lands at small render-space
                // coordinates, then place the player at the re-based position.
                set_world_origin(sro, &mut origin, &mut terrain);
                if let Ok((mut transform, mut nav_location)) = player.single_mut() {
                    transform.translation = origin.to_render(sro);
                    // Discontinuous move: the tracked nav surface is meaningless
                    // now and must be re-resolved geometrically (see ADR 0007).
                    *nav_location = NavLocation::Unresolved;
                }
                // Drop any in-flight click-to-move so the player doesn't walk
                // back toward the pre-teleport target.
                commands.stop();
            }
        });
}
