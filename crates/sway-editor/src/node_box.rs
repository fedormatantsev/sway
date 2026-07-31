//! `NodeBox` -- a leaf widget standing in for one graph node.
//!
//! This is throwaway UI (M7 rewrites the whole editor once a real graph
//! model exists -- controller dispatch ruling R1), so there is deliberately
//! no trait, no generics over node kinds, and no serialization here: just a
//! label and a `selected` flag. The point of Task 6 was structural, not
//! visual -- each node is a real child widget with its own [`WidgetId`].
//! Task 7 adds the interaction: selection, dragging, and drag-to-connect.
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
//! Reported deltas/positions are all in raw window-space (logical pixels),
//! deliberately *not* run through `ctx.local_position` -- see the task 7
//! report for why: this widget's own transform changes mid-drag (it encodes
//! the node's canvas position, which the drag is busy updating), so reading
//! a delta back out of that same transform via `local_position` would double
//! count each step. Window space is stable, so `GraphCanvas` (which alone
//! knows `pan`/`zoom`) converts once, centrally.

use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, PaintCtx, PointerButton,
    PointerButtonEvent, PointerEvent, PointerState, PointerUpdate, PropertiesMut, PropertiesRef,
    RegisterCtx, Widget, WidgetMut,
};
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry_core::kurbo::{Axis, Circle, Point, RoundedRect, Size, Stroke};
use peniko::Color;

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

/// Width of the drag-to-connect zone, as a fraction of `SIZE.width`, measured
/// from the right edge -- the brief's "right-hand quarter".
const CONNECT_ZONE_FRACTION: f64 = 0.25;

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
    /// Dragging a new edge out of the connector zone.
    Connecting,
}

/// The action a [`NodeBox`] reports to its parent [`GraphCanvas`] through
/// [`EventCtx::submit_action`]/[`Widget::on_action`] (R2, controller dispatch
/// ruling). All positions/deltas are window-space (logical pixels); see the
/// module doc.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NodeBoxAction {
    /// This node was pressed in its body (not the connector zone): the
    /// canvas should select it.
    Selected,
    /// The pointer moved by this delta while dragging the node body.
    DraggedBy(masonry_core::kurbo::Vec2),
    /// A drag-to-connect gesture started from this node's connector zone.
    ConnectStart,
    /// The drag-to-connect cursor moved to this window-space point.
    ConnectMove(Point),
    /// The drag-to-connect gesture ended (pointer released) at this
    /// window-space point.
    ConnectEnd(Point),
}

/// A node box in the graph canvas: a rounded rectangle with a border, drawn
/// through `imaging::Painter`. Selection flips its fill color; a small
/// connector dot on the right edge marks the drag-to-connect zone.
pub struct NodeBox {
    label: String,
    selected: bool,
    gesture: Gesture,
}

impl NodeBox {
    /// Creates a new, unselected node box with the given label.
    pub fn new(label: String) -> Self {
        Self {
            label,
            selected: false,
            gesture: Gesture::None,
        }
    }
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
}

/// Converts a [`PointerState`]'s position to a window-space (logical pixels)
/// [`Point`], the coordinate space every [`NodeBoxAction`] reports in.
fn window_point(state: &PointerState) -> Point {
    let p = state.logical_position();
    Point::new(p.x, p.y)
}

impl Widget for NodeBox {
    type Action = NodeBoxAction;

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

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
                // `ctx.local_position` is safe to use here (and only here):
                // at `Down`, this widget's transform hasn't been touched by
                // the gesture we're about to start, so there's no
                // moving-frame issue yet -- we just want to know which zone,
                // in this node's own unscaled coordinates, was clicked.
                let local = ctx.local_position(state.position);
                ctx.capture_pointer();
                if local.x >= SIZE.width * (1.0 - CONNECT_ZONE_FRACTION) {
                    self.gesture = Gesture::Connecting;
                    ctx.submit_action::<Self::Action>(NodeBoxAction::ConnectStart);
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
                let window = window_point(current);
                match &mut self.gesture {
                    Gesture::Dragging { last_window } => {
                        let delta = window - *last_window;
                        *last_window = window;
                        ctx.submit_action::<Self::Action>(NodeBoxAction::DraggedBy(delta));
                    }
                    Gesture::Connecting => {
                        ctx.submit_action::<Self::Action>(NodeBoxAction::ConnectMove(window));
                    }
                    Gesture::None => {}
                }
                ctx.set_handled();
            }
            PointerEvent::Up(PointerButtonEvent { state, .. }) => {
                if self.gesture == Gesture::Connecting {
                    ctx.submit_action::<Self::Action>(NodeBoxAction::ConnectEnd(window_point(
                        state,
                    )));
                }
                self.gesture = Gesture::None;
                ctx.set_handled();
            }
            PointerEvent::Cancel(..) => {
                self.gesture = Gesture::None;
            }
            _ => {}
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

        // Connector affordance: a small dot marking the drag-to-connect zone,
        // so a human running the app can see where to grab an edge from.
        let connector = Point::new(SIZE.width, SIZE.height / 2.0);
        painter
            .fill(Circle::new(connector, 4.0), Color::from_rgb8(220, 200, 120))
            .draw();
    }

    fn accessibility_role(&self) -> Role {
        Role::GenericContainer
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, node: &mut Node) {
        node.set_description(self.label.as_str());
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }

    // R6 (controller dispatch ruling): explicit even though `true` is also
    // the trait's own default, so Task 7's hit-testing test has something
    // concrete to point at on this widget.
    fn accepts_pointer_interaction(&self) -> bool {
        true
    }
}
