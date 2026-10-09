use std::collections::HashMap;

use bevy::camera::Camera3d;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::prelude::*;

use crate::assets::ban::JMXVBAN;
use crate::assets::bms::mesh::JMXVBMS;
use crate::assets::bmt::material::{
    rim_material_label, sheen_cutout_material_label, sheen_material_label, sheen_probe_textures,
    BmtMaterialDefaults, JMXVBMT,
};
use crate::assets::bmt::rim::SroRimMaterial;
use crate::assets::bmt::sheen::SroSheenMaterial;
use crate::assets::bsk::JMXVBSK;
use crate::assets::bsr::resource::SroResource;
use crate::assets::cpd::JMXVCPD;
use crate::assets::ifo::object::{ObjectInfo, ObjectInfoIndex};
use crate::assets::ifo::IFOAsset;
use crate::assets::o2::{MapObject, JMXVMAPO2};
use crate::plugins::cursor::interactions::GameCursorTarget;
use crate::plugins::dev::render_debug::RenderDebugSettings;
use crate::plugins::dynamic_resource_loader::{MirroredResource, UnloadedResource};
use crate::plugins::map::assets::MapsAssets;
use crate::plugins::map::terrain::{Terrain, TerrainLoadState, TerrainObjectData};
use crate::util::mesh::needs_winding_reversal;
use crate::util::region::RegionIdExt;

/// Extra slack past the fog end before an object wrapper is hidden: the
/// wrapper's translation is the object's *anchor*, and its meshes can extend
/// from there toward the camera (long wall segments, large buildings). An
/// anchor at fog_end + m whose geometry reaches m units closer is exactly at
/// the fully-fogged distance — invisible either way.
const OBJECT_HIDE_MARGIN: f32 = 480.0;

