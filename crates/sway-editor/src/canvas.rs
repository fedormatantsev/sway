//! `GraphCanvas` -- the node-editor canvas, parent of the node boxes.
//!
//! This is the piece that actually exercises the spec's case for masonry:
//! each node is a real child widget (`node_box::NodeBox`) with its own
//! `WidgetId`, held retained in `nodes`/`positions`/`edges`, rather than an
//! immediate-mode `Vec` that gets painted from scratch every frame. Selection
//! and drag state (Task 7) will live on those child widgets, not be
//! re-derived here.
//!
//! Modeled on `masonry::widgets::flex::Flex` for the container shape
//! (`WidgetPod`, `register_children`, `run_layout`/`place_child` in
//! `layout`), and on `masonry_core::properties::box_shadow::BoxShadow` and
//! `masonry::widgets::canvas::Canvas` for how to reach `imaging::Painter`
//! from a widget's own `paint`.
//!
//! No traits, no generics over node types, no serialization (controller
//! dispatch ruling R1) -- this is throwaway code that M7 replaces once a real
//! graph model exists. Nodes are a `Vec`, edges are `Vec<(usize, usize)>`.

use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ActionCtx, ChildrenIds, ErasedAction, EventCtx, LayoutCtx, MeasureCtx, Modifiers,
    NewWidget, NoAction, PaintCtx, PointerButton, PointerButtonEvent, PointerEvent,
    PointerScrollEvent, PointerState, PointerUpdate, PropertiesMut, PropertiesRef, RegisterCtx,
    Update, UpdateCtx, Widget, WidgetId, WidgetMut, WidgetPod,
};
use masonry::dpi::PhysicalPosition;
use masonry::imaging::Painter;
use masonry_core::kurbo::{Affine, Axis, BezPath, Point, Rect, Size, Stroke, Vec2};
use masonry::layout::{LenReq, Length};
use peniko::Color;

use crate::external::ViewportPlaceholder;
use crate::node_box::{self, NodeBox, NodeBoxAction};

/// Converts a [`PointerState`]'s position to a window-space (logical pixels)
/// [`Point`]. Same helper as `node_box::window_point`, duplicated rather than
/// shared (R3, controller dispatch ruling) since it's three lines and the two
/// modules otherwise share no pointer-handling code.
fn window_point(state: &PointerState) -> Point {
    let p = state.logical_position();
    Point::new(p.x, p.y)
}

/// The external-mode viewport, held separately from the `NodeBox` children
/// because it is a different widget type (Task 5's `ViewportPlaceholder`,
/// not a `NodeBox`) -- see `GraphCanvas::with_viewport`.
struct ViewportSlot {
    pod: WidgetPod<ViewportPlaceholder>,
    pos: Point,
    size: Size,
}

/// The node-editor canvas: owns pan/zoom, lays out its `NodeBox` children
/// (plus, in this app, the Bevy viewport placeholder) at explicit
/// canvas-space positions, and paints the bezier edges between them.
pub struct GraphCanvas {
    nodes: Vec<WidgetPod<NodeBox>>,
    positions: Vec<Point>,
    edges: Vec<(usize, usize)>,
    viewport: Option<ViewportSlot>,
    /// Pan offset, applied to every node (and to edge painting) before zoom.
    pan: Vec2,
    /// Zoom factor. `layout` places every child at `Point::ZERO` (see its
    /// doc comment); the *real* position, pan and zoom are all folded into
    /// each child's own `set_transform` (Task 7), since `LayoutCtx` has no
    /// `set_transform` (fact independently verified for this task) and pan/
    /// zoom must therefore be pushed from an event/mutate/update context.
    zoom: f64,
    /// The currently selected node, by insertion index. `None` if nothing is
    /// selected.
    selected: Option<usize>,
    /// A drag-to-connect gesture in progress: the source node's index, and
    /// the live cursor position in canvas space (for painting the live
    /// bezier -- R1, controller dispatch ruling, still applies: this pending
    /// edge is never itself hit-tested, only the commit target is).
    pending_edge: Option<(usize, Point)>,
    /// A middle-drag pan in progress (brief step 4): the last-seen
    /// window-space (logical) pointer position. `None` when not panning.
    panning: Option<Point>,
}

