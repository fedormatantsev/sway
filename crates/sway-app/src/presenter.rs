//! What gets put on screen once Bevy has updated. `ShowPresenter` blits the
//! viewport fullscreen, no masonry, no vello. `EditorPresenter` (Task 4) adds
//! a masonry `RenderRoot`, painted through vello into a transparent UI
//! texture; Task 5 makes masonry's widget tree decide the viewport rect
//! (`sway_editor::EditorUi::viewport_rect`) instead of a hardcoded inset.

use bevy::app::App;
use bevy::ecs::change_detection::DetectChangesMut;
use bevy::ecs::reflect::AppTypeRegistry;
use bevy::math::UVec2;
use crossbeam_channel::Sender;
use masonry_core::core::CursorIcon;
use sway_editor::edit::EditorEdit;
use sway_editor_viewport::{ViewportCamera, fit_aspect};
use sway_gpu::{
    Compositor, GpuContext, Quad, ReadbackPool, UiRenderer, UiTexture, ViewportTexture,
    WindowSurface,
};
use sway_graph::Graph;
use sway_graph::graph::NodeId;
use sway_runtime::{CameraTargets, PresentedCamera};
use sway_selection::Selection;
use sway_viewport_input::ViewportInput;
use winit::dpi::PhysicalSize;

/// A whole-window readback to encode into this frame, if one was asked for.
///
/// The ticket is the pool's tag; the pool is the caller's, because it also
/// owns the comparison the capture settles by.
pub type WindowReadback<'a> = Option<(u64, &'a mut ReadbackPool)>;

/// The rectangle `resolution` occupies when fitted into a pane of `size`,
/// offset to `origin` — the letterbox, in the destination's own pixels.
///
/// The arithmetic is `sway_editor_viewport::fit_aspect`, shared with the
/// editor preview so the window and the pane cannot disagree about where a
/// camera's image goes.
fn letterboxed(origin: (f64, f64), size: UVec2, resolution: UVec2) -> kurbo::Rect {
    let fit = fit_aspect(size, resolution);
    kurbo::Rect::new(
        origin.0 + f64::from(fit.offset.x),
        origin.1 + f64::from(fit.offset.y),
        origin.0 + f64::from(fit.offset.x + fit.size.x),
        origin.1 + f64::from(fit.offset.y + fit.size.y),
    )
}

/// Blits the presented camera into the window. No masonry, no vello.
pub struct ShowPresenter;

impl ShowPresenter {
    pub fn present(
        &mut self,
        app: &mut App,
        gpu: &GpuContext,
        surface: &WindowSurface,
        compositor: &mut Compositor,
        window_readback: WindowReadback<'_>,
    ) {
        app.update();

        // `None` means the surface is not presentable this frame (Occluded /
        // Timeout). Skip it and let the caller request another redraw -- this
        // is routine, not an error.
        let Some(mut frame) = surface.begin_frame(&gpu.device, &gpu.queue, compositor) else {
            return;
        };

        // What is presented is authored: the camera the document's `Output`
        // node names, and nothing else. No output node, or none with a camera,
        // composites no viewport quad at all — a case the compositor already
        // handles, and the specified behaviour rather than a fallback to
        // whichever camera happened to render last.
        let world = app.world();
        let presented = world.resource::<PresentedCamera>().0;
        let targets = world.resource::<CameraTargets>();
        let quad = presented.and_then(|presented| {
            let target = targets.target(presented.node)?;
            Some(Quad {
                view: &target.sample_view,
                // The camera's authored resolution and the window's size are
                // independent and are not reconciled by changing either one:
                // the image is fitted, and the rest of the window is
                // letterboxing.
                dst: letterboxed(
                    (0.0, 0.0),
                    UVec2::new(surface.width(), surface.height()),
                    presented.resolution,
                ),
                blend: false,
            })
        });

        match quad {
            Some(quad) => frame.composite(&[quad]),
            None => frame.composite(&[]),
        }

        if let Some((ticket, pool)) = window_readback {
            // After the composite, before the present: the copy is encoded
            // into this frame's encoder and submitted with it.
            let _ = frame.read_back(pool, ticket);
        }

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
    /// What each entry of the toolbar's camera list means, in the same order.
    /// `None` is the editor's own camera; the rest are the document's camera
    /// nodes. Rebuilt with the list, so an index the toolbar reports back
    /// always resolves against the list it was showing.
    camera_choices: Vec<Option<NodeId>>,
}

impl EditorPresenter {
    pub fn new(
        gpu: &GpuContext,
        size: PhysicalSize<u32>,
        scale_factor: f64,
        commands: Sender<EditorEdit>,
        viewport_input: Sender<ViewportInput>,
    ) -> Self {
        let editor = sway_editor::EditorUi::new(size, scale_factor, commands, viewport_input);
        let ui_texture = UiTexture::new(&gpu.device, size.width.max(1), size.height.max(1));
        let ui_renderer = UiRenderer::new(gpu.device.clone(), gpu.queue.clone());
        Self {
            editor,
            ui_texture,
            ui_renderer,
            camera_choices: Vec::new(),
        }
    }

