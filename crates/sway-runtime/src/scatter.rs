//! Compute-cooked scatter: `scatter.wgsl` (Task 2) filled by the GPU, driven
//! by a dirty set so each source cooks once rather than every frame.
//!
//! Adapted from Bevy 0.19's `shader_advanced/compute_mesh.rs` example, the
//! closest thing in the example set to spec §2.10's "compute shader fills
//! a GPU buffer, no CPU round trip" description. See task-5-brief.md for the
//! deltas this module applies; the important ones are noted inline below.
//!
//! ## Delta 5 — how far this got
//!
//! The brief's preferred bridge is: scatter writes positions, and the point
//! cloud's (Task 3) instanced draw reads them directly as its per-instance
//! vertex buffer, with no CPU round trip anywhere.
//!
//! That bridge is not what this module does — but it is not blocked by a
//! fundamental format incompatibility between fixed contracts. The two
//! formats really do differ: `scatter.wgsl` (not to be modified) writes
//! *only* `xyz` position triples, 12 bytes per point, while
//! `point_cloud.rs`'s `PointInstance` is a 32-byte interleaved
//! position+scale+colour record. But `point_cloud.rs`'s
//! `SpecializedMeshPipeline::specialize` (around lines 305-334) already uses
//! independently-strided vertex buffer slots — the mesh's own attributes in
//! one slot, the per-instance `PointInstance` data pushed as a separate
//! slot. A *second*, non-interleaved vertex buffer slot fed directly from
//! scatter's raw position buffer, paired with a hardcoded uniform scale and
//! colour, would bridge the two formats with no second compute pass needed.
//! Only `scatter.wgsl` was ever a fixed contract for this task;
//! `point_cloud.wgsl` and `point_cloud.rs`'s instance layout were never
//! declared immutable — they simply belong to Task 3, and this task's Files
//! list (`scatter.rs`, `lib.rs`) doesn't include them.
//!
//! So the real finding is narrower than "hard incompatibility": level (a)
//! was **unreachable within Task 5's own stated file scope**, not blocked by
//! the data formats themselves. A modest instance-layout change on Task 3's
//! side (`point_cloud.rs`/`point_cloud.wgsl`) would close the gap without a
//! second GPU-side expansion pass.
//!
//! This module implements level **(b)** from the spike brief: compute writes
//! the buffer on the GPU via the dirty-set-gated pipeline below. The
//! `gpu_readback.rs`-style readback that was used to verify values during
//! development has been removed with the rest of the demo scaffolding.
//!
//! ## Delta 1 — the dirty set (the actual point of this task)
//!
//! `queue_scatter_jobs` below cooks a given [`ScatterSource`] (identified by
//! its output buffer's `AssetId`) at most once: a `Local` set of already-seen
//! ids, checked and updated by the pure [`jobs_to_cook`] helper, means a
//! source that is still present next frame produces an empty job list rather
//! than a repeat dispatch. `dispatch_scatter` only ever iterates whatever
//! `queue_scatter_jobs` put in the queue that frame, so a source with no
//! fresh job in the queue causes no `dispatch_workgroups` call at all: cooked
//! once, not per frame. See `jobs_to_cook_only_returns_each_id_once` for the
//! unit test that pins this down independent of any render world.
//!
//! ## Delta 4 — `extra_buffer_usages` was not needed
//!
//! The reference example needs `MeshAllocatorSettings::extra_buffer_usages =
//! BufferUsages::STORAGE` because it writes into the mesh allocator's own
//! vertex/index slabs, which are not created with `STORAGE` usage by
//! default. This module writes into a plain [`ShaderBuffer`] asset instead,
//! whose default `buffer_description.usage` is already
//! `STORAGE | COPY_SRC | COPY_DST` — storage-bindable out of the box.
//! Nothing to opt into.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::schedule::camera_driver;
use bevy::platform::collections::HashSet as BevyHashSet;
use bevy::prelude::*;
use bevy::render::{
    Render, RenderApp, RenderStartup,
    extract_component::{ExtractComponent, ExtractComponentPlugin},
    render_asset::RenderAssets,
    render_resource::{
        binding_types::{storage_buffer, uniform_buffer},
        *,
    },
    renderer::{RenderContext, RenderGraph, RenderQueue},
    storage::{GpuShaderBuffer, ShaderBuffer},
};

