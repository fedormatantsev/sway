## Context

See `proposal.md` — Why. The constraints that shape the approach:

- **A node is one reflected value with three parts, and an edge is data.** The tick's only vocabulary is "copy the field at the source path into the field at the destination path" (`graph`: An edge addresses fields by path). Anything an event wire needs must be expressible as a *field type*, or it becomes a fourth mechanism.
- **Connect legality is exact type match** plus `Option<S>` and `Vec<S>` wrappers, decided at connect time from reflected type info. Two different types cannot connect, however related they look — which is why the value on the wire is the same type at both ends.
- **`evaluate(&mut self, world: &World)`** hands a node its own three parts mutably and a `World` the graph is absent from. `MidiCc` and `MidiTime` already establish the pattern of reading a plugin-owned resource out of that `&World` during evaluation.
- **`World::get_non_send_resource::<R>(&self)` is bound by `R: 'static` alone** — no `Send`, no `Sync`. A non-send resource is therefore free to hold a `RefCell` and an `Rc`, and is reachable from the `&World` a node is handed. The tick is an exclusive system, so it runs on the thread the plugin inserted the resource from.
- **Dirty tracking compares reflectively.** `evaluate` snapshots state and outlets with `to_dynamic()` and compares with `reflect_partial_eq` in both directions; a type that answers `None` is reported changed on every tick (the `Placer`/glam regression).
- **A node is built from `ReflectDefault`, and only its inlets are then applied.** Both `Graph::create` and the v4 loader do exactly this, so whatever `Default` produces for `state` and `outlets` is what a node starts every session with.
- **The document RON-serializes the whole `inlets` struct** through `TypedReflectSerializer`. A field type with no `ReflectSerialize` fails the save of the node that declares it.
- **`sway-graph`'s manifest is a constraint, not a habit.** Its comment states what the engine must never pull in — the `bevy` facade, `bevy_render`, the document format, a channel, a UI toolkit — and `serde` left with the document format in M6-2.
- **`docs/architecture.md` §3 describes the pre-redesign `sway-events`** — `TriggerOut<P>` components, `Relationship` wires, per-wire `TriggerIn<W>` buffers. That design is superseded here; §3 and the `sway-events` rows in the ownership table and crate list need rewriting to describe this one.

## Goals / Non-Goals

**Goals:**

- A trigger connection that is an ordinary edge between ordinary fields, with no new legality rule, no new step kind in the plan, and no second evaluation path.
- A read path and a write path that are separated by construction: a consumer that holds occurrences has no operation that could add to them.
- One lifecycle system — empty the arena before the tick — and nothing else that has to run for occurrences to be correct.
- **Not one line of `sway-graph`.** The engine's public surface is treated as fixed; if something the design needs turns out not to be reachable, that is a finding to bring back, not a patch to the engine.

**Non-Goals:**

- Sub-tick timing on an occurrence. The payload may carry an offset if a domain wants one; nothing in `sway-events` reads it.
- Event-driven scheduling. Every node still evaluates every tick; occurrences change what a node computes, not whether it runs.
- Cross-tick queues, replay, coalescing, or a batch that outlives its tick.
- A payload-aware inspector view. A handle field falls through to the existing read-only control.

## Decisions

### D1: The batches live in an arena in the `World`; the wire carries a handle

`EventArena` is a resource holding this tick's batches. A producing node, during its own `evaluate`, reaches it through the `&World` it is already handed — the same way `MidiCc` reaches `MidiControls` — hands over its whole batch, gets an `EventHandle<P>` back, and writes that handle to its own outlet. Consumers receive the handle by ordinary propagation and read the batch back out of the arena.

Two operations, and nothing else reaches a batch:

```rust
impl EventArena {
    pub fn publish<P: 'static>(&self, occurrences: impl IntoIterator<Item = P>) -> EventHandle<P>;
    pub fn read<P: 'static>(&self, handle: EventHandle<P>) -> Option<EventBatch<P>>;
}

/// What `read` hands back: an owned share of the batch, `Deref<Target = [P]>`.
pub struct EventBatch<P>(Rc<Vec<P>>);
```

This is what separates the read and write paths **structurally**. A handle is a name, not a capability: `read` is the only thing it opens, so a consumer holding one has no operation that could add to the batch it received. Nothing relies on a node's good behaviour, and the "do not write into what you received" rule that a shared mutable buffer would have needed does not exist.

It also puts the producer's own bookkeeping at zero. The batch is the arena's, the handle is on the outlet where the propagate machinery can find it, and the node's `state` stays empty — `Emitter` has `state: ()`.

