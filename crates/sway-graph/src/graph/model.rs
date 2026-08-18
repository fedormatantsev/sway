//! The graph itself: nodes, edges, the dirty set and the invariants.
//!
//! Design D8 — nodes live in a `Vec` addressed by generational index, because
//! propagate needs `&outlets` of one node and `&mut inlets` of another at once
//! and `slice::get_disjoint_mut` gives that with no allocation. That method
//! errors on duplicate indices, which is one of the reasons self-edges are
//! refused at connect time.

use std::collections::BTreeSet;

use bevy_ecs::resource::Resource;
use bevy_reflect::PartialReflect;

use crate::graph::edge::{Compat, Edge, Port};
use crate::graph::id::{EdgeId, NodeId};
use crate::graph::legality;
use crate::graph::node::{Node, Part};
use crate::graph::order::{self, EvalOrder};
use crate::graph::path;

/// Why a connection was refused.
///
/// Every variant is discovered without evaluating anything: the graph holds
/// concrete values, so the types at both paths are known the moment the
/// connection is attempted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectError {
    /// A node may not drive itself. `slice::get_disjoint_mut` would error on
    /// the duplicate index, and the sort would report a cycle of one.
    SelfConnection,
    /// One of the two nodes does not exist (or the id is stale).
    MissingNode(NodeId),
    /// The source path does not resolve within the producer's `outlets`.
    MissingSourcePath,
    /// The destination path does not resolve within the consumer's `inlets`.
    MissingDestinationPath,
    /// Neither `D == S`, `D == Option<S>` nor `D == Vec<S>` holds.
    IncompatibleTypes {
        /// The reflected type path at the source.
        src: String,
        /// The reflected type path at the destination.
        dst: String,
    },
    /// A value at one of the paths has no static type, so the rule cannot be
    /// applied. Only reachable with dynamic values in a node.
    UntypedValue,
}

struct Slot {
    generation: u32,
    node: Option<Node>,
}

/// The authored model: nodes and edges and nothing else.
///
/// A `Resource`, not an ECS sub-world. The tick takes it out of the `World`
/// with `resource_scope`, so a node's `&World` cannot reach it.
#[derive(Resource)]
pub struct Graph {
    slots: Vec<Slot>,
    free: Vec<u32>,
    edges: Vec<Edge>,
    next_edge: u64,
    /// Nodes changed since a consumer last drained. `BTreeSet` so drains come
    /// out in a deterministic order.
    dirty: BTreeSet<NodeId>,
    /// Nodes deleted since a consumer last drained. Projectors need this to
    /// despawn; the node itself is gone by the time they look.
    removed: Vec<NodeId>,
    selection: Option<NodeId>,
    topology_dirty: bool,
    order: EvalOrder,
}

impl Default for Graph {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            edges: Vec::new(),
            next_edge: 0,
            dirty: BTreeSet::new(),
            removed: Vec::new(),
            selection: None,
            // Starts dirty so the first tick builds an order.
            topology_dirty: true,
            order: EvalOrder::default(),
        }
    }
}

impl Graph {
    // --- nodes ---------------------------------------------------------

