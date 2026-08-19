//! Instanced point-cloud render pipeline.
//!
//! Adapted from Bevy 0.19's `shader_advanced/custom_shader_instancing.rs`
//! example, which draws one mesh many times in a single instanced draw call
//! with a custom vertex/fragment shader, custom `SpecializedMeshPipeline` and
//! a custom `RenderCommand`. A point cloud is structurally the same problem:
//! one small mesh, tens of thousands of instances, one draw call.
//!
//! Deltas from the reference example (see task-3-brief.md):
//! 1. Instance data keeps the example's shape: `position: Vec3`, `scale:
//!    f32`, `color: [f32; 4]` — per-point scale is exactly what a point
//!    cloud wants.
//! 2. Base mesh is a low-subdivision icosphere instead of a cuboid: a cuboid
//!    per point is wasteful at 50k instances, and a flat quad would need
//!    camera-facing (billboard) logic in the vertex shader to avoid
//!    disappearing edge-on. An icosphere with `subdivisions = 1` (42
//!    vertices, 80 triangles) looks correct from any angle with nothing more
//!    than translate + scale, at a fraction of a cuboid's vertex cost.
//! 3. Demo data is 50,000 points on a fibonacci sphere (golden-angle
//!    spiral), not the example's 10x10 grid of 100 cubes — the whole point
//!    of this milestone is finding out whether the approach holds at scale.
//! 4. The `DrawMeshInstanced` command below uses `draw`/`draw_indexed`
//!    rather than indirect variants; callers must attach `NoFrustumCulling`
//!    to point-cloud mesh entities (no `GlobalTransform` per point to cull
//!    against) and `NoIndirectDrawing` to the camera.
//! 5. `point_cloud.wgsl` uses Bevy's `#import` preprocessor (for
//!    `mesh_functions`), so naga cannot parse it; it is listed in
//!    `PREPROCESSOR_SHADERS` in `shader_validation.rs` as a deliberate,
//!    reviewed skip.
//! 6. Two per-frame costs are inherited from the reference example, not
//!    chosen. `ExtractComponentPlugin::<PointCloudData>` is registered with
//!    `QueryFilter = ()`, verbatim from the example, which runs the default
//!    (not visibility-gated) `extract_components` system every
//!    `ExtractSchedule`; that system calls `PointCloudData::extract_component`
//!    below, which clones the entire 50,000-element `Vec<PointInstance>`
//!    (~1.6MB) on the CPU every frame. `prepare_instance_buffers` then
//!    rebuilds and re-uploads a matching ~1.6MB GPU buffer every frame on
//!    top of that. Neither is amortized across frames for this static demo
//!    data.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::core_3d::TransparentSortingInfo3d;
use bevy::pbr::{
    self, MeshInputUniform, MeshPipelineSystems, MeshUniform, SetMeshViewBindingArrayBindGroup,
    ViewKeyCache,
};
use bevy::{
    core_pipeline::core_3d::Transparent3d,
    ecs::{
        query::QueryItem,
        system::{SystemParamItem, lifetimeless::*},
    },
    mesh::{MeshVertexBufferLayoutRef, VertexBufferLayout},
    pbr::{
        MeshPipeline, MeshPipelineKey, RenderMeshInstances, SetMeshBindGroup, SetMeshViewBindGroup,
    },
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        batching::gpu_preprocessing::BatchedInstanceBuffers,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        mesh::{RenderMesh, RenderMeshBufferInfo, allocator::MeshAllocator},
        render_asset::RenderAssets,
        render_phase::{
            AddRenderCommand, DrawFunctions, PhaseItem, PhaseItemExtraIndex, RenderCommand,
            RenderCommandResult, SetItemPipeline, TrackedRenderPass, ViewSortedRenderPhases,
        },
        render_resource::*,
        renderer::RenderDevice,
        sync_component::SyncComponent,
        sync_world::MainEntity,
        view::ExtractedView,
    },
};
use bytemuck::{Pod, Zeroable};

/// Per-instance data for one point: world position, point scale, and RGBA
/// colour. Matches the vertex-buffer layout declared in
/// `PointCloudPipeline::specialize` and the `Vertex` struct in
/// `point_cloud.wgsl`.
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct PointInstance {
    pub position: Vec3,
    pub scale: f32,
    pub color: [f32; 4],
}

/// The set of points to draw for one instanced entity.
#[derive(Component, Deref)]
pub struct PointCloudData(pub Vec<PointInstance>);

