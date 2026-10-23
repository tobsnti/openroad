//! World map window state (M key).
//!
//! Idea: `worldmap_mapinfo.txt` declares both the tiled world map (id 0) and
//! the single-image city maps with their region rectangles; a kind-1
//! rectangle containing the player's region *is* the client's town test, so
//! opening the window inside Jangan shows the Jangan map and opening it in
//! the field shows the world map (vanilla behavior). The map to show is
//! picked on every open; the world↔city toggle and the world map's city
//! buttons switch it while browsing. The extensible [`MapMarkers`] layer
//! carries the member signs (`wmap_sign_party*.ddj`,
//! `wmap_sign_apprenticeship.ddj`); [`sync_party_markers`] fills its party
//! entries from the net-side [`PartyRoster`] and [`sync_quest_markers`] its
//! quest pins from the net-side [`QuestMarks`], while union/academy rosters
//! have no client state yet.

use bevy::prelude::*;

use crate::assets::textdata::teleport::{TeleportInfo, TeleportTable};
use crate::plugins::hud::chat::model::ChatState;
use crate::plugins::hud::minimap::MinimapDungeonContext;
use crate::plugins::map::terrain::REGION_SIZE;
use crate::plugins::net::entities::{DisplayName, NetworkEntities};
use crate::plugins::net::party::PartyRoster;
use crate::plugins::net::quest::{QuestMarkTarget, QuestMarks};
use crate::plugins::player::Player;
use crate::plugins::settings::keymap::KEY_WORLD_MAP;
use crate::plugins::settings::options::GameOptions;
use crate::plugins::textdata::{ClientTeleport, ClientTextNames, ClientWorldMap};
use crate::plugins::world_origin::WorldOrigin;
use crate::scenes::game_scene::server_position_to_sro;
use crate::util::region::RegionIdExt;

#[derive(Resource)]
pub struct WorldMapState {
    pub open: bool,
    /// The `MapDef.id` currently shown (0 = world map).
    pub current_map: u32,
    /// Dungeon mode: `(dungeon region id, DungeonMapDef.map_id)` of the
    /// floor map shown instead of `current_map`. Set on open while inside a
    /// dungeon; the floor selector buttons switch it.
    pub dungeon_map: Option<(u16, u32)>,
    /// One-shot: center the viewport on the player after the next rebuild.
    pub center_on_player: bool,
}

impl Default for WorldMapState {
    fn default() -> Self {
        Self {
            open: false,
            current_map: 0,
            dungeon_map: None,
            center_on_player: true,
        }
    }
}

/// What kind of sign a map marker renders as (art exists for all of these).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    /// The catch-all pin (`wmap_sign_location`), filled from the teleporter
    /// table by [`static_location_markers`].
    Generic,
    /// A server-placed quest mark (`0x30D6`), fed from
    /// [`QuestMarks`] by [`sync_quest_markers`].
    Quest,
    /// Fed from [`PartyRoster`] by [`sync_party_markers`].
    Party,
    /// Union (an alliance of parties) — no union packet is parsed at all, so
    /// there is no roster to feed this from.
    #[allow(dead_code)]
    UnionParty,
    /// Academy — the academy roster packet (0x3C81,
    /// `docs/net-academy-0x3C81.md`) has no `packets` model yet, so there are
    /// no member positions to place.
    #[allow(dead_code)]
    Academy,
}

#[derive(Clone, Debug)]
pub struct MapMarker {
    pub kind: MarkerKind,
    /// Display name. Drawn as the world map's hover label, and the join that
    /// tells the minimap which spawned entity a party marker belongs to.
    pub name: String,
    /// Global server-space coordinates (the minimap's `(-sro.x, sro.z)`
    /// convention), world units.
    pub gx: f32,
    pub gz: f32,
}

/// Positions to draw on every map (party members, academy members, quest
/// targets…). Producers push into it; [`sync_party_markers`] owns the
/// [`MarkerKind::Party`] entries and leaves every other kind untouched.
#[derive(Resource, Default)]
pub struct MapMarkers(pub Vec<MapMarker>);

