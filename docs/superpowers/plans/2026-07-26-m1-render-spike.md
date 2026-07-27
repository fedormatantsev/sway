# M1 Render Spike Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Learn Bevy 0.19's custom-pipeline and compute-dispatch APIs by building a point cloud, a z-depth sprite layer, and one compute-cooked geometry operator — before anything architectural depends on them.

**Architecture:** A new `sway-runtime` crate holding shaders and render plugins, consumed by `sway-app` behind a CLI flag. Point cloud uses a custom render pipeline with instanced drawing. The sprite layer uses billboarded quads with a custom material. `Scatter` runs as a compute shader whose output buffer the draw consumes without a CPU round trip. All parameters are hardcoded; there is no graph, no node, no port.

**Tech Stack:** Rust 2024, Bevy 0.19, WGSL, naga 29 (for GPU-free shader validation).

---

## This is a spike — read this before Task 1

The spec (§5, M1) says: *"The code is provisional — the goal is knowledge, not architecture."* This plan is therefore **not uniformly specifiable**, and pretending otherwise would produce confident-looking code that does not run.

**Tasks 1, 2 and 6 are exact.** Complete code, real tests, normal TDD.

**Tasks 3, 4 and 5 are adaptations.** Each names a Bevy example that ships in the published crate, does substantially what the task needs, and is version-matched to our pinned `=0.19.0`. For those, this plan gives the reference path, the specific deltas, the exit condition, and a fallback — not line-by-line code. Verifying them requires a GPU and a window, which means **verification is by eye**, per spec §4 (*"Rendering — no pixel-diff tests. Verified by eye."*).

Read the reference example in full before writing code. It is the authority; this plan is the delta.

### What was verified before writing this plan

- The published `bevy` crate **does ship its examples**, at `~/.cargo/registry/src/*/bevy-0.19.0/examples/`. They are version-matched and compile.
- It does **not** ship `assets/`, so every WGSL file referenced by those examples must be written from scratch here.
- **The render graph is now a schedule, not a node graph.** Bevy 0.19 uses `add_systems(RenderGraph, my_system.before(camera_driver))` and a `RenderStartup` schedule for one-time pipeline init. Any recollection of `impl render_graph::Node` is out of date.
- **naga validates WGSL with no GPU and no window** — confirmed for both vertex/fragment and compute shaders, with readable errors. This is the only automated check available for shaders and Task 1 builds it.
- **naga rejects Bevy's `#import`** preprocessor directives. Shaders using them cannot be validated and must be skipped loudly.
- There is **no `Sprite3d`** in Bevy 0.19. The z-depth sprite layer is billboarded quads.
- The shaders in Tasks 2 and 5 were written and validated against naga before being pasted here.

## Global Constraints

- **Bevy pinned to exactly `=0.19.0`.** M1b adds Vello and this pin becomes load-bearing (spec §2.8).
- **No new runtime dependencies beyond `bytemuck` and `naga`.** `bytemuck` is needed for `Pod`/`Zeroable` on instance data; `naga` is a **dev-dependency only**.
- **`sway-midi` must not gain a Bevy dependency**, and `sway-runtime` must not depend on `sway-midi`.
- **Do not touch `crates/sway-app/src/graph.rs`.** M1 is orthogonal to the graph; M2 lifts that file into `sway-graph` and unrelated edits there create conflicts.
- **Every shader lives in `crates/sway-runtime/assets/shaders/`** and is covered by Task 1's validator or explicitly listed as skipped.
- **Hardcoded parameters only.** No node types, no ports, no project file. Resisting the urge to generalise here is the point of the milestone.
- Existing tests must keep passing: `cargo test --workspace` is currently 15 green.

## File Structure

```
crates/sway-runtime/
  Cargo.toml
  assets/shaders/
    scatter.wgsl              compute: writes point positions into a storage buffer
    sprite_layer.wgsl         billboarded, alpha-blended, atlas-sampled quad
    point_cloud.wgsl          instanced point sprites (uses bevy imports — not validated)
  src/lib.rs                  plugin group re-export
  src/shader_validation.rs    naga harness + the test that walks assets/shaders
  src/point_cloud.rs          custom pipeline, instance buffer, draw command
  src/sprite_layer.rs         billboard mesh + custom Material
  src/scatter.rs              compute pipeline, dirty set, render-graph dispatch
crates/sway-app/
  src/main.rs                 gains --demo <point-cloud|sprites|scatter|all>
```

