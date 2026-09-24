//! Minimap (top-right): a 3x3 grid of per-region terrain tiles centered on
//! the player, entity dots, an area-name/coordinate readout and zoom buttons.
//!
//! Idea: the layout is hand-transcribed from the vanilla
//! `Media.pk2/resinfo/ifminimap.txt` (like the mini-info panel transcribes
//! ifplayerminiinfo.txt), uniformly scaled. The map viewport is a square
//! `overflow: clip` node; the `mm_window.ddj` frame drawn on top is opaque
//! everywhere except its circular hole, which turns the square map into the
//! vanilla round minimap for free. Terrain tiles are `minimap/{x}x{z}.ddj`,
//! one 256px image per 1920-unit region, re-pointed when the player crosses a
//! region border; dots and tile offsets are recomputed per frame from the SRO
//! world positions (mind the mirrored render X, see `world_origin`).
//!
//! The tile/dot/arrow children carry per-frame-written absolute positions, so
//! they are spawned programmatically ([`populate_minimap_viewport`]) instead
//! of via bsn; sibling paint order inside the viewport is fixed with `ZIndex`
//! (tiles 0, dots 1, player arrow 2).

use bevy::asset::LoadState;
use bevy::log::warn_once;
use bevy::prelude::*;
use bevy::ui::{Overflow, UiTargetCamera, UiTransform};
use bevy::ui_widgets::Activate;

use crate::assets::dof::JMXVDOF;
use crate::assets::textdata::worldmap::DUNGEON_CENTER_REGION;
use crate::assets::FontAssets;
use crate::plugins::assets::sro::MediaArchive;
use crate::plugins::dungeon::ActiveDungeon;
use crate::plugins::hud::game_window::scaled;
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::hud::world_map::model::{MapMarkers, MarkerKind};
use crate::plugins::map::terrain::REGION_SIZE;
use crate::plugins::net::entities::{DisplayName, RemoteEntity, UniqueMonster};
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientWorldMap, ClientZoneNames};
use crate::plugins::ui_v2::style::ImageButtonStyle;
use crate::plugins::ui_v2::widgets::{image_button, label};
use crate::plugins::world_origin::WorldOrigin;

// --- Layout constants (resinfo/ifminimap.txt, window space) -----------------

/// Window placement, verbatim from `resinfo/ginterface.txt:807`
/// (`GDR_MINIMAP:CIFMinimap`, ID 10): `Rect=RECT,"892,6,140,184"`, authored
/// against the vanilla 1024x768 canvas. 892 + 140 = 1032, so the window
/// overhangs the right screen edge by 8px — harmless, because mm_window.ddj's
/// rightmost 11 columns are fully transparent, so visible art still ends at
/// x=1020. Anchored from the right to preserve that overhang at any
/// resolution.
const WINDOW_RECT: (f32, f32, f32, f32) = (892.0, 6.0, 140.0, 184.0);
/// Width of the design canvas the resinfo rects are authored against.
const DESIGN_SCREEN_W: f32 = 1024.0;

/// mm_window.ddj frame art size (= the window rect's extent).
const FRAME_W: f32 = WINDOW_RECT.2;
const FRAME_H: f32 = WINDOW_RECT.3;
/// Top and right screen margins in unscaled px; the right one is negative,
/// which is the 8px overhang described above.
const WINDOW_TOP: f32 = WINDOW_RECT.1;
const WINDOW_RIGHT: f32 = DESIGN_SCREEN_W - WINDOW_RECT.0 - WINDOW_RECT.2;

// Element rects (x, y, w, h) in window space, verbatim from the resinfo.
const VIEWPORT_RECT: (f32, f32, f32, f32) = (14.0, 57.0, 105.0, 105.0);

// --- The round mask ----------------------------------------------------------
//
// Idea: the map viewport is a square, and the frame drawn on top is NOT an
// opaque plate with a hole — `mm_window.ddj` is transparent over 13811 of its
// 25760 px, because its outer silhouette is round too. Inside the viewport
// rect the frame is transparent at **210 px that are not part of the map
// disc**, all of them in rows 145..161 where the round silhouette curves back
// inside the square. That is exactly the reported defect: the map's bottom
// corners stick out past the ring.
//
// bevy 0.19 cannot clip a circle: `CalculatedClip` is a `Rect`
// (`bevy_ui-0.19.0/src/ui_node.rs:2409`, written in `update.rs:78`) and
// `BorderRadius` only rounds a node's *own* surface, never its children — so
// a rounded viewport would leave the nine tile children square. An overlay
// cannot help either: the leaking pixels sit *outside* the window silhouette,
// where the world must show through, and UI cannot erase.
//
// What the art itself offers instead: the disc admits an **exact two-rectangle
// cover**. In the art (`mm_alpha.ddj`, 104x104, 7999 opaque px, matching the
// frame's hole at offset (14,58) with zero differing pixels), the disc is
// 101x101 at (14,58) in frame
// space. Rows 57..145 of the square carry **no** leaking pixel at all,
// and rows 145..159 need only x 29..100 to hold every disc
// pixel while touching no leaking one. Two clip rects, both derived from the
// alpha channel, hence no invented number — and nothing of the map is drawn
// outside the ring any more.
const VIEWPORT_BANDS: [(f32, f32, f32, f32); 2] =
    [(14.0, 57.0, 105.0, 88.0), (29.0, 145.0, 71.0, 14.0)];
/// The disc the two bands cover, in frame space: centre and radius of
/// `mm_alpha.ddj`'s opaque area (101x101 at (14,58)). Used to cull markers, so
/// a dot can never land on one of the 210 leaking pixels. The map itself stays
/// centred on the *rect* centre (66.5, 109.5) — §9-U1b is an open question
/// about the art, not something to settle by taste here.
const DISC_CENTER: (f32, f32) = (64.0, 108.0);
const DISC_RADIUS: f32 = 50.5;
/// The markers (dots, party signs, player arrow) live in one node spanning the
/// whole disc, clipped to the square: 57..159 is where the disc has pixels.
const MARKER_LAYER_RECT: (f32, f32, f32, f32) = (14.0, 57.0, 105.0, 102.0);
const AREA_NAME_RECT: (f32, f32, f32, f32) = (12.0, 9.0, 104.0, 12.0);
// `GDR_MINIMAP_TEXT_POS_X` (ifminimap.txt:63) and `..._TEXT_POS_Y` (:44): the
// original labels the second readout **Y**, though it carries world Z.
const POS_X_RECT: (f32, f32, f32, f32) = (8.0, 32.0, 56.0, 11.0);
const POS_Z_RECT: (f32, f32, f32, f32) = (67.0, 32.0, 56.0, 11.0);
const ZOOM_IN_RECT: (f32, f32, f32, f32) = (107.0, 136.0, 20.0, 20.0);
const ZOOM_OUT_RECT: (f32, f32, f32, f32) = (90.0, 152.0, 20.0, 20.0);
const MAP_BUTTON_RECT: (f32, f32, f32, f32) = (99.0, 47.0, 24.0, 24.0);
/// `GDR_MINIMAP_DUNGEON_FLOOR_INFO` — the floor badge shown inside dungeons.
const FLOOR_BADGE_RECT: (f32, f32, f32, f32) = (1.0, 42.0, 32.0, 32.0);

const FRAME_TEXTURE: &str = "media://interface/minimap/mm_window.ddj";
const SIGN_CHARACTER: &str = "media://interface/minimap/mm_sign_character.ddj";
const SIGN_MONSTER: &str = "media://interface/minimap/mm_sign_monster.ddj";
const SIGN_NPC: &str = "media://interface/minimap/mm_sign_npc.ddj";
const SIGN_OTHER_PLAYER: &str = "media://interface/minimap/mm_sign_otherplayer.ddj";
const SIGN_UNIQUE: &str = "media://interface/minimap/mm_sign_unique.ddj";
/// The party pair, both previously unused art (the resinfo lists 13 sign
/// textures, 8 of them undrawn). Sizes are the DDS headers'
/// in the user's own PK2: the dot is 8x8 like every other dot, the arrow 16x16
/// like `mm_sign_character`.
const SIGN_PARTY: &str = "media://interface/minimap/mm_sign_party.ddj";
const SIGN_PARTY_ARROW: &str = "media://interface/minimap/mm_sign_partyarrow.ddj";

/// Sign art sizes (window-space px): 8x8 dots, 12x12 unique, 16x16 arrow.
const DOT_SIZE: f32 = 8.0;
const UNIQUE_DOT_SIZE: f32 = 12.0;
const ARROW_SIZE: f32 = 16.0;

