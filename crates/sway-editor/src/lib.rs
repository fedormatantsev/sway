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
pub mod inspector;
pub mod node_box;
pub mod palette;
pub mod scene_tree;
pub mod snapshot;
pub mod transport_bar;

#[cfg(test)]
mod test_graph;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use bevy_ecs::entity::Entity;
use crossbeam_channel::Sender;
use imaging::record::replay_transformed;
use masonry_core::app::{
    RenderRoot, RenderRootOptions, RenderRootSignal, VisualLayerKind, VisualLayerPlan,
    WindowSizePolicy,
};
use masonry_core::core::{
    CursorIcon, NewWidget, TextEvent, Widget, WidgetTag, WindowEvent as MasonryWindowEvent,
};
use masonry::kurbo::{Affine, Rect};
use masonry::layout::AsUnit;
use masonry::widgets::{Portal, Split};
use masonry_core::kurbo::Axis;
use sway_graph::EditorCommand;
use ui_events_winit::{WindowEventReducer, WindowEventTranslation};
use winit::dpi::PhysicalSize;

use crate::canvas::GraphCanvas;
use crate::external::ViewportPlaceholder;
use crate::inspector::Inspector;
use crate::scene_tree::SceneTree;
use crate::snapshot::{NodeId, WorldSnapshot};
use crate::transport_bar::{TRANSPORT_BAR_HEIGHT, TransportBar};

/// Reaches the hierarchy pane from `EditorUi::apply_snapshot`.
pub const SCENE_TREE_TAG: WidgetTag<SceneTree> = WidgetTag::named("sway-scene-tree");
/// Reaches the inspector pane from `EditorUi::apply_snapshot`.
pub const INSPECTOR_TAG: WidgetTag<Inspector> = WidgetTag::named("sway-inspector");
/// Reaches the graph pane from `EditorUi::apply_snapshot`.
pub const GRAPH_CANVAS_TAG: WidgetTag<GraphCanvas> = WidgetTag::named("sway-graph-canvas");
/// Reaches the viewport placeholder from `EditorUi::viewport_rect`.
///
/// Needed because `VisualLayerKind::External`'s reported `transform` is *not*
/// this widget's accumulated window transform -- see `EditorUi::viewport_rect`'s
/// doc comment for why that reading (the naive one, and the one this crate
/// shipped with through Task 7) is wrong for any `External` widget nested
/// under an offsetting ancestor, which the graph canvas's own `Split` is.
pub const VIEWPORT_TAG: WidgetTag<ViewportPlaceholder> = WidgetTag::named("sway-viewport");
/// Reaches the transport readout from `EditorUi::apply_snapshot`.
pub const TRANSPORT_BAR_TAG: WidgetTag<TransportBar> = WidgetTag::named("sway-transport-bar");

/// A file operation the shell performs, asked for by the toolbar.
///
/// Lives here rather than in `sway-document` because it is a UI intent: the
/// editor asks for a file to be opened without knowing what parsing one means.
/// It carries no path -- see the deviation note on this task: only `sway-app`
/// owns `rfd`, so a path does not exist until the shell has run a dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileRequest {
    Open,
    Save,
    SaveAs,
}

