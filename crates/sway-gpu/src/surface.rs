//! The window's presentable swapchain: configuration, resize, and frame
//! acquisition.

use std::sync::Arc;

use wgpu::{
    Adapter, CompositeAlphaMode, CurrentSurfaceTexture, Device, Instance, PresentMode, Queue,
    Surface, SurfaceConfiguration, SurfaceTexture, TextureFormat, TextureUsages,
};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::compositor::Compositor;
use crate::frame::Frame;

/// Whether the host is willing to wait for the display's refresh before
/// presenting.
///
/// `Wait` is the default and is what a show wants: `Fifo` never tears. `DontWait`
/// is the opt-in a capture run makes, where the host's own pacing should be the
/// only clock and a display slower than the show's rate must not bound it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VsyncPreference {
    Wait,
    DontWait,
}

/// Picks the present mode from what the surface actually offers.
///
/// `SurfaceCapabilities::present_modes` is the authority — Metal does not
/// offer the same set everywhere — so this is a preference, not a demand.
/// `Mailbox` first because it stops blocking without tearing; `Immediate`
/// second because it tears, which is a fair trade for a capture run and a poor
/// one for a show; `Fifo` last, which every surface supports.
///
/// A pure function of the preference and the advertised list, so the fallback
/// order is testable without a window.
pub fn choose_present_mode(preference: VsyncPreference, available: &[PresentMode]) -> PresentMode {
    if preference == VsyncPreference::Wait {
        return PresentMode::Fifo;
    }
    for candidate in [PresentMode::Mailbox, PresentMode::Immediate] {
        if available.contains(&candidate) {
            return candidate;
        }
    }
    PresentMode::Fifo
}

/// The window's swapchain.
///
/// Always configured `Bgra8Unorm` (non-sRGB — the compositor shader assumes
/// every source texture already holds sRGB-encoded bytes and passes them
/// through untouched, so the presented format must not itself apply sRGB
/// encoding again). The present mode is chosen from the surface's own
/// capabilities against a [`VsyncPreference`]; [`WindowSurface::present_mode`]
/// reports which one it got, so a caller that asked not to wait can say when
/// the request could not be honoured.
pub struct WindowSurface {
    surface: Surface<'static>,
    device: Device,
    config: SurfaceConfiguration,
    readable: bool,
}

impl WindowSurface {
    /// Creates and configures the surface for `window` against `adapter`.
    ///
    /// `instance` must be the same instance used to (or that will) request
    /// `adapter` — wgpu requires a surface and the adapter checked against it
    /// to come from one instance.
    pub fn new(
        instance: &Instance,
        device: &Device,
        adapter: &Adapter,
        window: Arc<Window>,
        vsync: VsyncPreference,
    ) -> Self {
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

        // `COPY_SRC` is what makes the presented image readable back, which is
        // the whole-window capture's only honest source: anything else would
        // be a re-composite that merely ought to match the screen. Requested
        // only where the surface advertises it, and never load-bearing for an
        // ordinary run — `readable()` tells the capture path whether it can
        // proceed.
        let readable = caps.usages.contains(TextureUsages::COPY_SRC);
        let usage = if readable {
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC
        } else {
            TextureUsages::RENDER_ATTACHMENT
        };

        let config = SurfaceConfiguration {
            usage,
            format: TextureFormat::Bgra8Unorm,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: choose_present_mode(vsync, &caps.present_modes),
            desired_maximum_frame_latency: 2,
            alpha_mode: CompositeAlphaMode::Auto,
            view_formats: vec![],
        };
        surface.configure(device, &config);

        Self {
            surface,
            device: device.clone(),
            config,
            readable,
        }
    }

    /// The present mode the surface was actually configured with. `Fifo` after
    /// a [`VsyncPreference::DontWait`] request means the request could not be
    /// honoured and the caller should say so.
    pub fn present_mode(&self) -> PresentMode {
        self.config.present_mode
    }

    /// Whether the presented texture can be copied out of. False on a surface
    /// that does not advertise `COPY_SRC`, where the whole-window capture has
    /// no honest source and must refuse rather than write something else.
    pub fn readable(&self) -> bool {
        self.readable
    }

