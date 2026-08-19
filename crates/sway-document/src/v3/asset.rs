//! The version 3 document as a Bevy asset — the loading mechanism only
//! (design D1: "The asset is a loading mechanism only. It is not kept in sync
//! with the resource and is not consulted after initialization.").
//!
//! Deliberately **not** wired into `crate::ProjectPlugin`: `ProjectLoader`
//! already claims the `sway.ron` extension, and Bevy's `AssetServer` does not
//! support two loaders registered for one extension in the same `App`. This
//! plugin registers the version 3 asset type and its loader only; a startup
//! system that reads a loaded [`GraphAsset`] and calls [`crate::v3::load`] to
//! build the live `Graph` resource — and never consults the asset again — is
//! for whichever wave wires this plugin into an `App` (`sway-app`, group 8),
//! once it drops `ProjectPlugin`.

use bevy_app::{App, Plugin};
use bevy_asset::io::Reader;
use bevy_asset::{Asset, AssetApp, AssetLoader, LoadContext};
use bevy_reflect::TypePath;

use crate::v3::doc::{GraphDoc, ParseError, parse};

#[derive(Asset, TypePath, Debug, Clone)]
pub struct GraphAsset {
    pub doc: GraphDoc,
}

#[derive(Default, TypePath)]
pub struct GraphAssetLoader;

impl AssetLoader for GraphAssetLoader {
    type Asset = GraphAsset;
    type Settings = ();
    type Error = ParseError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _context: &mut LoadContext<'_>,
    ) -> Result<GraphAsset, ParseError> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| ParseError::Ron(e.to_string()))?;
        let text = String::from_utf8(bytes).map_err(|e| ParseError::Ron(e.to_string()))?;
        Ok(GraphAsset { doc: parse(&text)? })
    }

    fn extensions(&self) -> &[&str] {
        &["sway.ron"]
    }
}

/// Registers [`GraphAsset`] and [`GraphAssetLoader`] only. Not added
/// alongside `crate::ProjectPlugin` in the same `App` — see the module docs.
pub struct GraphAssetPlugin;

impl Plugin for GraphAssetPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<GraphAsset>()
            .init_asset_loader::<GraphAssetLoader>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_asset::AssetPlugin;

    fn asset_app() -> App {
        let mut app = App::new();
        app.add_plugins(AssetPlugin::default())
            .add_plugins(GraphAssetPlugin);
        app
    }

    #[test]
    fn the_plugin_registers_the_asset_type_without_panicking() {
        // The point of the test: adding this plugin alone, with no
        // `ProjectPlugin` in the same `App`, must not conflict with anything.
        let _app = asset_app();
    }

    #[test]
    fn the_loader_reads_text_into_a_document() {
        let text = r#"Graph(version: 3, nodes: {}, edges: [])"#;
        let doc = parse(text).expect("parses");
        assert!(doc.nodes.is_empty());
    }
}