/// Registers the compute-cooked scatter pipeline and its dirty-set-driven
/// dispatch. Structurally mirrors `compute_mesh.rs`'s
/// `ComputeShaderMeshGeneratorPlugin`, minus its `finish()` hook — see the
/// module doc's delta-4 note for why that hook has no equivalent here.
pub struct ScatterPlugin;

impl Plugin for ScatterPlugin {
    fn build(&self, app: &mut App) {
        // Compiles `scatter.wgsl` into the binary and registers it under the
        // `embedded://` asset source, so the shader is reachable without any
        // filesystem path resolution relative to a running binary's location
        // (see `init_scatter_pipeline`'s matching `load_embedded_asset!`
        // call below).
        embedded_asset!(app, "../assets/shaders/scatter.wgsl");

        app.add_plugins(ExtractComponentPlugin::<ScatterSource>::default());

        app.sub_app_mut(RenderApp)
            .init_resource::<ScatterQueue>()
            .add_systems(RenderStartup, init_scatter_pipeline)
            .add_systems(Render, queue_scatter_jobs)
            // `RenderGraph` here is a *schedule* (`bevy::render::renderer::RenderGraph`),
            // not the node-based render graph from older Bevy — see the task
            // brief and the report for this surprise. Ordering before
            // `camera_driver` is what makes the compute dispatch (and its
            // buffer writes) happen before the draw that would consume them.
            .add_systems(RenderGraph, dispatch_scatter.before(camera_driver));
    }
}

/// A scatter job: fill `buffer` with `params.count` GPU-computed points.
/// Extracted into the render world by `ExtractComponentPlugin` below;
/// `buffer`'s `AssetId` doubles as this job's identity for the dirty set.
#[derive(Component, ExtractComponent, Clone)]
struct ScatterSource {
    buffer: Handle<ShaderBuffer>,
    params: ScatterParams,
}

/// Mirrors `scatter.wgsl`'s `ScatterParams` uniform exactly: same fields,
/// same order, same types. Binding 0 of the compute bind group.
#[derive(Clone, Copy, ShaderType)]
struct ScatterParams {
    count: u32,
    seed: u32,
    extent: f32,
    _pad: f32,
}

/// Render-world queue of jobs to cook *this frame* — populated fresh each
/// frame by `queue_scatter_jobs`, and empty whenever nothing new needs
/// cooking. Named after `compute_mesh.rs`'s `ChunksToProcess`.
#[derive(Resource, Default)]
struct ScatterQueue(Vec<(AssetId<ShaderBuffer>, ScatterParams)>);

#[derive(Resource)]
struct ScatterPipeline {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
}

fn init_scatter_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    // Binding 0: ScatterParams uniform. Binding 1: read_write storage buffer
    // of f32 — matches `scatter.wgsl`'s two `@group(0)` bindings exactly.
    let layout = BindGroupLayoutDescriptor::new(
        "scatter_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<ScatterParams>(false),
                storage_buffer::<Vec<f32>>(false),
            ),
        ),
    );
    let shader = load_embedded_asset!(asset_server.as_ref(), "../assets/shaders/scatter.wgsl");
    let pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("scatter compute pipeline".into()),
        layout: vec![layout.clone()],
        shader,
        ..default()
    });
    commands.insert_resource(ScatterPipeline { layout, pipeline });
}

/// Delta 1: the dirty set. Cooks a given `ScatterSource` (identified by its
/// output buffer's `AssetId`) at most once — a source still present next
/// frame produces no job, so `dispatch_scatter` dispatches nothing for it.
/// See the module doc's "Delta 1" section and `jobs_to_cook` below.
fn queue_scatter_jobs(
    sources: Query<&ScatterSource>,
    mut queue: ResMut<ScatterQueue>,
    pipeline_cache: Res<PipelineCache>,
    pipeline: Res<ScatterPipeline>,
    mut processed: Local<BevyHashSet<AssetId<ShaderBuffer>>>,
) {
    // As in `compute_mesh.rs`'s `prepare_chunks`: don't mark a source as
    // processed until the pipeline actually exists, so a source seen before
    // the pipeline finishes compiling isn't silently dropped forever.
    if pipeline_cache
        .get_compute_pipeline(pipeline.pipeline)
        .is_none()
    {
        return;
    }

    queue.0 = jobs_to_cook(
        &mut processed,
        sources.iter().map(|s| (s.buffer.id(), s.params)),
    );
}

