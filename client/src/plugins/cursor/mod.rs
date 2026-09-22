use crate::plugins::cos::{interacts_as_character, CosEntity};
use crate::plugins::cursor::interactions::entity_select::HoveredEntity;
use crate::plugins::net::entities::RemoteEntity;
use bevy::asset::LoadState;
use bevy::prelude::*;
use bevy::window::{CursorIcon, CustomCursor, CustomCursorImage};
use bevy_inspector_egui::bevy_egui::EguiContexts;

pub mod interactions;

/// The camera the cursor ray is cast from. Its ray is what every click
/// consumer reads: entity selection (`interactions::entity_select`) and
/// click-to-move (`plugins::nav::decal`).
#[derive(Component, Default)]
pub struct GameCursorCamera {
    pub(crate) cursor_ray: Option<Ray3d>,
}

/// Which cursor image is (or should be) shown, resolved from the hover target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CursorKind {
    Default,
    /// Sword — hovering a monster.
    Attack,
    /// Chat bubble — hovering an NPC.
    Talk,
    /// An item is armed and waiting for the player to click what it acts on
    /// (`hud::inventory::use_on_item` — pet revival and its siblings). Unlike
    /// the other three this is **not** a function of what is hovered: it wins
    /// over them for as long as the item stays armed, which is what makes the
    /// mode visible.
    ///
    /// Note: which slice the amulet uses is a choice of OURS. Media ships eight
    /// unwired cursor slices and no doc says which one the original arms here.
    Targeting,
}

/// Every cursor image this plugin loads, as `(asset path, hotspot)`.
///
/// One table rather than four `load()` calls scattered through
/// [`setup_cursor`]: these paths are files that have to be *shipped*, and a
/// path with no file behind it is not a compile error — it is a
/// `bevy_asset: Path not found` line on every single start, and a cursor that
/// silently never changes. The table is what's walked by
/// `every_cursor_image_is_shipped`.
///
/// Hotspots sit on the sprite's tip, except the amulet, which has none — it
/// points at its own gem.
const CURSOR_IMAGES: [(&str, (u16, u16)); 4] = [
    ("cursors/Cursor3.png", (1, 2)),
    ("cursors/cursor_attack.png", (2, 2)),
    ("cursors/cursor_talk.png", (2, 2)),
    ("cursors/cursor_targeting.png", (13, 14)),
];

/// The custom cursor images (sliced from `.raw/Cursors.png` into
/// `assets/cursors/`), loaded once at startup.
#[derive(Resource)]
struct GameCursors {
    default: Handle<Image>,
    attack: Handle<Image>,
    talk: Handle<Image>,
    targeting: Handle<Image>,
}

impl GameCursors {
    /// Image + click-point hotspot per kind.
    fn get(&self, kind: CursorKind) -> (&Handle<Image>, (u16, u16)) {
        match kind {
            CursorKind::Default => (&self.default, CURSOR_IMAGES[0].1),
            CursorKind::Attack => (&self.attack, CURSOR_IMAGES[1].1),
            CursorKind::Talk => (&self.talk, CURSOR_IMAGES[2].1),
            CursorKind::Targeting => (&self.targeting, CURSOR_IMAGES[3].1),
        }
    }
}

pub struct CursorPlugin;

impl Plugin for CursorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, setup_cursor)
            .add_systems(Update, (update_cursor_icon, move_cursor))
            .add_plugins(interactions::CursorInteractionsPlugin)
            .add_plugins(interactions::entity_select::EntitySelectionPlugin)
            .add_plugins(interactions::npcs::NpcInteractionPlugin);
    }
}

fn setup_cursor(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(GameCursors {
        default: asset_server.load(CURSOR_IMAGES[0].0),
        attack: asset_server.load(CURSOR_IMAGES[1].0),
        talk: asset_server.load(CURSOR_IMAGES[2].0),
        targeting: asset_server.load(CURSOR_IMAGES[3].0),
    });
}

