//! Instanced rendering for effect plates and meshes.
//!
//! Idea: effect particles stay entities for *simulation* (programs, graphs,
//! emission, pooling, billboarding all keep working on `Transform`s), but they
//! stop being individual render objects. Instead of `Mesh3d` +
//! `MeshMaterial3d` they carry [`EffectInstanced`]; once transforms and
//! visibility are final, [`gather_effect_instances`] packs every visible one
//! into a per-frame instance list, grouped by (effect root, mesh, material) and
//! sorted back to front inside each group. The render world uploads that list as
//! one instance-rate vertex buffer and draws each group with a single
//! `Transparent3d` item through a small custom pipeline
//! (`sro_effect_instanced.wgsl`, blend state shared with the material path via
//! `material::apply_effect_render_state`).
//!
//! Why: a waterfall's mist is ~2,300 particles, and as separate mesh entities
//! each one was visibility-tested, extracted, specialized, sorted and prepared
//! every frame — measured at 5–7 ms per frame with the waterfall in view.
//! Grouping per effect root keeps different effects sorting against each other
//! (and against water) as before; within a group, the CPU sort keeps alpha-
//! blended particles back to front. Modeled on Bevy's
//! `custom_shader_instancing` example, minus its mesh bind group: the per-
//! instance world matrix comes from the instance buffer, so the camera keeps
//! indirect drawing. Ribbon trails keep the `Material` path (their mesh is
//! rebuilt per node anyway).

use std::ops::Range;

use bevy::asset::AssetId;
use bevy::camera::visibility::{RenderLayers, VisibilitySystems};
use bevy::camera::RenderTarget;
use bevy::core_pipeline::core_3d::{Transparent3d, TransparentSortingInfo3d};
use bevy::ecs::entity::EntityHashMap;
use bevy::ecs::system::{lifetimeless::SRes, SystemParamItem};
use bevy::mesh::{MeshTag, MeshVertexBufferLayoutRef, VertexBufferLayout};
use bevy::pbr::{
    MeshPipeline, MeshPipelineKey, MeshPipelineSystems, SetMeshViewBindGroup,
    SetMeshViewBindingArrayBindGroup, ViewKeyCache,
};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::mesh::{RenderMesh, RenderMeshBufferInfo};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_phase::{
    AddRenderCommand, DrawFunctions, PhaseItem, PhaseItemExtraIndex, RenderCommand,
    RenderCommandResult, SetItemPipeline, TrackedRenderPass, ViewSortedRenderPhases,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::sync_world::MainEntity;
use bevy::render::texture::{FallbackImage, GpuImage};
use bevy::render::view::ExtractedView;
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::transform::TransformSystems;
use bevy::utils::Parallel;
use bytemuck::{Pod, Zeroable};

use crate::plugins::effects::components::{EffectNode, EffectVisual};
use crate::plugins::effects::material::{apply_effect_render_state, SroEffectMaterial};
use crate::plugins::effects::systems::main_world_camera;

const SHADER_PATH: &str = "shaders/sro_effect_instanced.wgsl";

/// The render half of an instanced effect node: which mesh and (shared)
/// material its instance is drawn with. Replaces `Mesh3d` + `MeshMaterial3d`
/// on effect plates/meshes; the tint stays in the node's `MeshTag` and the
/// TextureSlide UV in its `EffectVisual::last_uv`.
#[derive(Component, Clone)]
pub struct EffectInstanced {
    pub mesh: Handle<Mesh>,
    pub material: Handle<SroEffectMaterial>,
}

/// One particle as the instanced shader reads it (locations 8–13 in
/// `sro_effect_instanced.wgsl`).
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct EffectInstance {
    /// Column-major world-from-local matrix.
    world_from_local: [[f32; 4]; 4],
    /// xy = UV offset, zw = UV scale (TextureSlide).
    uv_offset_scale: [f32; 4],
    /// Packed 0xAARRGGBB tint, sRGB bytes.
    tint: u32,
    _pad: [u32; 3],
}

/// One draw: the instances of one effect root that share a mesh and a
/// material.
#[derive(Clone)]
pub struct EffectBatch {
    pub mesh: AssetId<Mesh>,
    pub material: AssetId<SroEffectMaterial>,
    pub layers: RenderLayers,
    /// Where the batch sorts against other transparent draws.
    pub center: Vec3,
    pub instances: Range<u32>,
}

/// This frame's instanced effect particles: rebuilt in `PostUpdate`, then
/// extracted to the render world.
#[derive(Resource, Default)]
pub struct EffectInstanceFrame {
    pub instances: Vec<EffectInstance>,
    pub batches: Vec<EffectBatch>,
}

const DEFAULT_UV: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

type BatchKey = (Entity, AssetId<Mesh>, AssetId<SroEffectMaterial>);

pub struct InstancedEffectsPlugin;

impl Plugin for InstancedEffectsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EffectInstanceFrame>().add_systems(
            PostUpdate,
            gather_effect_instances
                .after(TransformSystems::Propagate)
                .after(VisibilitySystems::VisibilityPropagate),
        );

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<RenderEffectInstances>()
            .init_resource::<EffectBatchEntities>()
            .init_resource::<EffectBatchLookup>()
            .init_resource::<EffectInstanceBuffer>()
            .init_resource::<EffectMaterialBindGroups>()
            .init_resource::<SpecializedMeshPipelines<EffectInstancedPipeline>>()
            .add_render_command::<Transparent3d, DrawEffectInstanced>()
            .add_systems(
                RenderStartup,
                init_effect_instanced_pipeline.after(MeshPipelineSystems),
            )
            .add_systems(ExtractSchedule, extract_effect_instances)
            .add_systems(
                Render,
                (
                    queue_effect_instances.in_set(RenderSystems::QueueMeshes),
                    prepare_effect_instance_buffer.in_set(RenderSystems::PrepareResources),
                    prepare_effect_material_bind_groups
                        .in_set(RenderSystems::PrepareBindGroups)
                        .after(bevy::render::render_asset::prepare_assets::<GpuImage>),
                ),
            );
    }
}

