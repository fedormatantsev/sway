//! `NodeBox` -- a leaf widget standing in for one graph node.
//!
//! This is throwaway UI (M7 rewrites the whole editor once a real graph
//! model exists -- controller dispatch ruling R1), so there is deliberately
//! no trait, no generics over node kinds, and no serialization here: just a
//! label and a `selected` flag. The point of Task 6 is structural, not
//! visual -- each node is a real child widget with its own [`WidgetId`], so
//! that Task 7 can give it real selection/drag state instead of re-deriving
//! it from a `Vec` every frame.
//!
//! Modeled on `masonry::widgets::canvas::Canvas` (a leaf custom-drawing
//! widget) for the overall `Widget` impl shape, and on
//! `sway_editor::external::ViewportPlaceholder` for how a small leaf widget
//! in this crate is put together.

use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, LayoutCtx, MeasureCtx, NoAction, PaintCtx, PropertiesRef, RegisterCtx,
    Widget,
};
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry_core::kurbo::{Axis, RoundedRect, Size, Stroke};
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

/// A node box in the graph canvas: a rounded rectangle with a border, drawn
/// through `imaging::Painter`. Selection later flips its fill color; nothing
/// in Task 6 sets `selected` yet (Task 7 owns that).
pub struct NodeBox {
    label: String,
    selected: bool,
}

impl NodeBox {
    /// Creates a new, unselected node box with the given label.
    pub fn new(label: String) -> Self {
        Self {
            label,
            selected: false,
        }
    }
}

impl Widget for NodeBox {
    type Action = NoAction;

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
            .stroke(&rect, &Stroke::new(1.5), Color::from_rgb8(200, 200, 210))
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
