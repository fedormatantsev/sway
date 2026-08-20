//! Document -> live `Graph`. Spec: "A document is nodes and edges keyed by
//! stable ids" and "Unresolved ids, kinds and paths are reported and
//! skipped".
//!
//! A load builds a **fresh** `Graph` from nothing — there is no reconcile
//! pass and no claim pass (task 6.2). Every edge goes through
//! `Graph::connect`, never a hand-built `Edge`, because `connect` is what
//! decides `compat` and `valueless` (graph-core API contract §8). A node
//! whose inlets cannot be read is skipped whole: this module never inserts a
//! node into the graph before its inlets have been fully deserialized and
//! applied.

use std::collections::HashMap;

use bevy_reflect::serde::{ReflectDeserializer, TypedReflectDeserializer};
use bevy_reflect::std_traits::ReflectDefault;
use bevy_reflect::{PartialReflect, ReflectFromReflect, TypeRegistry};
use ron::value::RawValue;
use serde::de::DeserializeSeed;
use sway_graph::graph::{
    Graph, Node, NodeParts, Part, Port, node_kind_type_id, registered_node_kinds,
};

use crate::v4::diagnostics::{LoadDiagnostics, LoadItemError};
use crate::v4::doc::{EdgeDoc, GraphDoc, NodeDoc};
use crate::v4::ids::StableIds;

/// Builds a fresh `Graph` from `doc`, reporting and skipping anything it
/// could not resolve. Never panics: a document is authored text, and a
/// half-typed one (mid-edit, or written by an older node-kind set) is the
/// normal state of a file on disk.
pub fn load(doc: &GraphDoc, registry: &TypeRegistry) -> (Graph, StableIds, LoadDiagnostics) {
    let mut graph = Graph::default();
    let mut ids = StableIds::new();
    let mut diagnostics = LoadDiagnostics::default();
    let kinds = short_name_index(registry);

    for (id, node_doc) in &doc.nodes {
        match build_node(id, node_doc, &kinds, registry, &mut diagnostics) {
            Ok(node) => {
                let node_id = graph.insert(node);
                ids.assign(id.clone(), node_id);
            }
            Err(error) => diagnostics.items.push(error),
        }
    }

    for edge in &doc.edges {
        if let Err(error) = connect_edge(&mut graph, &ids, edge) {
            diagnostics.items.push(error);
        }
    }

    graph.rebuild_order();
    (graph, ids, diagnostics)
}

/// Short node-kind name -> every registered kind's full `TypePath` sharing
/// it. Almost always one entry; more than one is the "two registered kinds
/// share a short name" hazard the task calls out — reported per node as
/// [`LoadItemError::AmbiguousNodeKind`] rather than aborting the load, since a
/// single ambiguous kind should not cost every other node in the file.
fn short_name_index(registry: &TypeRegistry) -> HashMap<&'static str, Vec<&'static str>> {
    let mut index: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
    for full_path in registered_node_kinds(registry) {
        let short = full_path.rsplit("::").next().unwrap_or(full_path);
        index.entry(short).or_default().push(full_path);
    }
    index
}

