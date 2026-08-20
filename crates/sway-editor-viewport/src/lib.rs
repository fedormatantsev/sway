//! The editor's viewport: camera orbit, the transform gizmo and picking.
//!
//! Editor-only interaction. Nothing on stage runs it, which is why it is a
//! crate of its own rather than part of `sway-runtime`: a show build never
//! links it. The dependency runs surface -> runtime — it reads `NodeEntities`
//! to resolve a picked entity back to the node that produced it — which is
//! the same direction the host runs, so `sway-runtime` must not depend on it.

pub mod camera;
pub mod gizmo;
pub mod pick;

use bevy::gizmos::transform_gizmo::{
    TransformGizmoSettings, TransformGizmoSpace, TransformGizmoState,
};
use bevy::prelude::*;
use crossbeam_channel::Receiver;
use sway_viewport_input::ViewportInput;

pub use camera::{ActiveViewportCamera, FittedRect, ViewportCamera, fit_aspect};

/// The receiving half of the editor's viewport input channel, held by the
/// world. Present only in an editor build.
///
/// Owned here rather than in the engine: the boundary that drains a channel
/// owns its plumbing, and `drain_viewport_input` below is the only thing that
/// reads it.
#[derive(Resource)]
pub struct ViewportInputRx(pub Receiver<ViewportInput>);

/// This frame's viewport input, replaced wholesale each `PreUpdate`.
///
/// One drain, several readers: the camera, the picker and the gizmo all need
/// the same events, and a channel can only be drained once.
#[derive(Resource, Default)]
pub struct ViewportEvents(pub Vec<ViewportInput>);

/// Everything M7 adds, ordered.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub enum ViewportSystems {
    /// Fills `ViewportEvents`. `PreUpdate`.
    Drain,
    /// Reads them and moves the editor camera. `PreUpdate`, after `Drain`.
    Camera,
    /// Gizmo drag. `PostUpdate`, before transform propagation.
    GizmoDrag,
    /// Gizmo hover and click-to-select. `PostUpdate`, after propagation.
    Pick,
}

pub fn drain_viewport_input(rx: Option<Res<ViewportInputRx>>, mut events: ResMut<ViewportEvents>) {
    events.0.clear();
    let Some(rx) = rx else {
        return;
    };
    events.0.extend(rx.0.try_iter());
}

/// Everything the editor's viewport needs in the world. Added by `sway-app`
/// only under `--editor`; a show build never sees it, so nothing here can
/// affect what happens on stage.
pub struct EditorViewportPlugin;

impl Plugin for EditorViewportPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewportEvents>()
            .init_resource::<camera::ViewportCamera>()
            .init_resource::<camera::ActiveViewportCamera>()
            // The editor's consumer claim on a camera. Owned here, read by
            // the runtime's target allocator.
            .init_resource::<sway_runtime::EditorCameraPreview>()
            // The picker sets it and the gizmo follows it. Owned by the
            // editor, not the graph, so an editor build has to insert it.
            .init_resource::<sway_selection::Selection>()
            .init_resource::<sway_graph::graph::Graph>()
            .init_resource::<sway_runtime::project::NodeEntities>()
            // Switches `TransformGizmoRenderPlugin`'s systems on; both must
            // exist before `Startup`, when `spawn_gizmo_meshes` runs.
            .init_resource::<TransformGizmoState>()
            .insert_resource(TransformGizmoSettings {
                space: TransformGizmoSpace::World,
                // Nothing here owns a cursor to confine, and the setting
                // reaches for `CursorOptions` on a window that does not
                // exist.
                confine_cursor: false,
                ..Default::default()
            })
            .add_systems(
                PreUpdate,
                drain_viewport_input.in_set(ViewportSystems::Drain),
            )
            .add_systems(Startup, camera::spawn_editor_camera)
            .add_systems(
                PreUpdate,
                camera::navigate_editor_camera
                    .in_set(ViewportSystems::Camera)
                    .after(ViewportSystems::Drain),
            )
            .add_systems(
                Update,
                (
                    // Settle the selection first: a camera deleted since the
                    // last frame must fall back before anything acts on it.
                    camera::settle_viewport_camera,
                    camera::publish_camera_preview,
                )
                    .chain()
                    // Before projection, so the claim this frame publishes is
                    // the one the allocator reads this frame.
                    .before(sway_runtime::ProjectionSet),
            )
            .add_systems(
                Update,
                (camera::apply_active_camera, gizmo::mark_gizmo_camera)
                    .chain()
                    // *After* projection: which cameras render is decided from
                    // the targets this frame allocated, not last frame's.
                    .after(sway_runtime::ProjectionSet),
            )
            .add_systems(
                Update,
                (gizmo::follow_selection, gizmo::disable_gizmo_camera_clear),
            )
            .add_systems(
                PostUpdate,
                gizmo::viewport_gizmo_drag
                    .in_set(ViewportSystems::GizmoDrag)
                    .before(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(
                PostUpdate,
                (gizmo::set_gizmo_mode, gizmo::viewport_gizmo_hover)
                    .chain()
                    .after(bevy::transform::TransformSystems::Propagate)
                    .before(ViewportSystems::Pick),
            )
            .add_systems(
                PostUpdate,
                pick::pick_on_click
                    .in_set(ViewportSystems::Pick)
                    .after(bevy::transform::TransformSystems::Propagate)
                    .after(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_drain_serves_every_reader_for_a_frame() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.insert_resource(ViewportInputRx(rx))
            .init_resource::<ViewportEvents>()
            .add_systems(Update, drain_viewport_input);

        tx.send(ViewportInput::Cancel).unwrap();
        app.update();
        assert_eq!(app.world().resource::<ViewportEvents>().0.len(), 1);

        // Nothing sent this frame: the buffer must empty, or a click would
        // fire again every frame forever.
        app.update();
        assert!(app.world().resource::<ViewportEvents>().0.is_empty());
    }

    #[test]
    fn a_world_with_no_receiver_drains_nothing_and_does_not_panic() {
        // A show build has no editor channel at all.
        let mut app = App::new();
        app.init_resource::<ViewportEvents>()
            .add_systems(Update, drain_viewport_input);
        app.update();
        assert!(app.world().resource::<ViewportEvents>().0.is_empty());
    }
}