/// Hides map-object wrappers whose anchor sits past the cull distance
/// (`ViewRange::live_cull`, by default where the fog turns opaque).
/// The region-root hiding (`terrain::region_visibility`) only covers regions
/// *entirely* past the fog end — objects in boundary regions, and objects
/// parented to a nearer region than the one they geographically occupy
/// (`load_terrain_objects_system` spawns region-edge objects under whichever
/// region's `.o2` listed them first), still rendered from deep inside the
/// fog. Stands down while the render-debug `render_objects` toggle is off —
/// that path owns wrapper visibility then — and, like the region hiding,
/// while the camera carries no `DistanceFog` to hide behind.
pub fn cull_fogged_objects(
    settings: Res<RenderDebugSettings>,
    view: Res<crate::plugins::map::view_range::ViewRange>,
    cameras: Query<(&Transform, &Camera, Has<DistanceFog>), With<Camera3d>>,
    mut objects: Query<(&GlobalTransform, &mut Visibility), With<MapObject>>,
) {
    if !settings.render_objects {
        return;
    }
    let Some((camera_pos, fog_active)) = cameras
        .iter()
        .find(|(_, camera, _)| camera.is_active)
        .map(|(transform, _, fog)| (transform.translation, fog))
    else {
        return;
    };
    let hide_dist = view.live_cull + OBJECT_HIDE_MARGIN;
    let hide_dist_sq = hide_dist * hide_dist;

    for (global, mut visibility) in &mut objects {
        let desired =
            if fog_active && global.translation().distance_squared(camera_pos) > hide_dist_sq {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
        // set_if_neq: an unconditional write would re-propagate visibility
        // through every object's resource subtree each frame.
        visibility.set_if_neq(desired);
    }
}

/// Caches converted prop meshes by source `.bms` handle, so repeated instances of the
/// same model (a common tree, rock, etc. reused across many regions) share one GPU mesh
/// instead of each spawn rebuilding and registering its own copy — shared handles are
/// also what lets Bevy batch/instance the repeated draws. Keyed also on the two
/// spawn-time parameters that change the produced mesh bytes: the winding direction
/// and whether joint attributes were built in (`JMXVBMS::to_mesh(reverse_winding,
/// with_skinning)`). The skinning flag must be part of the key: the same skinned
/// `.bms` is spawned both skinned (worn armor) and unskinned (ground drop), and a
/// shared mesh would give one of the two a pipeline/bind-group mismatch (wgpu
/// validation error, see `to_mesh`).
///
/// Entries are weak `AssetId`s, not `Handle`s: the spawned entities are the
/// only strong owners, so a mesh (and its source `.bms`) frees once the last
/// region using it unloads instead of staying pinned for the whole session.
/// Lookups resolve via `Assets::get_strong_handle` and rebuild on a dead id;
/// `prune_spawn_caches` sweeps dead entries so the map itself stays bounded.
#[derive(Resource, Default)]
pub struct SroMeshes(pub HashMap<(AssetId<JMXVBMS>, bool, bool), AssetId<Mesh>>);

/// Caches skinned-mesh inverse bind poses by (source `.bms` handle, skeleton handle) —
/// this pair fully determines the bind pose matrices, so repeated spawns of the same
/// model+skeleton combination (e.g. many instances of one monster/NPC type) share one
/// asset instead of each spawn registering its own identical copy. Only populated for
/// skinned meshes; non-skinned props don't need bind poses at all (see
/// `SpawnResource::prepare_mesh_groups`). The skeleton is usually the resource's own,
/// but for attached clothes it is the skeleton of the character wearing them.
/// Weak `AssetId` entries, same lifetime scheme as [`SroMeshes`].
#[derive(Resource, Default)]
pub struct SroBindPoses(
    pub HashMap<(AssetId<JMXVBMS>, AssetId<JMXVBSK>), AssetId<SkinnedMeshInverseBindposes>>,
);

/// Caches retargeted animation clips by (`.ban`, skeleton, wrapper name) —
/// together these fully determine the clip `JMXVBAN::to_animation_clip`
/// builds (the name roots every bone's `AnimationTargetId`). Converting a clip
/// builds a curve per animated bone, and a resource carries dozens of `.ban`s,
/// so building them per *instance* made every spawn of a common monster or NPC
/// re-convert its whole animation set, and every gait/skill change rebuild one.
/// Clips are never mutated after creation, so instances can share them.
/// Weak `AssetId` entries, same lifetime scheme as [`SroMeshes`].
#[derive(Resource, Default)]
pub struct SroAnimationClips(
    pub HashMap<(AssetId<JMXVBAN>, AssetId<JMXVBSK>, String), AssetId<AnimationClip>>,
);

impl SroAnimationClips {
    /// The shared clip for `ban` retargeted onto `skeleton` under
    /// `wrapper_name`, converting it only on a cache miss (or once the last
    /// user dropped it). Inserts through `Assets::add` so a second lookup in
    /// the same frame already hits.
    pub fn get_or_build(
        &mut self,
        clips: &mut Assets<AnimationClip>,
        (ban_id, ban): (AssetId<JMXVBAN>, &JMXVBAN),
        (skeleton_id, skeleton): (AssetId<JMXVBSK>, &JMXVBSK),
        wrapper_name: &String,
    ) -> Handle<AnimationClip> {
        let key = (ban_id, skeleton_id, wrapper_name.clone());
        if let Some(handle) = self.0.get(&key).and_then(|id| clips.get_strong_handle(*id)) {
            return handle;
        }
        let handle = clips.add(ban.to_animation_clip(skeleton, wrapper_name));
        self.0.insert(key, handle.id());
        handle
    }
}

/// The rim and sheen material variants, built on demand. The `.bmt` loader
/// used to add every variant of every material as a labeled sub-asset, so each
/// material in each loaded set cost several extra material assets (each one
/// extracted and prepared by the renderer) when a mesh only ever uses one —
/// and map props, the bulk of all sets, use none of them. Now the loader keeps
/// only the default variant plus each material's texture
/// ([`JMXVBMT::diffuse_textures`]), and the spawn path asks here.
///
/// Keyed by (material set, variant key — `rim_material_label` & co.), so one
/// variant is shared by every instance of a set exactly as the labeled
/// sub-asset was; every consumer that customizes one (selection highlight,
/// `+N` shine) clones it first. Weak `AssetId` entries like [`SroMeshes`],
/// resolved through the asset server: variants go in via `AssetServer::add`,
/// which registers the id at once, so a second spawn in the same frame already
/// hits (a duplicate would also split the batch). `prune_spawn_caches` drops
/// dead ids.
///
/// Behind a `Mutex` so both spawn commands reach it through a shared world
/// borrow while they hold the resource's own assets borrowed.
#[derive(Resource, Default)]
pub struct SroMaterialVariants(std::sync::Mutex<MaterialVariantMaps>);

type VariantKey = (AssetId<JMXVBMT>, String);

#[derive(Default)]
pub struct MaterialVariantMaps {
    rim: HashMap<VariantKey, AssetId<SroRimMaterial>>,
    sheen: HashMap<VariantKey, AssetId<SroSheenMaterial>>,
    unsheened: HashMap<VariantKey, AssetId<StandardMaterial>>,
}

/// What building a variant reads from the world. All shared borrows.
pub struct VariantSources<'a> {
    pub asset_server: &'a AssetServer,
    pub material_sets: &'a Assets<JMXVBMT>,
    pub defaults: BmtMaterialDefaults,
}

impl<'a> VariantSources<'a> {
    /// `None` when the variant material types are not registered (apps built
    /// without the client's material plugins, e.g. some tests).
    pub fn from_world(world: &'a World) -> Option<Self> {
        if !world.contains_resource::<Assets<SroRimMaterial>>()
            || !world.contains_resource::<Assets<SroSheenMaterial>>()
        {
            return None;
        }
        Some(Self {
            asset_server: world.get_resource::<AssetServer>()?,
            material_sets: world.get_resource::<Assets<JMXVBMT>>()?,
            defaults: world
                .get_resource::<BmtMaterialDefaults>()
                .copied()
                .unwrap_or_default(),
        })
    }
}

