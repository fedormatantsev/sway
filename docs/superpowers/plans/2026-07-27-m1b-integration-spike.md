# M1b Integration Spike Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove that one process, one wgpu device and one winit event loop can carry both a live Bevy viewport and a masonry editor UI — and find out whether masonry can carry a node editor at all.

**Architecture:** The host (`sway-app`) owns winit and the frame loop. `sway-gpu` creates the one wgpu 29 device and owns the surface, the offscreen textures, the compositor pass and the vello renderer. Bevy runs headless against that device via `RenderCreation::Manual`, rendering into a texture we created via `ManualTextureViews`. `sway-editor` holds a masonry `RenderRoot` and depends on neither wgpu nor vello.

**Tech Stack:** Rust 2024, Bevy 0.19, wgpu 29.0.4, winit 0.30.13, masonry (git `main`), `imaging` 0.0.1 + `imaging_vello` 0.0.2 (`vello-0-9`), WGSL, naga 29.

---

## Read this before Task 1

Like M1, this plan is **not uniformly specifiable**, and pretending otherwise would produce confident-looking code that does not run.

**Tasks 1, 5 and 7 contain exact, verified code.** The compile-time wgpu identity assertion, the External-layer geometry arithmetic, and the hit-test-under-zoom test are all real code with real assertions.

**Tasks 2, 3, 4, 6 and 8 are adaptations.** They name the exact API surface (verified against the actual source, see below) and the exact deltas, but the glue is discovered by compiling. Verification is by eye, per spec §4.

### What was verified before writing this plan

Read from the actual sources, not recalled:

- `imaging_vello` 0.0.2 has features `vello-0-7` / `vello-0-8` / `vello-0-9`; **`vello-0-9` is the default** and resolves to `vello` 0.9 → **wgpu 29.0.4**, byte-identical to bevy 0.19's pin.
- `peniko` 0.6.1 and `kurbo` 0.13.1 unify across masonry `main`, `imaging` 0.0.1 and `vello` 0.9. There is no second copy of either.
- `VelloRenderer::new(device: wgpu::Device, queue: wgpu::Queue) -> Result<Self, Error>`.
- `VelloRenderer::render_to_texture_view(&mut self, scene: &vello::Scene, texture_view: &wgpu::TextureView, width: u32, height: u32) -> Result<(), Error>`.
- Its internal `RenderParams` hardcodes `base_color: Color::from_rgba8(0, 0, 0, 0)` — **the transparent clear the design needs is not configurable and not needed to be.**
- `VelloRenderer::supported_texture_formats()` returns **`vec![Rgba8Unorm]`** and nothing else. The UI texture format is therefore forced.
- `VelloSceneSink::new(scene: &'a mut vello::Scene, surface_clip: Rect)`, `finish(&mut self) -> Result<(), Error>`, and it `impl PaintSink`.
- `RenderRoot::new(root_widget: NewWidget<impl Widget + ?Sized>, signal_sink: impl FnMut(RenderRootSignal) + 'static, options: RenderRootOptions) -> Self`.
- `RenderRoot::redraw(&mut self) -> (VisualLayerPlan, Option<TreeUpdate>)`.
- `RenderRootOptions { default_properties, use_system_fonts, size_policy, size: PhysicalSize<u32>, scale_factor: f64, test_font }`.
- `VisualLayerPlan { pub layers: Vec<VisualLayer> }`; `VisualLayer { kind: VisualLayerKind, transform: Affine, widget_id: WidgetId }`; `VisualLayerKind::{ Scene(Scene), External { bounds: Rect } }`.
- `VisualLayerPlan::replay_into<S: PaintSink + ?Sized>(&self, sink: &mut S)` replays **only** `Scene` layers, already transformed to window space, and silently skips `External`. This is exactly what the UI texture wants.
- `ui_events_winit::{WindowEventReducer, WindowEventTranslation}`; `reducer.reduce(scale_factor: f64, &winit::event::WindowEvent) -> Option<WindowEventTranslation>`, matched to `RenderRoot::handle_pointer_event(p)` / `handle_text_event(TextEvent::Keyboard(k))`.
- Bevy's `RenderCreation::manual(device, queue, adapter_info, adapter, instance)` and the five public tuple structs it takes.
- `ManualTextureView { texture_view, size: UVec2, view_format: TextureFormat }`; `ManualTextureViews` derefs to `HashMap<ManualTextureViewHandle, ManualTextureView>`; `ManualTextureViewHandle(pub u32)`.
- Masonry widgets have `WidgetMut::set_transform(Affine)`, composed into `WidgetState::window_transform`, and hit-testing inverts it. Pan/zoom is one call.

### The one thing that is genuinely unknown upstream

`PaintLayerMode::External` is documented in masonry `main` as: *"Current hosts do not realize these placeholders yet; compatibility consumers simply skip them while flattening scene content. This mode exists so the core paint model can represent external boundaries before host integration lands."*

We are the first host to realize it. Task 5 is where that shows up. If `External` turns out not to carry usable bounds, the fallback is stated in Task 5 and is not a milestone failure.

## Global Constraints

