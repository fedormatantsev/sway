// Draws one textured quad into the target, positioned by a uniform rect given
// in normalised device coordinates. Two invocations per editor frame: the
// Bevy viewport, then the UI layer alpha-blended over it.
//
// No colour conversion happens here. Every texture bound to this shader
// already holds sRGB-encoded bytes and the target is a non-sRGB format, so
// values pass through untouched. See the plan's colour-space table.

struct QuadRect {
    // min_x, min_y, max_x, max_y in NDC: -1..1, y up.
    bounds: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> quad: QuadRect;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    // Two triangles as a strip-order quad: 0=(0,0) 1=(1,0) 2=(0,1) 3=(1,1),
    // emitted as a 6-vertex list 0,1,2, 2,1,3.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    let c = corners[index];

    var out: VertexOutput;
    out.clip_position = vec4<f32>(
        mix(quad.bounds.x, quad.bounds.z, c.x),
        mix(quad.bounds.y, quad.bounds.w, c.y),
        0.0,
        1.0,
    );
    // Texture v runs downward while NDC y runs upward, so v is flipped.
    out.uv = vec2<f32>(c.x, 1.0 - c.y);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(src, src_sampler, in.uv);
}
