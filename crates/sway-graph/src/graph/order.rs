//! Evaluation order over `NodeId`.
//!
//! Ported from the entity-based `crate::order`, with `Entity` swapped for
//! [`NodeId`]. Two things changed with the vertex type:
//!
//! - `NodeId::Ord` is ascending, so the ready heap is a plain `Reverse`-wrapped
//!   min-heap. The entity version had to compensate for `Entity::Ord` running
//!   *descending* in raw index.
//! - **The unit of ordering is the node.** `evaluate` reads every inlet and
//!   writes every outlet, so every outlet of a node genuinely depends on every
//!   one of its inlets. There is no finer vertex that could report a cycle the
//!   node does not actually have, which is why the old false-cycle caveat is
//!   gone rather than restated.
//!
//! A cycle never stops the tick: its members are appended after the acyclic
//! part and read the previous tick's values.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};

use bevy_reflect::{ParsedPath, PartialReflect, ReflectRef};

use crate::graph::edge::Compat;
use crate::graph::id::NodeId;
use crate::graph::model::Graph;
use crate::graph::node::Part;
use crate::graph::path;

/// One ordering constraint, flattened for the sort. A valueless edge produces
/// a `Link` exactly like a value edge — that is what makes a marker connection
/// order its two nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Link {
    /// The producing node.
    pub src: NodeId,
    /// The consuming node.
    pub dst: NodeId,
}

/// The result of the topological sort.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sorted {
    /// Evaluation order: the acyclic part first, then any cycle members in
    /// ascending `NodeId` order.
    pub order: Vec<NodeId>,
    /// Nodes participating in a cycle. Empty in a well-formed graph.
    pub cycles: Vec<NodeId>,
}

/// Kahn's algorithm over nodes.
///
/// Ties are broken by ascending [`NodeId`] so the order is deterministic — the
/// editor shows it and the golden traces assert on it.
pub(crate) fn topological_order(vertices: &[NodeId], links: &[Link]) -> Sorted {
    let mut indegree: HashMap<NodeId, usize> = vertices.iter().map(|&id| (id, 0)).collect();
    let mut successors: HashMap<NodeId, Vec<NodeId>> = HashMap::new();

    for link in links {
        // A link naming a node outside `vertices` is ignored: the sort orders
        // what exists.
        if !indegree.contains_key(&link.src) || !indegree.contains_key(&link.dst) {
            continue;
        }
        *indegree.get_mut(&link.dst).expect("dst is a vertex") += 1;
        successors.entry(link.src).or_default().push(link.dst);
    }

    let mut ready: BinaryHeap<Reverse<NodeId>> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| Reverse(*id))
        .collect();

    let mut order: Vec<NodeId> = Vec::with_capacity(vertices.len());
    while let Some(Reverse(id)) = ready.pop() {
        order.push(id);
        for &next in successors.get(&id).map_or(&[][..], |v| v.as_slice()) {
            let degree = indegree.get_mut(&next).expect("successor is a vertex");
            *degree -= 1;
            if *degree == 0 {
                ready.push(Reverse(next));
            }
        }
    }

    let placed: HashSet<NodeId> = order.iter().copied().collect();
    let mut cycles: Vec<NodeId> = vertices
        .iter()
        .copied()
        .filter(|id| !placed.contains(id))
        .collect();
    cycles.sort_unstable();
    order.extend(cycles.iter().copied());

    Sorted { order, cycles }
}

/// One value-carrying edge, resolved for the tick.
#[derive(Clone, Debug)]
pub(crate) struct PropagateStep {
    /// The producing node.
    pub src: NodeId,
    /// `outlets.<path>`, pre-parsed.
    pub src_path: ParsedPath,
    /// The consuming node.
    pub dst: NodeId,
    /// `inlets.<path>`, pre-parsed.
    pub(crate) dst_path: ParsedPath,
    /// How the destination accepts the value — the edge's connect-time
    /// verdict, carried through unchanged.
    pub(crate) compat: Compat,
    /// Which element of a variadic inlet this step writes. Derived from the
    /// slot sort at rebuild, and meaningless for the other two compats — which
    /// is why it is a plain index rather than a variant that only one of them
    /// can carry.
    pub(crate) index: usize,
}

