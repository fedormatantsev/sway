//! Click-to-select. Spec M7-6.

use std::collections::HashSet;

use bevy::camera::Camera;
use bevy::gizmos::transform_gizmo::{TransformGizmoMeshMarker, TransformGizmoRoot};
use bevy::math::Ray3d;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use sway_graph::graph::{Graph, GraphCommand, apply_graph_command};
use sway_graph::{ViewportButton, ViewportInput};

use crate::project::NodeEntities;
use crate::viewport::{ViewportCamera, ViewportCameraRole};

/// Builds a world-space ray from a normalized viewport position.
///
/// `pos` is `[0,1]²` from the top-left (spec M7-1); `Camera::viewport_to_world`
/// wants viewport pixels, and `logical_viewport_size` is what "viewport
/// pixels" means for this camera's own target.
///
/// **Step 1 finding (verified against the pinned `bevy_camera-0.19.0` and
/// `bevy_render-0.19.0` sources, not assumed):** `Camera::viewport_to_world`
/// (`bevy_camera/src/camera.rs:647`) calls `viewport_to_ndc`
/// (`bevy_camera/src/camera.rs:799`), which divides the incoming position by
/// `logical_viewport_rect()`'s size — i.e. by `logical_viewport_size()`
/// (`bevy_camera/src/camera.rs:479`). That in turn is `to_logical` of the
/// physical size, and `Camera::to_logical` (`bevy_camera/src/camera.rs:434`)
/// divides by `computed.target_info.scale_factor`. That field is populated by
/// `bevy_render`'s `camera_system`
/// (`bevy_render/src/camera.rs:394` calling `get_render_target_info` at
/// line 268), and for `NormalizedRenderTarget::TextureView` specifically
/// (`bevy_render/src/camera.rs:294-300`) the `scale_factor` is *hardcoded to
/// `1.0`* — unlike the `Window` and `Image` branches, which read a real DPI
/// scale factor. This crate's viewport always renders to
/// `RenderTarget::TextureView` (see `headless.rs`'s `retarget_cameras`), so
/// for every camera this function is ever called with,
/// `logical_viewport_size() == physical_viewport_size()` (as floats) and
/// `pos * camera.logical_viewport_size()` is an exact pixel position, not an
/// approximation. The brief's given implementation is therefore correct as
/// written and needed no fix.
///
/// `None` covers a camera with no viewport size yet (the first frame, or a
/// zero-sized target) and a degenerate projection. Both are routine, not
/// errors.
pub fn viewport_ray(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    pos: Vec2,
) -> Option<Ray3d> {
    let size = camera.logical_viewport_size()?;
    camera.viewport_to_world(camera_transform, pos * size).ok()
}

/// True for an entity belonging to the transform gizmo's own rendered
/// handles (`TransformGizmoRoot` or `TransformGizmoMeshMarker`, spec M7-8).
fn is_gizmo_mesh(entity: Entity, gizmo_meshes: &HashSet<Entity>) -> bool {
    gizmo_meshes.contains(&entity)
}

