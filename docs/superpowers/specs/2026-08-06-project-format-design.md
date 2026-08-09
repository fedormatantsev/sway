# M4: project format and hot reload — Design

**Date:** 2026-08-06
**Status:** Approved, pre-implementation
**Parent spec:** `2026-07-25-sway-design.md` §5 (M4), §2.11, §4
**Prior:** `2026-08-05-wires-design.md` — the model this format serializes
**Placement:** M4, immediately after the wire migration; precedes M5's node set

## 1. What this is

A set is currently authored by editing `demo_graph.rs` and recompiling. M4 makes
it a text file the running app reloads: **a document of entities, components and
wires**, loaded through `bevy_asset`, reconciled into the live world on change.

This is what makes authoring possible long before the editor exists, and it is
why the editor can wait. It is also the first thing that forces a decision the
wire model deferred: what an authorable component *is*, and what identifies an
entity across two versions of a document.

Three commitments frame it:

1. **The document is the source of truth for authored state**, and nothing
   else. What is not in it is either derived (`Children`, `GlobalTransform`) or
   runtime-owned (a `Handle<Mesh>` a system attached).
2. **A reload never stops the show.** A syntax error keeps the running world; a
   semantic error costs one item, not the document.
3. **Nothing new touches the tick path.** Reflection stays where the wire spec
   put it: the editor, and this.

## 2. The format

### 2.1 Shape

One RON document per project. Valid RON, comment-bearing, one component and one
wire per line:

```ron
// sway project — the wire slice
Project(
    version: 1,
    entities: [
        Entity(
            id: "lfoA",
            components: {
                // the modulator: slow, half amplitude
                "Lfo": (beats: 8.0, shape: Sine, phase: 0.0, amplitude: 0.5),
                "FloatOut": (0.0),
                "EditorPos": ((-320.0, 40.0)),
            },
            wires: {},
        ),
        Entity(
            id: "lfoB",
            components: {
                "Lfo": (beats: 2.0, shape: Sine, phase: 0.0, amplitude: 0.0),
                "FloatOut": (0.0),
                "EditorPos": ((-120.0, 40.0)),
            },
            wires: { "amplitude": "lfoA" },
        ),
        Entity(
            id: "cubeB",
            components: {
                "Transform": (translation: (0.8, 0.0, 0.0)),
                "DemoCube": (),
            },
            wires: { "translation.y": "lfoB", "parent": "group" },
        ),
    ],
)
```

**Note against the sketch this was approved from:** that sketch used a
top-level repeated `entity "id" (...)` form, which is not legal RON — a RON
document is a single value. The shape above is the closest legal form and keeps
every property that mattered: comments survive, ids are strings, component and
wire keys are short registered names, and each item is its own line.

Three keys, three registries behind them:

| Key | Resolved against | Failure |
|---|---|---|
| `id` | the document itself, and `DocId` in the world | `DuplicateId` |
| a component name | `ComponentDocRegistry` | `UnknownComponent`, `BadPayload` |
| a wire name | `WireRegistry`'s `NAME` | `UnknownWire`, `UnresolvedTarget` |

### 2.2 One item per line is a format constraint

It is what lets M7's writer replace a single line in place — locate
`"Lfo":` inside `id: "lfoA"`, swap the payload, leave every byte around it
alone — rather than re-emitting the document and destroying comments and
ordering. M4 does not build that writer, but the format must not foreclose it,
so the emitter (§5) writes exactly this layout and a test pins it.

### 2.3 Why not `DynamicScene`

`bevy_scene` already serializes entities, components and entity-referencing
relationship components, with remapping — most of §4 for free. It is not used:
its files are keyed by full reflect `TypePath`
(`"sway_nodes::osc::Lfo"`), its output is verbose, and it round-trips by
rewriting the file wholesale. The format is human- and machine-authored, and
that outranks the code saved. The parts of `bevy_reflect` underneath it — the
type registry, `ReflectComponent`, the typed (de)serializers — are used
directly.

### 2.4 Identity

**An entity's `id` is its identity across reloads and its `Name` in the world.**
The loader inserts `DocId(String)` and `Name` from it; neither is authorable as
a component. Renaming an entity is therefore a delete plus an add, which is
honest: nothing else in the document identifies it, and every wire naming it has
to change anyway.

## 3. What becomes authorable

```rust
pub fn register_authorable<C>(app: &mut App, name: &'static str)
where
    C: Component + Reflect + FromReflect + TypePath + Default;
```

