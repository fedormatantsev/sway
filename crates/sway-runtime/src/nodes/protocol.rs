//! The protocols (design D6).
//!
//! A **protocol** is a pair: a ZST marker type used as an outlet, and a
//! `#[reflect_trait]` the projector calls. A node joins a protocol by
//! declaring an outlet of the marker type and implementing the trait.
//!
//! | protocol | marker | trait |
//! |---|---|---|
//! | material | [`SceneMaterial`] | [`MaterialNode`] |
//! | image sequence | [`ImageSequence`] | [`ImageSequenceNode`] |
//! | mesh source | [`MeshSource`] | [`MeshNode`] |
//! | hierarchy | [`SceneChild`] | — (the projector needs only the `NodeId`) |
//! | camera target | [`CameraTarget`] | — (the projector needs only the `NodeId`) |
//!
//! **A marker edge propagates nothing** — `sway-graph` sets `Edge::valueless`
//! from `size_of_val == 0` at connect time — but it **stays in the sort**,
//! which is what gives projector ordering for free. `FrameSequence ->
//! SpriteMaterial` needs exactly that: the published array texture only
//! exists once the folder resolves.
//!
//! **Marker inlets are pure schema.** `children: Vec<SceneChild>` has no
//! useful runtime value. It exists to declare that the port exists, what its
//! type is, and that it is variadic — the editor reads that from the kind's
//! declared part type,
//! the legality rule reads it at connect time, and the projectors read the
//! *edge index*, never the field. Do not "optimize away" an unread marker
//! field: removing it removes the port.
//!
//! This is where the last type erasure goes. A material node is the only
//! thing that ever knows its concrete `M`, and it inserts `MeshMaterial3d<M>`
//! itself — no `UntypedHandle`, and no `TypeId -> insert fn` registry.
//!
//! ## Naming
//!
//! [`MeshNode`] here is the mesh-source *trait*; `super::scene::MeshNode` is
//! the scene node *kind* that consumes it. Design D6 and D7 name both, so
//! they are deliberately kept in separate modules and neither is re-exported
//! flat. Always reach the trait as `protocol::MeshNode`.

use bevy::ecs::system::EntityCommands;
use bevy::prelude::*;
use bevy::reflect::reflect_trait;

// ---------------------------------------------------------------------------
// Markers
// ---------------------------------------------------------------------------

/// The material protocol's marker. An outlet of this type says "this node is
/// a material"; an inlet of this type says "a material may be connected
/// here".
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash)]
pub struct SceneMaterial;

/// The image-sequence protocol's marker.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash)]
pub struct ImageSequence;

/// The mesh-source protocol's marker.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash)]
pub struct MeshSource;

/// The hierarchy protocol's marker. It has no trait: a parent connection
/// carries identity and nothing else, so the projector needs only the
/// `NodeId` at either end of the edge.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash)]
pub struct SceneChild;

/// The camera-target protocol's marker. An outlet of this type says "this node
/// renders a frame you may consume"; an inlet of this type says "a camera may
/// be connected here".
///
/// Like [`SceneChild`] it has no trait. The `nodes` spec requires the
/// connection to carry identity only — no value and no image travels along it
/// — so a consumer reaches the frames by asking the runtime for the target
/// belonging to the node at the other end of the edge, never by reading a
/// field. What the edge buys beyond schema is its place in the sort: a
/// consumer is projected after the camera that feeds it.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash)]
pub struct CameraTarget;

// ---------------------------------------------------------------------------
// Shared outlet parts
// ---------------------------------------------------------------------------

/// The outlets of a node that produces a mesh.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct MeshSourceOut {
    pub mesh: MeshSource,
}

/// The outlets of a node that produces a material.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct SceneMaterialOut {
    pub material: SceneMaterial,
}

/// The outlets of a node that produces an image sequence.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct ImageSequenceOut {
    pub sequence: ImageSequence,
}

/// The outlets of a node that renders a frame.
///
/// A camera declares this *alongside* its `SceneNodeOut`, because a camera is
/// both a placement in the scene and something a consumer can look through.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct CameraTargetOut {
    pub child: SceneChild,
    pub camera: CameraTarget,
}

/// The outlets of a node that produces a camera target and is not a scene
/// placement — a post-process node. No [`SceneChild`]: offering one would
/// admit child and pose connections the `nodes` spec forbids.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct CameraFeedOut {
    pub camera: CameraTarget,
}

/// The outlets of every scene node: one port, offering itself as a child.
///
/// Every scene node accepts children *and* can be one, which is what makes
/// the hierarchy a graph connection rather than a node kind.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct SceneNodeOut {
    pub child: SceneChild,
}

// ---------------------------------------------------------------------------
// Port names
// ---------------------------------------------------------------------------

