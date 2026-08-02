//! `ViewportPlaceholder` -- the Bevy viewport's seat in the widget tree.
//!
//! `VisualLayerKind::External` is masonry's placeholder for content a host
//! renders itself; setting `PaintLayerMode::External` in `paint` is what
//! keeps this widget's subtree out of masonry's own vello scene, leaving a
//! genuine hole for the compositor's Bevy quad to show through.
//!
//! Finding *where* that hole is, in window space, does **not** go through
//! `VisualLayerKind::External`'s reported `bounds`/`transform` -- an earlier
//! version of this module did, and it was wrong for any nested layout (see
//! `EditorUi::viewport_rect`'s doc comment in `sway-editor/src/lib.rs` for
//! the full story of why, and what reads the rect correctly instead).
//!
//! Upstream documents `External` as pre-integration ("current hosts do not
//! realize these placeholders yet"), so this module is the host integration
//! it is waiting for, in the narrow form this app needs: exactly one
//! external layer, the Bevy viewport.

use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, LayoutCtx, MeasureCtx, NoAction, PaintCtx, PropertiesMut, PropertiesRef,
    RegisterCtx, Update, UpdateCtx, Widget,
};
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry_core::kurbo::{Axis, Size};

/// A leaf widget that paints nothing itself and instead marks its subtree as
/// an external paint layer (`PaintLayerMode::External`) -- masonry's
/// placeholder for content a host renders outside masonry's own scene graph.
/// This is the Bevy viewport's seat in the widget tree: its layout box is
/// exactly the rectangle the Bevy-rendered point cloud should occupy on
/// screen. `EditorUi::viewport_rect` reads that rectangle back out, off this
/// widget's own state rather than the `VisualLayerPlan`.
///
/// # Continuous repaint (a real gap in `External`, not a hypothetical one)
///
/// `PaintCtx::set_paint_layer_mode` resets to `Inline` at the start of
/// *every* paint pass for a widget, and is only set back to `External` while
/// that widget's own `paint` method actually runs. `paint` only runs when the
/// widget has a pending paint request; masonry does not repaint idle widgets
/// on every frame just because the host calls `redraw()` again. A one-shot
/// placeholder would therefore appear in the very first `VisualLayerPlan`
/// (new widgets start dirty) and then silently vanish from every plan after
/// that, taking the viewport rect with it.
///
/// This widget keeps itself dirty forever by requesting an animation frame
/// on creation and re-requesting one (plus a repaint) every time one fires,
/// the same mechanism `masonry::widgets::Spinner` uses to animate. See
/// `EditorUi::redraw` for the other half: it pumps a `WindowEvent::AnimFrame`
/// ahead of every real `redraw()` call so the request is actually serviced,
/// since this host does not otherwise drive masonry's animation clock.
pub struct ViewportPlaceholder;

impl Default for ViewportPlaceholder {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewportPlaceholder {
    /// Returns a fresh widget, ready for `.prepare()` (the usual
    /// `Widget::prepare` convenience, wrapping it in a `NewWidget`) and
    /// `.with_props(Dimensions::fixed(..))` to give it a real size.
    pub fn new() -> Self {
        Self
    }
}

impl Widget for ViewportPlaceholder {
    type Action = NoAction;

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if let Update::WidgetAdded = event {
            ctx.request_anim_frame();
        }
    }

    fn on_anim_frame(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, _interval: u64) {
        // Keep the loop going and force another `paint` call so
        // `PaintLayerMode::External` gets re-asserted every frame.
        ctx.request_anim_frame();
        ctx.request_paint_only();
    }

    fn measure(
        &mut self,
        _ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        _axis: Axis,
        len_req: LenReq,
        cross_length: Option<Length>,
    ) -> Length {
        // Unreachable in practice: the widget is always given explicit
        // `Dimensions`, and per `Widget::measure`'s own docs that resolves
        // before `measure` would ever be called. Kept total and harmless
        // regardless.
        match len_req {
            LenReq::FitContent(space) => space,
            LenReq::MinContent | LenReq::MaxContent => cross_length.unwrap_or(Length::ZERO),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        // Matches `masonry/src/tests/paint.rs`'s only example of
        // `PaintLayerMode::External` (R4, controller dispatch ruling):
        // the placeholder clips to its full content box.
        ctx.set_clip_path(size.to_rect());
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, _painter: &mut Painter<'_>) {
        ctx.set_paint_layer_mode(masonry::core::PaintLayerMode::External);
    }

    fn accessibility_role(&self) -> Role {
        Role::GenericContainer
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, _node: &mut Node) {}

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }

    /// The Bevy viewport is composited by the host into this widget's layout
    /// box; the placeholder itself has no pointer behavior. Returning false
    /// lets hits fall through to overlapping `NodeBox`es (masonry picks the
    /// last z-order child that accepts interaction -- this widget is
    /// registered after the nodes, so accepting would steal the center of
    /// the canvas).
    fn accepts_pointer_interaction(&self) -> bool {
        false
    }
}
