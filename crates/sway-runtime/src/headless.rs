//! A Bevy `App` that owns no window and creates no device.
//!
//! The host supplies both (spec §2.8): winit lives in `sway-app`, the device in
//! `sway-gpu`. Bevy is advanced by explicit `app.update()` calls rather than by
//! a runner, which is what lets the host interleave a masonry redraw and a
//! compositor pass around it.

use std::path::Path;
use std::sync::Arc;

use bevy::asset::AssetPlugin;
use bevy::camera::ManualTextureViewHandle;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::render_resource::{TextureFormat, TextureView as BevyTextureView};
use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue, WgpuWrapper,
};
use bevy::render::settings::RenderCreation;
use bevy::render::texture::{ManualTextureView, ManualTextureViews};
use bevy::winit::WinitPlugin;

/// The editor camera's render target: the pane-sized viewport texture.
///
/// Handle zero, and no longer the only one — every camera the graph declares
/// gets a handle of its own from
/// [`CameraTargets`](crate::project::cameras::CameraTargets), sized by that
/// camera's authored resolution. This one belongs to the cameras the graph did
/// *not* produce (the editor's own camera and the gizmo overlay camera), whose
/// size comes from the pane rather than from the document.
pub const VIEWPORT_HANDLE: ManualTextureViewHandle = ManualTextureViewHandle(0);

/// Builds the headless Bevy `App`: `RenderPlugin` in manual mode against the
/// host-supplied device and adapter, no window (`primary_window: None`), and
/// `WinitPlugin` disabled so Bevy never tries to create its own event loop --
/// see the module docs on `sway-app`'s `main.rs` for why two event loops in
/// one process panics.
///
/// `project_dir` is the asset root: every path a graph names resolves relative
/// to it (`architecture`: A project is a directory).
///
/// Also wires up [`retarget_cameras`](crate::project::cameras::retarget_cameras)
/// (see its docs) and points `VIEWPORT_HANDLE` at `viewport` via
/// [`set_viewport_view`].
pub fn build_app(
    gpu: &sway_gpu::GpuContext,
    viewport: &sway_gpu::ViewportTexture,
    size: UVec2,
    project_dir: impl AsRef<Path>,
) -> App {
    let mut app = App::new();

    app.add_plugins(
        DefaultPlugins
            .set(RenderPlugin {
                render_creation: RenderCreation::manual(
                    RenderDevice::from(gpu.device.clone()),
                    RenderQueue(Arc::new(WgpuWrapper::new(gpu.queue.clone()))),
                    RenderAdapterInfo(WgpuWrapper::new(gpu.adapter.get_info())),
                    RenderAdapter(Arc::new(WgpuWrapper::new(gpu.adapter.clone()))),
                    RenderInstance(Arc::new(WgpuWrapper::new(gpu.instance.clone()))),
                ),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            })
            .set(AssetPlugin {
                file_path: project_dir.as_ref().to_string_lossy().into_owned(),
                // Content assets (textures, meshes) still hot-reload. The
                // graph's own `AssetEvent::Modified` is ignored by the live
                // graph plugin, so a Save cannot bounce the project.
                watch_for_changes_override: Some(true),
                ..default()
            })
            .disable::<WinitPlugin>(),
    );

    app.add_systems(PostStartup, crate::project::cameras::retarget_cameras)
        .add_systems(
            Update,
            // After projection, so a camera allocated a target this frame is
            // pointed at it this frame rather than the next one.
            crate::project::cameras::retarget_cameras.after(crate::ProjectionSet),
        );

    set_viewport_view(&mut app, viewport, size);
    app
}