impl SroMaterialVariants {
    fn maps(&self) -> std::sync::MutexGuard<'_, MaterialVariantMaps> {
        // a panic while holding the lock leaves plain maps behind; keep going
        self.0.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    /// The always-on rim variant of `material` in `set`.
    pub fn rim(
        &self,
        sources: &VariantSources,
        set: &Handle<JMXVBMT>,
        material: &str,
    ) -> Handle<SroRimMaterial> {
        let key = (set.id(), rim_material_label(material));
        resolve_variant(&mut self.maps().rim, key, sources.asset_server, || {
            // only requested while the rim is configured on
            let settings = sources.defaults.rim?;
            let (mat, texture) = sources
                .material_sets
                .get(set)?
                .material_with_texture(material)?;
            Some(mat.to_rim_material(texture.clone(), settings))
        })
    }

    /// The sheen (or, with `cutout`, sheen-cutout) variant of `material` in
    /// `set`.
    pub fn sheen(
        &self,
        sources: &VariantSources,
        set: &Handle<JMXVBMT>,
        material: &str,
        cutout: bool,
    ) -> Handle<SroSheenMaterial> {
        let label = if cutout {
            sheen_cutout_material_label(material)
        } else {
            sheen_material_label(material)
        };
        resolve_variant(
            &mut self.maps().sheen,
            (set.id(), label),
            sources.asset_server,
            || {
                let (mat, texture) = sources
                    .material_sets
                    .get(set)?
                    .material_with_texture(material)?;
                Some(mat.to_sheen_material(
                    texture.clone(),
                    sources.defaults.sheen,
                    cutout,
                    sheen_probe_textures(sources.asset_server),
                ))
            },
        )
    }

    /// What a sheen resource's `material` draws with while metallic sheen is
    /// off (`graphics.sheen.enabled`): the plain opaque material, see
    /// `SroMaterial::to_unsheened_material`.
    pub fn unsheened(
        &self,
        sources: &VariantSources,
        set: &Handle<JMXVBMT>,
        material: &str,
        cutout: bool,
    ) -> Handle<StandardMaterial> {
        let label = format!(
            "{}.unsheened{}",
            crate::assets::bmt::material::material_label(material),
            if cutout { ".cutout" } else { "" }
        );
        resolve_variant(
            &mut self.maps().unsheened,
            (set.id(), label),
            sources.asset_server,
            || {
                let (mat, texture) = sources
                    .material_sets
                    .get(set)?
                    .material_with_texture(material)?;
                Some(mat.to_unsheened_material(texture.clone(), cutout))
            },
        )
    }

    /// Periodic sweep (see [`prune_spawn_caches`]).
    fn prune(&self, asset_server: &AssetServer) {
        let mut maps = self.maps();
        maps.rim.retain(|_, id| asset_server.is_managed(*id));
        maps.sheen.retain(|_, id| asset_server.is_managed(*id));
        maps.unsheened.retain(|_, id| asset_server.is_managed(*id));
    }

    pub fn len(&self) -> usize {
        let maps = self.maps();
        maps.rim.len() + maps.sheen.len() + maps.unsheened.len()
    }
}

/// Resolve-or-build for one variant map. A material the set does not have
/// (or a set that is not loaded) yields the default handle, which renders
/// nothing — what requesting a missing labeled sub-asset did before.
fn resolve_variant<M: Asset>(
    ids: &mut HashMap<VariantKey, AssetId<M>>,
    key: VariantKey,
    asset_server: &AssetServer,
    build: impl FnOnce() -> Option<M>,
) -> Handle<M> {
    if let Some(handle) = ids.get(&key).and_then(|id| asset_server.get_id_handle(*id)) {
        return handle;
    }
    let Some(material) = build() else {
        return Handle::default();
    };
    let handle = asset_server.add(material);
    ids.insert(key, handle.id());
    handle
}

/// `Assets<AnimationClip>` together with the [`SroAnimationClips`] cache, as
/// one system parameter: the systems that build clips on demand (gait, skill
/// and knockdown swaps in `player`) take this in place of the bare assets, so
/// they reuse the clips the spawn already built without growing their
/// parameter lists past Bevy's limit.
#[derive(bevy::ecs::system::SystemParam)]
pub struct SharedClips<'w> {
    clips: ResMut<'w, Assets<AnimationClip>>,
    cache: ResMut<'w, SroAnimationClips>,
}

impl SharedClips<'_> {
    /// See [`SroAnimationClips::get_or_build`].
    pub fn get_or_build(
        &mut self,
        ban: (AssetId<JMXVBAN>, &JMXVBAN),
        skeleton: (AssetId<JMXVBSK>, &JMXVBSK),
        wrapper_name: &String,
    ) -> Handle<AnimationClip> {
        self.cache
            .get_or_build(&mut self.clips, ban, skeleton, wrapper_name)
    }

    pub fn assets(&self) -> &Assets<AnimationClip> {
        &self.clips
    }
}