It calls `app.register_type::<C>()` and records `name ↔ TypeId` in a
`ComponentDocRegistry` resource. A duplicate name panics **at startup** — loud,
and before a show rather than during one.

Two requirements on `C`, both consequences of how reflect deserialization
works rather than choices:

- **`Default`, registered as `ReflectDefault`.** A document payload is a partial
  struct — `(translation: (0.8, 0.0, 0.0))` names one of `Transform`'s three
  fields — and reconstructing a whole component from a partial one needs a
  fallback for the rest. This is the load-bearing assumption of the whole
  milestone and §7 makes it the first thing verified.
- **`PartialEq` via reflect**, so the applier can compare before writing and
  leave `Changed<T>` alone for an unchanged value. That is the same discipline
  §2.11 of the parent spec imposes on wires, applied to the loader.

Wire registration gains three functions, all free from the existing generics:

```rust
insert: fn(&mut World, dst: Entity, src: Entity),   // W::from(src)
remove: fn(&mut World, dst: Entity),
read:   fn(&World, dst: Entity) -> Option<Entity>,  // Relationship::get
```

`ChildOf` comes along with everything else: its `NAME` is `"parent"`, and
parenting is a wire like any other in the document.

Components deliberately *not* authorable in M4: `Mesh3d`, `MeshMaterial3d`,
`Handle<T>` of any kind, `Visibility`, `GlobalTransform`, `Children`. The first
three are asset flow, which is M5's; the last three are derived.

## 4. Loading, and applying to the world

```
AssetLoader   text → ProjectDoc            pure, no World, all syntax errors here
AssetEvent    LoadedWithDependencies|Modified → apply
apply         reconcile by DocId, in four passes, then set TopologyDirty
```

`ProjectDoc` is plain data with no Bevy in it: `Vec<EntityDoc>`, each with an
id, a component map of `ron::Value` payloads, and a wire map of id strings. It
parses, validates ids for duplicates, and knows nothing about the registries —
which is what makes the parser testable without a world.

### 4.1 The four passes

1. **Index and despawn.** Build `HashMap<String, Entity>` from
   `Query<(Entity, &DocId)>`. Despawn every entity whose id is absent from the
   new document; Bevy's despawn takes its children and every wire pointing out
   of it.
2. **Spawn.** For each id not in the world, spawn with `DocId` and `Name`. Both
   passes complete before any wire is resolved, so a wire may name an entity
   declared later in the file.
3. **Components.** For each authorable component named in the document: build it
   through reflect, compare with what is there, and insert or apply only on a
   difference. For each *registered authorable* component present in the world
   but absent from the document: remove it. Unregistered components are never
   touched — that is the rule that keeps a runtime-attached `Mesh3d` alive
   across a reload.
4. **Wires.** For each registered wire type, compare the document's target
   against `read`'s answer and `insert`, `remove` or leave it. Never remove and
   re-insert an unchanged wire: that would churn `RelationshipTarget`
   collections and dirty the topology for nothing.

Then set `TopologyDirty`, and the existing rebuild produces the new order on the
next `FixedUpdate` (parent §2.5). The applier itself never touches `GraphOrder`.

### 4.2 What a reload preserves

An entity present in both documents keeps its `Entity` id, and therefore its
editor identity, its `Children`, and every component the document does not
mention. That is what makes hot reload authoring rather than restarting.

The wire model made this cheap: behaviours derive from absolute beat time rather
than accumulating, so there is almost no per-instance state left for a reconcile
to protect. The value is in what the *runtime* attached, not what the graph
accumulated.

### 4.3 Failure

Split, deliberately:

| Failure | Effect |
|---|---|
| RON syntax error | The reload is rejected whole. The running world is untouched. |
| Duplicate id | Same — the document is not coherent enough to apply. |
| Unknown component or wire name | That item is skipped; everything else applies. |
| Payload that will not deserialize | Same. |
| Wire naming an id that does not exist | Same. |

A bad keystroke mid-set must not empty the scene, which is why the first two
reject rather than partially apply. The rest are per-item because a half-typed
document is the normal state of a file being edited, and refusing to apply the
other forty entities would make live authoring unusable.

Everything lands in a `ProjectDiagnostics` resource — the same shape as
`GraphDiagnostics`. A syntax error is surfaced through
`AssetLoadFailedEvent`, so a rejected reload is visible rather than silent.
Rendering either diagnostics resource beside the other is M7's: neither
`ProjectDiagnostics` nor `GraphDiagnostics` has a widget yet, and M4 only
fills the resource.

