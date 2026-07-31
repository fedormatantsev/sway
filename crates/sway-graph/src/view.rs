//! `PortView` — a node's typed, scoped window onto the arena. Spec §4.
//!
//! Indices passed here are **node-relative ordinals** — the same ordinals a
//! node's index consts use (`Gain::GAIN`, `Emitter::OUT_PULSE`, ...) — not
//! absolute arena slots. `PortView` resolves them against the node's own
//! `continuous_base`/`event_base` internally, which is what stops a node
//! reaching another node's ports by arithmetic (spec §4).

use bevy_reflect::Reflect;

use crate::ports::{ContinuousIdx, EventIdx, Occurrence, PortArena};

/// Context shared by every node ticked this frame. Spec §6/§7.
pub struct TickCtx {
    /// The fixed timestep, in seconds.
    pub dt: f32,
    /// Absolute start of this tick's window, in seconds. A node needing
    /// absolute time writes `ctx.tick_start + offset as f64` (spec §7).
    pub tick_start: f64,
    /// Monotonically increasing tick counter, starting at 0.
    pub tick_index: u64,
}

/// One event occurrence, downcast to its payload type and borrowed from the
/// arena for the duration of the read.
pub struct EventRef<'a, T> {
    pub offset: f32,
    pub value: &'a T,
}

/// Scoped to one node: indices are that node's own ordinals, resolved against
/// its bases here, so a node cannot reach another node's ports by arithmetic.
pub struct PortView<'a> {
    arena: &'a mut PortArena,
    continuous_base: usize,
    event_base: usize,
}

impl<'a> PortView<'a> {
    pub fn new(arena: &'a mut PortArena, continuous_base: usize, event_base: usize) -> Self {
        Self {
            arena,
            continuous_base,
            event_base,
        }
    }

    /// Reads a continuous port's current value, downcast and cloned.
    ///
    /// A compile-validated graph guarantees the slot holds exactly `T` — the
    /// compiler checks every edge's `type_id` and prefill only ever writes
    /// the node's own authored field type — so a downcast failure here means
    /// the compiler failed to catch a type mismatch, not a normal runtime
    /// condition. Panicking (rather than silently skipping) is deliberate:
    /// the tick is documented infallible for genuinely valid graphs.
    pub fn read<T: Reflect + Clone>(&self, idx: ContinuousIdx) -> T {
        let slot = self.continuous_base + idx.0 as usize;
        self.arena.continuous[slot]
            .try_downcast_ref::<T>()
            .unwrap_or_else(|| {
                panic!(
                    "PortView::read: continuous port {} does not hold a `{}` — the compiler \
                     should have caught this type mismatch before the tick ran",
                    idx.0,
                    core::any::type_name::<T>()
                )
            })
            .clone()
    }

    /// Overwrites a continuous port's slot. Immediate — a node later in
    /// topological order sees this within the same tick (spec §6).
    pub fn write<T: Reflect>(&mut self, idx: ContinuousIdx, value: T) {
        let slot = self.continuous_base + idx.0 as usize;
        self.arena.continuous[slot] = Box::new(value);
    }

    /// Iterates this tick's occurrences on an event input, downcast to their
    /// payload type. Empty if nothing arrived this tick (spec §4).
    pub fn events<T: Reflect>(&self, idx: EventIdx) -> impl Iterator<Item = EventRef<'_, T>> {
        let slot = self.event_base + idx.0 as usize;
        self.arena.events[slot].iter().filter_map(|occ| {
            occ.value.try_downcast_ref::<T>().map(|value| EventRef {
                offset: occ.offset,
                value,
            })
        })
    }

    /// Appends an occurrence to an event output's slot for this tick.
    pub fn emit<T: Reflect>(&mut self, idx: EventIdx, offset: f32, value: T) {
        let slot = self.event_base + idx.0 as usize;
        self.arena.events[slot].push(Occurrence {
            offset,
            value: Box::new(value),
        });
    }
}