// --- MARK: BUILDERS
impl GraphCanvas {
    /// Creates an empty canvas: no nodes, no edges, no viewport.
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            positions: Vec::new(),
            edges: Vec::new(),
            viewport: None,
            pan: Vec2::ZERO,
            zoom: 1.0,
            selected: None,
            pending_edge: None,
            panning: None,
        }
    }

    /// Adds a node box at the given canvas-space position.
    ///
    /// `id` must equal the number of nodes already added (0, 1, 2, ...):
    /// with no real graph model yet (R1), edges reference nodes purely by
    /// their insertion index, and this assertion catches a mismatched `id`
    /// immediately instead of silently drawing an edge to the wrong box.
    pub fn with_node(mut self, id: usize, pos: Point, label: &str) -> Self {
        assert_eq!(
            id,
            self.nodes.len(),
            "GraphCanvas::with_node: id {id} does not match insertion index {} -- \
             nodes must be added in order 0, 1, 2, ... (no graph model exists yet, R1)",
            self.nodes.len()
        );
        self.nodes.push(NodeBox::new(label.to_string()).prepare().to_pod());
        self.positions.push(pos);
        self
    }

    /// Adds an edge between two previously-added nodes, by insertion index.
    pub fn with_edge(mut self, from: usize, to: usize) -> Self {
        self.edges.push((from, to));
        self
    }

    /// Adds the external-mode viewport as a child, at the given canvas-space
    /// position and size. Not part of the brief's minimal interface (`new`
    /// / `with_node` / `with_edge`), but needed so `EditorUi` can still seat
    /// Task 5's `ViewportPlaceholder` in the tree (R5, controller dispatch
    /// ruling).
    pub fn with_viewport(mut self, viewport: NewWidget<ViewportPlaceholder>, pos: Point, size: Size) -> Self {
        self.viewport = Some(ViewportSlot {
            pod: viewport.to_pod(),
            pos,
            size,
        });
        self
    }
}