/// Selects the mesh under a plain primary press.
///
/// Resolves the hit `Entity` to a `NodeId` through [`NodeEntities`] and
/// writes [`GraphCommand::Select`] — identity only, never a value
/// (`architecture`: Authoring writes reach the world only through the graph).
///
/// `MeshRayCast` is used as a bare `SystemParam`. `MeshPickingPlugin` is
/// deliberately not added: it exists to run `bevy_picking`'s own pointer
/// backend, which needs `bevy_winit` — disabled here. Its `SystemParam`
/// fields are `Res<Assets<Mesh>>`, three `Local`s and two `Query`s, none of
/// which that plugin initialises (spec M7-6).
#[allow(clippy::type_complexity)] // an ECS query filter tuple, not a type to simplify
pub fn pick_on_click(
    events: Res<crate::viewport::ViewportEvents>,
    active: Res<ViewportCamera>,
    cameras: Query<(&Camera, &GlobalTransform, &ViewportCameraRole)>,
    gizmo_state: Option<Res<bevy::gizmos::transform_gizmo::TransformGizmoState>>,
    gizmo_meshes: Query<Entity, Or<(With<TransformGizmoRoot>, With<TransformGizmoMeshMarker>)>>,
    mut ray_cast: MeshRayCast,
    nodes: Res<NodeEntities>,
    mut graph: ResMut<Graph>,
    type_registry: Res<AppTypeRegistry>,
) {
    // A drag on a gizmo handle is not a pick. `Option<Res<...>>` rather than
    // a plain `Res`: `pick_on_click` (Tasks 11-12) predates the gizmo
    // (Tasks 13-15) and does not otherwise depend on it, so this keeps that
    // ordering real — `pick_on_click` still runs correctly (picking is
    // simply never suppressed) if registered anywhere `TransformGizmoState`
    // was never inserted, rather than panicking on a missing resource.
    if gizmo_state.is_some_and(|state| state.active) {
        return;
    }

    // Collected up front: a closure can't borrow both a `Query` and the
    // `MeshRayCast` `SystemParam` at once.
    let gizmo_meshes: HashSet<Entity> = gizmo_meshes.iter().collect();

    for event in &events.0 {
        let ViewportInput::Down {
            button: ViewportButton::Primary,
            pos,
            modifiers,
        } = event
        else {
            continue;
        };
        if modifiers.alt {
            // Alt+drag is navigation (spec M7-3).
            continue;
        }

        let Some((camera, camera_transform)) = cameras.iter().find_map(|(camera, tf, role)| {
            matches!(
                (*active, role),
                (ViewportCamera::Editor, ViewportCameraRole::Editor)
                    | (ViewportCamera::Scene, ViewportCameraRole::Scene)
            )
            .then_some((camera, tf))
        }) else {
            continue;
        };

        let Some(ray) = viewport_ray(camera, camera_transform, *pos) else {
            continue;
        };

        // The gizmo's own handle meshes are `Mesh3d` entities sitting right
        // under the cursor whenever a gizmo is up (spec M7-8, consequence 1).
        let filter = |entity: Entity| !is_gizmo_mesh(entity, &gizmo_meshes);
        let settings = MeshRayCastSettings::default()
            .with_filter(&filter)
            .always_early_exit();

        let hit = ray_cast
            .cast_ray(ray, &settings)
            .first()
            .map(|(entity, _)| *entity);
        let node = hit.and_then(|entity| nodes.node(entity));
        let registry = type_registry.read();
        apply_graph_command(&mut graph, &registry, &GraphCommand::Select { node });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A camera at +Z looking down -Z, with a known viewport size.
    ///
    /// Built through a real headless `App` (the same scaffolding as
    /// `headless.rs`'s own `bevy_render_output_reaches_the_viewport_texture`
    /// test) rather than `Camera::default()`, because `viewport_to_world`
    /// reads `Camera::computed.clip_from_view` and
    /// `computed.target_info.scale_factor`, both of which only Bevy's own
    /// `bevy_render::camera::camera_system` fills in — a bare, unrun `Camera`
    /// has `computed: ComputedCameraValues::default()`, which yields `None`
    /// out of `logical_viewport_size()`. Chosen over the brief's
    /// input-assertion fallback because it is not actually heavier here:
    /// `camera_system` runs in ordinary `PreUpdate`/`PostUpdate` scheduling
    /// before any GPU work, so, unlike `headless.rs`'s pixel-readback test,
    /// this needs no wait for asynchronous pipeline compilation — a handful
    /// of `app.update()` calls populate `computed` deterministically. Getting
    /// the real geometric fixture here also verifies the actual ray math
    /// (the point of Task 11), not just that `viewport_ray` forwards its
    /// arguments unchanged.
    fn test_camera() -> (Camera, GlobalTransform) {
        let gpu = sway_gpu::GpuContext::new(None);
        let size = UVec2::new(4, 4);
        let viewport = sway_gpu::ViewportTexture::new(&gpu.device, size.x, size.y);
        let mut app = crate::headless::build_app(&gpu, &viewport, size, std::env::temp_dir());

        let transform = Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y);
        let entity = app.world_mut().spawn((Camera3d::default(), transform)).id();

        app.finish();
        app.cleanup();
        for _ in 0..5 {
            app.update();
        }

        let world = app.world_mut();
        let camera = world
            .get::<Camera>(entity)
            .expect("camera component")
            .clone();
        let global_transform = *world
            .get::<GlobalTransform>(entity)
            .expect("transform propagation ran");
        assert!(
            camera.logical_viewport_size().is_some(),
            "fixture camera should have a populated viewport size by now",
        );
        (camera, global_transform)
    }

    #[test]
    fn the_centre_of_the_viewport_casts_down_the_camera_forward_axis() {
        let (camera, transform) = test_camera();
        let ray = viewport_ray(&camera, &transform, Vec2::splat(0.5)).expect("a ray");
        let forward = transform.forward().as_vec3();
        assert!(
            ray.direction.as_vec3().dot(forward) > 0.999,
            "centre ray {:?} should point along {forward:?}",
            ray.direction,
        );
    }

    #[test]
    fn the_left_and_right_edges_cast_to_opposite_sides() {
        let (camera, transform) = test_camera();
        let left = viewport_ray(&camera, &transform, Vec2::new(0.0, 0.5)).expect("a ray");
        let right = viewport_ray(&camera, &transform, Vec2::new(1.0, 0.5)).expect("a ray");
        let right_axis = transform.right().as_vec3();
        assert!(left.direction.as_vec3().dot(right_axis) < 0.0);
        assert!(right.direction.as_vec3().dot(right_axis) > 0.0);
    }

    #[test]
    fn a_camera_with_no_viewport_size_yields_no_ray_rather_than_a_panic() {
        let camera = Camera::default();
        let ray = viewport_ray(&camera, &GlobalTransform::default(), Vec2::splat(0.5));
        assert!(ray.is_none());
    }
}

#[cfg(test)]
pub(crate) mod click_tests {
    use super::*;
    use crate::nodes::scene::MeshNode;
    use crate::project::NodeEntities;
    use crate::viewport::{ViewportCamera, ViewportCameraRole};
    use sway_graph::graph::{Graph, Node};
    use sway_graph::{ViewportButton, ViewportInput, ViewportModifiers};

