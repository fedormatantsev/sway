//! Node and edge components. Spec §5.

use bevy_ecs::change_detection::Tick;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;

use crate::registry::NodeTypeId;

/// Stable authored identity, used by M4's reconcile. Carried now because it
/// costs nothing and the loader will need it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

#[derive(Component)]
pub struct GraphNode {
    pub id: NodeId,
    pub node_type: NodeTypeId,
}

/// Engine-owned, inserted by `compile`. Spec §4.
#[derive(Component, Default)]
pub struct NodeRuntime {
    pub continuous_base: usize,
    pub event_base: usize,
    /// The `Params` change tick this node last prefilled against. `None`
    /// forces a prefill, which is how a recompile makes a disconnect take
    /// effect.
    pub last_params_tick: Option<Tick>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PortKind {
    Continuous,
    Event,
}

/// A param edge is an entity (spec §5), so Bevy maintains the reverse index
/// and `linked_spawn` below makes despawning a node despawn its edges.
#[derive(Component)]
pub struct ParamEdge {
    /// Ordinal within the source node's kind-space.
    pub source_port: u16,
    /// Ordinal within the target node's kind-space.
    pub target_port: u16,
    pub kind: PortKind,
}

#[derive(Component)]
#[relationship(relationship_target = OutEdges)]
pub struct EdgeFrom(#[entities] pub Entity);

#[derive(Component)]
#[relationship_target(relationship = EdgeFrom, linked_spawn)]
pub struct OutEdges(Vec<Entity>);

#[derive(Component)]
#[relationship(relationship_target = InEdges)]
pub struct EdgeTo(#[entities] pub Entity);

#[derive(Component)]
#[relationship_target(relationship = EdgeTo, linked_spawn)]
pub struct InEdges(Vec<Entity>);
