//! The per-tick node view: the restricted, per-node window onto the port
//! arena and tick context that `NodeType::tick` implementations read and
//! write through.
//!
//! **Stub for Task 5.** Task 5 owns this module and fills in the actual
//! windowed access (continuous/event get/set scoped to one node's slice of
//! the arena) and the tick context (timestep, tick start time, etc). Task 3
//! only needs these types to exist and be constructible so that a stub
//! node's empty `tick` body type-checks.

/// Per-node window onto the port arena for the duration of one tick.
///
/// Task 5 will give this fields (arena reference, this node's base offsets)
/// and methods (`get_continuous`, `set_continuous`, `events`, `emit`, ...).
#[derive(Default)]
pub struct PortView;

/// Context shared by every node ticked this frame.
///
/// Task 5 will give this fields such as the timestep and tick start time
/// (spec §7).
#[derive(Default)]
pub struct TickCtx;
