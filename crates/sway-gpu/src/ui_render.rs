//! Wraps `imaging_vello::VelloRenderer`: lowers a semantic `imaging::record::Scene`
//! into vello's native scene representation and renders it to a caller-owned
//! texture view.

use imaging::record::Scene;
use imaging_vello::VelloRenderer;
use wgpu::{Device, Queue, TextureView};

pub struct UiRenderer {
    inner: VelloRenderer,
}

impl UiRenderer {
    pub fn new(device: Device, queue: Queue) -> Self {
        let inner = VelloRenderer::new(device, queue)
            .expect("could not create the vello renderer on the shared device");
        Self { inner }
    }

    /// Encodes `scene` and renders it into `view`.
    ///
    /// `render_to_texture_view` clears to fully transparent before drawing
    /// (hardcoded in `imaging_vello`) — that's what makes the UI texture
    /// compositable over the viewport underneath it.
    pub fn render_scene(&mut self, scene: &Scene, view: &TextureView, width: u32, height: u32) {
        let native = self
            .inner
            .encode_scene(scene, width, height)
            .expect("vello could not encode the UI scene");
        self.inner
            .render_to_texture_view(&native, view, width, height)
            .expect("vello could not render the UI scene");
    }
}
