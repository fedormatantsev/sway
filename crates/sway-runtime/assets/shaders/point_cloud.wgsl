// Point cloud vertex/fragment shader for the instanced draw in point_cloud.rs.
//
// Uses Bevy's mesh view/mesh-functions preprocessor imports (mesh_functions
// wraps the per-instance world-from-local matrix and the local-to-clip
// transform), so naga cannot parse this file directly. It is deliberately
// listed in `PREPROCESSOR_SHADERS` in shader_validation.rs rather than
// stripped down to be self-contained.
#import bevy_pbr::mesh_functions

// Locations 0-2 (position, normal, uv) come from the base mesh, per vertex.
// Locations 3-4 are the per-instance attributes pushed in point_cloud.rs's
// `SpecializedMeshPipeline` impl: a position+scale vec4 and an RGBA colour.
struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) point_position_scale: vec4<f32>,
    @location(4) point_color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    let scale = vertex.point_position_scale.w;
    let offset = vertex.point_position_scale.xyz;
    // Base-mesh vertices are centred on the origin; scale them down to point
    // size, then translate to this instance's position in local space before
    // the (identity, for this draw) mesh transform and view projection.
    let local_position = vertex.position * scale + offset;

    var out: VertexOutput;
    out.clip_position = mesh_functions::mesh_position_local_to_clip(
        mesh_functions::get_world_from_local(vertex.instance_index),
        vec4<f32>(local_position, 1.0),
    );
    out.color = vertex.point_color;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
