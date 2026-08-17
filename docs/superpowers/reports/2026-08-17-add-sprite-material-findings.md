# add-sprite-material — sprite layers with depth: findings

**Date:** 2026-08-17
**Verdict:** GO — every exit criterion passed, including the one the design said to stop on
**Change:** [`openspec/changes/add-sprite-material`](../../../openspec/changes/add-sprite-material/)
**Spec:** decision D3 in [`2026-08-09-mvp-roadmap-design.md`](../specs/2026-08-09-mvp-roadmap-design.md)
**Predecessor:** [`2026-08-10-sprite-depth-spike-findings.md`](2026-08-10-sprite-depth-spike-findings.md) (verdict GO)

## Question

Can a sprite layer carry a depth channel that parallaxes when the quad is
rotated against the camera, and interpenetrate both opaque meshes and other
sprite layers — with the animation driven from the node graph rather than a
bespoke component?

## Answer

Yes, on all four counts.

The exit criterion the whole design turned on is D1's claim that displacing
*vertices* along the mesh normals — rather than writing `@builtin(frag_depth)`
as the M8 spike did — produces real parallax. The design said in as many words:
"if it does not appear, stop." It appeared. Near relief shifts further across
the image than far relief when the quad rotates, which is exactly what the
spike's approach could not have produced, since a fragment rasterized from a
flat quad keeps its screen position by construction.

Automated: `cargo test --workspace --exclude sway-midi-core` at `fba6379`:
**434 passed, 0 failed**. `sway-midi-core` is excluded because
`virtual_destination_receives_midisend_note_on` times out waiting on CoreMIDI
on this machine — verified pre-existing by stashing to a clean tree, and
unrelated to anything here.

## What was built

Two commits on `add-sprite-material`:

- `de82df4` — `feat(nodes,runtime,app)`: `PlaneMesh` in `sway-nodes`;
  `FrameSequence`, `SpriteMaterial`, `sprite_material.wgsl` and six wires in
  `sway-runtime`; demo scene and 60 generated frames in `sway-app`.
- `fba6379` — `fix(graph,editor)`: two pre-existing inspector defects this work
  surfaced (see "Surprises").

Three nodes, wired on the canvas, contradicting the roadmap's `SpriteLayer` /
`SpriteAnim` component pair. Geometry is a mesh node, the material is a material
node, and the frame counter is `MidiTime → Oscillator(Saw) → Remap → frame` —
three nodes that already existed.

## Measured

### Sequence memory (task 4.7)

The demo's two sequences, 30 frames each:

| Run | Resolution | Format | On disk | In VRAM |
|---|---|---|---|---|
| Colour (`pulse_color`) | 256×256 | RGBA8 | 944 KiB | 7.50 MiB |
| Depth (`pulse_depth`) | 64×64 | 8-bit greyscale | 34.6 KiB | 0.47 MiB |
| **Total** | | | **~0.96 MiB** | **~7.97 MiB** |

Bevy expands both to 4 bytes/pixel regardless of source channel count, so the
greyscale depth PNG costs the same per texel as colour would. **The depth run
was authored at 64×64, not at the colour run's resolution** — that decoupling is
the single largest lever on sequence memory, and it costs nothing: at the
default 63 subdivisions a 64×64 depth frame is Nyquist-matched to the vertex
grid that consumes it. Authoring depth at the colour run's 256×256 would have
cost 7.50 MiB instead of 0.47 MiB — a 16× increase — to feed a vertex grid that
cannot resolve the extra detail.

For sizing: a 512² RGBA colour frame is 1 MiB. A 256-frame pair with a 64²
depth run is ~260 MB; a 1024-frame pair approaches 1 GB. Past roughly a
thousand frames this is video, which the roadmap puts out of MVP.

### Subdivision sweep (task 4.4)

Swept 0, 31, 63, 255 by eye. **The relief stops improving visibly at 63.** 31
was visibly coarser; 255 was indistinguishable from 63. The design's default of
63 is correct as chosen and needs no revision.

