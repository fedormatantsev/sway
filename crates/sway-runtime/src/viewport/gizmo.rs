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
//!
//! `set_gizmo_mode` and `viewport_gizmo_hover` are the mode-key and hover
//! halves of that rewrite. `viewport_gizmo_hover` is a faithful port of
//! Bevy's private `transform_gizmo_hover`
//! (`bevy_gizmos-0.19.0/src/transform_gizmo.rs:282-395`): the same geometry,
//! the same public constants and settings, with only the cursor source
//! changed (a `ViewportInput::Move`/`Down` position converted through
//! `cursor_in_viewport_pixels`, rather than `Window::cursor_position()`) and
//! the "exactly one camera" fallback dropped, since `mark_gizmo_camera`
//! above always keeps exactly one `TransformGizmoCamera` marked.

use bevy::camera::visibility::RenderLayers;
use bevy::gizmos::transform_gizmo::{
    effective_space, gizmo_rotation, point_to_ring_screen_dist, point_to_segment_dist,
    TransformGizmoAxis, TransformGizmoCamera, TransformGizmoFocus, TransformGizmoMeshMarker,
    TransformGizmoMode, TransformGizmoRoot, TransformGizmoSettings, TransformGizmoState,
    AXIS_START_OFFSET, VIEW_CIRCLE_MAJOR, VIEW_RING_MAJOR,
};
use bevy::prelude::*;
use sway_graph::{HiddenFromEditor, Selection, ViewportInput, ViewportKey};

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

/// The cursor in the viewport pixel space `world_to_viewport` reports in.
///
/// Bevy's own gizmo reads `window.cursor_position()`; there is no window
/// here, so the normalized position from the widget is scaled by the
/// camera's own viewport size — the same conversion `viewport_ray` (in
/// `pick.rs`) does.
fn cursor_in_viewport_pixels(camera: &Camera, pos: Vec2) -> Option<Vec2> {
    Some(pos * camera.logical_viewport_size()?)
}

/// Switches `TransformGizmoSettings::mode` on `ViewportKey::{Translate,
/// Rotate, Scale}` (spec M7-9). Bevy's own gizmo deliberately leaves mode
/// switching to the host app — see the module doc on
/// `bevy::gizmos::transform_gizmo` — so there is nothing upstream to reuse
/// here, only the mapping.
pub fn set_gizmo_mode(
    events: Res<crate::viewport::ViewportEvents>,
    mut settings: ResMut<TransformGizmoSettings>,
) {
    for event in &events.0 {
        let ViewportInput::Key { key } = event else {
            continue;
        };
        let mode = match key {
            ViewportKey::Translate => TransformGizmoMode::Translate,
            ViewportKey::Rotate => TransformGizmoMode::Rotate,
            ViewportKey::Scale => TransformGizmoMode::Scale,
        };
        if settings.mode != mode {
            settings.mode = mode;
        }
    }
}