impl SyncComponent for PointCloudData {
    type Target = Self;
}

impl ExtractComponent for PointCloudData {
    type QueryData = &'static PointCloudData;
    type QueryFilter = ();
    type Out = Self;

    fn extract_component(item: QueryItem<'_, '_, Self::QueryData>) -> Option<Self> {
        Some(PointCloudData(item.0.clone()))
    }
}

/// Registers the custom instanced draw path for `PointCloudData` entities.
/// Structurally identical to the reference example's `CustomMaterialPlugin`.
pub struct PointCloudPlugin;

impl Plugin for PointCloudPlugin {
    fn build(&self, app: &mut App) {
        // Compiles `point_cloud.wgsl` into the binary and registers it under
        // the `embedded://` asset source, so the shader is reachable without
        // any filesystem path resolution relative to a running binary's
        // location (see `init_point_cloud_pipeline`'s matching
        // `load_embedded_asset!` call below).
        embedded_asset!(app, "../assets/shaders/point_cloud.wgsl");

        app.add_plugins(ExtractComponentPlugin::<PointCloudData>::default());
        app.sub_app_mut(RenderApp)
            .add_render_command::<Transparent3d, DrawPointCloud>()
            .init_resource::<SpecializedMeshPipelines<PointCloudPipeline>>()
            .add_systems(
                RenderStartup,
                init_point_cloud_pipeline.after(MeshPipelineSystems),
            )
            .add_systems(
                Render,
                (
                    queue_point_cloud.in_set(RenderSystems::QueueMeshes),
                    prepare_instance_buffers.in_set(RenderSystems::PrepareResources),
                ),
            );
    }
}

// Bevy's own workspace lints blanket-allow this: systems take one parameter
// per resource/query they need, and this one is a direct port of the
// reference example's `queue_custom`, which has the same arity.
#[allow(clippy::too_many_arguments)]
fn queue_point_cloud(
    transparent_3d_draw_functions: Res<DrawFunctions<Transparent3d>>,
    point_cloud_pipeline: Res<PointCloudPipeline>,
    mut pipelines: ResMut<SpecializedMeshPipelines<PointCloudPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    meshes: Res<RenderAssets<RenderMesh>>,
    render_mesh_instances: Res<RenderMeshInstances>,
    maybe_batched_instance_buffers: Option<
        Res<BatchedInstanceBuffers<MeshUniform, MeshInputUniform>>,
    >,
    material_meshes: Query<(Entity, &MainEntity), With<PointCloudData>>,
    mut transparent_render_phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    views: Query<&ExtractedView>,
    view_key_cache: Res<ViewKeyCache>,
) {
    let draw_point_cloud = transparent_3d_draw_functions.read().id::<DrawPointCloud>();

    for view in &views {
        let Some(transparent_phase) = transparent_render_phases.get_mut(&view.retained_view_entity)
        else {
            continue;
        };

        let Some(&view_key) = view_key_cache.get(&view.retained_view_entity) else {
            continue;
        };

        for (entity, main_entity) in &material_meshes {
            let Some(mesh_instance) = render_mesh_instances.render_mesh_queue_data(*main_entity)
            else {
                continue;
            };
            let Some(mesh) = meshes.get(mesh_instance.mesh_asset_id()) else {
                continue;
            };
            let key = view_key
                | MeshPipelineKey::from_primitive_topology_and_strip_index(
                    mesh.primitive_topology(),
                    mesh.index_format(),
                );
            let pipeline = pipelines
                .specialize(&pipeline_cache, &point_cloud_pipeline, key, &mesh.layout)
                .unwrap();
            transparent_phase.add_retained(Transparent3d {
                sorting_info: TransparentSortingInfo3d::Sorted {
                    mesh_center: pbr::get_mesh_instance_world_from_local(
                        *main_entity,
                        mesh_instance.current_uniform_index,
                        &render_mesh_instances,
                        maybe_batched_instance_buffers.as_deref(),
                    )
                    .transform_point3(mesh.aabb_center),
                    depth_bias: 0.0,
                },
                entity: (entity, *main_entity),
                pipeline,
                draw_function: draw_point_cloud,
                distance: 0.0,
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::None,
                indexed: true,
            });
        }
    }
}

#[derive(Component)]
struct InstanceBuffer {
    buffer: Buffer,
    length: usize,
}

