//! `NodeBox` -- a leaf widget standing in for one graph node.
//!
//! This is throwaway UI (M7 rewrites the whole editor once a real graph
//! model exists -- controller dispatch ruling R1), so there is deliberately
//! no trait, no generics over node kinds, and no serialization here: just a
//! label and a `selected` flag. The point of Task 6 was structural, not
//! visual -- each node is a real child widget with its own [`WidgetId`].
//! Task 7 adds the interaction: selection and dragging.
//!
//! Modeled on `masonry::widgets::canvas::Canvas` (a leaf custom-drawing
//! widget) for the overall `Widget` impl shape, and on
//! `sway_editor::external::ViewportPlaceholder` for how a small leaf widget
//! in this crate is put together.
//!
//! # Child -> parent communication (controller dispatch ruling R2)
//!
//! `NodeBox` doesn't own its canvas position -- `GraphCanvas` does, since it's
//! the one doing layout and painting edges. So `NodeBox` never repositions
//! itself; it captures the pointer on its own `Down` (same idiom as
//! `masonry::widgets::split::Split`'s draggable bar, and `Selector`'s
//! open/close capture) and reports what happened upward through
//! `EventCtx::submit_action`/`Widget::on_action`, the same idiom used by
//! `masonry::widgets::pagination::Pagination` (its `Button` children report
//! `ButtonPress`; `Pagination::on_action` matches on the action and the
//! `source: WidgetId` to find which child fired). `GraphCanvas::on_action`
//! does the same thing here.
//!
//! Dragging/dragged-node deltas and positions (`DraggedBy`) are raw
//! window-space (logical pixels), deliberately *not* run through
//! `ctx.local_position` -- see the task 7 report for why: this widget's own
//! transform changes mid-drag (it encodes the node's canvas position, which
//! the drag is busy updating), so reading a delta back out of that same
//! transform via `local_position` would double count each step. Window space
//! is stable, so `GraphCanvas` (which alone knows `pan`/`zoom`) converts
//! once, centrally.
//!
//! Socket positions (`SocketPressed`/`ConnectDragged`/`ConnectReleased`) are
//! the opposite: this box's own transform is *not* changing mid-gesture (only
//! the node being dragged repositions itself), so `ctx.local_position`
//! already divides out pan/zoom safely, and is reported in this box's own
//! local space -- `GraphCanvas` adds `slot.pos` to get canvas space.

use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, PaintCtx, PointerButton,
    PointerButtonEvent, PointerEvent, PointerState, PointerUpdate, PropertiesMut, PropertiesRef,
    RegisterCtx, Update, UpdateCtx, Widget, WidgetMut, WidgetPod,
};
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry::widgets::Label;
use masonry_core::kurbo::{Affine, Axis, Circle, Point, RoundedRect, Size, Stroke};
use peniko::Color;

use crate::canvas::SocketKind;

/// Fixed footprint of every node box, in canvas-space logical pixels.
///
/// `GraphCanvas::layout` uses this directly when it calls `ctx.run_layout`
/// on each node -- the parent decides the child's border-box size outright
/// (which `Widget::layout`'s own docs call out as valid), so `NodeBox::measure`
/// below is never actually exercised in this app. It is still implemented
/// (measure has no default body in the `Widget` trait) and kept consistent
/// with this constant so nothing subtle breaks if that ever changes.
pub(crate) const SIZE: Size = Size::new(160.0, 72.0);

const CORNER_RADIUS: f64 = 8.0;

/// Inset of the label from the box's top-left corner, in logical pixels.
const LABEL_INSET: f64 = 10.0;

/// Radius of a drawn socket dot, in logical pixels. `pub(crate)` so
/// `GraphCanvas`'s own hit test (`SOCKET_HIT_RADIUS`) agrees with this on one
/// number.
pub(crate) const SOCKET_RADIUS: f64 = 4.0;

/// How close a probe must be to a socket to count as hitting it, in local
/// pixels. Deliberately larger than the dot itself -- an exact-radius target
/// is unhittable in practice. `pub(crate)` so `GraphCanvas`'s own hit test
/// (`canvas.rs`'s socket-drag/connect probing) uses the same number instead
/// of a second, independently-drifting `* 2.5` literal.
pub(crate) const SOCKET_HIT_RADIUS: f64 = SOCKET_RADIUS * 2.5;

