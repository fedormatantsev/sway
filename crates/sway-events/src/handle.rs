//! [`EventHandle`]: the value a trigger wire carries.

use core::any::Any;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;

use bevy_reflect::std_traits::ReflectDefault;
use bevy_reflect::{Reflect, ReflectDeserialize, ReflectSerialize, TypePath};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A name for one batch of occurrences of payload type `P`, valid for exactly
/// the tick it was published in.
///
/// A handle is two integers and a type tag: the arena's `generation` at the
/// moment of publication (design D3/D4 — what makes a handle from an earlier
/// tick read as *empty* rather than as whatever now occupies its slot), and
/// the `slot` naming the batch within that generation.
///
/// It is a **name, not a capability**: [`EventArena::read`] is the only thing
/// it opens, so a consumer holding one has no operation that could add to the
/// batch it received (design D1).
///
/// `PhantomData<fn() -> P>` rather than `PhantomData<P>` so the handle is
/// `Send + Sync` however `P` is spelled — a payload type needs no bounds it
/// does not otherwise want.
///
/// [`EventArena::read`]: crate::EventArena::read
#[derive(Reflect)]
#[reflect(opaque)]
#[reflect(Default, PartialEq, Debug, Serialize, Deserialize)]
#[reflect(where P: TypePath + Any)]
pub struct EventHandle<P> {
    generation: u64,
    slot: u32,
    _p: PhantomData<fn() -> P>,
}

impl<P> EventHandle<P> {
    /// The handle that names no batch: it reads as no occurrences on every
    /// tick and never becomes stale.
    ///
    /// This is what makes a freshly created node, a freshly loaded node and an
    /// unconnected inlet all correct with no linking step at spawn — the field
    /// starts as a handle that reads as nothing rather than as a dangling name
    /// something has to go and fix up.
    pub const EMPTY: Self = Self {
        // Generation 0 is never a live generation: the arena starts at 1, so
        // the sentinel cannot collide with a real handle.
        generation: 0,
        slot: 0,
        _p: PhantomData,
    };

    /// Builds a handle naming `slot` within `generation`. Crate-private: a
    /// handle becomes a value on a wire only by a node publishing it.
    pub(crate) const fn new(generation: u64, slot: u32) -> Self {
        Self {
            generation,
            slot,
            _p: PhantomData,
        }
    }

    /// The generation this handle was published in. `0` for [`Self::EMPTY`].
    pub(crate) const fn generation(self) -> u64 {
        self.generation
    }

    /// The slot this handle names within its generation.
    pub(crate) const fn slot(self) -> u32 {
        self.slot
    }

    /// Whether this handle names no batch at all.
    pub const fn is_empty(self) -> bool {
        self.generation == 0
    }
}

impl<P> Default for EventHandle<P> {
    fn default() -> Self {
        Self::EMPTY
    }
}

// `Copy`/`Clone`/`PartialEq`/`Eq`/`Hash`/`Debug` by hand rather than derived:
// the derives would put a `P: Copy` (and so on) bound on the impl, and a
// handle's payload type never has to be any of those things.
impl<P> Clone for EventHandle<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Copy for EventHandle<P> {}

impl<P> PartialEq for EventHandle<P> {
    /// **The generation is part of equality, and cannot be dropped from it**
    /// (design D7). Two handles from different ticks naming the same slot
    /// describe different batches; if they compared equal, propagate would
    /// skip the write as an equal value, the consumer would keep last tick's
    /// handle, and reading it would yield *nothing* — the mechanism would
    /// silently stop working. The cost is that a publishing node is reported
    /// changed every tick it publishes, which is specified rather than
    /// worked around.
    fn eq(&self, other: &Self) -> bool {
        self.generation == other.generation && self.slot == other.slot
    }
}

impl<P> Eq for EventHandle<P> {}

impl<P> Hash for EventHandle<P> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.generation.hash(state);
        self.slot.hash(state);
    }
}

impl<P> core::fmt::Debug for EventHandle<P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.generation == 0 {
            f.write_str("EventHandle::EMPTY")
        } else {
            write!(f, "EventHandle({}:{})", self.generation, self.slot)
        }
    }
}

/// Design D8: a handle is session state, so what a document records of it
/// names no batch and no generation. Writing the empty handle rather than the
/// live one is also what keeps saves byte-stable — a document saved on two
/// different ticks is identical — and there is nothing meaningful to restore,
/// since the first tick's propagate re-establishes the inlet anyway.
impl<P> Serialize for EventHandle<P> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_unit()
    }
}

impl<'de, P> Deserialize<'de> for EventHandle<P> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <()>::deserialize(deserializer)?;
        Ok(Self::EMPTY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(TypePath)]
    struct Ping;

    #[test]
    fn the_empty_handle_equals_itself_and_no_live_handle() {
        let empty: EventHandle<Ping> = EventHandle::EMPTY;
        assert_eq!(empty, EventHandle::EMPTY);
        assert_eq!(empty, EventHandle::default());
        assert!(empty.is_empty());
        assert_ne!(empty, EventHandle::<Ping>::new(1, 0));
    }

    #[test]
    fn two_generations_of_one_slot_are_not_equal() {
        // Design D7: dropping the generation from equality would make
        // propagate skip the write and the consumer read nothing.
        assert_ne!(
            EventHandle::<Ping>::new(1, 0),
            EventHandle::<Ping>::new(2, 0)
        );
        assert_eq!(
            EventHandle::<Ping>::new(1, 0),
            EventHandle::<Ping>::new(1, 0)
        );
    }

    #[test]
    fn a_live_handle_round_trips_through_serde_as_the_empty_one() {
        let live = EventHandle::<Ping>::new(7, 3);
        let text = ron::to_string(&live).expect("a handle serializes");
        let back: EventHandle<Ping> = ron::from_str(&text).expect("and deserializes");
        assert_eq!(back, EventHandle::EMPTY, "it names no batch and no tick");
    }

    #[test]
    fn a_handle_is_send_and_sync_whatever_its_payload_is() {
        // `Rc` is neither, which is exactly the case `PhantomData<fn() -> P>`
        // is there for.
        #[derive(TypePath)]
        struct NotThreadSafe(#[expect(dead_code)] std::rc::Rc<u32>);

        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<EventHandle<NotThreadSafe>>();
    }

    #[test]
    fn a_handle_debug_prints_without_naming_a_batch_it_does_not_have() {
        assert_eq!(
            format!("{:?}", EventHandle::<Ping>::EMPTY),
            "EventHandle::EMPTY"
        );
        assert_eq!(
            format!("{:?}", EventHandle::<Ping>::new(2, 5)),
            "EventHandle(2:5)"
        );
    }
}