- **Bevy pinned to exactly `=0.19.0`.** Unchanged from M1.
- **wgpu pinned to exactly `=29.0.4` and winit to exactly `=0.30.13`**, once, in `[workspace.dependencies]` (spec §2.8).
- **masonry pinned to git rev `c5950bcb03d4f3d187a20d1159f6aa276fd056bf`.** Never a branch. A moving `main` turns an unrelated task's failure into a mystery.
- **`sway-editor` must not depend on `wgpu`, `vello`, `imaging_vello`, or `bevy`.** This is the structural claim of the milestone; a `cargo tree` check is not needed because the manifest is the proof, but adding one of these deps to `sway-editor` means the design was abandoned rather than adjusted.
- **All wgpu object creation lives in `sway-gpu`.** No `Device`, `Queue`, `Instance`, `Adapter`, `Surface` or `Texture` is created anywhere else (spec §2.8).
- **Do not touch `crates/sway-app/src/graph.rs`.** M2 lifts it into `sway-graph`; unrelated edits create conflicts. Same constraint M1 carried.
- **Do not edit `point_cloud.rs`, `sprite_layer.rs` or `scatter.rs`.** M1's demos are the content under test and must keep working *unmodified* — that is what makes them a regression signal.
- **Every shader is naga-validated.** `sway-gpu` gets its own validation test (Task 2); `sway-runtime`'s existing harness is `#[cfg(test)]`-private and cannot be reused across crates.
- Existing tests must keep passing: `cargo test --workspace` was **25 green** at the start of this milestone (measured, not recalled — an earlier draft of this line said 15, inherited from M1's plan text, and was wrong). Task 1 adds 1, making 26.

## Colour space — decide once, here

Getting this wrong produces washed-out or double-dark output and costs hours of squinting. The scheme, fixed for the whole milestone:

| Surface | Format | Holds |
|---|---|---|
| Viewport texture | `Rgba8UnormSrgb`, created with `view_formats: &[Rgba8Unorm]` | Bevy renders linear; hardware encodes on write |
| — its Bevy view | `Rgba8UnormSrgb` | given to `ManualTextureView` |
| — its compositor view | `Rgba8Unorm` | samples the already-encoded bytes raw |
| UI texture | `Rgba8Unorm` | forced by vello's `supported_texture_formats()`; vello writes encoded bytes |
| Window surface | `Bgra8Unorm` (non-sRGB) | compositor writes encoded bytes straight through |

Everything downstream of Bevy's own output stays in **encoded** space, and the compositor performs no conversion at all. Alpha blending therefore happens in encoded space, which is technically wrong and is what every 2D UI does; it is not worth a linear round trip here.

Creating a non-sRGB view of an sRGB texture requires `view_formats` to be declared at texture creation. That is why the viewport texture's descriptor lists it.

## File Structure

```
crates/sway-gpu/                     NEW — the only crate that creates wgpu objects
  Cargo.toml
  src/lib.rs                         re-exports; the wgpu-identity assertion test
  src/context.rs                     GpuContext: instance/adapter/device/queue, feature+limit union
  src/surface.rs                     WindowSurface: configure, resize, begin_frame
  src/frame.rs                       Frame: owns the surface view + encoder; composite, present
  src/textures.rs                    ViewportTexture, UiTexture: creation and resize policy
  src/compositor.rs                  Compositor: the two-quad pass
  src/ui_render.rs                   UiRenderer: VisualLayerPlan -> vello::Scene -> UI texture
  assets/shaders/composite.wgsl      fullscreen-triangle sampler, one quad per draw

crates/sway-editor/                  NEW — no wgpu, no vello, no bevy
  Cargo.toml
  src/lib.rs                         EditorUi: RenderRoot ownership + winit event feeding
  src/canvas.rs                      GraphCanvas widget: pan/zoom, edge painting, drag-to-connect
  src/node_box.rs                    NodeBox widget: one per node, own drag/selection state

crates/sway-runtime/
  src/headless.rs                    NEW — builds the Bevy App against an external device
  src/lib.rs                         MODIFIED — add `pub mod headless;`

crates/sway-app/
  src/main.rs                        MODIFIED — arg parsing gains --editor; hands off to shell
  src/shell.rs                       NEW — winit ApplicationHandler, the frame loop
  src/presenter.rs                   NEW — ShowPresenter and EditorPresenter
```

`sway-graph` does not exist yet (M2). `sway-midi` is untouched.

---

### Task 1: `sway-gpu` crate, the shared device, and the identity gate

**Files:**
- Create: `crates/sway-gpu/Cargo.toml`
- Create: `crates/sway-gpu/src/lib.rs`
- Create: `crates/sway-gpu/src/context.rs`
- Modify: `Cargo.toml` (workspace members and dependencies)

**Interfaces:**
- Consumes: nothing
- Produces:
  - `sway_gpu::GpuContext { pub instance: wgpu::Instance, pub adapter: wgpu::Adapter, pub device: wgpu::Device, pub queue: wgpu::Queue }`
  - `sway_gpu::GpuContext::new(compatible_surface: Option<&wgpu::Surface<'_>>) -> GpuContext`
  - `sway_gpu::wgpu` (re-export, so no other crate names `wgpu` directly)

This task answers gate question 1 of the spec at compile time, before any GPU is touched.

- [ ] **Step 1: Add the crate and its dependencies to the workspace**

In the root `Cargo.toml`, add `"crates/sway-gpu"` and `"crates/sway-editor"` to `members`, and add to `[workspace.dependencies]`:

```toml
sway-gpu = { path = "crates/sway-gpu" }
sway-editor = { path = "crates/sway-editor" }
wgpu = "=29.0.4"
winit = "=0.30.13"
pollster = "0.4"
kurbo = "0.13.1"
peniko = "0.6.1"
imaging = "0.0.1"
imaging_vello = { version = "0.0.2", default-features = false, features = ["vello-0-9"] }
ui-events-winit = "0.3.0"
masonry = { git = "https://github.com/linebender/xilem.git", rev = "c5950bcb03d4f3d187a20d1159f6aa276fd056bf" }
masonry_core = { git = "https://github.com/linebender/xilem.git", rev = "c5950bcb03d4f3d187a20d1159f6aa276fd056bf" }
```

`masonry` and `masonry_core` are pinned by rev, never by branch — see Global Constraints.

- [ ] **Step 2: Create the crate manifest**

`crates/sway-gpu/Cargo.toml`:

```toml
[package]
name = "sway-gpu"
edition.workspace = true
version.workspace = true

[dependencies]
wgpu.workspace = true
pollster.workspace = true
imaging.workspace = true
imaging_vello.workspace = true
kurbo.workspace = true
peniko.workspace = true
bytemuck.workspace = true

[dev-dependencies]
bevy.workspace = true
naga.workspace = true
```

`bevy` is a **dev-dependency only**. `sway-gpu` must not depend on Bevy at runtime — it is the graphics substrate, not a Bevy crate — but the identity assertion in Step 3 needs both `wgpu` types in scope at once, and a test is exactly the right place for that.

- [ ] **Step 3: Write the failing test — the wgpu identity gate**

`crates/sway-gpu/src/lib.rs`:

```rust
//! The single place wgpu objects are created (spec §2.8).
//!
//! Every other crate reaches wgpu through `sway_gpu::wgpu`, so a version bump
//! is one manifest line and one crate's problem.

pub mod context;

pub use context::GpuContext;
pub use wgpu;

#[cfg(test)]
mod version_gate {
    /// The M1b go/no-go gate for device sharing, asserted at compile time.
    ///
    /// Bevy 0.19 and `imaging_vello`'s `vello-0-9` feature must resolve to the
    /// *same* `wgpu` crate, or `RenderDevice::from` cannot accept the device
    /// vello was built against. If cargo ever resolves two `wgpu` versions,
    /// this function stops compiling with a type mismatch naming both — which
    /// is a far better failure than a runtime error about an unrelated
    /// resource, and is why spec §2.8 asks for duplicate detection at all.
    #[test]
    fn bevy_and_vello_share_one_wgpu() {
        fn _same_device(d: imaging_vello::wgpu::Device) -> bevy::render::renderer::RenderDevice {
            bevy::render::renderer::RenderDevice::from(d)
        }
        fn _same_queue(q: imaging_vello::wgpu::Queue) -> wgpu::Queue {
            q
        }
    }
}
```

Note there is no `imaging_vello` dependency listed for the *test* specifically — it is a normal dependency (Step 2), and `bevy` is the dev-dependency. Both are in scope inside `#[cfg(test)]`.

- [ ] **Step 4: Run it to verify it compiles and passes**

Run: `cargo test -p sway-gpu bevy_and_vello_share_one_wgpu -- --nocapture`
Expected: PASS.

If it fails to **compile** with a type mismatch between two `wgpu` versions, the shared-device route is closed. Stop, record which crate pulled the second version (`cargo tree -i wgpu`), and take the two-device fallback described in the design doc §7 task 1. Do not continue improvising.

- [ ] **Step 5: Implement `GpuContext`**

`crates/sway-gpu/src/context.rs`:

```rust
//! Instance, adapter, device and queue creation.
//!
//! The device is requested with the **union** of what Bevy and vello need. This
//! is the most likely place the shared-device route fails, so the request is
//! explicit and the failure is loud rather than a missing-feature panic deep
//! inside a render pass.

use wgpu::{
    Adapter, Backends, Device, DeviceDescriptor, Instance, InstanceDescriptor, Limits,
    PowerPreference, Queue, RequestAdapterOptions, Surface,
};

/// The one wgpu context in the process.
pub struct GpuContext {
    pub instance: Instance,
    pub adapter: Adapter,
    pub device: Device,
    pub queue: Queue,
}

impl GpuContext {
    /// Creates the process-wide wgpu context.
    ///
    /// `compatible_surface` should be the window surface when one exists, so
    /// the adapter chosen can actually present to it.
    pub fn new(compatible_surface: Option<&Surface<'_>>) -> Self {
        let instance = Instance::new(&InstanceDescriptor {
            backends: Backends::from_env().unwrap_or(Backends::PRIMARY),
            ..Default::default()
        });

        let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
            power_preference: PowerPreference::HighPerformance,
            compatible_surface,
            force_fallback_adapter: false,
        }))
        .expect("no suitable wgpu adapter");

        // The union. Bevy's own initialisation asks for the adapter's full
        // feature set and then downgrades; vello needs no optional features on
        // the wgpu backend but does need non-default limits for its bind
        // groups. Taking the adapter's limits wholesale satisfies both without
        // guessing which specific limit each one reads.
        let features = adapter.features();
        let limits = adapter.limits();

        let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
            label: Some("sway shared device"),
            required_features: features,
            required_limits: limits,
            memory_hints: Default::default(),
            trace: Default::default(),
        }))
        .expect("could not create the shared wgpu device");

        Self { instance, adapter, device, queue }
    }
}
```

If `request_device` fails, the panic message plus `adapter.features()` is the finding to record — that is the "irreconcilable feature or limit set" branch of design §7.

- [ ] **Step 6: Verify the whole workspace still builds and tests green**

Run: `cargo test --workspace`
Expected: PASS, 16 tests (the 15 existing plus the identity gate).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/sway-gpu
git commit -m "feat(gpu): sway-gpu crate and the bevy/vello wgpu identity gate"
```

---

### Task 2: The winit shell, the surface, and a vello rectangle on screen

**Files:**
- Create: `crates/sway-gpu/src/surface.rs`
- Create: `crates/sway-gpu/src/textures.rs`
- Create: `crates/sway-gpu/src/compositor.rs`
- Create: `crates/sway-gpu/assets/shaders/composite.wgsl`
- Create: `crates/sway-gpu/src/ui_render.rs`
- Create: `crates/sway-app/src/shell.rs`
- Modify: `crates/sway-gpu/src/lib.rs` (module declarations, shader validation test)
- Modify: `crates/sway-app/src/main.rs` (add `mod shell;`, an `--editor` flag)

**Adaptation task.** Reference to read in full before writing code: the `imaging_vello` crate docs at `~/.cargo/registry/src/*/imaging_vello-0.0.2/src/lib.rs` — its module-level doc contains a complete worked example of recording into a `vello::Scene` through `VelloSceneSink` and rendering it.

**Interfaces:**
- Consumes: `sway_gpu::GpuContext`
- Produces:
  - `sway_gpu::WindowSurface::new(instance, device, adapter, window: Arc<winit::window::Window>) -> WindowSurface`
  - `WindowSurface::resize(&mut self, device: &wgpu::Device, size: winit::dpi::PhysicalSize<u32>)`
  - `WindowSurface::begin_frame<'a>(&self, device: &wgpu::Device, queue: &wgpu::Queue, compositor: &'a mut Compositor) -> Option<Frame<'a>>` — `None` when the surface is not presentable (`Occluded`/`Timeout`); the caller skips the frame and requests another redraw. `acquire` is private; this is the only route to a `Frame`.
  - `WindowSurface::format(&self) -> wgpu::TextureFormat` (always `Bgra8Unorm`)
  - `sway_gpu::UiTexture::new(device, width, height) -> UiTexture` with `pub view: wgpu::TextureView`, `resize(&mut self, device, width, height)`. Usage flags are `STORAGE_BINDING | TEXTURE_BINDING` — **not** `RENDER_ATTACHMENT`, because vello 0.9 writes through a compute pipeline.
  - `sway_gpu::Compositor::new(device, surface_format) -> Compositor` (`draw` is `pub(crate)`, reachable only via `Frame::composite`)
  - `sway_gpu::Frame::composite(&mut self, quads: &[Quad])` and `Frame::present(self)`, where `Quad<'a> { view: &'a wgpu::TextureView, dst: kurbo::Rect, blend: bool }`. `Frame` owns the surface view and the command encoder, so no crate outside `sway-gpu` creates a wgpu object.
  - `sway_gpu::UiRenderer::new(device: wgpu::Device, queue: wgpu::Queue) -> UiRenderer`
  - `UiRenderer::render_scene(&mut self, scene: &imaging::record::Scene, view: &wgpu::TextureView, width: u32, height: u32)`

- [ ] **Step 1: Write the compositor shader**

`crates/sway-gpu/assets/shaders/composite.wgsl`:

```wgsl
// Draws one textured quad into the target, positioned by a uniform rect given
// in normalised device coordinates. Two invocations per editor frame: the
// Bevy viewport, then the UI layer alpha-blended over it.
//
// No colour conversion happens here. Every texture bound to this shader
// already holds sRGB-encoded bytes and the target is a non-sRGB format, so
// values pass through untouched. See the plan's colour-space table.

struct QuadRect {
    // min_x, min_y, max_x, max_y in NDC: -1..1, y up.
    bounds: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> quad: QuadRect;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    // Two triangles as a strip-order quad: 0=(0,0) 1=(1,0) 2=(0,1) 3=(1,1),
    // emitted as a 6-vertex list 0,1,2, 2,1,3.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    let c = corners[index];

    var out: VertexOutput;
    out.clip_position = vec4<f32>(
        mix(quad.bounds.x, quad.bounds.z, c.x),
        mix(quad.bounds.y, quad.bounds.w, c.y),
        0.0,
        1.0,
    );
    // Texture v runs downward while NDC y runs upward, so v is flipped.
    out.uv = vec2<f32>(c.x, 1.0 - c.y);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(src, src_sampler, in.uv);
}
```

- [ ] **Step 2: Write the failing shader validation test**

Append to `crates/sway-gpu/src/lib.rs`:

```rust
#[cfg(test)]
mod shader_validation {
    //! `sway-runtime`'s equivalent harness is `#[cfg(test)]`-private and cannot
    //! be reached across crates, so this is a deliberate second copy rather
    //! than an oversight. It is small, and the alternative — making the other
    //! crate's test helpers public API — is worse.

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

    #[test]
    fn composite_shader_validates() {
        let src = include_str!("../assets/shaders/composite.wgsl");
        validate_wgsl("composite.wgsl", src).unwrap();
    }

    #[test]
    fn validator_rejects_a_type_error() {
        let bad = "@fragment fn fragment() -> @location(0) vec4<f32> { return vec3<f32>(1.0); }";
        assert!(validate_wgsl("bad", bad).is_err());
    }
}
```

- [ ] **Step 3: Run it to verify it passes**

Run: `cargo test -p sway-gpu shader_validation`
Expected: PASS, 2 tests.

- [ ] **Step 4: Implement `WindowSurface`, `UiTexture` and `Compositor`**

`surface.rs` configures the surface with `format: Bgra8Unorm`, `usage: RENDER_ATTACHMENT`, `present_mode: Fifo` (vsync — this is where the frame pacing comes from, and what makes M1's bogus 1600 fps measurement impossible here), `alpha_mode: Auto`.

`textures.rs` creates the UI texture as `Rgba8Unorm` with `usage: RENDER_ATTACHMENT | TEXTURE_BINDING`, and resizes by recreating when the requested size differs from the current one.

`compositor.rs` builds one render pipeline against the surface format, a `Linear` sampler, and a bind group per quad. Quad 0 uses `blend: None`, quad 1 uses `BlendState::ALPHA_BLENDING`. Two pipelines (blend and no-blend) rather than one, because blend state is baked into a pipeline in wgpu.

Convert a `kurbo::Rect` in physical pixels to the shader's NDC bounds with:

```rust
fn to_ndc(dst: kurbo::Rect, width: f32, height: f32) -> [f32; 4] {
    [
        (dst.x0 as f32 / width) * 2.0 - 1.0,
        1.0 - (dst.y1 as f32 / height) * 2.0,
        (dst.x1 as f32 / width) * 2.0 - 1.0,
        1.0 - (dst.y0 as f32 / height) * 2.0,
    ]
}
```

Note the y flip and the `y0`/`y1` swap: kurbo's y grows downward, NDC's grows upward.

- [ ] **Step 5: Implement `UiRenderer`**

`ui_render.rs` wraps `imaging_vello::VelloRenderer`. Given an `imaging::record::Scene`, it encodes and renders:

```rust
pub fn render_scene(
    &mut self,
    scene: &imaging::record::Scene,
    view: &wgpu::TextureView,
    width: u32,
    height: u32,
) {
    let native = self
        .inner
        .encode_scene(scene, width, height)
        .expect("vello could not encode the UI scene");
    self.inner
        .render_to_texture_view(&native, view, width, height)
        .expect("vello could not render the UI scene");
}
```

`render_to_texture_view` clears to fully transparent before drawing — that behaviour is hardcoded in `imaging_vello` and is what makes the UI texture compositable over the viewport.

- [ ] **Step 6: Implement the winit shell**

`crates/sway-app/src/shell.rs` implements `winit::application::ApplicationHandler`:

- `resumed`: create the `Window`, wrap it in an `Arc`, create the `Surface` from it, then `GpuContext::new(Some(&surface))`, then everything downstream.
- `window_event`: handle `CloseRequested`, `Resized` (reconfigure the surface and resize the textures), and `RedrawRequested` (run one frame).
- End every frame with `window.request_redraw()` so the loop is continuous.

The surface must be created **before** the context, because the adapter must be compatible with it — hence `GpuContext::new`'s `compatible_surface` argument.

For this task the frame body is: paint a solid rectangle into an `imaging::record::Scene` with `imaging::Painter::fill_rect`, render it to the UI texture, and composite that one quad fullscreen onto the surface.

- [ ] **Step 7: Run it and look at it**

Run: `cargo run -p sway-app -- --editor --windowed`
Expected: a window containing a solid coloured rectangle on a transparent-therefore-black background, at vsync, resizable without panicking.

**This is the first half of gate question 1: vello renders on our device, into our texture, through our compositor, to our surface.**

- [ ] **Step 8: Commit**

```bash
git add crates/sway-gpu crates/sway-app/src/shell.rs crates/sway-app/src/main.rs
git commit -m "feat(gpu): winit shell, surface, compositor, and vello rendering on the shared device"
```

---

### Task 3: Headless Bevy on the shared device, and the show presenter

**Files:**
- Create: `crates/sway-runtime/src/headless.rs`
- Create: `crates/sway-app/src/presenter.rs`
- Modify: `crates/sway-runtime/src/lib.rs` (add `pub mod headless;`)
- Modify: `crates/sway-runtime/Cargo.toml` (add `sway-gpu`)
- Modify: `crates/sway-app/src/main.rs` (route the demos through the shell)
- Modify: `crates/sway-gpu/src/textures.rs` (add `ViewportTexture`)

**Adaptation task.** Reference to read in full before writing code: `~/.cargo/registry/src/*/bevy_render-0.19.0/src/settings.rs` (the `RenderCreation` enum and `RenderResources`) and `.../src/texture/manual_texture_view.rs` (whose doc comment contains a complete usage example).

**Interfaces:**
- Consumes: `sway_gpu::GpuContext`, `sway_gpu::ViewportTexture`
- Produces:
  - `sway_gpu::ViewportTexture::new(device, width, height) -> ViewportTexture` with `pub bevy_view: wgpu::TextureView` (`Rgba8UnormSrgb`) and `pub sample_view: wgpu::TextureView` (`Rgba8Unorm`)
  - `sway_runtime::headless::VIEWPORT_HANDLE: ManualTextureViewHandle` (value `ManualTextureViewHandle(0)`)
  - `sway_runtime::headless::build_app(gpu: &GpuContext, viewport: &ViewportTexture, size: UVec2) -> App`
  - `sway_runtime::headless::set_viewport_view(app: &mut App, viewport: &ViewportTexture, size: UVec2)` — called on resize
  - `sway_app::presenter::ShowPresenter::present(&mut self, ...)`

- [ ] **Step 1: Add `ViewportTexture` with both views**

In `crates/sway-gpu/src/textures.rs`:

```rust
/// The texture Bevy renders into.
///
/// Two views of one texture, in different formats: Bevy writes through the
/// sRGB view (so the hardware encodes its linear output), and the compositor
/// samples through the non-sRGB view (so it reads those encoded bytes without
/// decoding them again). `view_formats` must list the second format at
/// creation or wgpu rejects the view.
pub struct ViewportTexture {
    texture: wgpu::Texture,
    pub bevy_view: wgpu::TextureView,
    pub sample_view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}
```

Create with `format: Rgba8UnormSrgb`, `view_formats: &[wgpu::TextureFormat::Rgba8Unorm]`, `usage: RENDER_ATTACHMENT | TEXTURE_BINDING | COPY_SRC`.

- [ ] **Step 2: Build the headless Bevy app**

`crates/sway-runtime/src/headless.rs`:

```rust
//! A Bevy `App` that owns no window and creates no device.
//!
//! The host supplies both (spec §2.8): winit lives in `sway-app`, the device in
//! `sway-gpu`. Bevy is advanced by explicit `app.update()` calls rather than by
//! a runner, which is what lets the host interleave a masonry redraw and a
//! compositor pass around it.

use bevy::prelude::*;
use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue,
};
use bevy::render::settings::RenderCreation;
use bevy::render::texture::{ManualTextureView, ManualTextureViews};
use bevy::render::{RenderPlugin, camera::ManualTextureViewHandle};
use bevy::winit::WinitPlugin;
use std::sync::Arc;