/// Wrapper entity of every spawned map object, keyed by (region id << 16 | uid).
/// Region-edge objects are listed in *several* regions' `.o2` files, and regions
/// complete on different frames (mesh builds are budgeted per frame), so the
/// dedup must persist across frames — a per-frame map spawned an object once per
/// listing region, each duplicate dragging a whole compound/resource subtree
/// (and double draws + z-fighting on the overlap). Entries self-heal instead of
/// needing despawn bookkeeping: a key whose entity no longer exists (region
/// unloaded, scene torn down) is treated as absent — `Entities::contains`
/// matches by generation and counts same-frame reservations as alive.
#[derive(Resource, Default)]
pub struct SpawnedMapObjects(pub HashMap<u32, Entity>);

/// `ObjID`s seen in a `.o2` that have no `object.ifo` row, so the warning is
/// logged once per id rather than once per placement.
///
/// This is a real gap in the shipped data, not a parse error: this build's
/// `Map/object.ifo` declares 2767 rows (`ObjID` 0..2766) while `.o2` placements
/// reference ids up to **3253** — 357 distinct ids across **42,746 placements
/// (18.5% of 231,261)** in **520 of 4,506** files. Measured with an independent
/// reader over the user's `Map/`; see #429.
#[derive(Resource, Default)]
pub struct UnknownObjectIds(pub std::collections::HashSet<u32>);

/// Periodic sweep of the spawn caches: they store weak ids (see
/// [`SroMeshes`]) so region unloads free the assets, but the map entries
/// themselves — and [`SpawnedMapObjects`]' dead `Entity` ids — would still
/// grow forever without it. Timer-driven rather than on-miss because a
/// player always moving into *new* areas never revisits a key.
pub fn prune_spawn_caches(
    meshes: Res<Assets<Mesh>>,
    inverse_bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    entities: &bevy::ecs::entity::Entities,
    mut mesh_cache: ResMut<SroMeshes>,
    mut bind_pose_cache: ResMut<SroBindPoses>,
    mut spawned: ResMut<SpawnedMapObjects>,
    clips: Res<Assets<AnimationClip>>,
    mut clip_cache: ResMut<SroAnimationClips>,
    variants: Res<SroMaterialVariants>,
    asset_server: Res<AssetServer>,
) {
    variants.prune(&asset_server);
    mesh_cache.0.retain(|_, id| meshes.contains(*id));
    bind_pose_cache
        .0
        .retain(|_, id| inverse_bindposes.contains(*id));
    clip_cache.0.retain(|_, id| clips.contains(*id));
    spawned.0.retain(|_, e| entities.contains(*e));
}

pub fn load_compound_system(
    mut commands: Commands,
    query: Query<(Entity, &LoadingCompound)>,
    compound_assets: Res<Assets<JMXVCPD>>,
    asset_server: Res<AssetServer>,
) {
    query.iter().for_each(|(entity, loading_compound)| {
        if asset_server.load_state(&loading_compound.0).is_loaded() {
            let compound = compound_assets.get(&loading_compound.0).expect("i failed");
            let resource_handles: Vec<Handle<SroResource>> = compound
                .resources
                .iter()
                .map(|res| asset_server.load(format!("data://{}", res.display())))
                .collect();
            if let Ok(mut entity) = commands.get_entity(entity) {
                entity
                    .remove::<LoadingCompound>()
                    .insert(LoadingResources(resource_handles));
            }
        }
    });
}

pub fn load_resources_system(
    mut commands: Commands,
    query: Query<(Entity, &LoadingResources, Has<MirroredResource>)>,
    asset_server: Res<AssetServer>,
) {
    query
        .iter()
        .for_each(|(entity, loading_resources, mirrored)| {
            if loading_resources
                .0
                .iter()
                .all(|res| asset_server.load_state(res.id()).is_loaded())
            {
                // A compound (.cpd) references several resources (walls, roof, ...),
                // each authored in the shared compound space. Fan each one out onto
                // its own child entity so `spawn_resources_when_loaded` spawns every
                // part; inserting `UnloadedResource` repeatedly on this single entity
                // would overwrite all but the last, dropping most of the building.
                let handles = loading_resources.0.clone();
                commands
                    .entity(entity)
                    .remove::<LoadingResources>()
                    .with_children(|parent| {
                        for res in handles {
                            let mut part = parent.spawn((
                                Transform::default(),
                                Visibility::default(),
                                UnloadedResource(res),
                                CompoundPart,
                            ));
                            // Propagate the mirror flag so each part's meshes get
                            // winding-corrected when spawned.
                            if mirrored {
                                part.insert(MirroredResource);
                            }
                        }
                    });
            }
        });
}

#[allow(dead_code)]
pub struct EnMat {
    pub entity: Entity,
    pub mat: Mat4,
}

#[derive(Component, Clone)]
pub struct LoadingCompound(Handle<JMXVCPD>);

#[derive(Component, Clone)]
pub struct LoadingResources(pub Vec<Handle<SroResource>>);

/// Anchor entity of one compound (.cpd) part, parenting that part's spawned
/// resource. Pure marker so entity-count diagnostics can attribute these
/// (their `UnloadedResource` is removed once the resource spawns). Plain
/// single-resource placements have none: their placement entity anchors the
/// resource directly.
#[derive(Component)]
pub struct CompoundPart;