*Alternative rejected — a shared buffer behind `Arc`, handed out as a handle to both sides.* One buffer, no arena, no staleness. Rejected because every reader would alias a mutable buffer: the read/write split becomes a naming convention, `Reflect`'s `Send + Sync` forces a lock or `unsafe impl Sync` on the buffer, and a node holding two aliases of one buffer turns "read then write" into a deadlock or a soundness argument. Handles into an arena need neither.

*Alternative rejected — copying the batch into each consumer's inlet as a plain `Vec<P>`.* Perfectly safe, and read/write separation would be structural too. Rejected on the fan-out cost the wire is meant to avoid: every consumer gets a deep clone of every occurrence, every tick.

### D2: Batches are refcounted, so nothing is borrowed across a call

```rust
pub struct EventArena {
    generation: u64,
    slots: RefCell<Vec<Rc<dyn Any>>>,
}
```

A published batch is never modified again — the mutation `publish` performs is to the **slot table**, not to any batch. That is the only thing needing interior mutability, and `RefCell` covers it, because **no borrow of the table ever escapes a method**: `publish` borrows to append and drops it; `read` borrows to clone one `Rc` out and drops it, handing back an owned `EventBatch<P>`. A read can therefore never conflict with a later publish, and the `RefCell` has no reachable panic. There is no rule for node authors to remember and no `unsafe` in the crate.

Refcounting also makes a stale hand-out harmless: a consumer that keeps an `EventBatch` past the tick holds data the arena has already forgotten, rather than a dangling reference into a table that has since been cleared.

**`Rc`, not `Arc`.** The arena is a **non-send resource**, which is the honest description of it: `World::get_non_send_resource` is bound by `'static` alone, so the arena needs no `Send`/`Sync` and neither does a payload — `P: 'static` and nothing more. The tick is an exclusive system, the clear is a `NonSendMut` system Bevy therefore schedules on the main thread, and `evaluate` reads the arena inside that exclusive tick. Nothing here wants to be touched from another thread, so an atomic refcount would be paying for a guarantee no one uses. `EventBatch<P>` hides the `Rc`, so making the arena `Send` later is a one-line change behind an unchanged API.

*Alternative rejected — `read` returning a reference into the table (`Option<&[P]>`).* The signature reads better, but it hands out a borrow that a later `publish` must not invalidate, which needs an append-only structure with stable addresses — a dependency (`elsa::FrozenVec`) or hand-written `unsafe` with a provenance argument. An `Rc` clone per read buys the same zero-copy access to the payloads with neither.

*Alternative rejected — `read` returning `Ref<'_, [P]>`.* Zero-copy and dependency-free, but it holds the `RefCell` borrow across the caller's use of the batch, so a node that reads and then publishes panics. That is a real hazard on a stage instrument, traded away for one refcount bump.

*Alternative rejected — a `Send + Sync` resource with a `Mutex` or `RwLock`.* The access pattern is one producer then several readers, serially, on one thread; a lock would buy nothing and cost an atomic on every access.

### D3: A handle is `{ generation, slot }`, `Copy`, with an empty sentinel

```rust
pub struct EventHandle<P> { generation: u64, slot: u32, _p: PhantomData<fn() -> P> }
pub const EMPTY: EventHandle<P>;   // names no slot, never stale
```

- **`generation`** is the arena's, bumped on every clear (D5). It is what makes a stale handle *readable and empty* rather than a silent read of whatever now occupies that slot — see D4.
- **`slot`** indexes the arena's batches for the current generation.
- **`PhantomData<fn() -> P>`** rather than `PhantomData<P>`, so the handle is `Send + Sync` however `P` is spelled, and so a payload type needs no bounds it does not otherwise want.
- **`Copy`, `Eq`, `Hash`, `Debug`** — a handle is two integers; nothing about it needs to be expensive.
- **`Default` is `EMPTY`.** That is what makes a freshly created node, a freshly loaded node, and an unconnected inlet all correct with no linking step at spawn: the field starts as a handle that reads as no occurrences and never goes stale.
- **Reflection is `#[reflect(opaque)]`** with `Default`, `PartialEq`, `Debug`, `Serialize` and `Deserialize`. Opaque because a handle has no fields an editor should walk into; `PartialEq` because a type whose `reflect_partial_eq` answers `None` is reported changed on every tick.

### D4: Staleness is read, not prevented

A handle outlives its batch — the arena is emptied before the next tick while the handle sits on an outlet until its producer overwrites it. Rather than hunt those handles down, `read` compares generations: a handle from an earlier generation yields an empty slice.

This is what makes several specified behaviours fall out instead of needing machinery:

- A producer that stops publishing leaves a stale handle behind, and it reads as nothing.
- A trigger connection inside a **cycle** carries nothing: the cycle member holds the handle its partner published last tick, which is stale. Deterministic, and it needs no cycle-awareness anywhere in this crate.
- A handle that survives a save (it does not — D8 — but if one did) cannot address a live batch in another session.

