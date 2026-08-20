//! The project document: version 3, nodes and edges keyed by stable ids.

pub mod v4;

pub use v4::{
    EdgeDoc, FORMAT_VERSION, GraphAsset, GraphAssetLoader, GraphAssetPlugin, GraphDoc, GraphFile,
    GraphHandle, GraphInitialized, LiveGraphPlugin, LoadDiagnostics, LoadItemError, NodeDoc,
    ParseError, ProjectDirectory, SaveError, SessionIds, StableIds, load, load_from_path, parse,
    save_open_graph, save_to_path, to_document, to_ron,
};
