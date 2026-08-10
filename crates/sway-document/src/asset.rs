//! The document as a Bevy asset. Spec §4.
//!
//! `AssetServer` supplies file watching, debounce and the write-then-rename
//! behaviour real text editors use; none of that is hand-rolled here.

use bevy_app::{App, Plugin, PreUpdate};
use bevy_asset::io::Reader;
use bevy_asset::{
    Asset, AssetApp, AssetEvent, AssetId, AssetLoadFailedEvent, AssetLoader, Assets, Handle,
    LoadContext,
};
use bevy_ecs::message::MessageReader;
use bevy_ecs::resource::Resource;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::system::ResMut;
use bevy_ecs::world::World;
use bevy_reflect::TypePath;

use crate::apply::apply;
use crate::diagnostics::ProjectDiagnostics;
use crate::doc::{ParseError, ProjectDoc, parse};

#[derive(Asset, TypePath, Debug, Clone)]
pub struct ProjectAsset {
    pub doc: ProjectDoc,
}

/// The project the app is currently running. Set by whatever loads it.
#[derive(Resource, Default)]
pub struct ProjectHandle(pub Option<Handle<ProjectAsset>>);

/// Set by [`note_project_changes`], drained by [`apply_pending_project`].
#[derive(Resource, Default)]
struct PendingProject(Option<AssetId<ProjectAsset>>);

#[derive(Default, TypePath)]
pub struct ProjectLoader;

impl AssetLoader for ProjectLoader {
    type Asset = ProjectAsset;
    type Settings = ();
    type Error = ParseError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _context: &mut LoadContext<'_>,
    ) -> Result<ProjectAsset, ParseError> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| ParseError::Ron(e.to_string()))?;
        let text = String::from_utf8(bytes).map_err(|e| ParseError::Ron(e.to_string()))?;
        Ok(ProjectAsset { doc: parse(&text)? })
    }

    fn extensions(&self) -> &[&str] {
        &["sway.ron"]
    }
}

/// Records that the project asset arrived or changed. Ordinary system, so it
/// can read events.
fn note_project_changes(
    mut events: MessageReader<AssetEvent<ProjectAsset>>,
    mut pending: ResMut<PendingProject>,
) {
    for event in events.read() {
        match event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::LoadedWithDependencies { id } => pending.0 = Some(*id),
            _ => {}
        }
    }
}

/// Records a load that failed. Spec §4.3: a syntax error rejects the reload
/// whole and leaves the running world exactly as it was — which is what
/// happens naturally, since a failed load produces no asset. All that is
/// needed is to make it visible.
fn note_load_failures(
    mut events: MessageReader<AssetLoadFailedEvent<ProjectAsset>>,
    mut diagnostics: ResMut<ProjectDiagnostics>,
) {
    for event in events.read() {
        diagnostics.parse = Some(event.error.to_string());
    }
}

/// Applies the pending document. Exclusive, because applying spawns,
/// despawns and inserts relationship components.
fn apply_pending_project(world: &mut World) {
    let Some(id) = world.resource_mut::<PendingProject>().0.take() else {
        return;
    };
    let Some(doc) = world
        .resource::<Assets<ProjectAsset>>()
        .get(id)
        .map(|asset| asset.doc.clone())
    else {
        return;
    };

    let mut diagnostics = apply(world, &doc);
    // A successful apply clears the previous parse error: the file is
    // readable again.
    diagnostics.parse = None;
    world.insert_resource(diagnostics);
}

/// Loading, watching and applying the project document.
///
/// Added alongside `WiresPlugin`. Requires `AssetPlugin`, which
/// `DefaultPlugins` supplies; a headless test app adds `AssetPlugin::default()`
/// itself.
pub struct ProjectPlugin;

impl Plugin for ProjectPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<ProjectAsset>()
            .init_asset_loader::<ProjectLoader>()
            .init_resource::<ProjectHandle>()
            .init_resource::<PendingProject>()
            .init_resource::<ProjectDiagnostics>()
            .add_systems(
                PreUpdate,
                (note_project_changes, note_load_failures, apply_pending_project).chain(),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::DocId;
    use bevy_app::App;
    use bevy_asset::AssetPlugin;
    use bevy_ecs::entity::Entity;

    fn asset_app() -> App {
        let mut app = App::new();
        app.add_plugins(AssetPlugin::default())
            .add_plugins(ProjectPlugin);
        app
    }

    fn doc_ids(app: &mut App) -> Vec<String> {
        let mut ids: Vec<String> = app
            .world_mut()
            .query::<&DocId>()
            .iter(app.world())
            .map(|id| id.0.clone())
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn adding_the_asset_applies_it_to_the_world() {
        let mut app = asset_app();
        let doc = parse(r#"Project(version: 1, entities: [Entity(id: "a")])"#).expect("parses");
        let handle = app
            .world_mut()
            .resource_mut::<Assets<ProjectAsset>>()
            .add(ProjectAsset { doc });
        app.world_mut().resource_mut::<ProjectHandle>().0 = Some(handle);

        // `Assets::add` queues its `AssetEvent` but bevy_asset only drains
        // that queue into `Messages` from `PostUpdate` (`Assets::<A>::
        // asset_events`), one stage after this plugin's `PreUpdate` chain
        // runs. So the event is not readable by `note_project_changes` until
        // the following frame's `PreUpdate` -- the same one-frame latency a
        // real file change goes through via the asset server.
        app.update();
        app.update();

        assert_eq!(doc_ids(&mut app), vec!["a".to_string()]);
    }

    #[test]
    fn modifying_the_asset_reapplies_it() {
        let mut app = asset_app();
        let doc = parse(r#"Project(version: 1, entities: [Entity(id: "a")])"#).expect("parses");
        let handle = app
            .world_mut()
            .resource_mut::<Assets<ProjectAsset>>()
            .add(ProjectAsset { doc });
        app.world_mut().resource_mut::<ProjectHandle>().0 = Some(handle.clone());
        // Two updates for the same reason as above: the first frame only
        // flushes the queued `AssetEvent` into `Messages`, the second reads
        // it and actually applies the document.
        app.update();
        app.update();
        let before: Vec<Entity> = app
            .world_mut()
            .query::<(Entity, &DocId)>()
            .iter(app.world())
            .map(|(entity, _)| entity)
            .collect();
        assert_eq!(before.len(), 1, "the first load must have already applied");

        let next =
            parse(r#"Project(version: 1, entities: [Entity(id: "a"), Entity(id: "b")])"#)
                .expect("parses");
        app.world_mut()
            .resource_mut::<Assets<ProjectAsset>>()
            .insert(&handle, ProjectAsset { doc: next })
            .expect("the handle's id is still valid");
        app.update();
        app.update();

        assert_eq!(doc_ids(&mut app), vec!["a".to_string(), "b".to_string()]);
        let after: Vec<Entity> = app
            .world_mut()
            .query::<(Entity, &DocId)>()
            .iter(app.world())
            .map(|(entity, _)| entity)
            .collect();
        assert!(
            before.iter().all(|entity| after.contains(entity)),
            "the surviving entity kept its Entity across the reload"
        );
    }

    #[test]
    fn the_loader_reads_text_into_a_document() {
        // The loader's own parse path, without an AssetServer.
        let text = r#"Project(version: 1, entities: [Entity(id: "a")])"#;
        let doc = parse(text).expect("parses");
        assert_eq!(doc.entities.len(), 1);
    }
}