#[derive(Component, Clone)]
#[allow(dead_code)]
pub struct LoadingResource(pub Handle<SroResource>);

#[derive(Component, Clone)]
#[allow(dead_code)]
pub struct LoadingSkeleton(Handle<JMXVBSK>);

#[derive(Component, Default, Clone, Reflect)]
#[reflect(Component)]
pub struct Bones {
    pub bones: Vec<String>,
}

#[derive(Component, Clone)]
#[allow(dead_code)]
pub struct CompletedSkinningData(SkinnedMesh);

#[derive(Component, Clone)]
#[allow(dead_code)]
pub struct CompletedMesh(Handle<Mesh>);

#[derive(Component)]
#[allow(dead_code)]
pub struct CompletedSkeleton;

/// The `object.ifo` row for a placed `ObjID`, or `None` if the table has no
/// such row.
///
/// `object.ifo` does not cover every placed id: this build's table declares 2767
/// rows (`ObjID` 0..2766) while `.o2` placements reference ids up to **3253** —
/// 357 distinct ids across **42,746 placements (18.5 % of 231,261)** in **520 of
/// 4,506** files, measured with an independent reader over the user's `Map/`
/// (#429). Panicking on the lookup therefore takes the whole client down the
/// moment one of those regions streams in; skipping the placement costs one prop
/// and reports it **once per id** rather than once per placement.
fn object_details<'a>(
    object_info: &'a ObjectInfoIndex,
    id: u32,
    unknown: &mut UnknownObjectIds,
) -> Option<&'a ObjectInfo> {
    match object_info.0.get(&id) {
        Some(details) => Some(details),
        None => {
            if unknown.0.insert(id) {
                warn!("map: ObjID {id} has no object.ifo row; skipping its placements");
            }
            None
        }
    }
}

/// Object spawns budgeted per frame, across every region processed this
/// call. A freshly-loaded region (`TerrainLoadState::LoadedMeshes`) can list
/// hundreds of objects across its blocks/LOD groups, and spawning them all
/// in one frame — on top of the mirrored despawn burst on the opposite side
/// of the same crossing — was the ~208ms region-crossing hitch measured live
/// and documented in `docs/perf-remote.md`. The mesh-build stage already
/// solved this exact problem (`GROUP_BUILDS_PER_FRAME`, `terrain/mod.rs`);
/// this is the same idiom for object spawning. Deferred objects cost nothing
/// extra to retry: the cross-frame `SpawnedMapObjects` dedup below already
/// makes re-walking a partially-spawned region safe.
pub(crate) const OBJECT_SPAWNS_PER_FRAME: u32 = 64;

/// Whether the map placement `key` (region id << 16 | uid) of the resource
/// at `path` spawns, given `graphics.objects.nature_density`. Everything
/// outside `res/nature/` always does. Vegetation is kept for a share
/// `density` of placements, chosen by a hash of the placement key. That keeps
/// the choice stable, so the same trees stay away on every visit, and spread
/// evenly instead of thinning one side of a forest.
pub(crate) fn keeps_nature_placement(path: &str, key: u32, density: f32) -> bool {
    if density >= 1.0 {
        return true;
    }
    let lower = path.to_ascii_lowercase().replace('\\', "/");
    if !lower.contains("res/nature/") {
        return true;
    }
    // splitmix32-style finalizer: neighbouring uids land far apart
    let mut h = key.wrapping_mul(0x9E37_79B9);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    (h as f32 / u32::MAX as f32) < density.max(0.0)
}

