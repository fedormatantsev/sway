//! The `Output` node: what the window and the fullscreen display present.
//!
//! **Not a scene node.** The scene-node set is closed (design D7) and this is
//! not in it: `Output` declares no pose, no `children` and no `SceneNodeOut`,
//! so `Graph::connect` refuses a mesh, a material or a child connection by
//! schema — the same way `Group` refuses geometry — and nothing is spawned in
//! the world for it.
//!
//! What it carries is a single choice: which camera is presented. A document
//! with no output node, or one with nothing connected, presents nothing
//! rather than picking a camera by creation order or by whichever rendered
//! last.

use bevy::ecs::world::World;
use bevy::reflect::Reflect;
use bevy::reflect::std_traits::ReflectDefault;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::nodes::protocol::CameraTarget;

/// [`Output`]'s inlets: one camera, and nothing else.
#[derive(Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct OutputIn {
    /// The camera port. A marker inlet: pure schema (design D6), and
    /// **non-variadic**, so connecting a second camera evicts the first
    /// rather than failing — which is the `nodes` spec's "connecting a second
    /// MUST replace the first". The projector reads the edge; it never reads
    /// this field.
    pub camera: CameraTarget,
}

/// Names the camera the running system presents.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct Output {
    pub inlets: OutputIn,
    pub state: (),
    /// Nothing: an output node is the end of a chain, and offering itself as a
    /// child would make it a scene node.
    pub outlets: (),
}

impl NodeKind for Output {
    fn evaluate(&mut self, _world: &World) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_output_declares_a_camera_port_and_nothing_a_scene_node_has() {
        // Mirrors `a_group_declares_no_geometry_and_no_material_port`: the
        // absence is the requirement, and it is invisible unless pinned.
        // `Graph::connect` resolves a destination path on the *inlets* value,
        // so a port that does not exist is a refusal.
        use bevy::reflect::{Typed, structs::Struct};
        let bevy::reflect::TypeInfo::Struct(info) = OutputIn::type_info() else {
            panic!("OutputIn is a struct");
        };
        let names: Vec<&str> = info.iter().map(|field| field.name()).collect();
        assert_eq!(names, vec!["camera"]);

        let inlets = OutputIn::default();
        assert!(inlets.field("mesh").is_none(), "an output draws nothing");
        assert!(inlets.field("material").is_none());
        assert!(
            inlets.field("children").is_none(),
            "an output is not a placement, so nothing sits under it"
        );
        assert!(inlets.field("translation").is_none(), "and it has no pose");
        assert!(inlets.field("rotation").is_none());
        assert!(inlets.field("scale").is_none());
    }

    #[test]
    fn an_output_offers_itself_as_nothing() {
        // No `SceneNodeOut`, so it cannot be connected under a group and the
        // closed scene-node set is untouched.
        use bevy::reflect::{TypeInfo, Typed};
        assert!(
            matches!(<Output as Typed>::type_info(), TypeInfo::Struct(_)),
            "the node kind is still the three-part struct"
        );
        let bevy::reflect::TypeInfo::Struct(info) = <Output as Typed>::type_info() else {
            unreachable!()
        };
        let outlets = info
            .field("outlets")
            .expect("every node kind has an outlets part");
        assert_eq!(outlets.type_path(), "()");
    }
}
