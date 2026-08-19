//! Version 3 document format: nodes and edges keyed by stable ids.
//!
//! `openspec/changes/redesign-graph-model` group 6. Lands **beside** the
//! version 2 format (`crate::doc`, `crate::apply`, `crate::emit`,
//! `crate::claim`, `crate::file`, `crate::asset`) — nothing there is touched;
//! task 9.3 deletes it in a later wave, not this one.
//!
//! - `doc` — the on-disk shape and its parser (tasks 6.1, 6.5).
//! - `ids` — the stable-id map, design D9 (task 6.2).
//! - `load` — document -> a fresh `sway_graph::graph::Graph`, reporting and
//!   skipping what does not resolve (tasks 6.2, 6.3, 6.4).
//! - `save` — the inverse (task 6.3).
//! - `asset` — the loading mechanism, design D1 (not the live model).
//! - `diagnostics` — what a load could not do.

pub mod asset;
pub mod diagnostics;
pub mod doc;
pub mod ids;
pub mod live;
pub mod load;
pub mod save;

pub use asset::{GraphAsset, GraphAssetLoader, GraphAssetPlugin};
pub use diagnostics::{LoadDiagnostics, LoadItemError};
pub use doc::{EdgeDoc, FORMAT_VERSION, GraphDoc, NodeDoc, ParseError, parse};
pub use ids::StableIds;
pub use live::{
    GraphFile, GraphHandle, GraphInitialized, LiveGraphPlugin, ProjectDirectory, SessionIds,
    save_open_graph,
};
pub use load::load;
pub use save::{SaveError, to_document, to_ron};

use std::path::Path;

use bevy_reflect::TypeRegistry;
use sway_graph::graph::Graph;

/// Reads `path`, parses it, and builds a fresh `Graph` from it.
///
/// A plain path read, not an asset-pipeline operation — for a caller that
/// already has a `TypeRegistry` in hand and no `App` running (a test, or a
/// CLI tool). `crate::v3::asset::GraphAssetPlugin` is the `App`-integrated
/// alternative.
pub fn load_from_path(
    path: &Path,
    registry: &TypeRegistry,
) -> Result<(Graph, StableIds, LoadDiagnostics), ParseError> {
    let text = std::fs::read_to_string(path).map_err(|e| ParseError::Ron(e.to_string()))?;
    let doc = parse(&text)?;
    Ok(load(&doc, registry))
}

/// Serializes `graph` and writes it to `path`.
///
/// Not an asset-pipeline operation (design D1): `serialize(&graph)` then
/// `fs::write`, exactly as `crate::file::save_to_path` does for version 2.
/// `ids` is the session's stable-id map; it is mutated in place so a node
/// created since the last load or save gets a durable id here rather than a
/// fresh one on every save.
pub fn save_to_path(
    graph: &Graph,
    registry: &TypeRegistry,
    ids: &mut StableIds,
    path: &Path,
) -> Result<(), String> {
    let doc = to_document(graph, registry, ids).map_err(|e| e.to_string())?;
    let text = to_ron(&doc).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_math::Vec2;
    use bevy_reflect::Reflect;
    use sway_graph::graph::{Node, ReflectNodeKind, register_node_kind};

    #[derive(Reflect, Default, Debug)]
    struct ConstIn {
        value: f32,
    }
    #[derive(Reflect, Default, Debug)]
    struct ConstOut {
        out: f32,
    }
    #[derive(Reflect, Default, Debug)]
    #[reflect(NodeKind)]
    struct Constant {
        inlets: ConstIn,
        state: (),
        outlets: ConstOut,
    }
    impl sway_graph::graph::NodeKind for Constant {
        fn evaluate(&mut self, _world: &bevy_ecs::world::World) {
            self.outlets.out = self.inlets.value;
        }
    }

    fn registry() -> TypeRegistry {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Constant>(&mut registry);
        registry.register::<ConstIn>();
        registry.register::<ConstOut>();
        registry
    }

    #[test]
    fn save_then_load_reproduces_the_graph() {
        let registry = registry();
        let dir = std::env::temp_dir().join("sway-document-v3-save-load");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.sway.ron", std::process::id()));

        let mut graph = Graph::default();
        graph.insert(Node::of(
            Vec2::new(3.0, 4.0),
            Constant {
                inlets: ConstIn { value: 1.5 },
                ..Default::default()
            },
        ));
        let mut ids = StableIds::new();
        save_to_path(&graph, &registry, &mut ids, &path).expect("saves");

        let (reopened, _ids, diagnostics) = load_from_path(&path, &registry).expect("opens");
        assert!(diagnostics.is_clean(), "{diagnostics:?}");
        assert_eq!(reopened.len(), 1);
        let (_id, node) = reopened.iter().next().expect("one node");
        assert_eq!(node.pos(), Vec2::new(3.0, 4.0));
    }

    #[test]
    fn a_missing_file_is_reported_not_panicked() {
        let registry = registry();
        let result = load_from_path(Path::new("/nonexistent/path.sway.ron"), &registry);
        let Err(error) = result else {
            panic!("a missing file must fail to load");
        };
        assert!(matches!(error, ParseError::Ron(_)));
    }
}