    /// Adds a node and mints its id. The node is dirty and the topology is
    /// marked for rebuild.
    pub fn insert(&mut self, node: Node) -> NodeId {
        let id = match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                slot.node = Some(node);
                NodeId::new(index, slot.generation)
            }
            None => {
                let index = u32::try_from(self.slots.len()).expect("node index fits in u32");
                self.slots.push(Slot {
                    generation: 0,
                    node: Some(node),
                });
                NodeId::new(index, 0)
            }
        };
        self.dirty.insert(id);
        self.topology_dirty = true;
        id
    }

    /// Removes a node and **every edge naming it, in either direction**, so
    /// the graph is never left holding an edge that names a node which does
    /// not exist.
    ///
    /// Bumping the slot's generation is what makes the removed id stop
    /// resolving, even after the slot is reused.
    pub fn remove(&mut self, id: NodeId) -> Option<Node> {
        if !self.contains(id) {
            return None;
        }
        let index = id.index() as usize;
        let node = self.slots[index].node.take();
        self.slots[index].generation = self.slots[index].generation.wrapping_add(1);
        self.free.push(id.index());
        self.edges
            .retain(|edge| edge.src.node != id && edge.dst.node != id);
        self.dirty.remove(&id);
        self.removed.push(id);
        if self.selection == Some(id) {
            self.selection = None;
        }
        self.topology_dirty = true;
        node
    }

    /// Whether `id` names a live node.
    pub fn contains(&self, id: NodeId) -> bool {
        self.slot(id).is_some()
    }

    fn slot(&self, id: NodeId) -> Option<&Slot> {
        let slot = self.slots.get(id.index() as usize)?;
        (slot.generation == id.generation() && slot.node.is_some()).then_some(slot)
    }

    /// The node `id` names, if it is live.
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.slot(id)?.node.as_ref()
    }

    /// The node `id` names, mutably. Does **not** dirty — the caller decides,
    /// because an equal write must report nothing.
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let slot = self.slots.get_mut(id.index() as usize)?;
        if slot.generation != id.generation() {
            return None;
        }
        slot.node.as_mut()
    }

    /// Every live node, in ascending id order.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> + '_ {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            let node = slot.node.as_ref()?;
            Some((NodeId::new(index as u32, slot.generation), node))
        })
    }

    /// Every live node's id, in ascending order.
    pub fn node_ids(&self) -> Vec<NodeId> {
        self.iter().map(|(id, _)| id).collect()
    }

    /// How many live nodes there are.
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|slot| slot.node.is_some()).count()
    }

    /// Whether the graph holds no nodes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    // --- edges ---------------------------------------------------------

    /// Every edge, in creation order.
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// One edge by id.
    pub fn edge(&self, id: EdgeId) -> Option<&Edge> {
        self.edges.iter().find(|edge| edge.id == id)
    }

    /// Edges arriving at `node`.
    pub fn edges_into(&self, node: NodeId) -> impl Iterator<Item = &Edge> + '_ {
        self.edges.iter().filter(move |edge| edge.dst.node == node)
    }

    /// Edges leaving `node`.
    pub fn edges_from(&self, node: NodeId) -> impl Iterator<Item = &Edge> + '_ {
        self.edges.iter().filter(move |edge| edge.src.node == node)
    }

    /// Connects an outlet to an inlet, enforcing every invariant
    /// `Relationship` used to provide (design D8).
    ///
    /// Legality is decided here, from the reflected types at the two paths —
    /// nothing is evaluated to discover a refusal. A single-connection inlet is
    /// **replaced**, not doubled; a variadic (`Vec<S>`) inlet accumulates.
    pub fn connect(&mut self, src: Port, dst: Port, slot: i32) -> Result<EdgeId, ConnectError> {
        if src.node == dst.node {
            return Err(ConnectError::SelfConnection);
        }
        if !self.contains(src.node) {
            return Err(ConnectError::MissingNode(src.node));
        }
        if !self.contains(dst.node) {
            return Err(ConnectError::MissingNode(dst.node));
        }

        let src_node = self.get(src.node).expect("checked above");
        let src_value = path::resolve(src_node, Part::Outlets, &src.path)
            .ok_or(ConnectError::MissingSourcePath)?;
        let src_info = src_value
            .get_represented_type_info()
            .ok_or(ConnectError::UntypedValue)?;
        let valueless = legality::is_valueless(src_value);

        let dst_node = self.get(dst.node).expect("checked above");
        let dst_value = path::resolve(dst_node, Part::Inlets, &dst.path)
            .ok_or(ConnectError::MissingDestinationPath)?;
        let dst_info = dst_value
            .get_represented_type_info()
            .ok_or(ConnectError::UntypedValue)?;

        let compat = legality::compatibility(src_info, dst_info).ok_or_else(|| {
            ConnectError::IncompatibleTypes {
                src: src_info.type_path().to_owned(),
                dst: dst_info.type_path().to_owned(),
            }
        })?;

        if compat != Compat::Variadic {
            // At most one source per inlet: rewire evicts rather than doubling.
            self.edges
                .retain(|edge| !(edge.dst.node == dst.node && edge.dst.path == dst.path));
        }

        let id = EdgeId::new(self.next_edge);
        self.next_edge += 1;
        let dst_node_id = dst.node;
        self.edges.push(Edge {
            id,
            src,
            dst,
            slot,
            compat,
            valueless,
        });
        self.dirty.insert(dst_node_id);
        self.topology_dirty = true;
        Ok(id)
    }

    /// Removes one edge. `true` when it existed.
    pub fn disconnect(&mut self, id: EdgeId) -> bool {
        let Some(position) = self.edges.iter().position(|edge| edge.id == id) else {
            return false;
        };
        let edge = self.edges.remove(position);
        self.dirty.insert(edge.dst.node);
        self.topology_dirty = true;
        true
    }

    /// Changes one edge's ordering key. Nothing else is touched, which is the
    /// point of a sort key: reordering a variadic inlet is one write.
    pub fn set_slot(&mut self, id: EdgeId, slot: i32) -> bool {
        let Some(edge) = self.edges.iter_mut().find(|edge| edge.id == id) else {
            return false;
        };
        if edge.slot == slot {
            return false;
        }
        edge.slot = slot;
        let dst = edge.dst.node;
        self.dirty.insert(dst);
        self.topology_dirty = true;
        true
    }

    // --- dirty tracking ------------------------------------------------

    /// Records `node` as changed. Ignored for a node that does not exist.
    pub fn mark_dirty(&mut self, node: NodeId) {
        if self.contains(node) {
            self.dirty.insert(node);
        }
    }

    /// Whether `node` is currently recorded as changed.
    pub fn is_dirty(&self, node: NodeId) -> bool {
        self.dirty.contains(&node)
    }

    /// Peeks at the dirty set without clearing it, in ascending id order, so
    /// several consumers can read it before one drains.
    pub fn dirty(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.dirty.iter().copied()
    }

    /// Takes the dirty set and clears it.
    pub fn drain_dirty(&mut self) -> Vec<NodeId> {
        core::mem::take(&mut self.dirty).into_iter().collect()
    }

    /// Peeks at the removed set without clearing it.
    pub fn removed(&self) -> &[NodeId] {
        &self.removed
    }

    /// Takes the removed set and clears it.
    pub fn drain_removed(&mut self) -> Vec<NodeId> {
        core::mem::take(&mut self.removed)
    }

    // --- selection -----------------------------------------------------

    /// The node the editor is currently pointed at.
    pub fn selection(&self) -> Option<NodeId> {
        self.selection
    }

    /// Points the editor at a node. Selection is not node content, so it does
    /// not dirty anything.
    pub fn set_selection(&mut self, node: Option<NodeId>) {
        self.selection = match node {
            Some(id) if !self.contains(id) => None,
            other => other,
        };
    }

    // --- order ---------------------------------------------------------

    /// Whether the shape changed since the last rebuild.
    pub fn topology_dirty(&self) -> bool {
        self.topology_dirty
    }

    /// Marks the shape as changed, forcing a rebuild on the next tick.
    pub fn mark_topology_dirty(&mut self) {
        self.topology_dirty = true;
    }

    /// The current evaluation plan.
    pub fn order(&self) -> &EvalOrder {
        &self.order
    }

    /// Rebuilds the evaluation plan if the shape changed.
    pub fn rebuild_order_if_dirty(&mut self) {
        if self.topology_dirty {
            self.rebuild_order();
        }
    }

    /// Rebuilds the evaluation plan unconditionally.
    pub fn rebuild_order(&mut self) {
        self.order = order::rebuild(self);
        self.topology_dirty = false;
    }

    // --- internals used by the tick ------------------------------------

    /// Two node slots at once, checked against their generations.
    ///
    /// `slice::get_disjoint_mut` is what makes propagate allocation-free; it
    /// errors on duplicate indices, which self-edge rejection guarantees will
    /// not happen.
    pub(crate) fn pair_mut(&mut self, a: NodeId, b: NodeId) -> Option<(&Node, &mut Node)> {
        if a.index() == b.index() {
            return None;
        }
        let [slot_a, slot_b] = self
            .slots
            .get_disjoint_mut([a.index() as usize, b.index() as usize])
            .ok()?;
        if slot_a.generation != a.generation() || slot_b.generation != b.generation() {
            return None;
        }
        Some((slot_a.node.as_ref()?, slot_b.node.as_mut()?))
    }

    pub(crate) fn take_order(&mut self) -> EvalOrder {
        core::mem::take(&mut self.order)
    }

    pub(crate) fn put_order(&mut self, order: EvalOrder) {
        self.order = order;
    }

    pub(crate) fn dirty_insert(&mut self, node: NodeId) {
        self.dirty.insert(node);
    }
}

