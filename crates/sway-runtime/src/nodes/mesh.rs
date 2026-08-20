//! `MeshAsset` and `PlaneMesh` — the two mesh producers, as graph nodes.
//!
//! Ports of `sway_base_nodes::mesh_asset::MeshAsset` and
//! `sway_base_nodes::plane_mesh::PlaneMesh`, which stay where they are until group
//! 9. They land in `sway-runtime` rather than `sway-nodes` because every
//! render-coupled node kind belongs in the crate that already owns the full
//! `bevy` facade, leaving `sway-nodes` as the pure value-node crate.
//!
//! Neither node carries a placement, and neither hands its mesh along a
//! connection (`nodes`: "A node that owns an asset does not pass it along a
//! connection"). The handle lives in `state`, which is never serialized, and
//! a consumer reaches it through [`protocol::MeshNode`].

use bevy::ecs::world::World;
use bevy::prelude::*;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::nodes::protocol::{self, MeshSourceOut, ReflectMeshNode};

// ---------------------------------------------------------------------------
// MeshAsset
// ---------------------------------------------------------------------------

/// [`MeshAsset`]'s inlets.
#[derive(Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct MeshAssetIn {
    /// The sub-asset label is part of the path —
    /// `"cube.gltf#Mesh0/Primitive0"` — because a glTF file holds many
    /// meshes.
    pub path: String,
}

/// [`MeshAsset`]'s state. Never authored and never serialized (design D6):
/// a handle has no business round-tripping through a document.
#[derive(Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct MeshAssetState {
    /// Allocated structurally by the projector the first time the node is
    /// seen with a non-empty path (design D7), so a connection is never
    /// waiting on a handle that does not exist yet.
    pub handle: Handle<Mesh>,
    /// The path `handle` was loaded for, so an unrelated edit does not
    /// restart the load.
    pub loaded: String,
}

/// A mesh that comes from a file.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, MeshNode, Default)]
pub struct MeshAsset {
    pub inlets: MeshAssetIn,
    pub state: MeshAssetState,
    pub outlets: MeshSourceOut,
}

impl NodeKind for MeshAsset {
    /// Nothing: loading is the projector's job, because it needs the
    /// `AssetServer` and `evaluate` must stay reproducible (design D4).
    fn evaluate(&mut self, _world: &World) {}
}

impl protocol::MeshNode for MeshAsset {
    fn handle(&self) -> &Handle<Mesh> {
        &self.state.handle
    }
}

// ---------------------------------------------------------------------------
// PlaneMesh
// ---------------------------------------------------------------------------

/// [`PlaneMesh`]'s inlets.
///
/// A quad facing +Z, subdivided independently per axis. `PlaneMeshBuilder`'s
/// own fields are `subdivisions_x` / `subdivisions_z`, named for a plane whose
/// default normal is +Y; this node fixes the normal at +Z, under which the
/// builder's X axis stays horizontal and its Z axis becomes vertical — hence
/// `horizontal` / `vertical`, which name the quad as authored.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct PlaneMeshIn {
    pub size: Vec2,
    pub horizontal: u32,
    pub vertical: u32,
}

impl Default for PlaneMeshIn {
    fn default() -> Self {
        Self {
            size: Vec2::ONE,
            // Cost is flat across the useful range (~41k triangles at 63×63,
            // ~655k at 255×255), so this is a knob to turn by eye rather than
            // a budget to compute.
            horizontal: 63,
            vertical: 63,
        }
    }
}

/// [`PlaneMesh`]'s state. Not authored, not serialized.
#[derive(Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct PlaneMeshState {
    /// Allocated once, then **mutated in place** on every rebuild, so a
    /// consumer that already holds it picks the new geometry up without the
    /// handle moving.
    pub handle: Handle<Mesh>,
}

/// A tessellated quad, built rather than loaded.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, MeshNode, Default)]
pub struct PlaneMesh {
    pub inlets: PlaneMeshIn,
    pub state: PlaneMeshState,
    pub outlets: MeshSourceOut,
}

impl NodeKind for PlaneMesh {
    /// Nothing: building needs `ResMut<Assets<Mesh>>`, which `&World` cannot
    /// give. The projector rebuilds this node's mesh when it is dirty.
    fn evaluate(&mut self, _world: &World) {}
}

impl protocol::MeshNode for PlaneMesh {
    fn handle(&self) -> &Handle<Mesh> {
        &self.state.handle
    }
}

/// Builds the quad `inlets` describes. Pure — no ECS, no GPU.
pub fn build_plane(inlets: &PlaneMeshIn) -> Mesh {
    bevy::mesh::PlaneMeshBuilder::new(Dir3::Z, inlets.size)
        .subdivisions_x(inlets.horizontal)
        .subdivisions_z(inlets.vertical)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn positions(mesh: &Mesh) -> Vec<Vec3> {
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            .expect("PlaneMeshBuilder always writes positions")
            .as_float3()
            .expect("positions are stored as f32x3")
            .iter()
            .map(|&p| Vec3::from(p))
            .collect()
    }

    fn distinct_along(positions: &[Vec3], axis: impl Fn(Vec3) -> f32) -> usize {
        positions
            .iter()
            .map(|&p| (axis(p) * 10_000.0).round() as i64)
            .collect::<BTreeSet<_>>()
            .len()
    }

    #[test]
    fn independent_subdivision_counts_tessellate_the_two_axes_independently() {
        // Carried over from the old `plane_mesh` module: catches a
        // horizontal/vertical mix-up, which would swap the two densities and
        // still look plausible.
        let mesh = build_plane(&PlaneMeshIn {
            size: Vec2::ONE,
            horizontal: 3,
            vertical: 1,
        });
        let positions = positions(&mesh);
        assert_eq!(distinct_along(&positions, |p| p.x), 5);
        assert_eq!(distinct_along(&positions, |p| p.y), 3);
    }

    #[test]
    fn zero_subdivisions_on_both_axes_is_a_flat_quad_of_four_vertices() {
        let mesh = build_plane(&PlaneMeshIn {
            size: Vec2::ONE,
            horizontal: 0,
            vertical: 0,
        });
        assert_eq!(positions(&mesh).len(), 4);
    }
}
