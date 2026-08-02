//! The masonry half of the editor: a widget tree and the events that reach it.
//!
//! Depends on `bevy_ecs`, `bevy_reflect`, `bevy_transform` and `sway-graph`,
//! because the editor reads the live world directly (main design §2.8, §3:
//! "The editor links `sway-graph` regardless"). It deliberately depends on
//! none of `bevy` (the full facade), `bevy_render`, `wgpu`, `vello`, or
//! `imaging_vello` -- nothing here creates a device or touches a pipeline,
//! which is the M1b invariant that actually matters. `winit` appears only
//! because `ui-events-winit` takes `&winit::event::WindowEvent`; nothing here
//! draws with it.

pub mod canvas;
pub mod external;
pub mod node_box;
pub mod scene_tree;
pub mod snapshot;

#[cfg(test)]
mod test_graph;

use std::sync::Arc;
use std::time::Instant;

use imaging::record::replay_transformed;
use masonry_core::app::{
    RenderRoot, RenderRootOptions, RenderRootSignal, VisualLayerKind, VisualLayerPlan,
    WindowSizePolicy,
};
use masonry_core::core::{NewWidget, TextEvent, Widget, WindowEvent as MasonryWindowEvent};
use masonry::kurbo::{Affine, Point};
use masonry::layout::AsUnit;
use masonry::properties::Dimensions;
use ui_events_winit::{WindowEventReducer, WindowEventTranslation};
use winit::dpi::PhysicalSize;

use crate::canvas::GraphCanvas;
use crate::external::ViewportPlaceholder;

/// The Bevy viewport's fixed footprint in the graph canvas, in logical
/// pixels. Matches `EDITOR_VIEWPORT_SIZE` in `sway-app`, purely for visual
/// continuity across Tasks 5-6 -- nothing requires this exact number now
/// that masonry's widget tree decides the rect.
const VIEWPORT_WIDTH: f64 = 640.0;
const VIEWPORT_HEIGHT: f64 = 360.0;

