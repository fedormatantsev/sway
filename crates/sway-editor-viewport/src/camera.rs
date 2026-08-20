//! The editor's own camera. Spec M7-3.
//!
//! Navigation is a pure function of four numbers, which is what makes it
//! testable with no window, no app and no render device.

use bevy::prelude::*;
use bevy::render::texture::ManualTextureViews;
use sway_graph::graph::{Graph, NodeId};
use sway_runtime::headless::VIEWPORT_HANDLE;
use sway_runtime::nodes::Camera as CameraNode;
use sway_runtime::{CameraTargets, EditorCameraPreview, NodeEntities};
use sway_viewport_input::{ViewportButton, ViewportInput};

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
/// Not a graph node: the editor camera is a viewport tool, so it does not
/// appear on the canvas and is not saved with the document.
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
    events: Res<crate::ViewportEvents>,
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
            ViewportInput::Down {
                button,
                pos,
                modifiers,
            } if modifiers.alt => {
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

/// Which camera the viewport shows: the editor's own, or one of the
/// document's camera nodes.
///
/// A selection rather than the two-state toggle it used to be, because a
/// document may hold any number of cameras and every one of them must be
/// offerable as a preview. Editor state throughout: not a graph value, never
/// reported as a node change, and not persisted with the document — which is
/// why reopening a project shows the editor camera again.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewportCamera {
    #[default]
    Editor,
    /// The camera node being previewed. Falls back to [`Self::Editor`] when
    /// that node leaves the document — see [`settle_viewport_camera`].
    Node(NodeId),
}

impl ViewportCamera {
    /// The camera node being previewed, if any.
    pub fn node(self) -> Option<NodeId> {
        match self {
            Self::Editor => None,
            Self::Node(node) => Some(node),
        }
    }
}

/// Which camera entity is drawing into the viewport this frame, resolved once
/// so the picker and the gizmo do not each re-derive it.
///
/// `None` while the selected camera has not been projected yet — a legitimate
/// state for a frame or two after a reload, not an error.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveViewportCamera(pub Option<Entity>);

/// Falls back to the editor's own camera when the previewed one leaves the
/// document — deleted, or gone after a reload.
///
/// Without this the viewport would hold a `NodeId` naming nothing, and show
/// either a blank pane or, worse, whatever a reused id now points at.
pub fn settle_viewport_camera(graph: Option<Res<Graph>>, mut active: ResMut<ViewportCamera>) {
    let Some(node) = active.node() else {
        return;
    };
    let still_a_camera = graph.is_some_and(|graph| {
        graph
            .get(node)
            .is_some_and(|node| node.value().downcast_ref::<CameraNode>().is_some())
    });
    if !still_a_camera {
        *active = ViewportCamera::Editor;
    }
}

