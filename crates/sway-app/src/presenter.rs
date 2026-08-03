//! What gets put on screen once Bevy has updated. `ShowPresenter` blits the
//! viewport fullscreen, no masonry, no vello. `EditorPresenter` (Task 4) adds
//! a masonry `RenderRoot`, painted through vello into a transparent UI
//! texture; Task 5 makes masonry's widget tree decide the viewport rect
//! (EditorUi::viewport_rect) instead of a hardcoded inset.
//!
//! NOTE (Task 7): EditorPresenter is stubbed due to sway-editor not being
//! migrated yet. The app runs fine with ShowPresenter (the default).

use bevy::app::App;
use bevy::math::UVec2;
use sway_gpu::{Compositor, GpuContext, Quad, UiRenderer, UiTexture, ViewportTexture, WindowSurface};
use winit::dpi::PhysicalSize;

/// Blits the viewport fullscreen. No masonry, no vello.
pub struct ShowPresenter;

impl ShowPresenter {
    pub fn present(
        &mut self,
        app: &mut App,
        gpu: &GpuContext,
        surface: &WindowSurface,
        viewport: &ViewportTexture,
        compositor: &mut Compositor,
    ) {
        app.update();

        // `None` means the surface is not presentable this frame (Occluded /
        // Timeout). Skip it and let the caller request another redraw -- this
        // is routine, not an error.
        let Some(mut frame) = surface.begin_frame(&gpu.device, &gpu.queue, compositor) else {
            return;
        };

        frame.composite(&[Quad {
            view: &viewport.sample_view,
            dst: kurbo::Rect::new(0.0, 0.0, surface.width() as f64, surface.height() as f64),
            blend: false,
        }]);

        frame.present();
    }
}

/// Bootstrap size for the editor's Bevy viewport texture (logical CSS
/// pixels), used only before the first `EditorPresenter::present` runs and
/// discovers the real layout. `sway_editor` no longer has a fixed viewport
/// size to match -- the viewport pane's actual size depends on the window
/// size and the three-pane `Split` layout's fractions -- so this is purely
/// an arbitrary, reasonable starting point; the first `present` call resizes
/// it to whatever `EditorUi::viewport_rect` actually reports.
pub const EDITOR_VIEWPORT_SIZE: kurbo::Size = kurbo::Size::new(640.0, 360.0);

/// Masonry + vello UI, composited over the live Bevy viewport.
///
/// Owns the UI's offscreen texture and the vello renderer that paints into
/// it -- both are per-window resources tied to the shared device, just like
/// `Compositor`, so they live for the run's duration rather than being
/// recreated per frame.
/// EditorPresenter is disabled in this task (Task 7) because sway-editor is
/// expected to be broken due to unfinished migrations. Using a stub that panics
/// if instantiated; the app defaults to ShowPresenter.
pub struct EditorPresenter;

impl EditorPresenter {
    pub fn new(_gpu: &GpuContext, _size: PhysicalSize<u32>, _scale_factor: f64) -> Self {
        panic!("EditorPresenter is disabled in Task 7: sway-editor has not been migrated");
    }

    pub fn handle_winit_event(&mut self, _scale_factor: f64, _event: &winit::event::WindowEvent) {
        panic!("EditorPresenter is disabled in Task 7: sway-editor has not been migrated");
    }

    pub fn resize(&mut self, _size: PhysicalSize<u32>, _scale_factor: f64) {
        panic!("EditorPresenter is disabled in Task 7: sway-editor has not been migrated");
    }

    pub fn rescale(&mut self, _scale_factor: f64) {
        panic!("EditorPresenter is disabled in Task 7: sway-editor has not been migrated");
    }

    pub fn present(
        &mut self,
        _app: &mut App,
        _gpu: &GpuContext,
        _surface: &WindowSurface,
        _viewport: &mut ViewportTexture,
        _compositor: &mut Compositor,
    ) {
        panic!("EditorPresenter is disabled in Task 7: sway-editor has not been migrated");
    }
}