/// What the pointer is currently doing to this node box, between a `Down`
/// that started a gesture and the `Up`/`Cancel` that ends it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Gesture {
    /// No button is down, or the last gesture already ended.
    None,
    /// Dragging the node body. Carries the last-seen window-space (logical)
    /// pointer position, so `Move` can report an *incremental* delta rather
    /// than a delta-since-`Down` (see the module doc for why this can't be
    /// derived from `ctx.local_position` instead).
    Dragging { last_window: Point },
    /// Dragging an edge out of one of this box's sockets.
    Connecting,
}

/// The action a [`NodeBox`] reports to its parent [`GraphCanvas`] through
/// [`EventCtx::submit_action`]/[`Widget::on_action`]. Deltas are window-space
/// (logical pixels); see the module doc.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NodeBoxAction {
    /// This node was pressed: the canvas should select it.
    Selected,
    /// The pointer moved by this delta while dragging the node.
    DraggedBy(masonry_core::kurbo::Vec2),
    /// The drag finished. The canvas writes the node's settled position back
    /// to the world; a press with no movement reports this too, and the
    /// world-side equal-value guard makes that a no-op.
    DragEnded,
    /// A press landed on one of this box's sockets. Positions in the two
    /// variants below are in this box's own local space; the canvas adds the
    /// box's canvas position to get canvas space (see the task preamble).
    SocketPressed(SocketKind),
    /// The pointer moved while dragging from a socket.
    ConnectDragged(Point),
    /// The socket drag ended here.
    ConnectReleased(Point),
    /// A socket drag was cancelled (`PointerEvent::Cancel` -- the window lost
    /// input focus, or an OS-level gesture cancellation) rather than
    /// released. Distinct from `ConnectReleased` rather than reusing it at
    /// some synthetic point: `GraphCanvas::connect_released` is what Task 15
    /// teaches to turn a landing on a legal inlet into a `Connect`, and a
    /// cancellation must never be mistaken for a landing there.
    ConnectCanceled,
}

/// A node box in the graph canvas: a rounded rectangle with a border and a
/// text label, drawn through `imaging::Painter` and one `Label` child.
///
/// `Label` rather than painting text directly: `imaging::Painter` exposes
/// only `glyphs`, which takes *pre-shaped* glyphs, and shaping is masonry's
/// job. `Label::accepts_pointer_interaction` is `false`, so the child never
/// steals a press from this widget's own gesture handling.
pub struct NodeBox {
    label: WidgetPod<Label>,
    label_text: String,
    selected: bool,
    gesture: Gesture,
    /// Slot count per inlet field, in order -- the same numbers
    /// `NodeView::inlets` carries, so a `Vec` inlet (e.g. a `Group`'s
    /// `children`) draws one socket per element rather than one per field.
    inlets: Vec<u16>,
    /// How many outlet fields this node has. Never per-slot: an outlet can't
    /// be a `Vec` (design §12).
    outlets: u16,
    /// Applied to this widget's own transform on `Update::WidgetAdded`.
    ///
    /// A freshly created `WidgetPod` isn't yet registered in masonry's arena
    /// (that happens during the update pass that immediately follows), so
    /// `GraphCanvas` cannot reach into a brand-new `NodeBox` via
    /// `get_mut`/`mutate_child_later` the moment it creates one -- both panic
    /// ("child not found") or silently drop the callback. `Update::WidgetAdded`
    /// is delivered to the widget itself once it *is* registered, which is
    /// exactly the documented purpose of that event ("initial setup that
    /// cannot be done when constructing the widget"), so `NodeBox` applies
    /// its own seed transform there instead of waiting on its parent.
    initial_transform: Affine,
}

impl NodeBox {
    /// Creates a new, unselected node box with the given label.
    pub fn new(label: String) -> Self {
        Self {
            label: Label::new(label.clone()).prepare().to_pod(),
            label_text: label,
            selected: false,
            gesture: Gesture::None,
            inlets: Vec::new(),
            outlets: 0,
            initial_transform: Affine::IDENTITY,
        }
    }

    /// Sets the transform this box applies to itself the moment it's added to
    /// the tree. Called by `GraphCanvas::apply_snapshot` before `.prepare()`,
    /// since that's the only time it can hand a new box its seed position --
    /// see `initial_transform`'s doc comment.
    pub(crate) fn with_initial_transform(mut self, transform: Affine) -> Self {
        self.initial_transform = transform;
        self
    }

