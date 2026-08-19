//! The `NodeId <-> Entity` map.
//!
//! Graph shape and world shape are not assumed to match (`architecture`: "the
//! graph is the authored model and the world is derived"): only *scene* nodes
//! appear here. A producer node owns an asset and no entity, and a material
//! node owns an `Assets<M>` entry and no entity, so neither is in this map.
//!
//! The reverse direction exists for picking. Resolving a picked entity back
//! to the node that produced it carries **identity only** — no value ever
//! travels world to graph.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use sway_graph::graph::NodeId;

/// Which entity each projected scene node owns.
#[derive(Resource, Default, Debug)]
pub struct NodeEntities {
    forward: HashMap<NodeId, Entity>,
    reverse: HashMap<Entity, NodeId>,
}

impl NodeEntities {
    /// The entity a node produced, if it produced one.
    pub fn entity(&self, node: NodeId) -> Option<Entity> {
        self.forward.get(&node).copied()
    }

    /// The node that produced an entity. Identity only — this is what
    /// picking resolves through.
    pub fn node(&self, entity: Entity) -> Option<NodeId> {
        self.reverse.get(&entity).copied()
    }

    /// Records a projection. Replacing an existing entry drops the old
    /// reverse mapping so a stale entity can never resolve to a node.
    pub fn insert(&mut self, node: NodeId, entity: Entity) {
        if let Some(previous) = self.forward.insert(node, entity) {
            self.reverse.remove(&previous);
        }
        self.reverse.insert(entity, node);
    }

    /// Forgets a node's projection, returning the entity it owned.
    pub fn remove(&mut self, node: NodeId) -> Option<Entity> {
        let entity = self.forward.remove(&node)?;
        self.reverse.remove(&entity);
        Some(entity)
    }

    /// Every projection, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, Entity)> + '_ {
        self.forward.iter().map(|(node, entity)| (*node, *entity))
    }

    /// How many nodes are projected.
    pub fn len(&self) -> usize {
        self.forward.len()
    }

    /// Whether nothing is projected.
    pub fn is_empty(&self) -> bool {
        self.forward.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_a_projection_forgets_the_entity_it_replaced() {
        // A stale reverse entry would let picking resolve a despawned entity
        // to a live node, which is a selection that jumps to the wrong place.
        let mut map = NodeEntities::default();
        let node = NodeId::new(3, 1);
        let first = Entity::from_raw_u32(7).expect("a valid entity index");
        let second = Entity::from_raw_u32(9).expect("a valid entity index");

        map.insert(node, first);
        map.insert(node, second);

        assert_eq!(map.entity(node), Some(second));
        assert_eq!(map.node(first), None);
        assert_eq!(map.node(second), Some(node));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn removing_a_projection_clears_both_directions() {
        let mut map = NodeEntities::default();
        let node = NodeId::new(1, 0);
        let entity = Entity::from_raw_u32(4).expect("a valid entity index");
        map.insert(node, entity);

        assert_eq!(map.remove(node), Some(entity));
        assert_eq!(map.entity(node), None);
        assert_eq!(map.node(entity), None);
        assert!(map.is_empty());
    }
}