/// The one manual texture view in the process: Bevy's render target.
pub const VIEWPORT_HANDLE: ManualTextureViewHandle = ManualTextureViewHandle(0);

pub fn build_app(gpu: &sway_gpu::GpuContext, viewport: &sway_gpu::ViewportTexture, size: UVec2) -> App {
    let mut app = App::new();

    app.add_plugins(
        DefaultPlugins
            .set(RenderPlugin {
                render_creation: RenderCreation::manual(
                    RenderDevice::from(gpu.device.clone()),
                    RenderQueue(Arc::new(bevy::render::WgpuWrapper::new(gpu.queue.clone()))),
                    RenderAdapterInfo(bevy::render::WgpuWrapper::new(gpu.adapter.get_info())),
                    RenderAdapter(Arc::new(bevy::render::WgpuWrapper::new(gpu.adapter.clone()))),
                    RenderInstance(Arc::new(bevy::render::WgpuWrapper::new(gpu.instance.clone()))),
                ),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            })
            .disable::<WinitPlugin>(),
    );

    set_viewport_view(&mut app, viewport, size);
    app
}

/// Points `VIEWPORT_HANDLE` at the current viewport texture.
///
/// Called once at construction and again on every resize, because a resize
/// recreates the texture and therefore invalidates the stored view.
pub fn set_viewport_view(app: &mut App, viewport: &sway_gpu::ViewportTexture, size: UVec2) {
    let view = RenderDevice::from(/* see note */).into();
    app.world_mut()
        .resource_mut::<ManualTextureViews>()
        .insert(
            VIEWPORT_HANDLE,
            ManualTextureView {
                texture_view: view,
                size,
                view_format: bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            },
        );
}
```

**Note on the view conversion:** `ManualTextureView::texture_view` is Bevy's `TextureView` newtype wrapping `wgpu::TextureView`. Find its constructor or `From` impl in `bevy_render::render_resource::texture` and use it on `viewport.bevy_view.clone()`. Do not clone the whole texture. This is the one line in this task that must be read out of the source rather than trusted from this plan.

**`ManualTextureViews` lives in the render sub-app, not the main world.** `extract_resource::<ManualTextureViews, ()>` copies it across each frame (see `bevy_render/src/camera.rs:105`), so inserting into the main world is correct and is what the resource's own doc example does.

- [ ] **Step 3: Retarget every camera at the viewport**

The M1 demos and `scene::setup_scene` each spawn a camera targeting the (now nonexistent) primary window. Rather than edit those files — which Global Constraints forbid — add a system that retargets whatever cameras exist:

```rust
/// Points every camera at the viewport texture.
///
/// Runs in `PostStartup` so it sees cameras spawned by any `Startup` system,
/// and re-runs for cameras added later. The M1 demo files each spawn their own
/// camera targeting the primary window; there is no primary window now, and
/// editing four demo files to say so would destroy their value as an unmodified
/// regression signal.
fn retarget_cameras(mut cameras: Query<&mut Camera, Added<Camera>>) {
    for mut camera in &mut cameras {
        camera.target = bevy::camera::RenderTarget::TextureView(VIEWPORT_HANDLE);
    }
}
```

Register it with `app.add_systems(PostStartup, retarget_cameras).add_systems(Update, retarget_cameras);`.

- [ ] **Step 4: Write the show presenter**

`crates/sway-app/src/presenter.rs`:

```rust
/// Blits the viewport fullscreen. No masonry, no vello.
pub struct ShowPresenter;

