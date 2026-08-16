//! The transform gizmo. Spec M7-8.
//!
//! Bevy 0.19 ships a complete one in `bevy_gizmos::transform_gizmo`, and its
//! renderer is already in this app — `GizmoRenderPlugin::build` (in
//! `bevy_gizmos_render`, wired in via `DefaultPlugins`) adds
//! `TransformGizmoRenderPlugin` whenever `PbrPlugin` is present, gated only on
//! `TransformGizmoSettings` existing. What this module supplies is the half
//! that cannot be reused: `transform_gizmo_hover` and `transform_gizmo_drag`
//! are private and both take `Single<&Window, With<PrimaryWindow>>` plus
//! `ButtonInput<MouseButton>`, and this app has no Bevy window at all. Their
//! geometry, however, is public — `intersect_plane`, `axis_direction`,
//! `point_to_segment_dist` and the rest — so only the input plumbing is
//! rewritten here, against normalized viewport coordinates.

use bevy::camera::visibility::RenderLayers;
use bevy::gizmos::transform_gizmo::{
    TransformGizmoCamera, TransformGizmoFocus, TransformGizmoMeshMarker, TransformGizmoRoot,
};
use bevy::prelude::*;
use sway_graph::{HiddenFromEditor, Selection};

/// Keeps `TransformGizmoFocus` on the selection, and only there.
pub fn follow_selection(
    mut commands: Commands,
    selection: Res<Selection>,
    focused: Query<Entity, With<TransformGizmoFocus>>,
    transforms: Query<(), With<Transform>>,
) {
    // Only an entity with a `Transform` can carry a gizmo: selecting an
    // `Lfo` must leave the viewport alone.
    let wanted = selection.0.filter(|entity| transforms.get(*entity).is_ok());
    for entity in &focused {
        if Some(entity) != wanted {
            commands.entity(entity).remove::<TransformGizmoFocus>();
        }
    }
    if let Some(entity) = wanted
        && focused.get(entity).is_err()
    {
        commands.entity(entity).insert(TransformGizmoFocus);
    }
}

/// Puts `TransformGizmoCamera` on whichever camera is currently rendering.
///
/// Not optional here: the marker may be omitted only when the world holds
/// exactly one camera, and this world holds three — the editor camera, the
/// document's scene camera, and the gizmo renderer's own overlay camera.
pub fn mark_gizmo_camera(
    mut commands: Commands,
    active: Res<crate::viewport::ViewportCamera>,
    cameras: Query<(Entity, &crate::viewport::ViewportCameraRole)>,
    marked: Query<Entity, With<TransformGizmoCamera>>,
) {
    let wanted = cameras.iter().find_map(|(entity, role)| {
        matches!(
            (*active, role),
            (crate::viewport::ViewportCamera::Editor, crate::viewport::ViewportCameraRole::Editor)
                | (crate::viewport::ViewportCamera::Scene, crate::viewport::ViewportCameraRole::Scene)
        )
        .then_some(entity)
    });
    for entity in &marked {
        if Some(entity) != wanted {
            commands.entity(entity).remove::<TransformGizmoCamera>();
        }
    }
    if let Some(entity) = wanted
        && marked.get(entity).is_err()
    {
        commands.entity(entity).insert(TransformGizmoCamera);
    }
}

/// Tags every gizmo mesh entity as [`HiddenFromEditor`] as it appears.
///
/// The renderer spawns `TransformGizmoRoot` and its `TransformGizmoMeshMarker`
/// children once, in `Startup` — but this runs in `Update` rather than
/// ordered after that private system, because ordering against a system this
/// crate cannot name is not possible; `With<T>, Without<HiddenFromEditor>`
/// makes running every frame both correct and (after the first frame) free.
pub fn hide_gizmo_meshes_from_editor(
    mut commands: Commands,
    roots: Query<Entity, (With<TransformGizmoRoot>, Without<HiddenFromEditor>)>,
    meshes: Query<Entity, (With<TransformGizmoMeshMarker>, Without<HiddenFromEditor>)>,
) {
    for entity in roots.iter().chain(meshes.iter()) {
        commands.entity(entity).insert(HiddenFromEditor);
    }
}

/// The gizmo overlay camera's own render layer (`GIZMO_RENDER_LAYER` in
/// `bevy_gizmos_render::transform_gizmo_render`, private to that crate).
/// Nothing else in this codebase attaches a `RenderLayers` to a camera — see
/// `camera::tag_scene_cameras` — so this literal is the same public
/// stand-in used there.
const GIZMO_RENDER_LAYER: usize = 15;

