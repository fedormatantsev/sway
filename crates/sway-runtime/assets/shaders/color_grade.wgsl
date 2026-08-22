// Parametric colour grade of a camera target. Identity inlets (exposure 0,
// contrast/saturation 1, temperature/tint/hue 0) MUST copy the source.

struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

struct GradeParams {
    exposure: f32,
    contrast: f32,
    saturation: f32,
    temperature: f32,
    tint: f32,
    hue: f32,
    _pad0: f32,
    _pad1: f32,
}

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> params: GradeParams;

fn hue_rotate(color: vec3<f32>, hue: f32) -> vec3<f32> {
    let cos_h = cos(hue);
    let sin_h = sin(hue);
    return vec3<f32>(
        color.r * cos_h - color.g * sin_h,
        color.r * sin_h + color.g * cos_h,
        color.b,
    );
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let src = textureSample(source_texture, source_sampler, in.uv);
    var color = src.rgb * exp2(params.exposure);
    color = mix(vec3<f32>(0.5), color, params.contrast);
    color.r = color.r + params.temperature;
    color.b = color.b - params.temperature;
    color.g = color.g + params.tint;
    color = hue_rotate(color, params.hue);
    let luma = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    color = mix(vec3<f32>(luma), color, params.saturation);
    return vec4<f32>(color, src.a);
}