Each render feature is one file because they are independent spikes; two of the three may be thrown away.

---

### Task 1: `sway-runtime` crate and the shader validation harness

**Files:**
- Create: `crates/sway-runtime/Cargo.toml`
- Create: `crates/sway-runtime/src/lib.rs`
- Create: `crates/sway-runtime/src/shader_validation.rs`
- Create: `crates/sway-runtime/assets/shaders/.gitkeep`
- Modify: `Cargo.toml` (workspace members and dependencies)

**Interfaces:**
- Consumes: nothing
- Produces: `sway_runtime::shader_validation::validate_wgsl(name: &str, src: &str) -> Result<(), String>`, and a test that walks `assets/shaders/*.wgsl`

This harness is the only automated feedback available for the rest of M1 — everything else is verified by eye. Build it first.

- [ ] **Step 1: Add the crate to the workspace**

In the root `Cargo.toml`, add `"crates/sway-runtime"` to `members`, and add to `[workspace.dependencies]`:

```toml
sway-runtime = { path = "crates/sway-runtime" }
bytemuck = { version = "1", features = ["derive"] }
naga = { version = "29", features = ["wgsl-in"] }
```

- [ ] **Step 2: Create the crate manifest**

`crates/sway-runtime/Cargo.toml`:

```toml
[package]
name = "sway-runtime"
edition.workspace = true
version.workspace = true

[dependencies]
bevy.workspace = true
bytemuck.workspace = true

[dev-dependencies]
naga.workspace = true
```

`naga` is a dev-dependency deliberately: shader validation is a test-time concern and must not ship in the binary.

- [ ] **Step 3: Write the failing test**

`crates/sway-runtime/src/shader_validation.rs`:

```rust
//! GPU-free WGSL validation.
//!
//! M1 is otherwise verified entirely by eye, so this is the only automated
//! feedback on shader correctness. naga parses and validates WGSL without a
//! device, which catches syntax and type errors on any machine.
//!
//! Limitation: naga does not understand Bevy's `#import` preprocessor, so
//! shaders using it are skipped. Skips are printed rather than silent — an
//! unvalidated shader should be visible, not forgotten.

#[cfg(test)]
mod tests {
    use std::path::Path;

    fn validate_wgsl(name: &str, src: &str) -> Result<(), String> {
        let module = naga::front::wgsl::parse_str(src)
            .map_err(|e| format!("{name}: parse failed:\n{}", e.emit_to_string(src)))?;

        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );

        validator
            .validate(&module)
            .map_err(|e| format!("{name}: validation failed: {e:?}"))?;

        Ok(())
    }

    fn uses_bevy_preprocessor(src: &str) -> bool {
        src.lines().any(|l| {
            let t = l.trim_start();
            t.starts_with("#import") || t.starts_with("#define_import_path")
        })
    }

    #[test]
    fn every_shader_parses_and_validates() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/shaders");
        let mut checked = 0;
        let mut skipped = Vec::new();
        let mut failures = Vec::new();

        for entry in std::fs::read_dir(&dir).expect("assets/shaders must exist") {
            let path = entry.unwrap().path();
            if path.extension().and_then(|s| s.to_str()) != Some("wgsl") {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let src = std::fs::read_to_string(&path).unwrap();

            if uses_bevy_preprocessor(&src) {
                skipped.push(name);
                continue;
            }
            match validate_wgsl(&name, &src) {
                Ok(()) => checked += 1,
                Err(e) => failures.push(e),
            }
        }

        if !skipped.is_empty() {
            println!("NOT VALIDATED (bevy preprocessor imports): {skipped:?}");
        }
        println!("validated {checked} shader(s)");
        assert!(failures.is_empty(), "shader validation failed:\n{}", failures.join("\n\n"));
    }

    #[test]
    fn validator_rejects_a_type_error() {
        // Guards the harness itself: if this ever passes, the validator has
        // been neutered and the test above is worthless.
        let bad = "@fragment fn fragment() -> @location(0) vec4<f32> { return vec3<f32>(1.0, 0.0, 0.0); }";
        assert!(validate_wgsl("bad", bad).is_err());
    }
}
```

- [ ] **Step 4: Create the lib and the shader directory**

`crates/sway-runtime/src/lib.rs`:

```rust
//! Provisional render spike code for M1. Point cloud, z-depth sprite layer,
//! and a compute-cooked scatter operator, all with hardcoded parameters.
//!
//! Per spec §5 the goal here is knowledge, not architecture — expect most of
//! this to be rewritten at M5.