fn build_node(
    id: &str,
    node_doc: &NodeDoc,
    kinds: &HashMap<&'static str, Vec<&'static str>>,
    registry: &TypeRegistry,
    diagnostics: &mut LoadDiagnostics,
) -> Result<Node, LoadItemError> {
    let full_path = match kinds.get(node_doc.kind.as_str()) {
        None => {
            return Err(LoadItemError::UnknownNodeKind {
                id: id.to_string(),
                kind: node_doc.kind.clone(),
            });
        }
        Some(candidates) if candidates.len() > 1 => {
            return Err(LoadItemError::AmbiguousNodeKind {
                id: id.to_string(),
                kind: node_doc.kind.clone(),
                candidates: candidates.iter().map(|s| s.to_string()).collect(),
            });
        }
        Some(candidates) => candidates[0],
    };

    // `short_name_index` is built from `registered_node_kinds`, so these two
    // lookups cannot fail for a path it produced.
    let type_id = node_kind_type_id(registry, full_path).expect("a registered node kind");
    let default_value = registry
        .get_type_data::<ReflectDefault>(type_id)
        .expect("register_node_kind registers ReflectDefault")
        .default();

    let mut node = Node::new(full_path, default_value);

    let inlets_type_id = registry
        .get_type_data::<NodeParts>(type_id)
        .expect("register_node_kind registers NodeParts")
        .inlets()
        .type_id;
    let inlets_registration =
        registry
            .get(inlets_type_id)
            .ok_or_else(|| LoadItemError::BadInlets {
                id: id.to_string(),
                message: "the inlets type is not in the type registry".to_string(),
            })?;

    let mut deserializer =
        ron::de::Deserializer::from_str(node_doc.inlets.get_ron()).map_err(|error| {
            LoadItemError::BadInlets {
                id: id.to_string(),
                message: error.to_string(),
            }
        })?;
    let value = TypedReflectDeserializer::new(inlets_registration, registry)
        .deserialize(&mut deserializer)
        .map_err(|error| LoadItemError::BadInlets {
            id: id.to_string(),
            message: error.to_string(),
        })?;

    node.part_mut(Part::Inlets)
        .expect("a registered node kind always has an inlets part")
        .try_apply(&*value)
        .map_err(|error| LoadItemError::BadInlets {
            id: id.to_string(),
            message: error.to_string(),
        })?;

    // Annotations last, and never fatally: one whose type the registry does
    // not know is reported and dropped, leaving the rest of the node loaded.
    // The document declares nothing about what a key holds, so each payload
    // is read with the *untyped* deserializer and recovers its concrete type
    // from the type path it carries.
    for (key, payload) in &node_doc.metadata {
        match read_annotation(payload, registry) {
            Ok(value) => {
                node.metadata_mut().insert(key.clone(), value);
            }
            Err(message) => diagnostics.items.push(LoadItemError::BadMetadata {
                id: id.to_string(),
                key: key.clone(),
                message,
            }),
        }
    }

    Ok(node)
}

/// One annotation payload -> a concrete reflected value.
///
/// `ReflectDeserializer` yields a *dynamic* value for a type that carries no
/// `ReflectDeserialize`, which would lose the annotation's type on the way
/// back out. `ReflectFromReflect` is what turns that back into the concrete
/// type it was written as — spec: "it is readable as that same type, without
/// the reader parsing it out of another representation".
fn read_annotation(
    payload: &RawValue,
    registry: &TypeRegistry,
) -> Result<Box<dyn PartialReflect>, String> {
    let mut deserializer =
        ron::de::Deserializer::from_str(payload.get_ron()).map_err(|error| error.to_string())?;
    let value = ReflectDeserializer::new(registry)
        .deserialize(&mut deserializer)
        .map_err(|error| error.to_string())?;

    let Some(info) = value.get_represented_type_info() else {
        return Ok(value);
    };
    let type_id = info.type_id();
    let type_path = info.type_path().to_string();
    let Some(from_reflect) = registry.get_type_data::<ReflectFromReflect>(type_id) else {
        // Registered enough to deserialize but not to reconstruct: keeping the
        // dynamic value would hand the reader something that is not the type
        // it was written as, which is exactly what the annotation contract
        // forbids.
        return Err(format!("\"{type_path}\" cannot be rebuilt from reflection"));
    };
    Ok(from_reflect
        .from_reflect(value.as_ref())
        .ok_or_else(|| format!("\"{type_path}\" did not rebuild from its payload"))?
        .into_partial_reflect())
}

