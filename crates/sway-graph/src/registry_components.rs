//! Which components a document may name, and what they are called there.
//! Spec §3.
//!
//! Short names, not reflect `TypePath`: nobody should type
//! `sway_nodes::osc::Lfo` into a hand-authored file, and a type path pins an
//! internal module layout into the file format.

use std::any::TypeId;

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy_ecs::resource::Resource;
use bevy_reflect::{GetTypeRegistration, Reflect, TypePath};
use bevy_reflect::std_traits::ReflectDefault;

pub struct ComponentEntry {
    /// The key used in a document, e.g. `"Lfo"`.
    pub name: &'static str,
    pub type_id: TypeId,
    /// For diagnostics and the inspector's fallback rendering.
    pub type_path: &'static str,
}

#[derive(Resource, Default)]
pub struct ComponentDocRegistry {
    pub entries: Vec<ComponentEntry>,
}

impl ComponentDocRegistry {
    pub fn by_name(&self, name: &str) -> Option<&ComponentEntry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    pub fn by_type(&self, type_id: TypeId) -> Option<&ComponentEntry> {
        self.entries.iter().find(|entry| entry.type_id == type_id)
    }
}

/// Makes `C` nameable in a document.
///
/// Panics on a duplicate name, on a type without `#[reflect(Component)]`, and
/// on one without `#[reflect(Default)]` — all three at startup, which is the
/// only place this plan allows a panic. A show that would fail to load its
/// project fails while the operator is still looking at a terminal.
pub fn register_authorable<C>(app: &mut App, name: &'static str)
where
    C: Component + Reflect + TypePath + GetTypeRegistration,
{
    app.register_type::<C>();
    app.init_resource::<ComponentDocRegistry>();

    let type_id = TypeId::of::<C>();
    {
        let registry = app.world().resource::<AppTypeRegistry>().read();
        let registration = registry
            .get(type_id)
            .unwrap_or_else(|| panic!("{} was just registered", C::type_path()));
        assert!(
            registration.data::<ReflectComponent>().is_some(),
            "authorable component {} needs #[reflect(Component)]",
            C::type_path()
        );
        assert!(
            registration.data::<ReflectDefault>().is_some(),
            "authorable component {} needs #[reflect(Default)]: a document may \
             name a subset of its fields, and the rest come from Default",
            C::type_path()
        );
    }

    let mut docs = app.world_mut().resource_mut::<ComponentDocRegistry>();
    assert!(
        docs.by_name(name).is_none(),
        "two components are registered as \"{name}\"; document keys must be unique"
    );
    docs.entries.push(ComponentEntry {
        name,
        type_id,
        type_path: C::type_path(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::component::Component;
    use bevy_reflect::Reflect;

    #[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    struct Gain {
        factor: f32,
    }

    #[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    struct Bias {
        offset: f32,
    }

    /// Missing #[reflect(Default)] on purpose.
    #[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq)]
    #[reflect(Component)]
    struct NoDefault {
        value: f32,
    }

    #[test]
    fn registering_records_the_short_name_and_the_type() {
        let mut app = App::new();
        register_authorable::<Gain>(&mut app, "Gain");

        let registry = app.world().resource::<ComponentDocRegistry>();
        let entry = registry.by_name("Gain").expect("registered");
        assert_eq!(entry.type_id, TypeId::of::<Gain>());
        assert!(registry.by_name("Bias").is_none());
    }

    #[test]
    fn two_components_register_independently() {
        let mut app = App::new();
        register_authorable::<Gain>(&mut app, "Gain");
        register_authorable::<Bias>(&mut app, "Bias");

        assert_eq!(app.world().resource::<ComponentDocRegistry>().entries.len(), 2);
    }

    #[test]
    #[should_panic(expected = "document keys must be unique")]
    fn a_duplicate_name_panics_at_startup() {
        let mut app = App::new();
        register_authorable::<Gain>(&mut app, "Same");
        register_authorable::<Bias>(&mut app, "Same");
    }

    #[test]
    #[should_panic(expected = "needs #[reflect(Default)]")]
    fn a_component_without_reflect_default_panics_at_startup() {
        // Spec §3: partial payloads need a fallback for the fields the
        // document does not name, and this is where that is enforced.
        let mut app = App::new();
        register_authorable::<NoDefault>(&mut app, "NoDefault");
    }
}
