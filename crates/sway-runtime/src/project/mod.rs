//! Projection — the graph is authored, the world is derived (design D7).
//!
//! Projection is deliberately **not** a generic mirror. Each projector knows
//! what it targets: a producer node owns an asset and no entity, a material
//! node owns an `Assets<M>` entry and no entity, a scene node owns an entity.
//! Handles are allocated **structurally** the first time a node is projected
//! and asset *contents* are updated per tick for dirty producer nodes, so a
//! connection is never waiting on a handle that does not exist yet — only its
//! content is ever pending.
//!
//! Projection is one-directional: nothing here writes back into the graph
//! except a producer's own handle, which is state no authoring surface can
//! reach.
//!
//! ## What drives each pass
//!
//! `Graph::dirty()` peeks and `drain_dirty()` clears, so several projectors
//! read the same set and exactly one consumer ([`clear_dirty`]) drains it at
//! the end of the chain.
//!
//! - **Content** — a mesh to rebuild, a material's uniform, a folder to
//!   assemble — is driven by the dirty set, in graph order. Rebuilding a 4k
//!   vertex plane every frame is exactly what the dirty set exists to avoid.
//! - **Edge-derived** state — which mesh, which material, which parent — is
//!   recomputed from the edge list every pass and written only when it
//!   differs. This is not laziness: `Graph::remove` drops every edge naming
//!   the deleted node *without dirtying the node at the other end*, so a
//!   purely dirty-driven attachment pass would leave a scene node rendering
//!   with a material whose node is gone. The comparison is what keeps it from
//!   writing, and the write is what change detection sees.
//! - **Removal** additionally marks every surviving node dirty, because a
//!   removal changes the edge set arbitrarily and removals are rare.
//!
//! ## Ordering
//!
//! Every system is in [`ProjectionSet`] and the set is `.chain()`ed:
//! producers, then materials, then scene entities, then attachment, then
//! parenting, then the dirty drain. Within a pass, dirty nodes are visited in
//! **graph order** ([`dirty_in_graph_order`]) — which is what a marker edge
//! buys: `FrameSequence -> SpriteMaterial` is a sort constraint even though
//! it propagates nothing, so the sequence's texture exists before the
//! material asks for it.
//!
//! `sway-app` gates the whole set on assets being loaded (task 8.5) with
//! `.run_if(..)` on [`ProjectionSet`]; the MIDI drain stays ungated.

use bevy::prelude::*;
use sway_graph::graph::{Graph, NodeId};

pub mod entities;
pub mod materials;
pub mod producers;
pub mod scene;

pub use entities::NodeEntities;
pub use materials::{project_pbr_materials, project_sprite_materials};
pub use producers::{project_frame_sequences, project_mesh_assets, project_plane_meshes};
pub use scene::{
    MaterialAttachment, attach_materials, despawn_removed_nodes, project_children,
    project_scene_entities, spawn_scene_entities,
};

/// Every projection system. `sway-app` hangs the load gate off this set.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProjectionSet;

/// The dirty nodes, in evaluation order.
///
/// A node the order does not mention (a graph whose order has not been
/// rebuilt yet) sorts last, by ascending id, so the pass is still total and
/// still deterministic.
pub fn dirty_in_graph_order(graph: &Graph) -> Vec<NodeId> {
    let order = &graph.order().order;
    let mut dirty: Vec<NodeId> = graph.dirty().collect();
    dirty.sort_by_key(|node| {
        (
            order
                .iter()
                .position(|other| other == node)
                .unwrap_or(usize::MAX),
            *node,
        )
    });
    dirty
}

/// Every node, in evaluation order, whether dirty or not.
pub fn nodes_in_graph_order(graph: &Graph) -> Vec<NodeId> {
    let mut nodes = graph.node_ids();
    let order = &graph.order().order;
    nodes.sort_by_key(|node| {
        (
            order
                .iter()
                .position(|other| other == node)
                .unwrap_or(usize::MAX),
            *node,
        )
    });
    nodes
}

/// The node feeding one inlet, if anything does.
///
/// Marker inlets are non-variadic, so there is at most one. Ties (which
/// cannot happen through [`Graph::connect`], but can be constructed) break by
/// the edge's own sort key, so the answer is deterministic either way.
pub fn source_of(graph: &Graph, node: NodeId, path: &str) -> Option<NodeId> {
    graph
        .edges_into(node)
        .filter(|edge| edge.dst.path == path)
        .min_by_key(|edge| edge.sort_key())
        .map(|edge| edge.src.node)
}

/// Rebuilds the evaluation order if the topology moved.
///
/// The tick already does this, but the tick lives in `FixedUpdate` and may
/// not have run since the last command was applied. Projector ordering is
/// only worth anything against a current order.
pub fn prepare_projection(mut graph: ResMut<Graph>) {
    graph.rebuild_order_if_dirty();
}

/// Clears the dirty set. Exactly one consumer may do this, and it is the last
/// system in the chain so every projector has already peeked.
pub fn clear_dirty(mut graph: ResMut<Graph>) {
    graph.drain_dirty();
}

/// Schedules the projectors.
///
/// They run in `Update`, after `FixedUpdate`'s tick, so a node's inlets have
/// already been propagated and evaluated when they are read.
pub struct ProjectionPlugin;

impl Plugin for ProjectionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NodeEntities>().add_systems(
            Update,
            (
                prepare_projection,
                despawn_removed_nodes,
                // Producers first: a handle must exist before a consumer
                // looks for it, in the same pass.
                project_mesh_assets,
                project_plane_meshes,
                project_frame_sequences,
                // Then the materials, which read the sequences above.
                project_pbr_materials,
                project_sprite_materials,
                // Then the world.
                spawn_scene_entities,
                project_scene_entities,
                attach_materials,
                project_children,
                clear_dirty,
            )
                .chain()
                .in_set(ProjectionSet),
        );

        // The displacement-aware bounds for sprite materials. Owned by
        // `SpriteMaterialPlugin` while the wire model is still around; added
        // here when it is not, so a projected sprite plane is bounded either
        // way.
        if !app.is_plugin_added::<crate::sprite_material::SpriteMaterialPlugin>() {
            app.add_systems(
                Update,
                crate::sprite_material::sync_sprite_material_bounds
                    .after(ProjectionSet)
                    .run_if(resource_exists::<Assets<Mesh>>),
            );
        }
    }
}
