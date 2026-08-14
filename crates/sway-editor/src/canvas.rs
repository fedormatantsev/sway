//! `GraphCanvas` -- the node-editor canvas, parent of the node boxes.
//!
//! This is the piece that actually exercises the spec's case for masonry:
//! each node is a real child widget (`node_box::NodeBox`) with its own
//! `WidgetId`, retained across frames rather than rebuilt from scratch.
//! Driven entirely by [`WorldSnapshot`] through [`GraphCanvas::apply_snapshot`];
//! nodes are keyed by [`NodeId`], so a node that survives a snapshot keeps
//! its `WidgetId` -- and therefore its dragged position and its selection.
//! That identity model is what M7 needs.
//!
//! Modeled on `masonry::widgets::flex::Flex` for the container shape
//! (`WidgetPod`, `register_children`, `run_layout`/`place_child` in
//! `layout`), and on `masonry_core::properties::box_shadow::BoxShadow` and
//! `masonry::widgets::canvas::Canvas` for how to reach `imaging::Painter`
//! from a widget's own `paint`.

use std::collections::{HashMap, HashSet};

use bevy_ecs::entity::Entity;
use bevy_math::Vec2 as WorldVec2;
use crossbeam_channel::Sender;
use masonry::accesskit::{Node, Role};
use masonry::core::keyboard::{Key, NamedKey};
use masonry::core::{
    AccessCtx, ActionCtx, ChildrenIds, ErasedAction, EventCtx, LayerType, LayoutCtx, MeasureCtx,
    Modifiers, NewWidget, NoAction, PaintCtx, PointerButton, PointerButtonEvent, PointerEvent,
    PointerScrollEvent, PointerState, PointerUpdate, PropertiesMut, PropertiesRef, RegisterCtx,
    TextEvent, Widget, WidgetId, WidgetMut, WidgetPod,
};
use masonry::dpi::PhysicalPosition;
use masonry::imaging::Painter;
use masonry_core::kurbo::{Affine, Axis, BezPath, Point, Size, Stroke, Vec2};
use masonry::layout::{LenReq, Length};
use peniko::Color;
use sway_graph::EditorCommand;

use crate::node_box::{self, NodeBox, NodeBoxAction};
use crate::palette::{Palette, PaletteAction};
use crate::snapshot::{NodeId, WorldSnapshot};

/// Converts a [`PointerState`]'s position to a window-space (logical pixels)
/// [`Point`]. Same helper as `node_box::window_point`, duplicated rather than
/// shared (R3, controller dispatch ruling) since it's three lines and the two
/// modules otherwise share no pointer-handling code.
fn window_point(state: &PointerState) -> Point {
    let p = state.logical_position();
    Point::new(p.x, p.y)
}

/// Fallback grid for nodes with no authored `EditorPos` (design §5): column
/// `i / FALLBACK_ROWS`, row `i % FALLBACK_ROWS`, at the node box's own pitch.
/// Deterministic, so an unpositioned node is misplaced rather than invisible,
/// and two of them never land on top of each other.
const FALLBACK_ROWS: usize = 6;
const FALLBACK_PITCH: Size = Size::new(220.0, 120.0);

/// One node box and everything the canvas knows about it.
struct NodeSlot {
    pod: WidgetPod<NodeBox>,
    /// Canvas-space position. Seeded from the snapshot's `EditorPos` when the
    /// slot is created and owned by the widget thereafter -- see
    /// `apply_snapshot`.
    pos: Point,
    label: String,
    /// The world entity behind this node. A `NodeId` can outlive the entity
    /// it names across a recompile, so this is refreshed from the snapshot on
    /// every `apply_snapshot`, not just when the slot is created.
    entity: Entity,
    /// Mirrors the live `NodeBox`'s own socket counts, so `paint` (which
    /// cannot reach into a child widget -- masonry gives a parent no read
    /// access to a child's state from `PaintCtx`) can compute socket
    /// positions via `node_box::inlet_socket_local`/`outlet_socket_local`
    /// without one. Kept in sync by `apply_snapshot`, same reasoning as
    /// `label`.
    inlets: Vec<u16>,
    outlets: u16,
}

/// One painted edge, resolved to node keys rather than snapshot indices so it
/// survives a reordering.
struct EdgeSlot {
    from: NodeId,
    /// The source outlet's field ordinal. No matching `from_index`: an
    /// outlet can never be a `Vec` (design §12), so an outlet socket is
    /// always identified by field alone.
    from_field: u16,
    to: NodeId,
    to_field: u16,
    to_index: u16,
    wire: &'static str,
    /// The source port's current value, or `None` when this edge carries no
    /// readable activity (design §4).
    value: Option<f32>,
    /// Running observed range for this edge, used to normalise `value` into
    /// 0..1. Auto-ranging avoids a per-node-type table of expected ranges,
    /// which would be one more thing to keep in sync with the node set.
    range: Option<(f32, f32)>,
}

