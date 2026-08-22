## 1. The crate (`sway-events`)

- [ ] 1.1 Create `crates/sway-events` and add it to the workspace `members` and to `[workspace.dependencies]` as `sway-events = { path = "crates/sway-events" }`
- [ ] 1.2 Dependencies: `bevy_app`, `bevy_ecs`, `bevy_reflect`, `serde`, and `sway-graph` (for `GraphTickSet` only). Dev-dependencies: `sway-graph` with `features = ["test-support"]`, `bevy_time`, `sway-document`, `ron`. A manifest comment states what this crate is — the occurrence vocabulary between the engine and the domains — that it never reads the graph, and that it must never depend on a node domain
- [ ] 1.3 `lib.rs` module docs: the arena/handle split and why it separates the read and write paths structurally (D1), that a producer holds no state, and that a handle is valid for exactly the tick it was published in

## 2. The handle (`sway-events`)

- [ ] 2.1 `EventHandle<P> { generation: u64, slot: u32, _p: PhantomData<fn() -> P> }` — `Copy`, `Clone`, `PartialEq`, `Eq`, `Hash`, `Debug`; `PhantomData<fn() -> P>` so the handle is `Send + Sync` whatever `P` is (D3)
- [ ] 2.2 `EventHandle::EMPTY` naming no slot, and `Default` returning it — with the comment that this is what makes a created node, a loaded node, and an unconnected inlet all correct with no linking step at spawn
- [ ] 2.3 Derive `Reflect` with `#[reflect(opaque)]` and `#[reflect(Default, PartialEq, Debug, Serialize, Deserialize)]`; explicit `#[reflect(where ...)]` bounds so `P` needs only `TypePath + 'static`. The `PartialEq` comment records why the generation cannot be excluded from equality (D7): an equal-comparing handle would be skipped by propagate and the consumer would read nothing
- [ ] 2.4 `Serialize` always writes the empty handle; `Deserialize` yields it (D8)
- [ ] 2.5 Unit tests: `EMPTY` equals `EMPTY` and no live handle; handles from two generations are unequal; a handle round-trips through serde as `EMPTY`; the handle is `Send + Sync` for a payload that is neither

## 3. The arena (`sway-events`)

- [ ] 3.1 `EventArena`: a generation counter and `slots: RefCell<Vec<Rc<dyn Any>>>`, inserted as a **non-send** resource. Comment records why: a published batch is never modified again, so the only interior mutability is the slot table's, and no borrow of it escapes a method — hence no lock, no `unsafe`, and no reachable panic (D2). Plus `EventBatch<P>(Rc<Vec<P>>)` with `Deref<Target = [P]>`, which is also what makes a later move to `Arc` a one-line change
- [ ] 3.2 `publish<P: 'static>(&self, occurrences: impl IntoIterator<Item = P>) -> EventHandle<P>` — collects the batch into an `Rc<Vec<P>>`, appends it to the slot table under a borrow it drops, and returns the handle naming it. **An empty batch returns `EMPTY` and allocates no slot** (D7): this is what keeps a producer that publishes unconditionally from dirtying its downstream on a tick where nothing happened
- [ ] 3.3 `read<P: 'static>(&self, handle: EventHandle<P>) -> Option<EventBatch<P>>` — `None` for the empty handle, `None` for a handle from an earlier generation (D4), `None` for a slot whose payload type does not match, the batch otherwise. Clones the `Rc` out under a borrow it drops, so nothing is borrowed when it returns
- [ ] 3.4 `clear(&mut self)`: drop every batch and bump the generation
- [ ] 3.5 Tests: publish then read yields the batch in order; two reads of one handle yield the same; a handle read after a clear yields `None`; a handle read after a clear does **not** yield a batch published into the same slot in the new generation; `EMPTY` reads `None` in every generation; publishing an empty batch returns `EMPTY` and leaves the slot table untouched; reading a handle whose payload type differs from the batch's yields `None` rather than panicking
- [ ] 3.6 Test: reading a handle and then publishing a new batch in the same scope both succeed — a live `EventBatch` is an owned share, not a borrow of the arena (D2)

