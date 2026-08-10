//! The camera and the lights, as nodes.

use bevy::prelude::*;
use sway_graph::EditorPos;

/// The scene's camera, as opposed to M7's editor camera. A bare marker: the
/// render target is set by `sway_runtime::headless::retarget_cameras`, and
/// field of view and clear colour stay at Bevy's defaults until something asks
/// otherwise. What this component carries is identity — which of the cameras in
/// the world is the one the show looks through.
#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
#[require(Camera3d, EditorPos)]
pub struct SceneCamera;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_camera_brings_a_working_camera_with_it() {
        // The render target is not set here: headless::retarget_cameras points
        // every camera at the viewport texture each Update, which is the whole
        // of "SceneCamera produces Camera3d + RenderTarget".
        let mut world = World::new();
        let entity = world.spawn(SceneCamera).id();

        assert!(world.get::<Camera3d>(entity).is_some());
        assert!(world.get::<Camera>(entity).is_some());
        assert!(world.get::<Projection>(entity).is_some());
        assert!(world.get::<Transform>(entity).is_some(), "authored by the document");
    }

    #[test]
    fn the_camera_and_both_lights_are_authorable() {
        let mut app = App::new();
        app.add_plugins(sway_graph::WiresPlugin)
            .add_plugins(crate::WireNodesPlugin);

        let registry = app.world().resource::<sway_graph::ComponentDocRegistry>();
        for name in ["SceneCamera", "DirectionalLight", "PointLight"] {
            assert!(registry.by_name(name).is_some(), "{name} must be authorable");
        }
    }
}
