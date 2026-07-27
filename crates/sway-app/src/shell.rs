//! The winit shell: the one event loop shared by both run paths.
//!
//! Task 2 built this file as a standalone editor-only shell (window, shared
//! device, a vello-painted UI texture, compositor). Task 3 unifies it with
//! the plain Bevy path: `DefaultPlugins` creates its own winit event loop as
//! soon as `add_plugins` runs (not lazily at `app.run()`), and winit allows
//! only one event loop per process, so there can no longer be a separate
//! "just call `app.run()`" path alongside this shell -- every run, demo or
//! editor, now goes through here. Task 2's vello demo (a solid rectangle) is
//! retired in favour of what Task 4 adds for real: masonry input and vello
//! UI composited alongside the Bevy viewport. Until then `--editor` falls
//! back to the same `ShowPresenter` as the default path (see `ShellConfig`).

use std::sync::Arc;

use bevy::app::App;
use bevy::math::UVec2;
use sway_gpu::{Compositor, GpuContext, ViewportTexture, WindowSurface};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::presenter::ShowPresenter;

/// Builds the demo-specific Bevy `App` once the window, shared device, and
/// viewport texture exist. Boxed so `main` can hand the shell a closure that
/// closes over MIDI setup and the `--demo`/scene selection without the shell
/// needing to know about either -- `sway_runtime::headless::build_app` does
/// the actual `App` construction; this closure just adds whatever's specific
/// to this run on top of the `App` it returns.
pub type AppBuilder = Box<dyn FnOnce(&GpuContext, &ViewportTexture, UVec2) -> App>;

/// What to run once the window is up.
pub struct ShellConfig {
    /// Selects the window title and (eventually, Task 4) the editor
    /// presenter. For M1b Task 3 there is only `ShowPresenter`, so this only
    /// affects the title.
    pub editor: bool,
    pub build_app: AppBuilder,
}

/// Everything that exists only once the window (and therefore the GPU
/// context bound to its surface) is up. `None` before the first `resumed`
/// and, in principle, across a suspend/resume cycle -- this app only runs on
/// desktop, where that cycle doesn't happen, but the `Option` costs nothing
/// and matches winit's own lifecycle expectations.
struct Running {
    window: Arc<Window>,
    gpu: GpuContext,
    surface: WindowSurface,
    viewport: ViewportTexture,
    compositor: Compositor,
    app: App,
    presenter: ShowPresenter,
}

impl Running {
    fn redraw(&mut self) {
        self.presenter.present(
            &mut self.app,
            &self.gpu,
            &self.surface,
            &self.viewport,
            &mut self.compositor,
        );
        // Keeps the loop continuous: vsync (the surface is `Fifo`) paces us,
        // not this call. This also covers `begin_frame` returning `None`
        // (occluded/timeout, handled inside `ShowPresenter::present`): asking
        // again is how the loop notices when the surface becomes presentable.
        self.window.request_redraw();
    }
}

struct Shell {
    config: Option<ShellConfig>,
    running: Option<Running>,
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        let Some(config) = self.config.take() else {
            // Only happens on a second `resumed` (suspend/resume), which
            // desktop winit doesn't raise; nothing to rebuild without a
            // config to rebuild from.
            return;
        };

        let title = if config.editor { "sway (editor)" } else { "sway" };
        let window = event_loop
            .create_window(Window::default_attributes().with_title(title))
            .expect("could not create the window");
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
        let viewport = ViewportTexture::new(&gpu.device, width, height);
        let compositor = Compositor::new(&gpu.device, surface.format());

        let mut app = (config.build_app)(&gpu, &viewport, UVec2::new(width, height));
        // Must run once, after construction and before the first `app.update()`,
        // or render resources stay uninitialised (normally an `App::run` runner
        // busy-waits on `plugins_state() == Ready` before calling these; we skip
        // that wait). Calling `finish()` immediately, with no wait, is safe here
        // *only* because `RenderCreation::Manual` (used by `headless::build_app`)
        // populates its `FutureRenderResources` synchronously inside `Plugin::build`
        // -- under the default `RenderCreation::Automatic`, whose device resolution
        // is asynchronous, `RenderPlugin::finish` would panic (`.unwrap()` on a
        // still-empty resource) if called before that resolution completes.
        app.finish();
        app.cleanup();

        self.running = Some(Running {
            window,
            gpu,
            surface,
            viewport,
            compositor,
            app,
            presenter: ShowPresenter,
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
                running.viewport.resize(&running.gpu.device, width, height);
                // The resize just recreated the viewport texture (and its
                // views), invalidating whatever `ManualTextureViews` entry
                // the app's `VIEWPORT_HANDLE` pointed at -- repoint it.
                sway_runtime::headless::set_viewport_view(
                    &mut running.app,
                    &running.viewport,
                    UVec2::new(width, height),
                );
            }
            WindowEvent::RedrawRequested => running.redraw(),
            _ => {}
        }
    }
}

/// Runs the shell. Blocks until the window is closed.
pub fn run(config: ShellConfig) {
    let event_loop = EventLoop::new().expect("could not create the winit event loop");
    let mut shell = Shell {
        config: Some(config),
        running: None,
    };
    event_loop
        .run_app(&mut shell)
        .expect("shell event loop exited with an error");
}
