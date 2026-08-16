//! Claiming editor-created entities for the document. Spec M6-3.
//!
//! `to_document` emits only entities carrying a `DocId`, and a
//! palette-created entity has none — but `DocId` is a document component and
//! the editor cannot write one. So the document layer notices and claims.
//!
//! `EditorPos` is the marker because it already means "authored on the
//! canvas": runtime-spawned entities never carry one, which is what keeps
//! `emit.rs`'s `an_entity_without_a_doc_id_is_not_in_the_document` true.

use std::collections::HashSet;

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use sway_graph::{ComponentDocRegistry, EditorPos};

use crate::diagnostics::DocId;

pub fn claim_editor_entities(world: &mut World) {
    let unclaimed: Vec<Entity> = world
        .iter_entities()
        .filter(|entity| entity.contains::<EditorPos>() && !entity.contains::<DocId>())
        .map(|entity| entity.id())
        .collect();
    if unclaimed.is_empty() {
        return;
    }

    let mut taken: HashSet<String> = world
        .iter_entities()
        .filter_map(|entity| entity.get::<DocId>().map(|id| id.0.clone()))
        .collect();

    for entity in unclaimed {
        let stem = stem_for(world, entity);
        let mut candidate = stem.clone();
        let mut n = 0u32;
        while taken.contains(&candidate) {
            n += 1;
            candidate = format!("{stem}.{n:03}");
        }
        taken.insert(candidate.clone());
        if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
            entity_mut.insert(DocId(candidate));
        }
    }
}

/// The name of the first component this entity carries in
/// `ComponentDocRegistry` order — registration order, fixed at startup and
/// therefore deterministic.
fn stem_for(world: &World, entity: Entity) -> String {
    let Some(registry) = world.get_resource::<ComponentDocRegistry>() else {
        return "node".to_string();
    };
    let Ok(entity_ref) = world.get_entity(entity) else {
        return "node".to_string();
    };
    for entry in &registry.entries {
        let Some(component_id) = world.components().get_id(entry.type_id) else {
            continue;
        };
        if entity_ref.contains_id(component_id) {
            return entry.name.to_string();
        }
    }
    "node".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::App;
    use bevy_math::Vec2;
    use sway_graph::EditorPos;

    fn claim_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy_asset::AssetPlugin::default())
            .add_plugins(crate::ProjectPlugin);
        sway_graph::register_authorable::<EditorPos>(&mut app, "EditorPos");
        app
    }

    #[test]
    fn an_editor_pos_entity_without_a_doc_id_is_claimed() {
        let mut app = claim_app();
        let entity = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();

        app.update();

        assert!(app.world().get::<DocId>(entity).is_some());
    }

    #[test]
    fn a_runtime_entity_without_an_editor_pos_is_not_claimed() {
        let mut app = claim_app();
        let entity = app.world_mut().spawn_empty().id();

        app.update();

        assert!(
            app.world().get::<DocId>(entity).is_none(),
            "emit.rs's guarantee that runtime-owned entities stay out of the \
             document depends on this",
        );
    }

    #[test]
    fn claimed_ids_do_not_collide() {
        let mut app = claim_app();
        let a = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();
        let b = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();

        app.update();

        let id_a = app.world().get::<DocId>(a).cloned().unwrap();
        let id_b = app.world().get::<DocId>(b).cloned().unwrap();
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn a_claimed_id_does_not_collide_with_one_the_document_already_named() {
        let mut app = claim_app();
        app.world_mut()
            .spawn((EditorPos(Vec2::ZERO), DocId("EditorPos".to_string())));
        let fresh = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();

        app.update();

        assert_ne!(
            app.world().get::<DocId>(fresh).cloned().unwrap().0,
            "EditorPos".to_string(),
        );
    }

    #[test]
    fn an_already_claimed_entity_keeps_its_id() {
        let mut app = claim_app();
        let entity = app
            .world_mut()
            .spawn((EditorPos(Vec2::ZERO), DocId("keepme".to_string())))
            .id();

        app.update();
        app.update();

        assert_eq!(app.world().get::<DocId>(entity).unwrap().0, "keepme");
    }
}