## 4. Registration and the plugin (`sway-events`)

- [ ] 4.1 `register_event_handle::<P>(&mut TypeRegistry)` registering `EventHandle<P>` with `ReflectDefault`, `ReflectSerialize` and `ReflectDeserialize`; plus a `RegisterEventHandle` `App` extension shaped like `sway-graph`'s `RegisterNodeKind`
- [ ] 4.2 `EventsPlugin` — the crate's one plugin: inserts the arena as a non-send resource and adds `clear_event_arena(NonSendMut<EventArena>)` to `FixedUpdate` in `EventClearSet`, `.before(sway_graph::GraphTickSet)`, deliberately not gated on asset loading
- [ ] 4.3 Export `EventArena`, `EventHandle`, `register_event_handle`, `RegisterEventHandle`, `EventClearSet` and `EventsPlugin`; export nothing a caller outside the crate does not need
- [ ] 4.4 Test: in an `App` with `GraphPlugin` + `EventsPlugin`, a batch published on one update is not readable on the next — the clear really does run before the tick

## 5. Fixtures and behaviour tests (`sway-events`)

- [ ] 5.1 Fixtures (test-only), over `sway-graph`'s `test-support` harness: payload `Ping(u32)`; `Emitter` (publishes `count` occurrences and puts the handle on its outlet; `count` an `f32` inlet; `state: ()`); `Tally` (reads its inlet handle into an `f32` outlet); `Relay` (reads its inlet's batch and publishes a batch of its own). A test registry registering them plus `EventHandle<Ping>`
- [ ] 5.2 Connect legality: handle to handle of the same payload is legal; to a different payload is refused at connect; `Option<EventHandle<Ping>>` is optional and `Vec<EventHandle<Ping>>` is variadic (`events`: An occurrence handle is a value a wire may carry)
- [ ] 5.3 Same-tick delivery: `Emitter` → `Relay` → `Tally` lands end to end in one tick; `Relay`'s outlet handle is not the handle it received
- [ ] 5.4 Fan-out: two `Tally` nodes on one emitter read the same batch; one reading does not change what the other reads
- [ ] 5.5 Producer discipline: an emitter with nothing to publish writes `EMPTY`; a producer's `state` holds neither the occurrences nor the handle; an emitter that publishes on one tick and not the next leaves its consumer reading nothing
- [ ] 5.6 Lifetime: nothing survives to the next tick; a stale handle reads empty and not another producer's batch; publishing every tick for many ticks leaves only the current tick's batches in the arena
- [ ] 5.7 Variadic merge in ordering-key order; an unconnected optional handle inlet reads absent; an unconnected plain handle inlet reads as no occurrences and evaluation still succeeds
- [ ] 5.8 Dirty: a silent producer and its consumers report no change across two ticks; a producer that publishes an *empty* batch every tick likewise reports no change, tick after tick; a publishing producer is reported changed and so is each node its handle reaches (`events`: Publishing a batch is a change to the producer's outlet)
- [ ] 5.9 Cycle: two nodes wired into each other by trigger connections both still evaluate and neither reads the other's occurrences
- [ ] 5.10 No arena: a graph of these fixtures ticks with no `EventsPlugin` added — evaluation succeeds and handle outlets are empty (`events`: No arena is no occurrences)

## 6. Document round-trip (`sway-events` tests, `sway-document` untouched)

- [ ] 6.1 A fixture node kind whose inlets hold an `EventHandle<Ping>` beside an ordinary authored field, saved and loaded through `sway-document`'s v4 save/load
- [ ] 6.2 Test: saving that node writes it like any other node with no diagnostic, and the entry names no batch or generation (`document`: A document stores inlets only)
- [ ] 6.3 Test: a node whose handle inlet names a live batch saves and reloads with its other inlets restored and an empty handle
- [ ] 6.4 Test: saving that document on two different ticks produces the same bytes

## 7. MIDI note events (`sway-midi`)

- [ ] 7.1 Add `sway-events` to `sway-midi`'s dependencies
- [ ] 7.2 `NoteEvent { channel: u8, note: u8, velocity: u8, on: bool, offset: f32 }` — `Reflect`, `Clone`, `Copy`, `Debug`, `PartialEq`; doc comments record protocol 0–15 channel numbering, release velocity on a note-off, and the offset being seconds from the start of the tick (D11)
- [ ] 7.3 `nodes/midi_notes.rs`: `MidiNotes` with `inlets: ()`, `state: ()`, `outlets: MidiNotesOut { notes: EventHandle<NoteEvent> }`. `evaluate` reads `TickMidi` from `&World`, keeps `NoteOn`/`NoteOff` in arrival order, publishes the batch, and writes the handle — falling back to `EMPTY` when the arena or `TickMidi` is missing. The comment records why there is no channel or note filter here (D11)
- [ ] 7.4 Export from `nodes/mod.rs` and `lib.rs`; `MidiPlugin` registers `MidiNotes`, `MidiNotesOut`, `NoteEvent` and `EventHandle<NoteEvent>` — so a host that already adds `MidiPlugin` needs nothing else (`midi`: MIDI note events live in the MIDI domain)
- [ ] 7.5 Node tests over a seeded `TickMidi`: a note-on then a note-off publish two occurrences in that order with their offsets; a zero-velocity note-on publishes as a note-off; two channels both publish; a tick with no note messages leaves `EMPTY`; a tick with only Control/Clock messages leaves `EMPTY`; no arena and no `TickMidi` each leave `EMPTY` rather than failing
- [ ] 7.6 Plugin test through the real path: notes pushed into `MidiInbox`, `app.update()`, then read the node's outlet handle through the arena and assert the batch; a second update with no input leaves `EMPTY` and the first tick's notes are not republished (`midi`: Notes do not survive their tick)
- [ ] 7.7 `cargo test -p sway-midi`

## 8. Host wiring (`sway-app`)

- [ ] 8.1 Add `sway-events` to `sway-app`'s dependencies and `sway_events::EventsPlugin` beside `sway_graph::GraphPlugin` in `main.rs`
- [ ] 8.2 Confirm `sway-app` still builds and starts with no project loaded (the clear runs from startup; it is not gated on assets), and that a `MidiNotes` node can be created from the palette and ticks without MIDI hardware present

## 9. Docs and verify

- [ ] 9.1 Rewrite `docs/architecture.md` §3 for this design: an arena of per-tick batches owned by `sway-events`, handles copied along ordinary edges, a producer publishing each tick and holding no state, a consumer reading by handle, the arena emptied before the tick, a stale handle reading empty, a trigger connection in a cycle carrying nothing, and `MidiNotes` as the first producer. Delete the `TriggerOut<P>` / `Relationship` / `TriggerIn<W>` description
- [ ] 9.2 In the same file: the ownership-table row `| Event-wire buffers + pre-tick clear/copy |` becomes the arena and its pre-tick clear owned by `sway-events`; the supporting-crates line for `sway-events` describes this design and the `sway-midi` line lists `MidiNotes` beside `MidiTime` and `MidiCc`; the `FixedUpdate` schedule block becomes `sway-events: clear the arena → rebuild if dirty → graph tick`; fix the §11 testing bullet that lists "clear/copy/clear-out invariants"
- [ ] 9.3 Record the follow-up in the roadmap: the converter nodes that read note occurrences and fire the generic events other domains understand (`OnNotePressed` and its kin), including the crate-dependency question D11 leaves open about where that generic payload lives
- [ ] 9.4 **Verify `sway-graph` is untouched**: `git diff --stat -- crates/sway-graph` is empty (`events`: The engine names no occurrence)
- [ ] 9.5 `cargo test -p sway-events -p sway-midi`, then `cargo test -p sway-graph -p sway-document` to confirm nothing regressed, then `cargo fmt --all` and `cargo clippy --workspace --all-targets`