This is consistent with the design's reasoning that density is bounded by the
frequency of the depth sheet rather than by screen pixels — the sprite's visible
outline comes from the alpha channel, which is per-fragment and already exact,
so the vertex grid only has to resolve the relief's shape. At 63×63 that is ~8k
triangles per layer; the cost is flat across the useful range, so this was
always a knob to turn by eye rather than a budget to compute.

### 8-bit depth quantization (task 4.6)

**No banding, in the relief or at the interpenetration seam.** 8-bit holds up
for this content.

This confirms the design's expectation that quantization would smooth rather
than terrace: the depth value is interpolated to float by the sampler before it
displaces a vertex, so the 256 steps are smoothed by the same linear filtering
that array layers make unconditionally safe.

**Trigger for the upgrade path**, documented here so the next author does not
have to re-derive it: the KTX2 `Rgba16Float` route (design D8) becomes worth
paying for when a depth run holds a *slow gradient across a large depth range* —
that is where 256 steps spread thinnest. The demo's content has neither. Two
traps on the way: 16-bit greyscale PNG is not the answer, because Bevy maps it
to `R16Uint`, which is not filterable at all; and `R16Unorm` / `Rgba16Unorm`
need the non-default wgpu feature `TEXTURE_FORMAT_16BIT_NORM`.

### Skirt artefact (task 4.8)

**Not visible.** The design predicted that stretched triangles at depth
discontinuities would usually coincide with the alpha edge and be discarded
anyway, leaving the mitigation authorial — keep depth continuous where alpha is
continuous. That prediction held, though it held against content generated
specifically to satisfy it: the demo's depth field is a closed-form smooth
function, eased toward the pivot over the last ~0.12 radius units before the
silhouette. **This is evidence that the authorial mitigation works, not evidence
that the artefact is hard to provoke.** Hand-authored content with a genuine
interior depth cliff has not been tried.

### Frame-to-frame bleeding (task 4.5)

**None** — no colour ghosting, and no vertex twitching toward the next frame's
shape. D7 claimed array layers make this structurally impossible rather than
merely unlikely, and this check confirms the claim rather than assuming it. It
matters more here than in an ordinary atlas: under D1 the depth run is sampled
in the *vertex* stage, so bleeding would pull a neighbouring frame's height into
the geometry, not merely ghost its colour.

### Animation from the graph (task 4.1)

The sequence animates from transport. Swapping the oscillator's `Saw` for
`Triangle` turns the loop into ping-pong **with no material edit** — D3/D4's
central claim, that animation policy belongs in the node network where it is
visible and interchangeable, rather than as a `loop`/`ping-pong`/`hold` field on
a component. Scrubbing the frame value past the layer count holds on the last
layer rather than sampling out of range, which is the read-side clamp working as
the safeguard it was specified to be.

### Interpenetration (task 4.3)

Both cases confirmed: sprite-vs-mesh, and sprite-vs-sprite — the latter being
the case the M8 spike explicitly did not test. The spike's one surviving
contribution, the `specialize` flip of `depth_write_enabled`, is what makes this
work.

## Surprises

**The inspector could not edit two of the new node's three fields, and neither
failure was in the new code.** `PlaneMesh` is the first authorable component
with a `u32` field and the first with a `Vec2` field, and both types were
silently broken in the editor's reflected write path — the edit looked accepted,
the component never changed, and the inspector snapped back on the next refresh.

- Integers: the inspector has one integer commit path (parse as `i64`, send
  `FieldValue::Int`) and boxed that `i64` whatever the field's real width was.
  Reflection matches on the concrete type, so applying it to a `u32` field
  failed — and `apply_editor_command` discards the `try_apply` error, with a
  comment explaining that nothing could reach it. Something could. The existing
  test fixture's only integer field was `i64`, the single width where the boxed
  type happens to match, which is why this survived to now.