/// The party roster as world-map markers.
///
/// A member's position is region-local, so it goes through the same
/// `server_position_to_sro` conversion as every other networked position, then
/// into the minimap's mirrored-X global convention that [`MapMarker`] uses.
///
/// Members inside a dungeon are skipped: their coordinates live in the
/// dungeon's own local frame (`region & 0x8000`), which only a dungeon floor
/// map can place — the overworld marker layer has no position for them. So is
/// a member whose position field is absent, which is now the common case rather
/// than an edge one: party records are presence-masked, so a member the server
/// has not placed yet simply has no `region` at all.
///
/// **So is the local player.** 0x3065 sends the *whole* party, not "the
/// others", so the player is one of its members — and every surface that draws
/// this layer already draws the player itself (the world map's `WmArrow`, the
/// minimap's centre arrow), so a party sign for yourself is a duplicate
/// wherever it lands. The join is `own_name`, which is the only one available:
/// a member is keyed by a party JID and a spawned entity by its network id, so
/// nothing but the name connects them. An unknown own name keeps every marker —
/// drawing one dot too many beats dropping a real member's.
pub fn party_markers(roster: &PartyRoster, own_name: Option<&str>) -> Vec<MapMarker> {
    roster
        .members
        .iter()
        .filter(|member| match (own_name, member.name.as_deref()) {
            (Some(own), Some(name)) => own != name,
            _ => true,
        })
        .filter(|member| !member.region.is_some_and(|region| region.is_dungeon()))
        .filter_map(|member| {
            let region = member.region?;
            let position = member.position_world.as_ref()?;
            let sro = server_position_to_sro(
                region,
                position.x as f32,
                position.y as f32,
                position.z as f32,
            );
            Some(MapMarker {
                kind: MarkerKind::Party,
                // An unnamed member still gets a marker: the position is what
                // the map is for, and the label is only a tooltip.
                name: member.name.clone().unwrap_or_default(),
                gx: -sro.x,
                gz: sro.z,
            })
        })
        .collect()
}

/// Rebuild the party marker entries whenever the roster changes.
///
/// The roster resource lives in the networking plugin, which a HUD-only test
/// app need not have — hence the optional lookup. The player query is fallible
/// for the same reason, and a missing local player simply means no name to
/// filter on (see [`party_markers`]).
pub fn sync_party_markers(
    roster: Option<Res<PartyRoster>>,
    player: Query<&DisplayName, With<Player>>,
    mut markers: ResMut<MapMarkers>,
) {
    let Some(roster) = roster else {
        return;
    };
    if !roster.is_changed() {
        return;
    }
    let own_name = player.single().ok().map(|name| name.0.as_str());
    markers.0.retain(|marker| marker.kind != MarkerKind::Party);
    markers.0.extend(party_markers(&roster, own_name));
}

/// Server-placed quest marks as world-map markers.
///
/// Idea: the wire gives the mark's *target spawn record*, not a quest (see
/// [`QuestMarks`]), so the position is exact and the label is whatever the
/// client already knows about that entity — its [`DisplayName`], joined
/// through the unique-id index. A target the player has never been near has no
/// spawned entity and so no label; the pin still draws, because the position
/// is what a map is for and the label is only a hover tooltip. There is no
/// second source to try: the body carries no quest id, so no quest name exists
/// to fall back on.
///
/// Dungeon marks are skipped for the same reason a party member in a dungeon
/// is: their coordinates live in the dungeon's own frame (`region & 0x8000`),
/// which no overworld map can place.
///
/// Sorted by mark handle — the handle is monotonic, so this is placement
/// order, and `QuestMarks` is a map whose iteration order is not stable.
pub fn quest_markers(
    marks: &QuestMarks,
    name_of: impl Fn(u32) -> Option<String>,
) -> Vec<MapMarker> {
    let mut handles: Vec<(&u32, &QuestMarkTarget)> = marks.0.iter().collect();
    handles.sort_by_key(|(handle, _)| **handle);
    handles
        .into_iter()
        .filter(|(_, target)| !target.region.is_dungeon())
        .map(|(_, target)| {
            let sro = server_position_to_sro(
                target.region,
                target.x as f32,
                // Truncated toward zero by the server, so a target below zero
                // arrives wrapped; the map discards height anyway.
                target.y as i32 as f32,
                target.z as f32,
            );
            MapMarker {
                kind: MarkerKind::Quest,
                name: name_of(target.target_unique_id).unwrap_or_default(),
                gx: -sro.x,
                gz: sro.z,
            }
        })
        .collect()
}