pub mod shader_validation;
```

Create the shader directory so the test has something to walk:

```bash
mkdir -p crates/sway-runtime/assets/shaders
touch crates/sway-runtime/assets/shaders/.gitkeep
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p sway-runtime`
Expected: PASS, `2 passed`, with `validated 0 shader(s)` printed. Zero is correct — no shaders exist yet.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/sway-runtime
git commit -m "feat(runtime): sway-runtime crate with GPU-free WGSL validation harness"
```

---

### Task 2: The Scatter compute shader

**Files:**
- Create: `crates/sway-runtime/assets/shaders/scatter.wgsl`

**Interfaces:**
- Consumes: Task 1's validator
- Produces: a compute entry point `scatter`, workgroup size 64, binding group 0 with a `ScatterParams` uniform at binding 0 and a `read_write` `array<f32>` at binding 1. Task 5 builds the pipeline against exactly this layout.

Written before the pipeline deliberately: the shader is the part that can be checked automatically, and its binding layout is what Task 5's bind group must match.

- [ ] **Step 1: Write the shader**

`crates/sway-runtime/assets/shaders/scatter.wgsl`:

```wgsl
// Scatter: writes `count` pseudo-random points into a storage buffer as xyz
// triples. Self-contained WGSL — no Bevy preprocessor imports — so it is
// covered by the naga validator.

struct ScatterParams {
    count: u32,
    seed: u32,
    extent: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> params: ScatterParams;
@group(0) @binding(1) var<storage, read_write> positions: array<f32>;

// PCG hash -> [0,1). Deterministic in (index, seed), which is what lets a
// recooked or replayed scatter reproduce the same cloud.
fn rand(x: u32) -> f32 {
    var h: u32 = x * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return f32(h) * 2.3283064e-10;
}

@compute @workgroup_size(64)
fn scatter(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i: u32 = gid.x;
    if (i >= params.count) {
        return;
    }
    let base: u32 = i * 3u;
    let s: u32 = params.seed;
    positions[base + 0u] = (rand(i * 3u + 0u + s) * 2.0 - 1.0) * params.extent;
    positions[base + 1u] = (rand(i * 3u + 1u + s) * 2.0 - 1.0) * params.extent;
    positions[base + 2u] = (rand(i * 3u + 2u + s) * 2.0 - 1.0) * params.extent;
}
```

The `count` guard matters: dispatch rounds up to whole workgroups, so the last workgroup runs invocations past the end of the buffer.

- [ ] **Step 2: Run the validator**

Run: `cargo test -p sway-runtime -- --nocapture`
Expected: PASS, with `validated 1 shader(s)` printed.

- [ ] **Step 3: Verify the validator would catch a break**

Temporarily change `positions[base + 0u] = ...` to assign a `vec3<f32>` instead of a float, re-run, confirm it **fails**, then revert. Paste both outputs into your report. A validator nobody has seen fail is not known to work.

- [ ] **Step 4: Commit**

```bash
git add crates/sway-runtime/assets/shaders/scatter.wgsl
git commit -m "feat(runtime): scatter compute shader, naga-validated"
```

---

### Task 3: Point cloud render pipeline — **adaptation, not transcription**

**Files:**
- Create: `crates/sway-runtime/src/point_cloud.rs`
- Create: `crates/sway-runtime/assets/shaders/point_cloud.wgsl`
- Modify: `crates/sway-runtime/src/lib.rs`

**Reference — read this first, in full:**
`~/.cargo/registry/src/*/bevy-0.19.0/examples/shader_advanced/custom_shader_instancing.rs`

That example draws one mesh many times in a single instanced draw call with a custom vertex and fragment shader, using a custom pipeline and a custom `RenderCommand`. That is structurally what a point cloud is.

**Interfaces:**
- Produces: `PointCloudPlugin`, and a component `PointCloudData(Vec<PointInstance>)` where `PointInstance` is `#[repr(C)] #[derive(Clone, Copy, Pod, Zeroable)]` with `position: Vec3` and `color: [f32; 4]`.

**Deltas from the example:**