/// Builds the root widget: a [`GraphCanvas`] carrying a handful of
/// placeholder node boxes and edges around Task 5's [`ViewportPlaceholder`],
/// which keeps its seat in the tree as one of the canvas's children so the
/// Bevy viewport still appears (`sway_editor::external::viewport_rect` reads
/// its layout box back out of the `VisualLayerPlan`, same as before).
///
/// Deviation from a stale claim this function's predecessor's doc comment
/// made: the previous placeholder root relied on `Dimensions::MAX` to fill
/// the window, attributing that to `RenderRoot`'s `LayerStack` also using
/// `Dimensions::MAX` internally. That was wrong -- under
/// `WindowSizePolicy::User`, `run_layout_pass` resolves the root via
/// `SizeDef::fixed(window_size)` and `LayerStack::layout` forwards it
/// unconditionally, so `Dimensions::MAX` was inert on that path the whole
/// time. `GraphCanvas` below sets no `Dimensions` property at all and fills
/// the window regardless, which is the actual mechanism at work.
fn graph_root() -> NewWidget<dyn Widget> {
    let viewport = ViewportPlaceholder::new()
        .prepare()
        .with_props(Dimensions::fixed(VIEWPORT_WIDTH.px(), VIEWPORT_HEIGHT.px()));

    GraphCanvas::new()
        .with_node(0, Point::new(20.0, 20.0), "Source")
        .with_node(1, Point::new(20.0, 160.0), "Filter")
        .with_node(2, Point::new(20.0, 300.0), "Transform")
        .with_node(3, Point::new(860.0, 20.0), "Output")
        .with_node(4, Point::new(860.0, 160.0), "Debug View")
        .with_node(5, Point::new(860.0, 300.0), "Camera")
        .with_edge(0, 1)
        .with_edge(1, 2)
        .with_edge(3, 4)
        .with_edge(4, 5)
        .with_viewport(
            viewport,
            Point::new(200.0, 20.0),
            masonry::kurbo::Size::new(VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
        )
        .prepare()
        .erased()
}

/// The masonry widget tree, driven by winit events, one `RenderRoot` per
/// window. There is exactly one window in this app, so exactly one
/// `EditorUi`.
pub struct EditorUi {
    root: RenderRoot,
    reducer: WindowEventReducer,
    scale_factor: f64,
    /// When `redraw` last pumped an anim frame. See `redraw`'s docs: this
    /// host drives masonry's animation clock itself rather than through a
    /// real windowing event, because nothing else in this shell does.
    last_anim_tick: Instant,
}

impl EditorUi {
    pub fn new(size: PhysicalSize<u32>, scale_factor: f64) -> Self {
        let root = RenderRoot::new(
            graph_root(),
            // R2 (controller dispatch ruling): the signal sink is a no-op.
            // Masonry emits `RenderRootSignal`s for cursor changes, IME, and
            // window requests (resize, title, exit, ...); a spike driving one
            // hardcoded window with no interactive widgets needs none of
            // them. Dropped silently and deliberately -- Task 8 records this
            // as a known simplification, not a bug to fix here.
            |_signal: RenderRootSignal| {},
            RenderRootOptions {
                default_properties: Arc::new(masonry::theme::default_property_set()),
                use_system_fonts: true,
                size_policy: WindowSizePolicy::User,
                size,
                scale_factor,
                test_font: None,
            },
        );
        Self {
            root,
            reducer: WindowEventReducer::default(),
            scale_factor,
            last_anim_tick: Instant::now(),
        }
    }

    /// Feeds one winit event through `ui-events-winit`'s reducer and, if it
    /// translated to something masonry understands, into the `RenderRoot`.
    ///
    /// Not every winit event translates to a masonry event (e.g. most of
    /// `WindowEvent`'s variants -- `Resized`, `RedrawRequested`,
    /// `CloseRequested`, ... -- reduce to `None`); those are the host's job
    /// (`resize`, `redraw`, the shell's own `CloseRequested` handling), not
    /// this method's.
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

    /// The window's current DPI scale factor (physical pixels per logical
    /// pixel). Used by the host when painting / compositing into a physical
    /// framebuffer.
    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// Tells the `RenderRoot` about a window resize (and, if it changed, a
    /// scale-factor change). Prefer [`rescale`](Self::rescale) for
    /// `ScaleFactorChanged` alone -- masonry_winit sends only `Rescale` then.
    pub fn resize(&mut self, size: PhysicalSize<u32>, scale_factor: f64) {
        if (scale_factor - self.scale_factor).abs() > f64::EPSILON {
            self.scale_factor = scale_factor;
            self.root
                .handle_window_event(MasonryWindowEvent::Rescale(scale_factor));
        }
        self.root
            .handle_window_event(MasonryWindowEvent::Resize(size));
    }

    /// Applies a DPI scale-factor change without resizing. Matches
    /// `masonry_winit`'s handling of winit's `ScaleFactorChanged`.
    pub fn rescale(&mut self, scale_factor: f64) {
        if (scale_factor - self.scale_factor).abs() <= f64::EPSILON {
            return;
        }
        self.scale_factor = scale_factor;
        self.root
            .handle_window_event(MasonryWindowEvent::Rescale(scale_factor));
    }

    /// Runs masonry's paint pass and returns the resulting visual-layer plan.
    ///
    /// Ignores the `Option<TreeUpdate>` `RenderRoot::redraw` also returns
    /// (R4, controller dispatch ruling): accessibility is out of scope for
    /// M1b.
    ///
    /// Pumps a `WindowEvent::AnimFrame` first -- a real gap found while
    /// implementing Task 5, not a pre-existing part of this API. Masonry
    /// resets a widget's `PaintLayerMode` to `Inline` at the top of every
    /// paint pass and only restores it if that widget's own `paint` method
    /// actually runs, which only happens when something (an event, an anim
    /// tick, ...) has marked it dirty. `ViewportPlaceholder`
    /// (`external.rs`) keeps itself dirty via `request_anim_frame`, but that
    /// request is only serviced if the host actually delivers
    /// `WindowEvent::AnimFrame` -- masonry does not invent a clock on its
    /// own. This host has no other source of frame ticks (the signal sink
    /// that would normally carry `RequestAnimFrame` is a no-op, see `new`'s
    /// docs), so `redraw` supplies one directly, every call, using wall-clock
    /// elapsed time since the last call. Confirmed empirically before this
    /// was wired in: an `External` layer that never receives an anim frame
    /// vanishes from the very next `VisualLayerPlan`.
    pub fn redraw(&mut self) -> VisualLayerPlan {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_anim_tick);
        self.last_anim_tick = now;
        self.root
            .handle_window_event(MasonryWindowEvent::AnimFrame(elapsed));

        self.root.redraw().0
    }

    /// Replays every scene layer into one physical-pixel scene.
    ///
    /// Masonry's layer transforms are in logical window space; `scale_factor`
    /// maps them into the physical framebuffer (same as masonry_imaging's
    /// `PreparedFrame`). `External` layers are skipped: the viewport's
    /// pixels come from Bevy, and the hole they leave is what the compositor
    /// fills. `imaging::record::Scene` implements `PaintSink` directly, so
    /// it can be the sink with no `Painter` wrapper needed.
    pub fn flatten(plan: &VisualLayerPlan, scale_factor: f64) -> imaging::record::Scene {
        let mut scene = imaging::record::Scene::new();
        let scale = Affine::scale(scale_factor);
        for layer in &plan.layers {
            if let VisualLayerKind::Scene(layer_scene) = &layer.kind {
                replay_transformed(layer_scene, &mut scene, scale * layer.transform);
            }
        }
        scene
    }
}

#[cfg(test)]
mod tests {
    use super::EditorUi;
    use imaging::Painter;
    use kurbo::{Affine, Rect};
    use masonry_core::app::{VisualLayer, VisualLayerKind, VisualLayerPlan};
    use masonry_core::core::{NewWidget, WidgetId};
    use masonry::widgets::Label;
    use peniko::Color;

    fn dummy_widget_id() -> WidgetId {
        NewWidget::new(Label::new("")).id()
    }

    fn filled_scene() -> imaging::record::Scene {
        let mut scene = imaging::record::Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            painter.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::WHITE);
        }
        scene
    }

    #[test]
    fn flatten_applies_scale_factor() {
        let layer_scene = filled_scene();
        let unscaled = VisualLayerPlan {
            layers: vec![VisualLayer {
                kind: VisualLayerKind::Scene(layer_scene.clone()),
                transform: Affine::IDENTITY,
                widget_id: dummy_widget_id(),
            }],
        };
        let pre_scaled = VisualLayerPlan {
            layers: vec![VisualLayer {
                kind: VisualLayerKind::Scene(layer_scene),
                transform: Affine::scale(2.0),
                widget_id: dummy_widget_id(),
            }],
        };
        assert_eq!(
            EditorUi::flatten(&unscaled, 2.0),
            EditorUi::flatten(&pre_scaled, 1.0),
        );
    }
}
