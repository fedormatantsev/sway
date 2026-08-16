//! The editor's own camera. Spec M7-3.
//!
//! Navigation is a pure function of four numbers, which is what makes it
//! testable with no window, no app and no render device.

use bevy::prelude::*;
use sway_graph::{ViewportButton, ViewportInput};

/// How far a full-viewport drag turns the camera. Deltas arrive normalized
/// to the viewport rect (spec M7-1), so this is radians per viewport width —
/// a full sweep turns the camera all the way round.
const ORBIT_SENSITIVITY: f32 = std::f32::consts::TAU;
/// Pan distance per viewport width, per unit of `distance`.
const PAN_SENSITIVITY: f32 = 2.0;
/// Dolly is multiplicative so it feels the same at every scale.
const DOLLY_RATE: f32 = 0.15;
/// The pivot can be approached but never reached.
pub const MIN_DISTANCE: f32 = 0.05;
/// Just inside the poles, where the look-at basis degenerates.
const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_2 - 0.001;

/// The editor's viewpoint, as opposed to `SceneCamera`, which is what the
/// show looks through.
///
/// Carries no `EditorPos` and no `DocId` on purpose: `capture_nodes` walks
/// every `EditorPos` entity and `to_document` walks every `DocId` carrier, so
/// this camera is invisible to the graph canvas and to the saved file without
/// either of them needing a special case.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
#[require(Camera3d)]
pub struct EditorCamera {
    pub pivot: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
}

impl Default for EditorCamera {
    fn default() -> Self {
        Self {
            pivot: Vec3::ZERO,
            yaw: 0.0,
            pitch: -0.4,
            distance: 8.0,
        }
    }
}

/// Where the camera sits and what it looks at.
pub fn orbit_transform(cam: &EditorCamera) -> Transform {
    let offset = Vec3::new(
        cam.distance * cam.pitch.cos() * cam.yaw.sin(),
        -cam.distance * cam.pitch.sin(),
        cam.distance * cam.pitch.cos() * cam.yaw.cos(),
    );
    Transform::from_translation(cam.pivot + offset).looking_at(cam.pivot, Vec3::Y)
}

/// Alt + primary drag. `delta` is a normalized-viewport delta.
pub fn orbit(cam: &mut EditorCamera, delta: Vec2) {
    cam.yaw -= delta.x * ORBIT_SENSITIVITY;
    cam.pitch = (cam.pitch - delta.y * ORBIT_SENSITIVITY).clamp(-MAX_PITCH, MAX_PITCH);
}

/// Alt + secondary drag. Moves the pivot across the view plane.
pub fn pan(cam: &mut EditorCamera, delta: Vec2) {
    let tf = orbit_transform(cam);
    let scale = cam.distance * PAN_SENSITIVITY;
    cam.pivot += tf.right().as_vec3() * (-delta.x * scale) + tf.up().as_vec3() * (delta.y * scale);
}

/// Scroll or pinch. Positive dollies in.
pub fn dolly(cam: &mut EditorCamera, amount: f32) {
    cam.distance = (cam.distance * (-amount * DOLLY_RATE).exp()).max(MIN_DISTANCE);
    if !cam.distance.is_finite() {
        cam.distance = MIN_DISTANCE;
    }
}

