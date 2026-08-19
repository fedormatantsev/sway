//! Live `Graph` -> document. The inverse of `crate::v3::load`; proves the
//! format complete the same way `crate::emit` does for version 2.
//!
//! Not an asset-pipeline operation (design D1): the caller runs
//! `to_document` then `to_ron` then `fs::write` — see [`crate::v3::save_to_path`]
//! — never `AssetServer::save` (which does not exist) and never a write
//! through the `ProjectAsset`/`GraphAsset` the graph was loaded from. The
//! asset is a loading mechanism only; it is never consulted again once the
//! live `Graph` exists.

use std::collections::BTreeMap;

use bevy_reflect::TypeRegistry;
use bevy_reflect::serde::TypedReflectSerializer;
use sway_graph::graph::{Graph, Part};

use crate::v3::doc::{EdgeDoc, FORMAT_VERSION, GraphDoc, NodeDoc};
use crate::v3::ids::StableIds;

#[derive(Debug, Clone, PartialEq)]
pub enum SaveError {
    /// A node's `inlets` part failed to serialize or was not registered
    /// well enough for `ron` to turn its reflected value into text.
    Inlets { id: String, message: String },
}

impl core::fmt::Display for SaveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Inlets { id, message } => write!(f, "{id}.inlets: {message}"),
        }
    }
}

impl core::error::Error for SaveError {}

/// Serializes `graph` into a `GraphDoc`: inlets only, never state or outlets
/// (spec: "A document stores inlets only"); a node's kind is written as its
/// short name, never a full module path (design D9).
///
/// Mints a stable id for any live node `ids` does not already have one for —
/// a node created since the last load or save — before reading a single
/// node, so every node in the graph ends up in the document.
pub fn to_document(
    graph: &Graph,
    registry: &TypeRegistry,
    ids: &mut StableIds,
) -> Result<GraphDoc, SaveError> {
    ids.mint_missing(graph);

    let mut nodes = BTreeMap::new();
    for (node_id, node) in graph.iter() {
        let id = ids
            .id_of(node_id)
            .expect("mint_missing assigned one above")
            .to_string();

        let inlets = node
            .part(Part::Inlets)
            .expect("a live node always has an inlets part, even if it is ()");
        let text =
            ron::to_string(&TypedReflectSerializer::new(inlets, registry)).map_err(|error| {
                SaveError::Inlets {
                    id: id.clone(),
                    message: error.to_string(),
                }
            })?;
        let payload =
            ron::value::RawValue::from_boxed_ron(text.into_boxed_str()).map_err(|error| {
                SaveError::Inlets {
                    id: id.clone(),
                    message: error.to_string(),
                }
            })?;

        let pos = node.pos();
        nodes.insert(
            id,
            NodeDoc {
                kind: short_name(node.kind()),
                pos: (pos.x, pos.y),
                inlets: payload,
            },
        );
    }

    let mut edges: Vec<EdgeDoc> = Vec::with_capacity(graph.edges().len());
    for edge in graph.edges() {
        let from_id = ids
            .id_of(edge.src.node)
            .expect("an edge only ever names a live node")
            .to_string();
        let to_id = ids
            .id_of(edge.dst.node)
            .expect("an edge only ever names a live node")
            .to_string();
        edges.push(EdgeDoc {
            from: (from_id, edge.src.path.clone()),
            to: (to_id, edge.dst.path.clone()),
            slot: edge.slot,
        });
    }
    // Deterministic regardless of the edge list's internal storage order, so
    // an unrelated edit elsewhere in the graph does not reshuffle unrelated
    // lines (spec: "Connection order is preserved" is about slot order within
    // one inlet; this is what keeps every *other* line stable too).
    edges.sort_by(|a, b| (&a.to, a.slot, &a.from).cmp(&(&b.to, b.slot, &b.from)));

    Ok(GraphDoc {
        version: FORMAT_VERSION,
        nodes,
        edges,
    })
}

/// The last `::`-segment of a node's full reflected `TypePath` — the inverse
/// of `crate::v3::load`'s short-name resolution.
fn short_name(full_path: &str) -> String {
    full_path
        .rsplit("::")
        .next()
        .unwrap_or(full_path)
        .to_string()
}