/// AREA_NAME FontColor from the resinfo (ARGB 255,239,218,164).
const AREA_NAME_COLOR: Color = Color::srgb_u8(239, 218, 164);
// UNKNOWN: the resinfo carries only
// `FontIndex=0` and never a size or a face, so both sizes below are invented.
const AREA_FONT_SIZE: f32 = 10.0;
const POS_FONT_SIZE: f32 = 9.0;

// --- Zoom ---------------------------------------------------------------------
//
// Idea: vanilla's minimap zoom is NOT a step table over discrete levels — it
// is one continuous float "window-space px per 1920-unit region" that eases
// towards a click-set target. `CIFMinimap` keeps the pair as two adjacent
// members: the scale actually drawn and the target the buttons move. The four
// constants below are the literals the original's ctor, its two zoom-button
// handlers and its per-frame ease use: ctor 160.0;
// zoom-in `target += 19.2` clamped to 256.0; zoom-out
// `target -= 19.2` clamped to 64.0; ease `scale += frame_ms * 0.05` toward the
// target. The drawn scale is what the tile/marker placement multiplies with
// (`x/1920 * scale`), so the unit is settled: px per region, exactly our `p`
// below.
//
// Internal consistency check (not a second source, but it closes): the range
// is exactly ten clicks — `64 + 10*19.2 = 256` — and the default sits exactly
// on click five, `64 + 5*19.2 = 160`. That is why the previous invented table
// `[64,128,256,384,512]` with a 256 default was wrong at the top by 2x: 256
// is vanilla's *maximum* (the tiles' native resolution, i.e. 1:1 is the most
// the client will ever show), never a middle step, and 384/512 magnify a
// 256px tile past 1:1, which vanilla never does.
/// Zoom-out clamp, the original's literal.
/// It also satisfies our own coverage bound: the 3x3 grid must still cover
/// the 101x101 px viewport disc with
/// the player on a region border, i.e. `1.5 * p >= 50.5` -> `p >= 33.7`.
const ZOOM_MIN: f32 = 64.0;
/// Zoom-in clamp, the original's literal = one tile drawn
/// 1:1 (`minimap/*.ddj` are 256x256 per region).
const ZOOM_MAX: f32 = 256.0;
/// One click, the original's literal: 19.2 px per region.
const ZOOM_STEP: f32 = 19.2;
/// Ctor default, the original's literal.
const ZOOM_DEFAULT: f32 = 160.0;
/// Easing speed of the drawn scale towards the target: vanilla adds
/// `frame_ms * 0.05` px per frame, i.e. 50 px/s, frame-rate independent
/// because the multiplier is the frame time. Expressed per second here
/// because Bevy hands us `delta_secs`.
const ZOOM_EASE_PX_PER_SEC: f32 = 50.0;

const DOT_POOL_SIZE: usize = 64;
/// A separate pool for party signs, sized to the party cap of 8
/// (`PartySetup::capacity()` — 8 when EXP is shared, else 4). They cannot
/// share the entity-dot pool: a member out of view still draws, as a rim
/// arrow, whereas an entity out of view is simply culled.
const PARTY_POOL_SIZE: usize = 8;

/// Displayed-coordinate origin: game coords are world units / 10, offset so
/// that 0 sits at region x 135 / z 92 (the vanilla sector formula).
///
/// Two independent sides agree on the pair:
/// - the wire: a position block for region 24744 (regionX 168 / regionZ 96,
///   x = 1286.0, z = 1230.0, little endian) reproduces the original's head
///   `X:6464` / `Y:891` with exactly these two offsets, and no other pair
///   fits a second position in another region (25000).
/// - the original's placement routine writes
///   `(regionZ * 3 - 0x114) * 0x40`, i.e. `(regionZ - 92) * 192`, because
///   `0x114 = 276 = 92 * 3` and `0x40 * 3 = 192` — so `offZ = 92` is a literal
///   of the original. `offX = 135` rests on the wire side alone.
const COORD_X_OFFSET: f32 = 135.0 * 192.0;
const COORD_Z_OFFSET: f32 = 92.0 * 192.0;

// --- State ------------------------------------------------------------------

#[derive(Resource, Clone, Debug)]
pub struct MinimapState {
    /// The scale actually drawn, in window-space px per 1920-unit region
    /// (the original's drawn scale). Eased towards [`MinimapState::zoom_target`]
    /// by [`ease_minimap_zoom`].
    pub zoom: f32,
    /// What the zoom buttons set (the original's zoom target).
    pub zoom_target: f32,
    /// Region sector (x, z) the tile grid is currently centered on; `None`
    /// until the first player position is seen.
    pub center_region: Option<(i32, i32)>,
}

impl Default for MinimapState {
    fn default() -> Self {
        Self {
            zoom: ZOOM_DEFAULT,
            zoom_target: ZOOM_DEFAULT,
            center_region: None,
        }
    }
}

/// Move the drawn scale towards the clicked target, the way vanilla does it
/// (its minimap repaint does the same): a fixed px-per-millisecond crawl, not an
/// interpolation factor, and clamped to the target on the step that would
/// overshoot it. Vanilla runs this from the same function that repaints the
/// tiles; we run it as its own system in front of them.
pub fn ease_minimap_zoom(time: Res<Time>, mut state: ResMut<MinimapState>) {
    let delta = state.zoom_target - state.zoom;
    if delta == 0.0 {
        return;
    }
    let step = ZOOM_EASE_PX_PER_SEC * time.delta_secs();
    let zoom = if delta.abs() <= step {
        state.zoom_target
    } else {
        state.zoom + step * delta.signum()
    };
    state.zoom = zoom;
}

/// Entity-dot sign textures, loaded once at spawn.
#[derive(Resource)]
pub struct MinimapAssets {
    monster: Handle<Image>,
    npc: Handle<Image>,
    other_player: Handle<Image>,
    unique: Handle<Image>,
    party: Handle<Image>,
    party_arrow: Handle<Image>,
}

// --- Markers ----------------------------------------------------------------

#[derive(Component, Default, Clone)]
pub struct MinimapRoot;
/// One clipped map band; its terrain-tile children are added by
/// [`populate_minimap_viewport`]. There are two, indexing [`VIEWPORT_BANDS`]:
/// together they are the round map hole, which a single rect cannot be.
#[derive(Component, Default, Clone)]
pub struct MinimapViewport {
    pub band: usize,
}
/// The clipped square the markers (dots, party signs, player arrow) live in.
/// Separate from the tile bands because the markers are culled to the disc in
/// code and would otherwise have to be drawn once per band.
#[derive(Component, Default, Clone)]
pub struct MinimapMarkerLayer;
/// Marks a viewport or marker layer whose children have been populated.
#[derive(Component)]
pub struct MinimapViewportReady;
/// One of the 3x3 terrain tile slots, at grid offset (dx, dz) from the
/// player's region, in the band it was spawned into.
#[derive(Component)]
pub struct MinimapTile {
    dx: i8,
    dz: i8,
    band: usize,
}
/// One slot of the entity-dot pool.
#[derive(Component)]
pub struct MinimapDot;
/// One slot of the party-sign pool. Separate from [`MinimapDot`] because a
/// party member outside the viewport still draws (clamped to the rim as an
/// arrow) instead of being culled.
#[derive(Component)]
pub struct MinimapPartySign;
#[derive(Component)]
pub struct MinimapArrow;
#[derive(Component, Default, Clone)]
pub struct MinimapAreaText;
#[derive(Component, Default, Clone)]
pub struct MinimapPosXText;
#[derive(Component, Default, Clone)]
pub struct MinimapPosZText;
/// The dungeon floor badge art (`mm_dungeonfloor.ddj`), hidden outside
/// dungeons.
#[derive(Component, Default, Clone)]
pub struct MinimapFloorBadge;
/// The floor-number text on the badge (`1F` / `B3`).
#[derive(Component, Default, Clone)]
pub struct MinimapFloorText;

// --- Dungeon floor context --------------------------------------------------

/// Present while the minimap should draw dungeon floor tiles instead of the
/// overworld grid. Kept in sync by [`sync_minimap_dungeon_context`].
#[derive(Resource, Clone, PartialEq)]
pub struct MinimapDungeonContext {
    pub region_id: u16,
    /// Lowercased floor label of the player's current block — the
    /// `minimap_d` tile filename stem.
    pub floor: String,
    /// `minimap_d/<group>` directory holding the floor's tiles; `None` when
    /// the archive ships none (tiles hide, fail-soft).
    pub group: Option<String>,
    /// `(basement, number)` for the badge, from the Dungeonmap rows.
    pub floor_number: Option<(bool, u32)>,
}

