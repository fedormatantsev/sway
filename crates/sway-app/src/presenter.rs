//! What gets put on screen once Bevy has updated. `ShowPresenter` is the only
//! presenter for M1b Task 3; `EditorPresenter` (masonry + vello UI on top of
//! the same viewport quad) arrives at Task 4.

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
        // Timeout). Skip it and let the caller request another redraw -- this
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