/// Swap the OS cursor with the hover context: sword over monsters, chat bubble
/// over NPCs, the plain arrow otherwise. Only touches the window when the
/// wanted kind changes (and its image finished loading) — also handles the
/// initial arrow apply on startup, since `applied` starts as `None`.
///
/// "NPC" here means a *dialog* NPC. A COS spawns as one and is not: see
/// [`interacts_as_character`].
fn update_cursor_icon(
    windows: Query<Entity, With<Window>>,
    cursors: Res<GameCursors>,
    asset_server: Res<AssetServer>,
    hovered: Res<HoveredEntity>,
    kinds: Query<&RemoteEntity>,
    cos: Query<(), With<CosEntity>>,
    armed: Option<Res<crate::plugins::hud::inventory::use_on_item::PendingItemUse>>,
    mut applied: Local<Option<CursorKind>>,
    mut commands: Commands,
) {
    // An armed item outranks the hover: the whole point of the mode is that
    // the cursor stops describing what is under it and starts describing what
    // the next click will do.
    let wanted = if armed.is_some_and(|armed| armed.is_armed()) {
        CursorKind::Targeting
    } else {
        match hovered.0 {
            Some(entity) => match kinds.get(entity) {
                // Characters get the plain cursor — and a COS counts as one,
                // so the talk cursor does not appear over somebody's pet and
                // promise a dialog that has no business existing.
                Ok(kind) if interacts_as_character(kind, cos.contains(entity)) => {
                    CursorKind::Default
                }
                Ok(RemoteEntity::Monster) => CursorKind::Attack,
                Ok(RemoteEntity::Npc) => CursorKind::Talk,
                _ => CursorKind::Default,
            },
            None => CursorKind::Default,
        }
    };
    if *applied == Some(wanted) {
        return;
    }
    let (handle, hotspot) = cursors.get(wanted);
    if !matches!(asset_server.load_state(handle), LoadState::Loaded) {
        return;
    }
    let Ok(window_entity) = windows.single() else {
        return;
    };
    commands
        .entity(window_entity)
        .insert(CursorIcon::Custom(CustomCursor::Image(CustomCursorImage {
            handle: handle.clone(),
            hotspot,
            ..default()
        })));
    *applied = Some(wanted);
}

fn move_cursor(
    windows: Query<&Window>,
    mut cursor_cameras_query: Query<(&mut GameCursorCamera, &Camera, &GlobalTransform)>,
    mut egui_contexts: EguiContexts,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let position = window.cursor_position();

    // Suppress world interactions while the pointer is over an egui window
    // (debug panels): otherwise a click on the UI would fall through and, e.g.,
    // move the player. Leaving the ray `None` disables every ray-based cursor
    // interaction for this frame.
    let over_egui = egui_contexts
        .ctx_mut()
        .map(|ctx| ctx.is_pointer_over_egui() || ctx.egui_wants_pointer_input())
        .unwrap_or(false);

    for (mut game_cursor_camera, camera, camera_transform) in cursor_cameras_query.iter_mut() {
        if !camera.is_active || over_egui {
            game_cursor_camera.cursor_ray = None;
            continue;
        }

        let Some(position) = position else {
            game_cursor_camera.cursor_ray = None;
            continue;
        };

        game_cursor_camera.cursor_ray = camera.viewport_to_world(camera_transform, position).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::CURSOR_IMAGES;
    use std::path::PathBuf;

    /// Every path in [`CURSOR_IMAGES`] must name a file that is actually
    /// shipped under `assets/`.
    ///
    /// This is the only place the mistake is catchable: a missing cursor image
    /// compiles, links and runs — `AssetServer::load` just logs
    /// `Path not found: assets/cursors/<name>` once per start and hands back a
    /// handle that never reaches `LoadState::Loaded`, so
    /// [`update_cursor_icon`](super::update_cursor_icon) returns early forever
    /// and that cursor kind is simply invisible.
    #[test]
    fn every_cursor_image_is_shipped() {
        let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets");
        for (path, _) in CURSOR_IMAGES {
            let file = assets.join(path);
            assert!(
                file.is_file(),
                "cursor image `{path}` is loaded at startup but not shipped ({})",
                file.display()
            );
        }
    }

    /// A hotspot outside the image is a click point the OS cannot honour, and
    /// the cursors are 32x32 crops.
    #[test]
    fn every_cursor_hotspot_is_inside_a_32px_sprite() {
        for (path, (x, y)) in CURSOR_IMAGES {
            assert!(x < 32 && y < 32, "`{path}` hotspot {x},{y} is off-sprite");
        }
    }
}
