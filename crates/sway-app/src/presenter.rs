//! What gets put on screen once Bevy has updated. `ShowPresenter` blits the
//! viewport fullscreen, no masonry, no vello. `EditorPresenter` (Task 4) adds
//! a masonry `RenderRoot`, painted through vello into a transparent UI
//! texture; Task 5 makes masonry's widget tree decide the viewport rect
//! (`sway_editor::EditorUi::viewport_rect`) instead of a hardcoded inset.

use bevy::app::App;
use bevy::ecs::reflect::AppTypeRegistry;
use bevy::math::UVec2;
use crossbeam_channel::Sender;
use masonry_core::core::CursorIcon;
use sway_gpu::{
    Compositor, GpuContext, Quad, UiRenderer, UiTexture, ViewportTexture, WindowSurface,
};
use sway_graph::{EditorCommand, Graph, GraphCommand, ViewportInput};
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
pub struct EditorPresenter {
    editor: sway_editor::EditorUi,
    ui_texture: UiTexture,
    ui_renderer: UiRenderer,
}

impl EditorPresenter {
    pub fn new(
        gpu: &GpuContext,
        size: PhysicalSize<u32>,
        scale_factor: f64,
        commands: Sender<EditorCommand>,
        graph_commands: Sender<GraphCommand>,
        viewport_input: Sender<ViewportInput>,
    ) -> Self {
        let mut editor = sway_editor::EditorUi::new(size, scale_factor, commands, viewport_input);
        editor.set_graph_commands(graph_commands);
        let ui_texture = UiTexture::new(&gpu.device, size.width.max(1), size.height.max(1));
        let ui_renderer = UiRenderer::new(gpu.device.clone(), gpu.queue.clone());
        Self {
            editor,
            ui_texture,
            ui_renderer,
        }
    }

    /// The pending cursor request, if any. Forwards to `EditorUi::take_cursor`;
    /// reading it clears it, so the shell applies it at most once per request.
    pub fn take_cursor(&mut self) -> Option<CursorIcon> {
        self.editor.take_cursor()
    }

    /// What the toolbar has asked for. Drained by the shell each redraw.
    pub fn take_file_requests(&mut self) -> Vec<sway_editor::FileRequest> {
        self.editor.take_file_requests()
    }

    /// What the toolbar has asked for. Drained by the shell each redraw.
    pub fn take_view_requests(&mut self) -> Vec<sway_editor::ViewRequest> {
        self.editor.take_view_requests()
    }

    /// Forwards one winit window event to the masonry widget tree. Most
    /// winit events don't translate into a masonry event at all (redraws,
    /// resizes, close requests are the host's job, not `RenderRoot`'s); see
    /// `EditorUi::handle_winit_event`'s docs for which.
    pub fn handle_winit_event(&mut self, scale_factor: f64, event: &winit::event::WindowEvent) {
        self.editor.handle_winit_event(scale_factor, event);
    }

    /// Tells masonry about a window resize. Does *not* touch the viewport
    /// texture -- that is resized inside `present` from masonry's current
    /// `viewport_rect` (physical pixels).
    pub fn resize(&mut self, size: PhysicalSize<u32>, scale_factor: f64) {
        self.editor.resize(size, scale_factor);
    }

    /// Forwards a DPI scale-factor change without a size change (winit's
    /// `ScaleFactorChanged`), matching `masonry_winit`.
    pub fn rescale(&mut self, scale_factor: f64) {
        self.editor.rescale(scale_factor);
    }

    /// Reads one frame's graph state out of the Bevy world and pushes it into
    /// the widget tree.
    ///
    /// Called from `present` between the previous frame's `app.update()` and
    /// this frame's masonry redraw, which is the one place the two halves of
    /// the process meet (design D11). The borrow of `&Graph` only has to
    /// survive this call — there is no `Arc`, no mutex and no copy, because
    /// the UI read and the tick never overlap.
    fn apply_graph(&mut self, app: &App) {
        let type_registry = app.world().resource::<AppTypeRegistry>().clone();
        if let Some(graph) = app.world().get_resource::<Graph>() {
            self.editor.apply_graph(graph, &type_registry.read());
        }
        if let Some(transport) = app.world().get_resource::<sway_midi::Transport>() {
            self.editor.apply_transport(transport);
        }
    }

    /// One frame, in the fixed, load-bearing order (controller dispatch
    /// ruling R5): masonry redraws first (so a viewport resize costs no
    /// frame of lag), then -- Task 5 -- the viewport rect is read off the
    /// tagged `ViewportPlaceholder` widget itself
    /// (`sway_editor::EditorUi::viewport_rect`, not the `VisualLayerPlan` --
    /// see that method's doc comment for why) and the viewport texture is
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
        // 0. The graph, from the previous frame's `app.update()`.
        self.apply_graph(app);

        // 1. Masonry first.
        let plan = self.editor.redraw();
        let scale = self.editor.scale_factor();

        // 2/3. The viewport rect now comes from masonry's widget tree
        // (Task 5) instead of the old hardcoded bootstrap size.
        // `viewport_rect` is logical window space; the compositor and the
        // Bevy texture want physical pixels, so scale here.
        // `None` is a legitimate state -- the widget isn't in the tree --
        // not an error (R2); in that case the viewport texture is left alone
        // and no viewport quad is drawn below.
        let rect = self
            .editor
            .viewport_rect()
            .map(|logical| kurbo::Affine::scale(scale).transform_rect_bbox(logical));
        if let Some(rect) = rect {
            let rect_width = rect.width().round().max(1.0) as u32;
            let rect_height = rect.height().round().max(1.0) as u32;
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
        // viewport quad it composites over is inset). `flatten` applies
        // `scale_factor` so logical masonry coords land in physical pixels.
        self.ui_texture
            .resize(&gpu.device, surface.width(), surface.height());
        let scene = sway_editor::EditorUi::flatten(&plan, scale);
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
