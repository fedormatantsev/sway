//! Owns the whole per-frame GPU lifecycle so callers never need to create a
//! `wgpu::TextureView` or `wgpu::CommandEncoder` themselves: acquiring the
//! surface texture, creating its view and a command encoder, exposing
//! compositing, and finishing/submitting/presenting. `sway-app` should be
//! able to draw a frame while creating zero `wgpu::` objects of its own --
//! this is the type that makes that true.

use wgpu::{CommandEncoder, CommandEncoderDescriptor, Device, Queue, SurfaceTexture, TextureView};

use crate::compositor::{Compositor, Quad};

/// One acquired, in-progress frame. Obtained from
/// [`crate::WindowSurface::begin_frame`], which returns `None` instead of a
/// `Frame` when there is nothing to draw this pass (the window is occluded
/// or minimized) -- see that method's docs.
///
/// Borrows the `Compositor` for its lifetime because compositing mutates the
/// compositor's per-quad resource cache (see `compositor.rs`); the borrow
/// also statically prevents starting a second frame against the same
/// compositor before this one is `present`ed.
pub struct Frame<'a> {
    device: Device,
    queue: Queue,
    compositor: &'a mut Compositor,
    surface_texture: SurfaceTexture,
    view: TextureView,
    encoder: CommandEncoder,
}

impl<'a> Frame<'a> {
    pub(crate) fn new(
        device: &Device,
        queue: &Queue,
        compositor: &'a mut Compositor,
        surface_texture: SurfaceTexture,
    ) -> Self {
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("sway frame encoder"),
        });
        Self {
            device: device.clone(),
            queue: queue.clone(),
            compositor,
            surface_texture,
            view,
            encoder,
        }
    }

    /// Composites `quads`, in order, onto this frame's surface texture.
    /// Keeps the capability the old free-standing `Compositor::draw` had:
    /// multiple quads, each with its own destination rect and blend mode.
    pub fn composite(&mut self, quads: &[Quad]) {
        self.compositor.draw(
            &mut self.encoder,
            &self.device,
            &self.queue,
            &self.view,
            quads,
        );
    }

    /// Finishes the encoder, submits it, and presents the frame.
    pub fn present(self) {
        self.queue.submit(Some(self.encoder.finish()));
        self.surface_texture.present();
    }
}
