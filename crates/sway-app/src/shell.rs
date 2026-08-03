//! The winit shell: the one event loop shared by both run paths.
//!
//! Task 2 built this file as a standalone editor-only shell (window, shared
//! device, a vello-painted UI texture, compositor). Task 3 unified it with
//! the plain Bevy path: `DefaultPlugins` creates its own winit event loop as
//! soon as `add_plugins` runs (not lazily at `app.run()`), and winit allows
//! only one event loop per process, so there can no longer be a separate
//! "just call `app.run()`" path alongside this shell -- every run, demo or
//! editor, now goes through here. Task 4 retires Task 2's placeholder vello
//! demo (a solid rectangle, and `--editor` falling back to `ShowPresenter`)
//! in favour of the real thing: a masonry `RenderRoot` fed winit events
//! through this shell, painted through vello, and composited alongside the
//! Bevy viewport by `EditorPresenter` (see `presenter.rs`).

use std::sync::Arc;

use bevy::app::App;
use bevy::math::UVec2;
use sway_gpu::{Compositor, GpuContext, ViewportTexture, WindowSurface};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::presenter::{EditorPresenter, ShowPresenter, EDITOR_VIEWPORT_SIZE};

/// Which presenter this run uses, selected once at window creation
/// (`ShellConfig::editor`) and never switched at runtime.
enum Presenter {
    Show(ShowPresenter),
    Editor(Box<EditorPresenter>),
}

/// Builds the demo-specific Bevy `App` once the window, shared device, and
/// viewport texture exist. Boxed so `main` can hand the shell a closure that
/// closes over MIDI setup and the `--demo`/scene selection without the shell
/// needing to know about either -- `sway_runtime::headless::build_app` does
/// the actual `App` construction; this closure just adds whatever's specific
/// to this run on top of the `App` it returns.
pub type AppBuilder = Box<dyn FnOnce(&GpuContext, &ViewportTexture, UVec2) -> App>;

/// What to run once the window is up.
pub struct ShellConfig {
    /// Selects the window title and, below in `resumed`, which `Presenter`
    /// this run uses: `--editor` gets the real `EditorPresenter` (masonry's
    /// three-pane UI, see `sway_editor::EditorUi`); its absence gets the
    /// plain `ShowPresenter` (viewport fullscreen, no masonry).
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
    presenter: Presenter,
}

impl Running {
    fn redraw(&mut self) {
        match &mut self.presenter {
            Presenter::Show(presenter) => presenter.present(
                &mut self.app,
                &self.gpu,
                &self.surface,
                &self.viewport,
                &mut self.compositor,
            ),
            Presenter::Editor(presenter) => presenter.present(
                &mut self.app,
                &self.gpu,
                &self.surface,
                &mut self.viewport,
                &mut self.compositor,
            ),
        }
        // Keeps the loop continuous: vsync (the surface is `Fifo`) paces us,
        // not this call. This also covers `begin_frame` returning `None`
        // (occluded/timeout, handled inside the presenter's `present`):
        // asking again is how the loop notices when the surface becomes
        // presentable.
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
        let scale_factor = window.scale_factor();

        let surface = WindowSurface::new(&gpu.instance, &gpu.device, &gpu.adapter, window.clone());

        // The viewport texture's initial size differs by presenter: `Show`
        // fills the whole window; `Editor` bootstraps at the logical
        // `EDITOR_VIEWPORT_SIZE` converted to physical pixels so the
        // first `present` doesn't have to resize on Retina.
        let (viewport_width, viewport_height) = if config.editor {
            (
                (EDITOR_VIEWPORT_SIZE.width * scale_factor).round().max(1.0) as u32,
                (EDITOR_VIEWPORT_SIZE.height * scale_factor).round().max(1.0) as u32,
            )
        } else {
            (width, height)
        };

        let viewport = ViewportTexture::new(&gpu.device, viewport_width, viewport_height);
        let compositor = Compositor::new(&gpu.device, surface.format());

        let mut app = (config.build_app)(
            &gpu,
            &viewport,
            UVec2::new(viewport_width, viewport_height),
        );
        // Must run once, after construction and before the first `app.update()`,
        // or render resources stay uninitialised (normally an `App::run` runner
        // busy-waits on `plugins_state() == Ready` before calling these; we skip
        // that wait). Calling `finish()` immediately, with no wait, is safe here
        // because we build natively: both `RenderCreation::Manual` and the
        // default `RenderCreation::Automatic` resolve `FutureRenderResources`
        // synchronously inside `Plugin::build` on native targets (`Automatic`
        // does so via `bevy_tasks::block_on`) -- it's wasm32 that instead
        // detaches the resolution as a task, which is where `RenderPlugin::finish`'s
        // `.unwrap()` on a still-empty resource would actually panic, and where
        // the `plugins_state() == Ready` wait this shell skips would be needed.
        app.finish();
        app.cleanup();

        // R6 (controller dispatch ruling): `--editor` selects the real
        // `EditorPresenter`, not the `ShowPresenter` fallback Task 3 left in
        // place.
        let presenter = if config.editor {
            Presenter::Editor(Box::new(EditorPresenter::new(&gpu, size, scale_factor)))
        } else {
            Presenter::Show(ShowPresenter)
        };

        self.running = Some(Running {
            window,
            gpu,
            surface,
            viewport,
            compositor,
            app,
            presenter,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        let Some(running) = &mut self.running else {
            return;
        };

        // Feed every window event to masonry first, same as the reference
        // host (`masonry_winit::event_loop_runner::MasonryState::handle_window_event`,
        // which runs its event reducer ahead of its own resize/redraw match).
        // Most winit events (redraws, resizes, close requests) don't
        // translate into a masonry event at all -- that's `ui-events-winit`'s
        // reducer's call, not this shell's -- so this is safe to do
        // unconditionally for every event this shell also handles below.
        if let Presenter::Editor(presenter) = &mut running.presenter {
            presenter.handle_winit_event(running.window.scale_factor(), &event);
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                running.surface.resize(&running.gpu.device, size);
                match &mut running.presenter {
                    Presenter::Show(_) => {
                        let (width, height) = (size.width.max(1), size.height.max(1));
                        running.viewport.resize(&running.gpu.device, width, height);
                        // The resize just recreated the viewport texture (and
                        // its views), invalidating whatever
                        // `ManualTextureViews` entry the app's
                        // `VIEWPORT_HANDLE` pointed at -- repoint it.
                        sway_runtime::headless::set_viewport_view(
                            &mut running.app,
                            &running.viewport,
                            UVec2::new(width, height),
                        );
                    }
                    Presenter::Editor(presenter) => {
                        // Carried finding from Task 4: a minimized window can
                        // deliver `(0, 0)` here, and masonry's layout pass
                        // has a documented panic on non-finite/negative
                        // resolved dimensions. The show path above already
                        // clamps before touching its own resources; this
                        // path didn't clamp before handing `size` to
                        // masonry, so it's fixed here too.
                        let size = PhysicalSize::new(size.width.max(1), size.height.max(1));
                        presenter.resize(size, running.window.scale_factor());
                    }
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // Mirror masonry_winit: Rescale only. A `Resized` often
                // follows when the OS also changes the physical size.
                if let Presenter::Editor(presenter) = &mut running.presenter {
                    presenter.rescale(scale_factor);
                }
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