/// Rebuild the quest entries whenever the server's mark set changes.
///
/// The marks and the unique-id index live in the networking plugin, which a
/// HUD-only test app need not have — hence the optional lookups, the same way
/// [`sync_party_markers`] takes its roster.
pub fn sync_quest_markers(
    marks: Option<Res<QuestMarks>>,
    index: Option<Res<NetworkEntities>>,
    named: Query<&DisplayName>,
    mut markers: ResMut<MapMarkers>,
) {
    let Some(marks) = marks else {
        return;
    };
    if !marks.is_changed() {
        return;
    }
    let name_of = |unique_id: u32| {
        index
            .as_ref()
            .and_then(|index| index.get(unique_id))
            .and_then(|entity| named.get(entity).ok())
            .map(|name| name.0.clone())
    };
    markers.0.retain(|marker| marker.kind != MarkerKind::Quest);
    markers.0.extend(quest_markers(&marks, name_of));
}

/// Whether the map keeps itself centered on the moving player. Deliberately
/// its own resource (not a `WorldMapState` field): panning and toggling
/// mutate it every frame and must not trip the window-rebuild system's
/// change detection.
#[derive(Resource)]
pub struct WorldMapFollow(pub bool);

impl Default for WorldMapFollow {
    fn default() -> Self {
        Self(true)
    }
}

/// The player's global server-space XZ (same mirrored-X convention as the
/// minimap).
pub fn player_global_xz(origin: &WorldOrigin, transform: &Transform) -> (f32, f32) {
    let sro = origin.to_sro(transform.translation);
    (-sro.x, sro.z)
}

/// Build the static location markers from the teleporter table (#71).
///
/// Idea: the world map's marker layer has always been renderable but had no
/// producer — every category the issue lists is blocked on a system we do not
/// have yet (party, quest, pets, academy) except this one, which is pure data:
/// `teleportdata.txt` already names every teleporter, its arrival region and
/// its region-local position, and we already load it as [`TeleportTable`].
/// Converting a row to the marker layer's global convention is exactly
/// [`server_position_to_sro`] followed by the minimap's `(-sro.x, sro.z)`.
///
/// Dungeon rows (region bit 15) are skipped: their coordinates are
/// dungeon-local offsets in a DOF's own frame, so they have no place on an
/// overworld map — projecting them anyway would scatter markers across
/// Jangan. Rows whose `SN_ZONE_*` key does not resolve keep the key as the
/// name rather than being dropped, so a missing string is visible instead of
/// silently costing a marker.
pub fn static_location_markers(
    teleport: &TeleportTable,
    names: &ClientTextNames,
) -> Vec<MapMarker> {
    let mut markers: Vec<MapMarker> = Vec::new();
    let mut rows: Vec<(&u32, &TeleportInfo)> = teleport.info.iter().collect();
    // HashMap order is not stable across runs; the marker list is compared in
    // tests and drawn in order, so sort by the teleporter id.
    rows.sort_by_key(|(id, _)| **id);
    for (_, info) in rows {
        if info.region.is_dungeon() {
            continue;
        }
        let sro = server_position_to_sro(
            info.region,
            info.position.x,
            info.position.y,
            info.position.z,
        );
        markers.push(MapMarker {
            kind: MarkerKind::Generic,
            name: names
                .name(&info.name_key)
                .unwrap_or(&info.name_key)
                .to_string(),
            gx: -sro.x,
            gz: sro.z,
        });
    }
    markers
}

/// Publish the static markers once the teleporter table has loaded. Runs on
/// change rather than once, because textdata arrives asynchronously and the
/// resource is empty until it does.
pub fn publish_static_location_markers(
    teleport: Res<ClientTeleport>,
    names: Res<ClientTextNames>,
    mut markers: ResMut<MapMarkers>,
) {
    if !teleport.is_changed() && !names.is_changed() {
        return;
    }
    let Some(table) = teleport.table() else {
        return;
    };
    let fresh = static_location_markers(table, &names);
    // keep every dynamic marker (party/academy/quest producers push their own)
    markers.0.retain(|m| !matches!(m.kind, MarkerKind::Generic));
    markers.0.extend(fresh);
}