    /// Seeds this box's socket counts. Called by `GraphCanvas::apply_snapshot`
    /// when creating a new box, from that frame's `NodeView`; `set_sockets`
    /// (below) is the update path for a box that already exists.
    pub(crate) fn with_sockets(mut self, inlets: Vec<u16>, outlets: u16) -> Self {
        self.inlets = inlets;
        self.outlets = outlets;
        self
    }

    /// The text this box currently displays.
    pub fn label_text(&self) -> &str {
        &self.label_text
    }

    /// Total inlet sockets across every inlet field -- a `Vec` field
    /// contributes one socket per element, not one per field.
    pub fn inlet_socket_count(&self) -> usize {
        self.inlets.iter().map(|&len| len as usize).sum()
    }

    /// Total outlet sockets: one per outlet field.
    pub fn outlet_socket_count(&self) -> usize {
        self.outlets as usize
    }

    /// Canvas-space position (relative to this box's own (0,0) origin) of one
    /// inlet socket. `field`/`index` are read directly off an edge's target
    /// `Endpoint` -- `field` is the node's flat field ordinal with inlets
    /// first, so it also directly indexes `inlets`.
    pub fn inlet_socket_pos(&self, field: u16, index: u16) -> Point {
        inlet_socket_local(&self.inlets, field, index)
    }

    /// Canvas-space position of one outlet socket. `field` is the node's flat
    /// ordinal (inlets first), so `field - inlets.len()` is the outlet's own
    /// ordinal among just the outlets.
    pub fn outlet_socket_pos(&self, field: u16) -> Point {
        outlet_socket_local(self.inlets.len() as u16, self.outlets, field)
    }

    /// Which of this box's sockets a local-space point is on, if any. Same
    /// radius and same geometry the canvas uses.
    fn socket_at_local(&self, local: Point) -> Option<SocketKind> {
        let inlet_fields = self.inlets.len() as u16;
        if self.outlets > 0 {
            let outlet = outlet_socket_local(inlet_fields, self.outlets, inlet_fields);
            if outlet.distance(local) <= SOCKET_HIT_RADIUS {
                return Some(SocketKind::Outlet);
            }
        }
        for ordinal in 0..inlet_fields {
            if inlet_socket_local(&self.inlets, ordinal, 0).distance(local) <= SOCKET_HIT_RADIUS {
                return Some(SocketKind::Inlet(ordinal));
            }
        }
        None
    }
}

/// Local (relative to a box's own (0,0)-(160,72) origin) position of one
/// inlet socket, evenly spaced down the left edge. A free function, not just
/// `NodeBox::inlet_socket_pos`, because `GraphCanvas::paint` needs the same
/// math against `NodeSlot`'s own copy of `inlets`/`outlets` -- masonry gives
/// a parent no read access to a live child's widget state from `PaintCtx`,
/// and reaching in via `MutateCtx::get_mut` from `apply_snapshot` instead
/// isn't an option either: a `NodeBox` created *in that same call* isn't yet
/// registered in masonry's arena (see `NodeBox::initial_transform`'s doc
/// comment), so `get_mut` on a brand-new node's socket -- exactly the first
/// snapshot that draws the parenting edge in the `--editor` demo -- panics
/// ("child not found"). `NodeSlot` mirroring the counts, same as it already
/// does for `label`, sidesteps the whole registration-timing question.
///
/// Never panics: an out-of-range `field`/`index` (a stale edge against a
/// resized or removed field) degrades to the nearest valid socket rather than
/// crashing the whole canvas mid-paint.
pub(crate) fn inlet_socket_local(inlets: &[u16], field: u16, index: u16) -> Point {
    let total: usize = inlets.iter().map(|&len| len as usize).sum();
    let before: usize = inlets
        .get(..(field as usize).min(inlets.len()))
        .unwrap_or(&[])
        .iter()
        .map(|&len| len as usize)
        .sum();
    Point::new(0.0, socket_y(total, before + index as usize))
}

/// Local position of one outlet socket, evenly spaced down the right edge.
/// `field` is the node's flat ordinal (inlets first); `inlet_field_count`
/// (== the inlet-slot-count `Vec`'s own length) is what turns that into the
/// outlet's own ordinal among just the outlets. See `inlet_socket_local` for
/// why this is a free function.
pub(crate) fn outlet_socket_local(inlet_field_count: u16, outlets: u16, field: u16) -> Point {
    let ordinal = field.saturating_sub(inlet_field_count) as usize;
    Point::new(SIZE.width, socket_y(outlets as usize, ordinal))
}

