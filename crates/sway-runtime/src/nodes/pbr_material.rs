//! `PbrMaterial` — Bevy's `StandardMaterial` as a material node.
//!
//! A port of the old `pbr_material::PbrMaterial`, which stays where it is
//! until group 9. It lands here rather than in `sway-nodes` so that every
//! render-coupled node kind sits in one crate.
//!
//! The node owns its `Assets<StandardMaterial>` entry and no entity (design
//! D7), and it is the only thing in the process that knows its material type
//! is `StandardMaterial` — it inserts `MeshMaterial3d<StandardMaterial>`
//! itself through [`protocol::MaterialNode`].

use bevy::ecs::system::EntityCommands;
use bevy::ecs::world::World;
use bevy::prelude::*;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::nodes::protocol::{self, ReflectMaterialNode, SceneMaterialOut};

/// [`PbrMaterial`]'s inlets.
///
/// Colours are `Vec3` rather than `Color` because every colour inlet is a
/// `Vec3` connection, and the field an edge writes has to be the type the
/// edge carries. They are read as sRGB — what an author types — and converted
/// on the way to the asset.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct PbrMaterialIn {
    pub base_color: Vec3,
    pub emissive: Vec3,
    pub metallic: f32,
    pub roughness: f32,
}

impl Default for PbrMaterialIn {
    fn default() -> Self {
        Self {
            base_color: Vec3::splat(0.8),
            emissive: Vec3::ZERO,
            metallic: 0.0,
            roughness: 0.5,
        }
    }
}

/// [`PbrMaterial`]'s state. Not authored, not serialized.
#[derive(Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct PbrMaterialState {
    /// Allocated structurally when the node is first projected, then
    /// **mutated in place** — that is what makes one material node connected
    /// to three scene nodes stay one asset, so an edit reaches all three.
    pub handle: Handle<StandardMaterial>,
    /// Bumped only when `handle` changes identity. See
    /// [`protocol::MaterialNode::revision`].
    pub revision: u64,
}

/// A PBR material as a node.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, MaterialNode, Default)]
pub struct PbrMaterial {
    pub inlets: PbrMaterialIn,
    pub state: PbrMaterialState,
    pub outlets: SceneMaterialOut,
}

impl NodeKind for PbrMaterial {
    /// Nothing: building the asset needs `ResMut<Assets<StandardMaterial>>`.
    /// The projector does it for every dirty material node.
    fn evaluate(&mut self, _world: &World) {}
}

impl protocol::MaterialNode for PbrMaterial {
    fn attach(&self, commands: &mut EntityCommands) {
        let handle = self.state.handle.clone();
        commands.insert(MeshMaterial3d(handle));
    }

    fn detach(&self) -> fn(&mut EntityCommands) {
        |commands| {
            commands.remove::<MeshMaterial3d<StandardMaterial>>();
        }
    }

    fn revision(&self) -> u64 {
        self.state.revision
    }
}

/// The `StandardMaterial` these inlets describe. Pure — no ECS, no assets.
pub fn to_standard_material(inlets: &PbrMaterialIn) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::srgb(
            inlets.base_color.x,
            inlets.base_color.y,
            inlets.base_color.z,
        ),
        emissive: LinearRgba::rgb(inlets.emissive.x, inlets.emissive.y, inlets.emissive.z),
        metallic: inlets.metallic,
        perceptual_roughness: inlets.roughness,
        ..default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_parameters_reach_the_standard_material() {
        let material = to_standard_material(&PbrMaterialIn {
            base_color: Vec3::ONE,
            emissive: Vec3::ZERO,
            metallic: 0.25,
            roughness: 0.75,
        });
        assert_eq!(material.base_color, Color::srgb(1.0, 1.0, 1.0));
        assert_eq!(material.metallic, 0.25);
        assert_eq!(material.perceptual_roughness, 0.75);
    }
}