/// Toggle with M (unless the chat input is capturing keys). Opening picks the
/// map for the player's current region: a covering city map, else the world.
pub fn toggle_world_map(
    keys: Res<ButtonInput<KeyCode>>,
    chat: Res<ChatState>,
    options: Res<GameOptions>,
    origin: Res<WorldOrigin>,
    worldmap: Res<ClientWorldMap>,
    dungeon: Option<Res<MinimapDungeonContext>>,
    player: Query<&Transform, With<Player>>,
    mut state: ResMut<WorldMapState>,
) {
    // Rebindable via the Key Map tab; M by default.
    let Some(key) = options.key_for(KEY_WORLD_MAP) else {
        return;
    };
    if !keys.just_pressed(key) || chat.input_open {
        return;
    }
    if state.open {
        state.open = false;
        return;
    }
    open_for_player_region(&origin, &worldmap, &player, dungeon.as_deref(), &mut state);
}

/// Shared open path (M key and the minimap's map button). Inside a dungeon
/// (context present) the dungeon's own floor map opens — the player's floor
/// when the Dungeonmap table names it, else the region's first floor; a
/// dungeon without map data falls back to the overworld pick.
pub fn open_for_player_region(
    origin: &WorldOrigin,
    worldmap: &ClientWorldMap,
    player: &Query<&Transform, With<Player>>,
    dungeon: Option<&MinimapDungeonContext>,
    state: &mut WorldMapState,
) {
    if let Some(ctx) = dungeon {
        let def = worldmap.table().and_then(|table| {
            table
                .dungeon_floor(ctx.region_id, &ctx.floor)
                .or_else(|| table.dungeon_floors(ctx.region_id).first())
        });
        if let Some(def) = def {
            state.dungeon_map = Some((ctx.region_id, def.map_id));
            state.center_on_player = true;
            state.open = true;
            return;
        }
    }
    state.dungeon_map = None;
    let map_id = player
        .single()
        .ok()
        .and_then(|transform| {
            let (gx, gz) = player_global_xz(origin, transform);
            let rx = (gx / REGION_SIZE).floor() as i32;
            let rz = (gz / REGION_SIZE).floor() as i32;
            worldmap
                .table()
                .and_then(|table| table.city_map_for_region(rx, rz))
                .map(|map| map.id)
        })
        .unwrap_or(0);
    state.current_map = map_id;
    state.center_on_player = true;
    state.open = true;
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::plugins::net::entities::NetworkId;
    use crate::plugins::net::party::tests::{core, party_data};
    use crate::plugins::net::quest::{on_quest_mark_add, on_quest_mark_remove};
    use bevy::math::Vec3;
    use packets::agent::quest::{QuestMarkAdd, QuestMarkRemove, QUEST_MARK_NEW};
    use std::collections::HashMap;

    /// Drives a synthetic 0x3065 through the roster and asserts what the map
    /// layer gets out of it: one Party marker per overworld member, placed at
    /// the region-local position folded into global space.
    #[test]
    fn party_data_becomes_party_markers() {
        let mut roster = PartyRoster::default();
        // Jangan (25000 = 0x61A8 -> sector 168x97), and one member inside a
        // dungeon region (bit 15 set) that the overworld layer cannot place.
        let mut in_dungeon = core(2, "Bob", 0x8001);
        in_dungeon.position_world = None;
        roster.apply_data(&party_data(vec![core(1, "Alice", 25000), in_dungeon]));

        let markers = party_markers(&roster, None);
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].kind, MarkerKind::Party);
        assert_eq!(markers[0].name, "Alice");
        // core()'s position is (100, 0, 200) region-local.
        assert!((markers[0].gx - (168.0 * REGION_SIZE + 100.0)).abs() < 1e-3);
        assert!((markers[0].gz - (97.0 * REGION_SIZE + 200.0)).abs() < 1e-3);
    }

    /// 0x3065 sends the whole party, so the local player is one of its members
    /// — and every surface that draws this layer draws the player itself. A
    /// party sign under your own arrow is a duplicate, which is what put a
    /// second dot on the world map.
    #[test]
    fn the_local_player_is_not_one_of_their_own_party_markers() {
        let mut roster = PartyRoster::default();
        roster.apply_data(&party_data(vec![
            core(1, "Alice", 25000),
            core(2, "Bob", 25000),
        ]));

        let names = |own| {
            party_markers(&roster, own)
                .into_iter()
                .map(|marker| marker.name)
                .collect::<Vec<_>>()
        };

        assert_eq!(names(Some("Alice")), vec!["Bob".to_string()]);
        // not knowing our own name draws one dot too many rather than
        // dropping a real member's
        assert_eq!(names(None), vec!["Alice".to_string(), "Bob".to_string()]);
        // and a name nobody in the party has changes nothing
        assert_eq!(
            names(Some("Nobody")),
            vec!["Alice".to_string(), "Bob".to_string()]
        );
    }

    /// The sync system owns only its own kind: foreign markers survive, and a
    /// stale party marker does not.
    #[test]
    fn sync_party_markers_replaces_only_party_entries() {
        let mut app = App::new();
        app.init_resource::<MapMarkers>()
            .init_resource::<PartyRoster>()
            .add_systems(Update, sync_party_markers);
        app.world_mut()
            .resource_mut::<MapMarkers>()
            .0
            .push(MapMarker {
                kind: MarkerKind::Generic,
                name: "quest".into(),
                gx: 0.0,
                gz: 0.0,
            });
        app.world_mut()
            .resource_mut::<PartyRoster>()
            .apply_data(&party_data(vec![core(1, "Alice", 25000)]));
        app.update();

        let markers = &app.world().resource::<MapMarkers>().0;
        assert_eq!(markers.len(), 2);
        assert_eq!(
            markers
                .iter()
                .filter(|m| m.kind == MarkerKind::Party)
                .count(),
            1
        );

        // Party dissolves -> the party marker goes, the generic one stays.
        app.world_mut()
            .resource_mut::<PartyRoster>()
            .apply_data(&party_data(vec![]));
        app.update();
        let markers = &app.world().resource::<MapMarkers>().0;
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].kind, MarkerKind::Generic);
    }

    /// The captured `0x30D6` of 2026-08-10T18:19:22.552Z: region 25000, the
    /// target's position truncated to (1584, 0, 1407), target unique id 243.
    /// The fold must put it at the same global point a party member at those
    /// region-local coordinates gets, and label it with what the client knows
    /// about entity 243 — nothing, when it has never been spawned.
    #[test]
    fn quest_marks_become_quest_markers() {
        let mut marks = QuestMarks::default();
        marks.0.insert(
            0x52e647e8,
            QuestMarkTarget {
                region: 25000,
                x: 1584,
                y: 0,
                z: 1407,
                target_unique_id: 243,
            },
        );
        // and one inside a dungeon (bit 15), whose coordinates no overworld
        // map can place
        marks.0.insert(
            0x52e647e9,
            QuestMarkTarget {
                region: 0x8001,
                x: 10,
                y: 0,
                z: 20,
                target_unique_id: 7,
            },
        );

        let markers = quest_markers(&marks, |unique_id| {
            (unique_id == 243).then(|| "Smith".to_string())
        });
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].kind, MarkerKind::Quest);
        assert_eq!(markers[0].name, "Smith");
        assert!((markers[0].gx - (168.0 * REGION_SIZE + 1584.0)).abs() < 1e-3);
        assert!((markers[0].gz - (97.0 * REGION_SIZE + 1407.0)).abs() < 1e-3);

        // an unknown target still draws; the label is only a hover tooltip
        let unlabelled = quest_markers(&marks, |_| None);
        assert_eq!(unlabelled.len(), 1);
        assert_eq!(unlabelled[0].name, "");
    }

    /// The whole path the client was missing, driven by the two packets and
    /// nothing else: `0x30D6` puts a pin on the map layer, `0x30D7` takes it
    /// off again, and neither touches a marker of another kind.
    #[test]
    fn a_quest_mark_packet_adds_a_map_marker_and_its_removal_takes_it_away() {
        let mut app = App::new();
        app.add_message::<QuestMarkAdd>()
            .add_message::<QuestMarkRemove>()
            .init_resource::<QuestMarks>()
            .init_resource::<MapMarkers>()
            .init_resource::<NetworkEntities>()
            .add_systems(
                Update,
                (on_quest_mark_add, on_quest_mark_remove, sync_quest_markers).chain(),
            );
        app.world_mut()
            .resource_mut::<MapMarkers>()
            .0
            .push(MapMarker {
                kind: MarkerKind::Party,
                name: "Alice".into(),
                gx: 0.0,
                gz: 0.0,
            });
        // the target of mark 0x52e647e8 is spawned, so the pin gets its name
        app.world_mut()
            .spawn((NetworkId(243), DisplayName("Smith".to_string())));

        app.world_mut().write_message(QuestMarkAdd {
            mark_id: 0x52e647e8,
            unknown0: 0x82,
            mark: QUEST_MARK_NEW,
            region: 25000,
            x: 1584,
            y: 0,
            z: 1407,
            target_unique_id: 243,
        });
        app.update();

        let markers = &app.world().resource::<MapMarkers>().0;
        assert_eq!(markers.len(), 2, "the party marker survives");
        let quest: Vec<&MapMarker> = markers
            .iter()
            .filter(|m| m.kind == MarkerKind::Quest)
            .collect();
        assert_eq!(quest.len(), 1);
        assert_eq!(quest[0].name, "Smith");
        assert!((quest[0].gx - (168.0 * REGION_SIZE + 1584.0)).abs() < 1e-3);

        app.world_mut().write_message(QuestMarkRemove {
            mark_id: 0x52e647e8,
        });
        app.update();

        let markers = &app.world().resource::<MapMarkers>().0;
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].kind, MarkerKind::Party);
    }

    fn info(name_key: &str, region: u16, position: Vec3) -> TeleportInfo {
        TeleportInfo {
            codename: "TP".to_string(),
            name_key: name_key.to_string(),
            owner_ref: 0,
            region,
            position,
            radius: 1.0,
        }
    }

    fn table(rows: Vec<(u32, TeleportInfo)>) -> TeleportTable {
        TeleportTable {
            info: rows.into_iter().collect(),
            by_owner_ref: HashMap::new(),
            links: HashMap::new(),
            buildings: HashMap::new(),
        }
    }

    /// #71: the one category that is not blocked on a missing system. A
    /// teleporter's region + region-local position becomes a marker in the
    /// map layer's global convention.
    #[test]
    fn teleporters_become_static_location_markers() {
        // region 25000 = (x 168, z 97) in the 8/7-bit split
        let region: u16 = 25000;
        let (rx, rz) = region.to_x_z();
        let markers = static_location_markers(
            &table(vec![(
                1,
                info("SN_ZONE_JANGAN", region, Vec3::new(100.0, 0.0, 200.0)),
            )]),
            &ClientTextNames::default(),
        );
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].kind, MarkerKind::Generic);
        assert_eq!(markers[0].gx, rx as f32 * 1920.0 + 100.0);
        assert_eq!(markers[0].gz, rz as f32 * 1920.0 + 200.0);
        // an unresolved SN_ key stays visible as itself
        assert_eq!(markers[0].name, "SN_ZONE_JANGAN");
    }

    /// Dungeon rows carry dungeon-local offsets, not region-tiled ones, so
    /// projecting them onto the overworld map would scatter markers.
    #[test]
    fn dungeon_teleporters_are_not_projected_onto_the_world_map() {
        let dungeon: u16 = 0x8001;
        assert!(dungeon.is_dungeon());
        let markers = static_location_markers(
            &table(vec![(
                2,
                info("SN_ZONE_DUNGEON", dungeon, Vec3::new(-9000.0, 0.0, 12000.0)),
            )]),
            &ClientTextNames::default(),
        );
        assert!(markers.is_empty());
    }
}