1. **Instance data.** The example's `InstanceData` carries `position: Vec3`, `scale: f32`, `color: [f32; 4]`. Keep that shape; a per-point scale is wanted for point clouds anyway.
2. **Base mesh.** The example instances a `Cuboid`. Use a small quad or a `Sphere` with low subdivision — a cuboid per point is wasteful at 50k. Record what you chose and why.
3. **Count.** The example spawns 100 instances. Generate **50,000** in a hardcoded pattern (a fibonacci sphere or a simple grid with jitter). The point of the milestone is finding out whether the approach holds at that scale.
4. **Keep `NoFrustumCulling` and `NoIndirectDrawing`.** The example explains both; they are not optional, and removing them produces either invisible geometry or a panic.
5. The shader will need Bevy's mesh view bindings, so `point_cloud.wgsl` will use `#import`. **The validator will skip it — that is expected.** Confirm it appears in the skip list rather than silently vanishing.

- [ ] **Step 1: Read the reference example end to end.** Do not start writing until you can say what `SetItemPipeline`, `SetMeshViewBindGroup`, and the custom `DrawMeshInstanced` each contribute to the `RenderCommand` tuple.
- [ ] **Step 2: Port it into `point_cloud.rs`** as `PointCloudPlugin`, applying deltas 1-4. Write `point_cloud.wgsl` alongside; the example's `instancing.wgsl` is not shipped, so write it from the vertex layout you declare.
- [ ] **Step 3: Register the module** in `lib.rs` and export `PointCloudPlugin`.
- [ ] **Step 4: Run `cargo test -p sway-runtime -- --nocapture`.** Expected: the existing tests still pass and `point_cloud.wgsl` appears under `NOT VALIDATED`.
- [ ] **Step 5: Run `cargo clippy --workspace --all-targets`.** Expected: clean.
- [ ] **Step 6: Commit.** `feat(runtime): instanced point cloud pipeline`

**Exit condition:** 50,000 points visible, at frame rate, with your own vertex and fragment shader. Verified by eye in Task 6.

**Fallback if the custom pipeline fights back:** fall back to `examples/shader/automatic_instancing.rs` plus a custom `Material` — less control, but it still yields custom vertex/fragment shaders and a real answer about scale. Record which path you took; that choice is itself a milestone finding.

---

### Task 4: Z-depth sprite layer

**Files:**
- Create: `crates/sway-runtime/src/sprite_layer.rs`
- Create: `crates/sway-runtime/assets/shaders/sprite_layer.wgsl`
- Modify: `crates/sway-runtime/src/lib.rs`

**Reference:** `~/.cargo/registry/src/*/bevy-0.19.0/examples/shader/shader_material.rs` — the `Material` trait path, which is far less machinery than Task 3's custom pipeline and sufficient here.

**There is no `Sprite3d` in Bevy 0.19.** A z-depth sprite layer is billboarded quads: a quad mesh per layer, oriented to face the camera, alpha-blended, sampled from an atlas, positioned at a chosen depth.

**Interfaces:**
- Produces: `SpriteLayerPlugin`, and `SpriteLayerMaterial` implementing `Material` with `alpha_mode` returning `AlphaMode::Blend`.

- [ ] **Step 1: Write the shader**

`crates/sway-runtime/assets/shaders/sprite_layer.wgsl` — this is validated WGSL, use it verbatim:

```wgsl
// Z-depth sprite layer: a textured, alpha-blended billboard quad.
// Self-contained — takes its own view and layer uniforms rather than Bevy's
// mesh view bind group — so the naga validator covers it.

struct View {
    clip_from_world: mat4x4<f32>,
    camera_right: vec3<f32>,
    _pad0: f32,
    camera_up: vec3<f32>,
    _pad1: f32,
};

struct Layer {
    // xy = world centre, z = depth, w = uniform scale
    placement: vec4<f32>,
    tint: vec4<f32>,
    // xy = atlas cell size in UV, zw = atlas cell offset
    atlas: vec4<f32>,
};

@group(0) @binding(0) var<uniform> view: View;
@group(1) @binding(0) var<uniform> layer: Layer;
@group(1) @binding(1) var sprite_texture: texture_2d<f32>;
@group(1) @binding(2) var sprite_sampler: sampler;

struct VertexIn {
    @location(0) corner: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vertex(in: VertexIn) -> VertexOut {
    let centre = layer.placement.xyz;
    let scale = layer.placement.w;
    let world = centre
        + view.camera_right * in.corner.x * scale
        + view.camera_up * in.corner.y * scale;

    var out: VertexOut;
    out.clip_position = view.clip_from_world * vec4<f32>(world, 1.0);
    // corner is in [-0.5, 0.5]; map to the atlas cell.
    let cell_uv = in.corner + vec2<f32>(0.5, 0.5);
    out.uv = layer.atlas.zw + cell_uv * layer.atlas.xy;
    return out;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let sampled = textureSample(sprite_texture, sprite_sampler, in.uv);
    let c = sampled * layer.tint;
    if (c.a < 0.001) {
        discard;
    }
    return c;
}
```