impl ShowPresenter {
    pub fn present(
        &mut self,
        app: &mut bevy::app::App,
        gpu: &sway_gpu::GpuContext,
        surface: &sway_gpu::WindowSurface,
        viewport: &sway_gpu::ViewportTexture,
        compositor: &mut sway_gpu::Compositor,
    ) {
        app.update();

        // `None` means the surface is not presentable this frame (Occluded /
        // Timeout). Skip it and let the caller request another redraw — this
        // is routine, not an error.
        let Some(mut frame) = surface.begin_frame(&gpu.device, &gpu.queue, compositor) else {
            return;
        };

        frame.composite(&[sway_gpu::Quad {
            view: &viewport.sample_view,
            dst: kurbo::Rect::new(0.0, 0.0, surface.width() as f64, surface.height() as f64),
            blend: false,
        }]);

        frame.present();
    }
}
```

Both renderers submit to the same queue, so Bevy's work (submitted inside `app.update()`) is ordered before the compositor's without explicit synchronisation.

**`Frame` owns the encoder and the surface view**, so no crate outside `sway-gpu` creates a wgpu object — `Frame::present` finishes the encoder, submits, and presents, in that order. This shape replaced an earlier `Compositor::draw(encoder, device, target, quads)` that forced its caller to build both; the constraint that all wgpu creation lives in `sway-gpu` is what drove the change.

- [ ] **Step 5: Route `main.rs` through the shell**

`main()` keeps its MIDI setup and argument parsing, drops `app.run()`, and hands the demo selection to the shell as a closure that adds the right plugins and startup systems to the app the shell builds. `--editor` selects `EditorPresenter` (Task 4 onward); its absence selects `ShowPresenter`.

`app.finish()` and `app.cleanup()` must be called once after construction and before the first `app.update()`.

- [ ] **Step 6: Verify each demo still renders**

Run each and look at it:

```bash
cargo run -p sway-app -- --windowed
cargo run -p sway-app -- --windowed --demo point-cloud
cargo run -p sway-app -- --windowed --demo sprites
cargo run -p sway-app -- --windowed --demo scatter
```

Expected: the M0 cube, the point cloud, the sprite layers all look exactly as they did under M1 — same content, now arriving via our texture and our compositor. `scatter` still logs its readback (it renders nothing; it never did).

Record the FPS lines. **A drop here is a finding, not a nuisance:** it would mean manual device creation or the extra blit costs something M1 did not pay.

**This completes gate question 1.** Both renderers, one device, one window.

- [ ] **Step 7: Commit**

```bash
git add crates/sway-runtime crates/sway-app crates/sway-gpu
git commit -m "feat(runtime): headless Bevy on the shared device, and the show presenter"
```

---

### Task 4: Masonry in the shell — RenderRoot, events, and the UI layer over the viewport

**Files:**
- Create: `crates/sway-editor/Cargo.toml`
- Create: `crates/sway-editor/src/lib.rs`
- Modify: `crates/sway-app/src/presenter.rs` (add `EditorPresenter`)
- Modify: `crates/sway-app/src/shell.rs` (feed winit events to the editor)

**Adaptation task.** Reference to read in full before writing code: `masonry_winit/src/event_loop_runner.rs` from the pinned xilem rev (clone it or read it in the cargo git checkout). It is the only existing host and answers every question about ordering that this plan does not. Read `handle_window_event` around line 840 for the event path in particular.

**Interfaces:**
- Consumes: `sway_gpu::UiRenderer`, `sway_gpu::UiTexture`, `sway_gpu::Compositor`
- Produces:
  - `sway_editor::EditorUi::new(size: PhysicalSize<u32>, scale_factor: f64) -> EditorUi`
  - `EditorUi::handle_winit_event(&mut self, scale_factor: f64, event: &winit::event::WindowEvent)`
  - `EditorUi::resize(&mut self, size: PhysicalSize<u32>, scale_factor: f64)`
  - `EditorUi::redraw(&mut self) -> masonry_core::app::VisualLayerPlan`
  - `EditorUi::flatten(plan: &VisualLayerPlan) -> imaging::record::Scene`

- [ ] **Step 1: Create the manifest — and note what is absent**

`crates/sway-editor/Cargo.toml`:

```toml
[package]
name = "sway-editor"
edition.workspace = true
version.workspace = true