    /// A cube at the origin, a camera looking at it, in a real render-capable
    /// app — `MeshRayCast` needs `Assets<Mesh>` and the `Aabb` that Bevy's own
    /// systems compute, so a bare `World` will not do.
    /// Also wires up a real `ViewportInputRx` channel: `EditorViewportPlugin`
    /// includes `drain_viewport_input`, which unconditionally clears
    /// `ViewportEvents` at the start of every `PreUpdate` and refills it from
    /// the channel only if one exists (see its doc comment). Writing straight
    /// into `ViewportEvents` — as `nav_tests` in `camera.rs` does — would be
    /// wiped by that same-frame clear before `pick_on_click` ever runs in
    /// `PostUpdate`, since this fixture (unlike `nav_tests`'s bare `App`)
    /// registers the whole plugin. Sending through the channel instead is
    /// both correct and the more faithful test: it is genuinely how a click
    /// reaches this system in production.
    pub(crate) fn app_with_a_cube() -> (App, Entity, crossbeam_channel::Sender<ViewportInput>) {
        let gpu = sway_gpu::GpuContext::new(None);
        let size = UVec2::new(64, 64);
        let viewport = sway_gpu::ViewportTexture::new(&gpu.device, size.x, size.y);
        let mut app = crate::headless::build_app(&gpu, &viewport, size, std::env::temp_dir());
        app.add_plugins(crate::viewport::EditorViewportPlugin);
        let (tx, rx) = crossbeam_channel::unbounded();
        app.insert_resource(sway_graph::ViewportInputRx(rx));
        app.finish();
        app.cleanup();

        let cube = {
            let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
            let handle = meshes.add(Cuboid::new(2.0, 2.0, 2.0));
            // `Visibility::default()` is inert here, not load-bearing: it was
            // originally added to work around what looked like a missing
            // `InheritedVisibility`/`ViewVisibility` on `Mesh3d`-only
            // entities, but a phase review traced the real symptom to the
            // channel bug fixed below (`click`'s use of `ViewportInputRx`)
            // and confirmed, by removing this line, that `Mesh3d` already
            // gets `Visibility` (and so `InheritedVisibility`/
            // `ViewVisibility`) via `VisibilityPlugin`'s runtime
            // `register_required_components::<Mesh3d, Visibility>()`
            // (`bevy_camera-0.19.0/src/visibility/mod.rs:500`) — the same
            // mechanism this crate's other `Mesh3d` spawns
            // (`sprite_layer.rs`, `sprite_depth_spike.rs`, `point_cloud.rs`)
            // already rely on. Kept spelled out anyway, purely for
            // readability.
            app.world_mut()
                .spawn((Mesh3d(handle), Transform::default(), Visibility::default()))
                .id()
        };
        app.world_mut().spawn((
            Camera3d::default(),
            ViewportCameraRole::Scene,
            Transform::from_xyz(0.0, 0.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        // Several updates: `Aabb` is computed by a PostUpdate system and the
        // camera's projection is filled in by Bevy's camera systems.
        for _ in 0..4 {
            app.update();
        }
        (app, cube, tx)
    }

    fn bind_cube(app: &mut App, cube: Entity) -> sway_graph::NodeId {
        let node = {
            let mut graph = app.world_mut().resource_mut::<Graph>();
            graph.insert(Node::of(Vec2::ZERO, MeshNode::default()))
        };
        app.world_mut()
            .resource_mut::<NodeEntities>()
            .insert(node, cube);
        node
    }

    fn click(app: &mut App, tx: &crossbeam_channel::Sender<ViewportInput>, pos: Vec2) {
        tx.send(ViewportInput::Down {
            button: ViewportButton::Primary,
            pos,
            modifiers: ViewportModifiers::default(),
        })
        .unwrap();
        app.update();
    }

    #[test]
    fn clicking_a_mesh_selects_it() {
        let (mut app, cube, tx) = app_with_a_cube();
        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Scene;
        let node = bind_cube(&mut app, cube);
        click(&mut app, &tx, Vec2::splat(0.5));
        assert_eq!(app.world().resource::<Graph>().selection(), Some(node));
    }

    #[test]
    fn clicking_empty_space_clears_the_selection() {
        let (mut app, cube, tx) = app_with_a_cube();
        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Scene;
        let node = bind_cube(&mut app, cube);
        app.world_mut()
            .resource_mut::<Graph>()
            .set_selection(Some(node));
        click(&mut app, &tx, Vec2::new(0.02, 0.02));
        assert_eq!(app.world().resource::<Graph>().selection(), None);
    }

    #[test]
    fn an_alt_click_navigates_instead_of_picking() {
        let (mut app, cube, tx) = app_with_a_cube();
        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Scene;
        tx.send(ViewportInput::Down {
            button: ViewportButton::Primary,
            pos: Vec2::splat(0.5),
            modifiers: ViewportModifiers {
                alt: true,
                ..Default::default()
            },
        })
        .unwrap();
        app.update();
        assert_eq!(app.world().resource::<Graph>().selection(), None);
        let _ = cube;
    }
}