- [ ] **Step 2: Run the validator.** Expected: `validated 2 shader(s)`.

  > **Post-hoc note (final review, 2026-07-26):** this expectation did not
  > hold. Task 4's `sprite_layer.wgsl` had to take the escape hatch this
  > step's own Step 3 authorized (see below) and switched to Bevy's
  > `#import`, joining `point_cloud.wgsl` on the allowlist. Actual outcome:
  > `validated 1 shader(s)` (`scatter.wgsl`) + 2 allowlisted
  > (`point_cloud.wgsl`, `sprite_layer.wgsl`).

- [ ] **Step 3: Implement `SpriteLayerPlugin`.** If binding your own `View` uniform proves awkward against the `Material` trait's bind-group conventions, switching the shader to Bevy's mesh view import is acceptable — but then it leaves the validated set, so say so in your report.

- [ ] **Step 4: Spawn a hardcoded demo** of **five** layers at distinct z depths with a generated texture (no asset file needed — build an RGBA image in code, e.g. a soft radial gradient). Overlapping layers at different depths is what proves depth sorting works.

- [ ] **Step 5: Run tests and clippy.** Expected: clean.

- [ ] **Step 6: Commit.** `feat(runtime): z-depth billboarded sprite layers`

**Exit condition:** five sprite layers, correctly depth-sorted, alpha-blended, at frame rate.

---

### Task 5: Compute-cooked Scatter — **adaptation, not transcription**

**Files:**
- Create: `crates/sway-runtime/src/scatter.rs`
- Modify: `crates/sway-runtime/src/lib.rs`

**Reference — read this first, in full:**
`~/.cargo/registry/src/*/bevy-0.19.0/examples/shader_advanced/compute_mesh.rs`

This is the closest thing in the whole Bevy example set to what spec §2.10 describes: a compute shader fills GPU buffers, driven by a component extracted into the render world and a dirty-set resource, with no CPU round trip.

**The API shape, which differs from older Bevy:**

```rust
render_app
    .init_resource::<ScatterQueue>()
    .add_systems(RenderStartup, init_scatter_pipeline)
    .add_systems(Render, queue_scatter_jobs)
    .add_systems(RenderGraph, dispatch_scatter.before(camera_driver));
```

`RenderGraph` is a **schedule** here, not a node graph. `camera_driver` comes from `bevy::core_pipeline::schedule::camera_driver` and ordering before it is what makes the compute run before the draw that consumes its output.

**Deltas from the example:**

1. **The dirty set is the point.** The example uses `ChunksToProcess` plus a `Local<HashSet>` of already-processed asset ids so each mesh cooks once rather than every frame. That "cook only when dirty" behaviour is precisely what spec §2.10 requires — preserve it and name it in comments, because it is the finding this task exists to produce.
2. **Bind group layout must match `scatter.wgsl`** from Task 2: binding 0 a `ScatterParams` uniform, binding 1 a `read_write` storage buffer of `f32`. Declare a matching `#[derive(ShaderType)] struct ScatterParams` on the Rust side.
3. **Dispatch size.** The example dispatches `(1, 1, 1)`. Compute `count.div_ceil(64)` workgroups to match the shader's `@workgroup_size(64)`.
4. **`extra_buffer_usages`.** If you write into mesh allocator slabs as the example does, replicate its `finish()` hook setting `MeshAllocatorSettings::extra_buffer_usages = BufferUsages::STORAGE`. Without it the buffers are not bindable as storage and the bind group creation fails.
5. **Consuming the output.** The simplest bridge to Task 3 is to have Scatter write positions that the point cloud's instance buffer reads. If wiring compute output directly into the instanced draw proves too large for a spike, an acceptable reduced target is: compute writes the buffer, and `examples/shader/gpu_readback.rs`'s pattern reads it back once to prove the values are right. **Say clearly in your report which of the two you achieved** — they are very different levels of evidence.

