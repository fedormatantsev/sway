//! The port arena: where signal values live between nodes.
//!
//! One collection: every slot holds a value, whether that value is a plain
//! reflect value, an `Events<T>` list, or a `Product<T>` reference.

use core::marker::PhantomData;

use bevy_ecs::entity::Entity;
use bevy_ecs::resource::Resource;
use bevy_reflect::{FromReflect, GetTypeRegistration, PartialReflect, Reflect, Typed, TypePath};

/// Where every port value lives between nodes.
///
/// One collection, because every slot now holds a value: a plain reflect
/// value, an `Events<T>` list, or a `Product<T>` reference. The pre-unification
/// arena had a second collection for events, which is no longer a different
/// kind of thing.
#[derive(Resource)]
pub struct PortArena {
    pub values: Vec<Box<dyn PartialReflect>>,
}

impl PortArena {
    pub fn new(len: usize) -> Self {
        Self {
            // `()` rather than a zero: an unwritten read is then visibly
            // wrong rather than plausibly 0.0.
            values: (0..len).map(|_| Box::new(()) as Box<dyn PartialReflect>).collect(),
        }
    }

    /// Grows or shrinks to a new compiled layout, keeping the values that
    /// still have a slot. Recompilation calls this.
    pub fn resize(&mut self, len: usize) {
        self.values
            .resize_with(len, || Box::new(()) as Box<dyn PartialReflect>);
    }
}

/// The capability a scene node produces and a `children` inlet accepts.
///
/// The engine knows this one capability by name, because Bevy owns the scene
/// hierarchy: an edge into a `Product<Spatial>` inlet also emits `ChildOf`,
/// a `Product<Spatial>` outlet may feed at most one inlet, and `Spatial`
/// edges are excluded from the compiled order (design §3).
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct Spatial;

/// One event occurrence, stamped with its offset inside the tick window.
///
/// `offset` is seconds from the tick's start, so it is bounded by the
/// timestep (~8.3ms at 120Hz) and f32 has precision to spare.
///
/// Typed rather than boxed: one allocation for the whole [`Events`] list
/// replaces one box per occurrence, which is the allocation M2a identified as
/// the tick's dominant cost.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct Occurrence<T> {
    pub offset: f32,
    pub value: T,
}

/// An event port's value: the occurrences that landed this tick.
///
/// Empty means "nothing arrived", which is what distinguishes it from a
/// continuous value of zero (parent §2.4). Emptied before every tick by the
/// runner, in place, through [`clear_events_of`].
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct Events<T> {
    pub occurrences: Vec<Occurrence<T>>,
}

impl<T> Default for Events<T> {
    // Not derived: `#[derive(Default)]` would demand `T: Default`, and an
    // empty list needs nothing from `T`.
    fn default() -> Self {
        Self { occurrences: Vec::new() }
    }
}

/// A structural port's value: the entity that produces capability `T`.
///
/// The produced data itself never enters the arena — only this reference does
/// — so parent §2.1's rule that high-cardinality data lives in the ECS is
/// untouched. `None` is an unconnected inlet, which is also its authored
/// value, so the shadowing rule of parent §2.11 needs no special case here.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub struct Product<T: TypePath + Send + Sync + 'static> {
    pub source: Option<Entity>,
    #[reflect(ignore, clone)]
    _marker: PhantomData<fn() -> T>,
}

impl<T: TypePath + Send + Sync + 'static> Default for Product<T> {
    fn default() -> Self {
        Self { source: None, _marker: PhantomData }
    }
}

impl<T: TypePath + Send + Sync + 'static> Product<T> {
    pub fn from_source(source: Entity) -> Self {
        Self { source: Some(source), _marker: PhantomData }
    }
}

/// Empties an `Events<T>` slot **in place**, keeping its allocation.
///
/// Registered per payload type as a fn pointer (Task 2's `ReflectEventList`)
/// so the runner can clear a slot without knowing `T`. Replacing the value
/// with a fresh `Events::default()` would be correct and would also throw
/// away the buffer every tick — see the test above.
pub fn clear_events_of<T>(value: &mut dyn PartialReflect)
where
    T: Reflect + TypePath + Typed + FromReflect + GetTypeRegistration,
{
    if let Some(events) = value.try_downcast_mut::<Events<T>>() {
        events.occurrences.clear();
    }
}

/// Absolute index into [`PortArena`]'s slots.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct SlotIdx(pub u32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_preserves_existing_values() {
        let mut arena = PortArena::new(1);
        arena.values[0] = Box::new(3.5_f32);

        arena.resize(3);

        assert_eq!(arena.values[0].try_downcast_ref::<f32>().copied(), Some(3.5));
        assert_eq!(arena.values.len(), 3);
    }

    #[test]
    fn a_product_survives_reflect_clone_with_its_source() {
        // The gather clones every slot every tick via reflect_clone. A
        // Product whose `source` did not survive that would silently
        // disconnect every structural edge on the first tick.
        use bevy_ecs::entity::Entity;
        use bevy_reflect::PartialReflect;

        let original = Product::<Spatial>::from_source(Entity::from_raw_u32(7).unwrap());
        let cloned = original
            .reflect_clone()
            .expect("Product must reflect_clone")
            .into_partial_reflect();

        let cloned = cloned
            .try_downcast_ref::<Product<Spatial>>()
            .expect("reflect_clone must preserve the concrete type, not produce a proxy");
        assert_eq!(cloned.source, original.source);
        assert_eq!(cloned.source, Entity::from_raw_u32(7));
    }

    #[test]
    fn events_survive_reflect_clone_with_their_occurrences() {
        let mut original = Events::<u8>::default();
        original.occurrences.push(Occurrence { offset: 0.25, value: 9 });

        let cloned = original
            .reflect_clone()
            .expect("Events must reflect_clone")
            .into_partial_reflect();
        let cloned = cloned
            .try_downcast_ref::<Events<u8>>()
            .expect("reflect_clone must preserve the concrete type");

        assert_eq!(cloned.occurrences.len(), 1);
        assert_eq!(cloned.occurrences[0].offset, 0.25);
        assert_eq!(cloned.occurrences[0].value, 9);
    }

    #[test]
    fn clearing_events_in_place_retains_the_allocation() {
        // Spec §8: this is the one axis where a merged arena can be worse
        // than the split one. Clearing must empty the existing Vec, never
        // replace the value with a fresh Events::default().
        let mut events = Events::<u8>::default();
        for i in 0..16 {
            events.occurrences.push(Occurrence { offset: i as f32, value: i });
        }
        let capacity = events.occurrences.capacity();

        let mut boxed: Box<dyn bevy_reflect::PartialReflect> = Box::new(events);
        clear_events_of::<u8>(&mut *boxed);

        let cleared = boxed.try_downcast_ref::<Events<u8>>().expect("still Events<u8>");
        assert!(cleared.occurrences.is_empty());
        assert!(
            cleared.occurrences.capacity() >= capacity,
            "clear must retain the buffer, not reallocate"
        );
    }

    #[test]
    fn an_unset_product_is_none() {
        assert_eq!(Product::<Spatial>::default().source, None);
    }
}