- `Vec2`: `kind_of` had an arm for `Vec3` but none for `Vec2`, so it fell through
  to `Opaque`, which renders read-only. `format_value` already knew how to
  display a `Vec2`, so the field looked editable without being so.

Both are fixed in `fba6379`, with saturating rather than wrapping integer
narrowing (`-1i64 as u32` would wrap to `u32::MAX` — for a subdivision count,
a four-billion-segment mesh). Fixing the second also tightened the `Vec3` parse,
which had been discarding unparseable components before counting them, so
`"1, abc, 2, 3"` committed as `(1, 2, 3)`.

The general lesson: **a silent no-op in a reflected write path is invisible until
a new type reaches it**, and the `let _ = ...` that discarded the error was
justified by an argument that was true when written and quietly stopped being
true.

**The asset and shader sign conventions disagree.** The generated depth frames
follow the M8 spike's rule (0.0 = nearest the camera), while the shader
implements standard displacement-map convention (brighter = out along the
normal). The demo reconciles them with a negative `depth_range` on both layers.
This works and is documented at length in `demo.sway.ron`, but it is a trap: the
next author generating depth content has no way to know which convention to
target without reading the demo document. See "Not answered".

**`ColorRunFrom` / `DepthRunFrom` carry no field copy.** `field_wire!` copies
outlet tuple field 0, and `FrameSequenceOut` is a named-field struct. Rather
than mirror the layer count onto the material, the run wires are pure topology
(the same shape as `sway_graph`'s own `impl Wire for ChildOf`) and the sync
system reads through the relationship. This keeps the count derived per D7 — a
copied count would go stale for exactly one tick after a sequence finishes
loading, and that tick is an out-of-range array sample in the vertex stage.

## What is not answered

- **Blend order between interleaved layers.** A stated non-goal. Depth testing
  occludes correctly, but `Transparent3d` sorts per-entity by centre distance,
  so blend order between interpenetrating layers can still be wrong. Inherent to
  depth-writing transparency. Constrains how large `depth_range` can be relative
  to layer spacing.
- **Two material wires on one mesh.** An entity carrying both `MaterialFrom` and
  `SpriteMaterialFrom` gets two `MeshMaterial3d<M>` components and draws twice —
  the exact failure D5 exists to remove. The relationship hooks give *sequential*
  exclusivity only. Accepted and recorded in the change's design.md; closing it
  needs a shared material-consumer marker whose hook evicts the previous kind.
- **The sign-convention disagreement above.** Either regenerate the assets to
  the shader's convention, or state the convention somewhere an asset author
  will find it. Left as-is deliberately rather than churn 60 PNGs during a
  by-eye pass.
- **No automated coverage of the sprite render path.** `sprite_material.wgsl` is
  in `PREPROCESSOR_SHADERS`, so naga never validates it, and `demo_renders` only
  asserts the cubes draw. A throwaway GPU smoke test written during
  implementation did confirm on a real device that the shader compiles, the
  `texture_2d_array` bindings match, the uniform layout agrees field-for-field,
  and displacement moved the quad's on-screen span from 38px to 46px — exactly
  the predicted +1 world-unit shift — but it was deleted rather than kept.
  Restoring it would be the cheapest guard against a silent shader regression.
- **Skirt artefact against adversarial content** — see above. Only tested
  against content authored to avoid it.
- **Performance at real layer counts.** Two layers says little. Five layers at
  63×63 is ~41k triangles by the design's arithmetic, but that was computed, not
  measured.
- **Folder enumeration does not survive asset packing.** `load_folder` is a
  filesystem-asset-source capability. A show build that packs its assets needs a
  different enumeration; out of scope here, and the reason D7 writes down the
  alternatives it rejected rather than discarding them.
- **`EnvironmentMap`** — M8's other half, untouched.

## Updated documents

- `openspec/changes/add-sprite-material/tasks.md` — all 35 tasks complete.
- `openspec/changes/add-sprite-material/design.md` — Risks section gained the
  two-material-kinds gap.
