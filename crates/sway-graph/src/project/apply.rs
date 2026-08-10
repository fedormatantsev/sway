//! Applying a document to the world, by reconciling on `DocId`. Spec §4.
//!
//! Four passes, in order: index and despawn, spawn, components, wires. The
//! first two complete before any wire is resolved, so a wire may name an
//! entity declared later in the file.

use std::any::TypeId;
use std::collections::{HashMap, HashSet};

use bevy_ecs::component::ComponentId;
use bevy_ecs::entity::Entity;
use bevy_ecs::name::Name;
use bevy_ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy_ecs::world::World;
use bevy_reflect::TypeRegistry;
use bevy_reflect::serde::TypedReflectDeserializer;
use serde::de::DeserializeSeed;

use crate::order::TopologyDirty;
use crate::project::diagnostics::{DocId, ItemError, ProjectDiagnostics};
use crate::project::doc::{EntityDoc, ProjectDoc};
use crate::project::registry::ComponentDocRegistry;
use crate::registry_wires::WireRegistry;

/// Applies `doc` to `world` and returns what it could not do.
///
/// Never panics and never returns `Err`: a document is authored text, and a
/// half-typed one is the normal state of a file being edited. Missing
/// registries degrade to empty ones for the duration of this call and are
/// not inserted back if they were never present.
pub fn apply(world: &mut World, doc: &ProjectDoc) -> ProjectDiagnostics {
    let mut diagnostics = ProjectDiagnostics::default();
    let Some(type_registry) = world.get_resource::<AppTypeRegistry>().cloned() else {
        return diagnostics;
    };
    let ids = reconcile_entities(world, doc);

    // Taken out so the passes can hold `&mut World`, put back after. The
    // registries are read-only here; this is a borrow move, not a mutation.
    let had_components = world.get_resource::<ComponentDocRegistry>().is_some();
    let components = world
        .remove_resource::<ComponentDocRegistry>()
        .unwrap_or_default();

    {
        let type_registry = type_registry.read();
        for entity_doc in &doc.entities {
            let Some(&entity) = ids.get(&entity_doc.id) else {
                continue;
            };
            apply_components(
                world,
                entity,
                entity_doc,
                &components,
                &type_registry,
                &mut diagnostics,
            );
        }
    }

    let had_wires = world.get_resource::<WireRegistry>().is_some();
    let wires = world.remove_resource::<WireRegistry>().unwrap_or_default();
    for entity_doc in &doc.entities {
        let Some(&entity) = ids.get(&entity_doc.id) else {
            continue;
        };
        apply_wires(world, entity, entity_doc, &ids, &wires, &mut diagnostics);
    }
    if had_wires {
        world.insert_resource(wires);
    }

    if let Some(mut dirty) = world.get_resource_mut::<TopologyDirty>() {
        dirty.0 = true;
    }

    if had_components {
        world.insert_resource(components);
    }
    diagnostics
}

