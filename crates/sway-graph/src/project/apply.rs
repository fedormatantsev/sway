//! Applying a document to the world, by reconciling on `DocId`. Spec §4.
//!
//! Four passes, in order: index and despawn, spawn, components, wires. The
//! first two complete before any wire is resolved, so a wire may name an
//! entity declared later in the file.

use std::collections::HashMap;

use bevy_ecs::entity::Entity;
use bevy_ecs::name::Name;
use bevy_ecs::world::World;

use crate::project::diagnostics::{DocId, ProjectDiagnostics};
use crate::project::doc::ProjectDoc;

/// Applies `doc` to `world` and returns what it could not do.
///
/// Never panics and never returns `Err`: a document is authored text, and a
/// half-typed one is the normal state of a file being edited.
pub fn apply(world: &mut World, doc: &ProjectDoc) -> ProjectDiagnostics {
    let diagnostics = ProjectDiagnostics::default();
    let _ids = reconcile_entities(world, doc);
    diagnostics
}

/// Passes 1 and 2: despawn what left, spawn what arrived, keep what stayed.
/// Returns the document-id -> entity map the later passes resolve against.
fn reconcile_entities(world: &mut World, doc: &ProjectDoc) -> HashMap<String, Entity> {
    let mut existing: HashMap<String, Entity> = world
        .query::<(Entity, &DocId)>()
        .iter(world)
        .map(|(entity, id)| (id.0.clone(), entity))
        .collect();

    let wanted: Vec<&str> = doc.entities.iter().map(|e| e.id.as_str()).collect();

    // Pass 1. Despawn takes children and any wire on the despawned entity
    // with it; a wire *pointing at* it is left dangling until the next
    // rebuild, which is exactly what `propagate_of` already tolerates.
    let departed: Vec<String> = existing
        .keys()
        .filter(|id| !wanted.contains(&id.as_str()))
        .cloned()
        .collect();
    for id in departed {
        if let Some(entity) = existing.remove(&id) {
            world.despawn(entity);
        }
    }

    // Pass 2.
    for entity_doc in &doc.entities {
        if existing.contains_key(&entity_doc.id) {
            continue;
        }
        let entity = world
            .spawn((
                DocId(entity_doc.id.clone()),
                Name::new(entity_doc.id.clone()),
            ))
            .id();
        existing.insert(entity_doc.id.clone(), entity);
    }

    existing
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::doc::parse;

    fn doc(text: &str) -> ProjectDoc {
        parse(text).expect("test document parses")
    }

    fn ids(world: &mut World) -> Vec<String> {
        let mut found: Vec<String> = world
            .query::<&DocId>()
            .iter(world)
            .map(|id| id.0.clone())
            .collect();
        found.sort();
        found
    }

    fn entity_of(world: &mut World, id: &str) -> Option<Entity> {
        world
            .query::<(Entity, &DocId)>()
            .iter(world)
            .find(|(_, doc_id)| doc_id.0 == id)
            .map(|(entity, _)| entity)
    }

    #[test]
    fn a_first_load_spawns_every_entity_with_its_id_and_name() {
        let mut world = World::new();
        apply(
            &mut world,
            &doc(r#"Project(version: 1, entities: [Entity(id: "a"), Entity(id: "b")])"#),
        );

        assert_eq!(ids(&mut world), vec!["a".to_string(), "b".to_string()]);
        let a = entity_of(&mut world, "a").expect("spawned");
        assert_eq!(world.get::<Name>(a).map(|n| n.as_str().to_string()), Some("a".to_string()));
    }

    #[test]
    fn a_surviving_entity_keeps_its_entity_id() {
        // The whole point of reconciling rather than respawning: the editor's
        // identity, the entity's children, and anything a runtime system
        // attached all ride on this.
        let mut world = World::new();
        apply(&mut world, &doc(r#"Project(version: 1, entities: [Entity(id: "a")])"#));
        let before = entity_of(&mut world, "a").expect("spawned");

        apply(
            &mut world,
            &doc(r#"Project(version: 1, entities: [Entity(id: "a"), Entity(id: "b")])"#),
        );

        assert_eq!(entity_of(&mut world, "a"), Some(before), "same Entity across reloads");
        assert!(entity_of(&mut world, "b").is_some(), "the new one arrived");
    }

    #[test]
    fn an_entity_dropped_from_the_document_is_despawned() {
        let mut world = World::new();
        apply(
            &mut world,
            &doc(r#"Project(version: 1, entities: [Entity(id: "a"), Entity(id: "b")])"#),
        );
        let b = entity_of(&mut world, "b").expect("spawned");

        apply(&mut world, &doc(r#"Project(version: 1, entities: [Entity(id: "a")])"#));

        assert!(world.get_entity(b).is_err(), "b is gone");
        assert_eq!(ids(&mut world), vec!["a".to_string()]);
    }

    #[test]
    fn entities_without_a_doc_id_are_never_touched() {
        // The camera, the light, anything a runtime system spawned.
        let mut world = World::new();
        let runtime_owned = world.spawn(Name::new("camera")).id();

        apply(&mut world, &doc(r#"Project(version: 1, entities: [Entity(id: "a")])"#));
        apply(&mut world, &doc("Project(version: 1, entities: [])"));

        assert!(world.get_entity(runtime_owned).is_ok(), "not ours, not despawned");
    }

    #[test]
    fn an_empty_document_clears_the_authored_world() {
        let mut world = World::new();
        apply(&mut world, &doc(r#"Project(version: 1, entities: [Entity(id: "a")])"#));
        apply(&mut world, &doc("Project(version: 1, entities: [])"));

        assert!(ids(&mut world).is_empty());
    }
}
