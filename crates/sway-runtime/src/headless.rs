//! A Bevy `App` that owns no window and creates no device.
//!
//! The host supplies both (spec §2.8): winit lives in `sway-app`, the device in
//! `sway-gpu`. Bevy is advanced by explicit `app.update()` calls rather than by
//! a runner, which is what lets the host interleave a masonry redraw and a
//! compositor pass around it.

use std::sync::Arc;

use bevy::camera::{ManualTextureViewHandle, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureView as BevyTextureView};
use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue, WgpuWrapper,
};
use bevy::render::settings::RenderCreation;
use bevy::render::texture::{ManualTextureView, ManualTextureViews};
use bevy::render::RenderPlugin;
use bevy::winit::WinitPlugin;

/// The one manual texture view in the process: Bevy's render target.
pub const VIEWPORT_HANDLE: ManualTextureViewHandle = ManualTextureViewHandle(0);

/// Builds the headless Bevy `App`: `RenderPlugin` in manual mode against the
/// host-supplied device and adapter, no window (`primary_window: None`), and
/// `WinitPlugin` disabled so Bevy never tries to create its own event loop --
/// see the module docs on `sway-app`'s `main.rs` for why two event loops in
/// one process panics.
///
/// Also wires up [`retarget_cameras`] (see its docs) and points
/// `VIEWPORT_HANDLE` at `viewport` via [`set_viewport_view`].
pub fn build_app(
    gpu: &sway_gpu::GpuContext,
    viewport: &sway_gpu::ViewportTexture,
    size: UVec2,
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
            .disable::<WinitPlugin>(),
    );

    app.add_systems(PostStartup, retarget_cameras)
        .add_systems(Update, retarget_cameras);

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

/// Points every camera at the viewport texture.
///
/// **Real API note (deviation from the task brief):** in Bevy 0.19,
/// `RenderTarget` is not a field of `Camera` -- `Camera` is `#[require(...
/// RenderTarget)]`, so the target lives on its own `RenderTarget` component,
/// defaulting to `RenderTarget::Window(WindowRef::Primary)`. `RenderTarget`
/// also does not derive `PartialEq` (unlike `ManualTextureViewHandle`, which
/// does), so idempotence is checked by matching the variant and its handle,
/// not by comparing whole `RenderTarget`s.
///
/// Runs in `PostStartup` so it sees cameras spawned by any `Startup` system,
/// and re-runs in `Update` for cameras added later. The M1 demo files (and
/// `scene::setup_scene`) each spawn their own camera targeting the (now
/// nonexistent) primary window; editing four files to say otherwise would
/// destroy their value as an unmodified regression signal, so this retargets
/// whatever cameras exist instead of touching them.
fn retarget_cameras(mut targets: Query<&mut RenderTarget, With<Camera>>) {
    for mut target in &mut targets {
        let already_set = matches!(*target, RenderTarget::TextureView(h) if h == VIEWPORT_HANDLE);
        if !already_set {
            *target = RenderTarget::TextureView(VIEWPORT_HANDLE);
        }
    }
}
