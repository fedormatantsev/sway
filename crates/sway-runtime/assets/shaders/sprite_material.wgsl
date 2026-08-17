// Sprite material: an ordinary Bevy mesh material whose depth run displaces
// the mesh's own vertices.
//
// This is NOT the M8 spike (sprite_depth_spike.wgsl). Design D1 retires that
// approach wholesale:
//
// - No @builtin(frag_depth). A fragment is rasterized from the undisplaced
//   surface, so writing a displaced depth changes only what the depth buffer
//   believes, never where the pixel is drawn — the relief stays locked to the
//   image while the plane tilts underneath it. Moving the *vertices* instead
//   lets the rasterizer project the moved geometry honestly, which is what
//   produces parallax, and fills the depth buffer as a side effect. The
//   spike's reverse-Z reprojection and its clamp(clip.z / clip.w) landmine go
//   with it: a vertex pushed behind the camera is ordinary geometry that the
//   rasterizer clips correctly.
// - No billboarding and no `placement` uniform. Unlike sprite_layer.wgsl, the
//   quad is placed by the entity's own Transform, like any other mesh, which
//   is what lets this material be wired onto arbitrary geometry.
//
// Two constraints do carry over from sprite_layer.wgsl, for the reasons
// documented in its header:
//
// - Bevy's own view/mesh bindings are imported rather than redeclared. A
//   hand-rolled struct would misalign every field after the first. This takes
//   the shader out of naga's reach (see PREPROCESSOR_SHADERS in
//   shader_validation.rs).
// - The material bind group is @group(3), matching bevy_pbr's
//   MATERIAL_BIND_GROUP_INDEX (verified as a fixed 3 in bevy_pbr::material,
//   rather than the symbolic #{MATERIAL_BIND_GROUP} shader-def, so this stays
//   parseable WGSL). @group(0)/@group(1) are the view groups and @group(2) is
//   the mesh group; a material can claim none of them.
//
// Real API note (bevy_pbr 0.19): the helpers below are
// mesh_functions::get_world_from_local / mesh_position_local_to_world /
// mesh_normal_local_to_world and view_transformations::position_world_to_clip.
// These names have churned across Bevy versions; they were read out of
// bevy_pbr-0.19.0/src/render/{mesh_functions,view_transformations}.wgsl, not
// remembered.

#import bevy_pbr::{
    mesh_functions,
    view_transformations::position_world_to_clip,
}

struct SpriteMaterialUniform {
    // rgb = tint (linear), a = opacity
    tint: vec4<f32>,
    // The array layer to sample. Already clamped into [0, layers) on the CPU
    // by `layer_index` — this shader knows nothing about frame numbers, which
    // is design D4's point: all frame mangling belongs in the node graph.
    layer: u32,
    // The run value that leaves a vertex on the undisplaced surface.
    depth_pivot: f32,
    // World units spanned by the full 0..1 depth channel.
    depth_range: f32,
};

@group(3) @binding(0) var<uniform> material: SpriteMaterialUniform;
// texture_2d_array, not texture_2d: frames are array layers (design D7), so
// they have no neighbours to bleed into each other under linear filtering.
// That matters more here than for an ordinary atlas, because the depth run is
// sampled in the vertex stage below — bleeding would drag a neighbouring
// frame's height into the geometry and make edge vertices twitch toward the
// next frame's shape.
@group(3) @binding(1) var color_texture: texture_2d_array<f32>;
@group(3) @binding(2) var color_sampler: sampler;
@group(3) @binding(3) var depth_texture: texture_2d_array<f32>;
@group(3) @binding(4) var depth_sampler: sampler;

// Bevy's standard mesh attributes, at the locations its MeshPipeline binds
// them (bevy_pbr::forward_io::Vertex). Declared here rather than imported
// because forward_io's struct is fenced by VERTEX_* shader-defs; a mesh wired
// to this material must therefore carry normals and UV0, and pipeline
// creation fails loudly if it does not.
struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vertex(in: Vertex) -> VertexOut {
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    let world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(in.position, 1.0),
    ).xyz;
    // Normalized by mesh_normal_local_to_world, so `offset` below is a length
    // in world units rather than a length scaled by the normal's magnitude.
    let world_normal = mesh_functions::mesh_normal_local_to_world(in.normal, in.instance_index);

    // textureSampleLevel with an explicit level of 0.0, NOT textureSample:
    // the latter needs implicit derivatives, which do not exist in a vertex
    // stage. Easy to get wrong once, so it is called out in the design's
    // Risks as well as here.
    let height = textureSampleLevel(
        depth_texture,
        depth_sampler,
        in.uv,
        material.layer,
        0.0,
    ).r;

    // Standard displacement-map convention: a value above the pivot pushes
    // out along the normal, below it pushes in. Note this is the opposite
    // sign to the M8 spike, where a higher value meant *farther from the
    // camera* — that convention only made sense because the spike displaced
    // along the camera's forward axis, which is exactly the thing D1 retires.
    let offset = (height - material.depth_pivot) * material.depth_range;
    let displaced = world_position + world_normal * offset;

    var out: VertexOut;
    // The displaced position is what goes to clip space. This single line is
    // the whole of D1: the rasterizer projects the moved geometry, so the
    // relief parallaxes as the mesh turns, and the depth buffer is filled by
    // ordinary rasterization with early-Z intact.
    out.clip_position = position_world_to_clip(displaced);
    out.uv = in.uv;
    return out;
}

// @location(0) only. No FragmentOut struct and no @builtin(frag_depth) —
// depth comes from the displaced geometry above.
@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    // The colour run is addressed by the same layer index as the depth run,
    // so colour and relief can never come from different frames.
    let sampled = textureSample(color_texture, color_sampler, in.uv, material.layer);
    let c = vec4<f32>(
        sampled.rgb * material.tint.rgb,
        sampled.a * material.tint.a,
    );
    // Discard rather than blend at zero alpha, so a transparent region of the
    // run neither shades nor writes depth — with depth writes forced on (see
    // `enable_depth_write`), a kept-but-invisible fragment would occlude
    // whatever lies behind it. Threshold matches sprite_layer.wgsl.
    if (c.a < 0.001) {
        discard;
    }
    return c;
}