/// Builds the root widget: a transport strip above four panes.
///
/// ```text
/// +-------------------------------------------+
/// |              transport bar                 |
/// +--------+------------------------------------+
/// | SCENE  |      bevy viewport                 |
/// | v root |                                    |
/// |  v rig +------------------------------------+
/// +--------+  graph canvas (pan/zoom)           |
/// |inspect |                                    |
/// +--------+------------------------------------+
/// ```
///
/// The Bevy viewport is a sibling of the graph canvas now, not a child of it
/// at a hardcoded rect. Its window-space rect comes from `EditorUi::viewport_rect`,
/// which reads it directly off the tagged widget's own state -- see that
/// method's doc comment for why.
///
/// All five content widgets carry a `WidgetTag` so `apply_snapshot` and
/// `viewport_rect` can reach them typed, without downcasting through the
/// `Split`s.
///
/// `commands` is handed to the two panes that write: the inspector edits
/// fields, the canvas creates, deletes, moves and rewires. The tree and the
/// transport bar are read-only and do not get it.
fn graph_root(commands: Sender<EditorCommand>) -> NewWidget<dyn Widget> {
    let tree = Portal::new(SceneTree::new().prepare().with_tag(SCENE_TREE_TAG))
        .constrain_horizontal(true)
        .prepare();

    let inspector = Portal::new(Inspector::new(commands.clone()).prepare().with_tag(INSPECTOR_TAG))
        .constrain_horizontal(true)
        .prepare();

    let left = Split::new(tree, inspector)
        .split_axis(Axis::Vertical)
        .split_fraction(0.6)
        .draggable(true)
        .solid_bar(true)
        .prepare();

    let viewport = ViewportPlaceholder::new().prepare().with_tag(VIEWPORT_TAG);
    let canvas = GraphCanvas::new(commands).prepare().with_tag(GRAPH_CANVAS_TAG);

    let right = Split::new(viewport, canvas)
        .split_axis(Axis::Vertical)
        .split_fraction(0.55)
        .draggable(true)
        .solid_bar(true)
        .prepare();

    let panes = Split::new(left, right)
        .split_axis(Axis::Horizontal)
        .split_point_from_start(260.0.px())
        .draggable(true)
        .solid_bar(true)
        .prepare();

    let bar = TransportBar::new().prepare().with_tag(TRANSPORT_BAR_TAG);

    Split::new(bar, panes)
        .split_axis(Axis::Vertical)
        .split_point_from_start(TRANSPORT_BAR_HEIGHT.px())
        .draggable(false)
        .solid_bar(true)
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
    /// The `NodeId` behind each entity, from the most recent snapshot.
    /// Populated by `apply_snapshot`; used by `sync_selection` to translate a
    /// tree-row selection (an `Entity`) into a canvas selection (a `NodeId`).
    node_ids: HashMap<Entity, NodeId>,
    /// Masonry emits signals while it holds `RenderRoot` borrowed, and
    /// servicing a layer signal needs `&mut RenderRoot` -- so they are
    /// collected here and drained afterwards, exactly as `masonry_winit` does.
    signals: Rc<RefCell<Vec<RenderRootSignal>>>,
    /// The most recent cursor request, for the shell to apply to the window.
    cursor: Option<CursorIcon>,
    /// Handed to `graph_root`'s write-capable children at construction time;
    /// kept here too so a future caller doesn't have to thread its own copy
    /// through `EditorUi`. Unused as a read from this struct until then.
    #[allow(dead_code)]
    commands: Sender<EditorCommand>,
}