[dependencies]
masonry.workspace = true
masonry_core.workspace = true
imaging.workspace = true
kurbo.workspace = true
peniko.workspace = true
ui-events-winit.workspace = true
winit.workspace = true
```

**No `wgpu`, no `vello`, no `imaging_vello`, no `bevy`.** That absence is the structural result masonry's renderer split buys and the thing this milestone is proving; if a later step wants to add one of them, the design was abandoned rather than adjusted.

`winit` appears only because `ui-events-winit` takes `&winit::event::WindowEvent`. It draws nothing.

- [ ] **Step 2: Implement `EditorUi`**

```rust
//! The masonry half of the editor: a widget tree and the events that reach it.

use masonry_core::app::{RenderRoot, RenderRootOptions, RenderRootSignal, VisualLayerPlan};
use masonry_core::core::TextEvent;
use ui_events_winit::{WindowEventReducer, WindowEventTranslation};

pub struct EditorUi {
    root: RenderRoot,
    reducer: WindowEventReducer,
}

impl EditorUi {
    pub fn new(size: winit::dpi::PhysicalSize<u32>, scale_factor: f64) -> Self {
        let root = RenderRoot::new(
            /* root widget — a placeholder for this task, the canvas from Task 6 */,
            |_signal: RenderRootSignal| {},
            RenderRootOptions {
                default_properties: Default::default(),
                use_system_fonts: true,
                size_policy: Default::default(),
                size,
                scale_factor,
                test_font: None,
            },
        );
        Self { root, reducer: WindowEventReducer::default() }
    }