/// One unit of work. Data, not a closure — the editor shows the order and the
/// tests assert on it.
#[derive(Clone, Debug)]
pub(crate) enum GraphStep {
    /// Shrink a list-shaped inlet to the number of value edges landing on it,
    /// so the `Vec` really is derived from the edge set (design D5). Growth
    /// happens in [`GraphStep::Propagate`], which pushes the value it is about
    /// to write.
    TruncateList {
        /// The node holding the list.
        node: NodeId,
        /// `inlets.<path>`, pre-parsed.
        path: ParsedPath,
        /// The number of value edges landing on it.
        len: usize,
    },
    /// Copy one outlet field into one inlet field.
    Propagate(PropagateStep),
    /// Run the node.
    Evaluate {
        /// The node to run.
        node: NodeId,
    },
}

/// The rebuilt plan for one graph shape.
#[derive(Clone, Debug, Default)]
pub(crate) struct EvalOrder {
    /// Every step, in the order the tick walks them.
    pub steps: Vec<GraphStep>,
    /// The node order the steps were derived from.
    pub order: Vec<NodeId>,
    /// Nodes in a cycle. They still run, appended after the acyclic part.
    pub cycles: Vec<NodeId>,
}

/// Every path within `value` that resolves to a list, depth-first.
///
/// Used to find the list-shaped inlets whose length is derived from the edge
/// set. Recursion stops at a list: a `Vec<Vec<T>>` inlet is one variadic port,
/// not a nested family of them.
fn collect_list_paths(value: &dyn PartialReflect, prefix: &str, out: &mut Vec<String>) {
    match value.reflect_ref() {
        ReflectRef::List(_) => out.push(prefix.to_owned()),
        ReflectRef::Struct(target) => {
            for index in 0..target.field_len() {
                let (Some(name), Some(field)) = (target.name_at(index), target.field_at(index))
                else {
                    continue;
                };
                let child = if prefix.is_empty() {
                    name.to_owned()
                } else {
                    format!("{prefix}.{name}")
                };
                collect_list_paths(field, &child, out);
            }
        }
        ReflectRef::TupleStruct(target) => {
            for index in 0..target.field_len() {
                let Some(field) = target.field(index) else {
                    continue;
                };
                let child = if prefix.is_empty() {
                    index.to_string()
                } else {
                    format!("{prefix}.{index}")
                };
                collect_list_paths(field, &child, out);
            }
        }
        _ => {}
    }
}