/// Rebuilds and re-uploads the full ~1.6MB instance buffer from scratch
/// every frame via `create_buffer_with_data` — the GPU-side half of the
/// per-frame cost recorded in the module doc (delta 6); the CPU-side half is
/// the `Vec<PointInstance>` clone in `PointCloudData::extract_component`
/// above. Both are inherited from the reference example, not chosen, and
/// neither is amortized for this static demo data.
fn prepare_instance_buffers(
    mut commands: Commands,
    query: Query<(Entity, &PointCloudData)>,
    render_device: Res<RenderDevice>,
) {
    for (entity, instance_data) in &query {
        let buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("point cloud instance buffer"),
            contents: bytemuck::cast_slice(instance_data.as_slice()),
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
        });
        commands.entity(entity).insert(InstanceBuffer {
            buffer,
            length: instance_data.len(),
        });
    }
}

#[derive(Resource)]
struct PointCloudPipeline {
    shader: Handle<Shader>,
    mesh_pipeline: MeshPipeline,
}

fn init_point_cloud_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mesh_pipeline: Res<MeshPipeline>,
) {
    commands.insert_resource(PointCloudPipeline {
        shader: load_embedded_asset!(asset_server.as_ref(), "../assets/shaders/point_cloud.wgsl"),
        mesh_pipeline: mesh_pipeline.clone(),
    });
}

impl SpecializedMeshPipeline for PointCloudPipeline {
    type Key = MeshPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh_pipeline.specialize(key, layout)?;

        descriptor.vertex.shader = self.shader.clone();
        descriptor.vertex.buffers.push(VertexBufferLayout {
            array_stride: size_of::<PointInstance>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: vec![
                VertexAttribute {
                    format: VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 3, // shader locations 0-2 are taken up by Position, Normal and UV attributes
                },
                VertexAttribute {
                    format: VertexFormat::Float32x4,
                    offset: VertexFormat::Float32x4.size(),
                    shader_location: 4,
                },
            ],
        });
        descriptor.fragment.as_mut().unwrap().shader = self.shader.clone();
        Ok(descriptor)
    }
}

type DrawPointCloud = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    SetMeshBindGroup<2>,
    DrawMeshInstanced,
);

struct DrawMeshInstanced;

impl<P: PhaseItem> RenderCommand<P> for DrawMeshInstanced {
    type Param = (
        SRes<RenderAssets<RenderMesh>>,
        SRes<RenderMeshInstances>,
        SRes<MeshAllocator>,
    );
    type ViewQuery = ();
    type ItemQuery = Read<InstanceBuffer>;

    #[inline]
    fn render<'w>(
        item: &P,
        _view: (),
        instance_buffer: Option<&'w InstanceBuffer>,
        (meshes, render_mesh_instances, mesh_allocator): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        // A borrow check workaround.
        let mesh_allocator = mesh_allocator.into_inner();

        let Some(mesh_instance) = render_mesh_instances.render_mesh_queue_data(item.main_entity())
        else {
            return RenderCommandResult::Skip;
        };
        let Some(gpu_mesh) = meshes.into_inner().get(mesh_instance.mesh_asset_id()) else {
            return RenderCommandResult::Skip;
        };
        let Some(instance_buffer) = instance_buffer else {
            return RenderCommandResult::Skip;
        };
        let Some(vertex_buffer_slice) =
            mesh_allocator.mesh_vertex_slice(&mesh_instance.mesh_asset_id())
        else {
            return RenderCommandResult::Skip;
        };

        pass.set_vertex_buffer(0, vertex_buffer_slice.buffer.slice(..));
        pass.set_vertex_buffer(1, instance_buffer.buffer.slice(..));

        match &gpu_mesh.buffer_info {
            RenderMeshBufferInfo::Indexed {
                index_format,
                count,
            } => {
                let Some(index_buffer_slice) =
                    mesh_allocator.mesh_index_slice(&mesh_instance.mesh_asset_id())
                else {
                    return RenderCommandResult::Skip;
                };

                pass.set_index_buffer(index_buffer_slice.buffer.slice(..), *index_format);
                pass.draw_indexed(
                    index_buffer_slice.range.start..(index_buffer_slice.range.start + count),
                    vertex_buffer_slice.range.start as i32,
                    0..instance_buffer.length as u32,
                );
            }
            RenderMeshBufferInfo::NonIndexed => {
                pass.draw(vertex_buffer_slice.range, 0..instance_buffer.length as u32);
            }
        }
        RenderCommandResult::Success
    }
}