The cost is that a handle is not self-validating at the type level; the compensation is that reading one is always safe and always cheap.

### D5: Emptying is dropping everything and bumping the generation

`clear_event_arena(NonSendMut<EventArena>)` runs in `FixedUpdate` in its own set, `EventClearSet`, ordered `.before(sway_graph::GraphTickSet)` and added by `EventsPlugin` — the crate's single plugin, per the spec's "one crate with one plugin".

The whole system is: drop every batch, bump the generation. It is O(batches), not O(nodes); it does not read the graph, does not touch the type registry, and needs no per-kind index of which fields are handles. **This is the largest simplification the arena buys**: the earlier shared-buffer design had to walk every node's parts reflectively every tick to find buffers to clear.

Clearing is **not** gated on asset loading, even though the tick is. If a producer outside the graph ever publishes a batch while the project is still loading, an ungated clear is what keeps the arena bounded; and with no tick running there is nothing to starve.

An external producer — a MIDI drain, say — can publish a batch before the tick and park the handle in a resource of its own; a node then reads that resource during `evaluate` and puts the handle on its outlet. That is how the follow-up MIDI change is expected to work, and it needs nothing new here: `publish` already takes `&self`.

### D6: A batch is published whole, and read as a whole

`publish` takes the batch at once and returns its handle, which is the natural shape for a producer: build the occurrences, hand them over, put the handle on the outlet. `read` hands back an `EventBatch<P>` that derefs to `[P]`, so a consumer iterates the payloads without copying any of them.

`read` answers `None` for the empty handle, for a stale handle (D4), and for a slot whose payload type does not match — three ways of saying "no occurrences", which is what the spec asks a consumer to treat them as. A node's read path is therefore one `if let`, with nothing borrowed afterwards and nothing to remember about ordering its reads against its writes.

### D7: Publishing dirties the producer, and this is stated rather than hidden

A handle names one tick's batch, so a publishing node writes a different outlet value every tick, is reported changed, and dirties every node its handle reaches. Only a silent tick (the empty handle, equal to the one already there) reports nothing.

This is inherent, not an oversight. The generation cannot be excluded from the handle's equality: if two handles from different ticks compared equal, the propagate step would skip the write as an equal value, the consumer would keep last tick's handle, and it would read *nothing* — the mechanism would silently stop working. Equality has to track the value's real meaning.

The one case that must never dirty is the empty handle replacing the empty handle, and that is hardened rather than left to node authors: `publish` folds an empty batch to `EMPTY` and allocates no slot. A producer that publishes unconditionally therefore reports no change on a tick where nothing happened, without having to check first — the mistake it would otherwise make (a live handle to an empty `Vec`, new every tick) would dirty its whole downstream forever while carrying nothing.

What it costs: a projector keyed on the dirty set re-runs for a node that emits every tick. What it does not cost: anything for a node that is silent, which is the common case for most of a scene most of the time.

### D8: A handle serializes as the empty handle

`Serialize` writes the empty handle regardless of what stands in the field; `Deserialize` yields the empty handle. `ReflectSerialize`/`ReflectDeserialize` are registered so `TypedReflectSerializer` finds them.

This is not cosmetic: the document serializes the whole `inlets` struct in one go, so without these impls a node kind with a handle inlet fails `SaveError::Inlets` and the node is lost on save. Writing the empty handle rather than the live one also keeps saves byte-stable — a document saved on two different ticks is identical — and there is nothing meaningful to restore, since the first tick's propagate re-establishes the inlet anyway.

### D9: Registration is one call per payload type

`register_event_handle::<P>(registry)` registers `EventHandle<P>` with `ReflectDefault`, `ReflectSerialize` and `ReflectDeserialize`; `RegisterEventHandle` is the `App` extension beside `sway-graph`'s `RegisterNodeKind`. The arena itself needs no registration of any kind — it never asks what a payload is, only that `Vec<P>` downcasts back out of the box it stored.

### D10: Fixtures and every test live in `sway-events`

`sway-graph::graph::testing` is off limits under the no-touch rule, and does not need to be touched: `sway-events` dev-depends on `sway-graph` with `features = ["test-support"]` for `trace_world`, `tick_once`, `read_field` and `set_field`, and declares its own fixtures:

- payload `Ping(u32)`;
- `Emitter` — publishes `count` occurrences per tick and puts the handle on its outlet (`count` is an ordinary `f32` inlet, so the rate is drivable; `state: ()`, which is the point);
- `Tally` — reads the handle on its inlet and adds the occurrence count into an `f32` outlet;
- `Relay` — reads its inlet's batch and publishes a batch of its own, which is the two-hop case and the worked example of D1's "forwarding publishes a new batch".