pub fn load_terrain_objects_system(
    mut commands: Commands,
    mut query: Query<(Entity, &Terrain, &TerrainObjectData, &mut TerrainLoadState)>,
    map_object_assets: Res<Assets<JMXVMAPO2>>,
    asset_server: Res<AssetServer>,
    object_info_assets: Res<Assets<IFOAsset>>,
    maps_assets: Res<MapsAssets>,
    mut spawned: ResMut<SpawnedMapObjects>,
    mut unknown_objects: ResMut<UnknownObjectIds>,
    entities: &bevy::ecs::entity::Entities,
    config: Res<crate::plugins::config::ClientConfig>,
) {
    let object_info = object_info_assets
        .get(&maps_assets.object_index)
        .expect("i failed");
    let Some(object_info) = &object_info.object_info_index else {
        return;
    };
    // `graphics.streaming.object_spawns_per_frame` (the constant is its default)
    let mut budget = crate::plugins::config::graphics::StreamingSettings::budget(
        config.graphics.streaming.object_spawns_per_frame,
    ) as i32;
    query
        .iter_mut()
        .for_each(|(terrain_entity, terrain, object_data, mut load_state)| {
            if asset_server.load_state(&object_data.0).is_loaded() {
                let object_data = map_object_assets.get(&object_data.0).expect("i failed");

                match load_state.as_ref() {
                    TerrainLoadState::LoadedMeshes => {
                        // Only true once every object in this region has been
                        // spawned (or was already); stays `LoadedMeshes` (retried
                        // next frame) instead of advancing to `Completed` below
                        // if the budget ran out partway through.
                        let mut all_spawned = true;
                        object_data
                            .blocks
                            .iter()
                            .flat_map(|block| block.lod_groups.iter())
                            .for_each(|lod| {
                                commands.entity(terrain_entity).with_children(|parent| {
                                    lod.objects.iter().for_each(|object| {
                                        let obj_key: u32 =
                                            ((object.region_id as u32) << 16) | object.uid as u32;
                                        // Cross-frame dedup via the persistent registry;
                                        // dead entries (unloaded regions) read as absent.
                                        let already_spawned = spawned
                                            .0
                                            .get(&obj_key)
                                            .is_some_and(|&e| entities.contains(e));
                                        if already_spawned {
                                            return;
                                        }
                                        if budget <= 0 {
                                            all_spawned = false;
                                            return;
                                        }
                                        budget -= 1;
                                        {
                                            let (x, z) = object.region_id.to_x_z();
                                            let (cx, cz) = terrain.to_x_z();
                                            let dx = x as i32 - cx as i32;
                                            let dz = z as i32 - cz as i32;

                                            let relative_pos = Vec3::new(
                                                dx as f32 * 1920.0,
                                                0.0,
                                                dz as f32 * 1920.0,
                                            );
                                            let mut translation = object.position + relative_pos;
                                            translation.x *= -1.0;
                                            let trs_matrix = Mat4::from_scale_rotation_translation(
                                                Vec3::new(-1.0, 1.0, 1.0),
                                                Quat::from_rotation_y(object.yaw),
                                                translation,
                                            );
                                            let transform = Transform::from_matrix(trs_matrix);
                                            // `object.ifo` does not cover every
                                            // placed `ObjID`: this build's table
                                            // ends at 2766, while `.o2` records
                                            // reference ids up to 3253 — 357
                                            // distinct ids over 42,746 placements
                                            // (18.5%) in 520 of 4,506 files. An
                                            // `expect` here takes the whole client
                                            // down as soon as one of those regions
                                            // streams in; skipping the placement
                                            // loses one prop and says so once.
                                            let Some(object_details) = object_details(
                                                object_info,
                                                object.id,
                                                &mut unknown_objects,
                                            ) else {
                                                return;
                                            };
                                            let path =
                                                format!("data://{}", object_details.path.display());
                                            // `graphics.objects.nature_density`: a stable
                                            // share of vegetation placements is not spawned
                                            if !keeps_nature_placement(
                                                &path,
                                                obj_key,
                                                config.graphics.objects.nature_density,
                                            ) {
                                                return;
                                            }

                                            // Map objects are placed with a mirroring transform
                                            // (scale.x = -1). Derive whether their meshes need
                                            // reversed winding from the transform determinant,
                                            // via the single shared rule (see `needs_winding_reversal`).
                                            let mut object_entity =
                                                parent.spawn((transform, Visibility::default()));
                                            spawned.0.insert(obj_key, object_entity.id());
                                            if needs_winding_reversal(&trs_matrix) {
                                                object_entity.insert(MirroredResource);
                                            }
                                            if object_details.is_compound() {
                                                object_entity
                                                    .insert(Name::from(format!(
                                                        "Compound - {}|{} {}",
                                                        object.id,
                                                        object.uid,
                                                        &object_details.path.display()
                                                    )))
                                                    .insert(object.clone())
                                                    .insert(GameCursorTarget::default());
                                                let compound_handle: Handle<JMXVCPD> =
                                                    asset_server.load(path);
                                                object_entity
                                                    .insert(LoadingCompound(compound_handle));
                                            } else {
                                                let resource_handle: Handle<SroResource> =
                                                    asset_server.load(path);
                                                object_entity
                                                    .insert(Name::from(format!(
                                                        "Resource - {}",
                                                        object_details.path.display()
                                                    )))
                                                    .insert(object.clone())
                                                    .insert(GameCursorTarget::default())
                                                    // The placement itself anchors its one
                                                    // resource (as characters and dungeon
                                                    // objects do): routing it through
                                                    // `LoadingResources` gave every plain
                                                    // object an identity `CompoundPart` child
                                                    // in between — ~4,800 extra entities in a
                                                    // loaded area, each walked by the
                                                    // per-frame visibility passes and spawned
                                                    // and despawned with its region. The
                                                    // spawner waits for the load itself.
                                                    .insert(UnloadedResource(resource_handle));
                                            }
                                        }
                                    });
                                });
                            });
                        if all_spawned {
                            *load_state = TerrainLoadState::Completed;
                        }
                    }
                    TerrainLoadState::None | TerrainLoadState::BuildingMeshes { .. } => {}
                    TerrainLoadState::Completed => {
                        // Disarm the per-frame polling (the run condition is on
                        // `TerrainLoadState`, see `map/mod.rs`) but *keep*
                        // `TerrainObjectData`, so this region can be re-armed and
                        // re-run its object pass later — see
                        // `rearm_object_passes_on_region_unload` for why it has to.
                        if let Ok(mut entity) = commands.get_entity(terrain_entity) {
                            entity.remove::<TerrainLoadState>();
                        }
                    }
                };
            }
        });
}

