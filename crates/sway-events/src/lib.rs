//! Occurrences carried through the graph's ordinary wires.
//!
//! A value wire carries a *level*: the tick copies a field, and a node reads
//! whatever stands there this tick. This crate is what carries the things that
//! *happen* — a note on, a beat boundary, a one-shot retrigger — over exactly
//! the same wires, with no new step kind, no new legality rule and no second
//! evaluation path.
//!
//! ## The arena and the handle
//!
//! The occurrences themselves live in an [`EventArena`] — a `World` resource
//! holding this tick's batches. What travels the wire is an [`EventHandle`]:
//! a small payload-typed value *naming* one of those batches. There are two
//! operations and nothing else reaches a batch — [`EventArena::publish`] takes
//! a whole batch and returns its handle, and [`EventArena::read`] takes a
//! handle and returns that batch.
//!
//! That split separates the read and write paths **structurally** rather than
//! by discipline (design D1). A handle is a name, not a capability: `read` is
//! the only thing it opens, so a consumer holding one has no operation that
//! could add to what it received. The "do not write into what you were handed"
//! rule a shared mutable buffer would have needed does not exist here.
//!
//! Fan-out is free for the same reason: every consumer of an outlet holds the
//! same handle and reads the same refcounted batch, so a batch is never
//! duplicated per connection.
//!
//! ## A producer holds no state
//!
//! A producing node publishes during its own `evaluate`: it hands its whole
//! batch to the arena, gets a handle back, and writes that handle to its own
//! outlet. It keeps nothing — no buffer, no occurrences, no handle — between
//! ticks. Everything it published is reachable from the handle standing on its
//! outlet, and only until the end of the tick. With nothing to publish it
//! writes [`EventHandle::EMPTY`], and publishing an empty batch yields that
//! same empty handle, so an unconditional producer cannot report a change on a
//! tick where nothing happened.
//!
//! ## A handle is valid for exactly one tick
//!
//! [`EventsPlugin`] empties the arena before every tick, in [`EventClearSet`],
//! ordered before `sway_graph::GraphTickSet`. A handle carries the generation
//! it was published in, so one that outlived its tick reads as *no
//! occurrences* — never as whatever now occupies its slot, and never as a
//! failed evaluation (design D4). Several specified behaviours fall out of
//! that instead of needing machinery: a producer that stops publishing leaves
//! nothing observable behind, and a trigger connection inside a cycle carries
//! nothing, because the handle its partner published last tick is stale.
//!
//! ## What the engine knows about all this
//!
//! Nothing. A handle is an ordinary reflected field value and the arena is an
//! ordinary world resource, so `sway-graph` names no handle, occurrence,
//! payload type or arena; this crate depends on it for `GraphTickSet` alone.

mod arena;
mod handle;
mod plugin;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

pub use arena::{EventArena, EventBatch};
pub use handle::EventHandle;
pub use plugin::{
    EventClearSet, EventsPlugin, RegisterEventHandle, clear_event_arena, register_event_handle,
};