// ---------------------------------------------------------------------------
// Main world: gather
// ---------------------------------------------------------------------------

/// One visible particle on its way into the instance list.
struct GatheredInstance {
    key: BatchKey,
    /// Squared distance to the camera, for the back-to-front order.
    depth: f32,
    layers: Option<RenderLayers>,
    instance: EffectInstance,
}

/// Packs every visible instanced effect node into this frame's instance list.
/// Hidden nodes (paused or culled effects, parked pool particles, a disabled
/// effects toggle) are skipped through `InheritedVisibility`.
///
/// The per-node work runs across the compute pool into per-thread lists; one
/// flat sort by (batch key, farthest first) then lays the batches out
/// contiguously. Grouping through a hash map on one thread instead cost
/// ~0.6 ms per frame at the Jangan West waterfall (trace of 2026-10-05).
#[allow(clippy::type_complexity)]
fn gather_effect_instances(
    mut frame: ResMut<EffectInstanceFrame>,
    mut gathered: Local<Vec<GatheredInstance>>,
    mut per_thread: Local<Parallel<Vec<GatheredInstance>>>,
    cameras: Query<(&Camera, &RenderTarget, &GlobalTransform), With<Camera3d>>,
    nodes: Query<(
        &EffectInstanced,
        &EffectNode,
        &GlobalTransform,
        &InheritedVisibility,
        &MeshTag,
        Option<&EffectVisual>,
        Option<&RenderLayers>,
    )>,
) {
    let frame = &mut *frame;
    frame.instances.clear();
    frame.batches.clear();
    let camera = main_world_camera(&cameras).map_or(Vec3::ZERO, |t| t.translation());

    nodes.par_iter().for_each(
        |(instanced, node, global, inherited, tag, visual, layers)| {
            if !inherited.get() {
                return;
            }
            per_thread.borrow_local_mut().push(GatheredInstance {
                key: (node.root, instanced.mesh.id(), instanced.material.id()),
                depth: global.translation().distance_squared(camera),
                layers: layers.cloned(),
                instance: EffectInstance {
                    world_from_local: global.to_matrix().to_cols_array_2d(),
                    uv_offset_scale: visual.map_or(DEFAULT_UV, |v| v.last_uv.to_array()),
                    tint: tag.0,
                    _pad: [0; 3],
                },
            });
        },
    );
    gathered.clear();
    per_thread.drain_into(&mut gathered);

    // Batches in key order (the phase sort is stable, so batches of one
    // effect that tie on distance keep a fixed draw order); inside a batch
    // back to front (farthest first) for alpha-blended combos.
    gathered.sort_unstable_by(|a, b| a.key.cmp(&b.key).then(b.depth.total_cmp(&a.depth)));
    for group in gathered.chunk_by(|a, b| a.key == b.key) {
        let (_, mesh, material) = group[0].key;
        let start = frame.instances.len() as u32;
        let mut sum = Vec3::ZERO;
        for gathered in group {
            let t = gathered.instance.world_from_local[3];
            sum += Vec3::new(t[0], t[1], t[2]);
            frame.instances.push(gathered.instance);
        }
        frame.batches.push(EffectBatch {
            mesh,
            material,
            // one effect's nodes share their layers (the paper doll tags its
            // whole clone), so any node of the group decides
            layers: group[0].layers.clone().unwrap_or_default(),
            center: sum / group.len() as f32,
            instances: start..frame.instances.len() as u32,
        });
    }
}

