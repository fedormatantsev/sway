//! Viewport input, editor to world. Spec M7-1.
//!
//! A vocabulary of pointer, scroll and key events over the Bevy viewport,
//! shared by the crate that produces them (the editor's masonry widget) and
//! the crate that consumes them (the editor viewport's camera, gizmo and
//! picker). Neither may depend on the other, so it lives on its own.

use bevy_math::Vec2;

/// Which pointer button an event carries. This crate cannot name masonry's
/// `PointerButton`, and the world side has no business knowing masonry
/// exists, so the widget translates at the boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportButton {
    Primary,
    Secondary,
}

/// The modifier keys held when an event was produced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewportModifiers {
    pub alt: bool,
    pub shift: bool,
    pub control: bool,
    pub meta: bool,
}

/// The only keys the viewport consumes. Everything else bubbles past it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportKey {
    Translate,
    Rotate,
    Scale,
}

/// One input event over the Bevy viewport.
///
/// Every `pos` is **normalized to the viewport rect**: `[0,1]²` with the
/// origin at the top-left, unclamped. Not logical window pixels and not
/// physical ones — see spec M7-1: `Camera::viewport_to_ndc` divides by
/// `logical_viewport_rect()`, which for a `RenderTarget::TextureView` is the
/// texture's own (physical) size, while masonry's coordinates are logical.
/// Normalizing here makes the world side `pos * camera.logical_viewport_size()`
/// with no scale factor anywhere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewportInput {
    Down {
        button: ViewportButton,
        pos: Vec2,
        modifiers: ViewportModifiers,
    },
    Move {
        pos: Vec2,
        modifiers: ViewportModifiers,
    },
    Up {
        button: ViewportButton,
        pos: Vec2,
    },
    /// The pointer capture was lost. Any drag in progress must be abandoned;
    /// M6 Task 14 shipped a stuck rubber-band by leaving this case out.
    Cancel,
    /// `delta` is in logical pixels, already reduced from masonry's
    /// line/page/pixel policy by the widget. Positive `y` dollies in.
    Scroll {
        delta: Vec2,
        pos: Vec2,
        modifiers: ViewportModifiers,
    },
    /// A trackpad pinch magnification delta.
    Pinch {
        delta: f32,
    },
    Key {
        key: ViewportKey,
    },
}

/// Maps a widget-local position (logical pixels) into `[0,1]²` across the
/// viewport rect. Deliberately unclamped, and zero-safe.
pub fn normalize_viewport_pos(local: Vec2, size: Vec2) -> Vec2 {
    if size.x <= 0.0 || size.y <= 0.0 {
        return Vec2::ZERO;
    }
    local / size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_normalizes_against_the_viewport_rect() {
        let size = Vec2::new(800.0, 400.0);
        assert_eq!(normalize_viewport_pos(Vec2::ZERO, size), Vec2::ZERO);
        assert_eq!(normalize_viewport_pos(size, size), Vec2::ONE);
        assert_eq!(
            normalize_viewport_pos(Vec2::new(400.0, 100.0), size),
            Vec2::new(0.5, 0.25),
        );
    }

    #[test]
    fn a_drag_outside_the_rect_is_not_clamped() {
        // `capture_pointer` keeps delivering moves past the edge, and orbit
        // reads deltas from them. Clamping here would stall the gesture at
        // the border.
        let size = Vec2::new(100.0, 100.0);
        assert_eq!(
            normalize_viewport_pos(Vec2::new(-50.0, 150.0), size),
            Vec2::new(-0.5, 1.5),
        );
    }

    #[test]
    fn a_zero_sized_viewport_yields_zero_rather_than_nan() {
        // A minimized window delivers (0, 0) here; M6 Task 4 hit the same
        // hazard in the shell. NaN would propagate into every ray this
        // milestone builds.
        let out = normalize_viewport_pos(Vec2::new(10.0, 10.0), Vec2::ZERO);
        assert!(out.is_finite(), "got {out:?}");
        assert_eq!(out, Vec2::ZERO);
    }
}