// --- MARK: IMPL WIDGET
impl Widget for GraphCanvas {
    type Action = NoAction;

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        for node in &mut self.nodes {
            ctx.register_child(node);
        }
        if let Some(slot) = &mut self.viewport {
            ctx.register_child(&mut slot.pod);
        }
    }

    fn measure(
        &mut self,
        _ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        _axis: Axis,
        len_req: LenReq,
        _cross_length: Option<Length>,
    ) -> Length {
        // `GraphCanvas` is only ever used as `EditorUi`'s root widget. Under
        // `WindowSizePolicy::User`, `run_layout_pass` resolves the root via
        // `SizeDef::fixed(window_size)` and `LayerStack::layout` forwards
        // that unconditionally, so this is unreachable in practice (see the
        // note on `EditorUi::root` construction) -- kept total regardless,
        // since `measure` has no default body in `Widget`.
        match len_req {
            LenReq::FitContent(space) => space,
            _ => Length::ZERO,
        }
    }

    /// Lays out every child at `Point::ZERO`.
    ///
    /// Every node's *real* position is now expressed entirely through its
    /// own `set_transform` (see the `pan`/`zoom` field docs): the composed
    /// transform for node `i` is
    /// `Affine::translate(pan) * Affine::scale(zoom) * Affine::translate(positions[i])`
    /// (see `child_transform`), applied via `WidgetMut::set_transform` from
    /// `update` (`Update::WidgetAdded`, for the initial position),
    /// `on_pointer_event` (pan/zoom, all nodes), `on_action`
    /// (drag, one node), or the `WidgetMut`-based `set_zoom`/`set_pan`/
    /// `set_selected` below -- never from here, because `LayoutCtx` has no
    /// `set_transform` (independently verified fact for this task): pan/zoom
    /// must be applied from an event/mutate/update context, never `layout`.
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        for node in self.nodes.iter_mut() {
            ctx.run_layout(node, node_box::SIZE);
            ctx.place_child(node, Point::ZERO);
        }
        if let Some(slot) = &mut self.viewport {
            ctx.run_layout(&mut slot.pod, slot.size);
            ctx.place_child(&mut slot.pod, slot.pos);
        }
        ctx.set_clip_path(size.to_rect());
    }

    /// Paints the bezier edges. Runs *before* children paint (R2, controller
    /// dispatch ruling), so the edges sit behind the node boxes -- this is
    /// the correct z-order for a node editor and is why this uses `paint`
    /// rather than `post_paint`.
    ///
    /// Unlike the `NodeBox` children, `GraphCanvas` itself carries no
    /// transform (it's the root), so edges are painted in the same window
    /// frame the children's transforms map into -- `to_visual` applies
    /// `pan`/`zoom` by hand to every point, exactly mirroring what each
    /// child's `set_transform` does for itself (brief step 4: "Edge painting
    /// applies the same affine to its own path").
    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let edge_brush = Color::from_rgb8(140, 140, 155);
        let half_height = Vec2::new(0.0, node_box::SIZE.height / 2.0);
        let right_edge = Vec2::new(node_box::SIZE.width, 0.0) + half_height;

        for &(from_idx, to_idx) in &self.edges {
            let (Some(&from_pos), Some(&to_pos)) =
                (self.positions.get(from_idx), self.positions.get(to_idx))
            else {
                continue;
            };

            // R4 (controller dispatch ruling): right edge of the source box
            // to the left edge of the target box, vertically centred.
            let from = self.to_visual(from_pos + right_edge);
            let to = self.to_visual(to_pos + half_height);
            self.paint_edge(painter, from, to, edge_brush);
        }

        // The live drag-to-connect edge, from the source node's connector to
        // the cursor (already tracked in canvas space -- see `on_action`).
        if let Some((src_idx, cursor_canvas)) = self.pending_edge
            && let Some(&src_pos) = self.positions.get(src_idx)
        {
            let from = self.to_visual(src_pos + right_edge);
            let to = self.to_visual(cursor_canvas);
            self.paint_edge(painter, from, to, Color::from_rgb8(220, 200, 120));
        }
    }

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        match event {
            PointerEvent::Down(PointerButtonEvent {
                button: Some(PointerButton::Auxiliary),
                state,
                ..
            }) => {
                // Middle-drag pans directly (brief step 4). `NodeBox::on_pointer_event`
                // only claims the primary button, so this reaches `GraphCanvas`
                // regardless of whether the press landed over a node or empty
                // canvas -- panning isn't node-specific.
                ctx.capture_pointer();
                self.panning = Some(window_point(state));
                ctx.set_handled();
            }
            PointerEvent::Down(..) => {
                // masonry hit-tests children before the parent (deepest hit
                // wins -- see `find_widget_under_pointer`), and every
                // `NodeBox::on_pointer_event` marks its own `Down` handled
                // (for the primary button; see the `Auxiliary` arm above for
                // the middle button). So a `Down` reaching *this* widget's
                // own handler for the primary button means the press landed
                // outside every node: a background click.
                self.clear_selection(ctx);
                ctx.set_handled();
            }
            PointerEvent::Move(PointerUpdate { current, .. })
                if ctx.is_active() && self.panning.is_some() =>
            {
                let window = window_point(current);
                if let Some(anchor) = &mut self.panning {
                    // Unlike node dragging (`delta / zoom`, canvas space),
                    // panning moves the viewport itself: the raw window-space
                    // delta is exactly how far the whole canvas should shift.
                    let delta = window - *anchor;
                    *anchor = window;
                    self.pan += delta;
                }
                self.retransform_all_from_event(ctx);
                ctx.set_handled();
            }
            PointerEvent::Up(..) | PointerEvent::Cancel(..) => {
                self.panning = None;
            }
            PointerEvent::Scroll(PointerScrollEvent { delta, state, .. }) => {
                let pixels = delta.to_pixel_delta(
                    PhysicalPosition { x: 32.0, y: 32.0 },
                    PhysicalPosition { x: 800.0, y: 800.0 },
                );
                if state.modifiers.contains(Modifiers::CONTROL) {
                    // Zoom about the cursor (brief step 4's formula, R1
                    // controller dispatch ruling doesn't apply here --
                    // that's about edge hit-testing, not this).
                    let logical = state.logical_position();
                    let cursor = Vec2::new(logical.x, logical.y);
                    let old_zoom = self.zoom;
                    let new_zoom = (old_zoom * (1.0 - pixels.y * 0.002)).clamp(0.1, 8.0);
                    self.pan = cursor - (cursor - self.pan) * (new_zoom / old_zoom);
                    self.zoom = new_zoom;
                } else {
                    self.pan -= Vec2::new(pixels.x, pixels.y);
                }
                self.retransform_all_from_event(ctx);
                ctx.set_handled();
            }
            _ => {}
        }
    }

    fn on_action(
        &mut self,
        ctx: &mut ActionCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        action: &ErasedAction,
        source: WidgetId,
    ) {
        let Some(idx) = self.nodes.iter().position(|n| n.id() == source) else {
            return;
        };
        let Some(&action) = action.downcast_ref::<NodeBoxAction>() else {
            return;
        };
        match action {
            NodeBoxAction::Selected => {
                self.select_from_action(ctx, idx);
            }
            NodeBoxAction::DraggedBy(delta) => {
                self.positions[idx] += delta / self.zoom;
                self.retransform_one_from_action(ctx, idx);
                ctx.request_paint_only();
            }
            NodeBoxAction::ConnectStart => {
                self.pending_edge = Some((idx, self.positions[idx]));
                ctx.request_paint_only();
            }
            NodeBoxAction::ConnectMove(window_pos) => {
                let canvas_pos = self.window_to_canvas(window_pos);
                if let Some(pending) = &mut self.pending_edge {
                    pending.1 = canvas_pos;
                }
                ctx.request_paint_only();
            }
            NodeBoxAction::ConnectEnd(window_pos) => {
                if let Some((src, _)) = self.pending_edge.take() {
                    let canvas_pos = self.window_to_canvas(window_pos);
                    if let Some(target) = self.node_at_canvas_point(canvas_pos)
                        && target != src
                    {
                        self.edges.push((src, target));
                    }
                }
                ctx.request_paint_only();
            }
        }
        ctx.set_handled();
    }

    /// Applies each node's initial pan/zoom/position transform once it's
    /// added to the tree (same idiom as
    /// `masonry::widgets::pagination::Pagination::update`'s
    /// `Update::WidgetAdded` handler).
    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if let Update::WidgetAdded = event {
            for idx in 0..self.nodes.len() {
                let transform = self.child_transform(idx);
                ctx.mutate_child_later(&mut self.nodes[idx], move |mut node: WidgetMut<'_, NodeBox>| {
                    node.set_transform(transform);
                });
            }
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::GenericContainer
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, _node: &mut Node) {}

    fn children_ids(&self) -> ChildrenIds {
        let mut ids: ChildrenIds = self.nodes.iter().map(|n| n.id()).collect();
        if let Some(slot) = &self.viewport {
            ids.push(slot.pod.id());
        }
        ids
    }
}