/// The inlet a scene node's material arrives on, and the outlet a material
/// node offers. Both sides share one name so a document reads symmetrically.
pub const MATERIAL: &str = "material";
/// The inlet a scene node's mesh arrives on, and the outlet a mesh producer
/// offers.
pub const MESH: &str = "mesh";
/// The variadic inlet child connections arrive on.
pub const CHILDREN: &str = "children";
/// The outlet a scene node offers itself as a child through.
pub const CHILD: &str = "child";
/// A sprite material's colour-run inlet.
pub const COLOR: &str = "color";
/// A sprite material's depth-run inlet.
pub const DEPTH: &str = "depth";
/// The outlet an image-sequence producer offers.
pub const SEQUENCE: &str = "sequence";
/// The inlet a consumer's camera arrives on, and the outlet a camera offers
/// its rendered frames through. Both sides share one name, like `material`
/// and `mesh`.
pub const CAMERA: &str = "camera";
/// The inlet a post-process node takes its source camera target on.
pub const SOURCE: &str = "source";
/// A camera's authored resolution.
pub const RESOLUTION: &str = "resolution";
/// A capture node's output path pattern.
pub const PATH: &str = "path";
/// A capture node's recording flag.
pub const RECORDING: &str = "recording";

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// A node that owns a material asset and applies it to the scene nodes
/// connected to it.
///
/// The node is the only thing in the process that knows its concrete
/// material type `M`; the projector reaches it only through this trait.
#[reflect_trait]
pub trait MaterialNode {
    /// Inserts this node's own typed material component (`MeshMaterial3d<M>`)
    /// on a connected scene node's entity.
    ///
    /// Called only when [`MaterialNode::revision`] or the connected node
    /// changes, so an implementation may insert unconditionally.
    fn attach(&self, commands: &mut EntityCommands);

    /// How to take this material back off an entity.
    ///
    /// A plain `fn` rather than a `&self` method because an attachment
    /// outlives the node that made it: deleting a material node drops the
    /// node before anything gets a chance to ask it to detach, and the
    /// entity it wrote to still carries the component. The projector records
    /// this pointer alongside the attachment and calls it later.
    fn detach(&self) -> fn(&mut EntityCommands);

    /// Changes exactly when what [`MaterialNode::attach`] would insert
    /// changes — that is, when the published asset handle changes identity.
    ///
    /// It does **not** change when the asset's *contents* change, because
    /// those are mutated in place and every consumer already holds the
    /// handle. This is what keeps an animating material from re-inserting
    /// its component (and re-triggering change detection) every tick.
    fn revision(&self) -> u64;
}

/// A node that owns a layered image sequence.
#[reflect_trait]
pub trait ImageSequenceNode {
    /// The published array texture. Never `Handle::default()` once the
    /// sequence has assembled; `Handle::default()` means "nothing published".
    fn texture(&self) -> &Handle<Image>;

    /// How many layers the published texture actually has.
    ///
    /// Not in design D6's table, but required by the `runtime` spec: "the
    /// number of frames available MUST be the layer count of the connected
    /// sequences rather than an authored number". A consumer cannot ask a
    /// `Handle<Image>` how many layers it has without resolving it, and a
    /// count copied into the consumer would be one tick stale exactly when a
    /// sequence finishes loading — which is an out-of-range array sample in
    /// the vertex stage.
    fn layers(&self) -> u32;
}

/// A node that owns a mesh asset.
///
/// Note the name: this is the mesh **source** protocol's trait, not the
/// `MeshNode` scene node kind (`super::scene::MeshNode`). Design D6 and D7
/// name both.
#[reflect_trait]
pub trait MeshNode {
    /// The published mesh. `Handle::default()` means "nothing published".
    fn handle(&self) -> &Handle<Mesh>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use sway_graph::graph::legality::is_valueless;

    #[test]
    fn every_protocol_marker_is_valueless() {
        // `sway-graph` decides `Edge::valueless` from `size_of_val`, never by
        // naming a marker type (design D6). A marker that accidentally grew a
        // field would start propagating a value and silently stop being a
        // protocol.
        assert!(is_valueless(&SceneMaterial));
        assert!(is_valueless(&ImageSequence));
        assert!(is_valueless(&MeshSource));
        assert!(is_valueless(&SceneChild));
        assert!(is_valueless(&CameraTarget));
    }

    #[test]
    fn a_camera_feed_outlet_is_the_camera_target_and_nothing_a_scene_node_has() {
        // Post-process nodes produce frames without being placements.
        // `CameraTargetOut` carries a `child` port; this one must not.
        use bevy::reflect::{Typed, structs::Struct};
        let bevy::reflect::TypeInfo::Struct(info) = CameraFeedOut::type_info() else {
            panic!("CameraFeedOut is a struct");
        };
        let names: Vec<&str> = info.iter().map(|field| field.name()).collect();
        assert_eq!(names, vec!["camera"]);

        let outlets = CameraFeedOut::default();
        assert!(
            outlets.field("child").is_none(),
            "a feed is not a scene child"
        );
        assert_eq!(
            std::mem::size_of::<CameraFeedOut>(),
            0,
            "the outlet part is a protocol marker and must stay valueless"
        );
        assert!(is_valueless(&outlets.camera));
    }
}
