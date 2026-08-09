// M8 spike: a billboard quad that writes per-pixel depth from a depth sheet.
//
// Same three Material-trait constraints as sprite_layer.wgsl, for the same
// reasons documented there: Bevy's own `View` is imported rather than
// redeclared (a hand-rolled struct would misalign every field after the
// first); camera right/up come from `world_from_view`'s columns; and the
// material bind group is @group(3), because MATERIAL_BIND_GROUP_INDEX is a
// fixed 3 in this Bevy version.
//
// What is new here is the fragment stage. It returns @builtin(frag_depth)
// alongside colour, computed by displacing the fragment's world position
// along the camera's forward axis by the sampled depth channel and
// re-projecting through Bevy's own clip_from_world.
//
// Re-projecting rather than computing a depth value directly is deliberate.
// Bevy renders reverse-Z (Depth32Float, CompareFunction::GreaterEqual, so
// 1.0 is the near plane) with an infinite far plane. Hand-rolling a depth
// under that convention is easy to get subtly wrong and impossible to
// verify by eye. Pushing a world position back through the same matrix the
// vertex stage used cannot disagree with it.

#import bevy_pbr::mesh_view_bindings::view

struct Layer {
    // xy = world centre, z = depth, w = uniform scale
    placement: vec4<f32>,
    tint: vec4<f32>,
    // xy = atlas cell size in UV, zw = atlas cell offset
    atlas: vec4<f32>,
    // x = the sheet value meaning "at the quad's plane", y = world units
    // spanned by the full 0..1 depth channel, zw = unused
    depth_params: vec4<f32>,
};

@group(3) @binding(0) var<uniform> layer: Layer;
@group(3) @binding(1) var color_texture: texture_2d<f32>;
@group(3) @binding(2) var color_sampler: sampler;
@group(3) @binding(3) var depth_texture: texture_2d<f32>;
@group(3) @binding(4) var depth_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // Carried through so the fragment stage can displace it and reproject.
    @location(1) world_position: vec3<f32>,
};

struct FragmentOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
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
    out.world_position = world;
    let cell_uv = corner + vec2<f32>(0.5, 0.5);
    out.uv = layer.atlas.zw + cell_uv * layer.atlas.xy;
    return out;
}

@fragment
fn fragment(in: VertexOut) -> FragmentOut {
    let sampled = textureSample(color_texture, color_sampler, in.uv);
    let c = sampled * layer.tint;
    if (c.a < 0.001) {
        discard;
    }

    // Higher channel value means farther from the camera.
    let sheet_depth = textureSample(depth_texture, depth_sampler, in.uv).r;

    // `world_from_view[2]` is view-space +z expressed in world space, which
    // points back toward the viewer in a right-handed view space. Negating
    // it gives the direction away from the camera.
    let camera_forward = -view.world_from_view[2].xyz;
    let offset = (sheet_depth - layer.depth_params.x) * layer.depth_params.y;
    let displaced = in.world_position + camera_forward * offset;

    let clip = view.clip_from_world * vec4<f32>(displaced, 1.0);

    var out: FragmentOut;
    out.color = c;
    // Clamp is a guard against undefined behaviour, not a correct fallback.
    // A fragment pushed behind the camera has w <= 0, so clip.z / clip.w
    // is typically positive-large and clamps to 1.0 -- which under this
    // reverse-Z convention is the near plane, so that fragment would
    // incorrectly WIN the depth test against everything else in the scene
    // (w == 0 exactly yields NaN, whose behaviour through clamp is also
    // unspecified). Production code should discard such a fragment, or
    // constrain depth_params so the displaced position cannot cross the
    // camera, rather than rely on this clamp to do something sane.
    out.depth = clamp(clip.z / clip.w, 0.0, 1.0);
    return out;
}