/// The node-editor canvas: owns pan/zoom, lays out its `NodeBox` children at
/// explicit canvas-space positions, and paints the edges between them.
///
/// Driven entirely by [`WorldSnapshot`] through [`GraphCanvas::apply_snapshot`].
/// Nodes are keyed by [`NodeId`], not by insertion index, so a node that
/// survives a snapshot keeps its `WidgetId` -- and therefore its dragged
/// position and its selection. That identity model is what M7 needs.
pub struct GraphCanvas {
    nodes: Vec<NodeId>,
    slots: HashMap<NodeId, NodeSlot>,
    edges: Vec<EdgeSlot>,
    pan: Vec2,
    zoom: f64,
    selected: Option<NodeId>,
    /// A middle-drag pan in progress (brief step 4): the last-seen
    /// window-space (logical) pointer position. `None` when not panning.
    panning: Option<Point>,
    /// Where edits go. The canvas produces data; `sway-graph` applies it.
    commands: Sender<EditorCommand>,
    /// Every authorable component name, from the last snapshot — what the
    /// palette is built from.
    palette: Vec<&'static str>,
    /// The open palette layer, if any, and the canvas-space position it was
    /// opened at (which is where a pick creates the node).
    palette_layer: Option<(WidgetId, Point)>,
}

// --- MARK: BUILDERS
impl GraphCanvas {
    /// Creates an empty canvas. Content arrives through `apply_snapshot`.
    pub fn new(commands: Sender<EditorCommand>) -> Self {
        Self {
            nodes: Vec::new(),
            slots: HashMap::new(),
            edges: Vec::new(),
            pan: Vec2::ZERO,
            zoom: 1.0,
            selected: None,
            panning: None,
            commands,
            palette: Vec::new(),
            palette_layer: None,
        }
    }
}

/// Design §5's fallback grid slot for the node at snapshot index `index`.
fn fallback_pos(index: usize) -> Point {
    Point::new(
        (index / FALLBACK_ROWS) as f64 * FALLBACK_PITCH.width,
        (index % FALLBACK_ROWS) as f64 * FALLBACK_PITCH.height,
    )
}