    /// Reconfigures the swapchain for a new window size. A no-op at 0x0
    /// (minimized windows report this on some platforms); wgpu requires
    /// nonzero dimensions, so both are clamped to at least 1.
    pub fn resize(&mut self, device: &Device, size: PhysicalSize<u32>) {
        self.config.width = size.width.max(1);
        self.config.height = size.height.max(1);
        self.surface.configure(device, &self.config);
    }

    /// Begins a frame: acquires the next presentable texture and wraps it
    /// (plus a fresh command encoder) in a [`Frame`], so the caller never
    /// needs to create a `wgpu::TextureView` or `wgpu::CommandEncoder`
    /// itself -- every wgpu object a frame touches is created inside
    /// `sway-gpu`.
    ///
    /// Returns `None` for the surface's transient not-ready states --
    /// `Timeout` and `Occluded` -- which windowing systems raise routinely
    /// (a minimized or backgrounded window, a frame that briefly took too
    /// long) and which their own documentation says to handle by skipping
    /// the frame and trying again next redraw, not by treating them as
    /// errors. Forcing one of these routine states through `.expect()`
    /// would panic the app the first time the window is minimized, so
    /// there is no frame to return in that case rather than a broken one.
    ///
    /// Reconfigures and retries once on `Outdated` (e.g. a resize the
    /// window's `Resized` event hasn't caught up with yet). Panics on `Lost`
    /// or `Validation`, which retrying cannot fix.
    pub fn begin_frame<'a>(
        &self,
        device: &Device,
        queue: &Queue,
        compositor: &'a mut Compositor,
    ) -> Option<Frame<'a>> {
        let surface_texture = self.acquire()?;
        Some(Frame::new(device, queue, compositor, surface_texture))
    }

    /// Acquires the next presentable texture, or `None` for a transient
    /// not-ready state. Private: [`Self::begin_frame`] is the only public
    /// way to reach a frame, so a caller outside this crate can never hold a
    /// bare `wgpu::SurfaceTexture` and build its own view/encoder from it.
    fn acquire(&self) -> Option<SurfaceTexture> {
        match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(texture)
            | CurrentSurfaceTexture::Suboptimal(texture) => Some(texture),
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => None,
            CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                match self.surface.get_current_texture() {
                    CurrentSurfaceTexture::Success(texture)
                    | CurrentSurfaceTexture::Suboptimal(texture) => Some(texture),
                    CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => None,
                    other => {
                        panic!("could not acquire a surface texture after reconfigure: {other:?}")
                    }
                }
            }
            other => panic!("could not acquire a surface texture: {other:?}"),
        }
    }

    /// Always `Bgra8Unorm` — see the struct docs.
    pub fn format(&self) -> TextureFormat {
        TextureFormat::Bgra8Unorm
    }

    /// The swapchain's current configured width, in physical pixels.
    pub fn width(&self) -> u32 {
        self.config.width
    }

    /// The swapchain's current configured height, in physical pixels.
    pub fn height(&self) -> u32 {
        self.config.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_for_the_refresh_is_fifo_whatever_else_is_offered() {
        // A show wants `Fifo`: it never tears. Offering `Mailbox` is not a
        // reason to take it.
        assert_eq!(
            choose_present_mode(
                VsyncPreference::Wait,
                &[PresentMode::Mailbox, PresentMode::Immediate, PresentMode::Fifo],
            ),
            PresentMode::Fifo
        );
    }

    #[test]
    fn not_waiting_prefers_mailbox_over_immediate() {
        // Both stop blocking; only `Immediate` tears.
        assert_eq!(
            choose_present_mode(
                VsyncPreference::DontWait,
                &[PresentMode::Fifo, PresentMode::Immediate, PresentMode::Mailbox],
            ),
            PresentMode::Mailbox
        );
    }

    #[test]
    fn not_waiting_falls_back_to_immediate_then_to_fifo() {
        assert_eq!(
            choose_present_mode(
                VsyncPreference::DontWait,
                &[PresentMode::Fifo, PresentMode::Immediate],
            ),
            PresentMode::Immediate
        );
        // A surface that offers neither must still start, waiting as it does
        // by default — the caller reports that the request went unhonoured by
        // seeing `Fifo` come back.
        assert_eq!(
            choose_present_mode(VsyncPreference::DontWait, &[PresentMode::Fifo]),
            PresentMode::Fifo
        );
    }
}
