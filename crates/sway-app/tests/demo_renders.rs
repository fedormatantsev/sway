//! Does the demo document actually put pixels on the screen?
//!
//! Architecture §9 says rendering is verified by eye, and that stands for how
//! the scene looks. It does not cover whether the glTF resolves at all: a wrong
//! sub-asset label, an asset root that differs under `cargo test`, or a mesh
//! whose material never arrives all produce a world of exactly the right shape
//! and an empty frame. This asserts only "lit geometry rendered", not what it
//! looks like — the by-eye run is still what judges that.
//!
//! Follows the readback precedent in `sway-runtime/tests/
//! sprite_depth_interpenetration.rs` and `sway-runtime/src/headless.rs`.

use bevy::prelude::*;
use sway_gpu::wgpu;

const VIEWPORT: u32 = 128;
const DEMO_DOCUMENT: &str = include_str!("../assets/demo.sway.ron");

/// Reads the whole viewport back as RGBA8 pixels.
///
/// `bytes_per_row` must be padded to `COPY_BYTES_PER_ROW_ALIGNMENT` (256);
/// wgpu does not do it for you. Mapping is async, so `device.poll` has to drive
/// the callback or the recv below hangs forever.
fn read_pixels(gpu: &sway_gpu::GpuContext, viewport: &sway_gpu::ViewportTexture) -> Vec<[u8; 4]> {
    let bytes_per_pixel = 4u32;
    let unpadded = VIEWPORT * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;

    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("demo renders readback"),
        size: u64::from(padded) * u64::from(VIEWPORT),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: viewport.texture(),
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(VIEWPORT),
            },
        },
        wgpu::Extent3d {
            width: VIEWPORT,
            height: VIEWPORT,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("device poll failed");
    rx.recv()
        .expect("map_async callback never ran")
        .expect("buffer mapping failed");

    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((VIEWPORT * VIEWPORT) as usize);
    for row in 0..VIEWPORT {
        let start = (row * padded) as usize;
        for col in 0..VIEWPORT {
            let at = start + (col * bytes_per_pixel) as usize;
            pixels.push([data[at], data[at + 1], data[at + 2], data[at + 3]]);
        }
    }
    drop(data);
    readback.unmap();
    pixels
}

/// The cubes are pale blue (`base_color: (0.6, 0.7, 0.9)`) and lit; the default
/// clear colour is a dark neutral grey (43, 44, 47). "Blue clearly ahead of red,
/// and bright" is true of the cube and of nothing else in this frame.
fn is_cube(pixel: [u8; 4]) -> bool {
    pixel[2] > 90 && pixel[2] as i16 - pixel[0] as i16 > 15
}

#[test]
fn the_demo_document_renders_its_cubes() {
    let gpu = sway_gpu::GpuContext::new(None);
    let size = UVec2::new(VIEWPORT, VIEWPORT);
    let viewport = sway_gpu::ViewportTexture::new(&gpu.device, size.x, size.y);
    let mut app = sway_runtime::headless::build_app(&gpu, &viewport, size);
    app.add_plugins((sway_graph::WiresPlugin, sway_nodes::WireNodesPlugin));
    app.finish();
    app.cleanup();

    // The document is applied directly rather than through ProjectPlugin's
    // asset load: this test is about the scene it describes, not about the
    // .ron's own loading path, which demo_document.rs already covers.
    let document = sway_document::parse(DEMO_DOCUMENT).expect("parses");
    let diagnostics = sway_document::apply(app.world_mut(), &document);
    assert!(diagnostics.is_clean(), "{:?}", diagnostics.items);

    // A bounded poll, not a fixed count. Two independent asynchronous things
    // have to finish: bevy_core_pipeline's upscaling pipeline compiles (until
    // it does, the viewport is cleared to the wrong colour with no validation
    // error), and the glTF loads off disk. Cold caches in this codebase have
    // needed as many as 60 updates for the first alone.
    const MAX_UPDATES: u32 = 400;
    let total = (VIEWPORT * VIEWPORT) as usize;
    let mut cube_pixels = 0;
    let mut converged = None;
    for updates in 1..=MAX_UPDATES {
        app.update();
        cube_pixels = read_pixels(&gpu, &viewport).into_iter().filter(|p| is_cube(*p)).count();
        // Two cubes of 1 unit at ~5 units from a 45-degree camera cover a few
        // percent of the frame. 1% is far above stray-pixel noise and far below
        // what the real coverage should be.
        if cube_pixels * 100 > total {
            converged = Some(updates);
            break;
        }
    }

    let updates = converged.unwrap_or_else(|| {
        panic!(
            "no lit cube pixels after {MAX_UPDATES} updates ({cube_pixels} of {total} matched). \
             Either cube.gltf never loaded (check the path and its #Mesh0/Primitive0 label), \
             the material never reached the mesh (check the MaterialFrom wire), the light or \
             camera did not spawn, or nothing rendered at all."
        )
    });
    eprintln!("demo document rendered {cube_pixels}/{total} cube pixels after {updates} update(s)");
}