Tests run at the fixed delta like the existing golden traces. The document round-trip is tested here too, with `sway-document` as a dev-dependency, so `sway-document` is untouched as well. The dependency direction is fine: both crates depend on `sway-graph`, neither on the other outside tests.

### D11: `MidiNotes` is the first producer, and it selects nothing

`sway-midi` gains a `NoteEvent` payload and a `MidiNotes` node. `evaluate` reads `TickMidi` out of `&World` — the same pattern `MidiCc` uses for `MidiControls` — keeps the `NoteOn`/`NoteOff` messages, publishes them as one batch, and puts the handle on its outlet. Nothing new is scheduled: the drain already fills `TickMidi` with each message and its offset before `GraphTickSet`, and the node publishes during the tick rather than before it, so there is no ordering constraint against `EventClearSet` to get wrong. (A future MIDI *system* that published directly would need `.after(EventClearSet)`; a node cannot, because the tick is already after it.)

```rust
pub struct NoteEvent {
    pub channel: u8,   // protocol 0-15, as MidiMessage carries it
    pub note: u8,
    pub velocity: u8,  // release velocity on a note-off
    pub on: bool,
    pub offset: f32,   // seconds from the start of this tick
}
```

- **A struct with `on: bool`, not an enum.** Every field is meaningful for both kinds — note-off carries release velocity — so an enum would duplicate the payload to distinguish one flag.
- **The sub-tick offset rides along** because `TickMidi` already records it. Nothing in this change reads it; it is there so the converter nodes that need exact retrigger timing do not need a new payload.
- **No channel or note filter on the node.** `MidiCc` filters because it publishes *one* held value and has to pick which. A batch does not: publishing everything and letting a later node choose costs nothing, avoids an "omni" encoding in an `f32` inlet, and means one `MidiNotes` in a scene can feed every consumer. `MidiNotes` therefore has `inlets: ()` and `state: ()`.
- **The payload stays in `sway-midi`**, not in `sway-midi-core` and not in a vocabulary crate of its own, because no other domain is meant to name it. The boundary is crossed inside the MIDI domain by converter nodes (`OnNotePressed` and its kin) that fire the *generic* events other domains understand.

*Worth recording for that follow-up:* those converter nodes imply `sway-midi` naming a generic payload that is planned to live in `sway-base-nodes`, which would be a domain crate depending on another domain crate — something `architecture`: "Dependencies point from host to domain to engine" forbids, and whose own scenario says shared vocabulary belongs in a crate both depend on. That is a decision for the change that adds the converters, not this one; it is noted here so it is not discovered late.

## Risks / Trade-offs

- **[Risk] A host that adds `GraphPlugin` and forgets `EventsPlugin` gets no arena at all.** Every publish and read then finds no resource. → Mitigation: specified as "no arena is no occurrences" — nodes evaluate normally and publish empty handles rather than panicking; `sway-app` adds the plugin in this change; and a test asserts the plugin schedules the clear before `GraphTickSet`.
- **[Risk] Nothing in the app *reads* occurrences when this lands.** `MidiNotes` publishes on every tick in a real app, so the mechanism is reached rather than scaffolded, but the read path outside tests has no consumer until the converter nodes land. → Mitigation: accepted deliberately — the read side is covered by `sway-events`' fixtures, and a producer whose handle nothing consumes is still correct by construction (the batch is published, the arena forgets it at the next clear). The follow-up that adds `OnNotePressed` and the generic events closes it.
- **[Trade-off] Publishing dirties the producer and everything downstream of the handle** (D7). → Accepted and specified; the alternative breaks the mechanism.
- **[Trade-off] A stale handle is indistinguishable from a silent producer** — both read as no occurrences. → Accepted: a consumer has no legitimate use for the difference, and collapsing them is what makes cycles and dropped producers behave without special cases.
- **[Trade-off] A consumer can keep an `EventBatch` alive past its tick.** Nothing breaks — it holds a refcount on data the arena has forgotten — but a node that stashes one in `state` is reading last tick's occurrences on purpose, which is not what the mechanism means. → Accepted: it is ordinary node memory, indistinguishable from a node copying the payloads into its own state, and no requirement it could violate is stated in terms of what a node chooses to remember.
- **[Risk] The arena is main-thread-only.** Anything that ever wants to publish from a worker cannot. → Accepted: the tick is main-thread by construction, and the MIDI feed already lands its messages in a resource on the main thread before the tick.
- **[Trade-off] `publish` allocates an `Rc<Vec<P>>` per publishing node per tick.** At MIDI scale this is noise. → Accepted; if it ever shows up, the arena can keep its per-generation storage and reuse allocations across clears without any change to the handle or the spec.

## Migration Plan

None. Nothing exists to migrate: no document, node kind or API in the workspace publishes or reads occurrences today, the format version is untouched, and `sway-events` is a new crate rather than a revival of an existing one — the name has never had contents.