/// Floor-string (lowercased) → `minimap_d` group directory, discovered once
/// from the Media archive listing (`minimap_d/<group>/<floor>_<x>x<z>.ddj`).
/// This resolves the `<group>` UNKNOWN of `docs/formats/minimap.md`
/// operationally — the mapping lives nowhere in the data tables, but the
/// archive layout itself is authoritative.
#[derive(Resource, Default)]
pub struct MinimapDungeonGroups(pub std::collections::HashMap<String, String>);

fn build_dungeon_groups(archive: &bevy_pk2::prelude::Archive) -> MinimapDungeonGroups {
    let mut groups = std::collections::HashMap::new();
    for (path, entry) in archive.root.get_all_entries() {
        if !entry.is_file() {
            continue;
        }
        let lower = path.to_string_lossy().replace('\\', "/").to_lowercase();
        let Some(rest) = lower.strip_prefix("minimap_d/") else {
            continue;
        };
        let (Some((group, file)), true) = (rest.split_once('/'), rest.ends_with(".ddj")) else {
            continue;
        };
        // `dh_a01_floor01_127x126.ddj` → floor stem before the `_<x>x<z>`.
        let stem = file.trim_end_matches(".ddj");
        let Some((floor, _coords)) = stem.rsplit_once('_') else {
            continue;
        };
        groups
            .entry(floor.to_string())
            .or_insert_with(|| group.to_string());
    }
    info!(
        "minimap: indexed {} dungeon floor tile sets under minimap_d/",
        groups.len()
    );
    MinimapDungeonGroups(groups)
}

/// Keep [`MinimapDungeonContext`] in sync with the active dungeon and the
/// player's current floor; entering/leaving/floor changes reset the tile
/// grid's center so the tiles re-point.
#[allow(clippy::too_many_arguments)]
pub fn sync_minimap_dungeon_context(
    active: Option<Res<ActiveDungeon>>,
    dofs: Res<Assets<JMXVDOF>>,
    worldmap: Res<ClientWorldMap>,
    media: Option<Res<MediaArchive>>,
    groups: Option<Res<MinimapDungeonGroups>>,
    context: Option<Res<MinimapDungeonContext>>,
    mut state: ResMut<MinimapState>,
    mut commands: Commands,
) {
    let Some(active) = active else {
        if context.is_some() {
            commands.remove_resource::<MinimapDungeonContext>();
            state.center_region = None;
        }
        return;
    };
    let Some(groups) = groups else {
        // Lazy one-time index; lands as a resource next frame.
        if let Some(media) = media {
            commands.insert_resource(build_dungeon_groups(&media.0));
        }
        return;
    };

    // Current floor: the player's block's floor label; single-floor
    // dungeons without labels fall back to their lone Dungeonmap row.
    let floor = dofs
        .get(&active.dof)
        .and_then(|dof| {
            active
                .current_block
                .and_then(|index| dof.blocks.get(index))
                .and_then(|block| dof.floor_names.get(block.floor_index as usize))
                .map(|name| name.to_lowercase())
        })
        .or_else(|| {
            worldmap.table().and_then(|table| {
                let floors = table.dungeon_floors(active.region_id);
                (floors.len() == 1).then(|| floors[0].floor_string.to_lowercase())
            })
        });
    let Some(floor) = floor else {
        if context.is_some() {
            commands.remove_resource::<MinimapDungeonContext>();
            state.center_region = None;
        }
        return;
    };

    let floor_number = worldmap
        .table()
        .and_then(|table| table.dungeon_floor(active.region_id, &floor))
        .map(|def| (def.basement, def.floor_number));
    let new = MinimapDungeonContext {
        region_id: active.region_id,
        group: groups.0.get(&floor).cloned(),
        floor,
        floor_number,
    };
    if context.as_deref() != Some(&new) {
        state.center_region = None;
        commands.insert_resource(new);
    }
}

/// Show the floor badge + number while a dungeon floor is displayed.
pub fn update_minimap_floor_badge(
    context: Option<Res<MinimapDungeonContext>>,
    mut badges: Query<&mut Visibility, (With<MinimapFloorBadge>, Without<MinimapFloorText>)>,
    mut texts: Query<(&mut Text, &mut Visibility), With<MinimapFloorText>>,
) {
    let target = if context.is_some() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in badges.iter_mut() {
        if *visibility != target {
            *visibility = target;
        }
    }
    let label = context
        .as_ref()
        .map(|ctx| match ctx.floor_number {
            Some((true, n)) => format!("B{n}"),
            Some((false, n)) => format!("{n}F"),
            // No Dungeonmap row (e.g. donwhang_event): derive from the
            // floor string's trailing digits, else stay blank.
            None => ctx
                .floor
                .rsplit(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|digits| digits.parse::<u32>().ok())
                .map(|n| format!("{n}F"))
                .unwrap_or_default(),
        })
        .unwrap_or_default();
    for (mut text, mut visibility) in texts.iter_mut() {
        if text.0 != label {
            text.0 = label.clone();
        }
        if *visibility != target {
            *visibility = target;
        }
    }
}

// --- Spawn / cleanup --------------------------------------------------------

pub fn spawn_minimap(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    mut state: ResMut<MinimapState>,
    cam_query: Query<Entity, With<Camera2d>>,
) {
    let Some(camera) = cam_query.iter().next() else {
        warn!("no 2d camera found for the minimap");
        return;
    };

    // fresh session state (re-entry must re-point the tile grid)
    *state = MinimapState::default();

    commands.insert_resource(MinimapAssets {
        monster: asset_server.load(SIGN_MONSTER),
        npc: asset_server.load(SIGN_NPC),
        other_player: asset_server.load(SIGN_OTHER_PLAYER),
        unique: asset_server.load(SIGN_UNIQUE),
        party: asset_server.load(SIGN_PARTY),
        party_arrow: asset_server.load(SIGN_PARTY_ARROW),
    });

    commands
        .spawn_scene(minimap(&asset_server, &fonts))
        .insert(UiTargetCamera(camera));
}