// --- MARK: IMPL WIDGET
impl Widget for GraphCanvas {
    type Action = NoAction;

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

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        for id in &self.nodes {
            if let Some(slot) = self.slots.get_mut(id) {
                ctx.register_child(&mut slot.pod);
            }
        }
    }

    /// Lays out every child at `Point::ZERO`.
    ///
    /// Every node's *real* position is now expressed entirely through its
    /// own `set_transform` (see the `pan`/`zoom` field docs): the composed
    /// transform for node `i` is
    /// `Affine::translate(pan) * Affine::scale(zoom) * Affine::translate(positions[i])`
    /// (see `child_transform`), applied via `WidgetMut::set_transform` from
    /// `apply_snapshot` (for the initial position), `on_pointer_event`
    /// (pan/zoom, all nodes), `on_action` (drag, one node), or the
    /// `WidgetMut`-based `set_zoom`/`set_pan`/`set_selected` below -- never
    /// from here, because `LayoutCtx` has no `set_transform` (independently
    /// verified fact for this task): pan/zoom must be applied from an
    /// event/mutate/update context, never `layout`.
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        for id in self.nodes.clone() {
            if let Some(slot) = self.slots.get_mut(&id) {
                ctx.run_layout(&mut slot.pod, node_box::SIZE);
                ctx.place_child(&mut slot.pod, Point::ZERO);
            }
        }
        ctx.set_clip_path(size.to_rect());
    }

    /// Paints the edges. Runs *before* children paint, so edges sit behind
    /// the node boxes -- the correct z-order for a node editor, and why this
    /// uses `paint` rather than `post_paint`.
    ///
    /// `GraphCanvas` itself carries no transform (it's the root of its pane),
    /// so edges are painted in the same window frame the children's
    /// transforms map into: `to_visual` applies `pan`/`zoom` by hand to every
    /// point, mirroring what each child's `set_transform` does for itself.
    ///
    /// Each endpoint is a socket, not the box's centre/edge midpoint --
    /// `node_box::outlet_socket_local`/`inlet_socket_local` place it from the
    /// edge's own `field`/`index`, using the `inlets`/`outlets` counts
    /// `apply_snapshot` mirrors onto `NodeSlot` (see that struct's doc
    /// comment for why `paint` can't read them off the live `NodeBox`
    /// instead).
    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        for edge in &self.edges {
            let (Some(from_slot), Some(to_slot)) =
                (self.slots.get(&edge.from), self.slots.get(&edge.to))
            else {
                continue;
            };
            let from_local =
                node_box::outlet_socket_local(from_slot.inlets.len() as u16, from_slot.outlets, edge.from_field);
            let to_local = node_box::inlet_socket_local(&to_slot.inlets, edge.to_field, edge.to_index);
            let from = self.to_visual(from_slot.pos + from_local.to_vec2());
            let to = self.to_visual(to_slot.pos + to_local.to_vec2());
            let (brush, width) = edge_style(edge);
            self.paint_edge(painter, from, to, brush, width);
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
            PointerEvent::Down(PointerButtonEvent {
                button: Some(PointerButton::Secondary),
                state,
                ..
            }) => {
                let local = ctx.local_position(state.position);
                let canvas_pos = self.to_canvas(local);
                let palette = NewWidget::new(Palette::new(self.palette.clone()));
                self.palette_layer = Some((palette.id(), canvas_pos));
                ctx.create_layer(LayerType::Other, palette, ctx.to_window(local));
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
                // Line/page policy is in logical CSS px; multiply by DPR so
                // `to_pixel_delta` yields physical, then convert back so pan/
                // zoom stay in logical window space. PixelDelta is already
                // physical and only needs `to_logical`.
                let scale = state.scale_factor.max(f64::EPSILON);
                let physical = delta.to_pixel_delta(
                    PhysicalPosition {
                        x: 32.0 * scale,
                        y: 32.0 * scale,
                    },
                    PhysicalPosition {
                        x: 800.0 * scale,
                        y: 800.0 * scale,
                    },
                );
                let logical = physical.to_logical(scale);
                let pixels = Vec2::new(logical.x, logical.y);
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
                    self.pan -= pixels;
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
        if let Some(PaletteAction::Picked(component)) = action.downcast_ref::<PaletteAction>() {
            // The position the palette was *opened* at, not where the pick
            // landed: the node belongs where the user pointed, not where the
            // list happened to place that row.
            if let Some((layer_id, pos)) = self.palette_layer.take() {
                let _ = self.commands.send(EditorCommand::Create {
                    component,
                    pos: WorldVec2::new(pos.x as f32, pos.y as f32),
                });
                ctx.remove_layer(layer_id);
            }
            ctx.set_handled();
            return;
        }
        let Some(id) = self
            .nodes
            .iter()
            .copied()
            .find(|id| self.slots.get(id).is_some_and(|slot| slot.pod.id() == source))
        else {
            return;
        };
        let Some(&action) = action.downcast_ref::<NodeBoxAction>() else {
            return;
        };
        match action {
            NodeBoxAction::Selected => self.select_from_action(ctx, id),
            NodeBoxAction::DraggedBy(delta) => {
                if let Some(slot) = self.slots.get_mut(&id) {
                    slot.pos += delta / self.zoom;
                }
                self.retransform_one_from_action(ctx, id);
                ctx.request_paint_only();
            }
            NodeBoxAction::DragEnded => {
                if let Some(slot) = self.slots.get(&id) {
                    let _ = self.commands.send(EditorCommand::MoveNode {
                        entity: slot.entity,
                        pos: WorldVec2::new(slot.pos.x as f32, slot.pos.y as f32),
                    });
                }
            }
        }
        ctx.set_handled();
    }

    fn on_text_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &TextEvent,
    ) {
        let TextEvent::Keyboard(key_event) = event else {
            return;
        };
        if !key_event.state.is_down() {
            return;
        }
        if matches!(
            key_event.key,
            Key::Named(NamedKey::Delete) | Key::Named(NamedKey::Backspace)
        ) {
            self.delete_selected();
            ctx.set_handled();
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::GenericContainer
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, _node: &mut Node) {}

    fn children_ids(&self) -> ChildrenIds {
        self.nodes
            .iter()
            .filter_map(|id| self.slots.get(id).map(|slot| slot.pod.id()))
            .collect()
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
    /// Reconciles the canvas against one frame's snapshot.
    ///
    /// Nodes present in both keep their `WidgetId`, their dragged position,
    /// and their selection; nodes only in the snapshot are created (seeded
    /// from `EditorPos`, or the fallback grid); nodes only in the canvas are
    /// removed. Edge geometry is rebuilt outright -- an edge has no identity
    /// worth preserving -- but each edge's observed value range is carried
    /// across by `(from, to, wire)` so auto-ranging does not restart.
    pub fn apply_snapshot(this: &mut WidgetMut<'_, Self>, snap: &WorldSnapshot) {
        this.widget.palette.clone_from(&snap.palette);

        let mut ranges: HashMap<(NodeId, NodeId, &'static str), (f32, f32)> = HashMap::new();
        for edge in &this.widget.edges {
            if let Some(range) = edge.range {
                ranges.insert((edge.from, edge.to, edge.wire), range);
            }
        }

        // Nodes: create, update, remove.
        let wanted: Vec<NodeId> = snap.nodes.iter().map(|node| node.id).collect();
        let stale: Vec<NodeId> = this
            .widget
            .nodes
            .iter()
            .copied()
            .filter(|id| !wanted.contains(id))
            .collect();
        for id in stale {
            if let Some(slot) = this.widget.slots.remove(&id) {
                this.ctx.remove_child(slot.pod);
            }
            if this.widget.selected == Some(id) {
                this.widget.selected = None;
            }
        }

        for (index, view) in snap.nodes.iter().enumerate() {
            // One socket per inlet: a wire inlet is scalar. `NodeSlot.inlets`
            // and `NodeBox` still speak in per-field slot counts, so this is
            // the one place the two representations meet.
            let inlet_counts: Vec<u16> = vec![1; view.inlets.len()];
            match this.widget.slots.get_mut(&view.id) {
                Some(slot) => {
                    if slot.label != view.name {
                        view.name.clone_into(&mut slot.label);
                        let mut child = this.ctx.get_mut(&mut slot.pod);
                        NodeBox::set_label(&mut child, &view.name);
                    }
                    // Socket counts change when a `Vec` inlet is resized
                    // (e.g. a `Group`'s `children`) -- not on every snapshot,
                    // so this is gated the same way `label` is.
                    if slot.inlets != inlet_counts || slot.outlets != view.outlets {
                        slot.inlets = inlet_counts.clone();
                        slot.outlets = view.outlets;
                        let mut child = this.ctx.get_mut(&mut slot.pod);
                        NodeBox::set_sockets(&mut child, inlet_counts.clone(), view.outlets);
                    }
                    // A `NodeId` can outlive the entity it names across a
                    // recompile, so this is refreshed every snapshot even
                    // when nothing else about the node changed.
                    slot.entity = view.entity;
                }
                None => {
                    // `EditorPos` seeds a *new* box only; an existing box owns
                    // its position from here on (design §5).
                    let pos = view.pos.unwrap_or_else(|| fallback_pos(index));
                    // Baked into the box itself and self-applied on
                    // `Update::WidgetAdded` -- a box this fresh isn't yet
                    // registered in masonry's arena, so `GraphCanvas` can't
                    // reach in and set its transform directly (see
                    // `NodeBox::initial_transform`'s doc comment).
                    let transform = Affine::translate(this.widget.pan)
                        * Affine::scale(this.widget.zoom)
                        * Affine::translate(pos.to_vec2());
                    let pod = NodeBox::new(view.name.clone())
                        .with_initial_transform(transform)
                        .with_sockets(inlet_counts.clone(), view.outlets)
                        .prepare()
                        .to_pod();
                    this.widget.slots.insert(
                        view.id,
                        NodeSlot {
                            pod,
                            pos,
                            label: view.name.clone(),
                            entity: view.entity,
                            inlets: inlet_counts,
                            outlets: view.outlets,
                        },
                    );
                    this.ctx.children_changed();
                }
            }
        }
        this.widget.nodes = wanted;

        // Edges: rebuilt outright, carrying observed ranges across.
        //
        // "Both endpoints exist" now means "both `NodeId`s are present in
        // this snapshot's node list" -- not, as before Task 8/9, "both
        // indices are in range of `snap.nodes`". An edge's `from`/`to` are
        // `NodeId`s addressed independently of list position (a compiled
        // node's `NodeId` can be, and often is, unrelated to where it lands
        // in `snap.nodes`), so the membership test has to be a lookup by
        // value, not a bounds check. The intent this preserves from before
        // is unchanged: an edge naming a node that didn't survive the
        // snapshot (despawned, or never compiled) is dropped rather than
        // drawn to nowhere.
        let live_nodes: HashSet<NodeId> = snap.nodes.iter().map(|node| node.id).collect();
        this.widget.edges = snap
            .edges
            .iter()
            .filter(|edge| live_nodes.contains(&edge.from) && live_nodes.contains(&edge.to))
            .map(|edge| {
                let mut range = ranges.get(&(edge.from, edge.to, edge.wire)).copied();
                if let Some(value) = edge.activity {
                    range = Some(match range {
                        Some((lo, hi)) => (lo.min(value), hi.max(value)),
                        None => (value, value),
                    });
                }
                EdgeSlot {
                    from: edge.from,
                    from_field: edge.from_field,
                    to: edge.to,
                    to_field: edge.to_field,
                    to_index: edge.to_index,
                    wire: edge.wire,
                    value: edge.activity,
                    range,
                }
            })
            .collect();

        this.ctx.request_render();
    }

    /// The `WidgetId` of a node's box, for tests and for M7's selection
    /// plumbing. `None` if the canvas has no such node.
    pub fn widget_id_of(&self, id: NodeId) -> Option<WidgetId> {
        self.slots.get(&id).map(|slot| slot.pod.id())
    }

    /// A node's current canvas-space position.
    pub fn position_of(&self, id: NodeId) -> Option<Point> {
        self.slots.get(&id).map(|slot| slot.pos)
    }

    /// The world entity behind a node, so a canvas selection can address the
    /// matching tree row.
    pub fn entity_of(&self, id: NodeId) -> Option<Entity> {
        self.slots.get(&id).map(|slot| slot.entity)
    }

    /// How many edges are currently painted.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

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
        for id in this.widget.nodes.clone() {
            if !this.widget.slots.contains_key(&id) {
                continue;
            }
            let transform = this.widget.child_transform(id);
            let mut child = this.ctx.get_mut(&mut this.widget.slots.get_mut(&id).unwrap().pod);
            child.set_transform(transform);
        }
    }

    /// Sets which node is selected, updating both the previously- and
    /// newly-selected `NodeBox`'s own `selected` flag.
    pub fn set_selected(this: &mut WidgetMut<'_, Self>, selected: Option<NodeId>) {
        let previous = this.widget.selected;
        if previous == selected {
            return;
        }
        this.widget.selected = selected;
        if let Some(id) = previous
            && let Some(slot) = this.widget.slots.get_mut(&id)
        {
            let mut child = this.ctx.get_mut(&mut slot.pod);
            NodeBox::set_selected(&mut child, false);
        }
        if let Some(id) = selected
            && let Some(slot) = this.widget.slots.get_mut(&id)
        {
            let mut child = this.ctx.get_mut(&mut slot.pod);
            NodeBox::set_selected(&mut child, true);
        }
    }

    /// Returns the currently selected node, if any.
    pub fn selected_node(&self) -> Option<NodeId> {
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
    fn child_transform(&self, id: NodeId) -> Affine {
        let pos = self.slots.get(&id).map(|slot| slot.pos).unwrap_or_default();
        Affine::translate(self.pan) * Affine::scale(self.zoom) * Affine::translate(pos.to_vec2())
    }

    fn paint_edge(&self, painter: &mut Painter<'_>, from: Point, to: Point, brush: Color, width: f64) {
        let dx = ((to.x - from.x) * 0.5).abs().max(30.0);
        let mut path = BezPath::new();
        path.move_to(from);
        path.curve_to(Point::new(from.x + dx, from.y), Point::new(to.x - dx, to.y), to);
        painter.stroke(&path, &Stroke::new(width), brush).draw();
    }

    /// Maps a canvas-space point to the window frame `GraphCanvas::paint`
    /// runs in (the canvas itself carries no transform -- see `paint`'s doc
    /// comment).
    fn to_visual(&self, p: Point) -> Point {
        (p.to_vec2() * self.zoom + self.pan).to_point()
    }

    /// `EventCtx` version of clearing the selection (background click).
    fn clear_selection(&mut self, ctx: &mut EventCtx<'_>) {
        if let Some(id) = self.selected.take()
            && let Some(slot) = self.slots.get_mut(&id)
        {
            ctx.mutate_child_later(&mut slot.pod, |mut node: WidgetMut<'_, NodeBox>| {
                NodeBox::set_selected(&mut node, false);
            });
        }
    }

    /// `ActionCtx` version of selecting a node (a `NodeBox` reported
    /// `NodeBoxAction::Selected`).
    fn select_from_action(&mut self, ctx: &mut ActionCtx<'_>, id: NodeId) {
        let previous = self.selected;
        if previous == Some(id) {
            return;
        }
        self.selected = Some(id);
        if let Some(prev_id) = previous
            && let Some(slot) = self.slots.get_mut(&prev_id)
        {
            ctx.mutate_child_later(&mut slot.pod, |mut node: WidgetMut<'_, NodeBox>| {
                NodeBox::set_selected(&mut node, false);
            });
        }
        if let Some(slot) = self.slots.get_mut(&id) {
            ctx.mutate_child_later(&mut slot.pod, |mut node: WidgetMut<'_, NodeBox>| {
                NodeBox::set_selected(&mut node, true);
            });
        }
    }

    /// `EventCtx` version of re-applying pan/zoom/position to every node
    /// (scroll-driven pan/zoom affects all of them at once).
    fn retransform_all_from_event(&mut self, ctx: &mut EventCtx<'_>) {
        for id in self.nodes.clone() {
            let transform = self.child_transform(id);
            if let Some(slot) = self.slots.get_mut(&id) {
                ctx.mutate_child_later(&mut slot.pod, move |mut node: WidgetMut<'_, NodeBox>| {
                    node.set_transform(transform);
                });
            }
        }
        ctx.request_paint_only();
    }

    /// `ActionCtx` version of re-applying pan/zoom/position to a single node
    /// (a drag only moves the one node being dragged).
    fn retransform_one_from_action(&mut self, ctx: &mut ActionCtx<'_>, id: NodeId) {
        let transform = self.child_transform(id);
        if let Some(slot) = self.slots.get_mut(&id) {
            ctx.mutate_child_later(&mut slot.pod, move |mut node: WidgetMut<'_, NodeBox>| {
                node.set_transform(transform);
            });
        }
    }
}