/// Decides which cameras render, and records which one the pane is showing.
///
/// **A camera renders iff it has somewhere to render.** Each camera the graph
/// declares gets a render target of its own, allocated only for a camera
/// something consumes — an output node, a capture node, or this preview. So
/// "has a target" already *is* the question "does anything want this camera's
/// frames", and it is the whole rule for a graph camera. The editor's own
/// camera is the exception: it has no authored resolution and renders into the
/// pane-sized viewport texture, so it draws only while the pane is showing it.
///
/// **Why this is not "exactly one camera is active".** It was, before each
/// camera had a target of its own: every camera pointed at the one viewport
/// texture, so two active cameras overwrote each other and the selection had to
/// switch the losers off. With per-camera targets there is nothing to
/// overwrite, and switching a consumed camera off instead *starves* it — a
/// capture node recording a camera the author was not previewing wrote frame
/// after frame of the default clear colour. The `nodes` spec's "one camera
/// serves several consumers" requires it to keep rendering; the `editor`
/// spec's "exactly one camera is drawing" is about the pane, and the pane's
/// image is chosen by the presenter compositing one target, not by `is_active`.
///
/// `Camera::is_active` is the real off switch: `bevy_render::camera`'s
/// `extract_cameras` checks it first thing and, when false, strips
/// `ExtractedCamera`/`ExtractedView` from the render entity, so the camera
/// driver node never runs a pass — an inactive camera neither draws nor clears.
///
/// Cameras are identified by the node that produced them ([`NodeEntities`]).
/// Anything that is neither the editor's own camera nor a graph camera — the
/// gizmo renderer's overlay camera, above all — is left alone, because it must
/// keep rendering unconditionally. That is the identity-based exclusion that
/// replaced the old "carries no `RenderLayers`" heuristic.
///
/// Runs *after* projection, so it reads the same frame's allocation rather
/// than the previous frame's.
pub fn apply_active_camera(
    active: Res<ViewportCamera>,
    nodes: Option<Res<NodeEntities>>,
    targets: Option<Res<CameraTargets>>,
    mut resolved: ResMut<ActiveViewportCamera>,
    editor_cameras: Query<Entity, With<EditorCamera>>,
    mut cameras: Query<&mut Camera>,
) {
    // Which camera the *pane* shows. Only this drives picking and the gizmo.
    let shown = match *active {
        ViewportCamera::Editor => editor_cameras.iter().next(),
        ViewportCamera::Node(node) => nodes.as_ref().and_then(|nodes| nodes.entity(node)),
    };

    let editor_camera = editor_cameras.iter().next();
    for (node, entity) in nodes.iter().flat_map(|nodes| nodes.iter()) {
        let Ok(mut camera) = cameras.get_mut(entity) else {
            // Not every projected node owns a camera — most own a mesh or
            // nothing at all.
            continue;
        };
        let has_target = targets
            .as_ref()
            .is_some_and(|targets| targets.target(node).is_some());
        // Never write an equal value (architecture §7): `Camera` is extracted
        // every frame and a needless write dirties it.
        if camera.is_active != has_target {
            camera.is_active = has_target;
        }
    }

    if let Some(entity) = editor_camera
        && let Ok(mut camera) = cameras.get_mut(entity)
    {
        let should_be_active = Some(entity) == shown;
        if camera.is_active != should_be_active {
            camera.is_active = should_be_active;
        }
    }

    if resolved.0 != shown {
        resolved.0 = shown;
    }
}

/// The largest rectangle of `aspect`'s ratio that fits inside `pane`, centred.
///
/// Sizes are floored rather than rounded, so the result can never exceed the
/// pane by a pixel: a 641-pixel-wide pane fits 641x360 for a 16:9 camera
/// (design D4), whose aspect of 1.7806 differs from 16:9 by well under a
/// pixel's worth of framing. The alternative — snapping the pane to
/// exact-aspect sizes — would make the preview jitter as the pane is dragged.
///
/// A zero-component pane or aspect yields a zero-size rect at the origin,
/// which allocates no target and draws nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FittedRect {
    /// Top-left corner, relative to the pane's own origin.
    pub offset: UVec2,
    pub size: UVec2,
}

pub fn fit_aspect(pane: UVec2, aspect: UVec2) -> FittedRect {
    if pane.x == 0 || pane.y == 0 || aspect.x == 0 || aspect.y == 0 {
        return FittedRect::default();
    }

    // Compare `pane.x / pane.y` against `aspect.x / aspect.y` without
    // dividing: integers here mean the comparison is exact, and the only
    // rounding in the whole function is the one floor below.
    let pane_is_wider = u64::from(pane.x) * u64::from(aspect.y)
        > u64::from(pane.y) * u64::from(aspect.x);

    let size = if pane_is_wider {
        // Height-bound: use the full height and take the width from it.
        let width = (u64::from(pane.y) * u64::from(aspect.x) / u64::from(aspect.y)) as u32;
        UVec2::new(width.max(1), pane.y)
    } else {
        // Width-bound (and the exactly-equal case, where both give the same
        // answer): use the full width and take the height from it.
        let height = (u64::from(pane.x) * u64::from(aspect.y) / u64::from(aspect.x)) as u32;
        UVec2::new(pane.x, height.max(1))
    };

    FittedRect {
        // Integer division: an odd remainder puts the extra pixel of
        // letterboxing on the bottom/right, consistently.
        offset: (pane - size) / 2,
        size,
    }
}