/// Points `VIEWPORT_HANDLE` at the current viewport texture.
///
/// Called once at construction (by [`build_app`]) and again on every resize,
/// because a resize recreates the texture and therefore invalidates the
/// stored view.
///
/// **Real API note:** `ManualTextureView::texture_view` is Bevy's own
/// `render_resource::TextureView` newtype wrapping `wgpu::TextureView` (see
/// `bevy_render::render_resource::texture`'s `impl From<wgpu::TextureView>
/// for TextureView`), not the raw wgpu type. `viewport.bevy_view.clone()` is
/// the sRGB view: Bevy must write through the view that applies sRGB
/// encoding to its linear output, matching `view_format` below.
pub fn set_viewport_view(app: &mut App, viewport: &sway_gpu::ViewportTexture, size: UVec2) {
    let view: BevyTextureView = viewport.bevy_view.clone().into();
    app.world_mut().resource_mut::<ManualTextureViews>().insert(
        VIEWPORT_HANDLE,
        ManualTextureView {
            texture_view: view,
            size,
            view_format: TextureFormat::Rgba8UnormSrgb,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use sway_gpu::wgpu;

    /// Proves Bevy's render output actually reaches `ViewportTexture::bevy_view`
    /// -- not merely that the frame loop runs without a wgpu validation error.
    ///
    /// Added in response to an Important code-review finding: since Task 2 the
    /// frame loop is paced by our own vsync'd `surface.present()`, so a steady
    /// ~60fps in the logs says nothing about whether Bevy ever wrote a pixel
    /// into the viewport texture. A swapped sRGB/non-sRGB view, a
    /// `retarget_cameras` that never actually fires, or a `ManualTextureViews`
    /// entry left pointing at a stale view after a resize would all produce a
    /// validation-clean, steady-fps, all-black window -- indistinguishable
    /// from success in every log this task's manual runs produced.
    /// `ViewportTexture` carries `COPY_SRC` specifically so this is checkable
    /// without a screen: render one distinctive clear colour with no geometry
    /// at all, copy the texture to a buffer, map it, and check the pixels.
    ///
    /// **How many `app.update()` calls it actually takes, and why (this is
    /// the part the task brief flagged as needing to be discovered, not
    /// assumed):** more than one or two, and the number is not fixed --
    /// it's governed by asynchronous pipeline compilation, not by the
    /// `PipelinedRenderingPlugin` render/extract handshake alone. Found by
    /// direct observation (an early version of this test hardcoded 2 updates
    /// and got a real, but wrong, pixel value -- `(43, 44, 47, 255)`, which
    /// turned out to be `bevy_camera::ClearColor`'s literal default
    /// (`Color::srgb_u8(43, 44, 47)`), not our custom red):
    /// - `bevy_render::view::prepare_view_targets` does clear the
    ///   *intermediate* main texture to our camera's own `clear_color`
    ///   correctly, every frame from the first.
    /// - The pixels that actually land in a `ManualTextureView` come from a
    ///   *second*, later pass: `bevy_core_pipeline::upscaling::node::upscaling`,
    ///   which blits the intermediate texture into the camera's real output
    ///   attachment. Reading that node directly: while its render pipeline
    ///   is still being compiled (`pipeline_cache.get_render_pipeline(..)`
    ///   returns `None` -- pipeline compilation is asynchronous and, per that
    ///   function's own comment, deliberately begins a render pass anyway "to
    ///   avoid pink screen uninit on macos"), it clears the destination to
    ///   `camera.output_mode`'s own clear colour (defaulting to the global
    ///   `ClearColor`) and returns *without drawing the blit*. So every frame
    ///   before that pipeline finishes compiling clears the viewport texture
    ///   to the wrong (but validation-clean, non-panicking) colour.
    /// - In this session, a cold run (fresh `cargo test`, nothing warmed up)
    ///   needed as many as 60 updates before the upscaling pipeline was ready
    ///   and the true red output appeared; once the OS/GPU driver's own
    ///   shader cache was warm (immediately-subsequent runs in the same
    ///   session), as few as 3 sufficed. This is real, observed variance, not
    ///   a hypothetical -- so a hardcoded small constant would be flaky by
    ///   construction on a cold cache (e.g. CI, or a fresh machine).
    ///
    /// The loop below is a bounded poll instead of a fixed count: it keeps
    /// calling `app.update()` and re-reading the texture until the pixels
    /// match (or a generous cap is hit), and reports back exactly how many
    /// updates this run needed.
    #[test]
    fn bevy_render_output_reaches_the_viewport_texture() {
        let gpu = sway_gpu::GpuContext::new(None);
        let size = UVec2::new(4, 4);
        let viewport = sway_gpu::ViewportTexture::new(&gpu.device, size.x, size.y);
        let mut app = build_app(&gpu, &viewport, size, std::env::temp_dir());

        // No mesh needed: Bevy clears its render target to `clear_color`
        // before drawing anything, regardless of what (if anything) else is
        // in the scene, so a camera alone is a sufficient signal.
        // `retarget_cameras` (registered by `build_app` in `PostStartup`)
        // points this camera at `VIEWPORT_HANDLE` before the first render.
        let camera = app.world_mut().spawn(Camera3d::default()).id();
        app.world_mut().entity_mut(camera).insert(Camera {
            clear_color: ClearColorConfig::Custom(Color::srgb(1.0, 0.0, 0.0)),
            ..Default::default()
        });

        app.finish();
        app.cleanup();

        // Bytes-per-row must be padded to `COPY_BYTES_PER_ROW_ALIGNMENT` (256)
        // for `copy_texture_to_buffer` -- wgpu does not do this for you.
        let bytes_per_pixel = 4u32; // Rgba8UnormSrgb
        let unpadded_bytes_per_row = size.x * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
        let buffer_size = u64::from(padded_bytes_per_row) * u64::from(size.y);

        // Copies `bevy_view`'s backing texture (the one Bevy actually wrote
        // through) to a fresh buffer, maps it, and returns
        // `(min_red, max_green, max_blue, min_alpha)` across every pixel.
        // Mapping is async -- `map_async`'s callback only runs once
        // `device.poll` has driven it, so the channel recv would hang
        // forever without the poll.
        let read_viewport = || -> (u8, u8, u8, u8) {
            let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("viewport readback (test)"),
                size: buffer_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("viewport readback encoder (test)"),
                });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: viewport.texture(),
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bytes_per_row),
                        rows_per_image: Some(size.y),
                    },
                },
                wgpu::Extent3d {
                    width: size.x,
                    height: size.y,
                    depth_or_array_layers: 1,
                },
            );
            gpu.queue.submit(Some(encoder.finish()));

            let slice = readback.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("device poll failed");
            rx.recv()
                .expect("map_async callback never ran")
                .expect("buffer mapping failed");

            let data = slice.get_mapped_range();
            let (mut min_red, mut max_green, mut max_blue, mut min_alpha) =
                (255u8, 0u8, 0u8, 255u8);
            for row in 0..size.y {
                let row_start = (row * padded_bytes_per_row) as usize;
                for col in 0..size.x {
                    let px = row_start + (col * bytes_per_pixel) as usize;
                    let pixel = &data[px..px + 4];
                    min_red = min_red.min(pixel[0]);
                    max_green = max_green.max(pixel[1]);
                    max_blue = max_blue.max(pixel[2]);
                    min_alpha = min_alpha.min(pixel[3]);
                }
            }
            drop(data);
            readback.unmap();
            (min_red, max_green, max_blue, min_alpha)
        };

        // Tolerant, not an exact byte match: `Camera3d` gets
        // `DebandDither::Enabled` as a required component by default
        // (`bevy_core_pipeline::core_3d`'s `Core3dPlugin::build`), which can
        // perturb a solid clear colour by a little dithering noise. These
        // thresholds are wide enough to absorb that while still clearly
        // failing for "all black" (never rendered), "the global default
        // clear grey" (the upscaling-pipeline-not-ready case described
        // above), or "wrong channel" (a swapped sRGB/non-sRGB view).
        let matches_expected = |(min_red, max_green, max_blue, min_alpha): (u8, u8, u8, u8)| {
            min_red >= 200 && max_green <= 50 && max_blue <= 50 && min_alpha >= 200
        };

        const MAX_UPDATES: u32 = 300; // generous headroom over the 60 observed cold in this session
        let mut last_seen = (0u8, 0u8, 0u8, 0u8);
        let mut converged_after = None;
        for updates in 1..=MAX_UPDATES {
            app.update();
            last_seen = read_viewport();
            if matches_expected(last_seen) {
                converged_after = Some(updates);
                break;
            }
        }

        let Some(updates) = converged_after else {
            panic!(
                "viewport texture never showed the expected clear colour after \
                 {MAX_UPDATES} updates; last observed (min_red, max_green, max_blue, min_alpha) \
                 = {last_seen:?} -- see this test's doc comment for why the update count varies \
                 (asynchronous pipeline compilation in bevy_core_pipeline's upscaling node)"
            );
        };
        eprintln!(
            "bevy_render_output_reaches_the_viewport_texture: converged after {updates} update(s) \
             (min_red={}, max_green={}, max_blue={}, min_alpha={})",
            last_seen.0, last_seen.1, last_seen.2, last_seen.3
        );
    }
}