// --- MARK: WIDGETMUT
//
// The following are called through `WidgetMut` -- from a driver holding a
// `RenderRoot` (e.g. via `edit_root_widget`), or, as here, from tests. They
// mirror the same operations `on_pointer_event`/`on_action` perform in
// response to real input (see the HELPERS block below), but go through
// `MutateCtx::get_mut` instead of `EventCtx`/`ActionCtx::mutate_child_later`,
// because that's the only child-`WidgetMut` access `WidgetMut<GraphCanvas>`'s
// own context (`MutateCtx`) exposes -- same idiom as
// `masonry::widgets::selector::Selector::select_option`/`child_mut`.
impl GraphCanvas {
    /// Sets the zoom factor and re-applies pan/zoom/position to every node.
    pub fn set_zoom(this: &mut WidgetMut<'_, Self>, zoom: f64) {
        this.widget.zoom = zoom;
        Self::retransform_via_mutate_ctx(this);
    }

    /// Sets the pan offset and re-applies pan/zoom/position to every node.
    pub fn set_pan(this: &mut WidgetMut<'_, Self>, pan: Vec2) {
        this.widget.pan = pan;
        Self::retransform_via_mutate_ctx(this);
    }

    fn retransform_via_mutate_ctx(this: &mut WidgetMut<'_, Self>) {
        for idx in 0..this.widget.nodes.len() {
            let transform = this.widget.child_transform(idx);
            let mut child = this.ctx.get_mut(&mut this.widget.nodes[idx]);
            child.set_transform(transform);
        }
    }