- [ ] **Step 1: Read `compute_mesh.rs` end to end**, and `examples/shader/storage_buffer.rs` for the simpler binding case.
- [ ] **Step 2: Implement `ScatterPlugin`** with the systems above, applying deltas 1-4.
- [ ] **Step 3: Wire the output** per delta 5, at whichever level you reach.
- [ ] **Step 4: Run tests and clippy.** Expected: clean.
- [ ] **Step 5: Commit.** `feat(runtime): compute-dispatched scatter driven by a dirty set`

**Exit condition:** points computed on the GPU from a dirty set, dispatched once rather than per frame, consumed without a CPU round trip.

**Stop condition — escalate rather than grind.** If the extract-and-dispatch shape does not come together, that is a genuine milestone finding, not a failure. Spec §2.10 records GPU residency as a *direction, not settled design*, and §7 lists "which geometry operators are GPU-resident" as an open question to be answered here. Report what blocked it. A clear negative result changes M5's operator set and is worth more than a forced success.

---

### Task 6: Wire the demos into the app and measure

**Files:**
- Modify: `crates/sway-app/src/main.rs`
- Modify: `crates/sway-app/Cargo.toml`

**Interfaces:**
- Consumes: `PointCloudPlugin`, `SpriteLayerPlugin`, `ScatterPlugin`
- Produces: `--demo <point-cloud|sprites|scatter|all>` on the existing binary

- [ ] **Step 1: Add the dependency.** Add `sway-runtime.workspace = true` to `crates/sway-app/Cargo.toml`.

- [ ] **Step 2: Extend argument parsing.** `main.rs` already hand-rolls args over `std::env::args` — follow that pattern exactly, and do **not** add a CLI crate. Add a `demo: Option<String>` field. When absent, behaviour is unchanged from M0: the MIDI-driven cube.

- [ ] **Step 3: Add the demo plugins conditionally**, based on the parsed flag.

- [ ] **Step 4: Add a frame-time readout.** Add Bevy's `FrameTimeDiagnosticsPlugin` and log smoothed FPS every second. "At frame rate" is an exit criterion for this milestone and needs a number, not an impression.

- [ ] **Step 5: Verify the whole suite.** Run `cargo test --workspace` and `cargo clippy --workspace --all-targets`. Expected: all green, existing 15 tests still passing.

- [ ] **Step 6: Manual verification — by eye, requires a GPU.** Run each and record observed FPS:

```bash
cargo run -p sway-app --release -- --demo point-cloud
cargo run -p sway-app --release -- --demo sprites
cargo run -p sway-app --release -- --demo scatter
cargo run -p sway-app --release -- --demo all
```

Also run `cargo run -p sway-app --release` with no flag and confirm the M0 cube still responds to MIDI — M1 must not regress it.

- [ ] **Step 7: Commit.** `feat(app): --demo flag wiring the M1 render spikes`

---

## Exit criteria

From spec §5, M1:

> *a point cloud and a z-depth sprite layer render at frame rate with custom vertex/fragment shaders, and one compute-cooked geometry operator dispatches from a graph-shaped dirty set.*

Concretely:
- 50,000 points, custom vertex/fragment shaders, measured FPS recorded.
- Five depth-sorted alpha-blended sprite layers, measured FPS recorded.
- Scatter dispatching from a dirty set — once, not per frame — with its output either feeding the draw or verified by readback, and which one stated plainly.
- `cargo test --workspace` green; every shader either validated or explicitly listed as skipped.

## What this milestone must produce besides code

M1 exists to answer questions, so the report matters as much as the commits. Record:

1. Whether the custom pipeline or the `Material` path was used for the point cloud, and why.
2. Measured frame rate for each demo, and where it fell over if it did.
3. Whether compute output reached the draw without a CPU round trip.
4. Anything in spec §2.10's GPU-residency section that turned out to be wrong. That section is explicitly marked *direction, not settled design*, and this is the milestone that settles it.

## Deliberately not in M1

No node types, no ports, no graph integration, no project format, no editor. No CPU fallback path for geometry operators. No variable-size compute output — spec §2.10 defers that behind "output size known before dispatch". The code is provisional and M5 rewrites it.