    /// What the toolbar's camera at `index` means.
    ///
    /// `None` for an index the last list did not have — a press against a
    /// list that has since changed, which is a no-op rather than a wrong
    /// camera.
    pub fn camera_choice(&self, index: usize) -> Option<ViewportCamera> {
        Some(match self.camera_choices.get(index)? {
            None => ViewportCamera::Editor,
            Some(node) => ViewportCamera::Node(*node),
        })
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
    /// the process meet (design D11). The borrow only has to survive this call
    /// — there is no `Arc`, no mutex and no copy, because the UI read and the
    /// tick never overlap.
    ///
    /// Mutable because the editor writes its own canvas placement onto the
    /// nodes' annotations here rather than sending it as an edit: this is the
    /// moment it holds the graph, and an annotation is not a scene change.
    fn apply_graph(&mut self, app: &mut App) {
        let type_registry = app.world().resource::<AppTypeRegistry>().clone();
        // Both resources at once: `apply_graph` writes the editor's own
        // placement and selection while it reads. `resource_scope` takes the
        // selection out so the graph can still be borrowed alongside it.
        if app.world().get_resource::<Graph>().is_some()
            && app.world().get_resource::<Selection>().is_some()
        {
            app.world_mut().resource_scope(
                |world, mut selection: bevy::ecs::change_detection::Mut<Selection>| {
                    let mut graph = world.resource_mut::<Graph>();
                    self.editor.apply_graph(
                        graph.bypass_change_detection(),
                        &mut selection,
                        &type_registry.read(),
                    );
                },
            );
        }
        if let Some(transport) = app.world().get_resource::<sway_midi::Transport>() {
            self.editor.apply_transport(transport);
        }
        self.apply_cameras(app);
    }

    /// Offers the editor's own camera plus every camera node in the document,
    /// so a document with several cameras can be inspected through each of
    /// them without rewiring the graph.
    ///
    /// Named and ordered here rather than in `sway-editor`, which has no
    /// notion of a camera at all: the shell is what depends on both the
    /// runtime and the editor. Order is the graph's own node order, so a
    /// steady document keeps steady names.
    fn apply_cameras(&mut self, app: &mut App) {
        let mut choices: Vec<Option<NodeId>> = vec![None];
        if let Some(graph) = app.world().get_resource::<Graph>() {
            let mut cameras: Vec<NodeId> = graph
                .iter()
                .filter(|(_, node)| {
                    node.value()
                        .downcast_ref::<sway_runtime::nodes::Camera>()
                        .is_some()
                })
                .map(|(id, _)| id)
                .collect();
            cameras.sort_unstable();
            choices.extend(cameras.into_iter().map(Some));
        }

        if choices != self.camera_choices {
            self.camera_choices = choices;
        }
        let names: Vec<String> = self
            .camera_choices
            .iter()
            .enumerate()
            .map(|(index, choice)| match choice {
                None => "Editor".to_string(),
                Some(_) => format!("Camera {index}"),
            })
            .collect();
        self.editor.apply_cameras(&names);
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
        window_readback: WindowReadback<'_>,
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

        // Previewing a camera node draws *that camera's* target into the pane,
        // letterboxed to its authored aspect — not the pane-sized viewport
        // texture, which only the editor's own camera renders into. The target
        // is ordinarily already the fitted size (the editor asked for exactly
        // that); where a graph consumer needed the authored resolution it is
        // larger, and the same rect samples it down.
        let world = app.world();
        let previewed = world
            .get_resource::<ViewportCamera>()
            .and_then(|active| active.node())
            .zip(world.get_resource::<CameraTargets>());
        let preview_quad = rect.and_then(|rect| {
            let (node, targets) = previewed?;
            let target = targets.target(node)?;
            Some(Quad {
                view: &target.sample_view,
                dst: letterboxed(
                    (rect.x0, rect.y0),
                    UVec2::new(
                        rect.width().round().max(1.0) as u32,
                        rect.height().round().max(1.0) as u32,
                    ),
                    UVec2::new(target.width, target.height),
                ),
                blend: false,
            })
        });

        match (preview_quad, rect) {
            (Some(preview), _) => frame.composite(&[preview, ui_quad]),
            (None, Some(rect)) => frame.composite(&[
                Quad {
                    view: &viewport.sample_view,
                    dst: rect,
                    blend: false,
                },
                ui_quad,
            ]),
            (None, None) => frame.composite(&[ui_quad]),
        }

        if let Some((ticket, pool)) = window_readback {
            // After the composite, before the present — the whole window
            // exactly as displayed, editor interface included.
            let _ = frame.read_back(pool, ticket);
        }

        frame.present();
    }
}