    pub fn handle_winit_event(&mut self, scale_factor: f64, event: &winit::event::WindowEvent) {
        if let Some(translated) = self.reducer.reduce(scale_factor, event) {
            match translated {
                WindowEventTranslation::Keyboard(k) => {
                    self.root.handle_text_event(TextEvent::Keyboard(k));
                }
                WindowEventTranslation::Pointer(p) => {
                    self.root.handle_pointer_event(p);
                }
            }
        }
    }

    pub fn redraw(&mut self) -> VisualLayerPlan {
        self.root.redraw().0
    }
}
```

The signal sink is a no-op closure for now. Masonry emits `RenderRootSignal`s for cursor changes, IME and window requests; a spike needs none of them, and dropping them silently is a deliberate simplification to record in Task 8, not an oversight to fix here.

Ignore the returned `TreeUpdate` — accessibility is out of scope.

For the placeholder root widget, use any masonry widget that paints something obvious (a `Label`, or a `SizedBox` with a background property). Task 6 replaces it with `GraphCanvas`.

- [ ] **Step 3: Flatten the plan into one scene**

```rust
/// Replays every scene layer into one window-space scene.
///
/// `replay_into` skips `External` layers by construction, which is exactly
/// wanted: the viewport's pixels come from Bevy, not from masonry, and the
/// hole they leave in this scene is what the compositor fills.
pub fn flatten(plan: &VisualLayerPlan) -> imaging::record::Scene {
    let mut scene = imaging::record::Scene::new();
    plan.replay_into(&mut scene);
    scene
}
```

`imaging::record::Scene` implements `PaintSink`, so it can be the sink directly. If it does not, wrap it with `imaging::Painter::new(&mut scene)` and use the painter's sink — check `imaging/src/record.rs` for which.

- [ ] **Step 4: Write the editor presenter with a hardcoded viewport rect**

In `presenter.rs`, add `EditorPresenter`. The frame, in the order the design fixed:

```rust
pub fn present(&mut self, /* ... */) {
    // 1. Masonry first, so a viewport resize costs no frame of lag.
    let plan = editor.redraw();

    // 2. Viewport rect. HARDCODED in this task; Task 5 reads it from the plan.
    let viewport_rect = kurbo::Rect::new(40.0, 40.0, 40.0 + 640.0, 40.0 + 360.0);

    // 3. Resize the viewport texture if the rect changed.
    viewport.resize(&gpu.device, viewport_rect.width() as u32, viewport_rect.height() as u32);
    sway_runtime::headless::set_viewport_view(app, viewport, size);

    // 4. Bevy renders into it.
    app.update();

    // 5. Masonry's scene into the transparent UI texture.
    let scene = sway_editor::EditorUi::flatten(&plan);
    ui_renderer.render_scene(&scene, &ui_texture.view, surface.width(), surface.height());

    // 6. Composite: viewport, then UI over it.
    // 7. Present.
}
```

Resizing the viewport texture invalidates the view Bevy holds, so `set_viewport_view` must run after every resize and before `app.update()`.

- [ ] **Step 5: Run it and look at it**

Run: `cargo run -p sway-app -- --editor --windowed --demo point-cloud`
Expected: the masonry placeholder widget filling the window, with the point cloud drawn in a 640×360 box inset 40px from the top-left corner, both live, at vsync. Resizing the window keeps the UI correct.

If the viewport appears washed out or too dark, the colour-space table was not followed — recheck the two views on `ViewportTexture` before changing anything else.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-editor crates/sway-app
git commit -m "feat(editor): masonry RenderRoot in the shell, composited over the Bevy viewport"
```

---

### Task 5: Realize the `External` layer — the viewport rect comes from masonry

**Files:**
- Create: `crates/sway-editor/src/external.rs`
- Modify: `crates/sway-editor/src/lib.rs` (`pub mod external;`)
- Modify: `crates/sway-app/src/presenter.rs` (use it instead of the hardcoded rect)

**Exact task.** The arithmetic is testable without a GPU and is tested first.

**Interfaces:**
- Consumes: `masonry_core::app::VisualLayerPlan`
- Produces: `sway_editor::external::viewport_rect(plan: &VisualLayerPlan) -> Option<kurbo::Rect>`

- [ ] **Step 1: Write the failing test**

`crates/sway-editor/src/external.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::viewport_rect;
    use kurbo::{Affine, Rect};
    use masonry_core::app::{VisualLayer, VisualLayerKind, VisualLayerPlan};
    use masonry_core::core::WidgetId;

    fn plan(layers: Vec<VisualLayer>) -> VisualLayerPlan {
        VisualLayerPlan { layers }
    }

    fn external(bounds: Rect, transform: Affine) -> VisualLayer {
        VisualLayer { kind: VisualLayerKind::External { bounds }, transform, widget_id: WidgetId::next() }
    }

    #[test]
    fn none_when_no_external_layer() {
        assert_eq!(viewport_rect(&plan(vec![])), None);
    }

    #[test]
    fn identity_transform_returns_bounds_unchanged() {
        let p = plan(vec![external(Rect::new(10.0, 20.0, 110.0, 80.0), Affine::IDENTITY)]);
        assert_eq!(viewport_rect(&p), Some(Rect::new(10.0, 20.0, 110.0, 80.0)));
    }

    #[test]
    fn translation_moves_the_rect_into_window_space() {
        let p = plan(vec![external(
            Rect::new(0.0, 0.0, 100.0, 60.0),
            Affine::translate((25.0, 45.0)),
        )]);
        assert_eq!(viewport_rect(&p), Some(Rect::new(25.0, 45.0, 125.0, 105.0)));
    }

    #[test]
    fn scale_and_translation_compose() {
        // Layer-local (0,0)-(100,60), scaled 2x then translated by (10,10).
        let p = plan(vec![external(
            Rect::new(0.0, 0.0, 100.0, 60.0),
            Affine::translate((10.0, 10.0)) * Affine::scale(2.0),
        )]);
        assert_eq!(viewport_rect(&p), Some(Rect::new(10.0, 10.0, 210.0, 130.0)));
    }

    #[test]
    fn first_external_layer_wins_when_several_exist() {
        let p = plan(vec![
            external(Rect::new(0.0, 0.0, 10.0, 10.0), Affine::IDENTITY),
            external(Rect::new(50.0, 50.0, 60.0, 60.0), Affine::IDENTITY),
        ]);
        assert_eq!(viewport_rect(&p), Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p sway-editor external`
Expected: FAIL — `cannot find function 'viewport_rect'`.

- [ ] **Step 3: Write the implementation**

Above the test module in the same file:

```rust
//! Finding the Bevy viewport's window-space rectangle in masonry's paint output.
//!
//! `VisualLayerKind::External` is masonry's placeholder for content a host
//! renders itself. Its `bounds` are in layer-local coordinates and the layer's
//! `transform` maps them into window space — the same convention the scene
//! layers use, and the reason `replay_into` takes the transform rather than
//! baking it in.
//!
//! Upstream documents this mode as pre-integration ("current hosts do not
//! realize these placeholders yet"), so this module is the host integration it
//! is waiting for, in the narrow form M1b needs: exactly one external layer,
//! the Bevy viewport.

use kurbo::Rect;
use masonry_core::app::{VisualLayerKind, VisualLayerPlan};

/// The window-space rectangle of the first external layer, if any.
///
/// Returns `None` when the widget tree contains no external boundary — which
/// is a legitimate state (the show presenter, or an editor layout with the
/// viewport collapsed), not an error. The caller draws no viewport quad.
pub fn viewport_rect(plan: &VisualLayerPlan) -> Option<Rect> {
    plan.layers.iter().find_map(|layer| match layer.kind {
        VisualLayerKind::External { bounds } => Some(layer.transform.transform_rect_bbox(bounds)),
        VisualLayerKind::Scene(_) => None,
    })
}
```