// ---------------------------------------------------------------------------
// Render world: extract
// ---------------------------------------------------------------------------

/// The material values the instanced draw needs, extracted per frame for the
/// materials this frame's batches use.
#[derive(Clone, PartialEq)]
struct ExtractedEffectMaterial {
    texture: Option<AssetId<Image>>,
    params: Vec4,
    src_blend: u32,
    dst_blend: u32,
    ldr_additive: bool,
}

#[derive(Resource, Default)]
struct RenderEffectInstances {
    batches: Vec<EffectBatch>,
    materials: HashMap<AssetId<SroEffectMaterial>, ExtractedEffectMaterial>,
}

/// Render-world entities phase items are keyed by — one per batch slot,
/// reused every frame (sorted-phase items are keyed by entity, so each batch
/// needs its own; the items themselves are transient).
#[derive(Resource, Default)]
struct EffectBatchEntities(Vec<Entity>);

/// Phase-item entity → batch index, rebuilt in `queue_effect_instances`.
#[derive(Resource, Default)]
struct EffectBatchLookup(EntityHashMap<u32>);

fn extract_effect_instances(
    mut commands: Commands,
    mut render: ResMut<RenderEffectInstances>,
    mut buffer: ResMut<EffectInstanceBuffer>,
    mut pool: ResMut<EffectBatchEntities>,
    frame: Extract<Res<EffectInstanceFrame>>,
    materials: Extract<Res<Assets<SroEffectMaterial>>>,
) {
    let render = &mut *render;
    // staged straight into the upload buffer's CPU side (one copy)
    let staged = buffer.0.values_mut();
    staged.clear();
    staged.extend_from_slice(&frame.instances);
    render.batches.clone_from(&frame.batches);
    render.materials.clear();
    for batch in &frame.batches {
        if render.materials.contains_key(&batch.material) {
            continue;
        }
        if let Some(material) = materials.get(batch.material) {
            render.materials.insert(
                batch.material,
                ExtractedEffectMaterial {
                    texture: material.texture.as_ref().map(Handle::id),
                    params: material.params,
                    src_blend: material.src_blend,
                    dst_blend: material.dst_blend,
                    ldr_additive: material.ldr_additive,
                },
            );
        }
    }
    while pool.0.len() < frame.batches.len() {
        pool.0.push(commands.spawn_empty().id());
    }
}

// ---------------------------------------------------------------------------
// Render world: pipeline
// ---------------------------------------------------------------------------

#[derive(Resource)]
struct EffectInstancedPipeline {
    mesh_pipeline: MeshPipeline,
    shader: Handle<Shader>,
    material_layout: BindGroupLayout,
    material_layout_descriptor: BindGroupLayoutDescriptor,
}

fn init_effect_instanced_pipeline(
    mut commands: Commands,
    mesh_pipeline: Res<MeshPipeline>,
    asset_server: Res<AssetServer>,
    render_device: Res<RenderDevice>,
) {
    let entries = vec![
        BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                multisampled: false,
                sample_type: TextureSampleType::Float { filterable: true },
                view_dimension: TextureViewDimension::D2,
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 1,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Sampler(SamplerBindingType::Filtering),
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 2,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: BufferSize::new(16),
            },
            count: None,
        },
    ];
    let label = "sro_effect_instanced_material_layout";
    commands.insert_resource(EffectInstancedPipeline {
        mesh_pipeline: mesh_pipeline.clone(),
        shader: asset_server.load(SHADER_PATH),
        material_layout: render_device.create_bind_group_layout(label, &entries),
        material_layout_descriptor: BindGroupLayoutDescriptor::new(label, &entries),
    });
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct EffectInstancedKey {
    mesh_key: MeshPipelineKey,
    src_blend: u32,
    dst_blend: u32,
    ldr_additive: bool,
}

