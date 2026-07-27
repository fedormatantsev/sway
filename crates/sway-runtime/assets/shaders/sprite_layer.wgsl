// Z-depth sprite layer: a textured, alpha-blended billboard quad.
//
// Deviations from the task-4 brief's verbatim WGSL (see sprite_layer.rs for
// the full rationale):
//
// 1. The view uniform is Bevy's own `View` (imported from
//    `bevy_pbr::mesh_view_bindings`), not a small self-contained struct. The
//    `Material` trait always renders through Bevy's `MeshPipeline`, which
//    owns @group(0) with its *real*, much larger `View` type (clip_from_world
//    is field 0, but there are a dozen more fields plus lights/shadow
//    bindings after it) — a hand-rolled struct with only two matrices would
//    misalign every field after the first. This takes the shader out of the
//    naga validator's reach (see PREPROCESSOR_SHADERS in
//    shader_validation.rs).
// 2. `camera_right` / `camera_up` are derived from `view.world_from_view`'s
//    first two columns (view-space +x/+y expressed in world space) instead
//    of being passed in directly, since Bevy's real `View` has no such
//    fields.
// 3. The material bind group is @group(3), matching bevy_pbr's
//    `MATERIAL_BIND_GROUP_INDEX` (fixed at 3 in this Bevy version — verified
//    against bevy_pbr::material source, not the symbolic `#{MATERIAL_BIND_GROUP}`
//    shader-def, so this stays valid, parseable WGSL). @group(1) is already
//    Bevy's mesh-view binding-array group and @group(2) is the mesh group;
//    a material can't claim either.
// 4. The vertex input is Bevy's standard `Mesh::ATTRIBUTE_POSITION`
//    (`position: vec3<f32>` at location 0) rather than a bespoke `corner:
//    vec2<f32>` attribute, so a plain `Rectangle` mesh (positions in
//    [-0.5, 0.5], z = 0) can be used as-is with no custom vertex-buffer
//    specialization — keeping this Material-trait path far lighter than
//    Task 3's custom pipeline. `corner` is just `position.xy`.

#import bevy_pbr::mesh_view_bindings::view

struct Layer {
    // xy = world centre, z = depth, w = uniform scale
    placement: vec4<f32>,
    tint: vec4<f32>,
    // xy = atlas cell size in UV, zw = atlas cell offset
    atlas: vec4<f32>,
};

@group(3) @binding(0) var<uniform> layer: Layer;
@group(3) @binding(1) var sprite_texture: texture_2d<f32>;
@group(3) @binding(2) var sprite_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vertex(in: VertexIn) -> VertexOut {
    let corner = in.position.xy;
    let camera_right = view.world_from_view[0].xyz;
    let camera_up = view.world_from_view[1].xyz;

    let centre = layer.placement.xyz;
    let scale = layer.placement.w;
    let world = centre
        + camera_right * corner.x * scale
        + camera_up * corner.y * scale;

    var out: VertexOut;
    out.clip_position = view.clip_from_world * vec4<f32>(world, 1.0);
    // corner is in [-0.5, 0.5]; map to the atlas cell.
    let cell_uv = corner + vec2<f32>(0.5, 0.5);
    out.uv = layer.atlas.zw + cell_uv * layer.atlas.xy;
    return out;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let sampled = textureSample(sprite_texture, sprite_sampler, in.uv);
    let c = sampled * layer.tint;
    if (c.a < 0.001) {
        discard;
    }
    return c;
}