/// Pass 3: writes each entity's named components from its payload, and
/// removes any registered-authorable component the document dropped.
///
/// A document names a *subset* of a component's fields (spec §4.1); the
/// deserializer fills the rest via `ReflectDefault` before the value ever
/// reaches the world (Task 1's characterization: `bevy_reflect` 0.19 does
/// this eagerly, per field, at deserialize time). So there is no unnamed-
/// field value left to preserve by the time this function has a `Box<dyn
/// Reflect>` in hand — inserting is the only option, not a choice between
/// `apply` and `insert`. A reload therefore resets a component's unnamed
/// fields to their defaults rather than leaving them at whatever a wire most
/// recently drove them to; accepted loss, recorded in Task 1's ledger.
fn apply_components(
    world: &mut World,
    entity: Entity,
    entity_doc: &EntityDoc,
    components: &ComponentDocRegistry,
    type_registry: &TypeRegistry,
    diagnostics: &mut ProjectDiagnostics,
) {
    let mut written: Vec<TypeId> = Vec::new();

    for (name, payload) in &entity_doc.components {
        let Some(entry) = components.by_name(name) else {
            diagnostics.items.push(ItemError::UnknownComponent {
                entity: entity_doc.id.clone(),
                name: name.clone(),
            });
            continue;
        };
        // Mark this component "named by the document" as soon as we know
        // it, before any fallible resolution step. A later failure in this
        // same iteration (bad payload, missing registry data, ...) must
        // still count as "the document named it" so the removal pass below
        // leaves the entity's existing value alone instead of deleting it —
        // spec §4.3: a failed item is skipped, not removed.
        written.push(entry.type_id);
        let Some(registration) = type_registry.get(entry.type_id) else {
            diagnostics.items.push(ItemError::BadPayload {
                entity: entity_doc.id.clone(),
                name: name.clone(),
                message: format!("{} is not in the reflect registry", entry.type_path),
            });
            continue;
        };
        let mut deserializer = match ron::de::Deserializer::from_str(payload.get_ron()) {
            Ok(deserializer) => deserializer,
            Err(error) => {
                diagnostics.items.push(ItemError::BadPayload {
                    entity: entity_doc.id.clone(),
                    name: name.clone(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        let value = match TypedReflectDeserializer::new(registration, type_registry)
            .deserialize(&mut deserializer)
        {
            Ok(value) => value,
            Err(error) => {
                diagnostics.items.push(ItemError::BadPayload {
                    entity: entity_doc.id.clone(),
                    name: name.clone(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            diagnostics.items.push(ItemError::BadPayload {
                entity: entity_doc.id.clone(),
                name: name.clone(),
                message: format!("{} is not a reflectable component", entry.type_path),
            });
            continue;
        };

        let current_matches = world
            .get_entity(entity)
            .ok()
            .and_then(|entity_ref| reflect_component.reflect(entity_ref))
            .and_then(|current| value.reflect_partial_eq(current.as_partial_reflect()))
            .unwrap_or(false);
        if current_matches {
            continue; // writing an equal value would mark Changed for nothing
        }

        let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
            continue;
        };
        // The deserializer already filled any unnamed field via
        // `ReflectDefault`, so there is nothing left for `apply` to preserve
        // that `insert` would not already carry.
        reflect_component.insert(&mut entity_mut, &*value, type_registry);
    }

    // A component the document did not name is removed below — but a
    // `#[require]` companion was never the document's to name. `Lfo` carries
    // `FloatOut` because `Lfo` requires it (roadmap D4), and a document that
    // names only `Lfo` must still load a node with an outlet. So anything
    // required, transitively, by a component this document named on this
    // entity is exempt. `ComponentInfo::required_components()` reports the
    // transitive set, so a `MeshAsset` that requires `Mesh3d` exempts
    // `Transform` too.
    let mut required_by_named: Vec<ComponentId> = Vec::new();
    for type_id in &written {
        let Some(component_id) = world.components().get_id(*type_id) else {
            continue;
        };
        let Some(info) = world.components().get_info(component_id) else {
            continue;
        };
        required_by_named.extend(info.required_components().iter_ids());
    }

    // Anything registered-authorable, present, absent from the document, and
    // not required by something the document did name is removed — including
    // components the entity acquired from a runtime system. `Transform` is the
    // sharpest case: a doc-owned entity that picks one up outside the document,
    // and whose named components do not require one, loses it on the next
    // reload. Spec §4.1; intended.
    for entry in &components.entries {
        if written.contains(&entry.type_id) {
            continue;
        }
        if world
            .components()
            .get_id(entry.type_id)
            .is_some_and(|id| required_by_named.contains(&id))
        {
            continue;
        }
        let Some(registration) = type_registry.get(entry.type_id) else {
            continue;
        };
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            continue;
        };
        let present = world
            .get_entity(entity)
            .ok()
            .and_then(|entity_ref| reflect_component.reflect(entity_ref))
            .is_some();
        if !present {
            continue;
        }
        if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
            reflect_component.remove(&mut entity_mut);
        }
    }
}

/// Pass 4: resolves each entity's declared wires against `WireRegistry`,
/// inserting, removing, or leaving each one alone, and reports an unknown
/// wire name or an unresolved target rather than panicking on either.
fn apply_wires(
    world: &mut World,
    entity: Entity,
    entity_doc: &EntityDoc,
    ids: &HashMap<String, Entity>,
    wires: &WireRegistry,
    diagnostics: &mut ProjectDiagnostics,
) {
    // Wire names this entity named in the document but that failed to
    // resolve (unknown wire, or a target id that doesn't resolve). These
    // are left alone below rather than treated as "wanted = None", so a
    // transient typo mid-edit doesn't rip out a wire that was already
    // successfully wired — spec §4.3: a failed item is skipped, not removed.
    let mut diagnosed: HashSet<&str> = HashSet::new();
    for (name, target_id) in &entity_doc.wires {
        if wires.entries.iter().all(|entry| entry.name != name) {
            diagnostics.items.push(ItemError::UnknownWire {
                entity: entity_doc.id.clone(),
                wire: name.clone(),
            });
            diagnosed.insert(name.as_str());
        } else if !ids.contains_key(target_id) {
            diagnostics.items.push(ItemError::UnresolvedTarget {
                entity: entity_doc.id.clone(),
                wire: name.clone(),
                target: target_id.clone(),
            });
            diagnosed.insert(name.as_str());
        }
    }

    for entry in &wires.entries {
        if diagnosed.contains(entry.name) {
            continue; // named but unresolved this round — leave it alone
        }
        let wanted = entity_doc
            .wires
            .get(entry.name)
            .and_then(|target_id| ids.get(target_id))
            .copied();
        let current = (entry.read)(world, entity);
        if wanted == current {
            continue; // never churn a RelationshipTarget for nothing
        }
        match wanted {
            Some(src) => (entry.insert)(world, entity, src),
            None => (entry.remove)(world, entity),
        }
    }
}

/// Passes 1 and 2: despawn what left, spawn what arrived, keep what stayed.
/// Returns the document-id -> entity map the later passes resolve against.
fn reconcile_entities(world: &mut World, doc: &ProjectDoc) -> HashMap<String, Entity> {
    let mut existing: HashMap<String, Entity> = world
        .query::<(Entity, &DocId)>()
        .iter(world)
        .map(|(entity, id)| (id.0.clone(), entity))
        .collect();

    let wanted: HashSet<&str> = doc.entities.iter().map(|e| e.id.as_str()).collect();

    // Pass 1. Despawn takes children and any wire on the despawned entity
    // with it; a wire *pointing at* it is left dangling until the next
    // rebuild, which is exactly what `propagate_of` already tolerates.
    let departed: Vec<String> = existing
        .keys()
        .filter(|id| !wanted.contains(id.as_str()))
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
    use crate::project::registry::register_authorable;
    use bevy_app::App;
    use bevy_ecs::component::Component;
    use bevy_ecs::query::Changed;
    use bevy_reflect::Reflect;
    use bevy_reflect::std_traits::ReflectDefault;

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
        world.init_resource::<AppTypeRegistry>();
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
        world.init_resource::<AppTypeRegistry>();
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
        world.init_resource::<AppTypeRegistry>();
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
        world.init_resource::<AppTypeRegistry>();
        let runtime_owned = world.spawn(Name::new("camera")).id();

        apply(&mut world, &doc(r#"Project(version: 1, entities: [Entity(id: "a")])"#));
        apply(&mut world, &doc("Project(version: 1, entities: [])"));

        assert!(world.get_entity(runtime_owned).is_ok(), "not ours, not despawned");
    }

    #[test]
    fn an_empty_document_clears_the_authored_world() {
        let mut world = World::new();
        world.init_resource::<AppTypeRegistry>();
        apply(&mut world, &doc(r#"Project(version: 1, entities: [Entity(id: "a")])"#));
        apply(&mut world, &doc("Project(version: 1, entities: [])"));

        assert!(ids(&mut world).is_empty());
    }

    #[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    struct Osc {
        hz: f32,
        amplitude: f32,
    }

    impl Default for Osc {
        fn default() -> Self {
            Self { hz: 1.0, amplitude: 0.5 }
        }
    }

    #[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    struct Outlet(f32);

    /// Stands in for `Lfo`, which requires `FloatOut` (roadmap D4).
    #[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    #[require(Outlet)]
    struct Emitter;

    fn require_app() -> App {
        let mut app = App::new();
        register_authorable::<Emitter>(&mut app, "Emitter");
        register_authorable::<Outlet>(&mut app, "Outlet");
        register_authorable::<Osc>(&mut app, "Osc");
        app
    }

    /// An app with `Osc` authorable, which is all the component pass needs.
    fn doc_app() -> App {
        let mut app = App::new();
        register_authorable::<Osc>(&mut app, "Osc");
        app
    }

    #[test]
    fn a_named_component_is_inserted_from_its_payload() {
        let mut app = doc_app();
        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Osc": (hz: 3.0, amplitude: 0.25) })
            ])"#),
        );

        let entity = entity_of(app.world_mut(), "a").expect("spawned");
        assert_eq!(
            app.world().get::<Osc>(entity),
            Some(&Osc { hz: 3.0, amplitude: 0.25 })
        );
    }

    #[test]
    fn a_partial_payload_resets_the_other_fields_to_default_on_reload() {
        // Spec §4.1 originally called for `apply` to touch only the named
        // fields, so a reload would not clobber what a wire is driving.
        // Task 1's characterization found `bevy_reflect` 0.19 fills unnamed
        // fields via `ReflectDefault` *during deserialization*, before this
        // pass ever sees a `Box<dyn Reflect>` — so there is no partial value
        // left for `apply` to merge, and `insert` is the only option
        // (ledger: Task 1 DECISION for Task 6). This test pins the accepted
        // loss: a reload resets unnamed fields to their default, not to
        // whatever a wire last drove them to.
        let mut app = doc_app();
        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Osc": (hz: 3.0, amplitude: 0.25) })
            ])"#),
        );
        let entity = entity_of(app.world_mut(), "a").expect("spawned");
        // Something else — a wire — moves amplitude.
        app.world_mut().get_mut::<Osc>(entity).expect("present").amplitude = 0.9;

        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Osc": (hz: 4.0) })
            ])"#),
        );

        assert_eq!(
            app.world().get::<Osc>(entity),
            Some(&Osc { hz: 4.0, amplitude: 0.5 })
        );
    }

    #[test]
    fn an_unchanged_component_is_not_marked_changed() {
        // The same discipline wires live under (parent spec §2.11): writing an
        // equal value destroys change detection for everything downstream.
        let mut app = doc_app();
        let text = r#"Project(version: 1, entities: [
            Entity(id: "a", components: { "Osc": (hz: 3.0, amplitude: 0.25) })
        ])"#;
        apply(app.world_mut(), &doc(text));
        app.world_mut().clear_trackers();

        apply(app.world_mut(), &doc(text));

        let changed = app
            .world_mut()
            .query_filtered::<(), Changed<Osc>>()
            .iter(app.world())
            .count();
        assert_eq!(changed, 0, "an identical reload must touch nothing");
    }

    #[test]
    fn a_component_dropped_from_the_document_is_removed() {
        let mut app = doc_app();
        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Osc": (hz: 3.0) })
            ])"#),
        );
        let entity = entity_of(app.world_mut(), "a").expect("spawned");

        apply(app.world_mut(), &doc(r#"Project(version: 1, entities: [Entity(id: "a")])"#));

        assert!(app.world().get::<Osc>(entity).is_none());
    }

    #[test]
    fn a_required_companion_survives_a_document_that_does_not_name_it() {
        // D4: the palette spawns one component and Bevy materialises the rest,
        // so a document naming `Emitter` alone must still load an entity with
        // its outlet attached. Without the exemption the removal pass strips
        // `Outlet` right back off again and the node has no output.
        let mut app = require_app();
        let text = r#"Project(version: 1, entities: [
            Entity(id: "a", components: { "Emitter": () })
        ])"#;

        apply(app.world_mut(), &doc(text));
        let entity = entity_of(app.world_mut(), "a").expect("spawned");
        assert!(app.world().get::<Outlet>(entity).is_some(), "first load");

        // A reload is the sharper case: now the component is already present
        // and unnamed, which is exactly what the removal pass looks for.
        apply(app.world_mut(), &doc(text));
        assert!(app.world().get::<Outlet>(entity).is_some(), "after reload");
    }

    #[test]
    fn a_component_no_named_component_requires_is_still_removed() {
        // The exemption must be narrow: only what a *named* component pulls in.
        let mut app = require_app();
        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Emitter": (), "Osc": (hz: 3.0) })
            ])"#),
        );
        let entity = entity_of(app.world_mut(), "a").expect("spawned");

        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Emitter": () })
            ])"#),
        );

        assert!(app.world().get::<Outlet>(entity).is_some(), "required, kept");
        assert!(app.world().get::<Osc>(entity).is_none(), "not required, dropped");
    }

    #[test]
    fn an_unregistered_component_on_the_entity_survives_a_reload() {
        // A `Mesh3d` a runtime system attached. The applier only removes
        // components it is registered to author.
        #[derive(Component)]
        struct RuntimeOwned;

        let mut app = doc_app();
        apply(app.world_mut(), &doc(r#"Project(version: 1, entities: [Entity(id: "a")])"#));
        let entity = entity_of(app.world_mut(), "a").expect("spawned");
        app.world_mut().entity_mut(entity).insert(RuntimeOwned);

        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Osc": (hz: 1.0) })
            ])"#),
        );

        assert!(app.world().get::<RuntimeOwned>(entity).is_some());
    }

    #[test]
    fn an_unknown_component_name_is_reported_and_the_rest_applies() {
        let mut app = doc_app();
        let diagnostics = apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Nope": (), "Osc": (hz: 2.0) })
            ])"#),
        );

        let entity = entity_of(app.world_mut(), "a").expect("spawned");
        assert_eq!(app.world().get::<Osc>(entity).map(|o| o.hz), Some(2.0));
        assert_eq!(
            diagnostics.items,
            vec![ItemError::UnknownComponent {
                entity: "a".to_string(),
                name: "Nope".to_string(),
            }]
        );
    }

    #[test]
    fn a_payload_that_will_not_deserialize_is_reported_not_panicked() {
        let mut app = doc_app();
        let diagnostics = apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Osc": (hz: "not a number") })
            ])"#),
        );

        assert!(
            matches!(diagnostics.items.as_slice(), [ItemError::BadPayload { name, .. }] if name == "Osc"),
            "got {:?}",
            diagnostics.items
        );
    }

    #[test]
    fn a_payload_that_will_not_deserialize_leaves_the_existing_component_alone() {
        // Finding 1: a live component from a prior successful reload must
        // survive a later reload where that same component's payload is
        // mid-edit and momentarily fails to deserialize. Losing it would be
        // strictly worse than the "everything else applies, this one item
        // is skipped" contract spec §4.3 promises.
        let mut app = doc_app();
        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Osc": (hz: 3.0, amplitude: 0.25) })
            ])"#),
        );
        let entity = entity_of(app.world_mut(), "a").expect("spawned");
        assert_eq!(
            app.world().get::<Osc>(entity),
            Some(&Osc { hz: 3.0, amplitude: 0.25 })
        );

        let diagnostics = apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Osc": (hz: "not a number") })
            ])"#),
        );

        assert!(
            matches!(diagnostics.items.as_slice(), [ItemError::BadPayload { name, .. }] if name == "Osc"),
            "got {:?}",
            diagnostics.items
        );
        assert_eq!(
            app.world().get::<Osc>(entity),
            Some(&Osc { hz: 3.0, amplitude: 0.25 }),
            "the live component must be left alone, not deleted"
        );
    }

    use crate::order::TopologyDirty;
    use crate::registry_wires::register_wire;
    use crate::test_wires::{FloatOut, Gain, GainFrom};

    /// `Gain` is the wire fixture's target and `FloatOut` its source; both
    /// become authorable so a document can build the whole graph.
    fn wired_app() -> App {
        let mut app = doc_app();
        app.init_resource::<TopologyDirty>();
        register_wire::<GainFrom>(&mut app);
        register_authorable::<Gain>(&mut app, "Gain");
        register_authorable::<FloatOut>(&mut app, "FloatOut");
        app
    }

    const WIRED: &str = r#"Project(version: 1, entities: [
        Entity(id: "src", components: { "FloatOut": (2.0) }),
        Entity(id: "dst", components: { "Gain": (factor: 0.0, value: 0.0) },
               wires: { "factor": "src" }),
    ])"#;

    #[test]
    fn a_document_wire_becomes_a_relationship_component() {
        let mut app = wired_app();
        apply(app.world_mut(), &doc(WIRED));

        let src = entity_of(app.world_mut(), "src").expect("spawned");
        let dst = entity_of(app.world_mut(), "dst").expect("spawned");
        assert_eq!(app.world().get::<GainFrom>(dst).map(|w| w.0), Some(src));
    }

    #[test]
    fn a_wire_may_name_an_entity_declared_later_in_the_file() {
        let mut app = wired_app();
        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "dst", components: { "Gain": (factor: 0.0, value: 0.0) },
                       wires: { "factor": "src" }),
                Entity(id: "src", components: { "FloatOut": (2.0) }),
            ])"#),
        );

        let src = entity_of(app.world_mut(), "src").expect("spawned");
        let dst = entity_of(app.world_mut(), "dst").expect("spawned");
        assert_eq!(app.world().get::<GainFrom>(dst).map(|w| w.0), Some(src));
    }

    #[test]
    fn a_wire_dropped_from_the_document_is_removed() {
        let mut app = wired_app();
        apply(app.world_mut(), &doc(WIRED));
        let dst = entity_of(app.world_mut(), "dst").expect("spawned");

        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "src", components: { "FloatOut": (2.0) }),
                Entity(id: "dst", components: { "Gain": (factor: 0.0, value: 0.0) }),
            ])"#),
        );

        assert!(app.world().get::<GainFrom>(dst).is_none());
    }

    #[test]
    fn an_unchanged_wire_is_not_churned() {
        // Removing and re-inserting would rewrite the producer's
        // RelationshipTarget collection for nothing.
        let mut app = wired_app();
        apply(app.world_mut(), &doc(WIRED));
        app.world_mut().clear_trackers();

        apply(app.world_mut(), &doc(WIRED));

        let changed = app
            .world_mut()
            .query_filtered::<(), Changed<GainFrom>>()
            .iter(app.world())
            .count();
        assert_eq!(changed, 0);
    }

    #[test]
    fn a_wire_naming_a_missing_entity_is_reported() {
        let mut app = wired_app();
        let diagnostics = apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "dst", components: { "Gain": (factor: 0.0, value: 0.0) },
                       wires: { "factor": "ghost" }),
            ])"#),
        );

        assert_eq!(
            diagnostics.items,
            vec![ItemError::UnresolvedTarget {
                entity: "dst".to_string(),
                wire: "factor".to_string(),
                target: "ghost".to_string(),
            }]
        );
        let dst = entity_of(app.world_mut(), "dst").expect("spawned anyway");
        assert!(app.world().get::<GainFrom>(dst).is_none());
    }

    #[test]
    fn a_wire_naming_a_missing_entity_leaves_an_existing_wire_alone() {
        // Finding 2: a live wire from a prior successful reload must
        // survive a later reload where that same wire's target id is
        // mid-edit and momentarily doesn't resolve to any entity. The
        // diagnostic loop correctly reports `UnresolvedTarget`; the
        // reconcile loop must not then also treat the wire as "wanted =
        // None" and remove it.
        let mut app = wired_app();
        apply(app.world_mut(), &doc(WIRED));
        let src = entity_of(app.world_mut(), "src").expect("spawned");
        let dst = entity_of(app.world_mut(), "dst").expect("spawned");
        assert_eq!(app.world().get::<GainFrom>(dst).map(|w| w.0), Some(src));

        let diagnostics = apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "src", components: { "FloatOut": (2.0) }),
                Entity(id: "dst", components: { "Gain": (factor: 0.0, value: 0.0) },
                       wires: { "factor": "lfoB_typo_mid_edit" }),
            ])"#),
        );

        assert_eq!(
            diagnostics.items,
            vec![ItemError::UnresolvedTarget {
                entity: "dst".to_string(),
                wire: "factor".to_string(),
                target: "lfoB_typo_mid_edit".to_string(),
            }]
        );
        assert_eq!(
            app.world().get::<GainFrom>(dst).map(|w| w.0),
            Some(src),
            "the live wire must be left alone, not removed"
        );
    }

    #[test]
    fn an_unknown_wire_name_is_reported() {
        let mut app = wired_app();
        let diagnostics = apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "src", components: { "FloatOut": (2.0) }),
                Entity(id: "dst", components: { "Gain": (factor: 0.0, value: 0.0) },
                       wires: { "nope": "src" }),
            ])"#),
        );

        assert_eq!(
            diagnostics.items,
            vec![ItemError::UnknownWire {
                entity: "dst".to_string(),
                wire: "nope".to_string(),
            }]
        );
    }

    #[test]
    fn an_unknown_wire_name_leaves_an_existing_wire_under_a_different_name_alone() {
        // Same contract as the unresolved-target case, but for a wire name
        // the registry doesn't know at all (e.g. a typo in the wire's own
        // name). `factor` is live from a prior reload; the second document
        // additionally names a bogus `nope` wire, which must not disturb
        // `factor`.
        let mut app = wired_app();
        apply(app.world_mut(), &doc(WIRED));
        let src = entity_of(app.world_mut(), "src").expect("spawned");
        let dst = entity_of(app.world_mut(), "dst").expect("spawned");
        assert_eq!(app.world().get::<GainFrom>(dst).map(|w| w.0), Some(src));

        let diagnostics = apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "src", components: { "FloatOut": (2.0) }),
                Entity(id: "dst", components: { "Gain": (factor: 0.0, value: 0.0) },
                       wires: { "factor": "src", "nope": "src" }),
            ])"#),
        );

        assert_eq!(
            diagnostics.items,
            vec![ItemError::UnknownWire {
                entity: "dst".to_string(),
                wire: "nope".to_string(),
            }]
        );
        assert_eq!(app.world().get::<GainFrom>(dst).map(|w| w.0), Some(src));
    }

    #[test]
    fn applying_marks_the_topology_dirty() {
        // Spec §4.1: the applier never touches GraphOrder; it sets the flag
        // and the existing rebuild does the rest on the next FixedUpdate.
        let mut app = wired_app();
        app.world_mut().resource_mut::<TopologyDirty>().0 = false;

        apply(app.world_mut(), &doc(WIRED));

        assert!(app.world().resource::<TopologyDirty>().0);
    }
}