impl SpecializedMeshPipeline for EffectInstancedPipeline {
    type Key = EffectInstancedKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh_pipeline.specialize(key.mesh_key, layout)?;
        descriptor.label = Some("sro_effect_instanced_pipeline".into());
        // Keep the view groups (0: view, 1: view binding arrays); the mesh
        // group is replaced by the effect material — instances carry their
        // own world matrix, so no per-mesh data is bound.
        descriptor.layout.truncate(2);
        descriptor
            .layout
            .push(self.material_layout_descriptor.clone());
        descriptor.vertex.shader = self.shader.clone();
        descriptor.vertex.buffers.push(VertexBufferLayout {
            array_stride: size_of::<EffectInstance>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: vec![
                VertexAttribute {
                    format: VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 8,
                },
                VertexAttribute {
                    format: VertexFormat::Float32x4,
                    offset: 16,
                    shader_location: 9,
                },
                VertexAttribute {
                    format: VertexFormat::Float32x4,
                    offset: 32,
                    shader_location: 10,
                },
                VertexAttribute {
                    format: VertexFormat::Float32x4,
                    offset: 48,
                    shader_location: 11,
                },
                VertexAttribute {
                    format: VertexFormat::Float32x4,
                    offset: 64,
                    shader_location: 12,
                },
                VertexAttribute {
                    format: VertexFormat::Uint32,
                    offset: 80,
                    shader_location: 13,
                },
            ],
        });
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = self.shader.clone();
        }
        apply_effect_render_state(
            &mut descriptor,
            key.src_blend,
            key.dst_blend,
            key.ldr_additive,
        );
        Ok(descriptor)
    }
}

// ---------------------------------------------------------------------------
// Render world: queue / prepare
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn queue_effect_instances(
    draw_functions: Res<DrawFunctions<Transparent3d>>,
    pipeline: Option<Res<EffectInstancedPipeline>>,
    mut pipelines: ResMut<SpecializedMeshPipelines<EffectInstancedPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    instances: Res<RenderEffectInstances>,
    pool: Res<EffectBatchEntities>,
    mut lookup: ResMut<EffectBatchLookup>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    views: Query<(&ExtractedView, Option<&RenderLayers>)>,
    view_key_cache: Res<ViewKeyCache>,
) {
    lookup.0.clear();
    let Some(pipeline) = pipeline else { return };
    for (index, &entity) in pool.0.iter().take(instances.batches.len()).enumerate() {
        lookup.0.insert(entity, index as u32);
    }
    let draw_function = draw_functions.read().id::<DrawEffectInstanced>();
    let default_layers = RenderLayers::default();

    for (view, view_layers) in &views {
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Some(view_key) = view_key_cache.get(&view.retained_view_entity) else {
            continue;
        };
        let view_layers = view_layers.unwrap_or(&default_layers);
        for (index, batch) in instances.batches.iter().enumerate() {
            if !batch.layers.intersects(view_layers) {
                continue;
            }
            let (Some(&entity), Some(material), Some(mesh)) = (
                pool.0.get(index),
                instances.materials.get(&batch.material),
                render_meshes.get(batch.mesh),
            ) else {
                continue;
            };
            let key = EffectInstancedKey {
                mesh_key: *view_key
                    | MeshPipelineKey::BLEND_ALPHA
                    | MeshPipelineKey::from_primitive_topology_and_strip_index(
                        mesh.primitive_topology(),
                        mesh.index_format(),
                    ),
                src_blend: material.src_blend,
                dst_blend: material.dst_blend,
                ldr_additive: material.ldr_additive,
            };
            let pipeline_id =
                match pipelines.specialize(&pipeline_cache, &pipeline, key, &mesh.layout) {
                    Ok(id) => id,
                    Err(err) => {
                        error!("effect instanced pipeline specialize: {err}");
                        continue;
                    }
                };
            phase.add_transient(Transparent3d {
                sorting_info: TransparentSortingInfo3d::Sorted {
                    mesh_center: batch.center,
                    depth_bias: 0.0,
                },
                // A placeholder main entity: no mesh instance exists for it,
                // so Bevy's sorted-phase batching skips the item untouched.
                entity: (entity, MainEntity::from(Entity::PLACEHOLDER)),
                pipeline: pipeline_id,
                draw_function,
                distance: 0.0,
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::None,
                indexed: matches!(mesh.buffer_info, RenderMeshBufferInfo::Indexed { .. }),
            });
        }
    }
}

