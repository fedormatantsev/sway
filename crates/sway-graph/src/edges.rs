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
    /// The cook gate (design §6). Sticky: set when a driven input changes,
    /// when prefill fires, or when an upstream product's change tick moves;
    /// cleared only by a cook that actually ran. Stickiness is what makes it
    /// survive a skipped cadence, which a `Changed<T>` filter cannot.
    pub cook_dirty: bool,
    /// Per slot ordinal: the source's `produced_change_tick` at this node's
    /// last cook.
    pub last_slot_ticks: Vec<Option<Tick>>,
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

/// A hierarchy edge. **Source is the child, target is the parent** — dataflow
/// runs leaf→root while parenting runs root→leaf (parent §2.10).
///
/// Authored as an edge entity rather than as Bevy's `ChildOf` directly, and
/// compiled into `ChildOf` once validation passes. §2.5 requires a `ChildOf`
/// fan-out to be a diagnosable error, and an entity holds exactly one
/// `ChildOf` — inserting a second replaces the first silently, so the illegal
/// state would be unrepresentable and the diagnostic unwritable (design §3).
#[derive(Component)]
pub struct ParentEdge;

/// A structural input edge into a named, typed slot on the target.
///
/// Also an edge entity, for the same diagnostic reason plus one of its own: a
/// node needs several slots at once (`Mesh` has `geo` and `material`) and one
/// Bevy relationship component per entity cannot carry two targets.
#[derive(Component)]
pub struct FeedsEdge {
    /// Ordinal within the target node type's `Slots` schema.
    pub slot: u16,
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
