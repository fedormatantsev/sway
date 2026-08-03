//! The port arena: where signal values live between nodes.
//!
//! Spec §4. Two collections, not one enum: nothing iterates slots
//! kind-agnostically, so an enum would buy a discriminant and a match arm at
//! every access and nothing else.

use core::marker::PhantomData;

use bevy_ecs::entity::Entity;
use bevy_ecs::resource::Resource;
use bevy_reflect::prelude::ReflectDefault;
use bevy_reflect::{FromReflect, GetTypeRegistration, PartialReflect, Reflect, Typed, TypePath};

/// Index of a continuous port, absolute within [`PortArena::continuous`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct ContinuousIdx(pub u32);

/// Index of an event port, absolute within [`PortArena::events`].
///
/// A distinct newtype from [`ContinuousIdx`] so that reading a continuous
/// port as an event stream is a type error rather than a runtime panic.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct EventIdx(pub u32);

/// One event occurrence in the pre-unification arena, with a boxed payload.
///
/// Deleted in the same change that replaces `PortArena::events`; the typed
/// `Occurrence<T>` below is its replacement.
pub struct BoxedOccurrence {
    pub offset: f32,
    pub value: Box<dyn PartialReflect>,
}

#[derive(Resource)]
pub struct PortArena {
    /// Persists across ticks — a continuous port always holds a current value.
    pub continuous: Vec<Box<dyn PartialReflect>>,
    /// Cleared at tick start — zero or more occurrences for *this* tick only.
    pub events: Vec<Vec<BoxedOccurrence>>,
}

impl PortArena {
    pub fn new(continuous_len: usize, events_len: usize) -> Self {
        Self {
            continuous: (0..continuous_len)
                .map(|_| Box::new(()) as Box<dyn PartialReflect>)
                .collect(),
            events: (0..events_len).map(|_| Vec::new()).collect(),
        }
    }

    /// Clears every event slot, retaining each vec's allocation.
    pub fn clear_events(&mut self) {
        for slot in &mut self.events {
            slot.clear();
        }
    }

    /// Grows or shrinks to a new compiled layout, keeping the continuous
    /// values that still have a slot. Recompilation calls this.
    pub fn resize(&mut self, continuous_len: usize, events_len: usize) {
        self.continuous
            .resize_with(continuous_len, || Box::new(()) as Box<dyn PartialReflect>);
        self.events.resize_with(events_len, Vec::new);
    }
}

/// Marks a `Params`/`Outputs` field as an **event** port.
///
/// Zero-sized: the occurrences live in [`PortArena::events`], not in the
/// struct. An event input has no authored value (spec §3), which is why this
/// carries no data — there is nothing for an author to write.
///
/// `PhantomData<fn() -> T>` rather than `PhantomData<T>` so the marker is
/// `Send + Sync + Default` regardless of `T`.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Default)]
pub struct Event<T: Reflect + TypePath> {
    #[reflect(ignore)]
    _marker: PhantomData<fn() -> T>,
}

impl<T: Reflect + TypePath> Default for Event<T> {
    fn default() -> Self {
        Self { _marker: PhantomData }
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
    fn continuous_slots_persist_across_event_clears() {
        let mut arena = PortArena::new(2, 1);
        arena.continuous[0] = Box::new(0.75_f32);
        arena.events[0].push(BoxedOccurrence { offset: 0.004, value: Box::new(7_u8) });

        arena.clear_events();

        // Spec §4: continuous persists, events clear. This is what makes
        // "CC is 0" distinguishable from "no CC arrived".
        assert_eq!(
            arena.continuous[0].try_downcast_ref::<f32>().copied(),
            Some(0.75)
        );
        assert!(arena.events[0].is_empty());
    }

    #[test]
    fn clearing_events_retains_allocation() {
        let mut arena = PortArena::new(0, 1);
        for i in 0..16 {
            arena.events[0].push(BoxedOccurrence { offset: i as f32, value: Box::new(i) });
        }
        let cap = arena.events[0].capacity();

        arena.clear_events();

        // Spec §4 claims per-tick event churn goes to zero after warm-up.
        // That is only true if clear() keeps the buffer.
        assert!(arena.events[0].capacity() >= cap);
    }

    #[test]
    fn resize_preserves_existing_continuous_values() {
        // Recompilation resizes the arena; a graph that grew must not lose
        // the values of the nodes that survived.
        let mut arena = PortArena::new(1, 0);
        arena.continuous[0] = Box::new(3.5_f32);

        arena.resize(3, 2);

        assert_eq!(
            arena.continuous[0].try_downcast_ref::<f32>().copied(),
            Some(3.5)
        );
        assert_eq!(arena.continuous.len(), 3);
        assert_eq!(arena.events.len(), 2);
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
