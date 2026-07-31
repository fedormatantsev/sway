//! The port arena: where signal values live between nodes.
//!
//! Spec §4. Two collections, not one enum: nothing iterates slots
//! kind-agnostically, so an enum would buy a discriminant and a match arm at
//! every access and nothing else.

use bevy_ecs::resource::Resource;
use bevy_reflect::PartialReflect;

/// Index of a continuous port, absolute within [`PortArena::continuous`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct ContinuousIdx(pub u32);

/// Index of an event port, absolute within [`PortArena::events`].
///
/// A distinct newtype from [`ContinuousIdx`] so that reading a continuous
/// port as an event stream is a type error rather than a runtime panic.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct EventIdx(pub u32);

/// One event occurrence, stamped with its offset inside the tick window.
///
/// `offset` is seconds from the tick's start, so it is bounded by the
/// timestep (~8.3ms at 120Hz) and f32 has precision to spare. A node needing
/// absolute time writes `ctx.tick_start + offset as f64` (spec §7).
pub struct Occurrence {
    pub offset: f32,
    pub value: Box<dyn PartialReflect>,
}

#[derive(Resource)]
pub struct PortArena {
    /// Persists across ticks — a continuous port always holds a current value.
    pub continuous: Vec<Box<dyn PartialReflect>>,
    /// Cleared at tick start — zero or more occurrences for *this* tick only.
    pub events: Vec<Vec<Occurrence>>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_slots_persist_across_event_clears() {
        let mut arena = PortArena::new(2, 1);
        arena.continuous[0] = Box::new(0.75_f32);
        arena.events[0].push(Occurrence { offset: 0.004, value: Box::new(7_u8) });

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
            arena.events[0].push(Occurrence { offset: i as f32, value: Box::new(i) });
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
}