impl EditorUi {
    pub fn new(size: PhysicalSize<u32>, scale_factor: f64, commands: Sender<EditorCommand>) -> Self {
        let signals: Rc<RefCell<Vec<RenderRootSignal>>> = Rc::new(RefCell::new(Vec::new()));
        let sink_signals = signals.clone();
        let root = RenderRoot::new(
            graph_root(commands.clone()),
            move |signal: RenderRootSignal| sink_signals.borrow_mut().push(signal),
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
            node_ids: HashMap::new(),
            signals,
            cursor: None,
            commands,
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
        self.drain_signals();
    }

    /// Services everything masonry asked the host for since the last call.
    ///
    /// Layers are the load-bearing case: `ctx.create_layer` only *emits*
    /// `NewLayer`, and a popup does not exist until the host calls back into
    /// `RenderRoot`. Signals this editor has no use for (IME, clipboard,
    /// window geometry, `Exit`) are dropped deliberately -- the shell owns
    /// the window and this editor has one, fixed, non-closable pane layout.
    fn drain_signals(&mut self) {
        let drained: Vec<RenderRootSignal> = std::mem::take(&mut *self.signals.borrow_mut());
        for signal in drained {
            match signal {
                RenderRootSignal::NewLayer(_layer_type, root, pos) => {
                    self.root.add_layer(root, pos);
                }
                RenderRootSignal::RemoveLayer(root_id) => {
                    self.root.remove_layer(root_id);
                }
                RenderRootSignal::RepositionLayer(root_id, pos) => {
                    self.root.reposition_layer(root_id, pos);
                }
                RenderRootSignal::SetCursor(icon) => self.cursor = Some(icon),
                _ => {}
            }
        }
    }

    /// The pending cursor request, if any. Cleared by reading it.
    pub fn take_cursor(&mut self) -> Option<CursorIcon> {
        self.cursor.take()
    }

    /// What the toolbar has asked the shell to do since the last call.
    pub fn take_file_requests(&mut self) -> Vec<FileRequest> {
        self.root.edit_widget_with_tag(TRANSPORT_BAR_TAG, |mut bar| {
            TransportBar::take_file_requests(&mut bar)
        })
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

    /// Pushes one frame's world snapshot into both content panes.
    ///
    /// Called by the host immediately before [`redraw`](Self::redraw). Each
    /// pane decides for itself whether the snapshot actually changed anything
    /// -- `SceneTree` compares its row signature, `GraphCanvas` reconciles by
    /// `NodeId` -- so calling this every frame is cheap in the steady state.
    pub fn apply_snapshot(&mut self, snap: &WorldSnapshot) {
        self.node_ids = snap
            .nodes
            .iter()
            .map(|node| (node.entity, node.id))
            .collect();

        self.root.edit_widget_with_tag(SCENE_TREE_TAG, |mut tree| {
            SceneTree::apply_snapshot(&mut tree, snap);
        });
        self.root.edit_widget_with_tag(GRAPH_CANVAS_TAG, |mut canvas| {
            GraphCanvas::apply_snapshot(&mut canvas, snap);
        });
        self.root.edit_widget_with_tag(TRANSPORT_BAR_TAG, |mut bar| {
            TransportBar::apply_snapshot(&mut bar, snap);
        });
        self.root.edit_widget_with_tag(INSPECTOR_TAG, |mut inspector| {
            Inspector::apply_snapshot(&mut inspector, snap);
        });
    }

    /// Mirrors selection between the two panes.
    ///
    /// Whichever pane changed since the last call wins; if both changed, the
    /// canvas does, arbitrarily but deterministically. `NodeId` is the shared
    /// key, and a tree row that is not a graph node (a Bevy internal, an edge
    /// entity) selects within the tree and highlights nothing in the canvas.
    pub fn sync_selection(&mut self) {
        let canvas_selection = self
            .root
            .edit_widget_with_tag(GRAPH_CANVAS_TAG, |canvas| {
                canvas.widget.selected_node().and_then(|id| {
                    canvas.widget.entity_of(id).map(|entity| (id, entity))
                })
            });
        let tree_selection = self
            .root
            .edit_widget_with_tag(SCENE_TREE_TAG, |tree| tree.widget.selected());

        match (canvas_selection, tree_selection) {
            (Some((_, entity)), tree) if tree != Some(entity) => {
                self.root.edit_widget_with_tag(SCENE_TREE_TAG, |mut tree| {
                    SceneTree::set_selected(&mut tree, Some(entity));
                });
            }
            (None, Some(entity)) => {
                let node_id = self.last_snapshot_node_id(entity);
                self.root.edit_widget_with_tag(GRAPH_CANVAS_TAG, |mut canvas| {
                    GraphCanvas::set_selected(&mut canvas, node_id);
                });
            }
            _ => {}
        }
    }

    /// The `NodeId` for an entity, from the most recent snapshot. `None` for
    /// a row that is not a graph node.
    fn last_snapshot_node_id(&self, entity: Entity) -> Option<NodeId> {
        self.node_ids.get(&entity).copied()
    }

    /// The entity the panes currently agree is selected.
    ///
    /// `sync_selection` keeps the tree and the canvas in step, so the tree's
    /// answer is the shared one.
    pub fn selected_entity(&mut self) -> Option<Entity> {
        self.root
            .edit_widget_with_tag(SCENE_TREE_TAG, |tree| tree.widget.selected())
    }

    /// The Bevy viewport's current window-space (logical pixel) rectangle, or
    /// `None` if the widget isn't in the tree.
    ///
    /// This deliberately does *not* go through `VisualLayerKind::External`'s
    /// `bounds`/`transform` (what `external::viewport_rect` used to read, and
    /// what `Widget::paint`'s doc comment for `PaintLayerMode::External`
    /// implies is the intended path). At the masonry rev this crate is
    /// pinned to, `push_external_layer` (`masonry_core::passes::paint`)
    /// pairs the widget's *local* border-box with `LayerCollector::transform`
    /// -- which is seeded once per *paint layer* (`RenderRoot`'s own overlay
    /// stack, e.g. popups) and never updated while walking down through
    /// ordinary `Inline` ancestors. For the base layer that seed is
    /// `Affine::IDENTITY`, so any `External` widget nested under an
    /// offsetting ancestor -- our `Split`s, which place the right pane and
    /// the viewport within it by translation -- gets reported at the wrong
    /// window position. Verified by running the three-pane editor and
    /// screenshotting it: the Bevy content rendered in a rect shifted by
    /// roughly the tree pane's width, overlapping it, rather than sitting in
    /// the black hole `ViewportPlaceholder` actually painted.
    ///
    /// `QueryCtx::bounding_box` (reachable off any tagged widget through
    /// `RenderRoot::get_widget_with_tag`) is unaffected: it's computed by the
    /// compose pass from the widget's own accumulated `window_transform`,
    /// which *does* include every ancestor's placement. `ViewportPlaceholder`
    /// has no children and clips to its full content box, so its bounding
    /// box is exactly its border-box mapped into window space -- precisely
    /// the rect the compositor needs.
    pub fn viewport_rect(&self) -> Option<Rect> {
        self.root
            .get_widget_with_tag(VIEWPORT_TAG)
            .map(|widget| widget.ctx().bounding_box())
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
        self.drain_signals();
        self.sync_selection();

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
    use crate::canvas::GraphCanvas;
    use crate::scene_tree::SceneTree;
    use crate::snapshot::{NodeId, NodeView, TreeGroup, TreeRow, WorldSnapshot};
    use bevy_ecs::entity::Entity;
    use imaging::Painter;
    use kurbo::{Affine, Point as KurboPoint, Rect};
    use masonry_core::app::{RenderRootSignal, VisualLayer, VisualLayerKind, VisualLayerPlan};
    use masonry_core::core::{NewWidget, WidgetId};
    use masonry::widgets::Label;
    use peniko::Color;
    use winit::dpi::PhysicalSize;

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

    fn one_node_snapshot() -> WorldSnapshot {
        let entity = Entity::from_raw_u32(3).expect("valid entity id");
        WorldSnapshot {
            tree: vec![TreeRow {
                entity,
                group: TreeGroup::Graph,
                depth: 0,
                label: "LFO #1".to_string(),
                node_id: Some(NodeId(1)),
            }],
            nodes: vec![NodeView {
                entity,
                id: NodeId(1),
                name: "LFO".to_string(),
                pos: Some(KurboPoint::new(10.0, 10.0)),
                inlets: Vec::new(),
                outlets: 0,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn selecting_a_node_box_highlights_its_tree_row() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut ui = EditorUi::new(PhysicalSize::new(800, 600), 1.0, tx);
        let snap = one_node_snapshot();
        ui.apply_snapshot(&snap);

        ui.root.edit_widget_with_tag(crate::GRAPH_CANVAS_TAG, |mut canvas| {
            GraphCanvas::set_selected(&mut canvas, Some(NodeId(1)));
        });
        ui.sync_selection();

        let selected = ui
            .root
            .edit_widget_with_tag(crate::SCENE_TREE_TAG, |tree| tree.widget.selected());
        assert_eq!(selected, Some(snap.nodes[0].entity));
    }

    #[test]
    fn selecting_a_graph_node_row_highlights_its_node_box() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut ui = EditorUi::new(PhysicalSize::new(800, 600), 1.0, tx);
        let snap = one_node_snapshot();
        ui.apply_snapshot(&snap);

        ui.root.edit_widget_with_tag(crate::SCENE_TREE_TAG, |mut tree| {
            SceneTree::set_selected(&mut tree, Some(snap.nodes[0].entity));
        });
        ui.sync_selection();

        let selected = ui
            .root
            .edit_widget_with_tag(crate::GRAPH_CANVAS_TAG, |canvas| canvas.widget.selected_node());
        assert_eq!(selected, Some(NodeId(1)));
    }

    /// Regression test for the bug fixed alongside Task 8: `viewport_rect`
    /// must read the viewport placeholder's own accumulated position, not
    /// `VisualLayerKind::External`'s reported transform (which masonry seeds
    /// once per *paint layer* and never updates while descending through
    /// ordinary `Inline` ancestors -- see `viewport_rect`'s doc comment). A
    /// regression to that reading would report the viewport at the window
    /// origin regardless of where the `Split`s actually placed it.
    #[test]
    fn viewport_rect_reflects_its_position_inside_nested_splits() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut ui = EditorUi::new(PhysicalSize::new(800, 600), 1.0, tx);
        // Settles layout/compose so the widget tree's geometry is current.
        ui.redraw();

        let rect = ui.viewport_rect().expect("the viewport placeholder is in the tree");

        // The tree pane is 260px wide (`graph_root`'s split_point_from_start),
        // so the viewport -- the right Split's top pane -- must start to its
        // right, not at the window origin.
        assert!(rect.x0 >= 260.0, "viewport rect {rect:?} must sit right of the tree pane");
        assert!(rect.width() > 0.0 && rect.height() > 0.0, "viewport rect {rect:?} must have real area");
        assert!(
            rect.y0 >= crate::transport_bar::TRANSPORT_BAR_HEIGHT,
            "viewport rect {rect:?} must sit below the transport strip"
        );
    }

    #[test]
    fn an_unchanged_selection_does_not_rebuild_the_inspector() {
        // Same discipline as SceneTree: a steady-state world costs one
        // comparison per frame.
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut ui = EditorUi::new(PhysicalSize::new(1200, 800), 1.0, tx);
        let snap = WorldSnapshot::default();
        ui.apply_snapshot(&snap);
        let first = ui
            .root
            .edit_widget_with_tag(crate::INSPECTOR_TAG, |i| i.widget.generation());
        ui.apply_snapshot(&snap);
        let second = ui
            .root
            .edit_widget_with_tag(crate::INSPECTOR_TAG, |i| i.widget.generation());

        assert_eq!(first, second);
    }

    #[test]
    fn a_new_layer_signal_puts_the_widget_in_the_tree() {
        // Before M6 the sink was a no-op, so no popup, tooltip or Selector
        // dropdown could appear at all: ctx.create_layer only *emits*
        // NewLayer, and the layer does not exist until the host calls back
        // into RenderRoot.
        use masonry_core::core::{LayerType, NewWidget};
        use masonry::widgets::Label;

        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut ui = EditorUi::new(PhysicalSize::new(800, 600), 1.0, tx);
        ui.redraw();

        let popup = NewWidget::new(Label::new("popup"));
        let popup_id = popup.id();
        assert!(!ui.root.has_widget(popup_id), "not in the tree before the signal");

        ui.signals.borrow_mut().push(RenderRootSignal::NewLayer(
            LayerType::Other,
            popup.erased(),
            KurboPoint::new(10.0, 10.0),
        ));
        ui.drain_signals();

        assert!(ui.root.has_widget(popup_id), "the layer signal was serviced");
    }

    #[test]
    fn a_remove_layer_signal_takes_the_widget_back_out() {
        use masonry_core::core::{LayerType, NewWidget};
        use masonry::widgets::Label;

        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut ui = EditorUi::new(PhysicalSize::new(800, 600), 1.0, tx);
        ui.redraw();

        let popup = NewWidget::new(Label::new("popup"));
        let popup_id = popup.id();
        ui.signals.borrow_mut().push(RenderRootSignal::NewLayer(
            LayerType::Other,
            popup.erased(),
            KurboPoint::new(10.0, 10.0),
        ));
        ui.drain_signals();

        ui.signals
            .borrow_mut()
            .push(RenderRootSignal::RemoveLayer(popup_id));
        ui.drain_signals();

        assert!(!ui.root.has_widget(popup_id));
    }

    #[test]
    fn a_set_cursor_signal_is_handed_to_the_shell_once() {
        // Drag-to-connect (Task 15) wants cursor feedback, and the shell owns
        // the window. Reading it clears it, so the shell does not re-apply the
        // same icon every frame.
        use masonry_core::core::CursorIcon;

        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut ui = EditorUi::new(PhysicalSize::new(800, 600), 1.0, tx);

        ui.signals
            .borrow_mut()
            .push(RenderRootSignal::SetCursor(CursorIcon::Crosshair));
        ui.drain_signals();

        assert_eq!(ui.take_cursor(), Some(CursorIcon::Crosshair));
        assert_eq!(ui.take_cursor(), None, "reading the request clears it");
    }
}