pub fn cleanup_minimap(mut commands: Commands, roots: Query<Entity, With<MinimapRoot>>) {
    for entity in roots.iter() {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<MinimapAssets>();
}

fn minimap(asset_server: &AssetServer, fonts: &FontAssets) -> impl Scene {
    let frame: Handle<Image> = asset_server.load(FRAME_TEXTURE);
    let floor_badge: Handle<Image> =
        asset_server.load("media://interface/minimap/mm_dungeonfloor.ddj");
    let zoom_in_style = button_style(asset_server, "mm_zoomin");
    let zoom_out_style = button_style(asset_server, "mm_zoomout");
    let map_button_style = button_style(asset_server, "mm_map_button");

    let area_font = fonts.nine.clone();
    let x_font = fonts.nine.clone();
    let z_font = fonts.nine.clone();
    let floor_font = fonts.nine.clone();

    let s = hud_scale();
    let (b0_l, b0_t, b0_w, b0_h) = scaled(VIEWPORT_BANDS[0], s);
    let (b1_l, b1_t, b1_w, b1_h) = scaled(VIEWPORT_BANDS[1], s);
    let (ml_l, ml_t, ml_w, ml_h) = scaled(MARKER_LAYER_RECT, s);
    let (an_l, an_t, an_w, an_h) = scaled(AREA_NAME_RECT, s);
    let (px_l, px_t, px_w, px_h) = scaled(POS_X_RECT, s);
    let (pz_l, pz_t, pz_w, pz_h) = scaled(POS_Z_RECT, s);
    let (zi_l, zi_t, zi_w, zi_h) = scaled(ZOOM_IN_RECT, s);
    let (zo_l, zo_t, zo_w, zo_h) = scaled(ZOOM_OUT_RECT, s);
    let (mb_l, mb_t, mb_w, mb_h) = scaled(MAP_BUTTON_RECT, s);
    let (fb_l, fb_t, fb_w, fb_h) = scaled(FLOOR_BADGE_RECT, s);

    bsn! {
        MinimapRoot
        Name("Minimap")
        Node {
            position_type: PositionType::Absolute,
            top: px(WINDOW_TOP * s),
            right: px(WINDOW_RIGHT * s),
            width: px(FRAME_W * s),
            height: px(FRAME_H * s),
        }
        // above the world, below the loading overlay (200)
        GlobalZIndex(50)
        Pickable::IGNORE
        Children [
            // The map, in the two bands that add up to the frame's
            // round hole (see VIEWPORT_BANDS). The black backdrop shows where
            // tiles are missing (world edge) or still streaming in.
            (
                MinimapViewport { band: 0 }
                Node {
                    position_type: PositionType::Absolute,
                    left: px(b0_l),
                    top: px(b0_t),
                    width: px(b0_w),
                    height: px(b0_h),
                    overflow: {Overflow::clip()},
                }
                BackgroundColor(Color::BLACK)
                Pickable::IGNORE
            ),
            (
                MinimapViewport { band: 1 }
                Node {
                    position_type: PositionType::Absolute,
                    left: px(b1_l),
                    top: px(b1_t),
                    width: px(b1_w),
                    height: px(b1_h),
                    overflow: {Overflow::clip()},
                }
                BackgroundColor(Color::BLACK)
                Pickable::IGNORE
            ),
            // the markers on top of both bands, in one square layer
            (
                MinimapMarkerLayer
                Node {
                    position_type: PositionType::Absolute,
                    left: px(ml_l),
                    top: px(ml_t),
                    width: px(ml_w),
                    height: px(ml_h),
                    overflow: {Overflow::clip()},
                }
                Pickable::IGNORE
            ),
            // the frame on top: opaque ring around a transparent circular
            // hole -> the square map reads as the vanilla round minimap
            (
                ImageNode { image: {frame}, image_mode: NodeImageMode::Stretch }
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px(0),
                    width: px(FRAME_W * s),
                    height: px(FRAME_H * s),
                }
                Pickable::IGNORE
            ),
            (
                label("", area_font, AREA_FONT_SIZE * s)
                MinimapAreaText
                TextColor({AREA_NAME_COLOR})
                Node { position_type: PositionType::Absolute, left: px(an_l), top: px(an_t), width: px(an_w), height: px(an_h) }
            ),
            (
                label("", x_font, POS_FONT_SIZE * s)
                MinimapPosXText
                TextColor(Color::WHITE)
                Node { position_type: PositionType::Absolute, left: px(px_l), top: px(px_t), width: px(px_w), height: px(px_h) }
            ),
            (
                label("", z_font, POS_FONT_SIZE * s)
                MinimapPosZText
                TextColor(Color::WHITE)
                Node { position_type: PositionType::Absolute, left: px(pz_l), top: px(pz_t), width: px(pz_w), height: px(pz_h) }
            ),
            (
                image_button(zoom_in_style, zi_w, zi_h)
                Node { position_type: PositionType::Absolute, left: px(zi_l), top: px(zi_t) }
                on(|_activate: On<Activate>, mut state: ResMut<MinimapState>| {
                    // vanilla: target += 19.2, clamped at 256
                    state.zoom_target = (state.zoom_target + ZOOM_STEP).min(ZOOM_MAX);
                })
            ),
            (
                image_button(zoom_out_style, zo_w, zo_h)
                Node { position_type: PositionType::Absolute, left: px(zo_l), top: px(zo_t) }
                on(|_activate: On<Activate>, mut state: ResMut<MinimapState>| {
                    // vanilla: target -= 19.2, clamped at 64
                    state.zoom_target = (state.zoom_target - ZOOM_STEP).max(ZOOM_MIN);
                })
            ),
            // dungeon floor badge (art + number), shown only inside dungeons
            (
                MinimapFloorBadge
                ImageNode { image: {floor_badge}, image_mode: NodeImageMode::Stretch }
                Node {
                    position_type: PositionType::Absolute,
                    left: px(fb_l),
                    top: px(fb_t),
                    width: px(fb_w),
                    height: px(fb_h),
                }
                Visibility::Hidden
                Pickable::IGNORE
            ),
            (
                label("", floor_font, POS_FONT_SIZE * s)
                MinimapFloorText
                TextColor(Color::WHITE)
                Node {
                    position_type: PositionType::Absolute,
                    left: px(fb_l),
                    top: px(fb_t),
                    width: px(fb_w),
                    height: px(fb_h),
                }
                Visibility::Hidden
            ),
            // world-map toggle (same behavior as the M key)
            (
                image_button(map_button_style, mb_w, mb_h)
                Node { position_type: PositionType::Absolute, left: px(mb_l), top: px(mb_t) }
                on(|_activate: On<Activate>,
                    origin: Res<crate::plugins::world_origin::WorldOrigin>,
                    worldmap: Res<crate::plugins::textdata::ClientWorldMap>,
                    dungeon: Option<Res<MinimapDungeonContext>>,
                    player: Query<&Transform, With<Player>>,
                    mut state: ResMut<crate::plugins::hud::world_map::model::WorldMapState>| {
                    if state.open {
                        state.open = false;
                    } else {
                        crate::plugins::hud::world_map::model::open_for_player_region(
                            &origin, &worldmap, &player, dungeon.as_deref(), &mut state,
                        );
                    }
                })
            ),
        ]
    }
}

fn button_style(asset_server: &AssetServer, stem: &str) -> ImageButtonStyle {
    ImageButtonStyle {
        normal: asset_server.load(format!("media://interface/minimap/{stem}.ddj")),
        hover: asset_server.load(format!("media://interface/minimap/{stem}_focus.ddj")),
        press: asset_server.load(format!("media://interface/minimap/{stem}_press.ddj")),
        ..Default::default()
    }
}

/// Add the per-frame-positioned children to a fresh viewport: 9 terrain tile
/// slots, the dot pool and the player arrow. Polling, because the scene's
/// entities only exist after a command flush.
pub fn populate_minimap_viewport(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    viewports: Query<(Entity, &MinimapViewport), Without<MinimapViewportReady>>,
    layers: Query<Entity, (With<MinimapMarkerLayer>, Without<MinimapViewportReady>)>,
) {
    let s = hud_scale();
    let h = VIEWPORT_RECT.2 / 2.0;
    // each band draws the whole 3x3 grid and shows the slice its clip admits
    for (viewport, band) in viewports.iter() {
        let band = band.band;
        commands
            .entity(viewport)
            .insert(MinimapViewportReady)
            .with_children(|parent| {
                for dz in -1..=1i8 {
                    for dx in -1..=1i8 {
                        parent.spawn((
                            MinimapTile { dx, dz, band },
                            ImageNode {
                                image_mode: NodeImageMode::Stretch,
                                ..default()
                            },
                            Node {
                                position_type: PositionType::Absolute,
                                ..default()
                            },
                            Visibility::Hidden,
                            ZIndex(0),
                            Pickable::IGNORE,
                        ));
                    }
                }
            });
    }
    for layer in layers.iter() {
        let arrow: Handle<Image> = asset_server.load(SIGN_CHARACTER);
        commands
            .entity(layer)
            .insert(MinimapViewportReady)
            .with_children(|parent| {
                for _ in 0..DOT_POOL_SIZE {
                    parent.spawn((
                        MinimapDot,
                        ImageNode {
                            image_mode: NodeImageMode::Stretch,
                            ..default()
                        },
                        Node {
                            position_type: PositionType::Absolute,
                            ..default()
                        },
                        Visibility::Hidden,
                        ZIndex(1),
                        Pickable::IGNORE,
                    ));
                }
                for _ in 0..PARTY_POOL_SIZE {
                    parent.spawn((
                        MinimapPartySign,
                        ImageNode {
                            image_mode: NodeImageMode::Stretch,
                            ..default()
                        },
                        Node {
                            position_type: PositionType::Absolute,
                            ..default()
                        },
                        // above the entity dots: a party member must not be
                        // hidden under a monster dot on the same pixel
                        ZIndex(2),
                        Visibility::Hidden,
                        Pickable::IGNORE,
                    ));
                }
                parent.spawn((
                    MinimapArrow,
                    ImageNode {
                        image: arrow,
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px((h - ARROW_SIZE / 2.0) * s),
                        top: Val::Px((h - ARROW_SIZE / 2.0) * s),
                        width: Val::Px(ARROW_SIZE * s),
                        height: Val::Px(ARROW_SIZE * s),
                        ..default()
                    },
                    UiTransform::default(),
                    ZIndex(2),
                    Pickable::IGNORE,
                ));
            });
    }
}

// --- Per-frame updates ------------------------------------------------------

/// Server-global (gx, gz) of the local player, in world units. The render
/// world mirrors X relative to the region grid (`server_position_to_render`),
/// hence the negation.
fn player_global_xz(origin: &WorldOrigin, transform: &Transform) -> (f32, f32) {
    let sro = origin.to_sro(transform.translation);
    (-sro.x, sro.z)
}