    /// Sets which node is selected (by insertion index), updating both the
    /// previously- and newly-selected `NodeBox`'s own `selected` flag.
    pub fn set_selected(this: &mut WidgetMut<'_, Self>, selected: Option<usize>) {
        let previous = this.widget.selected;
        if previous == selected {
            return;
        }
        this.widget.selected = selected;
        if let Some(idx) = previous {
            let mut child = this.ctx.get_mut(&mut this.widget.nodes[idx]);
            NodeBox::set_selected(&mut child, false);
        }
        if let Some(idx) = selected {
            let mut child = this.ctx.get_mut(&mut this.widget.nodes[idx]);
            NodeBox::set_selected(&mut child, true);
        }
    }

    /// Returns the currently selected node's insertion index, if any.
    pub fn selected_node(&self) -> Option<usize> {
        self.selected
    }

    /// Returns the current pan offset. Read-only test/inspection accessor,
    /// mirroring `selected_node` -- panning itself is driven by
    /// `on_pointer_event`'s middle-drag handling or `set_pan` above, never
    /// by writing this directly.
    pub fn pan(&self) -> Vec2 {
        self.pan
    }
}

// --- MARK: HELPERS
//
// Pure geometry helpers (no context needed) plus the `EventCtx`/`ActionCtx`
// versions of the `WidgetMut` operations above, used by `on_pointer_event`/
// `on_action` -- real pointer/action dispatch can't obtain a `WidgetMut` for
// a child directly (`get_mut` only exists on `MutateCtx`), so these go
// through `mutate_child_later` instead, which defers the callback to the
// mutate pass that runs immediately after within the same
// `RenderRoot::handle_pointer_event` call (see the task 7 report for the
// trace through `masonry_core::passes` that establishes this timing).
impl GraphCanvas {
    /// The affine each node's own `set_transform` is given: pan and zoom,
    /// then this node's canvas-space position. `layout` places every child
    /// at `Point::ZERO`, so this is the *entire* mapping from a node's own
    /// local (0,0)-(160,72) box to the window.
    fn child_transform(&self, idx: usize) -> Affine {
        Affine::translate(self.pan) * Affine::scale(self.zoom) * Affine::translate(self.positions[idx].to_vec2())
    }

    /// Converts a window-space (logical) point to canvas space, inverting
    /// `child_transform`'s pan/zoom (but not any per-node position -- this
    /// is "where in the infinite canvas is the cursor", independent of any
    /// particular node).
    fn window_to_canvas(&self, p: Point) -> Point {
        ((p.to_vec2() - self.pan) / self.zoom).to_point()
    }

    /// Finds the node (by insertion index) whose canvas-space border box
    /// contains the given canvas-space point, if any. Used only for
    /// drag-to-connect's release target -- R1 (controller dispatch ruling)
    /// still holds: edges themselves are never hit-tested, only nodes are.
    fn node_at_canvas_point(&self, p: Point) -> Option<usize> {
        self.positions
            .iter()
            .position(|&pos| Rect::from_origin_size(pos, node_box::SIZE).contains(p))
    }