/// Every inlet socket's local position, in slot order -- what `NodeBox::paint`
/// draws dots at.
fn inlet_socket_positions(inlets: &[u16]) -> impl Iterator<Item = Point> + '_ {
    let total: usize = inlets.iter().map(|&len| len as usize).sum();
    (0..total).map(move |ordinal| Point::new(0.0, socket_y(total, ordinal)))
}

/// Every outlet socket's local position -- what `NodeBox::paint` draws dots
/// at.
fn outlet_socket_positions(outlets: u16) -> impl Iterator<Item = Point> {
    let total = outlets as usize;
    (0..total).map(move |ordinal| Point::new(SIZE.width, socket_y(total, ordinal)))
}

/// One socket's vertical offset among `total` sockets evenly spaced over the
/// box's height -- `total + 1` gaps so the first and last sockets sit inset
/// from the corners, not flush with them. `total == 0` centres on the box's
/// vertical midpoint, which is unreachable in practice (nothing asks for a
/// position among zero sockets) but keeps this total rather than panicking.
fn socket_y(total: usize, ordinal: usize) -> f64 {
    if total == 0 {
        return SIZE.height / 2.0;
    }
    let ordinal = ordinal.min(total - 1);
    SIZE.height * (ordinal as f64 + 1.0) / (total as f64 + 1.0)
}

// --- MARK: WIDGETMUT
impl NodeBox {
    /// Sets whether this node box is drawn as selected.
    ///
    /// Called by `GraphCanvas` (which owns the single source of truth for
    /// *which* node is selected) via `ctx.mutate_child_later`/`ctx.get_mut`,
    /// never by `NodeBox` itself.
    pub fn set_selected(this: &mut WidgetMut<'_, Self>, selected: bool) {
        if this.widget.selected != selected {
            this.widget.selected = selected;
            this.ctx.request_paint_only();
        }
    }

    /// Replaces the displayed text. Called by `GraphCanvas` when a snapshot
    /// renames a node -- which happens on a node-type change under a
    /// surviving `NodeId`.
    pub fn set_label(this: &mut WidgetMut<'_, Self>, label: &str) {
        if this.widget.label_text == label {
            return;
        }
        label.clone_into(&mut this.widget.label_text);
        let mut child = this.ctx.get_mut(&mut this.widget.label);
        Label::set_text(&mut child, label.to_string());
    }

    /// Updates this box's socket counts on an existing node -- e.g. a
    /// `Group`'s `children` `Vec` grew or shrank across a recompile. A no-op,
    /// paint-only change: sockets carry no state of their own, so nothing
    /// downstream (edges are rebuilt outright every snapshot) needs telling.
    pub fn set_sockets(this: &mut WidgetMut<'_, Self>, inlets: Vec<u16>, outlets: u16) {
        if this.widget.inlets == inlets && this.widget.outlets == outlets {
            return;
        }
        this.widget.inlets = inlets;
        this.widget.outlets = outlets;
        this.ctx.request_paint_only();
    }
}

/// Converts a [`PointerState`]'s position to a window-space (logical pixels)
/// [`Point`], the coordinate space every [`NodeBoxAction`] reports in.
fn window_point(state: &PointerState) -> Point {
    let p = state.logical_position();
    Point::new(p.x, p.y)
}

