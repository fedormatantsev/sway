//! What gets put on screen once Bevy has updated. `ShowPresenter` blits the
//! viewport fullscreen, no masonry, no vello. `EditorPresenter` (Task 4) adds
//! a masonry `RenderRoot`, painted through vello into a transparent UI
//! texture; Task 5 makes masonry's widget tree decide the viewport rect
//! (`sway_editor::external::viewport_rect`) instead of a hardcoded inset.

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

/// The editor's viewport rect, in physical pixels, used only to size the
/// viewport texture *before* the first `EditorPresenter::present` call (i.e.
/// before masonry has laid out anything yet). Every frame after that,
/// `present` reads the real rect from masonry's `External` visual layer via
/// `sway_editor::external::viewport_rect` (Task 5) -- this constant no
/// longer drives where the viewport is drawn, only this one bootstrap size.
pub const EDITOR_VIEWPORT_RECT: kurbo::Rect = kurbo::Rect::new(40.0, 40.0, 40.0 + 640.0, 40.0 + 360.0);

/// Masonry + vello UI, composited over the live Bevy viewport.
///
/// Owns the UI's offscreen texture and the vello renderer that paints into
/// it -- both are per-window resources tied to the shared device, just like
/// `Compositor`, so they live for the run's duration rather than being
/// recreated per frame.
pub struct EditorPresenter {
    editor: sway_editor::EditorUi,
    ui_texture: UiTexture,
    ui_renderer: UiRenderer,
}

impl EditorPresenter {
    pub fn new(gpu: &GpuContext, size: PhysicalSize<u32>, scale_factor: f64) -> Self {
        let editor = sway_editor::EditorUi::new(size, scale_factor);
        let ui_texture = UiTexture::new(&gpu.device, size.width.max(1), size.height.max(1));
        let ui_renderer = UiRenderer::new(gpu.device.clone(), gpu.queue.clone());
        Self {
            editor,
            ui_texture,
            ui_renderer,
        }
    }

    /// Forwards one winit window event to the masonry widget tree. Most
    /// winit events don't translate into a masonry event at all (redraws,
    /// resizes, close requests are the host's job, not `RenderRoot`'s); see
    /// `EditorUi::handle_winit_event`'s docs for which.
    pub fn handle_winit_event(&mut self, scale_factor: f64, event: &winit::event::WindowEvent) {
        self.editor.handle_winit_event(scale_factor, event);
    }

    /// Tells masonry about a window resize. Does *not* touch the viewport
    /// texture -- that stays pinned at [`EDITOR_VIEWPORT_RECT`]'s size,
    /// resized (if at all) inside `present`, not here.
    pub fn resize(&mut self, size: PhysicalSize<u32>, scale_factor: f64) {
        self.editor.resize(size, scale_factor);
    }

    /// One frame, in the fixed, load-bearing order (controller dispatch
    /// ruling R5): masonry redraws first (so a viewport resize costs no
    /// frame of lag), then -- Task 5 -- the viewport rect is read from
    /// masonry's `External` visual layer
    /// (`sway_editor::external::viewport_rect`) and the viewport texture is
    /// resized to match if needed, then Bevy is re-pointed at it and
    /// updates, then vello paints masonry's scene into the transparent UI
    /// texture, then the compositor draws the viewport quad first (if any --
    /// R2, controller dispatch ruling) and the UI quad second (`blend:
    /// true`, over the viewport), then the frame is presented.
    pub fn present(
        &mut self,
        app: &mut App,
        gpu: &GpuContext,
        surface: &WindowSurface,
        viewport: &mut ViewportTexture,
        compositor: &mut Compositor,
    ) {
        // 1. Masonry first.
        let plan = self.editor.redraw();

        // 2/3. The viewport rect now comes from masonry's widget tree
        // (Task 5) instead of the old hardcoded `EDITOR_VIEWPORT_RECT`.
        // `None` is a legitimate state -- no external boundary in the
        // current layout -- not an error (R2); in that case the viewport
        // texture is left alone and no viewport quad is drawn below.
        let rect = sway_editor::external::viewport_rect(&plan);
        if let Some(rect) = rect {
            let (rect_width, rect_height) = (rect.width() as u32, rect.height() as u32);
            viewport.resize(&gpu.device, rect_width, rect_height);
            // Resizing just recreated the texture (and its views) if the
            // size changed, invalidating whatever `ManualTextureViews` entry
            // Bevy held -- repoint it before `app.update()` runs, every
            // frame, not just on an actual resize; the call is cheap and
            // always correct.
            sway_runtime::headless::set_viewport_view(
                app,
                viewport,
                UVec2::new(rect_width, rect_height),
            );
        }

        // 4. Bevy updates regardless of whether there's a viewport rect this
        // frame -- if `rect` is `None`, Bevy still renders into whatever the
        // viewport texture was last pointed at, but that output is simply
        // never composited (no viewport quad below), so it's harmless.
        app.update();

        // 5. Masonry's scene into the transparent UI texture, sized to the
        // whole surface (the UI layer covers the whole window; only the
        // viewport quad it composites over is inset).
        self.ui_texture
            .resize(&gpu.device, surface.width(), surface.height());
        let scene = sway_editor::EditorUi::flatten(&plan);
        self.ui_renderer.render_scene(
            &scene,
            &self.ui_texture.view,
            surface.width(),
            surface.height(),
        );

        // 6/7. Composite (viewport, if any, then UI over it) and present.
        // `None` means the surface is not presentable this frame (Occluded /
        // Timeout); skip it, same as `ShowPresenter`.
        let Some(mut frame) = surface.begin_frame(&gpu.device, &gpu.queue, compositor) else {
            return;
        };

        let ui_quad = Quad {
            view: &self.ui_texture.view,
            dst: kurbo::Rect::new(0.0, 0.0, surface.width() as f64, surface.height() as f64),
            blend: true,
        };
        match rect {
            Some(rect) => frame.composite(&[
                Quad {
                    view: &viewport.sample_view,
                    dst: rect,
                    blend: false,
                },
                ui_quad,
            ]),
            None => frame.composite(&[ui_quad]),
        }

        frame.present();
    }
}
