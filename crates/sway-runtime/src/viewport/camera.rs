//! The editor's own camera. Spec M7-3.
//!
//! Navigation is a pure function of four numbers, which is what makes it
//! testable with no window, no app and no render device.

use bevy::camera::visibility::RenderLayers;
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
    commands.spawn((cam, orbit_transform(&cam), ViewportCameraRole::Editor));
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

/// Which camera the viewport shows.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewportCamera {
    #[default]
    Editor,
    Scene,
}

/// Tags a camera as one of the two the toggle switches between.
///
/// A marker rather than a query over `EditorCamera` and `sway_nodes::SceneCamera`
/// because `sway-runtime` does not depend on `sway-nodes` — `sway-app` composes
/// the two. It is also what keeps the gizmo renderer's own overlay camera out
/// of this system's reach.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportCameraRole {
    Editor,
    Scene,
}

/// Sets exactly one camera's `Camera::is_active` per frame: the one whose
/// `ViewportCameraRole` matches the current `ViewportCamera`.
///
/// **Why this is needed (read `retarget_cameras` in `headless.rs` first):**
/// `retarget_cameras` points *every* camera at the one viewport texture each
/// `Update` and never touches `is_active`. With two cameras targeting the
/// same texture, whichever renders last simply overwrites the other's
/// pixels — both would appear to "work" until a second camera existed.
/// `Camera::is_active` is the actual off switch: `bevy_render::camera`'s
/// `extract_cameras` checks `if !camera.is_active` first thing and, when
/// true, removes `ExtractedCamera`/`ExtractedView`/etc. from the render
/// entity and `continue`s past the rest of extraction for that camera —
/// no `ExtractedView` means the camera driver node never runs a render
/// graph pass for it, so an inactive camera neither draws nor clears.
pub fn apply_active_camera(
    active: Res<ViewportCamera>,
    mut cameras: Query<(&ViewportCameraRole, &mut Camera)>,
) {
    for (role, mut camera) in &mut cameras {
        let should_be_active = matches!(
            (*active, role),
            (ViewportCamera::Editor, ViewportCameraRole::Editor)
                | (ViewportCamera::Scene, ViewportCameraRole::Scene)
        );
        // Never write an equal value (architecture §7): `Camera` is extracted
        // every frame and a needless write dirties it.
        if camera.is_active != should_be_active {
            camera.is_active = should_be_active;
        }
    }
}

/// Attaches `ViewportCameraRole::Scene` to any camera the document authored
/// (i.e. any camera `sway-app` didn't already tag itself, such as
/// `sway_nodes::SceneCamera`). Runs every `Update` because a camera can
/// arrive with a reload.
///
/// Excludes the gizmo renderer's own overlay camera, which must stay active
/// unconditionally. `GizmoOverlayCamera` (the marker
/// `bevy_gizmos_render::transform_gizmo_render` tags it with) is private to
/// that crate, so this filters on what is public instead: reading that
/// crate's `spawn_gizmo_meshes` (registered in `Startup`), the overlay camera
/// is spawned with `RenderLayers::layer(15)` (`GIZMO_RENDER_LAYER`) and
/// nothing else in this codebase ever attaches a `RenderLayers` to a camera,
/// so "carries no `RenderLayers`" is a safe, public stand-in for "is not the
/// gizmo overlay camera".
#[allow(clippy::type_complexity)] // an ECS query filter tuple, not a type to simplify
pub fn tag_scene_cameras(
    mut commands: Commands,
    cameras: Query<
        Entity,
        (
            With<Camera>,
            Without<ViewportCameraRole>,
            Without<EditorCamera>,
            Without<RenderLayers>,
        ),
    >,
) {
    for entity in &cameras {
        commands.entity(entity).insert(ViewportCameraRole::Scene);
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
mod active_camera_tests {
    use super::*;

    /// Stands in for `SceneCamera`, which lives in `sway-nodes` — a crate
    /// `sway-runtime` deliberately does not depend on. See `apply_active_camera`.
    ///
    /// Never constructed: it documents the shape being stood in for, not
    /// behaviour under test, so it is `#[allow(dead_code)]` rather than
    /// dropped, to keep `cargo test` output warning-free.
    #[derive(Component)]
    #[allow(dead_code)]
    struct TestSceneCamera;

    #[test]
    fn exactly_one_of_the_two_cameras_is_active_in_either_position() {
        let mut app = App::new();
        app.init_resource::<ViewportCamera>()
            .add_systems(Update, apply_active_camera);
        let editor = app
            .world_mut()
            .spawn((EditorCamera::default(), Camera::default(), ViewportCameraRole::Editor))
            .id();
        let scene = app
            .world_mut()
            .spawn((Camera::default(), ViewportCameraRole::Scene))
            .id();

        app.update();
        assert!(app.world().get::<Camera>(editor).unwrap().is_active);
        assert!(!app.world().get::<Camera>(scene).unwrap().is_active);

        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Scene;
        app.update();
        assert!(!app.world().get::<Camera>(editor).unwrap().is_active);
        assert!(app.world().get::<Camera>(scene).unwrap().is_active);
    }

    #[test]
    fn a_camera_with_no_role_is_left_alone() {
        // The gizmo renderer spawns its own overlay camera (spec M7-8). If
        // this system deactivated every camera it did not recognise, the
        // gizmo would vanish from the screen.
        let mut app = App::new();
        app.init_resource::<ViewportCamera>()
            .add_systems(Update, apply_active_camera);
        let overlay = app.world_mut().spawn(Camera { order: 1, ..Default::default() }).id();

        app.update();
        assert!(
            app.world().get::<Camera>(overlay).unwrap().is_active,
            "an unrelated camera must keep rendering",
        );
    }
}

#[cfg(test)]
mod tag_scene_cameras_tests {
    use super::*;
    use bevy::camera::visibility::RenderLayers;

    #[test]
    fn an_untagged_camera_is_tagged_as_scene() {
        let mut app = App::new();
        app.add_systems(Update, tag_scene_cameras);
        let camera = app.world_mut().spawn(Camera::default()).id();

        app.update();

        assert_eq!(app.world().get::<ViewportCameraRole>(camera), Some(&ViewportCameraRole::Scene));
    }

    #[test]
    fn a_camera_carrying_the_gizmo_render_layer_is_not_tagged() {
        // The gizmo renderer's own overlay camera (spec M7-8, spawned by
        // `bevy_gizmos_render::transform_gizmo_render::spawn_gizmo_meshes` in
        // `Startup`) carries `RenderLayers::layer(15)`
        // (`GIZMO_RENDER_LAYER`). See `tag_scene_cameras`'s doc comment for
        // why that is a safe, public discriminator for "not a scene camera".
        let mut app = App::new();
        app.add_systems(Update, tag_scene_cameras);
        let overlay = app.world_mut().spawn((Camera::default(), RenderLayers::layer(15))).id();

        app.update();

        assert_eq!(app.world().get::<ViewportCameraRole>(overlay), None);
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