/// Position (and, on a region change, re-point) the 3x3 terrain tiles so the
/// map stays centered on the player. East (+region x) runs right, north
/// (+region z) up.
pub fn update_minimap_tiles(
    mut state: ResMut<MinimapState>,
    origin: Res<WorldOrigin>,
    asset_server: Res<AssetServer>,
    dungeon: Option<Res<MinimapDungeonContext>>,
    player: Query<&Transform, With<Player>>,
    mut tiles: Query<(&MinimapTile, &mut Node, &mut ImageNode, &mut Visibility)>,
) {
    let Ok(player_tf) = player.single() else {
        return;
    };
    if tiles.is_empty() {
        return;
    }

    let (mut gx, mut gz) = player_global_xz(&origin, player_tf);
    if dungeon.is_some() {
        // Inside a dungeon (gx, gz) are raw dungeon-local coordinates; the
        // minimap_d tile grid lives in a sector space centred on (128,128).
        gx += DUNGEON_CENTER_REGION * REGION_SIZE;
        gz += DUNGEON_CENTER_REGION * REGION_SIZE;
    }
    let xsec = (gx / REGION_SIZE).floor() as i32;
    let ysec = (gz / REGION_SIZE).floor() as i32;
    let repoint = state.center_region != Some((xsec, ysec));
    if repoint {
        state.center_region = Some((xsec, ysec));
    }

    let s = hud_scale();
    let h = VIEWPORT_RECT.2 / 2.0;
    let p = state.zoom;
    let k = p / REGION_SIZE;
    // A dungeon floor with no shipped tile set shows the black backdrop.
    let no_dungeon_tiles = dungeon.as_ref().is_some_and(|ctx| ctx.group.is_none());

    for (tile, mut node, mut image, mut visibility) in tiles.iter_mut() {
        let tx = xsec + tile.dx as i32;
        let tz = ysec + tile.dz as i32;
        let in_range = (0..=255).contains(&tx) && (0..=255).contains(&tz) && !no_dungeon_tiles;
        if repoint && in_range {
            let path = match &dungeon {
                Some(ctx) => {
                    let group = ctx.group.as_deref().unwrap_or_default();
                    format!("media://minimap_d/{group}/{}_{tx}x{tz}.ddj", ctx.floor)
                }
                None => format!("media://minimap/{tx}x{tz}.ddj"),
            };
            image.image = asset_server.load(path);
        }

        // tile top edge = its region's north edge. Positions are in the
        // square viewport's space, so a band that starts elsewhere in the
        // frame shifts them by its own origin — both bands then show the same
        // map through different clips.
        let (band_x, band_y) = (
            VIEWPORT_BANDS[tile.band].0 - VIEWPORT_RECT.0,
            VIEWPORT_BANDS[tile.band].1 - VIEWPORT_RECT.1,
        );
        let left = Val::Px((h + (tx as f32 * REGION_SIZE - gx) * k - band_x) * s);
        let top = Val::Px((h - ((tz + 1) as f32 * REGION_SIZE - gz) * k - band_y) * s);
        let size = Val::Px(p * s);
        if node.left != left {
            node.left = left;
        }
        if node.top != top {
            node.top = top;
        }
        if node.width != size {
            node.width = size;
            node.height = size;
        }

        // hide world-edge tiles and tiles whose image doesn't exist (the
        // viewport's black backdrop shows through)
        let failed = matches!(
            asset_server.get_load_state(&image.image),
            Some(LoadState::Failed(_))
        );
        let target = if in_range && !failed {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != target {
            *visibility = target;
        }
    }
}

/// The displayed game coordinates: world units / 10, origin at region 135/92
/// (so Jangan reads ~6400 like the vanilla client).
pub fn update_minimap_coords(
    origin: Res<WorldOrigin>,
    dungeon: Option<Res<MinimapDungeonContext>>,
    player: Query<&Transform, With<Player>>,
    mut x_texts: Query<&mut Text, (With<MinimapPosXText>, Without<MinimapPosZText>)>,
    mut z_texts: Query<&mut Text, (With<MinimapPosZText>, Without<MinimapPosXText>)>,
) {
    let Ok(player_tf) = player.single() else {
        return;
    };
    let (gx, gz) = player_global_xz(&origin, player_tf);
    let (disp_x, disp_z) = display_coords(gx, gz, dungeon.is_some());

    let set_text = |text: &mut Text, value: String| {
        if text.0 != value {
            text.0 = value;
        }
    };
    for mut text in x_texts.iter_mut() {
        set_text(&mut text, coord_text('X', disp_x));
    }
    for mut text in z_texts.iter_mut() {
        set_text(&mut text, coord_text('Y', disp_z));
    }
}

/// The two integers behind the coordinate head. The original does **not**
/// round: it composes `(region - offset) * 192` (an integer) with the int
/// conversion of the negated tenth-coordinate, and that conversion truncates
/// (in the original: `-1286.0/10 = -128.6 -> -128`, so the head
/// reads `128`, and `Y:891` is `768 + trunc(1230/10)`).
/// We only ever see the *global*
/// world coordinate, so the equivalent operation on the composed value is
/// `floor`, not `trunc`: the local tenth is never negative, so truncating it is
/// flooring it, whereas truncating a *negative display value* (regions west or
/// north of the 135/92 origin) would land one unit off.
fn display_coords(gx: f32, gz: f32, in_dungeon: bool) -> (i32, i32) {
    // Dungeon-local coordinates display as-is (no overworld origin offset).
    if in_dungeon {
        ((gx / 10.0).floor() as i32, (gz / 10.0).floor() as i32)
    } else {
        (
            (gx / 10.0 - COORD_X_OFFSET).floor() as i32,
            (gz / 10.0 - COORD_Z_OFFSET).floor() as i32,
        )
    }
}

/// One coordinate readout, e.g. `"X:6400"`. The second axis is labelled **Y**
/// because the original element is `GDR_MINIMAP_TEXT_POS_Y`
/// (`resinfo/ifminimap.txt:44`), even though the value it shows is world Z.
///
/// The vanilla format strings are `L"X:%3d"` and `L"Y:%3d"`
/// (the original's placement routine): colon **without** a space,
/// field width 3 padded on the left, `%d` so negatives carry a minus sign, and
/// no thousands separator.
fn coord_text(axis: char, value: i32) -> String {
    format!("{axis}:{value:>3}")
}

/// Area name of the player's region, from textzonename.txt. Inside a
/// dungeon the dungeon's own region id names the whole interior (textzonename
/// stores dungeon ids in the same u16 space, e.g. 32769 = Donwhang Stone
/// Cave); the sector-derived lookup only applies to the overworld.
pub fn update_minimap_area_name(
    state: Res<MinimapState>,
    dungeon: Option<Res<MinimapDungeonContext>>,
    zone_names: Res<ClientZoneNames>,
    mut texts: Query<&mut Text, With<MinimapAreaText>>,
) {
    let region = match &dungeon {
        Some(ctx) => ctx.region_id,
        None => {
            let Some((xsec, ysec)) = state.center_region else {
                return;
            };
            if !(0..=255).contains(&xsec) || !(0..=255).contains(&ysec) {
                return;
            }
            ((ysec as u16) << 8) | (xsec as u16)
        }
    };
    let name = zone_names.name(region).unwrap_or("");
    for mut text in texts.iter_mut() {
        if text.0 != name {
            text.0 = name.to_string();
        }
    }
}

/// Rotate the center arrow to the player's heading. The arrow art points east
/// at zero rotation; `phi` is the server-space heading angle (0 = east,
/// counterclockwise looking down with north up), negated because UI rotation
/// runs clockwise on screen (y down).
pub fn update_minimap_arrow(
    player: Query<&Transform, With<Player>>,
    mut arrows: Query<&mut UiTransform, With<MinimapArrow>>,
) {
    let Ok(player_tf) = player.single() else {
        return;
    };
    // the model faces -Z at identity; un-mirror render X into server space
    let facing = player_tf.rotation * Vec3::NEG_Z;
    let phi = facing.z.atan2(-facing.x);
    let rotation = Rot2::radians(-phi);
    for mut ui_transform in arrows.iter_mut() {
        if ui_transform.rotation != rotation {
            ui_transform.rotation = rotation;
        }
    }
}

/// Project remote entities into the viewport and write them into the dot
/// pool. Decoded from the sign art: monsters red (255,0,0), NPCs blue
/// (65,131,255), other players yellow-green (189,230,0) and uniques purple
/// (156,0,255) at 12x12 — every other dot is 8x8.
pub fn update_minimap_dots(
    state: Res<MinimapState>,
    origin: Res<WorldOrigin>,
    assets: Option<Res<MinimapAssets>>,
    player: Query<&Transform, With<Player>>,
    entities: Query<(&Transform, &RemoteEntity, Has<UniqueMonster>)>,
    mut dots: Query<(&mut Node, &mut ImageNode, &mut Visibility), With<MinimapDot>>,
) {
    let Some(assets) = assets else {
        return;
    };
    let Ok(player_tf) = player.single() else {
        return;
    };
    let (gx, gz) = player_global_xz(&origin, player_tf);

    let s = hud_scale();
    let h = VIEWPORT_RECT.2 / 2.0;
    let k = state.zoom / REGION_SIZE;

    let mut visible: Vec<(f32, f32, f32, Handle<Image>)> = Vec::new();
    for (transform, kind, unique) in entities.iter() {
        let (image, size) = match kind {
            RemoteEntity::Item => continue,
            RemoteEntity::Monster if unique => (assets.unique.clone(), UNIQUE_DOT_SIZE),
            RemoteEntity::Monster => (assets.monster.clone(), DOT_SIZE),
            RemoteEntity::Npc => (assets.npc.clone(), DOT_SIZE),
            RemoteEntity::Player => (assets.other_player.clone(), DOT_SIZE),
        };
        let sro = origin.to_sro(transform.translation);
        let (ex, ez) = (-sro.x, sro.z);
        let cx = h + (ex - gx) * k;
        let cy = h - (ez - gz) * k;
        // Cull against the disc, not the square: a dot has to be
        // fully inside `mm_alpha.ddj`'s hole, because the frame is
        // transparent at 210 px of the square's bottom corners and anything
        // drawn there hangs outside the round window. The disc
        // is expressed in the square's space, hence the rect origin offset.
        let (dcx, dcy) = (
            DISC_CENTER.0 - VIEWPORT_RECT.0,
            DISC_CENTER.1 - VIEWPORT_RECT.1,
        );
        let reach = (DISC_RADIUS - size / 2.0).max(0.0);
        if (cx - dcx).powi(2) + (cy - dcy).powi(2) > reach.powi(2) {
            continue;
        }
        visible.push((cx, cy, size, image));
        if visible.len() > DOT_POOL_SIZE {
            warn_once!("minimap: more than {DOT_POOL_SIZE} entities in view, dropping overflow");
            break;
        }
    }

    let mut pending = visible.into_iter();
    for (mut node, mut image_node, mut visibility) in dots.iter_mut() {
        match pending.next() {
            Some((cx, cy, size, image)) => {
                let left = Val::Px((cx - size / 2.0) * s);
                let top = Val::Px((cy - size / 2.0) * s);
                let px_size = Val::Px(size * s);
                if node.left != left {
                    node.left = left;
                }
                if node.top != top {
                    node.top = top;
                }
                if node.width != px_size {
                    node.width = px_size;
                    node.height = px_size;
                }
                if image_node.image != image {
                    image_node.image = image;
                }
                if *visibility != Visibility::Inherited {
                    *visibility = Visibility::Inherited;
                }
            }
            None => {
                if *visibility != Visibility::Hidden {
                    *visibility = Visibility::Hidden;
                }
            }
        }
    }
}

/// Where one party marker draws inside the round viewport.
///
/// Idea: a party member is not an entity dot. An entity outside the hole is
/// simply culled, but a party member out of view is the case the player most
/// wants to see — which is what `mm_sign_partyarrow.ddj` exists for. The RE
/// doc reads the four `*arrow` sign textures as "rim-clamped direction
/// indicators for off-map targets" (speculative reading of the sign art),
/// and that is what this implements: inside the hole the 8x8 dot at the true
/// position, outside it the 16x16 arrow pushed back onto the rim along the
/// same bearing, rotated to point at the member.
///
/// `radius` is the hole's radius; both returned coordinates are viewport-space
/// centres, like [`update_minimap_dots`] produces.
fn party_sign_placement(dx: f32, dy: f32, radius: f32) -> PartySignPlacement {
    let distance = (dx * dx + dy * dy).sqrt();
    // Half the arrow, so a rim-clamped arrow sits fully inside the hole
    // instead of half-way under the frame ring.
    let rim = (radius - ARROW_SIZE / 2.0).max(0.0);
    if distance <= rim {
        return PartySignPlacement {
            dx,
            dy,
            size: DOT_SIZE,
            arrow: false,
            rotation: 0.0,
        };
    }
    // distance > rim >= 0 here, so it is never zero and the scale is finite.
    let scale = rim / distance;
    PartySignPlacement {
        dx: dx * scale,
        dy: dy * scale,
        size: ARROW_SIZE,
        arrow: true,
        // Screen y grows downward, so the bearing is measured against -dy.
        rotation: dx.atan2(-dy),
    }
}

/// Result of [`party_sign_placement`], in viewport-space offsets from centre.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PartySignPlacement {
    dx: f32,
    dy: f32,
    size: f32,
    arrow: bool,
    rotation: f32,
}