/// Stops the gizmo overlay camera from blanking the scene beneath it.
///
/// `spawn_gizmo_meshes` (`bevy_gizmos_render::transform_gizmo_render`, read
/// against the pinned 0.19.0 source) spawns that camera with
/// `Camera { order: 1, ..Default::default() }` — its doc comment claims the
/// camera draws "without clearing the color buffer", but `Camera::clear_color`
/// is left at `ClearColorConfig::Default`, not `::None`, which reads the
/// global `ClearColor` resource (an opaque dark gray by default). A
/// higher-order camera with `ClearColorConfig::Default` clears the shared
/// render target before drawing, wiping out whatever the scene camera drew
/// underneath it — the doc comment does not match the code. Fixed here by
/// writing `ClearColorConfig::None` onto the camera carrying
/// `RenderLayers::layer(GIZMO_RENDER_LAYER)`, once.
pub fn disable_gizmo_camera_clear(
    mut cameras: Query<(&RenderLayers, &mut Camera), Changed<RenderLayers>>,
) {
    let gizmo_layer = RenderLayers::layer(GIZMO_RENDER_LAYER);
    for (layers, mut camera) in &mut cameras {
        if *layers == gizmo_layer && !matches!(camera.clear_color, ClearColorConfig::None) {
            camera.clear_color = ClearColorConfig::None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::gizmos::transform_gizmo::{TransformGizmoFocus, TransformGizmoSettings};
    use sway_graph::Selection;

    #[test]
    fn the_selection_carries_the_gizmo_focus() {
        let mut app = App::new();
        app.init_resource::<Selection>()
            .add_systems(Update, follow_selection);
        let a = app.world_mut().spawn(Transform::default()).id();
        let b = app.world_mut().spawn(Transform::default()).id();

        app.world_mut().resource_mut::<Selection>().0 = Some(a);
        app.update();
        assert!(app.world().get::<TransformGizmoFocus>(a).is_some());

        app.world_mut().resource_mut::<Selection>().0 = Some(b);
        app.update();
        assert!(app.world().get::<TransformGizmoFocus>(a).is_none(), "only one focus at a time");
        assert!(app.world().get::<TransformGizmoFocus>(b).is_some());

        app.world_mut().resource_mut::<Selection>().0 = None;
        app.update();
        assert!(app.world().get::<TransformGizmoFocus>(b).is_none());
    }

    #[test]
    fn an_entity_with_no_transform_gets_no_focus() {
        // Selecting an `Lfo` must not put a gizmo anywhere.
        let mut app = App::new();
        app.init_resource::<Selection>()
            .add_systems(Update, follow_selection);
        let lfo = app.world_mut().spawn_empty().id();
        app.world_mut().resource_mut::<Selection>().0 = Some(lfo);
        app.update();
        assert!(app.world().get::<TransformGizmoFocus>(lfo).is_none());
    }

    #[test]
    fn the_plugin_initialises_what_the_renderer_needs() {
        let mut app = App::new();
        app.add_plugins(crate::viewport::EditorViewportPlugin);
        assert!(app.world().get_resource::<TransformGizmoSettings>().is_some());
        assert!(app.world().get_resource::<TransformGizmoState>().is_some());
        assert!(
            !app.is_plugin_added::<bevy::gizmos::transform_gizmo::TransformGizmoPlugin>(),
            "its two systems need a Window this app does not have (spec M7-8)",
        );
    }

    #[test]
    fn gizmo_root_and_mesh_entities_are_tagged_hidden_from_editor() {
        let mut app = App::new();
        app.add_systems(Update, hide_gizmo_meshes_from_editor);
        let root = app.world_mut().spawn(TransformGizmoRoot).id();
        let mesh = app
            .world_mut()
            .spawn(TransformGizmoMeshMarker {
                axis: bevy::gizmos::transform_gizmo::TransformGizmoAxis::X,
                mode: bevy::gizmos::transform_gizmo::TransformGizmoMode::Translate,
            })
            .id();
        // An ordinary scene entity, so the system does not tag everything.
        let other = app.world_mut().spawn(Transform::default()).id();

        app.update();

        assert!(app.world().get::<HiddenFromEditor>(root).is_some());
        assert!(app.world().get::<HiddenFromEditor>(mesh).is_some());
        assert!(app.world().get::<HiddenFromEditor>(other).is_none());
    }

    #[test]
    fn the_gizmo_overlay_camera_is_told_not_to_clear() {
        let mut app = App::new();
        app.add_systems(Update, disable_gizmo_camera_clear);
        let overlay = app
            .world_mut()
            .spawn((Camera { order: 1, ..Default::default() }, RenderLayers::layer(GIZMO_RENDER_LAYER)))
            .id();
        // A camera on a different layer must be left alone.
        let scene = app.world_mut().spawn((Camera::default(), RenderLayers::layer(0))).id();

        app.update();

        assert!(matches!(
            app.world().get::<Camera>(overlay).unwrap().clear_color,
            ClearColorConfig::None
        ));
        assert!(!matches!(
            app.world().get::<Camera>(scene).unwrap().clear_color,
            ClearColorConfig::None
        ));
    }

    #[test]
    fn the_active_viewport_camera_carries_the_gizmo_camera_marker() {
        use crate::viewport::{ViewportCamera, ViewportCameraRole};

        let mut app = App::new();
        app.init_resource::<ViewportCamera>()
            .add_systems(Update, mark_gizmo_camera);
        let editor = app.world_mut().spawn(ViewportCameraRole::Editor).id();
        let scene = app.world_mut().spawn(ViewportCameraRole::Scene).id();

        app.update();
        assert!(app.world().get::<TransformGizmoCamera>(editor).is_some());
        assert!(app.world().get::<TransformGizmoCamera>(scene).is_none());

        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Scene;
        app.update();
        assert!(
            app.world().get::<TransformGizmoCamera>(editor).is_none(),
            "only one camera carries the marker at a time",
        );
        assert!(app.world().get::<TransformGizmoCamera>(scene).is_some());
    }
}
