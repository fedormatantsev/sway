# M1 render spike — findings

Consolidated, tracked answers to the four questions the plan
(`docs/superpowers/plans/2026-07-26-m1-render-spike.md`, "What this milestone
must produce besides code") requires this milestone to record. Sourced from
`.superpowers/sdd/progress.md` and the `m1-task-*-report.md` files, which are
gitignored scratch and not a durable record on their own.

## 1. Point cloud: custom pipeline or `Material` path, and why

Custom `SpecializedMeshPipeline` (Task 3), not the `Material` fallback. The
reference example (`shader_advanced/custom_shader_instancing.rs`) ported
cleanly at Bevy 0.19 with no structural obstacles, so the fallback described
in the plan's Task 3 ("if the custom pipeline fights back") was never
needed. The sprite layers (Task 4), by contrast, used the `Material` path.

## 2. Measured frame rate per demo, and where it fell over

All measurements from Task 6, `--windowed`, vsync-capped at 60Hz on this
machine (Apple M4 integrated GPU, monitor refresh 60000 mHz):

| Demo | FPS (smoothed) | Notes |
|---|---|---|
| `--demo point-cloud` | ~58–64 | 50,000 points, custom vertex/fragment shader |
| `--demo sprites` | ~58–62 | 5 layers, depth-sorted, alpha-blended, verified by eye |
| `--demo scatter` | ~1600 (see below) | **not a real measurement** |
| no flag (M0 cube) | ~59–62 | baseline |

Nothing fell over. All of the real demos hold frame rate indistinguishably
from the M0 baseline; the per-frame instance-buffer cost (see finding 4 in
`point_cloud.rs`'s module doc, and Fix 1 below) does not show up as a
measurable frame-rate cost at this scale on this hardware.

The `--demo scatter` figure of ~1600 fps is **not a frame-rate
measurement** and must not be read as one: that demo spawns no camera, and
`bevy_render::view::window::prepare_windows` (mod.rs, ~line 252) skips
acquiring a swapchain texture entirely for a window no camera targets. With
nothing to throttle it, `FrameTimeDiagnosticsPlugin` just logs how fast the
app loop spins with no vsync wait in it at all — a number about the empty
loop, not about scatter's cost.

## 3. Did compute output reach the draw without a CPU round trip?

**No.** Level **(b)** was reached, not level (a): `scatter.wgsl`'s compute
pass writes a `ShaderBuffer` on the GPU (dirty-set-gated, dispatched once
per source, not per frame — the milestone's headline finding), and a
`Readback` proves the written values are correct. That buffer never feeds
the point cloud's instanced draw.

This was **not** blocked by a fundamental format incompatibility between the
two formats. `scatter.wgsl` writes 12-byte `xyz` triples;
`point_cloud.rs`'s `PointInstance` is a 32-byte interleaved
position+scale+colour record — genuinely different shapes — but
`point_cloud.rs`'s `SpecializedMeshPipeline::specialize` already binds two
independently-strided vertex buffer slots (the mesh's own attributes in one
slot, `PointInstance` data in another). A second, non-interleaved vertex
buffer slot fed directly from scatter's raw position buffer, paired with a
hardcoded uniform scale and colour, would close the gap with no second GPU
pass required. The reason it wasn't done in Task 5 is scope: Task 5's file
list was `scatter.rs` + `lib.rs`, and closing the gap requires touching
`point_cloud.rs`/`point_cloud.wgsl`, which belong to Task 3. This
distinction — "unreachable within one task's file scope" vs. "blocked by
the data formats" — is deliberate and should not drift back to the
stronger claim in any future retelling.

## 4. Anything in spec §2.10's GPU-residency section that turned out wrong?

Nothing observed contradicts it. Honestly: not enough was exercised deeply
enough at this milestone's scale to meaningfully stress-test that section's
claims either way — one compute dispatch, one readback, one instanced draw,
all well inside conservative bounds. This is not a confirmation of §2.10 so
much as an absence of a counterexample; it should not be inflated into
either.

## Other things a later milestone would otherwise have to rediscover

- **Two bugs only real GPU execution exposed** (both invisible to naga
  validation, code review, or the test suite):
  1. `point_cloud.wgsl` indexed the mesh world-from-local transform by the
     per-point `@builtin(instance_index)` instead of the hardcoded `0u` the
     reference example uses. Every point read a wrong/wrapped transform;
     the cloud rendered as garbled streaks and smears instead of a sphere.
     `point_cloud.wgsl` is naga-exempt (allowlisted, `#import`-using), so
     nothing short of running it on a GPU caught this — the strongest
     argument in M1 for why the by-eye verification gate is not optional.
  2. The app had no way to find its shaders outside of `cargo run` (a
     git-tracked symlink from `sway-app/assets` degraded to a plain text
     file on checkouts without symlink support). Solved with
     `embedded_asset!`/`load_embedded_asset!`, proven by launching the
     release binary directly from `/tmp`.
- **`sprite_layer.wgsl` could not keep a self-contained `View`.** The plan's
  brief proposed a minimal 3-field `View { clip_from_world, camera_right,
  camera_up }`. Under the `Material` trait this is not legal: group 1 is
  already Bevy's mesh-view binding-array group and group 2 is the mesh
  group, so a material's own bindings can only live at group 3
  (`MATERIAL_BIND_GROUP_INDEX = 3`, hardcoded in `bevy_pbr::material`).
  Group 0 binding 0 is always Bevy's real `View`
  (`bevy_render::view::view.wgsl`), which has 16 fields, not 3 — a
  self-contained struct would silently misread every field after the first.
  `sprite_layer.wgsl` imports Bevy's real view
  (`#import bevy_pbr::mesh_view_bindings::view`) and derives
  `camera_right`/`camera_up` from `view.world_from_view`'s first two
  columns instead.
- **`extra_buffer_usages` was unnecessary.** The `compute_mesh.rs` reference
  example sets `MeshAllocatorSettings::extra_buffer_usages =
  BufferUsages::STORAGE` because it writes into the mesh allocator's own
  vertex/index slabs, which aren't `STORAGE`-usage by default. `scatter.rs`
  writes into a plain `ShaderBuffer` asset instead, whose default
  `buffer_description.usage` is already `STORAGE | COPY_SRC | COPY_DST` —
  nothing to opt into.
- **Per-frame costs the point cloud carries, inherited from the reference
  example rather than chosen** (documented in `point_cloud.rs`'s module doc
  and near `prepare_instance_buffers`): `ExtractComponentPlugin::<PointCloudData>`
  is registered with `QueryFilter = ()`, so its default `extract_components`
  system clones the entire 50,000-element `Vec<PointInstance>` (~1.6MB) on
  the CPU every `ExtractSchedule`; `prepare_instance_buffers` separately
  rebuilds and re-uploads a matching ~1.6MB GPU buffer every frame. Neither
  showed up as a measurable frame-rate cost at this scale on this hardware,
  but neither is amortized either.
- **The scatter readback is not one-shot.** `scatter.rs`'s module doc
  originally said the `Readback` "reads it back once"; on hardware,
  `ReadbackComplete` fires every frame the readback entity exists. The
  compute dispatch itself genuinely is dirty-set-gated to run once — that
  distinction (once-per-source dispatch vs. every-frame readback polling)
  is now made explicit in the module doc.