/// Draw the party roster on the minimap (EP-14.4).
///
/// Positions come from `world_map::model::party_markers`, which folds
/// `PartyRoster` into `MapMarkers` in the minimap's own mirrored-X global
/// convention (`(-sro.x, sro.z)`), including the rule that a member inside a
/// dungeon has no overworld position and is skipped.
///
/// **With one deliberate exception**: a member who is spawned near us is drawn
/// from their live `Transform` instead. The roster's position is only as fresh
/// as the last 0x3864 position delta — every few seconds — so a member standing
/// beside you visibly lagged their own character. The world map keeps the
/// throttled fold, so the two maps *can* now disagree by a few metres for
/// in-range members; that is the trade, and it is the right way round, because
/// the minimap is the one you read while moving.
///
/// The local player is not among these: 0x3065 sends the *whole* party, so the
/// player is one of its members, and a party sign under their own centre arrow
/// would be a duplicate. That filtering now happens once at the producer
/// (`world_map::model::party_markers`) rather than here — the world map needed
/// it too, and two copies of the same rule is one to forget.
pub fn update_minimap_party_signs(
    state: Res<MinimapState>,
    origin: Res<WorldOrigin>,
    assets: Option<Res<MinimapAssets>>,
    markers: Option<Res<MapMarkers>>,
    player: Query<&Transform, With<Player>>,
    nearby: Query<(&DisplayName, &Transform), (With<RemoteEntity>, Without<Player>)>,
    mut signs: Query<
        (&mut Node, &mut ImageNode, &mut UiTransform, &mut Visibility),
        With<MinimapPartySign>,
    >,
) {
    let (Some(assets), Some(markers)) = (assets, markers) else {
        return;
    };
    let Ok(player_tf) = player.single() else {
        return;
    };
    let (gx, gz) = player_global_xz(&origin, player_tf);

    let s = hud_scale();
    let h = VIEWPORT_RECT.2 / 2.0;
    let k = state.zoom / REGION_SIZE;

    let mut placements: Vec<PartySignPlacement> = Vec::new();
    for marker in markers.0.iter() {
        if marker.kind != MarkerKind::Party {
            continue;
        }
        // Prefer a LIVE position when the member is spawned near us. The
        // roster's position only moves when the server pushes a 0x3864
        // type-6 kind-0x20 delta, every few seconds — which is why the dot
        // visibly trailed a member standing right beside you while your own
        // arrow moved smoothly. The join is the name, the same one the
        // producer uses to leave the local player out.
        let (mx, mz) = nearby
            .iter()
            .find(|(name, _)| name.0 == marker.name)
            .map(|(_, transform)| player_global_xz(&origin, transform))
            .unwrap_or((marker.gx, marker.gz));
        let placement = party_sign_placement((mx - gx) * k, -(mz - gz) * k, h);
        placements.push(placement);
        if placements.len() >= PARTY_POOL_SIZE {
            break;
        }
    }

    let mut pending = placements.into_iter();
    for (mut node, mut image_node, mut ui_transform, mut visibility) in signs.iter_mut() {
        let Some(placement) = pending.next() else {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        };
        let left = Val::Px((h + placement.dx - placement.size / 2.0) * s);
        let top = Val::Px((h + placement.dy - placement.size / 2.0) * s);
        let px_size = Val::Px(placement.size * s);
        if node.left != left {
            node.left = left;
        }
        if node.top != top {
            node.top = top;
        }
        if node.width != px_size {
            node.width = px_size;
            node.height = px_size;
        }
        let image = if placement.arrow {
            assets.party_arrow.clone()
        } else {
            assets.party.clone()
        };
        if image_node.image != image {
            image_node.image = image;
        }
        // The dot is round and must not spin with the bearing.
        let rotation = Rot2::radians(if placement.arrow {
            placement.rotation
        } else {
            0.0
        });
        if ui_transform.rotation != rotation {
            ui_transform.rotation = rotation;
        }
        if *visibility != Visibility::Inherited {
            *visibility = Visibility::Inherited;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// EP-14.4: a member inside the hole is a dot at their true offset; one
    /// outside is the arrow, clamped to the rim on the same bearing. The
    /// clamp uses `radius - ARROW_SIZE/2` so the arrow sits fully inside the
    /// hole rather than half under the frame ring.
    #[test]
    fn a_party_member_out_of_view_becomes_a_rim_arrow_on_the_same_bearing() {
        let h = VIEWPORT_RECT.2 / 2.0;
        let rim = h - ARROW_SIZE / 2.0;

        // well inside: the plain dot, untouched position, no rotation
        let inside = party_sign_placement(10.0, -4.0, h);
        assert!(!inside.arrow);
        assert_eq!((inside.dx, inside.dy), (10.0, -4.0));
        assert_eq!(inside.size, DOT_SIZE);
        assert_eq!(inside.rotation, 0.0);

        // far away due north: clamped to the rim, still due north
        let north = party_sign_placement(0.0, -10_000.0, h);
        assert!(north.arrow);
        assert_eq!(north.size, ARROW_SIZE);
        assert!((north.dx.abs()) < 1e-3);
        assert!(
            (north.dy + rim).abs() < 1e-3,
            "dy {} vs -rim {}",
            north.dy,
            -rim
        );
        // and it never overruns the hole
        assert!((north.dx.powi(2) + north.dy.powi(2)).sqrt() <= h - ARROW_SIZE / 2.0 + 1e-3);

        // the bearing survives the clamp: direction is preserved exactly
        let (dx, dy) = (300.0, 400.0);
        let far = party_sign_placement(dx, dy, h);
        assert!(far.arrow);
        assert!((far.dx / far.dy - dx / dy).abs() < 1e-4);
        assert!(((far.dx.powi(2) + far.dy.powi(2)).sqrt() - rim).abs() < 1e-3);
    }

    /// Screen y grows downward, so a member due north must rotate to 0 and one
    /// due east to +90 degrees. Getting the sign wrong points every arrow the
    /// wrong way, which no compile error catches.
    #[test]
    fn the_arrow_rotation_follows_screen_space_not_world_space() {
        let h = VIEWPORT_RECT.2 / 2.0;
        let north = party_sign_placement(0.0, -1000.0, h);
        assert!(north.rotation.abs() < 1e-4, "north {}", north.rotation);
        let east = party_sign_placement(1000.0, 0.0, h);
        assert!(
            (east.rotation - std::f32::consts::FRAC_PI_2).abs() < 1e-4,
            "east {}",
            east.rotation
        );
        let south = party_sign_placement(0.0, 1000.0, h);
        assert!((south.rotation.abs() - std::f32::consts::PI).abs() < 1e-4);
    }

    /// The party pool is the party cap, and the party art is the pair the RE
    /// doc lists as unused — both sizes are the DDS headers in the user's PK2.
    #[test]
    fn the_party_pool_and_art_match_the_data() {
        // PartySetup::capacity() is 8 when EXP is shared, else 4
        assert_eq!(PARTY_POOL_SIZE, 8);
        assert_eq!(SIGN_PARTY, "media://interface/minimap/mm_sign_party.ddj");
        assert_eq!(
            SIGN_PARTY_ARROW,
            "media://interface/minimap/mm_sign_partyarrow.ddj"
        );
        // mm_sign_party.ddj is 8x8 like every other dot; partyarrow 16x16
        // like mm_sign_character.
        assert_eq!(DOT_SIZE, 8.0);
        assert_eq!(ARROW_SIZE, 16.0);
    }

    /// True if the frame-space point (x, y) is drawn by one of the map bands.
    fn in_a_band(x: f32, y: f32) -> bool {
        VIEWPORT_BANDS
            .iter()
            .any(|(bx, by, bw, bh)| x >= *bx && x < bx + bw && y >= *by && y < by + bh)
    }

    /// The map must not stick out of the round frame.
    ///
    /// Two facts about the art are pinned here. (1) The disc
    /// `mm_alpha.ddj` cuts is 101x101 at (14,58), i.e. centre (64,108) radius
    /// 50.5 — the bands must draw all of it, or the map would show a chord.
    /// (2) Inside the viewport rect the frame is transparent at 210 px that
    /// are NOT disc: none above row 145, and in rows 145..158 only at columns
    /// x <= 27 or x >= 107. Anything the bands draw there hangs outside the
    /// round window.
    ///
    /// The red control is the old single square (14,57,105,105): the same two
    /// checks run against it below, and the second one fails — which is the
    /// defect. Without it this test would only be asserting
    /// that a rectangle contains itself.
    #[test]
    fn the_map_bands_cover_the_disc_and_nothing_that_leaks_past_the_frame() {
        let old_square = |x: f32, y: f32| {
            x >= VIEWPORT_RECT.0
                && x < VIEWPORT_RECT.0 + VIEWPORT_RECT.2
                && y >= VIEWPORT_RECT.1
                && y < VIEWPORT_RECT.1 + VIEWPORT_RECT.3
        };

        // (1) every pixel of the disc is drawn — by both shapes
        let mut disc_px = 0;
        for row in 58..159 {
            for col in 14..115 {
                let (dx, dy) = (
                    col as f32 + 0.5 - DISC_CENTER.0,
                    row as f32 + 0.5 - DISC_CENTER.1,
                );
                if dx * dx + dy * dy <= DISC_RADIUS * DISC_RADIUS {
                    disc_px += 1;
                    assert!(
                        in_a_band(col as f32 + 0.5, row as f32 + 0.5),
                        "disc pixel ({col},{row}) is not in any band"
                    );
                    assert!(old_square(col as f32 + 0.5, row as f32 + 0.5));
                }
            }
        }
        // ~pi * 50.5^2; the loop must have actually run over the disc
        assert!(disc_px > 7900 && disc_px < 8100, "disc pixels {disc_px}");

        // (2) the leaking columns are drawn by NEITHER band ...
        let mut leaking = 0;
        for row in 145..162 {
            for col in [14, 20, 27, 107, 112, 118] {
                if row >= 159 || !(28..107).contains(&col) {
                    leaking += 1;
                    assert!(
                        !in_a_band(col as f32 + 0.5, row as f32 + 0.5),
                        "band draws the leaking pixel ({col},{row})"
                    );
                }
            }
        }
        assert!(leaking > 0, "the leak sample must not be empty");
        // ... and red: the old square drew them
        assert!(old_square(14.5, 150.5));
        assert!(old_square(118.5, 157.5));
        assert!(old_square(66.5, 160.5));
    }

    /// A marker may only be drawn where the frame really is a hole: fully
    /// inside the disc. Red control: the cull this replaced worked
    /// from the *square's* centre with radius `h + size/2` = 56.5, which
    /// admits points well outside the 50.5 disc.
    #[test]
    fn a_marker_is_culled_against_the_disc_not_the_square() {
        let h = VIEWPORT_RECT.2 / 2.0;
        let (dcx, dcy) = (
            DISC_CENTER.0 - VIEWPORT_RECT.0,
            DISC_CENTER.1 - VIEWPORT_RECT.1,
        );
        let disc_keeps = |cx: f32, cy: f32, size: f32| {
            let reach = DISC_RADIUS - size / 2.0;
            (cx - dcx).powi(2) + (cy - dcy).powi(2) <= reach.powi(2)
        };
        let old_keeps = |cx: f32, cy: f32, size: f32| {
            (cx - h).powi(2) + (cy - h).powi(2) <= (h + size / 2.0).powi(2)
        };

        // the centre dot stays either way
        assert!(disc_keeps(dcx, dcy, DOT_SIZE));
        // a dot in the square's bottom-left corner is outside the disc: the
        // old rule kept it (it is 55.6 from the square centre, under 56.5),
        // the new one drops it
        let (cx, cy) = (13.0, 91.0);
        assert!(old_keeps(cx, cy, DOT_SIZE), "the red control must be red");
        assert!(!disc_keeps(cx, cy, DOT_SIZE));
        // and a dot right at the rim is only kept while it fits whole
        assert!(disc_keeps(
            dcx + DISC_RADIUS - DOT_SIZE / 2.0 - 0.01,
            dcy,
            DOT_SIZE
        ));
        assert!(!disc_keeps(
            dcx + DISC_RADIUS - DOT_SIZE / 2.0 + 0.01,
            dcy,
            DOT_SIZE
        ));
    }

    /// The window placement is a transcription of the v1.188 resinfo, so pin it
    /// to the cited bytes: it previously drifted to a hand-picked `(4, 4)`
    /// top-right margin (issue #305).
    #[test]
    fn window_anchor_matches_the_ginterface_rect() {
        // resinfo/ginterface.txt:807 — GDR_MINIMAP:CIFMinimap, ID 10.
        assert_eq!(WINDOW_RECT, (892.0, 6.0, 140.0, 184.0));
        assert_eq!((FRAME_W, FRAME_H), (140.0, 184.0));
        assert_eq!(WINDOW_TOP, 6.0);
        // 1024 - (892 + 140) = -8: the vanilla window overhangs the right edge.
        assert_eq!(WINDOW_RIGHT, -8.0);
    }

    /// Every element rect is byte-exact against `resinfo/ifminimap.txt`; the
    /// cited line is the element's block header.
    #[test]
    fn element_rects_match_ifminimap() {
        assert_eq!(VIEWPORT_RECT, (14.0, 57.0, 105.0, 105.0)); // :139 _ALPHA
        assert_eq!(AREA_NAME_RECT, (12.0, 9.0, 104.0, 12.0)); // :82 _TEXT_AREANAME
        assert_eq!(POS_X_RECT, (8.0, 32.0, 56.0, 11.0)); // :63 _TEXT_POS_X
        assert_eq!(POS_Z_RECT, (67.0, 32.0, 56.0, 11.0)); // :44 _TEXT_POS_Y
        assert_eq!(ZOOM_IN_RECT, (107.0, 136.0, 20.0, 20.0)); // :120 _ZOOMIN
        assert_eq!(ZOOM_OUT_RECT, (90.0, 152.0, 20.0, 20.0)); // :101 _ZOOMOUT
        assert_eq!(MAP_BUTTON_RECT, (99.0, 47.0, 24.0, 24.0)); // :25 _BTN_TOG_MAP
    }

    /// The second readout is `GDR_MINIMAP_TEXT_POS_Y` (ifminimap.txt:44), so it
    /// is labelled "Y" — it used to emit "Z" after the world-space field name.
    #[test]
    fn coordinate_labels_use_the_original_axis_letters() {
        assert_eq!(coord_text('X', 6400), "X:6400");
        assert_eq!(coord_text('Y', -70), "Y:-70");
    }

    /// `L"X:%3d"` / `L"Y:%3d"` are the original's format strings: no space after the
    /// colon, no thousands separator, minus sign for negatives, and width 3
    /// left-padded — visible only below 100.
    #[test]
    fn coordinate_readout_matches_the_vanilla_format_string() {
        // The original's head, character for character.
        assert_eq!(coord_text('X', 6464), "X:6464");
        assert_eq!(coord_text('Y', 891), "Y:891");
        // %d, not a locale format: no group separator even at five digits.
        assert_eq!(coord_text('X', 12345), "X:12345");
        // %d keeps the sign; "-70" already fills the 3-wide field.
        assert_eq!(coord_text('Y', -70), "Y:-70");
        assert_eq!(coord_text('X', -6), "X: -6");
        // Width 3, padded on the left with spaces.
        assert_eq!(coord_text('Y', 12), "Y: 12");
        assert_eq!(coord_text('X', 0), "X:  0");
    }

    /// The original truncates the tenth-coordinate instead of rounding
    /// (`1286.0 / 10 = 128.6` shows as `128`, and `Y:891` is
    /// `(96 - 92) * 192 + 123`). We compose the global
    /// coordinate first, so the faithful operation is `floor`.
    #[test]
    fn displayed_coordinates_truncate_the_tenth() {
        // A position in region 24744 (regionX 168, regionZ 96),
        // x = 1286.0, z = 1230.0 -> world units gx = 168 * 1920 + 1286.
        let gx = 168.0 * 1920.0 + 1286.0;
        let gz = 96.0 * 1920.0 + 1230.0;
        assert_eq!(display_coords(gx, gz, false), (6464, 891));
        // Rounding would have shown 6465 here; the .6 tenth must be dropped.
        assert_eq!(display_coords(gx, gz, false).0, 6464);
        // Dungeon-local readout truncates the same way.
        assert_eq!(display_coords(1286.0, 1230.0, true), (128, 123));
        // West/north of the 135/92 origin the display goes negative; one unit
        // below the origin must read -1, not 0 (that is where a plain `trunc`
        // would differ).
        let below = (135.0 * 192.0 - 0.6) * 10.0;
        assert_eq!(display_coords(below, below, false).0, -1);
    }

    /// Dot sizes are the sign textures' own dimensions: 8x8 for
    /// monster/npc/otherplayer, 12x12 for unique, 16x16 for the player arrow.
    #[test]
    fn dot_sizes_match_the_sign_art() {
        assert_eq!(DOT_SIZE, 8.0);
        assert_eq!(UNIQUE_DOT_SIZE, 12.0);
        assert_eq!(ARROW_SIZE, 16.0);
    }

    /// The four zoom constants are the original `CIFMinimap` literals
    /// (see the module header), not a step table we invented.
    /// Asserted as arithmetic so a future edit of any one of them has to face
    /// the other three: the click range is exactly ten steps and the ctor
    /// default sits exactly on step five.
    #[test]
    fn zoom_constants_are_the_measured_ones() {
        assert_eq!(
            (ZOOM_MIN, ZOOM_MAX, ZOOM_STEP, ZOOM_DEFAULT),
            (64.0, 256.0, 19.2, 160.0)
        );
        assert!(((ZOOM_MAX - ZOOM_MIN) / ZOOM_STEP - 10.0).abs() < 1e-4);
        assert!((ZOOM_MIN + 5.0 * ZOOM_STEP - ZOOM_DEFAULT).abs() < 1e-4);
        // 256 px per 1920-unit region is one `minimap/{x}x{z}.ddj` at 1:1 —
        // vanilla's ceiling, which is why nothing magnifies past it.
        assert_eq!(ZOOM_MAX, 256.0);
    }

    /// Ten zoom-out clicks reach the floor from the default and an eleventh
    /// does not undershoot it; the same going up. Clicking is what a player
    /// does, so the clamp is asserted through the button arithmetic.
    #[test]
    fn zoom_clicks_clamp_at_the_vanilla_bounds() {
        let mut target = ZOOM_DEFAULT;
        for _ in 0..5 {
            target = (target - ZOOM_STEP).max(ZOOM_MIN);
        }
        assert!((target - ZOOM_MIN).abs() < 1e-4, "five clicks down = 64");
        target = (target - ZOOM_STEP).max(ZOOM_MIN);
        assert_eq!(target, ZOOM_MIN);

        for _ in 0..10 {
            target = (target + ZOOM_STEP).min(ZOOM_MAX);
        }
        assert!((target - ZOOM_MAX).abs() < 1e-3, "ten clicks up = 256");
        target = (target + ZOOM_STEP).min(ZOOM_MAX);
        assert_eq!(target, ZOOM_MAX);
    }

    /// The drawn scale crawls at 50 px/s and lands *on* the target instead of
    /// oscillating around it (vanilla clamps on the overshooting step).
    #[test]
    fn zoom_eases_towards_the_target_and_stops_there() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<MinimapState>()
            .add_systems(Update, ease_minimap_zoom);
        app.world_mut().resource_mut::<MinimapState>().zoom_target = ZOOM_DEFAULT + ZOOM_STEP;

        // one 100ms frame moves 5 px, not the whole step
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(100));
        app.update();
        let zoom = app.world().resource::<MinimapState>().zoom;
        assert!((zoom - (ZOOM_DEFAULT + 5.0)).abs() < 1e-3, "got {zoom}");

        // a long frame lands exactly on the target and stays
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs(10));
        app.update();
        assert_eq!(
            app.world().resource::<MinimapState>().zoom,
            ZOOM_DEFAULT + ZOOM_STEP
        );
        app.update();
        assert_eq!(
            app.world().resource::<MinimapState>().zoom,
            ZOOM_DEFAULT + ZOOM_STEP
        );
    }
}

/// Self-registration for the minimap (#558). The HUD registry holds one line per
/// window, so two windows landing in the same lap no longer collide on it.
pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        use crate::scenes::SceneState;

        app.init_resource::<MinimapState>()
            .add_systems(OnEnter(SceneState::GameWorld), spawn_minimap)
            .add_systems(OnExit(SceneState::GameWorld), cleanup_minimap)
            // The Dungeons test scene runs the map widgets without the rest
            // of the HUD.
            .add_systems(OnExit(SceneState::Dungeons), cleanup_minimap)
            .add_systems(
                Update,
                (
                    sync_minimap_dungeon_context,
                    populate_minimap_viewport,
                    ease_minimap_zoom,
                    update_minimap_tiles,
                    update_minimap_coords,
                    update_minimap_area_name,
                    update_minimap_arrow,
                    update_minimap_dots,
                    update_minimap_party_signs,
                    update_minimap_floor_badge,
                )
                    .run_if(super::map_scenes),
            );
    }
}