`transform_rect_bbox` is used rather than transforming two corners by hand: under a rotation the transformed rectangle is not axis-aligned, and the bounding box is the only honest answer for a rectangular viewport. A rotated viewport is not supported and does not need to be.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sway-editor external`
Expected: PASS, 5 tests.

- [ ] **Step 5: Add an external paint layer to the widget tree**

The placeholder root widget from Task 4 gains a child widget whose `paint` does nothing and whose layout reserves the viewport's space, marked with `ctx.set_paint_layer_mode(PaintLayerMode::External)`. Read `masonry/src/tests/paint.rs:199` in the pinned rev — it is the only existing example of setting this mode, and it also shows what the resulting layer looks like.

- [ ] **Step 6: Use it in the presenter**

Replace the hardcoded rect in `EditorPresenter::present` with `sway_editor::external::viewport_rect(&plan)`. When it returns `None`, skip the viewport quad entirely and composite only the UI.

- [ ] **Step 7: Run it and look at it**

Run: `cargo run -p sway-app -- --editor --windowed --demo point-cloud`
Expected: the point cloud appears wherever the external-mode widget is laid out, and moves and resizes with it when the window resizes.

**If `External` does not carry usable bounds** — the layer never appears in the plan, or `bounds` is empty — the fallback is: give the viewport widget a `WidgetId` known to the presenter and read its window-space layout rect through `RenderRoot::get_widget(id)` instead. Same result, one masonry-version-specific hop. Record which one was needed in Task 8; this is the milestone's most likely upstream gap.

- [ ] **Step 8: Commit**

```bash
git add crates/sway-editor crates/sway-app
git commit -m "feat(editor): realize masonry's External paint layer as the Bevy viewport"
```

---

### Task 6: The canvas and node widgets, static

**Files:**
- Create: `crates/sway-editor/src/node_box.rs`
- Create: `crates/sway-editor/src/canvas.rs`
- Modify: `crates/sway-editor/src/lib.rs` (module declarations; use `GraphCanvas` as the root)

**Adaptation task.** Reference to read in full before writing code: any widget in `masonry/src/widgets/` from the pinned rev — `masonry/src/widgets/flex.rs` for a container that lays out children, and `masonry/src/properties/box_shadow.rs` for painting. The `Widget` trait's required methods are `layout`, `paint`, `accessibility_role`, `accessibility`, `children_ids`, `register_children`.

**Interfaces:**
- Consumes: nothing from earlier tasks
- Produces:
  - `sway_editor::node_box::NodeBox::new(label: String) -> NodeBox`, a `Widget`
  - `sway_editor::canvas::GraphCanvas::new() -> GraphCanvas`, a `Widget`
  - `GraphCanvas::with_node(self, id: usize, pos: kurbo::Point, label: &str) -> Self`
  - `GraphCanvas::with_edge(self, from: usize, to: usize) -> Self`

- [ ] **Step 1: Implement `NodeBox`**

A leaf widget. `layout` returns a fixed size (say 160×72). `paint` fills a rounded rect and strokes a border, with a different fill when `self.selected`. `accepts_pointer_interaction` returns `true`. `children_ids` returns an empty `ChildrenIds`.

Bezier and rounded-rect drawing go through `imaging::Painter`: `painter.fill(shape, brush)` and `painter.stroke(shape, &Stroke::new(w), brush)`, where shapes are `kurbo` types. The `fill`/`stroke` builders end in `.draw()`.

- [ ] **Step 2: Implement `GraphCanvas` layout**

`GraphCanvas` holds `Vec<WidgetPod<NodeBox>>`, a parallel `Vec<Point>` of canvas-space positions, and `Vec<(usize, usize)>` of edges.

In `layout`, call `ctx.run_layout(child)` for each child and `ctx.place_child(child, pos)` at its canvas-space position. **The pan/zoom transform is not applied here** — it goes on the canvas's own children via `set_transform`, so that masonry composes it into `window_transform` and inverts it for hit-testing. Applying it manually in `layout` would produce correct pixels and broken pointer routing, which is the single easiest way to accidentally prove nothing in this milestone.

- [ ] **Step 3: Paint the edges**

In `GraphCanvas::paint` (which runs *before* children, so edges sit behind the boxes):

```rust
// A cubic with horizontal control handles — the standard node-editor edge.
// Handle length scales with horizontal distance so short edges do not loop.
let dx = ((to.x - from.x) * 0.5).abs().max(30.0);
let mut path = kurbo::BezPath::new();
path.move_to(from);
path.curve_to(
    kurbo::Point::new(from.x + dx, from.y),
    kurbo::Point::new(to.x - dx, to.y),
    to,
);
painter.stroke(&path, &peniko::kurbo::Stroke::new(2.0), edge_brush).draw();
```

- [ ] **Step 4: Use it as the root widget**

Replace `EditorUi`'s placeholder root with a `GraphCanvas` carrying five or six nodes and three or four edges, plus the external-mode viewport widget from Task 5 as one of its children.

- [ ] **Step 5: Run it and look at it**

Run: `cargo run -p sway-app -- --editor --windowed --demo point-cloud`
Expected: node boxes and bezier edges drawn around a live point-cloud viewport, all in one window.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-editor
git commit -m "feat(editor): node box widgets and bezier edges on a masonry canvas"
```

---

### Task 7: Interaction — pan, zoom, drag, drag-to-connect

**Files:**
- Modify: `crates/sway-editor/src/canvas.rs`
- Modify: `crates/sway-editor/src/node_box.rs`
- Modify: `crates/sway-editor/Cargo.toml` (add `masonry_testing` as a dev-dependency)

**This task answers gate question 2 of the design.** The hit-test assertion is exact and is written first, because "did the click reach the right widget under a zoom" is not answerable by looking at the screen.

**Interfaces:**
- Consumes: `GraphCanvas`, `NodeBox`
- Produces: no new public API; behaviour only

- [ ] **Step 1: Add the test dependency**

```toml
[dev-dependencies]
masonry_testing = { git = "https://github.com/linebender/xilem.git", rev = "c5950bcb03d4f3d187a20d1159f6aa276fd056bf" }
```

Add the same rev to `[workspace.dependencies]` as `masonry_testing`.

- [ ] **Step 2: Write the failing test**