#[derive(Resource)]
struct EffectInstanceBuffer(RawBufferVec<EffectInstance>);

impl Default for EffectInstanceBuffer {
    fn default() -> Self {
        Self(RawBufferVec::new(BufferUsages::VERTEX))
    }
}

/// Uploads the instances `extract_effect_instances` staged in the buffer.
fn prepare_effect_instance_buffer(
    mut buffer: ResMut<EffectInstanceBuffer>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    if !buffer.0.is_empty() {
        buffer.0.write_buffer(&render_device, &render_queue);
    }
}

/// Bind group per material in use, rebuilt only when its extracted values
/// change (the render-debug intensity / LDR toggles edit material assets).
#[derive(Resource, Default)]
struct EffectMaterialBindGroups(
    HashMap<AssetId<SroEffectMaterial>, (ExtractedEffectMaterial, BindGroup)>,
);

fn prepare_effect_material_bind_groups(
    mut bind_groups: ResMut<EffectMaterialBindGroups>,
    pipeline: Option<Res<EffectInstancedPipeline>>,
    instances: Res<RenderEffectInstances>,
    images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    render_device: Res<RenderDevice>,
) {
    let Some(pipeline) = pipeline else { return };
    bind_groups
        .0
        .retain(|id, _| instances.materials.contains_key(id));
    for (id, material) in &instances.materials {
        if bind_groups
            .0
            .get(id)
            .is_some_and(|(cached, _)| cached == material)
        {
            continue;
        }
        let image = match material.texture {
            Some(texture) => match images.get(texture) {
                Some(image) => image,
                None => continue, // still uploading; retried next frame
            },
            // no texture: white, as the Material path's fallback image
            None => &fallback.d2,
        };
        let params = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("sro_effect_instanced_params"),
            contents: bytemuck::cast_slice(&material.params.to_array()),
            usage: BufferUsages::UNIFORM,
        });
        let bind_group = render_device.create_bind_group(
            "sro_effect_instanced_material",
            &pipeline.material_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&image.texture_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&image.sampler),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
            ],
        );
        bind_groups.0.insert(*id, (material.clone(), bind_group));
    }
}

// ---------------------------------------------------------------------------
// Render world: draw
// ---------------------------------------------------------------------------

type DrawEffectInstanced = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    SetEffectMaterialBindGroup<2>,
    DrawEffectBatch,
);

struct SetEffectMaterialBindGroup<const I: usize>;

impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetEffectMaterialBindGroup<I> {
    type Param = (
        SRes<EffectBatchLookup>,
        SRes<RenderEffectInstances>,
        SRes<EffectMaterialBindGroups>,
    );
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        item: &P,
        _view: (),
        _entity: Option<()>,
        (lookup, instances, bind_groups): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(&index) = lookup.into_inner().0.get(&item.entity()) else {
            return RenderCommandResult::Skip;
        };
        let Some(batch) = instances.into_inner().batches.get(index as usize) else {
            return RenderCommandResult::Skip;
        };
        let Some((_, bind_group)) = bind_groups.into_inner().0.get(&batch.material) else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(I, bind_group, &[]);
        RenderCommandResult::Success
    }
}

struct DrawEffectBatch;

impl<P: PhaseItem> RenderCommand<P> for DrawEffectBatch {
    type Param = (
        SRes<EffectBatchLookup>,
        SRes<RenderEffectInstances>,
        SRes<EffectInstanceBuffer>,
        SRes<RenderAssets<RenderMesh>>,
        SRes<MeshAllocator>,
    );
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        item: &P,
        _view: (),
        _entity: Option<()>,
        (lookup, instances, instance_buffer, meshes, mesh_allocator): SystemParamItem<
            'w,
            '_,
            Self::Param,
        >,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let mesh_allocator = mesh_allocator.into_inner();
        let Some(&index) = lookup.into_inner().0.get(&item.entity()) else {
            return RenderCommandResult::Skip;
        };
        let Some(batch) = instances.into_inner().batches.get(index as usize) else {
            return RenderCommandResult::Skip;
        };
        let Some(instance_buffer) = instance_buffer.into_inner().0.buffer() else {
            return RenderCommandResult::Skip;
        };
        let Some(gpu_mesh) = meshes.into_inner().get(batch.mesh) else {
            return RenderCommandResult::Skip;
        };
        let Some(vertex_slice) = mesh_allocator.mesh_vertex_slice(&batch.mesh) else {
            return RenderCommandResult::Skip;
        };

