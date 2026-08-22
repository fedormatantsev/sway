// Luminance-weighted film grain. Intensity 0 copies the source. The hash
// includes the show-frame index so a repeated capture slot keeps the same grain.

struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

struct GrainParams {
    intensity: f32,
    frame: f32,
    _pad0: f32,
    _pad1: f32,
}

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> params: GrainParams;

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let src = textureSample(source_texture, source_sampler, in.uv);
    if params.intensity <= 0.0 {
        return src;
    }
    let n = hash21(in.uv * 1024.0 + vec2<f32>(f32(params.frame), f32(params.frame) * 0.37));
    let luma = dot(src.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let noise = (n * 2.0 - 1.0) * params.intensity * luma;
    return vec4<f32>(src.rgb + vec3<f32>(noise), src.a);
}
