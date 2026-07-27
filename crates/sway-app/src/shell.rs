//! The winit shell for the editor path (`--editor`).
//!
//! Bevy does not appear here — this is M1b Task 2's minimum: a window, the
//! shared wgpu device, a vello-painted UI texture, and the compositor putting
//! that texture on screen. Task 3 adds the Bevy viewport texture as a second
//! quad; Task 4 adds masonry/ui-events for real UI input.

use std::sync::Arc;

use imaging::Painter;
use imaging::record::Scene;
use kurbo::Rect;
use peniko::{Brush, Color};
use sway_gpu::{Compositor, GpuContext, Quad, UiRenderer, UiTexture, WindowSurface};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

/// Everything that exists only once the window (and therefore the GPU
/// context bound to its surface) is up. `None` before the first `resumed`
/// and, in principle, across a suspend/resume cycle -- this app only runs on
/// desktop, where that cycle doesn't happen, but the `Option` costs nothing
/// and matches winit's own lifecycle expectations.
struct Running {
    window: Arc<Window>,
    gpu: GpuContext,
    surface: WindowSurface,
    ui_texture: UiTexture,
    compositor: Compositor,
    ui_renderer: UiRenderer,
}

impl Running {
    /// Paints one solid rectangle into the UI scene, renders it to the UI
    /// texture, and composites that single quad fullscreen onto the window
    /// surface.
    fn redraw(&mut self) {
        // Begin the frame first: if the window is occluded or minimized
        // there is nothing to draw, and no point painting the UI scene.
        // Keep the loop alive by asking for another redraw so we notice
        // when the surface becomes presentable again.
        let Some(mut frame) = self
            .surface
            .begin_frame(&self.gpu.device, &self.gpu.queue, &mut self.compositor)
        else {
            self.window.request_redraw();
            return;
        };

        let size = self.window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));

        let mut scene = Scene::new();
        {
            let mut painter = Painter::new(&mut scene);
            let margin_x = width as f64 * 0.2;
            let margin_y = height as f64 * 0.2;
            let rect = Rect::new(
                margin_x,
                margin_y,
                width as f64 - margin_x,
                height as f64 - margin_y,
            );
            let brush = Brush::Solid(Color::from_rgb8(0x2a, 0x6f, 0xdb));
            painter.fill_rect(rect, &brush);
        }

        self.ui_renderer
            .render_scene(&scene, &self.ui_texture.view, width, height);

        frame.composite(&[Quad {
            view: &self.ui_texture.view,
            dst: Rect::new(0.0, 0.0, width as f64, height as f64),
            blend: true,
        }]);

        frame.present();

        // Keeps the loop continuous: vsync (the surface is `Fifo`) paces us,
        // not this call.
        self.window.request_redraw();
    }
}

#[derive(Default)]
struct Shell {
    running: Option<Running>,
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }

        let window = event_loop
            .create_window(Window::default_attributes().with_title("sway (editor)"))
            .expect("could not create the editor window");
        let window = Arc::new(window);

        // `GpuContext::new`'s `compatible_surface` exists so the adapter
        // chosen can actually present. The textbook order is surface-first:
        // build the surface from an instance, then pick an adapter
        // compatible with it. That isn't available here in that order,
        // though, because `WindowSurface::new` (below) is the thing that
        // builds the real, lasting surface, and it needs `gpu.instance`,
        // `gpu.device` and `gpu.adapter` to do it -- so the context has to
        // exist first. Passing `None` is safe on this machine specifically:
        // Task 1's `GpuContext` established there is exactly one Metal
        // adapter here, so `None` selects the identical adapter `Some(&surface)`
        // would have chosen. A machine with more than one GPU would need
        // `GpuContext` split into an instance-creation step and a
        // device-request step so a throwaway compatibility surface could be
        // built in between; flagged in the task-2 report for whoever ports
        // this off this Mac.
        let gpu = GpuContext::new(None);

        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));

        let surface = WindowSurface::new(&gpu.instance, &gpu.device, &gpu.adapter, window.clone());
        let ui_texture = UiTexture::new(&gpu.device, width, height);
        let compositor = Compositor::new(&gpu.device, surface.format());
        let ui_renderer = UiRenderer::new(gpu.device.clone(), gpu.queue.clone());

        self.running = Some(Running {
            window,
            gpu,
            surface,
            ui_texture,
            compositor,
            ui_renderer,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        let Some(running) = &mut self.running else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                let (width, height) = (size.width.max(1), size.height.max(1));
                running.surface.resize(&running.gpu.device, size);
                running.ui_texture.resize(&running.gpu.device, width, height);
            }
            WindowEvent::RedrawRequested => running.redraw(),
            _ => {}
        }
    }
}

/// Runs the editor's winit shell. Blocks until the window is closed.
pub fn run() {
    let event_loop = EventLoop::new().expect("could not create the winit event loop");
    let mut shell = Shell::default();
    event_loop
        .run_app(&mut shell)
        .expect("editor event loop exited with an error");
}
