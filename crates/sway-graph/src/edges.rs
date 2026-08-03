//! Node and edge components. Spec §5.

use bevy_ecs::change_detection::Tick;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_math::Vec2;
use bevy_reflect::Reflect;

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
    /// The `Inlets` change tick this node last prefilled against. `None`
    /// forces a prefill, which is how a recompile makes a disconnect take
    /// effect.
    pub last_inlets_tick: Option<Tick>,
    /// The cook gate. Sticky: set when a driven inlet changes, when prefill
    /// fires, or when an upstream product's change tick moves; cleared only
    /// by a cook that actually ran. Stickiness is what makes it survive a
    /// skipped cadence, which a `Changed<T>` filter cannot.
    pub cook_dirty: bool,
    /// Per product inlet: the source's `produced_change_tick` at this node's
    /// last cook.
    pub last_product_ticks: Vec<Option<Tick>>,
}

/// One end of an edge: a field ordinal and, for a `Vec` field, which element.
///
/// Addressing by `(field, index)` rather than a flat ordinal is what makes a
/// `Vec` resize local — inserting a child renumbers nothing outside that
/// field, so authored edges and editor widget identity survive it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Endpoint {
    /// Ordinal within the node's fields: inlets first, then outlets.
    pub field: u16,
    /// Element within a `Vec` field. Always 0 for a non-`Vec` field.
    pub index: u16,
}

impl Endpoint {
    /// The single slot of a non-`Vec` field.
    pub fn field(field: u16) -> Self {
        Self { field, index: 0 }
    }
}

/// The one edge. Carries nothing but its two endpoints — what an edge *does*
/// is decided by the type of the inlet it lands on (design §2).
///
/// An entity, so Bevy maintains the reverse index and `linked_spawn` on
/// `EdgeFrom`/`EdgeTo` makes despawning a node despawn its edges.
#[derive(Component, Clone, Copy, Debug)]
pub struct Edge {
    pub from: Endpoint,
    pub to: Endpoint,
}

/// Where the editor draws this node, in graph-canvas space.
///
/// Authored in the graph builder today, serialized by M4's project format,
/// and written back by drag at M7 — which is why it is a component on the
/// node entity rather than a field in whatever built the graph.
///
/// The editor treats this as a *seed*, read once when a node box first
/// appears and never again, so that a node dragged in-session is not snapped
/// back by the next frame's snapshot. Absent means "no authored position":
/// the editor falls back to a deterministic grid slot.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
pub struct EditorPos(pub Vec2);

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

#[cfg(test)]
mod tests {
    use super::EditorPos;
    use bevy_ecs::world::World;
    use bevy_math::Vec2;

    #[test]
    fn editor_pos_is_a_component_readable_off_a_node_entity() {
        let mut world = World::new();
        let entity = world.spawn(EditorPos(Vec2::new(20.0, 140.0))).id();
        assert_eq!(world.get::<EditorPos>(entity).map(|p| p.0), Some(Vec2::new(20.0, 140.0)));
    }

    #[test]
    fn an_edge_addresses_an_element_of_a_field() {
        use super::{Edge, Endpoint};

        let edge = Edge {
            from: Endpoint::field(2),
            to: Endpoint { field: 0, index: 3 },
        };

        // A non-Vec field is element 0 of itself, so one addressing scheme
        // covers both cases and callers never branch on variadic-ness.
        assert_eq!(edge.from.index, 0);
        assert_eq!(edge.to.field, 0);
        assert_eq!(edge.to.index, 3);
    }
}