/// Rebuilds the evaluation plan from the graph's current shape.
///
/// Authoring-time only: the tick reads the plan and never rebuilds it.
pub(crate) fn rebuild(graph: &Graph) -> EvalOrder {
    let vertices: Vec<NodeId> = graph.iter().map(|(id, _)| id).collect();
    let links: Vec<Link> = graph
        .edges()
        .iter()
        .filter(|edge| graph.contains(edge.src.node) && graph.contains(edge.dst.node))
        .map(|edge| Link {
            src: edge.src.node,
            dst: edge.dst.node,
        })
        .collect();
    let sorted = topological_order(&vertices, &links);

    // Value edges per destination port, in slot order with `NodeId` breaking
    // ties. `BTreeMap` keeps the per-node emission order deterministic without
    // a second sort.
    let mut by_port: BTreeMap<(NodeId, &str), Vec<&crate::graph::edge::Edge>> = BTreeMap::new();
    for edge in graph.edges() {
        if !edge.carries_value() {
            // Design D6: a marker edge propagates nothing (a ZST copy is a
            // no-op) but it stayed in `links` above, so it still orders.
            continue;
        }
        if !graph.contains(edge.src.node) || !graph.contains(edge.dst.node) {
            continue;
        }
        by_port
            .entry((edge.dst.node, edge.dst.path.as_str()))
            .or_default()
            .push(edge);
    }
    for edges in by_port.values_mut() {
        edges.sort_by_key(|edge| edge.sort_key());
    }

    let mut inbound: HashMap<NodeId, Vec<(&str, &[&crate::graph::edge::Edge])>> = HashMap::new();
    for ((node, path), edges) in &by_port {
        inbound
            .entry(*node)
            .or_default()
            .push((path, edges.as_slice()));
    }

    let mut steps: Vec<GraphStep> = Vec::new();
    for node_id in &sorted.order {
        let Some(node) = graph.get(*node_id) else {
            continue;
        };

        // Every list-shaped inlet is sized from the edge set, including the
        // ones with no edges at all — otherwise a disconnect would leave a
        // stale element behind.
        if let Some(inlets) = node.part(Part::Inlets) {
            let mut list_paths = Vec::new();
            collect_list_paths(inlets, "", &mut list_paths);
            list_paths.sort();
            for list_path in list_paths {
                let len = by_port
                    .get(&(*node_id, list_path.as_str()))
                    .map_or(0, |edges| edges.len());
                if let Some(parsed) = path::parse(Part::Inlets, &list_path) {
                    steps.push(GraphStep::TruncateList {
                        node: *node_id,
                        path: parsed,
                        len,
                    });
                }
            }
        }

        for (_, edges) in inbound.get(node_id).map_or(&[][..], |v| v.as_slice()) {
            for (index, edge) in edges.iter().enumerate() {
                let (Some(src_path), Some(dst_path)) = (
                    path::parse(Part::Outlets, &edge.src.path),
                    path::parse(Part::Inlets, &edge.dst.path),
                ) else {
                    continue;
                };
                steps.push(GraphStep::Propagate(PropagateStep {
                    src: edge.src.node,
                    src_path,
                    dst: edge.dst.node,
                    dst_path,
                    compat: edge.compat,
                    index,
                }));
            }
        }

        steps.push(GraphStep::Evaluate { node: *node_id });
    }

    EvalOrder {
        steps,
        order: sorted.order,
        cycles: sorted.cycles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::edge::Port;
    use crate::graph::node::Node;
    use crate::graph::testing::{Counter, Fan, Sink, Source};

    fn n(index: u32) -> NodeId {
        NodeId::new(index, 0)
    }

    fn link(src: NodeId, dst: NodeId) -> Link {
        Link { src, dst }
    }

    #[test]
    fn a_chain_is_ordered_source_first() {
        let (a, b, c) = (n(3), n(1), n(2));
        let sorted = topological_order(&[a, b, c], &[link(a, b), link(b, c)]);
        assert_eq!(sorted.order, vec![a, b, c]);
        assert!(sorted.cycles.is_empty());
    }

    #[test]
    fn independent_nodes_are_ordered_by_id_for_determinism() {
        let sorted = topological_order(&[n(2), n(1), n(3)], &[]);
        assert_eq!(sorted.order, vec![n(1), n(2), n(3)]);
    }

    #[test]
    fn a_cycle_is_reported_and_its_members_appended() {
        let (free, a, b) = (n(1), n(2), n(3));
        let sorted = topological_order(&[free, a, b], &[link(a, b), link(b, a)]);
        assert_eq!(sorted.order, vec![free, a, b]);
        assert_eq!(sorted.cycles, vec![a, b]);
    }

    #[test]
    fn a_link_naming_an_unknown_node_is_ignored() {
        let sorted = topological_order(&[n(1)], &[link(n(77), n(1))]);
        assert_eq!(sorted.order, vec![n(1)]);
        assert!(sorted.cycles.is_empty());
    }

    #[test]
    fn two_edges_between_the_same_pair_still_order_correctly() {
        let (a, b) = (n(1), n(2));
        let sorted = topological_order(&[a, b], &[link(a, b), link(a, b)]);
        assert_eq!(sorted.order, vec![a, b]);
        assert!(sorted.cycles.is_empty());
    }

    #[test]
    fn rebuilding_twice_without_a_change_gives_the_same_order() {
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Source::default()));
        let b = graph.insert(Node::of(Counter::default()));
        graph
            .connect(Port::new(a, "out"), Port::new(b, "step"), 0)
            .expect("legal");

        let first = rebuild(&graph);
        let second = rebuild(&graph);
        assert_eq!(first.order, second.order);
        assert_eq!(shapes(&first), shapes(&second));
    }

    fn shapes(order: &EvalOrder) -> Vec<String> {
        order
            .steps
            .iter()
            .map(|step| match step {
                GraphStep::TruncateList { node, len, .. } => format!("truncate {node} -> {len}"),
                GraphStep::Propagate(step) => {
                    format!(
                        "propagate {} -> {} {:?}[{}]",
                        step.src, step.dst, step.compat, step.index
                    )
                }
                GraphStep::Evaluate { node } => format!("evaluate {node}"),
            })
            .collect()
    }

    #[test]
    fn a_propagate_step_comes_before_the_node_that_consumes_it() {
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Source::default()));
        let b = graph.insert(Node::of(Counter::default()));
        graph
            .connect(Port::new(a, "out"), Port::new(b, "step"), 0)
            .expect("legal");

        assert_eq!(
            shapes(&rebuild(&graph)),
            vec![
                format!("evaluate {a}"),
                format!("propagate {a} -> {b} Direct[0]"),
                format!("evaluate {b}"),
            ]
        );
    }

    #[test]
    fn a_valueless_edge_emits_no_propagate_step_but_still_orders() {
        let mut graph = Graph::default();
        // `Sink` is downstream of `Source` through a marker outlet.
        let a = graph.insert(Node::of(Source::default()));
        let b = graph.insert(Node::of(Sink::default()));
        graph
            .connect(Port::new(a, "marker"), Port::new(b, "marker"), 0)
            .expect("legal");

        assert_eq!(
            shapes(&rebuild(&graph)),
            vec![format!("evaluate {a}"), format!("evaluate {b}")],
            "no propagate step, but A still precedes B"
        );
    }

    #[test]
    fn variadic_edges_are_indexed_in_slot_order_not_slot_value() {
        let mut graph = Graph::default();
        let fan = graph.insert(Node::of(Fan::default()));
        let mut sources = Vec::new();
        for _ in 0..3 {
            sources.push(graph.insert(Node::of(Source::default())));
        }
        // Sparse, out-of-order slots.
        for (source, slot) in sources.iter().zip([30, 10, 20]) {
            graph
                .connect(Port::new(*source, "out"), Port::new(fan, "values"), slot)
                .expect("legal");
        }

        let order = rebuild(&graph);
        let indices: Vec<(NodeId, usize)> = order
            .steps
            .iter()
            .filter_map(|step| match step {
                GraphStep::Propagate(step) => {
                    assert_eq!(step.compat, Compat::Variadic);
                    Some((step.src, step.index))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            indices,
            vec![(sources[1], 0), (sources[2], 1), (sources[0], 2)]
        );
    }

    #[test]
    fn a_list_inlet_is_truncated_to_its_edge_count() {
        let mut graph = Graph::default();
        let fan = graph.insert(Node::of(Fan::default()));
        let source = graph.insert(Node::of(Source::default()));
        graph
            .connect(Port::new(source, "out"), Port::new(fan, "values"), 0)
            .expect("legal");

        let order = rebuild(&graph);
        let truncates: Vec<usize> = order
            .steps
            .iter()
            .filter_map(|step| match step {
                GraphStep::TruncateList { len, .. } => Some(*len),
                _ => None,
            })
            .collect();
        assert_eq!(truncates, vec![1]);
    }
}
