# Roadmap

Nice-to-have ideas, not committed work. Capture thoughts here and pick the next
OpenSpec change from this list. Items are unordered; move the next one to the
top when you prioritize.

Committed and in-progress work lives in `openspec/changes/`. MVP scope is in
`docs/architecture.md` §10.

Tags mark the part of the project an idea belongs to: `#editor` `#graph`
`#nodes` `#midi` `#scene` `#document` `#runtime`.

## Backlog

- [ ] Composite inspector widgets `#editor`
  Vec2 should be two f32 boxes, not a single text field. Same for other vectors
  and matrices.

- [ ] Zoom-aware wire drawing `#editor`
  Canvas wire width and bezier control points should scale with zoom so
  connections stay readable when zoomed in or out.

- [ ] Socket names on the canvas `#editor`
  Inlet and outlet names visible on the node in the canvas, not only in the
  inspector.

- [ ] Spring integrator `#nodes`
  A spring node (Framer Motion–style) for smooth value animation toward a
  target.

- [ ] Project asset browser `#editor` `#document`
  Browse project assets with previews.

- [ ] Event-driven UI `#editor`
  UI rendering driven by UI events, not the main render loop. True retained UI.

- [ ] Camera DOF and post-processing `#scene` `#runtime`
  Depth of field and other post-processing effects on the camera.

- [ ] Light and camera gizmos in the preview `#editor` `#scene`
  Render gizmos and icons for lights and cameras in the editor preview.

- [ ] Infinite unit grid in the preview `#editor` `#scene`
  Infinite unit grid in the editor preview.
