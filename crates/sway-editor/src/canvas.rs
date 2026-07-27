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
    AccessCtx, ChildrenIds, LayoutCtx, MeasureCtx, NewWidget, NoAction, PaintCtx, PropertiesRef,
    RegisterCtx, Widget, WidgetPod,
};
use masonry::imaging::Painter;
use masonry_core::kurbo::{Axis, BezPath, Point, Size, Stroke, Vec2};
use masonry::layout::{LenReq, Length};
use peniko::Color;

use crate::external::ViewportPlaceholder;
use crate::node_box::{self, NodeBox};

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
    // Reserved for Task 7 (R3, controller dispatch ruling): pan/zoom
    // interaction is explicitly out of scope for Task 6. This field is not
    // read anywhere yet -- `layout` places children at raw canvas-space
    // positions, unscaled and untranslated, exactly as the brief requires,
    // so that the eventual pan/zoom implementation lands as a `set_transform`
    // on children (from outside the layout pass, e.g. in response to pointer
    // events) rather than as hand-rolled math here that would desync
    // painted pixels from masonry's own hit-testing.
    #[allow(dead_code)]
    zoom: f64,
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
            zoom: 1.0,
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

    /// Lays out every child at its stored canvas-space position.
    ///
    /// The pan/zoom transform is deliberately *not* applied here (R3): each
    /// child's position is placed exactly as stored. Applying pan/zoom by
    /// hand-shifting positions in this method would produce correct pixels
    /// while leaving masonry's hit-testing (which inverts `window_transform`,
    /// built from each widget's own `set_transform`) pointed at the wrong
    /// place -- the easiest way to accidentally prove nothing in this
    /// milestone. Task 7 will instead call `set_transform` on the children
    /// directly, outside this pass.
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        for (node, &pos) in self.nodes.iter_mut().zip(self.positions.iter()) {
            ctx.run_layout(node, node_box::SIZE);
            ctx.place_child(node, pos);
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
            let from = from_pos + right_edge;
            let to = to_pos + half_height;
            let dx = ((to.x - from.x) * 0.5).abs().max(30.0);

            let mut path = BezPath::new();
            path.move_to(from);
            path.curve_to(
                Point::new(from.x + dx, from.y),
                Point::new(to.x - dx, to.y),
                to,
            );
            painter.stroke(&path, &Stroke::new(2.0), edge_brush).draw();
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