impl Widget for NodeBox {
    type Action = NodeBoxAction;

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.label);
    }

    fn measure(
        &mut self,
        _ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        axis: Axis,
        len_req: LenReq,
        _cross_length: Option<Length>,
    ) -> Length {
        // Unreachable in practice -- see `SIZE`'s doc comment. Kept total.
        let fallback = match axis {
            Axis::Horizontal => SIZE.width,
            Axis::Vertical => SIZE.height,
        };
        match len_req {
            LenReq::FitContent(space) => space,
            _ => Length::const_px(fallback),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        let inner = Size::new(
            (size.width - 2.0 * LABEL_INSET).max(0.0),
            (size.height - 2.0 * LABEL_INSET).max(0.0),
        );
        ctx.run_layout(&mut self.label, inner);
        ctx.place_child(&mut self.label, Point::new(LABEL_INSET, LABEL_INSET));
        ctx.set_clip_path(size.to_rect());
    }

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        match event {
            PointerEvent::Down(PointerButtonEvent { button, state, .. }) => {
                if *button != Some(PointerButton::Primary) {
                    // Anything other than the primary button -- in
                    // particular the middle button, which `GraphCanvas`
                    // uses to pan directly (brief step 4) -- is not a node
                    // gesture. Leave it unhandled so it bubbles up to
                    // `GraphCanvas::on_pointer_event`.
                    return;
                }
                // Claims keyboard focus (matching masonry's own `Button`/
                // `Split`'s drag bar): `GraphCanvas` owns the Delete/Backspace
                // handler, but this press is what `GraphCanvas` itself never
                // sees directly (it captures the pointer below), so the key
                // has to reach `GraphCanvas` by bubbling from *this* widget's
                // focus instead. `NodeBox` leaves text events unhandled, so
                // that bubbling reaches its real parent, `GraphCanvas`. Every
                // primary press claims focus this way, socket or not -- there
                // is no reason a socket-drag press should leave the box
                // unfocused when a body press wouldn't.
                ctx.request_focus();
                ctx.capture_pointer();
                let local = ctx.local_position(state.position);
                if let Some(kind) = self.socket_at_local(local) {
                    self.gesture = Gesture::Connecting;
                    ctx.submit_action::<Self::Action>(NodeBoxAction::SocketPressed(kind));
                } else {
                    self.gesture = Gesture::Dragging {
                        last_window: window_point(state),
                    };
                    ctx.submit_action::<Self::Action>(NodeBoxAction::Selected);
                }
                // Stop this from also bubbling to `GraphCanvas::on_pointer_event`,
                // which treats an unhandled `Down` as "background click, clear
                // selection" -- see that method's doc comment.
                ctx.set_handled();
            }
            PointerEvent::Move(PointerUpdate { current, .. }) if ctx.is_active() => {
                match &mut self.gesture {
                    Gesture::Dragging { last_window } => {
                        let window = window_point(current);
                        let delta = window - *last_window;
                        *last_window = window;
                        ctx.submit_action::<Self::Action>(NodeBoxAction::DraggedBy(delta));
                    }
                    Gesture::Connecting => {
                        let local = ctx.local_position(current.position);
                        ctx.submit_action::<Self::Action>(NodeBoxAction::ConnectDragged(local));
                    }
                    Gesture::None => {}
                }
                ctx.set_handled();
            }
            PointerEvent::Up(PointerButtonEvent { state, .. }) => {
                match self.gesture {
                    Gesture::Dragging { .. } => {
                        ctx.submit_action::<Self::Action>(NodeBoxAction::DragEnded);
                    }
                    Gesture::Connecting => {
                        let local = ctx.local_position(state.position);
                        ctx.submit_action::<Self::Action>(NodeBoxAction::ConnectReleased(local));
                    }
                    Gesture::None => {}
                }
                self.gesture = Gesture::None;
                ctx.set_handled();
            }
            PointerEvent::Cancel(..) => {
                // `NodeBox` captures the pointer on `Down`, so every
                // follow-up event during a socket drag -- including this one
                // -- routes here, never to `GraphCanvas`. Without reporting
                // it, `GraphCanvas.drag` (and the rubber-band paint) would be
                // left dangling with no `Up` ever coming to clear it.
                if matches!(self.gesture, Gesture::Connecting) {
                    ctx.submit_action::<Self::Action>(NodeBoxAction::ConnectCanceled);
                }
                self.gesture = Gesture::None;
            }
            _ => {}
        }
    }

    /// Applies `initial_transform` once this box is registered in the tree.
    /// See that field's doc comment for why this can't happen synchronously
    /// in `GraphCanvas::apply_snapshot` instead.
    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if let Update::WidgetAdded = event {
            ctx.set_transform(self.initial_transform);
        }
    }

    fn paint(
        &mut self,
        _ctx: &mut PaintCtx<'_>,
        _props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        let rect = RoundedRect::new(0.0, 0.0, SIZE.width, SIZE.height, CORNER_RADIUS);
        let fill = if self.selected {
            Color::from_rgb8(90, 120, 200)
        } else {
            Color::from_rgb8(60, 64, 74)
        };
        painter.fill(rect, fill).draw();
        painter
            .stroke(rect, &Stroke::new(1.5), Color::from_rgb8(200, 200, 210))
            .draw();

        // Sockets: one dot per slot, so every edge visibly starts and ends
        // somewhere on the box rather than at its unmarked centre.
        let socket_fill = Color::from_rgb8(220, 220, 230);
        for pos in inlet_socket_positions(&self.inlets) {
            painter
                .fill(Circle::new(pos, SOCKET_RADIUS), socket_fill)
                .draw();
        }
        for pos in outlet_socket_positions(self.outlets) {
            painter
                .fill(Circle::new(pos, SOCKET_RADIUS), socket_fill)
                .draw();
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::GenericContainer
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        node: &mut Node,
    ) {
        node.set_description(self.label_text.as_str());
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::from_slice(&[self.label.id()])
    }

    // R6 (controller dispatch ruling): explicit even though `true` is also
    // the trait's own default, so Task 7's hit-testing test has something
    // concrete to point at on this widget.
    fn accepts_pointer_interaction(&self) -> bool {
        true
    }

    /// Required for `ctx.request_focus()` (above) to actually grant this
    /// widget focus -- see that call site's doc comment for why the Delete
    /// key needs it to.
    fn accepts_focus(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{NodeBox, SIZE};
    use masonry::core::{DefaultProperties, PointerButton, Widget};
    use masonry_core::kurbo::Point;
    use masonry_testing::TestHarness;

    #[test]
    fn a_node_box_has_a_label_child_carrying_its_text() {
        let node = NodeBox::new("LFO #3".to_string());
        let harness = TestHarness::create(DefaultProperties::default(), node.prepare());
        assert_eq!(harness.root_widget().label_text(), "LFO #3");
        assert_eq!(harness.root_widget().children_ids().len(), 1);
    }

    #[test]
    fn a_press_in_the_right_hand_quarter_selects_rather_than_connects() {
        // Drag-to-connect is gone: the whole box is now one gesture, so a
        // press anywhere -- including where the connector dot used to be --
        // selects and drags.
        let node = NodeBox::new("n".to_string());
        let mut harness = TestHarness::create(DefaultProperties::default(), node.prepare());

        harness.mouse_move(Point::new(SIZE.width - 8.0, SIZE.height / 2.0));
        harness.mouse_button_press(Some(PointerButton::Primary));

        let action = harness.pop_action_erased();
        assert!(action.is_some(), "a press must still submit an action");
    }

    #[test]
    fn socket_positions_sit_on_the_left_and_right_edges() {
        let node = NodeBox::new("n".to_string()).with_sockets(vec![2, 1], 1);

        // children[0], children[1], the scalar inlet: all on the left edge.
        assert_eq!(node.inlet_socket_pos(0, 0).x, 0.0);
        assert_eq!(node.inlet_socket_pos(0, 1).x, 0.0);
        assert_eq!(node.inlet_socket_pos(1, 0).x, 0.0);
        // The one outlet: on the right edge.
        assert_eq!(node.outlet_socket_pos(2).x, SIZE.width);
    }

    #[test]
    fn socket_positions_are_distinct_and_ordered_top_to_bottom() {
        // 3 inlet slots across 2 fields (children[0], children[1], the
        // scalar), so every one of them must land at a different height, in
        // slot order -- otherwise two edges into different slots would draw
        // on top of each other.
        let node = NodeBox::new("n".to_string()).with_sockets(vec![2, 1], 1);

        let a = node.inlet_socket_pos(0, 0).y;
        let b = node.inlet_socket_pos(0, 1).y;
        let c = node.inlet_socket_pos(1, 0).y;
        assert!(a < b && b < c, "expected {a} < {b} < {c}");
    }

    #[test]
    fn a_lone_socket_centres_on_the_box() {
        let node = NodeBox::new("n".to_string()).with_sockets(vec![1], 1);
        assert_eq!(node.inlet_socket_pos(0, 0).y, SIZE.height / 2.0);
        assert_eq!(node.outlet_socket_pos(1).y, SIZE.height / 2.0);
    }

    #[test]
    fn socket_counts_sum_a_vec_inlet_by_its_slots_not_its_fields() {
        let node = NodeBox::new("n".to_string()).with_sockets(vec![2, 1], 1);
        assert_eq!(node.inlet_socket_count(), 3, "2 children + 1 scalar");
        assert_eq!(node.outlet_socket_count(), 1);
    }
}