/// Re-arms every completed region's object pass after any region unloads.
///
/// Idea: a map object is spawned as a child of whichever region's `.o2` listed
/// it *first*, but 10,820 of the 74,060 distinct objects in the shipped corpus
/// (14.6%) are listed by more than one region — one by 64 of them. Fences and
/// walls running along a region border are exactly this class. So when region A
/// unloads, Bevy despawns its whole subtree, including objects that
/// geographically belong to still-loaded region B; B has already `Completed`
/// and, before this system, could never spawn them again. The prop stayed
/// missing until B itself unloaded and reloaded (#571).
///
/// The obvious alternative — parent each object to its own `region_id`'s
/// terrain entity — was rejected on the data: 45 of those 74,060 objects are
/// listed *only* by a foreign region, 43 of which resolve to a real
/// `object.ifo` model. Strict ownership would silently delete them, and they
/// are not filler: `alex_paros` (the Pharos of Alexandria), `alex_harbor`,
/// `alex_obelisk` and two of Baghdad's city gates are in that set. Re-arming
/// keeps the listing-region spawn (nothing is lost) and repairs the lifetime
/// instead.
///
/// Re-arming is deliberately unconditional across loaded regions rather than
/// restricted to the unloaded region's neighbours: an object may be listed by a
/// region 8 sectors away (the 64-region case above), so adjacency is not a safe
/// filter. It costs nothing in steady state — regions unload only on a boundary
/// crossing, and the re-armed pass is one dedup lookup per listed object before
/// `Completed` disarms it again. `TerrainObjectData` survives `Completed`
/// precisely so this needs no asset reload.
pub fn rearm_object_passes_on_region_unload(
    mut commands: Commands,
    mut unloaded_regions: RemovedComponents<Terrain>,
    completed: Query<
        Entity,
        (
            With<Terrain>,
            With<TerrainObjectData>,
            Without<TerrainLoadState>,
        ),
    >,
) {
    if unloaded_regions.is_empty() {
        return;
    }
    unloaded_regions.clear();
    for terrain_entity in &completed {
        if let Ok(mut entity) = commands.get_entity(terrain_entity) {
            entity.insert(TerrainLoadState::LoadedMeshes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::ifo::object::ObjectInfo;
    use crate::plugins::map::terrain::TerrainId;
    use std::path::PathBuf;

    #[test]
    fn nature_density_thins_only_vegetation_and_stably() {
        let tree = "data://res/nature/china/tree/tre_pine03.bsr";
        let house = "data://res/bldg/china/house01.bsr";
        assert!((0..1000).all(|key| keeps_nature_placement(house, key, 0.0)));
        assert!((0..1000).all(|key| keeps_nature_placement(tree, key, 1.0)));
        let kept = (0..10_000)
            .filter(|&key| keeps_nature_placement(tree, key, 0.6))
            .count();
        assert!(
            (5_700..6_300).contains(&kept),
            "kept {kept} of 10000 at 0.6"
        );
        // the same placement decides the same way every time
        for key in 0..100 {
            assert_eq!(
                keeps_nature_placement(tree, key, 0.6),
                keeps_nature_placement(tree, key, 0.6)
            );
        }
    }

    fn index(ids: &[u32]) -> ObjectInfoIndex {
        ObjectInfoIndex(
            ids.iter()
                .map(|&id| {
                    (
                        id,
                        ObjectInfo {
                            id,
                            flag: 0,
                            path: PathBuf::from("res/bldg/china/whatever.bsr"),
                        },
                    )
                })
                .collect(),
        )
    }

    /// A placed `ObjID` with no `object.ifo` row must skip the placement, not
    /// take the client down. The gap is real and large in the shipped data:
    /// `Map/object.ifo` declares 2767 rows (ids 0..2766) while `.o2` records
    /// reference ids up to 3253 — 357 distinct ids over 42,746 placements
    /// (18.5%) in 520 of 4,506 files (#429). `2767` below is exactly the first
    /// id past the table.
    #[test]
    fn an_objid_without_an_ifo_row_is_skipped_not_fatal() {
        let index = index(&[47, 2766]);
        let mut unknown = UnknownObjectIds::default();

        assert!(object_details(&index, 2766, &mut unknown).is_some());
        assert!(object_details(&index, 2767, &mut unknown).is_none());
        assert!(object_details(&index, 3253, &mut unknown).is_none());
        assert!(unknown.0.contains(&2767) && unknown.0.contains(&3253));
    }

    /// A region that has `Completed` keeps `TerrainObjectData` and is re-armed
    /// when any *other* region unloads — the lifetime half of #571. Without
    /// this, an object listed by both A and B but parented under A is gone for
    /// good the moment A unloads, even though B is still on screen.
    #[test]
    fn an_unloaded_region_re_arms_the_surviving_regions() {
        let mut app = App::new();
        app.add_systems(Update, rearm_object_passes_on_region_unload);

        // Two "completed" regions: object data kept, load state disarmed.
        let a = app
            .world_mut()
            .spawn((
                Terrain(TerrainId::from_x_z(1, 1)),
                TerrainObjectData(Handle::default()),
            ))
            .id();
        let b = app
            .world_mut()
            .spawn((
                Terrain(TerrainId::from_x_z(2, 1)),
                TerrainObjectData(Handle::default()),
            ))
            .id();

        // Nothing unloaded yet: no region may be re-armed, or every completed
        // region would re-run its whole object pass every single frame.
        app.update();
        assert!(app.world().get::<TerrainLoadState>(b).is_none());

        app.world_mut().entity_mut(a).despawn();
        app.update();

        assert!(
            matches!(
                app.world().get::<TerrainLoadState>(b),
                Some(TerrainLoadState::LoadedMeshes)
            ),
            "surviving region must re-run its object pass so it can re-adopt \
             objects that died with the unloaded region"
        );
    }

    /// The re-arm needs the region's `.o2` handle, so a region that no longer
    /// carries `TerrainObjectData` must be left alone rather than re-armed into
    /// a pass it cannot run (`load_terrain_objects_system` queries it).
    #[test]
    fn a_region_without_object_data_is_not_re_armed() {
        let mut app = App::new();
        app.add_systems(Update, rearm_object_passes_on_region_unload);

        let a = app
            .world_mut()
            .spawn((
                Terrain(TerrainId::from_x_z(1, 1)),
                TerrainObjectData(Handle::default()),
            ))
            .id();
        let bare = app
            .world_mut()
            .spawn(Terrain(TerrainId::from_x_z(3, 1)))
            .id();

        app.world_mut().entity_mut(a).despawn();
        app.update();

        assert!(app.world().get::<TerrainLoadState>(bare).is_none());
    }

    /// `SpawnedMapObjects` is keyed globally and self-heals by entity liveness,
    /// so the re-armed pass above actually re-spawns: the registry entry of an
    /// object that died with its region must read as absent, not as "already
    /// spawned". This is the other half of the fix — re-arming a region whose
    /// dedup still claimed the dead object would change nothing.
    #[test]
    fn a_dead_registry_entry_reads_as_not_spawned() {
        let mut world = World::new();
        let object = world.spawn_empty().id();
        let key: u32 = ((0x595c_u32) << 16) | 12290;
        let mut spawned = SpawnedMapObjects::default();
        spawned.0.insert(key, object);

        assert!(world.entities().contains(object));
        world.entity_mut(object).despawn();
        assert!(
            !world.entities().contains(spawned.0[&key]),
            "an object despawned with its region must not keep the key claimed"
        );
    }

    /// The warning is per id, not per placement: those 357 ids cover 42,746
    /// placements, and one line each is diagnosable where 42,746 is noise.
    #[test]
    fn unknown_objids_are_reported_once_each() {
        let index = index(&[1]);
        let mut unknown = UnknownObjectIds::default();

        // first sighting records it, every repeat is silent
        assert!(unknown.0.insert(9999));
        assert!(object_details(&index, 9999, &mut unknown).is_none());
        assert_eq!(unknown.0.len(), 1);
        for _ in 0..10 {
            assert!(object_details(&index, 9999, &mut unknown).is_none());
        }
        assert_eq!(unknown.0.len(), 1);
    }

    #[derive(Asset, TypePath)]
    struct TestVariant;

    /// The variant cache shares one asset per key — also within a frame,
    /// before the asset is even inserted — and rebuilds once the last user
    /// dropped it instead of pinning it for the session.
    #[test]
    fn variants_are_shared_while_alive_and_rebuilt_after_release() {
        let mut app = App::new();
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ))
        .init_asset::<TestVariant>();
        let server = app.world().resource::<AssetServer>().clone();
        let mut ids = HashMap::new();
        let key = (AssetId::<JMXVBMT>::default(), "gyo.sheen".to_string());
        let mut builds = 0;

        let first = resolve_variant(&mut ids, key.clone(), &server, || {
            builds += 1;
            Some(TestVariant)
        });
        let second = resolve_variant(&mut ids, key.clone(), &server, || {
            builds += 1;
            Some(TestVariant)
        });
        assert_eq!(first.id(), second.id(), "same-frame request must hit");
        assert_eq!(builds, 1);

        drop((first, second));
        app.update();
        let _third = resolve_variant(&mut ids, key, &server, || {
            builds += 1;
            Some(TestVariant)
        });
        assert_eq!(builds, 2, "a released variant is rebuilt, not resurrected");
    }

    #[test]
    fn a_missing_material_yields_the_default_handle() {
        let mut app = App::new();
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ))
        .init_asset::<TestVariant>();
        let server = app.world().resource::<AssetServer>().clone();
        let mut ids: HashMap<VariantKey, AssetId<TestVariant>> = HashMap::new();
        let key = (AssetId::<JMXVBMT>::default(), "missing.rim".to_string());
        let handle = resolve_variant(&mut ids, key, &server, || None);
        assert_eq!(handle, Handle::default());
        assert!(
            ids.is_empty(),
            "nothing cached for a material that never built"
        );
    }
}
