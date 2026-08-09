# M8 spike — per-pixel sprite depth: findings

**Date:** 2026-08-10
**Verdict:** GO
**Plan:** [`2026-08-10-m8-sprite-depth-spike.md`](../plans/2026-08-10-m8-sprite-depth-spike.md)
**Spec:** decision D3 in [`2026-08-09-mvp-roadmap-design.md`](../specs/2026-08-09-mvp-roadmap-design.md)

## Question

Can an alpha-blended sprite quad write per-pixel depth from a depth channel,
so it interpenetrates opaque meshes and other sprite layers, rather than
sitting wholly in front of or behind them?

## Answer

Yes. The readback test measures the exact pixels the technique lives or dies
on and both come back correct — the near half of the sprite wins the depth
test against the cube, the far half loses it — on the first shader attempt,
with no sign-convention fix needed. The by-eye demo confirms the same
asymmetry from an off-axis camera, where a flat alpha mask and true
per-pixel depth would look different: a crisp, single-pixel-wide transition
from blended-red-over-cube to clean cube green, sitting at the sprite's own
midline and cut cleanly by the cube's silhouette.

## What was built

- `SpriteDepthMaterial` in `crates/sway-runtime/src/sprite_depth_spike.rs`
- `crates/sway-runtime/assets/shaders/sprite_depth_spike.wgsl`
- `crates/sway-runtime/tests/sprite_depth_interpenetration.rs`
- `--demo sprite-depth` in `sway-app`

## Measured

- Integration test: PASS
- Converged after 5 `app.update()` calls (cold cache: 5)
- Near-half pixel: [238, 79, 13, 255] (sprite-red wins). Far-half pixel:
  [62, 193, 45, 255] (cube-green wins).
- By-eye check: ran `cargo run -p sway-app -- --demo sprite-depth --windowed`
  on this machine (macOS, real GUI window) and screenshotted it
  (`screencapture -x`) a few seconds after launch, at steady ~60 fps with no
  errors in the run log. The result matched the brief's prediction exactly:
  a green cube with a red rectangle through it. The left (near) half sits in
  front of the cube — visible as the cube's front face blended to orange
  (red tint at alpha 0.85 over the cube's green) rather than pure green. The
  right (far) half is hidden wherever the cube covers it (pure cube green,
  no red at all) and still visible wherever the cube doesn't (a clean red
  border/overhang on the top, bottom, and right edges of the sprite's
  silhouette). The orange/green boundary sits at the sprite's own vertical
  midline, not at some arbitrary screen position, and the cube's silhouette
  cuts the far half's visibility cleanly along the cube's real edges — no
  bleeding, no dithering. A pixel-level scan across the boundary (via a
  cropped screenshot) found the transition from orange (220, 92, 43) to
  green (102, 190, 71) is a single pixel wide with no intermediate/blended
  values, i.e. no visible depth-seam fringing at this zoom level.
  One unrelated object was also visible in the window: a small
  light-blue-gray cube drifting near the scene. This is `DemoCube` from
  `crates/sway-app/assets/demo.sway.ron` — the project document's own wire
  demo — which renders because `load_project` and the graph/wire plugins in
  `sway-app/src/main.rs`'s `build_app` closure are added unconditionally,
  regardless of which `--demo` is selected. It is pre-existing `sway-app`
  behaviour untouched by this spike (confirmed by reading `main.rs` and
  `demo_assets.rs`), not a defect in `SpriteDepthMaterial`, and it did not
  overlap or interfere with the region under test.

## What M8 inherits

- `specialize` needs nothing beyond the depth-write flip proven here: one
  line (`depth_stencil.depth_write_enabled = Some(true)`) was sufficient for
  both the readback test and the by-eye demo. No other pipeline state
  needed adjustment.
- The depth sheet's sampler was not stressed here: `depth_step_image` is a
  binary step (only 0.0 and 1.0), and the observed screen-space transition
  was a single pixel wide with no intermediate colour, so no fringing was
  visible at this step. That is not the same as proving filtering is safe —
  a binary step atlas can land its boundary on a texel edge by luck, and it
  cannot expose the graduated-midtone interpolation problem a real depth
  sheet will have. M8 should still budget for testing NEAREST filtering (or
  a dedicated depth sampler) once the depth sheet holds continuous values
  rather than a hard 0/1 split.
- Layer-vs-layer interpenetration (two sprite layers overlapping each
  other, both writing depth) is untested — this spike used one quad against
  one opaque mesh. M8's `SpriteLayer` rewrite will need its own check for
  that case; nothing here rules it in or out.
- The sRGB-vs-linear distinction for the depth texture (`Rgba8Unorm`, not
  `Rgba8UnormSrgb` — see the comment on `depth_step_image`) mattered in
  principle but could not be *observed* mattering here, because the atlas
  only ever holds 0.0 and 1.0, both fixed points of the sRGB transfer curve
  that survive misclassification unchanged. M8's real depth sheets will
  hold midtones, where getting this wrong would visibly warp the depth
  mapping; this spike verifies the reasoning, not the pixels, for that
  point.

## Surprises

Nothing that contradicted the plan's pre-verified facts, and nothing that
cost more than ten minutes. The only unanticipated item was the incidental
`DemoCube` render described above, and that traces to `sway-app/main.rs`'s
existing unconditional project-loading, not to anything this spike built —
five minutes of reading `main.rs` and `demo_assets.rs` was enough to
identify and rule it out as a confound.

## Not answered

- Atlas-cell animation — out of scope here, M8 proper.
- More than one sprite layer at once.
- Performance. One quad says nothing about the real layer count.