impl GraphCanvas {
    /// Maps a point in this widget's local space to canvas space — the
    /// inverse of `to_visual`.
    fn to_canvas(&self, local: Point) -> Point {
        ((local.to_vec2() - self.pan) / self.zoom).to_point()
    }

    /// Sends `Delete` for the selected node, if there is one. The canvas does
    /// not remove the node itself: the world is the truth, and the next
    /// snapshot is what takes the box away.
    fn delete_selected(&mut self) {
        let Some(entity) = self.selected.and_then(|id| self.entity_of(id)) else {
            return;
        };
        let _ = self.commands.send(EditorCommand::Delete { entity });
    }

    /// The open palette layer's id, for tests.
    pub fn palette_layer_id(&self) -> Option<WidgetId> {
        self.palette_layer.map(|(id, _)| id)
    }

    /// Test seam for the palette pick, which otherwise needs a live layer.
    pub fn palette_picked_for_test(
        this: &mut WidgetMut<'_, Self>,
        component: &'static str,
        pos: Point,
    ) {
        let _ = this.widget.commands.send(EditorCommand::Create {
            component,
            pos: WorldVec2::new(pos.x as f32, pos.y as f32),
        });
    }

    /// Test seam for the Delete key.
    pub fn delete_selected_for_test(this: &mut WidgetMut<'_, Self>) {
        this.widget.delete_selected();
    }
}

