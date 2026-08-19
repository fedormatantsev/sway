//! `SpriteMaterial` — a sprite sheet as a material node.
//!
//! A port of `crate::sprite_material::SpriteMaterial`, which stays where it is
//! until group 9. The *asset* — [`SpriteMaterialAsset`], its uniform, its
//! shader, its `Material` impl and its `specialize` depth-write flip — is
//! reused unchanged; only the node is new. **Group 9 deletes the node and the
//! wires there, not the asset.**
//!
//! The colour and depth runs arrive as marker connections from
//! [`FrameSequence`](crate::nodes::frame_sequence::FrameSequence) nodes:
//! neither carries the sequence, and the number of frames is the connected
//! sequences' own layer count rather than an authored number, which is what
//! makes a sequence that failed to load impossible to sample out of range.

use bevy::ecs::system::EntityCommands;
use bevy::ecs::world::World;
use bevy::prelude::*;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::nodes::protocol::{self, ImageSequence, ReflectMaterialNode, SceneMaterialOut};
use crate::sprite_material::SpriteMaterialAsset;

/// [`SpriteMaterial`]'s inlets.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct SpriteMaterialIn {
    /// The colour run's port. A marker inlet: pure schema, declaring that the
    /// port exists and what may connect to it. The projector reads the edge,
    /// never this field (design D6).
    pub color: ImageSequence,
    /// The depth run's port. Same shape as `color`, and one
    /// [`FrameSequence`](crate::nodes::frame_sequence::FrameSequence) kind
    /// serves either role.
    pub depth: ImageSequence,
    /// Which frame of the connected sequences to show. `f32` so any float
    /// outlet can drive it; the read-side clamp
    /// ([`layer_index`](crate::sprite_material::layer_index)) bounds it.
    pub frame: f32,
    /// Authored as sRGB; the projector linearizes it, because the colour run
    /// is sampled through an sRGB view and is already linear where the shader
    /// multiplies.
    pub tint: Vec3,
    pub opacity: f32,
    /// World units spanned by the full 0..1 depth channel.
    pub depth_range: f32,
    /// The depth value that leaves a vertex on the undisplaced surface.
    pub depth_pivot: f32,
}

impl Default for SpriteMaterialIn {
    fn default() -> Self {
        Self {
            color: ImageSequence,
            depth: ImageSequence,
            frame: 0.0,
            tint: Vec3::ONE,
            opacity: 1.0,
            depth_range: 1.0,
            depth_pivot: 0.5,
        }
    }
}

/// [`SpriteMaterial`]'s state. Not authored, not serialized.
#[derive(Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct SpriteMaterialState {
    /// The published asset, or `Handle::default()` while the material is
    /// incomplete.
    ///
    /// Unlike every other producer here the handle is **not** allocated
    /// unconditionally at node creation: a sprite material with an
    /// unconnected run must render *nothing* rather than render incorrectly,
    /// and `ImagePlugin` seeds a real 1×1 white image at `Handle::default()`,
    /// so an asset published with a default texture would draw a plain white
    /// quad. Dropping the handle is what makes "renders nothing" happen.
    pub handle: Handle<SpriteMaterialAsset>,
    /// Bumped only when `handle` changes identity — which for this node is
    /// exactly the incomplete/complete transition. See
    /// [`protocol::MaterialNode::revision`].
    pub revision: u64,
    /// The layer count the published uniform was bounded by, so the projector
    /// can tell a settled material from one that needs rewriting.
    pub layers: u32,
    /// The last diagnostic reported, so a permanent disagreement between the
    /// two runs is logged once rather than once per frame.
    pub reported: Option<String>,
}

/// A sprite sheet as a material node.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, MaterialNode, Default)]
pub struct SpriteMaterial {
    pub inlets: SpriteMaterialIn,
    pub state: SpriteMaterialState,
    pub outlets: SceneMaterialOut,
}

impl NodeKind for SpriteMaterial {
    /// Nothing: the asset needs `ResMut<Assets<SpriteMaterialAsset>>` and the
    /// connected sequences' textures. The projector does it.
    fn evaluate(&mut self, _world: &World) {}
}

impl protocol::MaterialNode for SpriteMaterial {
    fn attach(&self, commands: &mut EntityCommands) {
        if self.state.handle == Handle::default() {
            // An incomplete material renders nothing, and "nothing" has to be
            // an actual removal: a scene node that was drawing must stop.
            commands.remove::<MeshMaterial3d<SpriteMaterialAsset>>();
        } else {
            commands.insert(MeshMaterial3d(self.state.handle.clone()));
        }
    }

    fn detach(&self) -> fn(&mut EntityCommands) {
        |commands| {
            commands.remove::<MeshMaterial3d<SpriteMaterialAsset>>();
        }
    }

    fn revision(&self) -> u64 {
        self.state.revision
    }
}
