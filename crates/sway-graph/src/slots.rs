//! `Feeds` slots: named, typed structural inputs. Design §4.
//!
//! A node's slots are derived from its `Slots` associated type exactly as its
//! ports are derived from `Params`/`Outputs` — the schema comes from the
//! types, never written beside them (parent §2.4). A field typed `Slot<T>` is
//! a slot accepting capability `T`.

use core::any::TypeId;
use core::marker::PhantomData;

use bevy_app::App;
use bevy_ecs::reflect::AppTypeRegistry;
use bevy_reflect::prelude::ReflectDefault;
use bevy_reflect::{FromType, Reflect, TypePath, TypeRegistry, Typed};

use crate::schema::SchemaError;

/// Type data marking a type as a slot marker, carrying the capability the
/// slot accepts.
#[derive(Clone)]
pub struct ReflectSlot {
    pub capability: TypeId,
    pub capability_path: &'static str,
}

impl<T: TypePath + Send + Sync + 'static> FromType<Slot<T>> for ReflectSlot {
    fn from_type() -> Self {
        Self {
            capability: TypeId::of::<T>(),
            capability_path: T::type_path(),
        }
    }
}

/// Marks a `Slots` field as a named `Feeds` input accepting capability `T`.
///
/// Zero-sized: a `Feeds` edge carries no value, and the target reads its
/// source's component or handle (parent §2.10). `PhantomData<fn() -> T>`
/// rather than `PhantomData<T>` so the marker is `Send + Sync` regardless of
/// `T`, matching `Event<T>`'s shape in `ports.rs`.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Default)]
pub struct Slot<T: TypePath + Send + Sync + 'static> {
    #[reflect(ignore)]
    _marker: PhantomData<fn() -> T>,
}

impl<T: TypePath + Send + Sync + 'static> Default for Slot<T> {
    fn default() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

/// The `Slots` type for a node with no structural inputs.
#[derive(Reflect, Default, Debug, Clone, Copy)]
pub struct NoSlots;

/// The `Outputs` type for a node with no output ports — the geometry
/// operators, whose product is a component rather than a port.
#[derive(Reflect, Default, Debug, Clone, Copy)]
pub struct NoOutputs;

/// Registers `Slot<T>` and its `ReflectSlot` data. A node type with a
/// `Slot<T>` field must call this in its `register`.
pub fn register_slot<T: TypePath + Send + Sync + 'static>(app: &mut App) {
    let registry = app.world().resource::<AppTypeRegistry>().clone();
    let mut registry = registry.write();
    registry.register::<Slot<T>>();
    registry.register_type_data::<Slot<T>, ReflectSlot>();
}

/// A resolved `Feeds` source: the entity a cook reads from, plus its position
/// in the compiled plans, which is how the cook gate reaches its
/// `produced_change_tick` fn without a second registry lookup.
///
/// Lives here rather than in `compile` so that `SlotView` (Task 3) can name it
/// without depending on compilation.
#[derive(Debug, Clone, Copy)]
pub struct SlotSource {
    pub entity: bevy_ecs::entity::Entity,
    pub plan_index: usize,
}

/// One slot, as derived from one `Slots` field.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotField {
    pub name: &'static str,
    pub field_index: usize,
    /// The capability this slot accepts — compared against the source node's
    /// `Produces` in the structure pass.
    pub capability: TypeId,
    pub capability_path: &'static str,
}

pub fn derive_slots<T: Typed>(registry: &TypeRegistry) -> Result<Vec<SlotField>, SchemaError> {
    let info = T::type_info();
    let s = info.as_struct().map_err(|_| SchemaError::NotAStruct {
        type_path: info.type_path(),
    })?;

    let mut slots = Vec::new();
    for i in 0..s.field_len() {
        let field = s.field_at(i).expect("index below field_len");
        match registry.get_type_data::<ReflectSlot>(field.type_id()) {
            Some(slot) => slots.push(SlotField {
                name: field.name(),
                field_index: i,
                capability: slot.capability,
                capability_path: slot.capability_path,
            }),
            None => {
                // Mirrors `schema::is_event_marker_path`: a `Slot<_>` field
                // whose type data is missing would otherwise silently not be
                // a slot at all.
                if is_slot_marker_path(field.type_path()) {
                    return Err(SchemaError::UnregisteredSlotField {
                        type_path: info.type_path(),
                        field: field.name(),
                    });
                }
            }
        }
    }
    Ok(slots)
}

/// Recognises `sway_graph::slots::Slot<..>` by path. The authoritative test
/// is the `ReflectSlot` type data above; this is the diagnostic for its
/// absence.
fn is_slot_marker_path(path: &str) -> bool {
    path.starts_with("sway_graph::slots::Slot<")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_reflect::{Reflect, TypePath, TypeRegistry};

    #[derive(TypePath)]
    struct FakeGeometry;

    #[derive(TypePath)]
    struct FakeMaterial;

    #[derive(Reflect, Default)]
    struct MeshSlots {
        geo: Slot<FakeGeometry>,
        material: Slot<FakeMaterial>,
    }

    fn registry() -> TypeRegistry {
        let mut r = TypeRegistry::new();
        r.register::<MeshSlots>();
        r.register::<Slot<FakeGeometry>>();
        r.register_type_data::<Slot<FakeGeometry>, ReflectSlot>();
        r.register::<Slot<FakeMaterial>>();
        r.register_type_data::<Slot<FakeMaterial>, ReflectSlot>();
        r
    }

    #[test]
    fn a_slot_field_carries_its_capability_not_the_marker_type() {
        // The structure pass compares a source's Produces TypeId against
        // this, so it must be the capability, not Slot<capability> (§4).
        let slots = derive_slots::<MeshSlots>(&registry()).expect("slots");

        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].name, "geo");
        assert_eq!(slots[0].capability, core::any::TypeId::of::<FakeGeometry>());
        assert_ne!(
            slots[0].capability,
            core::any::TypeId::of::<Slot<FakeGeometry>>()
        );
        assert_eq!(slots[1].name, "material");
        assert_eq!(slots[1].capability, core::any::TypeId::of::<FakeMaterial>());
    }

    #[test]
    fn slot_ordinals_are_field_order() {
        let slots = derive_slots::<MeshSlots>(&registry()).expect("slots");
        assert_eq!(slots[0].field_index, 0);
        assert_eq!(slots[1].field_index, 1);
    }

    #[test]
    fn a_node_with_no_slots_derives_an_empty_list() {
        let mut r = TypeRegistry::new();
        r.register::<NoSlots>();
        assert!(derive_slots::<NoSlots>(&r).expect("empty").is_empty());
    }

    #[test]
    fn an_unregistered_slot_field_is_an_error_not_a_silent_omission() {
        // The failure this prevents: a node author adds a Slot<T> field but
        // forgets register_slot, the slot vanishes from the schema, and every
        // FeedsEdge into it reports "slot ordinal out of range" instead of
        // naming the real mistake.
        let mut r = TypeRegistry::new();
        r.register::<MeshSlots>();
        r.register::<Slot<FakeGeometry>>();
        r.register::<Slot<FakeMaterial>>();
        // deliberately NOT register_type_data::<_, ReflectSlot>

        let msg = derive_slots::<MeshSlots>(&r).unwrap_err().to_string();
        assert!(msg.contains("geo"), "message must name the field: {msg}");
        assert!(msg.contains("register_slot"), "message must say the fix: {msg}");
    }
}