fn connect_edge(graph: &mut Graph, ids: &StableIds, edge: &EdgeDoc) -> Result<(), LoadItemError> {
    let (from_id, from_path) = (&edge.from.0, &edge.from.1);
    let (to_id, to_path) = (&edge.to.0, &edge.to.1);

    let Some(src_node) = ids.node_of(from_id) else {
        return Err(LoadItemError::MissingEdgeSource {
            from: from_id.clone(),
            to: to_id.clone(),
        });
    };
    let Some(dst_node) = ids.node_of(to_id) else {
        return Err(LoadItemError::MissingEdgeDestination {
            from: from_id.clone(),
            to: to_id.clone(),
        });
    };

    graph
        .connect(
            Port::new(src_node, from_path.clone()),
            Port::new(dst_node, to_path.clone()),
            edge.slot,
        )
        .map(|_| ())
        .map_err(|error| LoadItemError::EdgeRefused {
            from: format!("{from_id}.{from_path}"),
            to: format!("{to_id}.{to_path}"),
            reason: format!("{error:?}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v4::doc::parse;
    use bevy_reflect::Reflect;
    use sway_graph::graph::{ReflectNodeKind, register_node_kind};

    #[derive(Reflect, Default, Debug)]
    struct BoxIn {
        size: f32,
    }
    #[derive(Reflect, Default, Debug)]
    struct BoxOut {
        out: f32,
    }
    #[derive(Reflect, Default, Debug)]
    #[reflect(NodeKind)]
    struct Boxed {
        inlets: BoxIn,
        state: (),
        outlets: BoxOut,
    }
    impl sway_graph::graph::NodeKind for Boxed {
        fn evaluate(&mut self, _world: &bevy_ecs::world::World) {
            self.outlets.out = self.inlets.size;
        }
    }

    mod alpha {
        use bevy_reflect::Reflect;
        use sway_graph::graph::ReflectNodeKind;

        #[derive(Reflect, Default, Debug)]
        pub struct DupIn;
        #[derive(Reflect, Default, Debug)]
        #[reflect(NodeKind)]
        pub struct Dup {
            pub inlets: DupIn,
            pub state: (),
            pub outlets: (),
        }
        impl sway_graph::graph::NodeKind for Dup {
            fn evaluate(&mut self, _world: &bevy_ecs::world::World) {}
        }
    }

    mod beta {
        use bevy_reflect::Reflect;
        use sway_graph::graph::ReflectNodeKind;

        #[derive(Reflect, Default, Debug)]
        pub struct DupIn;
        #[derive(Reflect, Default, Debug)]
        #[reflect(NodeKind)]
        pub struct Dup {
            pub inlets: DupIn,
            pub state: (),
            pub outlets: (),
        }
        impl sway_graph::graph::NodeKind for Dup {
            fn evaluate(&mut self, _world: &bevy_ecs::world::World) {}
        }
    }

    fn registry() -> TypeRegistry {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Boxed>(&mut registry);
        registry.register::<BoxIn>();
        registry.register::<BoxOut>();
        registry
    }

    fn ambiguous_registry() -> TypeRegistry {
        let mut registry = registry();
        register_node_kind::<alpha::Dup>(&mut registry);
        register_node_kind::<beta::Dup>(&mut registry);
        registry.register::<alpha::DupIn>();
        registry.register::<beta::DupIn>();
        registry
    }

    fn doc(text: &str) -> GraphDoc {
        parse(text).expect("test document parses")
    }

    #[test]
    fn every_node_loads_with_a_stable_id() {
        let registry = registry();
        let (graph, ids, diagnostics) = load(
            &doc(r#"Graph(version: 4, nodes: {
                "a": Node(type: "Boxed", metadata: {}, inlets: (size: 3.0)),
            })"#),
            &registry,
        );
        assert!(diagnostics.is_clean(), "{diagnostics:?}");
        assert_eq!(graph.len(), 1);
        let node_id = ids.node_of("a").expect("assigned");
        assert_eq!(
            graph.get(node_id).unwrap().kind(),
            <Boxed as bevy_reflect::TypePath>::type_path()
        );
    }

    #[test]
    fn an_unknown_node_kind_is_skipped_and_the_rest_loads() {
        let registry = registry();
        let (graph, ids, diagnostics) = load(
            &doc(r#"Graph(version: 4, nodes: {
                "bad": Node(type: "Nope", metadata: {}, inlets: ()),
                "good": Node(type: "Boxed", metadata: {}, inlets: (size: 1.0)),
            })"#),
            &registry,
        );

        assert_eq!(
            diagnostics.items,
            vec![LoadItemError::UnknownNodeKind {
                id: "bad".to_string(),
                kind: "Nope".to_string(),
            }]
        );
        assert_eq!(graph.len(), 1, "the other node still loads");
        assert!(ids.node_of("bad").is_none());
        assert!(ids.node_of("good").is_some());
    }

    #[test]
    fn an_ambiguous_node_kind_is_skipped_and_reported() {
        let registry = ambiguous_registry();
        let (graph, _ids, diagnostics) = load(
            &doc(r#"Graph(version: 4, nodes: {
                "a": Node(type: "Dup", metadata: {}, inlets: ()),
            })"#),
            &registry,
        );

        assert_eq!(graph.len(), 0);
        assert!(
            matches!(
                diagnostics.items.as_slice(),
                [LoadItemError::AmbiguousNodeKind { id, kind, candidates }]
                    if id == "a" && kind == "Dup" && candidates.len() == 2
            ),
            "got {:?}",
            diagnostics.items
        );
    }

    #[test]
    fn a_node_whose_inlets_do_not_deserialize_is_skipped_whole() {
        let registry = registry();
        let (graph, ids, diagnostics) = load(
            &doc(r#"Graph(version: 4, nodes: {
                "a": Node(type: "Boxed", metadata: {}, inlets: (size: "not a number")),
            })"#),
            &registry,
        );

        assert_eq!(
            graph.len(),
            0,
            "a bad node is skipped whole, not partially applied"
        );
        assert!(ids.node_of("a").is_none());
        assert!(
            matches!(diagnostics.items.as_slice(), [LoadItemError::BadInlets { id, .. }] if id == "a"),
            "got {:?}",
            diagnostics.items
        );
    }

    #[test]
    fn an_edge_naming_a_missing_source_is_skipped_and_the_graph_loads_without_it() {
        let registry = registry();
        let (graph, _ids, diagnostics) = load(
            &doc(r#"Graph(version: 4, nodes: {
                "b": Node(type: "Boxed", metadata: {}, inlets: (size: 1.0)),
            }, edges: [
                Edge(from: ("ghost", "out"), to: ("b", "size"), slot: 0),
            ])"#),
            &registry,
        );

        assert!(graph.edges().is_empty());
        assert_eq!(graph.len(), 1, "the node named still loads");
        assert_eq!(
            diagnostics.items,
            vec![LoadItemError::MissingEdgeSource {
                from: "ghost".to_string(),
                to: "b".to_string(),
            }]
        );
    }

    #[test]
    fn an_edge_naming_a_missing_destination_is_skipped_and_the_graph_loads_without_it() {
        let registry = registry();
        let (graph, _ids, diagnostics) = load(
            &doc(r#"Graph(version: 4, nodes: {
                "a": Node(type: "Boxed", metadata: {}, inlets: (size: 1.0)),
            }, edges: [
                Edge(from: ("a", "out"), to: ("ghost", "size"), slot: 0),
            ])"#),
            &registry,
        );

        assert!(graph.edges().is_empty());
        assert_eq!(graph.len(), 1);
        assert_eq!(
            diagnostics.items,
            vec![LoadItemError::MissingEdgeDestination {
                from: "a".to_string(),
                to: "ghost".to_string(),
            }]
        );
    }

    #[test]
    fn an_edge_naming_a_missing_path_is_skipped_and_the_graph_loads_without_it() {
        let registry = registry();
        let (graph, _ids, diagnostics) = load(
            &doc(r#"Graph(version: 4, nodes: {
                "a": Node(type: "Boxed", metadata: {}, inlets: (size: 1.0)),
                "b": Node(type: "Boxed", metadata: {}, inlets: (size: 2.0)),
            }, edges: [
                Edge(from: ("a", "nope"), to: ("b", "size"), slot: 0),
            ])"#),
            &registry,
        );

        assert!(graph.edges().is_empty());
        assert_eq!(graph.len(), 2, "both nodes still loaded");
        assert!(
            matches!(
                diagnostics.items.as_slice(),
                [LoadItemError::EdgeRefused { .. }]
            ),
            "got {:?}",
            diagnostics.items
        );
    }

    #[test]
    fn a_variadic_inlets_edge_order_survives_a_load() {
        #[derive(Reflect, Default, Debug)]
        struct FanIn {
            values: Vec<f32>,
        }
        #[derive(Reflect, Default, Debug)]
        #[reflect(NodeKind)]
        struct Fan {
            inlets: FanIn,
            state: (),
            outlets: (),
        }
        impl sway_graph::graph::NodeKind for Fan {
            fn evaluate(&mut self, _world: &bevy_ecs::world::World) {}
        }

        let mut registry = registry();
        register_node_kind::<Fan>(&mut registry);
        registry.register::<FanIn>();

        let (graph, ids, diagnostics) = load(
            &doc(r#"Graph(version: 4, nodes: {
                "a": Node(type: "Boxed", metadata: {}, inlets: (size: 1.0)),
                "b": Node(type: "Boxed", metadata: {}, inlets: (size: 2.0)),
                "fan": Node(type: "Fan", metadata: {}, inlets: (values: [])),
            }, edges: [
                Edge(from: ("b", "out"), to: ("fan", "values"), slot: 20),
                Edge(from: ("a", "out"), to: ("fan", "values"), slot: 10),
            ])"#),
            &registry,
        );

        assert!(diagnostics.is_clean(), "{diagnostics:?}");
        let fan = ids.node_of("fan").expect("assigned");
        let mut edges: Vec<_> = graph.edges_into(fan).collect();
        edges.sort_by_key(|edge| edge.slot);
        assert_eq!(edges[0].slot, 10);
        assert_eq!(edges[1].slot, 20);
    }
}
