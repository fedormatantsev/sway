//! [`EventArena`]: this tick's batches, and the two operations that reach them.

use core::any::Any;
use core::cell::RefCell;
use core::ops::Deref;
use std::rc::Rc;

use crate::handle::EventHandle;

/// An owned share of one published batch: what [`EventArena::read`] hands
/// back, deref'ing to `[P]` so a consumer iterates the payloads without
/// copying any of them.
///
/// It is an owned share rather than a borrow of the arena, which is what lets
/// a node read a handle and then publish a batch of its own in the same scope
/// (design D2). The `Rc` is hidden behind this type, so moving the arena to
/// `Arc` later is a one-line change behind an unchanged API.
#[derive(Debug)]
pub struct EventBatch<P>(Rc<Vec<P>>);

impl<P> Clone for EventBatch<P> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<P> Deref for EventBatch<P> {
    type Target = [P];

    fn deref(&self) -> &[P] {
        &self.0
    }
}

impl<'a, P> IntoIterator for &'a EventBatch<P> {
    type Item = &'a P;
    type IntoIter = core::slice::Iter<'a, P>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// This tick's batches of occurrences, held outside the graph and addressed by
/// [`EventHandle`].
///
/// Inserted as a **non-send** resource by
/// [`EventsPlugin`](crate::EventsPlugin): `World::get_non_send_resource` is
/// bound by `'static` alone, so neither the arena nor a payload type needs
/// `Send`/`Sync`, and the arena is still reachable from the `&World` a node's
/// `evaluate` is handed. The tick is an exclusive system on the main thread,
/// which is the only thread this resource is ever touched from (design D2).
///
/// **Why a `RefCell` and no lock, no `unsafe`, and no reachable panic:** a
/// published batch is never modified again, so the only thing needing interior
/// mutability is the *slot table* — and no borrow of it escapes a method.
/// `publish` borrows to append and drops the borrow before it returns; `read`
/// borrows to clone one `Rc` out and drops the borrow before it returns. A
/// read can therefore never be live across a later publish, so the two can
/// never conflict.
#[derive(Debug)]
pub struct EventArena {
    /// Bumped on every clear. Generation 0 is reserved for
    /// [`EventHandle::EMPTY`], so a live arena starts at 1.
    generation: u64,
    slots: RefCell<Vec<Rc<dyn Any>>>,
}

impl Default for EventArena {
    fn default() -> Self {
        Self {
            generation: 1,
            slots: RefCell::new(Vec::new()),
        }
    }
}

impl EventArena {
    /// Hands a whole batch to the arena and returns the handle naming it.
    ///
    /// **An empty batch yields [`EventHandle::EMPTY`] and allocates no slot**
    /// (design D7). That is what keeps a producer which publishes
    /// unconditionally from dirtying its whole downstream on a tick where
    /// nothing happened: the mistake it would otherwise make — a live handle
    /// to an empty `Vec`, new every tick — compares unequal every tick while
    /// carrying nothing.
    pub fn publish<P: 'static>(&self, occurrences: impl IntoIterator<Item = P>) -> EventHandle<P> {
        let batch: Vec<P> = occurrences.into_iter().collect();
        if batch.is_empty() {
            return EventHandle::EMPTY;
        }
        let mut slots = self.slots.borrow_mut();
        let slot = slots.len() as u32;
        slots.push(Rc::new(batch) as Rc<dyn Any>);
        // The borrow is dropped here, before the handle leaves the method.
        drop(slots);
        EventHandle::new(self.generation, slot)
    }

