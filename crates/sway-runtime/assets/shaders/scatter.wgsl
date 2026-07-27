// Scatter: writes `count` pseudo-random points into a storage buffer as xyz
// triples. Self-contained WGSL — no Bevy preprocessor imports — so it is
// covered by the naga validator.

struct ScatterParams {
    count: u32,
    seed: u32,
    extent: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> params: ScatterParams;
@group(0) @binding(1) var<storage, read_write> positions: array<f32>;

// PCG hash -> [0,1). Deterministic in (index, seed), which is what lets a
// recooked or replayed scatter reproduce the same cloud.
fn rand(x: u32) -> f32 {
    var h: u32 = x * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return f32(h) * 2.3283064e-10;
}

@compute @workgroup_size(64)
fn scatter(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i: u32 = gid.x;
    if (i >= params.count) {
        return;
    }
    let base: u32 = i * 3u;
    let s: u32 = params.seed;
    positions[base + 0u] = (rand(i * 3u + 0u + s) * 2.0 - 1.0) * params.extent;
    positions[base + 1u] = (rand(i * 3u + 1u + s) * 2.0 - 1.0) * params.extent;
    positions[base + 2u] = (rand(i * 3u + 2u + s) * 2.0 - 1.0) * params.extent;
}