In `crates/sway-editor/src/canvas.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::GraphCanvas;
    use kurbo::Point;
    use masonry_testing::TestHarness;

    /// The claim spec §2.8 makes for masonry, reduced to an assertion.
    ///
    /// A node sits at canvas-space (100, 100). The canvas is zoomed 2x, so it
    /// occupies window space around (200, 200). A press at (210, 210) must
    /// reach *that node's* widget — not the canvas, not a neighbour. If
    /// masonry's `window_transform` inverse did not drive hit-testing, this
    /// press would land on whatever is at unscaled (210, 210) instead, and a
    /// node editor built on it would be subtly, unfixably wrong under zoom.
    #[test]
    fn press_under_zoom_reaches_the_scaled_node() {
        let canvas = GraphCanvas::new()
            .with_node(0, Point::new(100.0, 100.0), "a")
            .with_node(1, Point::new(400.0, 100.0), "b");

        let mut harness = TestHarness::create(canvas);
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::set_zoom(&mut canvas, 2.0);
        });

        harness.mouse_move(Point::new(210.0, 210.0));
        harness.mouse_button_press(masonry_core::core::PointerButton::Primary);

        let selected = harness.root_widget().downcast::<GraphCanvas>().selected_node();
        assert_eq!(selected, Some(0), "the press should have selected the node at canvas (100,100)");
    }

    #[test]
    fn press_outside_any_node_clears_selection() {
        let canvas = GraphCanvas::new().with_node(0, Point::new(100.0, 100.0), "a");
        let mut harness = TestHarness::create(canvas);

        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::set_selected(&mut canvas, Some(0));
        });
        harness.mouse_move(Point::new(20.0, 20.0));
        harness.mouse_button_press(masonry_core::core::PointerButton::Primary);

        assert_eq!(harness.root_widget().downcast::<GraphCanvas>().selected_node(), None);
    }
}
```

`TestHarness`'s exact method names must be read from `masonry_testing/src/` at the pinned rev before writing this — `create`, `mouse_move`, `mouse_button_press` and `edit_root_widget` are the shapes to look for, and the crate's own widget tests are full of working examples. Adjust the calls, not the assertions.

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test -p sway-editor press_under_zoom`
Expected: FAIL — `set_zoom`, `set_selected` and `selected_node` do not exist yet.

- [ ] **Step 4: Implement pan and zoom**

`GraphCanvas` gains `pan: Vec2` and `zoom: f64`, and a `compose` (or post-layout) step that applies `Affine::translate(self.pan) * Affine::scale(self.zoom)` to each child through `WidgetMut::set_transform`. Edge painting applies the same affine to its own path, since edges are painted by the canvas rather than by transformed children.

In `on_pointer_event`:
- `PointerEvent::Scroll` (or its wheel equivalent) with no modifier → pan; with the zoom modifier → multiply `zoom`, keeping the point under the cursor fixed:
  `pan = cursor - (cursor - pan) * (new_zoom / old_zoom)`
- Middle-drag or space-drag → pan directly.

Call `ctx.request_compose()` (or `request_layout` if transforms are only applied during layout) after changing either.

- [ ] **Step 5: Implement node dragging and selection**

In `NodeBox::on_pointer_event`, on `Down` call `ctx.capture_pointer()` and notify the canvas of selection; on `Move` while captured, move the node. The position lives in `GraphCanvas` (it owns layout), so the node reports the delta upward — through a masonry action or a shared `Rc<RefCell<>>`, whichever the pinned rev's widget examples use for child→parent communication.

Divide the pointer delta by `zoom` before applying it to a canvas-space position, or dragging will run at the wrong speed under zoom.

- [ ] **Step 6: Implement drag-to-connect**

A press in the right-hand quarter of a `NodeBox` starts an edge instead of a drag: the canvas records `pending_edge: Option<(usize, Point)>` and paints a bezier from the source node to the live cursor. A release over another node commits an edge; a release anywhere else discards it.

This uses the same pointer capture as node dragging and is where masonry fails if it is going to fail.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p sway-editor`
Expected: PASS, 7 tests (5 from Task 5, 2 here).

- [ ] **Step 8: Run it and look at it**

Run: `cargo run -p sway-app -- --editor --windowed --demo point-cloud`

Expected, all while the point cloud keeps rendering live in its viewport:
- scroll pans the canvas; the modified scroll zooms about the cursor
- nodes drag, at the right speed, at any zoom
- clicking a node selects it and its paint changes
- dragging from a node's right edge draws a bezier to the cursor and commits an edge on release over another node

**If any of these cannot be made to work, that is the milestone's no-go.** Stop and write it up rather than fighting it — the finding is the deliverable.

- [ ] **Step 9: Commit**

```bash
git add crates/sway-editor
git commit -m "feat(editor): pan, zoom, node dragging and drag-to-connect on the masonry canvas"
```

---

### Task 8: Findings report

**Files:**
- Create: `docs/superpowers/reports/2026-07-27-m1b-integration-findings.md`

M1's findings are the model: the four questions the design's §10 asks, answered honestly, with the negative results stated as plainly as the positive ones. That document's most valuable entries were the ones that admitted something was weaker than it looked; match that standard.

- [ ] **Step 1: Answer the four required questions**

1. **Did Bevy and vello share one device?** If not: what failed (feature union, limit union, resource construction), and what the two-device fallback cost per frame.
2. **Which parts of masonry's host-embedding API were missing or wrong**, given `External` is documented upstream as pre-integration. Say explicitly whether Task 5's `External` path worked or its `get_widget` fallback was needed.
3. **What the editor frame costs**, split between `app.update()`, the vello UI pass and the compositor pass. Use `Instant::now()` around each and log once a second, the way `log_fps` already does.
4. **Anything in spec §2.8 that turned out wrong.**

- [ ] **Step 2: Record what a later milestone would otherwise rediscover**

At minimum: the colour-space scheme and what it looked like when wrong; whether `retarget_cameras` was sufficient or the demos needed real changes; the dropped `RenderRootSignal`s and what breaks because of them; and any masonry API that moved between the pinned rev and whatever is current when this is read.

- [ ] **Step 3: Update the spec if the design was wrong**

If the shared device failed, or `External` was unusable, amend `docs/superpowers/specs/2026-07-27-m1b-integration-spike-design.md` with a **Revision** line at the top, in the style the parent spec uses. A design document that records what was believed before implementation and is never corrected afterwards is worse than none.

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/reports docs/superpowers/specs
git commit -m "docs: M1b integration spike findings"
```

---

## Self-review

**Spec coverage.** Every section of the design doc maps to a task: §2.1–§2.3 version findings → Task 1's identity gate; §3 architecture → Tasks 1–4 (crate boundaries, `sway-editor`'s absent dependencies asserted by manifest in Task 4 Step 1); §4 frame ordering → Task 4 Step 4 and Task 3 Step 4; §5 canvas → Tasks 6 and 7; §6 file layout → the File Structure section; §7 task order and failure branches → Task 1 Step 4, Task 5 Step 7, Task 7 Step 8; §8 testing (three named tests) → Task 1 Step 3, Task 5 Steps 1–4, Task 7 Steps 2–3; §9 deliberate regression → carried by Task 3's `--windowed`-only runs; §10 findings → Task 8.

**Two gaps found and closed while reviewing:** the colour-space scheme was implied by the design but never written down, and would have cost hours — it is now a table with a fixed decision before Task 1. And the M1 demos spawn their own window-targeted cameras, which no design section covers; Task 3 Step 3 adds `retarget_cameras` rather than editing files the Global Constraints protect.

**Type consistency.** `ViewportTexture` exposes `bevy_view`/`sample_view` and is used under those names in Tasks 3 and 4. `UiTexture::view` likewise. `viewport_rect` returns `Option<Rect>` in Task 5 and its `None` case is handled in Task 5 Step 6. `VIEWPORT_HANDLE` is defined once in Task 3 and used in `retarget_cameras` in the same task. `Frame::composite` takes `&[Quad]` and both presenters pass slices.

**Known soft spots, flagged in place rather than papered over:** the Bevy `TextureView` newtype conversion in Task 3 Step 2, `TestHarness`'s method names in Task 7 Step 2, and the child→parent communication idiom in Task 7 Step 5 each say to read the pinned source rather than trusting this plan. That is honest for a spike against an unreleased dependency; inventing plausible signatures for them would not be.