    /// Reads the batch a handle names.
    ///
    /// `None` — which a consumer treats as "no occurrences" — for the empty
    /// handle, for a handle published in an earlier generation (design D4:
    /// staleness is read, not prevented), and for a slot whose payload type is
    /// not `P`.
    pub fn read<P: 'static>(&self, handle: EventHandle<P>) -> Option<EventBatch<P>> {
        if handle.is_empty() || handle.generation() != self.generation {
            return None;
        }
        let slots = self.slots.borrow();
        let batch = slots.get(handle.slot() as usize)?;
        // Downcast before the borrow is dropped, clone the `Rc` out, and hand
        // back something owned: nothing is borrowed when this returns.
        let batch = Rc::downcast::<Vec<P>>(Rc::clone(batch)).ok()?;
        drop(slots);
        Some(EventBatch(batch))
    }

    /// Drops every batch and bumps the generation, which is the whole of
    /// emptying the arena (design D5). O(batches), not O(nodes): it reads no
    /// graph, no type registry, and no per-kind index of which fields are
    /// handles.
    pub fn clear(&mut self) {
        self.slots.get_mut().clear();
        self.generation = self.generation.wrapping_add(1);
        // Generation 0 is `EMPTY`'s. Skipping it keeps the sentinel from ever
        // colliding with a live handle, however long the show runs.
        if self.generation == 0 {
            self.generation = 1;
        }
    }

    /// How many batches the arena is holding. Test-facing: nothing in a show
    /// build asks.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.slots.borrow().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Ping(u32);

    #[derive(Debug, PartialEq)]
    struct Pong(u32);

    #[test]
    fn a_published_batch_reads_back_in_order() {
        let arena = EventArena::default();
        let handle = arena.publish([Ping(1), Ping(2), Ping(3)]);

        let batch = arena.read(handle).expect("the batch it just published");

        assert_eq!(&*batch, &[Ping(1), Ping(2), Ping(3)]);
    }

    #[test]
    fn reading_does_not_consume() {
        let arena = EventArena::default();
        let handle = arena.publish([Ping(1), Ping(2)]);

        let first = arena.read(handle).expect("a batch");
        let second = arena.read(handle).expect("and the same batch again");

        assert_eq!(&*first, &*second);
        assert_eq!(first.len(), 2);
    }

    #[test]
    fn a_handle_from_before_a_clear_reads_nothing() {
        let mut arena = EventArena::default();
        let handle = arena.publish([Ping(1)]);

        arena.clear();

        assert!(arena.read(handle).is_none());
    }

    #[test]
    fn a_stale_handle_does_not_read_the_batch_that_took_its_slot() {
        // Design D4: the generation is what makes staleness *readable and
        // empty* rather than a silent read of another producer's occurrences.
        let mut arena = EventArena::default();
        let stale = arena.publish([Ping(1)]);
        arena.clear();
        let fresh = arena.publish([Ping(99)]);
        assert_eq!(stale.slot(), fresh.slot(), "the same slot was reused");

        assert!(arena.read(stale).is_none());
        assert_eq!(&*arena.read(fresh).expect("the live batch"), &[Ping(99)]);
    }

    #[test]
    fn the_empty_handle_reads_nothing_in_every_generation() {
        let mut arena = EventArena::default();
        for _ in 0..3 {
            assert!(arena.read(EventHandle::<Ping>::EMPTY).is_none());
            arena.publish([Ping(0)]);
            arena.clear();
        }
    }

    #[test]
    fn publishing_an_empty_batch_is_the_empty_handle_and_costs_no_slot() {
        let arena = EventArena::default();

        let handle = arena.publish(Vec::<Ping>::new());

        assert_eq!(handle, EventHandle::EMPTY);
        assert_eq!(arena.len(), 0, "no slot was allocated");
    }

    #[test]
    fn reading_a_handle_whose_payload_type_differs_yields_nothing() {
        let arena = EventArena::default();
        let ping = arena.publish([Ping(1)]);
        // Only reachable by forging a handle, which is why `new` is
        // crate-private — but a mismatch must answer `None`, not panic.
        let forged = EventHandle::<Pong>::new(ping.generation(), ping.slot());

        assert!(arena.read(forged).is_none());
    }

    #[test]
    fn a_live_batch_is_an_owned_share_not_a_borrow_of_the_arena() {
        // Design D2: this is the shape that would deadlock if `read` handed
        // back a `Ref` into the slot table — a node reading its inlet and then
        // publishing its own batch is the ordinary `Relay` case.
        let arena = EventArena::default();
        let first = arena.publish([Ping(1)]);

        let batch = arena.read(first).expect("a batch");
        let second = arena.publish([Ping(2)]);

        assert_eq!(&*batch, &[Ping(1)], "still readable after the publish");
        assert_eq!(&*arena.read(second).expect("the new batch"), &[Ping(2)]);
    }

    #[test]
    fn a_batch_outlives_the_clear_that_forgot_it() {
        let mut arena = EventArena::default();
        let handle = arena.publish([Ping(1)]);
        let batch = arena.read(handle).expect("a batch");

        arena.clear();

        assert_eq!(&*batch, &[Ping(1)], "a refcount, not a dangling reference");
        assert_eq!(arena.len(), 0);
    }

    #[test]
    fn clearing_never_lands_on_the_sentinel_generation() {
        let mut arena = EventArena {
            generation: u64::MAX,
            ..Default::default()
        };

        arena.clear();

        assert_ne!(arena.generation, 0, "generation 0 is `EMPTY`'s alone");
        let handle = arena.publish([Ping(1)]);
        assert!(!handle.is_empty());
        assert!(arena.read(handle).is_some());
    }
}