/// Base colour per wire name, brightened and thickened by activity.
fn edge_style(edge: &EdgeSlot) -> (Color, f64) {
    let base = edge_color(edge.wire);
    let Some(t) = normalised(edge) else {
        return (base, 2.0);
    };
    let lift = |c: u8| (c as f32 + (255.0 - c as f32) * t).round().clamp(0.0, 255.0) as u8;
    let [r, g, b, _] = base.to_rgba8().to_u8_array();
    (Color::from_rgb8(lift(r), lift(g), lift(b)), 2.0 + 3.0 * t as f64)
}

/// Colour by wire name. Local editor policy.
fn edge_color(wire: &str) -> Color {
    match wire {
        "parent" => Color::from_rgb8(170, 150, 110),
        "amplitude" => Color::from_rgb8(150, 130, 170),
        _ => Color::from_rgb8(140, 140, 155),
    }
}

/// The edge's current value mapped into 0..1 through its observed range.
/// `None` when there is no value; `0.5` when the range is degenerate (every
/// sample so far identical), which is the only neutral answer available.
fn normalised(edge: &EdgeSlot) -> Option<f32> {
    let value = edge.value?;
    let (lo, hi) = edge.range?;
    if (hi - lo).abs() < f32::EPSILON {
        return Some(0.5);
    }
    Some(((value - lo) / (hi - lo)).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::GraphCanvas;
    use crate::node_box::NodeBox;
    use crate::snapshot::{EdgeView, NodeId, NodeView, WorldSnapshot};
    use bevy_ecs::entity::Entity;
    use bevy_math::Vec2 as WorldVec2;
    use masonry::core::{DefaultProperties, PointerButton, Widget};
    use masonry_core::kurbo::{Point, Vec2};
    use masonry_testing::TestHarness;
    use sway_graph::EditorCommand;

    fn node(id: u32, name: &str, pos: Option<Point>) -> NodeView {
        NodeView {
            entity: Entity::from_raw_u32(id).expect("valid entity id"),
            id: NodeId(id),
            name: name.to_string(),
            pos,
            inlets: Vec::new(),
            outlets: 0,
        }
    }

    // A positional test fixture mirroring `EdgeView`'s own fields one for
    // one; splitting it into a builder would cost more at each of this
    // test module's few call sites than the lint saves.
    #[allow(clippy::too_many_arguments)]
    fn edge(
        from: NodeId,
        from_field: u16,
        from_index: u16,
        to: NodeId,
        to_field: u16,
        to_index: u16,
        wire: &'static str,
        activity: Option<f32>,
    ) -> EdgeView {
        EdgeView { from, from_field, from_index, to, to_field, to_index, wire, activity }
    }

    fn snapshot(nodes: Vec<NodeView>, edges: Vec<EdgeView>) -> WorldSnapshot {
        WorldSnapshot { nodes, edges, ..Default::default() }
    }

    fn harness_with(snap: WorldSnapshot) -> TestHarness<GraphCanvas> {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut harness =
            TestHarness::create(DefaultProperties::default(), GraphCanvas::new(tx).prepare());
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::apply_snapshot(&mut canvas, &snap);
        });
        harness
    }

    fn harness_and_rx(
        snap: WorldSnapshot,
    ) -> (TestHarness<GraphCanvas>, crossbeam_channel::Receiver<EditorCommand>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut harness =
            TestHarness::create(DefaultProperties::default(), GraphCanvas::new(tx).prepare());
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::apply_snapshot(&mut canvas, &snap);
        });
        (harness, rx)
    }

    #[test]
    fn picking_from_the_palette_creates_at_the_canvas_position() {
        let (mut harness, rx) = harness_and_rx(snapshot(vec![], vec![]));

        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::palette_picked_for_test(&mut canvas, "Lfo", Point::new(120.0, 60.0));
        });

        let commands: Vec<_> = rx.try_iter().collect();
        assert_eq!(commands.len(), 1);
        assert!(
            matches!(
                &commands[0],
                EditorCommand::Create { component: "Lfo", pos }
                    if *pos == WorldVec2::new(120.0, 60.0)
            ),
            "got {:?}",
            commands[0],
        );
    }

    #[test]
    fn a_right_click_opens_the_palette_at_the_pointer() {
        let mut snap = snapshot(vec![], vec![]);
        snap.palette = vec!["Lfo", "Remap"];
        let (mut harness, _rx) = harness_and_rx(snap);

        harness.mouse_move(Point::new(200.0, 150.0));
        harness.mouse_button_press(Some(PointerButton::Secondary));

        // The harness services NewLayer/RemoveLayer signals itself (see
        // `masonry_testing::TestHarness::process_signals`), so a layer that was
        // asked for is a layer that exists.
        assert!(
            harness.root_widget().palette_layer_id().is_some(),
            "a secondary press opens the palette",
        );
    }

    #[test]
    fn the_delete_key_deletes_the_selected_node() {
        let entity = Entity::from_raw_u32(4).expect("valid entity id");
        let (mut harness, rx) = harness_and_rx(snapshot(
            vec![NodeView { entity, ..node(4, "a", Some(Point::new(10.0, 10.0))) }],
            vec![],
        ));
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::set_selected(&mut canvas, Some(NodeId(4)));
            GraphCanvas::delete_selected_for_test(&mut canvas);
        });

        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![EditorCommand::Delete { entity }],
        );
    }

    #[test]
    fn deleting_with_nothing_selected_sends_nothing() {
        let (mut harness, rx) = harness_and_rx(snapshot(vec![], vec![]));
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::delete_selected_for_test(&mut canvas);
        });
        assert_eq!(rx.try_iter().count(), 0);
    }

    #[test]
    fn ending_a_drag_reports_the_nodes_new_canvas_position() {
        // MoveNode is what carries a drag into EditorPos and therefore into
        // the saved document. Before M6 a dragged position lived only in the
        // widget and was lost on exit.
        let entity = Entity::from_raw_u32(4).expect("valid entity id");
        let (mut harness, rx) = harness_and_rx(snapshot(
            vec![NodeView { entity, ..node(4, "a", Some(Point::new(100.0, 100.0))) }],
            vec![],
        ));

        harness.mouse_move(Point::new(150.0, 130.0));
        harness.mouse_button_press(Some(PointerButton::Primary));
        harness.mouse_move(Point::new(200.0, 180.0));
        harness.mouse_button_release(Some(PointerButton::Primary));

        let moved = harness.root_widget().position_of(NodeId(4)).unwrap();
        let commands: Vec<_> = rx.try_iter().collect();
        assert!(
            commands.iter().any(|c| matches!(
                c,
                EditorCommand::MoveNode { entity: e, pos }
                    if *e == entity
                        && *pos == WorldVec2::new(moved.x as f32, moved.y as f32)
            )),
            "the released position must be the one sent; got {commands:?}",
        );
    }

    /// The claim spec §2.8 makes for masonry, reduced to an assertion.
    ///
    /// A node sits at canvas-space (100, 100). The canvas is zoomed 2x, so it
    /// occupies window space around (200, 200). A press at (210, 210) must
    /// reach *that node's* widget -- not the canvas, not a neighbour. If
    /// masonry's `window_transform` inverse did not drive hit-testing, this
    /// press would land on whatever is at unscaled (210, 210) instead, and a
    /// node editor built on it would be subtly, unfixably wrong under zoom.
    #[test]
    fn press_under_zoom_reaches_the_scaled_node() {
        let mut harness = harness_with(snapshot(
            vec![
                node(0, "a", Some(Point::new(100.0, 100.0))),
                node(1, "b", Some(Point::new(400.0, 100.0))),
            ],
            vec![],
        ));
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::set_zoom(&mut canvas, 2.0);
        });

        harness.mouse_move(Point::new(210.0, 210.0));
        harness.mouse_button_press(Some(PointerButton::Primary));

        assert_eq!(harness.root_widget().selected_node(), Some(NodeId(0)));
    }

    #[test]
    fn press_outside_any_node_clears_selection() {
        let mut harness = harness_with(snapshot(
            vec![node(0, "a", Some(Point::new(100.0, 100.0)))],
            vec![],
        ));
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::set_selected(&mut canvas, Some(NodeId(0)));
        });

        harness.mouse_move(Point::new(20.0, 20.0));
        harness.mouse_button_press(Some(PointerButton::Primary));

        assert_eq!(harness.root_widget().selected_node(), None);
    }

    #[test]
    fn middle_drag_pans_the_canvas_by_the_raw_delta() {
        let mut harness = harness_with(snapshot(
            vec![node(0, "a", Some(Point::new(100.0, 100.0)))],
            vec![],
        ));

        harness.mouse_move(Point::new(50.0, 50.0));
        harness.mouse_button_press(Some(PointerButton::Auxiliary));
        harness.mouse_move(Point::new(80.0, 65.0));
        harness.mouse_button_release(Some(PointerButton::Auxiliary));

        assert_eq!(harness.root_widget().pan(), Vec2::new(30.0, 15.0));
    }

    /// A middle-drag that starts *over* a node must still pan the canvas,
    /// not drag the node -- `NodeBox` only claims the primary button.
    #[test]
    fn middle_drag_over_a_node_pans_instead_of_dragging_it() {
        let mut harness = harness_with(snapshot(
            vec![node(0, "a", Some(Point::new(100.0, 100.0)))],
            vec![],
        ));

        harness.mouse_move(Point::new(150.0, 130.0));
        harness.mouse_button_press(Some(PointerButton::Auxiliary));
        harness.mouse_move(Point::new(170.0, 150.0));
        harness.mouse_button_release(Some(PointerButton::Auxiliary));

        assert_eq!(harness.root_widget().pan(), Vec2::new(20.0, 20.0));
        assert_eq!(harness.root_widget().selected_node(), None);
    }

    #[test]
    fn scroll_pixel_delta_converts_physical_to_logical() {
        use masonry::core::{PointerEvent, PointerScrollEvent, PointerState, ScrollDelta};
        use masonry::dpi::PhysicalPosition;
        use masonry_testing::PRIMARY_MOUSE;

        let mut harness = harness_with(snapshot(
            vec![node(0, "a", Some(Point::new(100.0, 100.0)))],
            vec![],
        ));
        let state = PointerState {
            scale_factor: 2.0,
            position: PhysicalPosition { x: 100.0, y: 100.0 },
            ..Default::default()
        };

        harness.process_pointer_event(PointerEvent::Scroll(PointerScrollEvent {
            pointer: PRIMARY_MOUSE,
            delta: ScrollDelta::PixelDelta(PhysicalPosition { x: 40.0, y: 20.0 }),
            state,
        }));

        assert_eq!(harness.root_widget().pan(), Vec2::new(-20.0, -10.0));
    }

    #[test]
    fn scroll_line_delta_is_dpi_invariant() {
        use masonry::core::{PointerEvent, PointerScrollEvent, PointerState, ScrollDelta};
        use masonry::dpi::PhysicalPosition;
        use masonry_testing::PRIMARY_MOUSE;

        let mut harness = harness_with(snapshot(
            vec![node(0, "a", Some(Point::new(100.0, 100.0)))],
            vec![],
        ));
        let state = PointerState {
            scale_factor: 2.0,
            position: PhysicalPosition { x: 100.0, y: 100.0 },
            ..Default::default()
        };

        harness.process_pointer_event(PointerEvent::Scroll(PointerScrollEvent {
            pointer: PRIMARY_MOUSE,
            delta: ScrollDelta::LineDelta(0.0, 1.0),
            state,
        }));

        assert_eq!(harness.root_widget().pan(), Vec2::new(0.0, -32.0));
    }

    #[test]
    fn a_node_surviving_a_snapshot_keeps_its_widget_id() {
        let first = snapshot(vec![node(0, "a", Some(Point::new(10.0, 10.0)))], vec![]);
        let mut harness = harness_with(first);
        let before = harness.root_widget().widget_id_of(NodeId(0)).unwrap();

        let second = snapshot(
            vec![
                node(0, "a", Some(Point::new(10.0, 10.0))),
                node(1, "b", Some(Point::new(300.0, 10.0))),
            ],
            vec![],
        );
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::apply_snapshot(&mut canvas, &second);
        });

        assert_eq!(harness.root_widget().widget_id_of(NodeId(0)), Some(before));
        assert!(harness.root_widget().widget_id_of(NodeId(1)).is_some());
    }

    #[test]
    fn a_node_dropped_from_a_snapshot_is_removed() {
        let mut harness = harness_with(snapshot(
            vec![node(0, "a", None), node(1, "b", None)],
            vec![],
        ));
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::apply_snapshot(&mut canvas, &snapshot(vec![node(1, "b", None)], vec![]));
        });

        assert_eq!(harness.root_widget().widget_id_of(NodeId(0)), None);
        assert!(harness.root_widget().widget_id_of(NodeId(1)).is_some());
    }

    /// Design §5: `EditorPos` is a *seed*, read when a node box first appears
    /// and never again. Without this, the next frame's snapshot would snap a
    /// dragged node straight back to its authored position.
    #[test]
    fn a_dragged_node_is_not_snapped_back_by_the_next_snapshot() {
        let snap = snapshot(vec![node(0, "a", Some(Point::new(100.0, 100.0)))], vec![]);
        let mut harness = harness_with(snap.clone());

        harness.mouse_move(Point::new(150.0, 130.0));
        harness.mouse_button_press(Some(PointerButton::Primary));
        harness.mouse_move(Point::new(200.0, 180.0));
        harness.mouse_button_release(Some(PointerButton::Primary));
        let dragged = harness.root_widget().position_of(NodeId(0)).unwrap();
        assert_ne!(dragged, Point::new(100.0, 100.0), "the drag must have moved it");

        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::apply_snapshot(&mut canvas, &snap);
        });

        assert_eq!(harness.root_widget().position_of(NodeId(0)), Some(dragged));
    }

    /// A node with no `EditorPos` lands on the fallback grid, and two such
    /// nodes never collide.
    #[test]
    fn unpositioned_nodes_land_on_distinct_fallback_slots() {
        let harness = harness_with(snapshot(
            vec![node(0, "a", None), node(1, "b", None)],
            vec![],
        ));

        let a = harness.root_widget().position_of(NodeId(0)).unwrap();
        let b = harness.root_widget().position_of(NodeId(1)).unwrap();
        assert_ne!(a, b);
    }

    /// "Both endpoints exist" now means "both `NodeId`s are in this
    /// snapshot's node list" -- an edge no longer carries a list index to
    /// range-check, since `NodeId(9)` naming a node this snapshot doesn't
    /// have is exactly the case a despawned/never-compiled node produces.
    #[test]
    fn edges_are_kept_only_when_both_endpoints_exist() {
        let harness = harness_with(snapshot(
            vec![node(0, "a", None), node(1, "b", None)],
            vec![edge(NodeId(0), 0, 0, NodeId(1), 0, 0, "translation.y", Some(0.5))],
        ));
        assert_eq!(harness.root_widget().edge_count(), 1);

        let mut harness = harness;
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::apply_snapshot(
                &mut canvas,
                &snapshot(
                    vec![node(0, "a", None)],
                    vec![edge(NodeId(0), 0, 0, NodeId(9), 0, 0, "translation.y", None)],
                ),
            );
        });
        assert_eq!(harness.root_widget().edge_count(), 0);
    }

    #[test]
    fn a_node_box_lays_out_one_socket_per_inlet() {
        // An inlet is one registered wire type, so it is always exactly one
        // socket. Variadic (`Vec`) inlets stay out of MVP; `NodeBox` still
        // carries the per-field count that would express them.
        use crate::snapshot::InletView;
        let view = NodeView {
            inlets: vec![
                InletView { wire: "amount", connected: true },
                InletView { wire: "parent", connected: false },
            ],
            outlets: 1,
            ..node(0, "Recv", None)
        };
        let mut harness = harness_with(snapshot(vec![view], vec![]));
        let box_id = harness.root_widget().widget_id_of(NodeId(0)).unwrap();

        harness.edit_widget_with_id(box_id, |mut widget| {
            let node_box = widget.downcast::<NodeBox>();
            assert_eq!(node_box.widget.inlet_socket_count(), 2);
            assert_eq!(node_box.widget.outlet_socket_count(), 1);
        });
    }
}