/// Which handle is under the cursor. A port of Bevy's private
/// `transform_gizmo_hover` with the window removed; the geometry — `scale`
/// from `screen_scale_factor`, `point_to_segment_dist` for translate/scale
/// handles, `point_to_ring_screen_dist` for rotate rings and the view
/// handle — is Bevy's own, unchanged.
pub fn viewport_gizmo_hover(
    events: Res<crate::viewport::ViewportEvents>,
    focus: Query<&GlobalTransform, With<TransformGizmoFocus>>,
    cameras: Query<(&Camera, &GlobalTransform), With<TransformGizmoCamera>>,
    settings: Res<TransformGizmoSettings>,
    mut state: ResMut<TransformGizmoState>,
) {
    // Bevy's own hover system returns early here too: recomputing while a
    // drag is in progress would change the hovered axis out from under the
    // cursor mid-drag.
    if state.active {
        return;
    }
    let Some(pos) = events.0.iter().rev().find_map(|event| match event {
        ViewportInput::Move { pos, .. } | ViewportInput::Down { pos, .. } => Some(*pos),
        _ => None,
    }) else {
        return;
    };
    let Some(global_tf) = focus.iter().next() else {
        state.hovered_axis = None;
        return;
    };
    let Some((camera, cam_tf)) = cameras.iter().next() else {
        state.hovered_axis = None;
        return;
    };
    let Some(cursor) = cursor_in_viewport_pixels(camera, pos) else {
        state.hovered_axis = None;
        return;
    };

    let gizmo_pos = global_tf.translation();
    let space = effective_space(&settings);
    let rotation = gizmo_rotation(global_tf, space);

    let scale = if settings.screen_scale_factor > 0.0 {
        (cam_tf.translation() - gizmo_pos).length() * settings.screen_scale_factor
    } else {
        1.0
    };

    let axes = [
        (TransformGizmoAxis::X, rotation * Vec3::X),
        (TransformGizmoAxis::Y, rotation * Vec3::Y),
        (TransformGizmoAxis::Z, rotation * Vec3::Z),
    ];

    let mut best_axis = None;
    let mut best_dist = f32::MAX;
    let threshold = settings.axis_hit_distance;

    for (axis, dir) in &axes {
        let dist = match settings.mode {
            TransformGizmoMode::Translate | TransformGizmoMode::Scale => {
                let start = gizmo_pos + *dir * (AXIS_START_OFFSET * scale);
                let endpoint = gizmo_pos + *dir * (settings.axis_length * scale);
                let Some(start_screen) = camera.world_to_viewport(cam_tf, start).ok() else {
                    continue;
                };
                let Some(end_screen) = camera.world_to_viewport(cam_tf, endpoint).ok() else {
                    continue;
                };
                point_to_segment_dist(cursor, start_screen, end_screen)
            }
            TransformGizmoMode::Rotate => point_to_ring_screen_dist(
                cursor,
                camera,
                cam_tf,
                gizmo_pos,
                *dir,
                settings.rotate_ring_radius * scale,
            ),
        };
        if dist < threshold && dist < best_dist {
            best_dist = dist;
            best_axis = Some(*axis);
        }
    }

    // The view-plane / view-axis handle. Falls out of the ported geometry
    // for free — same camera, same `cursor` — so it is kept rather than
    // dropped.
    let view_dist = match settings.mode {
        TransformGizmoMode::Translate => {
            if let Ok(center_screen) = camera.world_to_viewport(cam_tf, gizmo_pos) {
                let screen_radius = VIEW_CIRCLE_MAJOR * scale;
                let edge_world = gizmo_pos + cam_tf.right() * screen_radius;
                if let Ok(edge_screen) = camera.world_to_viewport(cam_tf, edge_world) {
                    let r = (edge_screen - center_screen).length();
                    let d = (cursor - center_screen).length();
                    (d - r).abs()
                } else {
                    f32::MAX
                }
            } else {
                f32::MAX
            }
        }
        TransformGizmoMode::Rotate => {
            let cam_forward = cam_tf.forward().as_vec3();
            point_to_ring_screen_dist(
                cursor,
                camera,
                cam_tf,
                gizmo_pos,
                cam_forward,
                VIEW_RING_MAJOR * scale,
            )
        }
        TransformGizmoMode::Scale => f32::MAX,
    };

    if view_dist < threshold && view_dist < best_dist {
        best_axis = Some(TransformGizmoAxis::View);
    }

    state.hovered_axis = best_axis;
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The `Sender` half of `app_with_a_cube`'s channel, stashed as a
    /// resource so `hover` can reach it without widening
    /// `app_with_a_focused_gizmo`'s return type past `(App, Entity)`.
    /// `drain_viewport_input` (wired in by `EditorViewportPlugin`, which
    /// `app_with_a_cube` adds) unconditionally clears `ViewportEvents` every
    /// `PreUpdate` — see its doc comment — so writing straight into that
    /// resource before `app.update()` would be wiped before
    /// `viewport_gizmo_hover` ever ran; sending through the channel instead
    /// is the same fix `pick.rs`'s own `click_tests` already documents.
    #[derive(Resource)]
    struct HoverChannel(crossbeam_channel::Sender<ViewportInput>);

    /// Builds on `pick::click_tests::app_with_a_cube`: the cube becomes the
    /// gizmo focus by way of `Selection` (exactly how the real editor drives
    /// it — `follow_selection` does the rest), and the scene camera gets
    /// `TransformGizmoCamera` by way of `ViewportCamera` (`mark_gizmo_camera`
    /// does the rest). The cube sits at the origin; the camera looks at it
    /// from `(0, 0, 10)` down `-Z` with `+Y` up, so world `+X` (the X handle)
    /// reads as screen-right of centre and world `+Y` (the Y handle) as
    /// screen-up — a deterministic, camera-aligned layout to hover-test
    /// against.
    fn app_with_a_focused_gizmo() -> (App, Entity) {
        use crate::viewport::ViewportCamera;
        use crate::viewport::pick::click_tests::app_with_a_cube;

        let (mut app, cube, tx) = app_with_a_cube();
        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Scene;
        app.world_mut().resource_mut::<Selection>().0 = Some(cube);
        app.update();
        assert!(
            app.world().get::<TransformGizmoFocus>(cube).is_some(),
            "follow_selection should have focused the cube",
        );
        assert!(
            app.world().get::<TransformGizmoCamera>(cube).is_none(),
            "the marker belongs on the camera, not the cube",
        );
        app.insert_resource(HoverChannel(tx));
        (app, cube)
    }

    /// Delivers one `ViewportInput::Move` at `pos` and lets it reach
    /// `viewport_gizmo_hover` — see `HoverChannel`'s doc comment for why this
    /// goes through the channel rather than `ViewportEvents` directly.
    fn hover(app: &mut App, pos: Vec2) {
        app.world()
            .resource::<HoverChannel>()
            .0
            .send(ViewportInput::Move { pos, modifiers: sway_graph::ViewportModifiers::default() })
            .unwrap();
        app.update();
    }

    #[test]
    fn the_mode_keys_switch_modes() {
        let mut app = App::new();
        app.init_resource::<crate::viewport::ViewportEvents>()
            .init_resource::<TransformGizmoSettings>()
            .add_systems(Update, set_gizmo_mode);

        for (key, expected) in [
            (ViewportKey::Rotate, TransformGizmoMode::Rotate),
            (ViewportKey::Scale, TransformGizmoMode::Scale),
            (ViewportKey::Translate, TransformGizmoMode::Translate),
        ] {
            app.world_mut().resource_mut::<crate::viewport::ViewportEvents>().0 =
                vec![ViewportInput::Key { key }];
            app.update();
            assert_eq!(app.world().resource::<TransformGizmoSettings>().mode, expected);
        }
    }

    #[test]
    fn hovering_an_axis_reports_it() {
        // A gizmo at the origin, a camera on +Z: a cursor to the right of centre
        // must land on the X handle and nothing else.
        let (mut app, _focus) = app_with_a_focused_gizmo();
        hover(&mut app, Vec2::new(0.62, 0.5));
        assert_eq!(
            app.world().resource::<TransformGizmoState>().hovered_axis,
            Some(TransformGizmoAxis::X),
        );
    }

    #[test]
    fn hovering_empty_space_reports_nothing() {
        let (mut app, _focus) = app_with_a_focused_gizmo();
        hover(&mut app, Vec2::new(0.05, 0.95));
        assert_eq!(app.world().resource::<TransformGizmoState>().hovered_axis, None);
    }

    #[test]
    fn hover_is_frozen_during_a_drag() {
        // Bevy's own hover system returns early when `state.active`; ours must
        // too, or the axis would change under the cursor mid-drag.
        let (mut app, _focus) = app_with_a_focused_gizmo();
        app.world_mut().resource_mut::<TransformGizmoState>().active = true;
        app.world_mut().resource_mut::<TransformGizmoState>().hovered_axis = Some(TransformGizmoAxis::Y);
        hover(&mut app, Vec2::new(0.62, 0.5));
        assert_eq!(
            app.world().resource::<TransformGizmoState>().hovered_axis,
            Some(TransformGizmoAxis::Y),
        );
    }
}