    fn paint_edge(&self, painter: &mut Painter<'_>, from: Point, to: Point, brush: Color) {
        let dx = ((to.x - from.x) * 0.5).abs().max(30.0);
        let mut path = BezPath::new();
        path.move_to(from);
        path.curve_to(Point::new(from.x + dx, from.y), Point::new(to.x - dx, to.y), to);
        painter.stroke(&path, &Stroke::new(2.0), brush).draw();
    }

    /// Maps a canvas-space point to the window frame `GraphCanvas::paint`
    /// runs in (the canvas itself carries no transform -- see `paint`'s doc
    /// comment).
    fn to_visual(&self, p: Point) -> Point {
        (p.to_vec2() * self.zoom + self.pan).to_point()
    }

    /// `EventCtx` version of clearing the selection (background click).
    fn clear_selection(&mut self, ctx: &mut EventCtx<'_>) {
        if let Some(idx) = self.selected.take() {
            ctx.mutate_child_later(&mut self.nodes[idx], |mut node: WidgetMut<'_, NodeBox>| {
                NodeBox::set_selected(&mut node, false);
            });
        }
    }

    /// `ActionCtx` version of selecting a node (a `NodeBox` reported
    /// `NodeBoxAction::Selected`).
    fn select_from_action(&mut self, ctx: &mut ActionCtx<'_>, idx: usize) {
        let previous = self.selected;
        if previous == Some(idx) {
            return;
        }
        self.selected = Some(idx);
        if let Some(prev_idx) = previous {
            ctx.mutate_child_later(&mut self.nodes[prev_idx], |mut node: WidgetMut<'_, NodeBox>| {
                NodeBox::set_selected(&mut node, false);
            });
        }
        ctx.mutate_child_later(&mut self.nodes[idx], |mut node: WidgetMut<'_, NodeBox>| {
            NodeBox::set_selected(&mut node, true);
        });
    }

    /// `EventCtx` version of re-applying pan/zoom/position to every node
    /// (scroll-driven pan/zoom affects all of them at once).
    fn retransform_all_from_event(&mut self, ctx: &mut EventCtx<'_>) {
        for idx in 0..self.nodes.len() {
            let transform = self.child_transform(idx);
            ctx.mutate_child_later(&mut self.nodes[idx], move |mut node: WidgetMut<'_, NodeBox>| {
                node.set_transform(transform);
            });
        }
        ctx.request_paint_only();
    }

    /// `ActionCtx` version of re-applying pan/zoom/position to a single node
    /// (a drag only moves the one node being dragged).
    fn retransform_one_from_action(&mut self, ctx: &mut ActionCtx<'_>, idx: usize) {
        let transform = self.child_transform(idx);
        ctx.mutate_child_later(&mut self.nodes[idx], move |mut node: WidgetMut<'_, NodeBox>| {
            node.set_transform(transform);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::GraphCanvas;
    use masonry::core::{DefaultProperties, PointerButton, Widget};
    use masonry_core::kurbo::{Point, Vec2};
    use masonry_testing::TestHarness;

    /// The claim spec §2.8 makes for masonry, reduced to an assertion.
    ///
    /// A node sits at canvas-space (100, 100). The canvas is zoomed 2x, so it
    /// occupies window space around (200, 200). A press at (210, 210) must
    /// reach *that node's* widget -- not the canvas, not a neighbour. If
    /// masonry's `window_transform` inverse did not drive hit-testing, this
    /// press would land on whatever is at unscaled (210, 210) instead, and a
    /// node editor built on it would be subtly, unfixably wrong under zoom.
    ///
    /// Deviations from the brief's literal harness calls (all required to
    /// compile against the pinned rev's real API -- see the task 7 report):
    /// - `TestHarness::create` takes `(DefaultProperties, NewWidget<W>)`, not
    ///   just the root widget; `GraphCanvas` doesn't use style properties, so
    ///   `DefaultProperties::default()` (empty) is enough.
    /// - `harness.root_widget()` already returns `WidgetRef<'_, GraphCanvas>`
    ///   (it's generic over the harness's own `W`), so the brief's extra
    ///   `.downcast::<GraphCanvas>()` doesn't type-check -- it would return
    ///   `Option<WidgetRef<GraphCanvas>>` -- and is dropped.
    /// - `mouse_button_press` takes `Option<PointerButton>`, not `PointerButton`.
    #[test]
    fn press_under_zoom_reaches_the_scaled_node() {
        let canvas = GraphCanvas::new()
            .with_node(0, Point::new(100.0, 100.0), "a")
            .with_node(1, Point::new(400.0, 100.0), "b");

        let mut harness = TestHarness::create(DefaultProperties::default(), canvas.prepare());
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::set_zoom(&mut canvas, 2.0);
        });

        harness.mouse_move(Point::new(210.0, 210.0));
        harness.mouse_button_press(Some(PointerButton::Primary));

        let selected = harness.root_widget().selected_node();
        assert_eq!(selected, Some(0), "the press should have selected the node at canvas (100,100)");
    }

    #[test]
    fn press_outside_any_node_clears_selection() {
        let canvas = GraphCanvas::new().with_node(0, Point::new(100.0, 100.0), "a");
        let mut harness = TestHarness::create(DefaultProperties::default(), canvas.prepare());

        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::set_selected(&mut canvas, Some(0));
        });
        harness.mouse_move(Point::new(20.0, 20.0));
        harness.mouse_button_press(Some(PointerButton::Primary));

        assert_eq!(harness.root_widget().selected_node(), None);
    }

    /// Fix round 1: brief step 4's "Middle-drag ... pans directly", the
    /// finding from the review that flagged it as unimplemented despite the
    /// original report claiming pan was complete. Widget-level (not one of
    /// the two gate tests, which stay untouched): presses the middle
    /// button, drags, and checks `pan` moved by exactly the raw window-space
    /// delta -- unscaled, unlike node dragging's `delta / zoom`.
    #[test]
    fn middle_drag_pans_the_canvas_by_the_raw_delta() {
        let canvas = GraphCanvas::new().with_node(0, Point::new(100.0, 100.0), "a");
        let mut harness = TestHarness::create(DefaultProperties::default(), canvas.prepare());

        harness.mouse_move(Point::new(50.0, 50.0));
        harness.mouse_button_press(Some(PointerButton::Auxiliary));
        harness.mouse_move(Point::new(80.0, 65.0));
        harness.mouse_button_release(Some(PointerButton::Auxiliary));

        assert_eq!(harness.root_widget().pan(), Vec2::new(30.0, 15.0));
    }

    /// A middle-drag that starts *over* a node must still pan the canvas,
    /// not drag the node -- `NodeBox` only claims the primary button (see
    /// its `on_pointer_event`), so this exercises that the middle button
    /// really does bubble up instead of being swallowed by the node.
    #[test]
    fn middle_drag_over_a_node_pans_instead_of_dragging_it() {
        let canvas = GraphCanvas::new().with_node(0, Point::new(100.0, 100.0), "a");
        let mut harness = TestHarness::create(DefaultProperties::default(), canvas.prepare());

        // (150, 130) is inside node 0's unscaled border box (100,100)-(260,172).
        harness.mouse_move(Point::new(150.0, 130.0));
        harness.mouse_button_press(Some(PointerButton::Auxiliary));
        harness.mouse_move(Point::new(170.0, 150.0));
        harness.mouse_button_release(Some(PointerButton::Auxiliary));

        assert_eq!(harness.root_widget().pan(), Vec2::new(20.0, 20.0));
        assert_eq!(harness.root_widget().selected_node(), None);
    }
}