/// Whether two reflected values compare equal. `None` from
/// `reflect_partial_eq` — a type that cannot answer — is treated as *not*
/// equal, so a write happens and the node dirties, which is the safe side.
pub(crate) fn reflect_equal(a: &dyn PartialReflect, b: &dyn PartialReflect) -> bool {
    a.reflect_partial_eq(b) == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::testing::{Counter, Fan, Sink, Source};
    use bevy_math::Vec2;

    fn source_and_counter() -> (Graph, NodeId, NodeId) {
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let b = graph.insert(Node::of(Vec2::ZERO, Counter::default()));
        (graph, a, b)
    }

    // --- 1.1 identity ---------------------------------------------------

    #[test]
    fn a_deleted_id_never_resolves_to_a_later_node() {
        let mut graph = Graph::default();
        let first = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        graph.remove(first);
        let second = graph.insert(Node::of(Vec2::ZERO, Source::default()));

        assert_eq!(
            first.index(),
            second.index(),
            "the slot really was reused, or this test proves nothing"
        );
        assert_ne!(first, second);
        assert!(graph.get(first).is_none());
        assert!(graph.get(second).is_some());
    }

    #[test]
    fn ids_are_stable_while_the_node_lives() {
        let (mut graph, a, b) = source_and_counter();
        graph.remove(a);
        assert!(graph.get(b).is_some(), "deleting a sibling did not move b");
    }

    // --- 1.5 dirty ------------------------------------------------------

    #[test]
    fn creating_a_node_dirties_only_that_node() {
        let (mut graph, a, b) = source_and_counter();
        graph.drain_dirty();
        let c = graph.insert(Node::of(Vec2::ZERO, Sink::default()));
        assert_eq!(graph.drain_dirty(), vec![c]);
        assert!(!graph.is_dirty(a) && !graph.is_dirty(b));
    }

    #[test]
    fn draining_clears_the_dirty_set() {
        let (mut graph, _, _) = source_and_counter();
        assert_eq!(graph.drain_dirty().len(), 2);
        assert!(graph.drain_dirty().is_empty());
    }

    #[test]
    fn deleting_a_node_records_it_as_removed_and_not_as_dirty() {
        let (mut graph, a, _) = source_and_counter();
        graph.drain_dirty();
        graph.remove(a);
        assert!(graph.drain_dirty().is_empty());
        assert_eq!(graph.drain_removed(), vec![a]);
    }

    // --- 2.2 / 2.4 connect ---------------------------------------------

    #[test]
    fn a_self_connection_is_refused() {
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Counter::default()));
        assert_eq!(
            graph.connect(Port::new(a, "total"), Port::new(a, "step"), 0),
            Err(ConnectError::SelfConnection)
        );
        assert!(graph.edges().is_empty());
    }

    #[test]
    fn an_illegal_connection_is_refused_without_evaluating_anything() {
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let b = graph.insert(Node::of(Vec2::ZERO, Sink::default()));
        // `Source.outlets.out` is f32; `Sink.inlets.flag` is bool.
        let error = graph
            .connect(Port::new(a, "out"), Port::new(b, "flag"), 0)
            .expect_err("f32 does not fit bool");
        assert!(matches!(error, ConnectError::IncompatibleTypes { .. }));
        assert!(graph.edges().is_empty());
        assert!(graph.order().steps.is_empty(), "nothing was evaluated");
    }

    #[test]
    fn an_unknown_path_is_refused() {
        let (mut graph, a, b) = source_and_counter();
        assert_eq!(
            graph.connect(Port::new(a, "nope"), Port::new(b, "step"), 0),
            Err(ConnectError::MissingSourcePath)
        );
        assert_eq!(
            graph.connect(Port::new(a, "out"), Port::new(b, "nope"), 0),
            Err(ConnectError::MissingDestinationPath)
        );
    }

    #[test]
    fn a_stale_node_id_is_refused() {
        let (mut graph, a, b) = source_and_counter();
        graph.remove(a);
        assert_eq!(
            graph.connect(Port::new(a, "out"), Port::new(b, "step"), 0),
            Err(ConnectError::MissingNode(a))
        );
    }

    #[test]
    fn a_single_connection_inlet_is_replaced_not_doubled() {
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let b = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let c = graph.insert(Node::of(Vec2::ZERO, Counter::default()));
        graph
            .connect(Port::new(a, "out"), Port::new(c, "step"), 0)
            .expect("legal");
        let second = graph
            .connect(Port::new(b, "out"), Port::new(c, "step"), 0)
            .expect("legal");

        assert_eq!(graph.edges().len(), 1);
        assert_eq!(graph.edges()[0].id, second);
        assert_eq!(graph.edges()[0].src.node, b);
    }

    #[test]
    fn a_variadic_inlet_accumulates() {
        let mut graph = Graph::default();
        let fan = graph.insert(Node::of(Vec2::ZERO, Fan::default()));
        for _ in 0..3 {
            let source = graph.insert(Node::of(Vec2::ZERO, Source::default()));
            graph
                .connect(Port::new(source, "out"), Port::new(fan, "values"), 0)
                .expect("legal");
        }
        assert_eq!(graph.edges().len(), 3);
        assert!(
            graph
                .edges()
                .iter()
                .all(|edge| edge.compat == Compat::Variadic)
        );
    }

    #[test]
    fn an_optional_inlet_takes_the_bare_source_type() {
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let b = graph.insert(Node::of(Vec2::ZERO, Sink::default()));
        let id = graph
            .connect(Port::new(a, "out"), Port::new(b, "maybe"), 0)
            .expect("f32 -> Option<f32> is legal");
        assert_eq!(graph.edge(id).unwrap().compat, Compat::Optional);
    }

    #[test]
    fn a_marker_edge_is_recorded_as_valueless() {
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let b = graph.insert(Node::of(Vec2::ZERO, Sink::default()));
        let id = graph
            .connect(Port::new(a, "marker"), Port::new(b, "marker"), 0)
            .expect("legal");
        assert!(graph.edge(id).unwrap().valueless);
    }

    #[test]
    fn deleting_a_node_deletes_every_edge_naming_it() {
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let b = graph.insert(Node::of(Vec2::ZERO, Counter::default()));
        let c = graph.insert(Node::of(Vec2::ZERO, Sink::default()));
        graph
            .connect(Port::new(a, "out"), Port::new(b, "step"), 0)
            .expect("legal");
        graph
            .connect(Port::new(b, "total"), Port::new(c, "value"), 0)
            .expect("legal");
        assert_eq!(graph.edges().len(), 2);

        graph.remove(b);

        assert!(
            graph.edges().is_empty(),
            "both the inbound and the outbound edge are gone"
        );
    }

    #[test]
    fn disconnect_removes_exactly_one_edge() {
        let mut graph = Graph::default();
        let fan = graph.insert(Node::of(Vec2::ZERO, Fan::default()));
        let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let b = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let first = graph
            .connect(Port::new(a, "out"), Port::new(fan, "values"), 0)
            .expect("legal");
        graph
            .connect(Port::new(b, "out"), Port::new(fan, "values"), 1)
            .expect("legal");

        assert!(graph.disconnect(first));
        assert_eq!(graph.edges().len(), 1);
        assert!(!graph.disconnect(first), "already gone");
    }

    #[test]
    fn reordering_changes_one_edges_key_only() {
        let mut graph = Graph::default();
        let fan = graph.insert(Node::of(Vec2::ZERO, Fan::default()));
        let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let b = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let first = graph
            .connect(Port::new(a, "out"), Port::new(fan, "values"), 10)
            .expect("legal");
        let second = graph
            .connect(Port::new(b, "out"), Port::new(fan, "values"), 20)
            .expect("legal");

        assert!(graph.set_slot(first, 30));

        assert_eq!(graph.edge(first).unwrap().slot, 30);
        assert_eq!(
            graph.edge(second).unwrap().slot,
            20,
            "the other edge's key is untouched"
        );
    }

    #[test]
    fn selection_drops_when_the_selected_node_is_deleted() {
        let (mut graph, a, _) = source_and_counter();
        graph.set_selection(Some(a));
        assert_eq!(graph.selection(), Some(a));
        graph.remove(a);
        assert_eq!(graph.selection(), None);
    }
}