/// Publishes what the editor is previewing, and at how many pixels.
///
/// The pane's own size is read from the viewport texture's registration
/// (`VIEWPORT_HANDLE`), which the host resizes to the pane every frame — so
/// this needs no second channel from the host to learn it.
///
/// The size published is the *fitted* one: previewing a camera costs the
/// pane's pixels, not the camera's authored resolution, and the authored
/// resolution contributes its aspect ratio only. Where the graph also consumes
/// the camera, the runtime allocates at the authored resolution regardless and
/// the preview samples that target down (design D4) — so this is a floor on
/// the target size, not a demand.
pub fn publish_camera_preview(
    active: Res<ViewportCamera>,
    graph: Option<Res<Graph>>,
    views: Option<Res<ManualTextureViews>>,
    mut preview: ResMut<EditorCameraPreview>,
    mut last_pane: Local<UVec2>,
) {
    if let Some(views) = views.as_ref()
        && let Some(view) = views.get(&VIEWPORT_HANDLE)
    {
        *last_pane = view.size;
    }

    let next = match (active.node(), graph.as_ref()) {
        (Some(node), Some(graph)) => graph
            .get(node)
            .and_then(|node| node.value().downcast_ref::<CameraNode>())
            .map(|camera| EditorCameraPreview {
                node: Some(node),
                size: fit_aspect(*last_pane, camera.inlets.resolution).size,
            })
            .unwrap_or_default(),
        _ => EditorCameraPreview::default(),
    };

    // Never write an equal value: the runtime reallocates a target whenever
    // the size it is asked for changes.
    if *preview != next {
        *preview = next;
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
        assert!(
            (forward - to_pivot).length() < 1e-4,
            "{forward:?} vs {to_pivot:?}"
        );
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
        assert!(
            moved.dot(right).abs() > 0.99,
            "moved {moved:?}, right {right:?}"
        );
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
    fn spawn_editor_camera_inserts_one_editor_camera() {
        let mut app = App::new();
        app.add_systems(Startup, spawn_editor_camera);
        app.update();

        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<EditorCamera>>();
        query
            .single(app.world())
            .expect("spawn_editor_camera should spawn one");
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
mod fit_tests {
    use super::*;

    #[test]
    fn a_pane_wider_than_the_camera_letterboxes_top_and_bottom() {
        // The `editor` spec's own example: 1920x1080 previewed in a 640x480
        // pane occupies a centred 640x360 region.
        let fit = fit_aspect(UVec2::new(640, 480), UVec2::new(1920, 1080));
        assert_eq!(fit.size, UVec2::new(640, 360));
        assert_eq!(fit.offset, UVec2::new(0, 60));
        assert_eq!(fit.offset.y * 2 + fit.size.y, 480, "centred exactly");
    }

    #[test]
    fn an_odd_pane_width_floors_rather_than_overflowing_the_pane() {
        // 641 / (16/9) is 360.5625. Flooring gives design D4's 641x360 — an
        // aspect of 1.7806 rather than 1.7778, below the threshold at which
        // framing is observable — and, unlike rounding, can never produce a
        // rect a pixel wider or taller than the pane it has to fit in.
        let fit = fit_aspect(UVec2::new(641, 480), UVec2::new(1920, 1080));
        assert_eq!(fit.size, UVec2::new(641, 360));
        assert!(fit.offset.x + fit.size.x <= 641);
        assert!(fit.offset.y + fit.size.y <= 480);
    }

    #[test]
    fn a_pane_narrower_than_the_camera_letterboxes_left_and_right() {
        // Taller than 16:9, so the width binds and the bars are vertical.
        let fit = fit_aspect(UVec2::new(400, 400), UVec2::new(1920, 1080));
        assert_eq!(fit.size, UVec2::new(400, 225));
        assert_eq!(fit.offset, UVec2::new(0, 87));

        // And a pane narrower than the camera in the other sense: a portrait
        // pane against a landscape camera is still width-bound.
        let portrait = fit_aspect(UVec2::new(300, 900), UVec2::new(1920, 1080));
        assert_eq!(portrait.size, UVec2::new(300, 168));
        assert_eq!(portrait.offset, UVec2::new(0, 366));
    }

    #[test]
    fn a_pane_taller_than_the_camera_binds_on_height() {
        // A 1:2 camera in a 1000x400 pane: the height binds, so the rect is
        // 200x400 with horizontal bars.
        let fit = fit_aspect(UVec2::new(1000, 400), UVec2::new(1, 2));
        assert_eq!(fit.size, UVec2::new(200, 400));
        assert_eq!(fit.offset, UVec2::new(400, 0));
    }

    #[test]
    fn a_matching_aspect_fills_the_pane_with_no_letterboxing() {
        let fit = fit_aspect(UVec2::new(1280, 720), UVec2::new(1920, 1080));
        assert_eq!(fit.size, UVec2::new(1280, 720));
        assert_eq!(fit.offset, UVec2::ZERO);
    }

    #[test]
    fn a_resolution_change_at_the_same_aspect_changes_nothing() {
        // "Editing a camera's resolution without changing its aspect ratio
        // MUST NOT change the preview at all."
        assert_eq!(
            fit_aspect(UVec2::new(640, 480), UVec2::new(1920, 1080)),
            fit_aspect(UVec2::new(640, 480), UVec2::new(1280, 720))
        );
    }

    #[test]
    fn a_zero_component_fits_nothing_rather_than_dividing_by_zero() {
        assert_eq!(fit_aspect(UVec2::ZERO, UVec2::new(16, 9)), FittedRect::default());
        assert_eq!(
            fit_aspect(UVec2::new(640, 480), UVec2::new(1920, 0)),
            FittedRect::default()
        );
    }
}

#[cfg(test)]
mod active_camera_tests {
    use super::*;
    use sway_graph::graph::Node;
    use sway_runtime::nodes::CameraIn;

    /// A world holding the editor camera, two projected graph cameras and the
    /// gizmo renderer's overlay camera — the shape `apply_active_camera`
    /// actually runs against.
    fn app() -> (App, Entity, NodeId, Entity, NodeId, Entity, Entity) {
        let mut app = App::new();
        app.init_resource::<ViewportCamera>()
            .init_resource::<ActiveViewportCamera>()
            .init_resource::<NodeEntities>()
            .init_resource::<Graph>()
            .add_systems(Update, apply_active_camera);

        let editor = app
            .world_mut()
            .spawn((EditorCamera::default(), Camera::default()))
            .id();

        let spawn_scene_camera = |app: &mut App, resolution: UVec2| {
            let node = app.world_mut().resource_mut::<Graph>().insert(Node::of(
                CameraNode {
                    inlets: CameraIn {
                        resolution,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ));
            let entity = app.world_mut().spawn(Camera::default()).id();
            app.world_mut()
                .resource_mut::<NodeEntities>()
                .insert(node, entity);
            (node, entity)
        };
        let (first, first_entity) = spawn_scene_camera(&mut app, UVec2::new(1920, 1080));
        let (second, second_entity) = spawn_scene_camera(&mut app, UVec2::new(512, 512));

        // The gizmo renderer's own overlay camera: no `EditorCamera`, and no
        // graph node produced it.
        let overlay = app
            .world_mut()
            .spawn(Camera {
                order: 1,
                ..Default::default()
            })
            .id();

        (
            app,
            editor,
            first,
            first_entity,
            second,
            second_entity,
            overlay,
        )
    }

    fn active(app: &App, entity: Entity) -> bool {
        app.world().get::<Camera>(entity).unwrap().is_active
    }

    #[test]
    fn the_pane_shows_exactly_one_camera_and_switching_moves_it() {
        // What the pane shows is one camera at a time, and every camera in the
        // document is offerable. Which cameras *render* is a separate
        // question — a camera the graph consumes keeps rendering into its own
        // target whatever the pane is showing (`consumed_camera_tests`); the
        // pane's image is one target composited by the presenter.
        let (mut app, editor, first, first_entity, second, second_entity, _) = app();

        app.update();
        assert_eq!(
            app.world().resource::<ActiveViewportCamera>().0,
            Some(editor),
            "the editor's own camera by default"
        );
        assert!(active(&app, editor), "and it draws into the pane");

        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Node(first);
        app.update();
        assert_eq!(
            app.world().resource::<ActiveViewportCamera>().0,
            Some(first_entity)
        );
        assert!(
            !active(&app, editor),
            "the editor camera stops drawing into a pane showing something else"
        );

        // Every camera in the document is previewable, not just one.
        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Node(second);
        app.update();
        assert_eq!(
            app.world().resource::<ActiveViewportCamera>().0,
            Some(second_entity)
        );
    }

    #[test]
    fn the_gizmo_overlay_camera_is_never_touched() {
        // Identity, not a `RenderLayers` heuristic: the overlay camera is
        // neither the editor camera nor a camera any graph node produced, so
        // it is not this system's business. If it were deactivated the gizmo
        // would vanish from the screen.
        let (mut app, _editor, first, _, _, _, overlay) = app();
        app.update();
        assert!(active(&app, overlay));

        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Node(first);
        app.update();
        assert!(active(&app, overlay), "an unrelated camera keeps rendering");
    }

    #[test]
    fn a_deleted_camera_falls_back_to_the_editors_own() {
        let (mut app, editor, first, _, _, second_entity, _) = app();
        app.add_systems(Update, settle_viewport_camera.before(apply_active_camera));
        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Node(first);
        app.update();

        app.world_mut().resource_mut::<Graph>().remove(first);
        app.update();

        assert_eq!(
            *app.world().resource::<ViewportCamera>(),
            ViewportCamera::Editor
        );
        assert!(active(&app, editor), "not a blank pane and not a stale image");
        assert!(!active(&app, second_entity));
    }

    #[test]
    fn previewing_a_camera_reports_no_node_as_changed() {
        // Which camera is showing is editor state: switching it must not
        // dirty a node or respawn anything projected.
        let (mut app, _editor, first, _, _, _, _) = app();
        app.update();
        app.world_mut().resource_mut::<Graph>().drain_dirty();

        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Node(first);
        app.update();

        assert!(
            app.world().resource::<Graph>().dirty().next().is_none(),
            "no node was reported as changed"
        );
    }
}

/// What the graph consumes has to keep rendering, whatever the editor is
/// looking at.
///
/// A real device and the real runtime projector, because the question is
/// exactly "does a camera the runtime allocated a target for stay active" —
/// and only the runtime allocates one.
#[cfg(test)]
mod consumed_camera_tests {
    use super::*;
    use bevy::asset::AssetPlugin;
    use bevy::render::renderer::RenderDevice;
    use sway_graph::graph::{Node, Port};
    use sway_runtime::nodes::{Capture, CaptureIn, Output, protocol};
    use sway_runtime::nodes::CameraIn;
    use sway_runtime::{CameraTargets, ProjectionSet, RuntimePlugin};

    fn app() -> App {
        let gpu = sway_gpu::GpuContext::new(None);
        let mut app = App::new();
        app.add_plugins((bevy::app::TaskPoolPlugin::default(), AssetPlugin::default()));
        app.init_asset::<Mesh>();
        app.init_asset::<Image>();
        app.init_asset::<StandardMaterial>();
        app.init_asset::<sway_runtime::SpriteMaterialAsset>();
        app.add_plugins(RuntimePlugin);
        app.insert_resource(RenderDevice::from(gpu.device.clone()))
            .init_resource::<bevy::render::texture::ManualTextureViews>()
            .init_resource::<ViewportCamera>()
            .init_resource::<ActiveViewportCamera>()
            // After projection, so `is_active` is decided from the same
            // frame's allocation rather than the previous one's.
            .add_systems(Update, apply_active_camera.after(ProjectionSet));
        app.world_mut()
            .spawn((EditorCamera::default(), Camera::default()));
        app
    }

    fn add_camera(app: &mut App) -> NodeId {
        app.world_mut()
            .resource_mut::<Graph>()
            .insert(Node::of(CameraNode {
                inlets: CameraIn {
                    resolution: UVec2::new(64, 64),
                    ..Default::default()
                },
                ..Default::default()
            }))
    }

    fn connect(app: &mut App, from: NodeId, to: NodeId) {
        app.world_mut()
            .resource_mut::<Graph>()
            .connect(
                Port::new(from, protocol::CAMERA),
                Port::new(to, protocol::CAMERA),
                0,
            )
            .expect("a camera connects to a consumer");
    }

    fn is_active(app: &App, node: NodeId) -> bool {
        let entity = app
            .world()
            .resource::<sway_runtime::NodeEntities>()
            .entity(node)
            .expect("the camera was projected");
        app.world().get::<Camera>(entity).unwrap().is_active
    }

    #[test]
    fn a_captured_camera_keeps_rendering_while_the_editor_looks_elsewhere() {
        // The bug this pins: a capture node recording a camera the editor is
        // not previewing used to write frame after frame of Bevy's default
        // clear colour, because the camera it reads was switched off. Each
        // camera has a target of its own now, so there is nothing to overwrite
        // and nothing to switch off.
        let mut app = app();
        let camera = add_camera(&mut app);
        let capture = app
            .world_mut()
            .resource_mut::<Graph>()
            .insert(Node::of(Capture {
                inlets: CaptureIn {
                    path: "grabs/frame_####.png".into(),
                    recording: true,
                    ..Default::default()
                },
                ..Default::default()
            }));
        connect(&mut app, camera, capture);

        // The editor is looking through its own camera, as it does by default.
        app.update();
        app.update();

        assert!(
            app.world().resource::<CameraTargets>().target(camera).is_some(),
            "test setup: the capture node should have made the runtime allocate a target"
        );
        assert!(
            is_active(&app, camera),
            "a camera something consumes must keep rendering, or its consumers \
             read an unrendered target"
        );
    }

    #[test]
    fn the_presented_camera_keeps_rendering_too() {
        // Same rule for the output node: the window shows its camera whether
        // or not the editor happens to be previewing that one.
        let mut app = app();
        let camera = add_camera(&mut app);
        let output = app
            .world_mut()
            .resource_mut::<Graph>()
            .insert(Node::of(Output::default()));
        connect(&mut app, camera, output);

        app.update();
        app.update();
        assert!(is_active(&app, camera));
    }

    #[test]
    fn a_camera_nothing_consumes_and_nobody_previews_does_not_render() {
        // It has no target, so there is nowhere for it to draw and no reason
        // to pay for it.
        let mut app = app();
        let camera = add_camera(&mut app);
        app.update();
        app.update();

        assert!(app.world().resource::<CameraTargets>().target(camera).is_none());
        assert!(!is_active(&app, camera));
    }

    #[test]
    fn the_editors_own_camera_stops_rendering_while_a_scene_camera_is_previewed() {
        // The pane shows one image; the editor camera renders into the
        // pane-sized viewport texture, so leaving it on while the pane shows
        // something else is pure waste.
        let mut app = app();
        let camera = add_camera(&mut app);
        let output = app
            .world_mut()
            .resource_mut::<Graph>()
            .insert(Node::of(Output::default()));
        connect(&mut app, camera, output);
        app.update();

        let editor_entity = app
            .world_mut()
            .query_filtered::<Entity, With<EditorCamera>>()
            .single(app.world())
            .expect("one editor camera");
        assert!(app.world().get::<Camera>(editor_entity).unwrap().is_active);

        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Node(camera);
        app.update();
        assert!(!app.world().get::<Camera>(editor_entity).unwrap().is_active);
        assert!(is_active(&app, camera), "and the previewed one still renders");
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    use bevy::render::render_resource::TextureFormat;
    use bevy::render::texture::ManualTextureView;
    use sway_graph::graph::Node;
    use sway_runtime::nodes::CameraIn;

    fn app(pane: UVec2) -> App {
        let gpu = sway_gpu::GpuContext::new(None);
        let texture = sway_gpu::ViewportTexture::new(&gpu.device, pane.x, pane.y);

        let mut app = App::new();
        app.init_resource::<ViewportCamera>()
            .init_resource::<Graph>()
            .init_resource::<EditorCameraPreview>()
            .init_resource::<ManualTextureViews>()
            .add_systems(Update, publish_camera_preview);
        app.world_mut().resource_mut::<ManualTextureViews>().insert(
            VIEWPORT_HANDLE,
            ManualTextureView {
                texture_view: texture.bevy_view.clone().into(),
                size: pane,
                view_format: TextureFormat::Rgba8UnormSrgb,
            },
        );
        // The texture has to outlive the registration for the world to hold a
        // live view; leaking it is what a real host's ownership does anyway.
        std::mem::forget(texture);
        app
    }

    fn add_camera(app: &mut App, resolution: UVec2) -> NodeId {
        app.world_mut()
            .resource_mut::<Graph>()
            .insert(Node::of(CameraNode {
                inlets: CameraIn {
                    resolution,
                    ..Default::default()
                },
                ..Default::default()
            }))
    }

    #[test]
    fn a_preview_asks_for_the_fitted_pane_size_not_the_authored_resolution() {
        // "The preview costs the pane's pixels, not the camera's."
        let mut app = app(UVec2::new(640, 480));
        let camera = add_camera(&mut app, UVec2::new(3840, 2160));
        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Node(camera);
        app.update();

        assert_eq!(
            *app.world().resource::<EditorCameraPreview>(),
            EditorCameraPreview {
                node: Some(camera),
                size: UVec2::new(640, 360),
            }
        );
    }

    #[test]
    fn the_editors_own_camera_claims_nothing() {
        // It has no authored resolution and fills the pane, so there is no
        // target for the runtime to allocate on its behalf.
        let mut app = app(UVec2::new(640, 480));
        add_camera(&mut app, UVec2::new(1920, 1080));
        app.update();
        assert_eq!(
            *app.world().resource::<EditorCameraPreview>(),
            EditorCameraPreview::default()
        );
    }

    #[test]
    fn editing_the_resolution_at_the_same_aspect_asks_for_the_same_size() {
        let mut app = app(UVec2::new(640, 480));
        let camera = add_camera(&mut app, UVec2::new(1920, 1080));
        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Node(camera);
        app.update();
        let before = *app.world().resource::<EditorCameraPreview>();

        app.world_mut()
            .resource_mut::<Graph>()
            .get_mut(camera)
            .unwrap()
            .value_mut()
            .downcast_mut::<CameraNode>()
            .unwrap()
            .inlets
            .resolution = UVec2::new(1280, 720);
        app.update();

        assert_eq!(*app.world().resource::<EditorCameraPreview>(), before);
    }

    #[test]
    fn a_larger_pane_asks_for_more_pixels_at_the_same_aspect() {
        // "Resizing the pane MUST change how many pixels the preview is drawn
        // with, and MUST NOT change what is framed."
        let mut app = app(UVec2::new(640, 480));
        let camera = add_camera(&mut app, UVec2::new(1920, 1080));
        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Node(camera);
        app.update();
        assert_eq!(
            app.world().resource::<EditorCameraPreview>().size,
            UVec2::new(640, 360)
        );

        let gpu = sway_gpu::GpuContext::new(None);
        let bigger = sway_gpu::ViewportTexture::new(&gpu.device, 1280, 960);
        app.world_mut().resource_mut::<ManualTextureViews>().insert(
            VIEWPORT_HANDLE,
            ManualTextureView {
                texture_view: bigger.bevy_view.clone().into(),
                size: UVec2::new(1280, 960),
                view_format: TextureFormat::Rgba8UnormSrgb,
            },
        );
        std::mem::forget(bigger);
        app.update();

        assert_eq!(
            app.world().resource::<EditorCameraPreview>().size,
            UVec2::new(1280, 720),
            "twice the pixels, the same 16:9 framing"
        );
    }
}

#[cfg(test)]
mod nav_tests {
    use super::*;
    use crate::ViewportEvents;
    use sway_viewport_input::{ViewportButton, ViewportInput, ViewportModifiers};

    fn alt() -> ViewportModifiers {
        ViewportModifiers {
            alt: true,
            ..Default::default()
        }
    }

    fn app_with_camera() -> App {
        let mut app = App::new();
        app.init_resource::<ViewportEvents>()
            .add_systems(Update, navigate_editor_camera);
        app.world_mut()
            .spawn((EditorCamera::default(), Transform::default()));
        app
    }

    fn feed(app: &mut App, events: Vec<ViewportInput>) {
        app.world_mut().resource_mut::<ViewportEvents>().0 = events;
        app.update();
    }

    #[test]
    fn alt_drag_orbits() {
        let mut app = app_with_camera();
        feed(
            &mut app,
            vec![
                ViewportInput::Down {
                    button: ViewportButton::Primary,
                    pos: Vec2::new(0.5, 0.5),
                    modifiers: alt(),
                },
                ViewportInput::Move {
                    pos: Vec2::new(0.75, 0.5),
                    modifiers: alt(),
                },
            ],
        );
        let cam = app
            .world_mut()
            .query::<&EditorCamera>()
            .single(app.world())
            .unwrap();
        assert_ne!(cam.yaw, EditorCamera::default().yaw);
    }

    #[test]
    fn a_plain_drag_does_not_move_the_camera() {
        // Without Alt the gesture belongs to picking and the gizmo. If this
        // regresses, every click drags the view instead of selecting.
        let mut app = app_with_camera();
        feed(
            &mut app,
            vec![
                ViewportInput::Down {
                    button: ViewportButton::Primary,
                    pos: Vec2::new(0.5, 0.5),
                    modifiers: ViewportModifiers::default(),
                },
                ViewportInput::Move {
                    pos: Vec2::new(0.9, 0.9),
                    modifiers: ViewportModifiers::default(),
                },
            ],
        );
        let cam = *app
            .world_mut()
            .query::<&EditorCamera>()
            .single(app.world())
            .unwrap();
        assert_eq!(cam, EditorCamera::default());
    }

    #[test]
    fn a_move_with_no_press_is_ignored() {
        let mut app = app_with_camera();
        feed(
            &mut app,
            vec![ViewportInput::Move {
                pos: Vec2::new(0.9, 0.9),
                modifiers: alt(),
            }],
        );
        let cam = *app
            .world_mut()
            .query::<&EditorCamera>()
            .single(app.world())
            .unwrap();
        assert_eq!(cam, EditorCamera::default());
    }

    #[test]
    fn a_cancel_ends_the_gesture() {
        let mut app = app_with_camera();
        feed(
            &mut app,
            vec![
                ViewportInput::Down {
                    button: ViewportButton::Primary,
                    pos: Vec2::new(0.5, 0.5),
                    modifiers: alt(),
                },
                ViewportInput::Cancel,
            ],
        );
        let before = *app
            .world_mut()
            .query::<&EditorCamera>()
            .single(app.world())
            .unwrap();
        feed(
            &mut app,
            vec![ViewportInput::Move {
                pos: Vec2::new(0.9, 0.9),
                modifiers: alt(),
            }],
        );
        let after = *app
            .world_mut()
            .query::<&EditorCamera>()
            .single(app.world())
            .unwrap();
        assert_eq!(before, after, "a cancelled drag must not keep orbiting");
    }

    #[test]
    fn scroll_and_pinch_both_dolly() {
        let mut app = app_with_camera();
        feed(
            &mut app,
            vec![ViewportInput::Scroll {
                delta: Vec2::new(0.0, 10.0),
                pos: Vec2::splat(0.5),
                modifiers: ViewportModifiers::default(),
            }],
        );
        let scrolled = app
            .world_mut()
            .query::<&EditorCamera>()
            .single(app.world())
            .unwrap()
            .distance;
        assert_ne!(scrolled, EditorCamera::default().distance);

        feed(&mut app, vec![ViewportInput::Pinch { delta: 0.5 }]);
        let pinched = app
            .world_mut()
            .query::<&EditorCamera>()
            .single(app.world())
            .unwrap()
            .distance;
        assert_ne!(pinched, scrolled);
    }

    #[test]
    fn navigating_writes_the_transform() {
        let mut app = app_with_camera();
        feed(
            &mut app,
            vec![
                ViewportInput::Down {
                    button: ViewportButton::Primary,
                    pos: Vec2::new(0.5, 0.5),
                    modifiers: alt(),
                },
                ViewportInput::Move {
                    pos: Vec2::new(0.75, 0.5),
                    modifiers: alt(),
                },
            ],
        );
        let (cam, tf) = app
            .world_mut()
            .query::<(&EditorCamera, &Transform)>()
            .single(app.world())
            .unwrap();
        assert_eq!(*tf, orbit_transform(cam));
    }
}
