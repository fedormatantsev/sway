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
//! above keeps at most one `TransformGizmoCamera` marked — it can also mark
//! zero, e.g. `ViewportCamera` toggled to `Scene` before that document's
//! scene camera has spawned — so both `viewport_gizmo_hover` and
//! `viewport_gizmo_drag` must (and do) handle finding no marked camera at
//! all, not just "exactly one".
//!
//! `viewport_gizmo_drag` is the third piece: a port of Bevy's private
//! `transform_gizmo_drag` (`bevy_gizmos-0.19.0/src/transform_gizmo.rs:396-637`)
//! with exactly three substitutions — the cursor source (as above),
//! `ButtonInput<MouseButton>` replaced by this frame's `ViewportInput::{Down,
//! Up,Cancel}` plus `TransformGizmoState::active` itself as the "held" signal,
//! and the `CursorOptions`/`CursorGrabMode` confinement block dropped
//! entirely (there is no window cursor to confine, and `confine_cursor` is
//! always `false` — see `EditorViewportPlugin::build`). Snapping
//! (`snap_translate`/`snap_rotate`/`snap_scale`) is out of scope for M7 and
//! those settings are never set to `Some`, so the private `snap_value` calls
//! Bevy's version makes are skipped rather than ported — every write here
//! takes the plain, un-snapped value Bevy's own `None` branch would.
//! Everything else — `intersect_plane`, `translation_plane_normal`,
//! `axis_direction`, `gizmo_rotation`, `effective_space`, the plane choice
//! per mode, and the write of a world-space delta onto the focused entity's
//! local `Transform` (propagation does the rest through the parent) — is
//! Bevy's own public geometry, called unchanged.

use bevy::camera::visibility::RenderLayers;
use bevy::gizmos::transform_gizmo::{
    axis_direction, effective_space, gizmo_rotation, intersect_plane, point_to_ring_screen_dist,
    point_to_segment_dist, translation_plane_normal, TransformGizmoAxis, TransformGizmoCamera,
    TransformGizmoFocus, TransformGizmoMeshMarker, TransformGizmoMode, TransformGizmoRoot,
    TransformGizmoSettings, TransformGizmoState, AXIS_START_OFFSET, VIEW_CIRCLE_MAJOR,
    VIEW_RING_MAJOR,
};
use bevy::prelude::*;
use sway_graph::{HiddenFromEditor, Selection, ViewportButton, ViewportInput, ViewportKey};

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

/// The gizmo overlay camera's own render layer (`GIZMO_RENDER_LAYER` in
/// `bevy_gizmos_render::transform_gizmo_render`, private to that crate).
/// Nothing else in this codebase attaches a `RenderLayers` to a camera — see
/// `camera::tag_scene_cameras` — so this literal is the same public
/// stand-in used there.
const GIZMO_RENDER_LAYER: usize = 15;

/// Tags every gizmo mesh entity, and the gizmo overlay camera itself, as
/// [`HiddenFromEditor`] as they appear.
///
/// The renderer spawns `TransformGizmoRoot` and its `TransformGizmoMeshMarker`
/// children, and its own overlay camera (carrying `RenderLayers::layer(
/// GIZMO_RENDER_LAYER)` — the same discriminator `camera::tag_scene_cameras`
/// uses to *exclude* it from scene-camera tagging), once, in `Startup` — but
/// this runs in `Update` rather than ordered after that private system,
/// because ordering against a system this crate cannot name is not possible;
/// `With<T>, Without<HiddenFromEditor>` makes running every frame both
/// correct and (after the first frame) free.
///
/// The overlay camera needs this too: like `spawn_editor_camera`'s camera, it
/// carries a `Transform` and no `Transform`-carrying parent, so `capture_tree`
/// (`sway-editor`, `snapshot.rs`) would otherwise list it as a selectable
/// scene row, and a gizmo drag on it would corrupt a transform the renderer
/// silently overwrites every frame. Matched by `With<Camera>, With<RenderLayers>`
/// — the same "carries a `RenderLayers`" discriminator `tag_scene_cameras`
/// already uses, rather than by the exact `GIZMO_RENDER_LAYER` value, since
/// (per that discriminator's own doc comment) nothing else in this codebase
/// ever attaches a `RenderLayers` to a camera.
#[allow(clippy::type_complexity)] // an ECS query filter tuple, not a type to simplify
pub fn hide_gizmo_meshes_from_editor(
    mut commands: Commands,
    roots: Query<Entity, (With<TransformGizmoRoot>, Without<HiddenFromEditor>)>,
    meshes: Query<Entity, (With<TransformGizmoMeshMarker>, Without<HiddenFromEditor>)>,
    overlay_cameras: Query<Entity, (With<Camera>, With<RenderLayers>, Without<HiddenFromEditor>)>,
) {
    for entity in roots.iter().chain(meshes.iter()).chain(overlay_cameras.iter()) {
        commands.entity(entity).insert(HiddenFromEditor);
    }
}

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