/// The dirty-set core, factored out of `queue_scatter_jobs` so it can be
/// tested without a render world: returns the subset of `candidates` whose
/// id is not already in `processed`, and marks those ids processed. Calling
/// this twice with the same ids returns nothing the second time — that is
/// exactly the "cook once, not per frame" behaviour delta 1 asks for.
fn jobs_to_cook<Id: Eq + std::hash::Hash + Copy, V>(
    processed: &mut BevyHashSet<Id>,
    candidates: impl IntoIterator<Item = (Id, V)>,
) -> Vec<(Id, V)> {
    let fresh: Vec<(Id, V)> = candidates
        .into_iter()
        .filter(|(id, _)| !processed.contains(id))
        .collect();
    processed.extend(fresh.iter().map(|(id, _)| *id));
    fresh
}

/// Workgroup count for `count` invocations at the shader's
/// `@workgroup_size(64)`: delta 3 from the brief.
fn workgroup_count(count: u32, workgroup_size: u32) -> u32 {
    count.div_ceil(workgroup_size)
}

fn dispatch_scatter(
    mut render_context: RenderContext,
    queue: Res<ScatterQueue>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    pipeline_cache: Res<PipelineCache>,
    pipeline: Res<ScatterPipeline>,
    render_queue: Res<RenderQueue>,
) {
    let Some(compute_pipeline) = pipeline_cache.get_compute_pipeline(pipeline.pipeline) else {
        return;
    };

    for (buffer_id, params) in &queue.0 {
        let Some(gpu_buffer) = buffers.get(*buffer_id) else {
            // `queue_scatter_jobs` already marked this id as processed before
            // this system ever ran, so skipping it here drops the job
            // permanently: the dirty set will never re-offer this id. That
            // is only expected if the buffer asset was despawned/unloaded
            // between queuing and dispatch (same-frame race), which should
            // not happen for the demo's `ScatterSource`/`ShaderBuffer`
            // lifetime. Log loudly rather than silently swallowing it so a
            // real occurrence is visible instead of a scatter source quietly
            // never getting its GPU-computed data.
            error!(
                "scatter: output buffer {buffer_id:?} not resolvable in \
                 RenderAssets<GpuShaderBuffer> at dispatch time; dropping \
                 this scatter job permanently (it will not be retried)"
            );
            continue;
        };

        let mut uniforms = UniformBuffer::from(*params);
        uniforms.write_buffer(render_context.render_device(), &render_queue);

        let bind_group = render_context.render_device().create_bind_group(
            None,
            &pipeline_cache.get_bind_group_layout(&pipeline.layout),
            &BindGroupEntries::sequential((
                &uniforms,
                gpu_buffer.buffer.as_entire_buffer_binding(),
            )),
        );

        let mut pass =
            render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor {
                    label: Some("scatter compute pass"),
                    ..default()
                });
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_pipeline(compute_pipeline);
        pass.dispatch_workgroups(workgroup_count(params.count, 64), 1, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workgroup_count_matches_shader_workgroup_size() {
        assert_eq!(workgroup_count(0, 64), 0);
        assert_eq!(workgroup_count(1, 64), 1);
        assert_eq!(workgroup_count(64, 64), 1);
        assert_eq!(workgroup_count(65, 64), 2);
        // matches scatter.wgsl's `@workgroup_size(64)` at point_cloud.rs's demo scale
        assert_eq!(workgroup_count(50_000, 64), 782);
    }

    #[test]
    fn jobs_to_cook_only_returns_each_id_once() {
        let mut processed = BevyHashSet::default();

        let first = jobs_to_cook(&mut processed, [(1u32, "a"), (2u32, "b")]);
        assert_eq!(first, vec![(1, "a"), (2, "b")]);

        // Same ids requeried, as a source still present next frame would
        // produce: nothing new to cook. This is the dirty-set behaviour
        // delta 1 requires — a source cooks once, not once per frame.
        let second = jobs_to_cook(&mut processed, [(1u32, "a"), (2u32, "b")]);
        assert!(second.is_empty());

        // A new id mixed in with already-seen ones: only the new one comes back.
        let third = jobs_to_cook(&mut processed, [(1u32, "a"), (3u32, "c")]);
        assert_eq!(third, vec![(3, "c")]);
    }
}
