//! The one check that `assets/cube.gltf` is a file Bevy can actually read.
//!
//! A world-shape test cannot reach this: a wrong sub-asset label, a malformed
//! buffer, or an asset root that differs under `cargo test` all leave a
//! perfectly-shaped world and an empty screen. Needs a real device only because
//! `Assets<Mesh>` comes from the render plugins.

use bevy::prelude::*;

#[test]
fn the_cube_asset_loads_as_a_mesh() {
    let gpu = sway_gpu::GpuContext::new(None);
    let size = UVec2::new(16, 16);
    let viewport = sway_gpu::ViewportTexture::new(&gpu.device, size.x, size.y);
    let mut app = sway_runtime::headless::build_app(&gpu, &viewport, size);
    app.finish();
    app.cleanup();

    let handle: Handle<Mesh> = app
        .world()
        .resource::<AssetServer>()
        .load("cube.gltf#Mesh0/Primitive0");

    // Asset loading is asynchronous, so this polls rather than assuming a
    // frame count.
    let mut loaded = None;
    for updates in 1..=300 {
        app.update();
        if let Some(mesh) = app.world().resource::<Assets<Mesh>>().get(&handle) {
            loaded = Some((updates, mesh.count_vertices(), mesh.indices().map(|i| i.len())));
            break;
        }
    }

    let state = app.world().resource::<AssetServer>().load_state(&handle);
    let (updates, vertices, indices) =
        loaded.unwrap_or_else(|| panic!("cube.gltf never loaded; load state = {state:?}"));
    eprintln!("cube.gltf loaded after {updates} update(s)");
    assert_eq!(vertices, 24, "six faces of four corners, hard edges");
    assert_eq!(indices, Some(36), "two triangles per face");
}