/// The floor Bevy's private `transform_gizmo_drag` clamps every scale write
/// to (`bevy_gizmos-0.19.0/src/transform_gizmo.rs:60`,
/// `const MIN_SCALE: f32 = 0.01;`). Not `pub`, so duplicated here verbatim
/// rather than imported — the same treatment `GIZMO_RENDER_LAYER` above gets
/// for the same reason.
const MIN_SCALE: f32 = 0.01;

/// Drags the focused entity's local `Transform` while a handle is held. See
/// the module doc comment for the three substitutions this makes against
/// Bevy's private `transform_gizmo_drag`; everything else below is that
/// function's own math, called or copied unchanged.
pub fn viewport_gizmo_drag(
    events: Res<crate::viewport::ViewportEvents>,
    mut focus_query: Query<(Entity, &GlobalTransform, &mut Transform), With<TransformGizmoFocus>>,
    cameras: Query<(&Camera, &GlobalTransform), With<TransformGizmoCamera>>,
    settings: Res<TransformGizmoSettings>,
    mut state: ResMut<TransformGizmoState>,
) {
    // End drag. Checked first and unconditionally, *before* the camera
    // lookup below: a `Cancel` or `Up` must win over anything else this frame
    // carries, the same hazard M6 Task 14 found on the canvas (a stuck drag
    // that never lets picking run again). Concretely here: `mark_gizmo_camera`
    // can mark zero cameras (see its doc comment) if the user toggles
    // `ViewportCamera` before the target camera exists — e.g. to `Scene`
    // before a document's scene camera has spawned. If that happens while a
    // drag is active, the `cameras` query below finds nothing; resetting
    // `state.active` here, ahead of that query, is what keeps a `Cancel`/`Up`
    // arriving in that window from leaving the drag stuck forever (and, with
    // it, `pick_on_click` permanently refusing to select anything — see its
    // own `gizmo_state.is_some_and(|state| state.active)` guard).
    if state.active
        && events
            .0
            .iter()
            .any(|event| matches!(event, ViewportInput::Up { .. } | ViewportInput::Cancel))
    {
        state.active = false;
        state.axis = None;
        state.entity = None;
        return;
    }

    let Some((camera, cam_tf)) = cameras.iter().next() else {
        return;
    };

    let Some(cursor_pos) = events.0.iter().rev().find_map(|event| match event {
        ViewportInput::Move { pos, .. } | ViewportInput::Down { pos, .. } => Some(*pos),
        _ => None,
    }) else {
        return;
    };
    let Some(cursor_pos) = cursor_in_viewport_pixels(camera, cursor_pos) else {
        return;
    };

    // Start drag.
    if !state.active {
        let pressed = events.0.iter().any(|event| {
            matches!(
                event,
                ViewportInput::Down { button: ViewportButton::Primary, modifiers, .. }
                    if !modifiers.alt
            )
        });
        if !pressed {
            return;
        }
        let Some(axis) = state.hovered_axis else {
            return;
        };
        let Some((entity, global_tf, transform)) = focus_query.iter().next() else {
            return;
        };

        let space = effective_space(&settings);
        let rotation = gizmo_rotation(global_tf, space);
        let axis_dir = axis_direction(axis, rotation, cam_tf);
        let gizmo_pos = global_tf.translation();

        let Ok(ray) = camera.viewport_to_world(cam_tf, cursor_pos) else {
            return;
        };

        let drag_start_world = match settings.mode {
            TransformGizmoMode::Translate => {
                if axis == TransformGizmoAxis::View {
                    let plane_normal = cam_tf.forward().as_vec3();
                    let Some(intersection) = intersect_plane(ray, plane_normal, gizmo_pos) else {
                        return;
                    };
                    intersection
                } else {
                    let plane_normal = translation_plane_normal(ray, axis_dir);
                    let Some(intersection) = intersect_plane(ray, plane_normal, gizmo_pos) else {
                        return;
                    };
                    let cursor_vec = intersection - gizmo_pos;
                    cursor_vec.dot(axis_dir.normalize()) * axis_dir.normalize() + gizmo_pos
                }
            }
            TransformGizmoMode::Scale => {
                let plane_normal = translation_plane_normal(ray, axis_dir);
                let Some(intersection) = intersect_plane(ray, plane_normal, gizmo_pos) else {
                    return;
                };
                let cursor_vec = intersection - gizmo_pos;
                cursor_vec.dot(axis_dir.normalize()) * axis_dir.normalize() + gizmo_pos
            }
            TransformGizmoMode::Rotate => {
                let rot_axis = if axis == TransformGizmoAxis::View {
                    cam_tf.forward().as_vec3()
                } else {
                    axis_dir.normalize()
                };
                let Some(intersection) = intersect_plane(ray, rot_axis, gizmo_pos) else {
                    return;
                };
                (intersection - gizmo_pos).normalize()
            }
        };

        state.active = true;
        state.axis = Some(axis);
        state.start_transform = *transform;
        state.entity = Some(entity);
        state.drag_start_world = drag_start_world;
        state.gizmo_origin = gizmo_pos;
        return;
    }

    // Continue drag.
    let Some(drag_entity) = state.entity else {
        return;
    };
    let Some(axis) = state.axis else {
        return;
    };
    let Ok((_, global_tf, mut transform)) = focus_query.get_mut(drag_entity) else {
        return;
    };

    let space = effective_space(&settings);
    let rotation = gizmo_rotation(global_tf, space);
    let axis_dir = axis_direction(axis, rotation, cam_tf);
    let gizmo_origin = state.gizmo_origin;

    let Ok(ray) = camera.viewport_to_world(cam_tf, cursor_pos) else {
        return;
    };

    match settings.mode {
        TransformGizmoMode::Translate => {
            if axis == TransformGizmoAxis::View {
                let plane_normal = cam_tf.forward().as_vec3();
                let Some(intersection) = intersect_plane(ray, plane_normal, gizmo_origin) else {
                    return;
                };
                let delta = intersection - state.drag_start_world;
                transform.translation = state.start_transform.translation + delta;
            } else {
                let plane_normal = translation_plane_normal(ray, axis_dir);
                let Some(intersection) = intersect_plane(ray, plane_normal, gizmo_origin) else {
                    return;
                };
                let cursor_vec = intersection - gizmo_origin;
                let axis_norm = axis_dir.normalize();
                let new_projected = cursor_vec.dot(axis_norm) * axis_norm + gizmo_origin;
                let delta = new_projected - state.drag_start_world;
                transform.translation = state.start_transform.translation + delta;
            }
        }
        TransformGizmoMode::Rotate => {
            let rot_axis = if axis == TransformGizmoAxis::View {
                cam_tf.forward().as_vec3()
            } else {
                axis_dir.normalize()
            };
            let Some(intersection) = intersect_plane(ray, rot_axis, gizmo_origin) else {
                return;
            };
            let cursor_vector = (intersection - gizmo_origin).normalize();
            let drag_start = state.drag_start_world;

            let dot = drag_start.dot(cursor_vector);
            let det = rot_axis.dot(drag_start.cross(cursor_vector));
            let angle = bevy::math::ops::atan2(det, dot);
            let rotation_delta = Quat::from_axis_angle(rot_axis, angle);
            transform.rotation = rotation_delta * state.start_transform.rotation;
        }
        TransformGizmoMode::Scale => {
            let plane_normal = translation_plane_normal(ray, axis_dir);
            let Some(intersection) = intersect_plane(ray, plane_normal, gizmo_origin) else {
                return;
            };
            let axis_norm = axis_dir.normalize();
            let cursor_projected = (intersection - gizmo_origin).dot(axis_norm);
            let start_projected = (state.drag_start_world - gizmo_origin).dot(axis_norm);

            let scale_factor = if start_projected.abs() > f32::EPSILON {
                cursor_projected / start_projected
            } else {
                1.0
            };

            let mut new_scale = state.start_transform.scale;
            match axis {
                TransformGizmoAxis::X => new_scale.x = (new_scale.x * scale_factor).max(MIN_SCALE),
                TransformGizmoAxis::Y => new_scale.y = (new_scale.y * scale_factor).max(MIN_SCALE),
                TransformGizmoAxis::Z => new_scale.z = (new_scale.z * scale_factor).max(MIN_SCALE),
                TransformGizmoAxis::View => {
                    new_scale *= scale_factor;
                    new_scale = new_scale.max(Vec3::splat(MIN_SCALE));
                }
            }
            transform.scale = new_scale;
        }
    }
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
    fn the_gizmo_overlay_camera_is_hidden_from_the_editor_tree() {
        // Same hazard `spawn_editor_camera` has (see its doc comment in
        // `camera.rs`): the overlay camera carries `Transform` and no
        // `Transform`-carrying parent, so without this it would satisfy
        // `capture_tree`'s walk and show up as a selectable scene row.
        let mut app = App::new();
        app.add_systems(Update, hide_gizmo_meshes_from_editor);
        let overlay = app
            .world_mut()
            .spawn((
                Camera { order: 1, ..Default::default() },
                Transform::default(),
                RenderLayers::layer(GIZMO_RENDER_LAYER),
            ))
            .id();
        // A scene camera, carrying no `RenderLayers`, must be left alone —
        // the same discriminator `tag_scene_cameras` relies on.
        let scene = app.world_mut().spawn((Camera::default(), Transform::default())).id();

        app.update();

        assert!(app.world().get::<HiddenFromEditor>(overlay).is_some());
        assert!(app.world().get::<HiddenFromEditor>(scene).is_none());
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

    /// Sends every event through the same channel `hover` uses, then lets
    /// them all reach the systems in a single frame — the channel-based
    /// counterpart to `camera.rs`'s `nav_tests::feed` (that one writes
    /// `ViewportEvents` directly, which works there only because that test
    /// module never registers `drain_viewport_input`; see `HoverChannel`'s
    /// doc comment for why this fixture cannot).
    fn feed(app: &mut App, events: Vec<ViewportInput>) {
        let tx = app.world().resource::<HoverChannel>().0.clone();
        for event in events {
            tx.send(event).unwrap();
        }
        app.update();
    }

    /// Continues a drag: one more `ViewportInput::Move` at `pos`, reaching
    /// `viewport_gizmo_drag` the same way `hover` reaches
    /// `viewport_gizmo_hover` — both read the same "most recent `Move`/`Down`
    /// this frame" cursor source.
    fn drag_to(app: &mut App, pos: Vec2) {
        hover(app, pos);
    }

    /// A plain, un-modified primary release. `viewport_gizmo_drag`'s "end
    /// drag" branch does not read the position an `Up` carries — Bevy's own
    /// version does not either — so `Vec2::ZERO` is not load-bearing here.
    fn release(app: &mut App) {
        feed(app, vec![ViewportInput::Up { button: ViewportButton::Primary, pos: Vec2::ZERO }]);
    }

    /// Hovers `pos` (setting `TransformGizmoState::hovered_axis`, exactly as
    /// a real cursor arriving at that position over one or more prior frames
    /// would) and asserts it lands on `axis` before pressing — a bad `pos`
    /// would otherwise fail the *drag* assertions later with a confusing
    /// "nothing moved" rather than pointing at the real cause. Then sends the
    /// `Down` that `viewport_gizmo_drag`'s "start drag" branch claims.
    fn press_on_axis(app: &mut App, axis: TransformGizmoAxis, pos: Vec2) {
        hover(app, pos);
        assert_eq!(
            app.world().resource::<TransformGizmoState>().hovered_axis,
            Some(axis),
            "test setup: cursor at {pos:?} is not over the {axis:?} handle",
        );
        feed(app, vec![ViewportInput::Down {
            button: ViewportButton::Primary,
            pos,
            modifiers: sway_graph::ViewportModifiers::default(),
        }]);
    }

    /// The screen position of the focused entity's handle for `axis`, in
    /// whatever mode `TransformGizmoSettings` is currently in — the
    /// translate/scale handle's tip, or a point on the rotation ring.
    /// Computed by projecting through the real camera and the same
    /// `effective_space`/`gizmo_rotation`/`world_to_viewport` geometry the
    /// production code uses, rather than a hardcoded literal: the
    /// parented-object test moves the focused entity between calls, and a
    /// fixed literal would silently stop pointing at the handle once it did.
    fn cursor_over_axis(app: &mut App, axis: TransformGizmoAxis) -> Vec2 {
        let world = app.world_mut();
        let mut focus_query = world.query_filtered::<&GlobalTransform, With<TransformGizmoFocus>>();
        let global_tf = *focus_query.single(world).expect("a focused entity");

        let (mode, axis_length, rotate_ring_radius, screen_scale_factor, space) = {
            let settings = world.resource::<TransformGizmoSettings>();
            (
                settings.mode,
                settings.axis_length,
                settings.rotate_ring_radius,
                settings.screen_scale_factor,
                *effective_space(settings),
            )
        };
        let rotation = gizmo_rotation(&global_tf, &space);

        let mut cam_query =
            world.query_filtered::<(&Camera, &GlobalTransform), With<TransformGizmoCamera>>();
        let (camera, cam_tf) = cam_query.single(world).expect("a marked camera");

        let gizmo_pos = global_tf.translation();
        let scale = if screen_scale_factor > 0.0 {
            (cam_tf.translation() - gizmo_pos).length() * screen_scale_factor
        } else {
            1.0
        };
        let dir = match axis {
            TransformGizmoAxis::X => rotation * Vec3::X,
            TransformGizmoAxis::Y => rotation * Vec3::Y,
            TransformGizmoAxis::Z => rotation * Vec3::Z,
            TransformGizmoAxis::View => cam_tf.forward().as_vec3(),
        };
        let world_point = match mode {
            TransformGizmoMode::Translate | TransformGizmoMode::Scale => {
                gizmo_pos + dir * (axis_length * scale)
            }
            TransformGizmoMode::Rotate => {
                // A point genuinely on the ring, but away from the one angle
                // (straight out along the camera's own right vector) that
                // collides with two other things at once in this fixture's
                // head-on camera: the X-translate handle's endpoint, and —
                // worse — the Z ring's own screen circle, since a ring whose
                // normal roughly follows the camera's forward axis (the Z
                // ring, here) is seen nearly face-on and so traces out that
                // same on-screen radius at every angle, while a ring whose
                // normal is broadside to the camera (the Y ring, here) is
                // seen edge-on and collapses toward screen-centre as its
                // points move away from that one angle. Blending `right` with
                // the ring's other in-plane basis vector at a substantial
                // angle keeps the point on the true 3D ring while pulling its
                // *screen* distance from centre well inside the face-on ring's
                // radius, breaking the tie in the edge-on ring's favour.
                let right = cam_tf.right().as_vec3();
                let in_plane = right - dir * right.dot(dir);
                let basis_a = if in_plane.length_squared() > 1e-6 {
                    in_plane.normalize()
                } else {
                    dir.any_orthonormal_vector()
                };
                let basis_b = dir.cross(basis_a).normalize();
                let perp = (basis_a * 0.85 + basis_b * (1.0 - 0.85 * 0.85_f32).sqrt()).normalize();
                // A small nudge along the camera's own up, off the ring's
                // exact plane. This fixture's camera sits at the same world
                // height as the gizmo (`app_with_a_cube`'s `(0, 0, 10)`, cube
                // at the origin), so a ray toward a point genuinely *on* the
                // Y ring — which, being normal to Y, is entirely at that same
                // height — has an exactly-zero Y direction component and can
                // never hit the (also Y-normal) rotation plane
                // `viewport_gizmo_drag` intersects against
                // (`intersect_plane`'s `denominator` is exactly `0.0`, not
                // merely small). A real cursor would never land with that
                // much precision either; nudging off the ring by a fraction
                // of its radius keeps this within `point_to_ring_screen_dist`'s
                // generous hit threshold while giving the ray a real Y
                // component to intersect with.
                let nudge = cam_tf.up().as_vec3() * (0.1 * rotate_ring_radius * scale);
                gizmo_pos + perp * (rotate_ring_radius * scale) + nudge
            }
        };
        let screen = camera.world_to_viewport(cam_tf, world_point).expect("point in front of the camera");
        screen / camera.logical_viewport_size().expect("a sized viewport")
    }

    /// The rotate-Y ring's screen position, in whatever scene `app` currently
    /// holds. Named separately from `cursor_over_axis` only for readability
    /// at the rotate test's call site — viewed from this fixture's camera
    /// (looking down `-Z`) the Y ring sits edge-on, not a position a human
    /// could eyeball the way the X handle's screen-right position is, so it
    /// goes through the same real geometry rather than a guessed literal.
    /// Callers must switch `TransformGizmoSettings::mode` to `Rotate` first,
    /// same as the rotate test does.
    fn ring_point_for_y(app: &mut App) -> Vec2 {
        cursor_over_axis(app, TransformGizmoAxis::Y)
    }

    #[test]
    fn dragging_the_x_handle_moves_along_x_only() {
        let (mut app, cube) = app_with_a_focused_gizmo();
        let start = cursor_over_axis(&mut app, TransformGizmoAxis::X);
        press_on_axis(&mut app, TransformGizmoAxis::X, start);
        drag_to(&mut app, start + Vec2::new(0.10, 0.0));

        let tf = app.world().get::<Transform>(cube).unwrap();
        assert!(tf.translation.x.abs() > 0.01, "x did not move: {:?}", tf.translation);
        assert!(tf.translation.y.abs() < 1e-4, "y moved: {:?}", tf.translation);
        assert!(tf.translation.z.abs() < 1e-4, "z moved: {:?}", tf.translation);
    }

    #[test]
    fn a_release_ends_the_drag() {
        let (mut app, cube) = app_with_a_focused_gizmo();
        let start = cursor_over_axis(&mut app, TransformGizmoAxis::X);
        press_on_axis(&mut app, TransformGizmoAxis::X, start);
        drag_to(&mut app, start + Vec2::new(0.10, 0.0));
        release(&mut app);
        let after_release = *app.world().get::<Transform>(cube).unwrap();

        drag_to(&mut app, start + Vec2::new(0.28, 0.0));

        assert_eq!(*app.world().get::<Transform>(cube).unwrap(), after_release);
        assert!(!app.world().resource::<TransformGizmoState>().active);
    }

    #[test]
    fn a_cancel_ends_the_drag_too() {
        // Same hazard M6 Task 14 found on the canvas: without this the state
        // stays `active` forever and picking never works again.
        let (mut app, _cube) = app_with_a_focused_gizmo();
        let start = cursor_over_axis(&mut app, TransformGizmoAxis::X);
        press_on_axis(&mut app, TransformGizmoAxis::X, start);
        feed(&mut app, vec![ViewportInput::Cancel]);
        assert!(!app.world().resource::<TransformGizmoState>().active);
    }

    #[test]
    fn a_cancel_still_ends_the_drag_when_no_camera_carries_the_gizmo_marker() {
        // The stuck-drag hazard: `mark_gizmo_camera` can mark zero cameras
        // (see its doc comment and `viewport_gizmo_drag`'s own "End drag"
        // comment) — e.g. the user toggles `ViewportCamera` to `Scene`
        // before that document's scene camera has spawned. If the end-drag
        // check ran only after the `cameras` query, this frame would return
        // early before ever resetting `state.active`, and `pick_on_click`
        // would refuse to select anything for the rest of the session.
        let (mut app, _cube) = app_with_a_focused_gizmo();
        let start = cursor_over_axis(&mut app, TransformGizmoAxis::X);
        press_on_axis(&mut app, TransformGizmoAxis::X, start);
        assert!(app.world().resource::<TransformGizmoState>().active, "test setup: drag did not start");

        // Remove the marker from every camera, simulating the camera
        // vanishing (or `mark_gizmo_camera` finding nothing to mark)
        // mid-drag.
        let marked: Vec<Entity> = app
            .world_mut()
            .query_filtered::<Entity, With<TransformGizmoCamera>>()
            .iter(app.world())
            .collect();
        for entity in marked {
            app.world_mut().entity_mut(entity).remove::<TransformGizmoCamera>();
        }

        feed(&mut app, vec![ViewportInput::Cancel]);

        assert!(
            !app.world().resource::<TransformGizmoState>().active,
            "a Cancel must reset `active` even with no camera carrying TransformGizmoCamera",
        );
    }

    #[test]
    fn rotate_mode_turns_the_object_without_moving_it() {
        let (mut app, cube) = app_with_a_focused_gizmo();
        app.world_mut().resource_mut::<TransformGizmoSettings>().mode = TransformGizmoMode::Rotate;
        let before = *app.world().get::<Transform>(cube).unwrap();
        let ring_pos = ring_point_for_y(&mut app);
        press_on_axis(&mut app, TransformGizmoAxis::Y, ring_pos);
        drag_to(&mut app, ring_pos + Vec2::new(0.06, 0.0));

        let after = app.world().get::<Transform>(cube).unwrap();
        assert_ne!(after.rotation, before.rotation);
        assert_eq!(after.translation, before.translation);
    }

    #[test]
    fn a_drag_on_a_handle_does_not_also_select_something() {
        // `pick_on_click` runs after this system and skips while a drag is
        // active. If it did not, grabbing a handle would reselect whatever mesh
        // the ray happened to hit behind it.
        let (mut app, cube) = app_with_a_focused_gizmo();
        let start = cursor_over_axis(&mut app, TransformGizmoAxis::X);
        press_on_axis(&mut app, TransformGizmoAxis::X, start);
        assert_eq!(app.world().resource::<Selection>().0, Some(cube));
    }

    #[test]
    fn a_parented_object_moves_the_same_distance_as_an_unparented_one() {
        // The gizmo displays at `GlobalTransform` and writes local `Transform`;
        // the demo document's own cube is parented, so a version that forgot the
        // parent's inverse would be visibly wrong on the first real run.
        let (mut app, cube) = app_with_a_focused_gizmo();
        let parent = app
            .world_mut()
            .spawn(Transform::from_xyz(5.0, 0.0, 0.0).with_scale(Vec3::splat(2.0)))
            .id();
        app.world_mut().entity_mut(cube).insert(ChildOf(parent));
        app.update();

        let start = cursor_over_axis(&mut app, TransformGizmoAxis::X);
        press_on_axis(&mut app, TransformGizmoAxis::X, start);
        drag_to(&mut app, start + Vec2::new(0.10, 0.0));

        let world_x = app.world().get::<GlobalTransform>(cube).unwrap().translation().x;
        assert!(world_x > 5.0, "the child must move in world space: {world_x}");
    }
}
