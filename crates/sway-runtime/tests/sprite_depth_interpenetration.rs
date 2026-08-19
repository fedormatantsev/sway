//! The M8 spike's measurement instrument.
//!
//! Architecture §9 says rendering is verified by eye with no pixel-diff
//! tests. This is exempt: it answers a go/no-go question rather than
//! guarding a regression, and squinting at a screenshot is not a
//! trustworthy way to answer it. It follows the readback precedent set by
//! `headless.rs`'s `bevy_render_output_reaches_the_viewport_texture`.
//!
//! The scene: a 2x2x2 unlit green cube at the origin, and a 3.0-wide red
//! billboard also centred at the origin, so its plane bisects the cube. The
//! depth sheet is a step -- left half near, right half far -- which with a
//! pivot of 0.5 and a range of 4.0 puts the left half 2 units in front of
//! the cube and the right half 2 units behind it.
//!
//! So over the cube, the left half of the sprite must win and the right
//! half must lose. Without frag_depth the whole quad sits at plane depth 0
//! and both halves render identically -- that asymmetry is the entire
//! proof.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use sway_gpu::wgpu;
use sway_runtime::sprite_depth_spike::{
    SPIKE_DEPTH_PIVOT, SPIKE_DEPTH_RANGE, SpriteDepthMaterial, SpriteDepthPlugin,
    SpriteDepthUniform, depth_step_image, solid_white_image,
};

const VIEWPORT: u32 = 64;
const ATLAS: u32 = 16;
const QUAD_SCALE: f32 = 3.0;

/// Reads the whole viewport back as RGBA8 rows.
///
/// `bytes_per_row` must be padded to `COPY_BYTES_PER_ROW_ALIGNMENT` (256);
/// wgpu does not do it for you. Mapping is async, so `device.poll` has to
/// drive the callback or the recv below hangs forever.
fn read_pixels(gpu: &sway_gpu::GpuContext, viewport: &sway_gpu::ViewportTexture) -> Vec<[u8; 4]> {
    let bytes_per_pixel = 4u32;
    let unpadded = VIEWPORT * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;

    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sprite depth spike readback"),
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
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("device poll failed");
    rx.recv()
        .expect("map_async never ran")
        .expect("mapping failed");

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

fn pixel(pixels: &[[u8; 4]], x: u32, y: u32) -> [u8; 4] {
    pixels[(y * VIEWPORT + x) as usize]
}

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut sprites: ResMut<Assets<SpriteDepthMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    // Unlit so the assertion does not depend on a light rig.
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(2.0, 2.0, 2.0))),
        MeshMaterial3d(standard.add(StandardMaterial {
            base_color: Color::srgb(0.0, 1.0, 0.0),
            unlit: true,
            ..default()
        })),
        Transform::default(),
    ));

    let material = sprites.add(SpriteDepthMaterial {
        layer: SpriteDepthUniform {
            placement: Vec3::ZERO.extend(QUAD_SCALE),
            tint: Vec4::new(1.0, 0.0, 0.0, 0.85),
            atlas: Vec4::new(1.0, 1.0, 0.0, 0.0),
            depth_params: Vec4::new(SPIKE_DEPTH_PIVOT, SPIKE_DEPTH_RANGE, 0.0, 0.0),
        },
        color_texture: images.add(solid_white_image(ATLAS)),
        depth_texture: images.add(depth_step_image(ATLAS)),
    });

    commands.spawn((
        Mesh3d(meshes.add(Rectangle::default())),
        MeshMaterial3d(material),
        // The shader billboards out of the uniform and never reads this,
        // but the transparent pass sorts by GlobalTransform, so the two
        // must agree. Same trade-off sprite_layer.rs documents.
        Transform::default(),
        NoFrustumCulling,
    ));

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

#[test]
fn a_sprite_with_a_depth_channel_interpenetrates_a_cube() {
    let gpu = sway_gpu::GpuContext::new(None);
    let size = UVec2::new(VIEWPORT, VIEWPORT);
    let viewport = sway_gpu::ViewportTexture::new(&gpu.device, size.x, size.y);
    let mut app = sway_runtime::headless::build_app(&gpu, &viewport, size, std::env::temp_dir());
    app.add_plugins(SpriteDepthPlugin)
        .add_systems(Startup, setup_scene);
    app.finish();
    app.cleanup();

    // At distance 6 with the default 45-degree FOV the visible half-height
    // is 6 * tan(22.5) ~= 2.49, so the 2.0-wide cube spans roughly the
    // middle 40% of the frame: x in [0.3, 0.7] of the width. These two
    // samples sit inside the cube, one either side of the sprite's centre
    // (and therefore either side of the depth sheet's step).
    let near_half = (VIEWPORT as f32 * 0.40) as u32;
    let far_half = (VIEWPORT as f32 * 0.60) as u32;
    let mid = VIEWPORT / 2;

    // Bounded poll, not a fixed count: bevy_core_pipeline's upscaling
    // pipeline compiles asynchronously, and until it is ready the viewport
    // is cleared to the wrong colour with no validation error. Cold caches
    // in this codebase have needed as many as 60 updates. See the doc
    // comment on headless.rs's readback test.
    const MAX_UPDATES: u32 = 300;
    let mut pixels = Vec::new();
    let mut converged = None;
    for updates in 1..=MAX_UPDATES {
        app.update();
        pixels = read_pixels(&gpu, &viewport);
        let near = pixel(&pixels, near_half, mid);
        if near[0] > 150 && near[1] < 100 {
            converged = Some(updates);
            break;
        }
    }

    let near = pixel(&pixels, near_half, mid);
    let far = pixel(&pixels, far_half, mid);

    assert!(
        converged.is_some(),
        "the sprite's near half never rendered red after {MAX_UPDATES} updates \
         (last read {near:?}); either nothing drew at all, the pipeline never \
         finished compiling, or per-pixel depth is not reaching the depth \
         buffer (see the far-half assertion below for the direct check)"
    );

    // The near half is 2 units in front of the cube: red must win.
    assert!(
        near[0] > near[1],
        "near half at x={near_half} should be sprite-red, got {near:?}"
    );

    // The far half is 2 units behind the cube: green must win. This is the
    // assertion that fails without frag_depth -- the quad would be one flat
    // plane and this pixel would be red too.
    assert!(
        far[1] > far[0],
        "far half at x={far_half} should be cube-green (the sprite is behind \
         the cube there), got {far:?}. If this is red, per-pixel depth is not \
         reaching the depth buffer: check that specialize actually ran and that \
         frag_depth is being written."
    );

    eprintln!(
        "sprite depth spike: converged after {} update(s); near={near:?} far={far:?}",
        converged.unwrap()
    );
}