        pass.set_vertex_buffer(0, vertex_slice.buffer.slice(..));
        pass.set_vertex_buffer(1, instance_buffer.slice(..));
        match &gpu_mesh.buffer_info {
            RenderMeshBufferInfo::Indexed {
                index_format,
                count,
            } => {
                let Some(index_slice) = mesh_allocator.mesh_index_slice(&batch.mesh) else {
                    return RenderCommandResult::Skip;
                };
                pass.set_index_buffer(index_slice.buffer.slice(..), *index_format);
                pass.draw_indexed(
                    index_slice.range.start..(index_slice.range.start + count),
                    vertex_slice.range.start as i32,
                    batch.instances.clone(),
                );
            }
            RenderMeshBufferInfo::NonIndexed => {
                pass.draw(vertex_slice.range, batch.instances.clone());
            }
        }
        RenderCommandResult::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::uuid_handle;
    use bevy::camera::RenderTarget;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::window::WindowRef;

    use crate::plugins::effects::components::Lifespan;

    const QUAD: Handle<Mesh> = uuid_handle!("6a1c0f3e-2a55-4d3b-9b1e-6f2f6c0d7a01");
    const GLOW: Handle<SroEffectMaterial> = uuid_handle!("6a1c0f3e-2a55-4d3b-9b1e-6f2f6c0d7a02");

    fn node(world: &mut World, root: Entity, at: Vec3, visible: bool) {
        world.spawn((
            EffectInstanced {
                mesh: QUAD,
                material: GLOW,
            },
            EffectNode {
                handle: Handle::default(),
                node: 0,
                age: 0.0,
                lifespan: Lifespan::Once(1.0),
                root,
            },
            GlobalTransform::from_translation(at),
            if visible {
                InheritedVisibility::VISIBLE
            } else {
                InheritedVisibility::HIDDEN
            },
            MeshTag(0xFF80_4020),
        ));
    }

    #[test]
    fn gathers_one_back_to_front_batch_per_effect() {
        let mut world = World::new();
        world.init_resource::<EffectInstanceFrame>();
        world.spawn((
            Camera3d::default(),
            Camera::default(),
            RenderTarget::Window(WindowRef::Primary),
            GlobalTransform::IDENTITY,
        ));
        let fountain = world.spawn_empty().id();
        let torch = world.spawn_empty().id();
        node(&mut world, fountain, Vec3::new(0.0, 0.0, -1.0), true);
        node(&mut world, fountain, Vec3::new(0.0, 0.0, -5.0), true);
        node(&mut world, fountain, Vec3::new(0.0, 0.0, -3.0), true);
        // parked pool particle / culled effect: not drawn
        node(&mut world, fountain, Vec3::new(0.0, 0.0, -9.0), false);
        node(&mut world, torch, Vec3::new(4.0, 0.0, 0.0), true);

        world
            .run_system_once(gather_effect_instances)
            .expect("system runs");

        let frame = world.resource::<EffectInstanceFrame>();
        assert_eq!(frame.instances.len(), 4);
        assert_eq!(frame.batches.len(), 2, "one batch per effect root");
        let fountain_batch = frame
            .batches
            .iter()
            .find(|b| b.instances.len() == 3)
            .expect("fountain batch");
        let depths: Vec<f32> = frame.instances
            [fountain_batch.instances.start as usize..fountain_batch.instances.end as usize]
            .iter()
            .map(|i| i.world_from_local[3][2])
            .collect();
        assert_eq!(depths, [-5.0, -3.0, -1.0], "farthest first");
        assert_eq!(fountain_batch.center, Vec3::new(0.0, 0.0, -3.0));
        let instance = frame.instances[0];
        assert_eq!(instance.tint, 0xFF80_4020);
        assert_eq!(instance.uv_offset_scale, DEFAULT_UV, "no EffectVisual");
    }
}
