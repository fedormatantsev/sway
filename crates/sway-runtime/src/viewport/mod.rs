//! Viewport interaction: the world half. Spec M7.

pub mod camera;

use bevy::prelude::*;
use sway_graph::{ViewportInput, ViewportInputRx};

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
                (camera::tag_scene_cameras, camera::apply_active_camera).chain(),
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
