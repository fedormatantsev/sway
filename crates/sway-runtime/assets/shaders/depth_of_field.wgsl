// Gaussian-ish depth of field onto a distinct destination. Samples the source
// camera's colour and depth; does not ping-pong the camera's own ViewTarget.
//
// Circle of confusion matches Bevy `dof.wgsl`: Wikipedia CoC (fraction of
// sensor height) times framebuffer height, clamped to max_coc pixels.

struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

struct DofParams {
    focal_distance: f32,
    focal_length: f32,
    coc_scale: f32,
    max_coc: f32,
    near: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0) var color_texture: texture_2d<f32>;
@group(0) @binding(1) var depth_texture: texture_depth_2d;
@group(0) @binding(2) var color_sampler: sampler;
@group(0) @binding(3) var<uniform> params: DofParams;

fn view_z(depth: f32) -> f32 {
    // Reverse-Z infinite perspective: positive distance = near / ndc_depth.
    // Bevy's `depth_ndc_to_view_z` returns negative view Z; DoF then negates it.
    return params.near / max(depth, 1.0e-6);
}

fn circle_of_confusion(depth: f32, dims: vec2<f32>) -> f32 {
    let z = min(view_z(depth), 1.0e6);
    let candidate = params.coc_scale * abs(z - params.focal_distance)
        / (z * max(params.focal_distance - params.focal_length, 1.0e-7));
    return clamp(candidate * dims.y, 0.0, params.max_coc);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(color_texture));
    let pixel = vec2<i32>(clamp(in.uv * dims, vec2<f32>(0.0), dims - vec2<f32>(1.0)));
    let depth = textureLoad(depth_texture, pixel, 0);
    let coc = circle_of_confusion(depth, dims);
    let center = textureSampleLevel(color_texture, color_sampler, in.uv, 0.0);
    if coc < 0.5 {
        return center;
    }

    // CoC is a diameter in pixels. Sample a disc of that radius.
    var acc = center;
    var weight = 1.0;
    let radius = (coc * 0.5) / max(dims.y, 1.0);
    let taps = array<vec2<f32>, 8>(
        vec2<f32>(1.0, 0.0),
        vec2<f32>(-1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, -1.0),
        vec2<f32>(0.7071, 0.7071),
        vec2<f32>(-0.7071, 0.7071),
        vec2<f32>(0.7071, -0.7071),
        vec2<f32>(-0.7071, -0.7071),
    );
    for (var i = 0; i < 8; i = i + 1) {
        let sample_uv = in.uv + taps[i] * radius;
        acc += textureSampleLevel(color_texture, color_sampler, sample_uv, 0.0);
        weight += 1.0;
    }
    return acc / weight;
}
