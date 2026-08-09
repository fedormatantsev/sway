//! World -> document. Spec §5.
//!
//! Exists to prove the format complete: a round-trip through here and back is
//! the only check that every authorable component and wire can be written
//! down. The *in-place, comment-preserving* writer is M7's; this one emits a
//! whole document.

use std::collections::BTreeMap;

use bevy_ecs::entity::Entity;
use bevy_ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy_ecs::world::World;
use bevy_reflect::serde::TypedReflectSerializer;

use crate::project::diagnostics::DocId;
use crate::project::doc::{EntityDoc, FORMAT_VERSION, ProjectDoc};
use crate::project::registry::ComponentDocRegistry;
use crate::registry_wires::WireRegistry;

pub fn to_document(world: &mut World) -> ProjectDoc {
    let empty = ProjectDoc {
        version: FORMAT_VERSION,
        entities: Vec::new(),
    };
    let Some(type_registry) = world.get_resource::<AppTypeRegistry>().cloned() else {
        return empty;
    };

    let mut carriers: Vec<(String, Entity)> = world
        .query::<(Entity, &DocId)>()
        .iter(world)
        .map(|(entity, id)| (id.0.clone(), entity))
        .collect();
    carriers.sort_by(|a, b| a.0.cmp(&b.0));

    let ids: BTreeMap<Entity, String> = carriers
        .iter()
        .map(|(id, entity)| (*entity, id.clone()))
        .collect();

    let had_components = world.get_resource::<ComponentDocRegistry>().is_some();
    let components = world
        .remove_resource::<ComponentDocRegistry>()
        .unwrap_or_default();
    let had_wires = world.get_resource::<WireRegistry>().is_some();
    let wires = world.remove_resource::<WireRegistry>().unwrap_or_default();

    let mut entities = Vec::with_capacity(carriers.len());
    {
        let type_registry = type_registry.read();
        for (id, entity) in &carriers {
            let mut component_map = BTreeMap::new();
            for entry in &components.entries {
                let Some(registration) = type_registry.get(entry.type_id) else {
                    continue;
                };
                let Some(reflect_component) = registration.data::<ReflectComponent>() else {
                    continue;
                };
                let Ok(entity_ref) = world.get_entity(*entity) else {
                    continue;
                };
                let Some(value) = reflect_component.reflect(entity_ref) else {
                    continue;
                };
                let serializer =
                    TypedReflectSerializer::new(value.as_partial_reflect(), &type_registry);
                let Ok(text) = ron::to_string(&serializer) else {
                    continue;
                };
                // `EntityDoc.components` stores each payload as raw, unparsed
                // RON text (`ron::value::RawValue`), not `ron::Value`: Task 1
                // found `ron::Value` cannot drive `TypedReflectDeserializer`
                // through an enum field (see doc.rs). Wrap the serialized
                // text directly rather than round-tripping it through
                // `ron::Value`.
                let Ok(payload) = ron::value::RawValue::from_boxed_ron(text.into_boxed_str())
                else {
                    continue;
                };
                component_map.insert(entry.name.to_string(), payload);
            }

            let mut wire_map = BTreeMap::new();
            for entry in &wires.entries {
                let Some(src) = (entry.read)(world, *entity) else {
                    continue;
                };
                let Some(src_id) = ids.get(&src) else {
                    continue; // wired to something the document does not own
                };
                wire_map.insert(entry.name.to_string(), src_id.clone());
            }

            entities.push(EntityDoc {
                id: id.clone(),
                components: component_map,
                wires: wire_map,
            });
        }
    }

    if had_components {
        world.insert_resource(components);
    }
    if had_wires {
        world.insert_resource(wires);
    }

    ProjectDoc { version: FORMAT_VERSION, entities }
}

/// One component per line, one wire per line — the format constraint M7's
/// in-place writer depends on (spec §2.2). `depth_limit` is what enforces it:
/// Project / entities / Entity / maps are formatted, and a payload below that
/// is written compactly on one line.
pub fn to_ron(doc: &ProjectDoc) -> Result<String, ron::Error> {
    let config = ron::ser::PrettyConfig::new()
        .struct_names(true)
        .depth_limit(4)
        .indentor("    ")
        .compact_arrays(false);
    ron::ser::to_string_pretty(doc, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::TopologyDirty;
    use crate::project::apply::apply;
    use crate::project::doc::parse;
    use crate::project::registry::register_authorable;
    use crate::registry_wires::register_wire;
    use crate::test_wires::{FloatOut, Gain, GainFrom};
    use bevy_app::App;

    fn round_trip_app() -> App {
        let mut app = App::new();
        app.init_resource::<TopologyDirty>();
        register_wire::<GainFrom>(&mut app);
        register_authorable::<Gain>(&mut app, "Gain");
        register_authorable::<FloatOut>(&mut app, "FloatOut");
        app
    }

    const SOURCE: &str = r#"Project(version: 1, entities: [
        Entity(id: "src", components: { "FloatOut": (2.0) }),
        Entity(id: "dst", components: { "Gain": (factor: 0.0, value: 0.5) },
               wires: { "factor": "src" }),
    ])"#;

    #[test]
    fn a_world_emits_the_document_that_built_it() {
        let mut app = round_trip_app();
        apply(app.world_mut(), &parse(SOURCE).expect("parses"));

        let emitted = to_document(app.world_mut());

        assert_eq!(emitted.version, FORMAT_VERSION);
        assert_eq!(emitted.entities.len(), 2);
        let dst = emitted.entities.iter().find(|e| e.id == "dst").expect("present");
        assert_eq!(dst.wires.get("factor").map(String::as_str), Some("src"));
        assert!(dst.components.contains_key("Gain"));
    }

    #[test]
    fn document_to_world_to_document_is_stable() {
        // The completeness check: anything the format cannot express is lost
        // here and the assertion fails.
        let mut app = round_trip_app();
        apply(app.world_mut(), &parse(SOURCE).expect("parses"));
        let once = to_document(app.world_mut());

        let mut second = round_trip_app();
        apply(second.world_mut(), &once);
        let twice = to_document(second.world_mut());

        assert_eq!(once, twice);
    }

    #[test]
    fn the_emitted_text_reparses() {
        let mut app = round_trip_app();
        apply(app.world_mut(), &parse(SOURCE).expect("parses"));
        let doc = to_document(app.world_mut());

        let text = to_ron(&doc).expect("emits");
        let reparsed = parse(&text).expect("the emitter writes what the parser reads");

        assert_eq!(reparsed, doc);
    }

    #[test]
    fn each_component_and_wire_gets_its_own_line() {
        // Spec §2.2: this is what lets M7's writer replace one line in place.
        let mut app = round_trip_app();
        apply(app.world_mut(), &parse(SOURCE).expect("parses"));
        let text = to_ron(&to_document(app.world_mut())).expect("emits");

        let gain_line = text
            .lines()
            .find(|line| line.contains("\"Gain\""))
            .expect("the Gain component is written");
        assert!(
            gain_line.contains("factor") && gain_line.contains("value"),
            "the whole payload is on one line: {gain_line}"
        );
        assert_eq!(
            text.lines().filter(|line| line.contains("\"factor\": \"src\"")).count(),
            1,
            "the wire is one line"
        );
    }

    #[test]
    fn an_entity_without_a_doc_id_is_not_in_the_document() {
        let mut app = round_trip_app();
        apply(app.world_mut(), &parse(SOURCE).expect("parses"));
        app.world_mut().spawn(FloatOut(9.0));

        let doc = to_document(app.world_mut());

        assert_eq!(doc.entities.len(), 2, "the runtime-owned entity stayed out");
    }
}
