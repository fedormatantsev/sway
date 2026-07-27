//! The window's presentable swapchain: configuration, resize, and frame
//! acquisition.

use std::sync::Arc;

use wgpu::{
    Adapter, CompositeAlphaMode, CurrentSurfaceTexture, Device, Instance, PresentMode, Surface,
    SurfaceConfiguration, SurfaceTexture, TextureFormat, TextureUsages,
};
use winit::dpi::PhysicalSize;
use winit::window::Window;

/// The window's swapchain.
///
/// Always configured `Bgra8Unorm` (non-sRGB — the compositor shader assumes
/// every source texture already holds sRGB-encoded bytes and passes them
/// through untouched, so the presented format must not itself apply sRGB
/// encoding again) and `Fifo` present mode (vsync — this is where frame
/// pacing comes from).
pub struct WindowSurface {
    surface: Surface<'static>,
    device: Device,
    config: SurfaceConfiguration,
}

impl WindowSurface {
    /// Creates and configures the surface for `window` against `adapter`.
    ///
    /// `instance` must be the same instance used to (or that will) request
    /// `adapter` — wgpu requires a surface and the adapter checked against it
    /// to come from one instance.
    pub fn new(instance: &Instance, device: &Device, adapter: &Adapter, window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let surface = instance
            .create_surface(window)
            .expect("could not create the window surface");

        let caps = surface.get_capabilities(adapter);
        assert!(
            caps.formats.contains(&TextureFormat::Bgra8Unorm),
            "surface does not support Bgra8Unorm on this adapter/backend (formats: {:?}); \
             the compositor's colour scheme assumes this format specifically, so silently \
             substituting an sRGB format would be wrong, not just different",
            caps.formats,
        );

        let config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: TextureFormat::Bgra8Unorm,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: CompositeAlphaMode::Auto,
            view_formats: vec![],
        };
        surface.configure(device, &config);

        Self {
            surface,
            device: device.clone(),
            config,
        }
    }

    /// Reconfigures the swapchain for a new window size. A no-op at 0x0
    /// (minimized windows report this on some platforms); wgpu requires
    /// nonzero dimensions, so both are clamped to at least 1.
    pub fn resize(&mut self, device: &Device, size: PhysicalSize<u32>) {
        self.config.width = size.width.max(1);
        self.config.height = size.height.max(1);
        self.surface.configure(device, &self.config);
    }

    /// Acquires the next presentable texture.
    ///
    /// Returns `None` for the surface's transient not-ready states --
    /// `Timeout` and `Occluded` -- which windowing systems raise routinely
    /// (a minimized or backgrounded window, a frame that briefly took too
    /// long) and which their own documentation says to handle by skipping
    /// the frame and trying again next redraw, not by treating them as
    /// errors. This deviates from a bare `-> SurfaceTexture` return (the
    /// brief's original shape): that signature has no way to express "there
    /// is no frame to draw right now," and forcing one of these routine
    /// states through `.expect()` would panic the app the first time the
    /// window is minimized.
    ///
    /// Reconfigures and retries once on `Outdated` (e.g. a resize the
    /// window's `Resized` event hasn't caught up with yet). Panics on `Lost`
    /// or `Validation`, which retrying cannot fix.
    pub fn acquire(&self) -> Option<SurfaceTexture> {
        match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(texture) | CurrentSurfaceTexture::Suboptimal(texture) => {
                Some(texture)
            }
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => None,
            CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                match self.surface.get_current_texture() {
                    CurrentSurfaceTexture::Success(texture)
                    | CurrentSurfaceTexture::Suboptimal(texture) => Some(texture),
                    CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => None,
                    other => panic!("could not acquire a surface texture after reconfigure: {other:?}"),
                }
            }
            other => panic!("could not acquire a surface texture: {other:?}"),
        }
    }

    /// Always `Bgra8Unorm` — see the struct docs.
    pub fn format(&self) -> TextureFormat {
        TextureFormat::Bgra8Unorm
    }
}