/// Which navigation gesture is in progress, and where the pointer was last
/// seen. Lives here rather than in the widget: the widget is stateless by
/// design (spec M7-2), because the gesture is resolved where the camera is.
#[derive(Default)]
pub struct NavigationDrag {
    mode: Option<NavigationMode>,
    last: Vec2,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NavigationMode {
    Orbit,
    Pan,
}

/// Spawns the one editor camera. `Startup`, editor builds only.
pub fn spawn_editor_camera(mut commands: Commands) {
    let cam = EditorCamera::default();
    commands.spawn((cam, orbit_transform(&cam)));
}

/// Turns this frame's viewport events into camera motion.
pub fn navigate_editor_camera(
    events: Res<crate::viewport::ViewportEvents>,
    mut drag: Local<NavigationDrag>,
    mut cameras: Query<(&mut EditorCamera, &mut Transform)>,
) {
    if events.0.is_empty() {
        return;
    }
    let Ok((mut cam, mut transform)) = cameras.single_mut() else {
        return;
    };

    let mut changed = false;
    for event in &events.0 {
        match event {
            ViewportInput::Down { button, pos, modifiers } if modifiers.alt => {
                drag.mode = Some(match button {
                    ViewportButton::Primary => NavigationMode::Orbit,
                    ViewportButton::Secondary => NavigationMode::Pan,
                });
                drag.last = *pos;
            }
            ViewportInput::Move { pos, .. } => {
                let Some(mode) = drag.mode else {
                    continue;
                };
                let delta = *pos - drag.last;
                drag.last = *pos;
                match mode {
                    NavigationMode::Orbit => orbit(&mut cam, delta),
                    NavigationMode::Pan => pan(&mut cam, delta),
                }
                changed = true;
            }
            ViewportInput::Up { .. } | ViewportInput::Cancel => drag.mode = None,
            ViewportInput::Scroll { delta, .. } => {
                dolly(&mut cam, delta.y * 0.05);
                changed = true;
            }
            ViewportInput::Pinch { delta } => {
                dolly(&mut cam, *delta * 4.0);
                changed = true;
            }
            _ => {}
        }
    }

    if changed {
        // Never write an equal value (architecture §7).
        let next = orbit_transform(&cam);
        if *transform != next {
            *transform = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_camera() -> EditorCamera {
        EditorCamera::default()
    }

    #[test]
    fn the_camera_looks_at_its_pivot_from_its_distance() {
        let cam = default_camera();
        let tf = orbit_transform(&cam);
        assert!(
            (tf.translation.distance(cam.pivot) - cam.distance).abs() < 1e-4,
            "expected to sit {} from the pivot, sat {}",
            cam.distance,
            tf.translation.distance(cam.pivot),
        );
        // Looking at the pivot means forward points from eye to pivot.
        let forward = tf.forward().as_vec3();
        let to_pivot = (cam.pivot - tf.translation).normalize();
        assert!((forward - to_pivot).length() < 1e-4, "{forward:?} vs {to_pivot:?}");
    }

    #[test]
    fn orbiting_turns_the_camera_without_moving_the_pivot_or_the_distance() {
        let mut cam = default_camera();
        let before = orbit_transform(&cam).translation;
        orbit(&mut cam, Vec2::new(0.25, 0.0));
        let after = orbit_transform(&cam);
        assert_ne!(before, after.translation);
        assert!((after.translation.distance(cam.pivot) - cam.distance).abs() < 1e-4);
        assert_eq!(cam.pivot, Vec3::ZERO);
    }

    #[test]
    fn pitch_stops_just_short_of_the_poles() {
        // At exactly ±90° the look-at basis is degenerate and the view rolls
        // over; every orbit camera clamps for this reason.
        let mut cam = default_camera();
        orbit(&mut cam, Vec2::new(0.0, -100.0));
        assert!(cam.pitch < std::f32::consts::FRAC_PI_2);
        assert!(orbit_transform(&cam).translation.is_finite());

        orbit(&mut cam, Vec2::new(0.0, 200.0));
        assert!(cam.pitch > -std::f32::consts::FRAC_PI_2);
        assert!(orbit_transform(&cam).translation.is_finite());
    }

    #[test]
    fn panning_moves_the_pivot_across_the_view_not_along_the_world_axes() {
        // Pan must feel the same whatever direction the camera faces, which
        // means it moves along the camera's own right/up, not X/Y.
        let mut cam = default_camera();
        cam.yaw = std::f32::consts::FRAC_PI_2;
        let right = orbit_transform(&cam).right().as_vec3();
        pan(&mut cam, Vec2::new(0.1, 0.0));
        let moved = (cam.pivot - Vec3::ZERO).normalize();
        assert!(moved.dot(right).abs() > 0.99, "moved {moved:?}, right {right:?}");
    }

    #[test]
    fn panning_scales_with_distance() {
        // The same drag should cover the same fraction of the screen whether
        // you are close in or far out.
        let mut near = default_camera();
        near.distance = 1.0;
        let mut far = default_camera();
        far.distance = 100.0;
        pan(&mut near, Vec2::new(0.1, 0.0));
        pan(&mut far, Vec2::new(0.1, 0.0));
        assert!(far.pivot.length() > near.pivot.length() * 10.0);
    }

    #[test]
    fn dollying_never_reaches_or_passes_the_pivot() {
        let mut cam = default_camera();
        for _ in 0..1000 {
            dolly(&mut cam, 10.0);
        }
        assert!(cam.distance >= MIN_DISTANCE, "distance {}", cam.distance);
        assert!(cam.distance.is_finite());
    }
}

#[cfg(test)]
mod nav_tests {
    use super::*;
    use crate::viewport::ViewportEvents;
    use sway_graph::{ViewportButton, ViewportInput, ViewportModifiers};

    fn alt() -> ViewportModifiers {
        ViewportModifiers { alt: true, ..Default::default() }
    }

    fn app_with_camera() -> App {
        let mut app = App::new();
        app.init_resource::<ViewportEvents>()
            .add_systems(Update, navigate_editor_camera);
        app.world_mut().spawn((EditorCamera::default(), Transform::default()));
        app
    }

    fn feed(app: &mut App, events: Vec<ViewportInput>) {
        app.world_mut().resource_mut::<ViewportEvents>().0 = events;
        app.update();
    }

    #[test]
    fn alt_drag_orbits() {
        let mut app = app_with_camera();
        feed(&mut app, vec![
            ViewportInput::Down { button: ViewportButton::Primary, pos: Vec2::new(0.5, 0.5), modifiers: alt() },
            ViewportInput::Move { pos: Vec2::new(0.75, 0.5), modifiers: alt() },
        ]);
        let cam = app.world_mut().query::<&EditorCamera>().single(app.world()).unwrap();
        assert_ne!(cam.yaw, EditorCamera::default().yaw);
    }

    #[test]
    fn a_plain_drag_does_not_move_the_camera() {
        // Without Alt the gesture belongs to picking and the gizmo. If this
        // regresses, every click drags the view instead of selecting.
        let mut app = app_with_camera();
        feed(&mut app, vec![
            ViewportInput::Down { button: ViewportButton::Primary, pos: Vec2::new(0.5, 0.5), modifiers: ViewportModifiers::default() },
            ViewportInput::Move { pos: Vec2::new(0.9, 0.9), modifiers: ViewportModifiers::default() },
        ]);
        let cam = *app.world_mut().query::<&EditorCamera>().single(app.world()).unwrap();
        assert_eq!(cam, EditorCamera::default());
    }

    #[test]
    fn a_move_with_no_press_is_ignored() {
        let mut app = app_with_camera();
        feed(&mut app, vec![
            ViewportInput::Move { pos: Vec2::new(0.9, 0.9), modifiers: alt() },
        ]);
        let cam = *app.world_mut().query::<&EditorCamera>().single(app.world()).unwrap();
        assert_eq!(cam, EditorCamera::default());
    }

    #[test]
    fn a_cancel_ends_the_gesture() {
        let mut app = app_with_camera();
        feed(&mut app, vec![
            ViewportInput::Down { button: ViewportButton::Primary, pos: Vec2::new(0.5, 0.5), modifiers: alt() },
            ViewportInput::Cancel,
        ]);
        let before = *app.world_mut().query::<&EditorCamera>().single(app.world()).unwrap();
        feed(&mut app, vec![
            ViewportInput::Move { pos: Vec2::new(0.9, 0.9), modifiers: alt() },
        ]);
        let after = *app.world_mut().query::<&EditorCamera>().single(app.world()).unwrap();
        assert_eq!(before, after, "a cancelled drag must not keep orbiting");
    }

    #[test]
    fn scroll_and_pinch_both_dolly() {
        let mut app = app_with_camera();
        feed(&mut app, vec![ViewportInput::Scroll {
            delta: Vec2::new(0.0, 10.0),
            pos: Vec2::splat(0.5),
            modifiers: ViewportModifiers::default(),
        }]);
        let scrolled = app.world_mut().query::<&EditorCamera>().single(app.world()).unwrap().distance;
        assert_ne!(scrolled, EditorCamera::default().distance);

        feed(&mut app, vec![ViewportInput::Pinch { delta: 0.5 }]);
        let pinched = app.world_mut().query::<&EditorCamera>().single(app.world()).unwrap().distance;
        assert_ne!(pinched, scrolled);
    }

    #[test]
    fn navigating_writes_the_transform() {
        let mut app = app_with_camera();
        feed(&mut app, vec![
            ViewportInput::Down { button: ViewportButton::Primary, pos: Vec2::new(0.5, 0.5), modifiers: alt() },
            ViewportInput::Move { pos: Vec2::new(0.75, 0.5), modifiers: alt() },
        ]);
        let (cam, tf) = app
            .world_mut()
            .query::<(&EditorCamera, &Transform)>()
            .single(app.world())
            .unwrap();
        assert_eq!(*tf, orbit_transform(cam));
    }
}