/// One node per line, one edge per line — design.md's explicit goal ("a
/// format meant to be read and occasionally hand-edited"). `depth_limit(2)`
/// is what enforces it: `ron`'s pretty-printer multi-lines a composite's body
/// only while its nesting depth is `<= depth_limit`, so depth 1 (`Graph`'s own
/// fields: `version` / `nodes` / `edges`) and depth 2 (the `nodes` map's
/// entries, the `edges` list's elements) each get one line per item, while
/// depth 3 — a `Node` or `Edge` struct's own fields — collapses to a single
/// compact line, and everything nested inside that (the `pos` tuple, the
/// `from`/`to` tuples) stays compact with it. The `inlets` payload text is
/// unaffected either way: `ron::value::RawValue` writes its stored string
/// verbatim, bypassing the pretty-printer's recursion entirely.
pub fn to_ron(doc: &GraphDoc) -> Result<String, ron::Error> {
    let config = ron::ser::PrettyConfig::new()
        .struct_names(true)
        .depth_limit(2)
        .indentor("    ")
        .compact_arrays(false);
    ron::ser::to_string_pretty(doc, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::doc::parse;
    use crate::v3::load::load;
    use bevy_math::Vec2;
    use bevy_reflect::Reflect;
    use sway_graph::graph::{Node, ReflectNodeKind, register_node_kind};

    #[derive(Reflect, Default, Debug)]
    struct GainIn {
        factor: f32,
    }
    #[derive(Reflect, Default, Debug)]
    struct GainOut {
        out: f32,
    }
    #[derive(Reflect, Default, Debug)]
    #[reflect(NodeKind)]
    struct Gain {
        inlets: GainIn,
        state: (),
        outlets: GainOut,
    }
    impl sway_graph::graph::NodeKind for Gain {
        fn evaluate(&mut self, _world: &bevy_ecs::world::World) {
            self.outlets.out = self.inlets.factor;
        }
    }

    fn registry() -> TypeRegistry {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Gain>(&mut registry);
        registry.register::<GainIn>();
        registry.register::<GainOut>();
        registry
    }

    #[test]
    fn a_graph_emits_the_document_that_built_it() {
        let registry = registry();
        let mut graph = Graph::default();
        let id = graph.insert(Node::of(
            Vec2::new(1.0, 2.0),
            Gain {
                inlets: GainIn { factor: 0.5 },
                ..Default::default()
            },
        ));
        let mut ids = StableIds::new();
        ids.assign("gainA".to_string(), id);

        let doc = to_document(&graph, &registry, &mut ids).expect("serializes");
        assert_eq!(doc.version, FORMAT_VERSION);
        assert_eq!(doc.nodes["gainA"].kind, "Gain");
        assert_eq!(doc.nodes["gainA"].pos, (1.0, 2.0));
    }

    #[test]
    fn the_emitted_text_reparses_into_the_same_document() {
        let registry = registry();
        let mut graph = Graph::default();
        let id = graph.insert(Node::of(Vec2::ZERO, Gain::default()));
        let mut ids = StableIds::new();
        ids.assign("gainA".to_string(), id);

        let doc = to_document(&graph, &registry, &mut ids).expect("serializes");
        let text = to_ron(&doc).expect("emits");
        let reparsed = parse(&text).expect("the emitter writes what the parser reads");

        assert_eq!(reparsed, doc);
    }

    #[test]
    fn each_node_and_edge_gets_its_own_line() {
        let registry = registry();
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Gain::default()));
        let b = graph.insert(Node::of(Vec2::new(10.0, 0.0), Gain::default()));
        graph
            .connect(
                sway_graph::graph::Port::new(a, "out"),
                sway_graph::graph::Port::new(b, "factor"),
                0,
            )
            .expect("legal");
        let mut ids = StableIds::new();
        ids.assign("a".to_string(), a);
        ids.assign("b".to_string(), b);

        let text =
            to_ron(&to_document(&graph, &registry, &mut ids).expect("serializes")).expect("emits");

        let node_line = text
            .lines()
            .find(|line| line.contains("\"a\""))
            .expect("node a is written");
        assert!(
            node_line.contains("Gain") && node_line.contains("factor"),
            "the whole node is on one line: {node_line}"
        );
        let edge_line_count = text
            .lines()
            .filter(|line| line.contains("\"a\"") && line.contains("\"out\""))
            .count();
        assert_eq!(edge_line_count, 1, "the edge is one line");
    }

    #[test]
    fn saving_omits_state_and_outlets() {
        let registry = registry();
        let mut graph = Graph::default();
        let id = graph.insert(Node::of(Vec2::ZERO, Gain::default()));
        let mut ids = StableIds::new();
        ids.assign("gainA".to_string(), id);

        // Simulate a tick having populated the outlet.
        graph.get_mut(id).unwrap().value_mut().apply(&Gain {
            inlets: GainIn { factor: 0.5 },
            state: (),
            outlets: GainOut { out: 0.5 },
        });

        let doc = to_document(&graph, &registry, &mut ids).expect("serializes");
        let text = ron::to_string(&doc.nodes["gainA"].inlets).expect("re-serializes");
        assert!(
            !text.contains("out"),
            "the outlet field must not appear in the saved payload: {text}"
        );
    }

    #[test]
    fn deleting_a_node_leaves_other_ids_and_edges_untouched() {
        let registry = registry();
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Gain::default()));
        let b = graph.insert(Node::of(Vec2::new(5.0, 0.0), Gain::default()));
        let c = graph.insert(Node::of(Vec2::new(10.0, 0.0), Gain::default()));
        graph
            .connect(
                sway_graph::graph::Port::new(b, "out"),
                sway_graph::graph::Port::new(c, "factor"),
                0,
            )
            .expect("legal");
        let mut ids = StableIds::new();
        ids.assign("a".to_string(), a);
        ids.assign("b".to_string(), b);
        ids.assign("c".to_string(), c);

        let before = to_document(&graph, &registry, &mut ids).expect("serializes");

        graph.remove(a);
        let after = to_document(&graph, &registry, &mut ids).expect("serializes");

        assert!(!after.nodes.contains_key("a"));
        assert_eq!(after.nodes["b"], before.nodes["b"]);
        assert_eq!(after.nodes["c"], before.nodes["c"]);
        assert_eq!(after.edges, before.edges);
    }

    #[test]
    fn load_save_reload_is_identical() {
        // The precise guarantee task 6.7 asks for: not that saving reproduces
        // a hand-authored file byte-for-byte (a semantic round trip through
        // `TypedReflectDeserializer`/`Serializer` does not promise that — a
        // payload's *formatting* is not preserved, only its meaning, exactly
        // as version 2's `crate::emit` accepts for component payloads), but
        // that loading what was just saved reproduces the same document
        // `crate::emit`'s `document_to_world_to_document_is_stable` proves the
        // same property for version 2.
        let registry = registry();
        let text = r#"Graph(
    version: 3,
    nodes: {
        "a": Node(type: "Gain", pos: (0.0, 0.0), inlets: (factor: 1.0)),
        "b": Node(type: "Gain", pos: (10.0, 0.0), inlets: (factor: 2.0)),
    },
    edges: [
        Edge(from: ("a", "out"), to: ("b", "factor"), slot: 0),
    ],
)"#;
        let doc = parse(text).expect("parses");
        let (graph, mut ids, diagnostics) = load(&doc, &registry);
        assert!(diagnostics.is_clean(), "{diagnostics:?}");

        let once = to_document(&graph, &registry, &mut ids).expect("serializes");
        let reparsed = parse(&to_ron(&once).expect("emits"))
            .expect("the emitter writes what the parser reads");
        let (graph2, mut ids2, diagnostics2) = load(&reparsed, &registry);
        assert!(diagnostics2.is_clean(), "{diagnostics2:?}");
        let twice = to_document(&graph2, &registry, &mut ids2).expect("serializes");

        assert_eq!(once, twice, "load, save, reload must be identical");
    }
}