## 5. Writing back

M4 builds `to_document(&World) -> ProjectDoc` plus a RON emitter, for one
purpose: **proving the format expresses everything the world holds.** A
world → document → world round-trip test is the completeness check that no
amount of reading proves.

It walks entities carrying `DocId`, serializes each registered authorable
component it finds through `TypedReflectSerializer`, and reads each registered
wire through `read`. Entities without `DocId` — the camera, the light, anything
a runtime system spawned — are not in the document and never become part of it.

**The in-place, comment-preserving writer is M7's**, along with everything that
needs it: dragged `EditorPos`, inspector edits, node creation. §2.2's one-item-
per-line rule is what M4 owes it.

## 6. The inspector

The read-only inspector M2 asked for, deferred twice, lands here — and it is
now the same walk as the format's: for the selected entity, iterate registered
authorable components, and for each, iterate its reflected fields into
name/value rows. `f32`, `Vec2`, `Vec3`, `bool`, `String` and enums render
directly; anything else prints its debug form and is a note that the type needs
`TypeData`.

That is the point of building it: **editor `TypeData` has been unexercised for
three milestones**, and an inspector is the cheapest thing that discovers what
is missing. It is read-only; editing is M7.

## 7. Testing

The order matters — the first item is a gate, in the shape the wires plan used.

1. **Reflect through `ron::Value`, pinned first.** A characterization test that
   a partial payload with an enum field deserializes into a whole component via
   `ReflectDefault`, and that a `ron::Value` drives `TypedReflectDeserializer` at
   all. If either fails, the format's payload strategy changes and the rest of
   the plan changes with it. **Write it before anything else.**
2. **Parser tests**, table-driven, with no `World`: shapes, comments, duplicate
   ids, malformed payloads, unknown keys.
3. **Reconcile tests.** A surviving entity keeps its `Entity` and its
   runtime-attached components; a removed one takes its children and wires; an
   added one appears; an unchanged component leaves `Changed<T>` false; an
   unchanged wire is not churned.
4. **Failure tests.** A syntax error leaves the world exactly as it was and
   records a parse diagnostic. Each per-item failure applies the rest.
5. **Round-trip.** World → document → world reproduces every authorable
   component and wire, and the emitted text is one item per line.
6. **End-to-end.** The demo document loads into the same world the current
   `demo_graph.rs` builds, and the modulated-LFO chain still resolves in one
   tick.

## 8. The demo, and the one thing it cannot express

`assets/demo.sway.ron` replaces `demo_graph.rs`. It expresses the two LFOs,
their `FloatOut`s, the group, the two cubes, their transforms and all four
wires.

It cannot express the cubes' mesh and material, because `Handle<T>` is asset
flow and asset flow is M5. The document authors a `DemoCube` marker component
instead, and a plain Bevy system on `Added<DemoCube>` attaches `Mesh3d` and
`MeshMaterial3d`. This is deliberately the ugly seam: it is exactly the shape of
what M5 replaces, and naming it here keeps M4 from inventing half an asset
system on the way past.

Hot reload also needs `bevy`'s `file_watcher` feature and
`AssetPlugin::watch_for_changes_override`; neither is on today.

## 9. Scope

**In:** the document type and parser, the two registries' additions,
`DocId`/`Name` identity, the four-pass applier, `ProjectDiagnostics`, the
whole-document emitter, the read-only inspector, the demo document, and the
`file_watcher` wiring.

**Out:** the in-place writer (M7), inspector editing (M7), a palette or any
creation UI (M7), asset flow (M5), resources and transport settings in the
document, sub-documents or includes, migrations between `version`s beyond
rejecting an unknown one, and any component not on §3's list.

## 10. Risks

**Reflect's partial-struct deserialization is assumed, not verified.** §7's
first test exists for this. If `ReflectDefault` fallback does not work the way
this design reads it, the fallback is requiring complete payloads, which makes
documents verbose but changes nothing structural.

**`ron::Value` as a serde `Deserializer` is assumed.** Same test. If it does not
hold, the parser keeps each payload as a raw string and runs
`ron::Deserializer::from_str` on it, which also gets the byte spans M7 will
want — so the fallback is arguably better and is only avoided for being more
code.

**Reconcile granularity may be too coarse.** Passes 3 and 4 compare per
component and per wire, not per field. A document that changes one field of
`Transform` rewrites the whole `Transform`, which is correct but marks
`Changed<Transform>` for everything downstream. Acceptable at authoring cadence;
noted in case it ever matters.
