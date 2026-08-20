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

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use bevy::app::App;
use bevy::math::UVec2;
use crossbeam_channel::Sender;
use sway_editor::{FileRequest, ViewRequest};
use sway_gpu::{Compositor, GpuContext, ViewportTexture, WindowSurface};
use sway_editor::edit::EditorEdit;
use sway_viewport_input::ViewportInput;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::presenter::{EDITOR_VIEWPORT_SIZE, EditorPresenter, ShowPresenter};

/// A file dialog in flight.
///
/// `rfd`'s async form returns a future the shell polls once per redraw; the
/// blocking form would spin a nested `NSApplication` modal on the thread
/// winit's event loop already owns (M6-8). Exactly one dialog is ever open:
/// a second request while one is pending is dropped, which is also what a
/// modal dialog would do.
struct Dialog {
    future: Pin<Box<dyn Future<Output = Option<rfd::FileHandle>>>>,
}

impl Dialog {
    fn open() -> Self {
        Self {
            future: Box::pin(
                rfd::AsyncFileDialog::new()
                    .add_filter("sway project", &["ron"])
                    .pick_file(),
            ),
        }
    }

    /// One poll. `None` means still open; `Some(None)` means cancelled.
    fn poll(&mut self) -> Poll<Option<PathBuf>> {
        let mut cx = Context::from_waker(Waker::noop());
        self.future
            .as_mut()
            .poll(&mut cx)
            .map(|handle| handle.map(|h| h.path().to_path_buf()))
    }
}

/// Which presenter this run uses, selected once at window creation
/// (`ShellConfig::editor`) and never switched at runtime.
enum Presenter {
    Show(ShowPresenter),
    Editor(Box<EditorPresenter>),
}

/// The project currently driving the `App`: a directory (the asset root) and
/// the graph file inside it.
#[derive(Clone, Debug)]
pub struct ProjectSpec {
    pub directory: PathBuf,
    pub graph_file: String,
}

/// Builds the demo-specific Bevy `App` once the window, shared device, and
/// viewport texture exist. Called again when a different project is opened:
/// the window and the wgpu device survive, the `App` does not.
pub type AppBuilder = Box<dyn Fn(&GpuContext, &ViewportTexture, UVec2, &ProjectSpec) -> App>;

/// What to run once the window is up.
pub struct ShellConfig {
    /// Selects the window title and, below in `resumed`, which `Presenter`
    /// this run uses: `--editor` gets the real `EditorPresenter` (masonry's
    /// three-pane UI, see `sway_editor::EditorUi`); its absence gets the
    /// plain `ShowPresenter` (viewport fullscreen, no masonry).
    pub editor: bool,
    pub build_app: AppBuilder,
    pub commands: Sender<EditorEdit>,
    pub viewport_input: Sender<ViewportInput>,
    pub project: ProjectSpec,
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
    /// The file dialog in flight, if any. See `Dialog`'s docs.
    pending_dialog: Option<Dialog>,
    build_app: AppBuilder,
    project: ProjectSpec,
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

        // The toolbar's requests, then one poll of whatever dialog is open.
        // Both only exist on the editor path; the show path has no toolbar.
        let (requests, view_requests) = if let Presenter::Editor(presenter) = &mut self.presenter {
            (
                presenter.take_file_requests(),
                presenter.take_view_requests(),
            )
        } else {
            (vec![], vec![])
        };

        for request in requests {
            if self.pending_dialog.is_some() {
                // A modal dialog is already up; ignore the rest.
                break;
            }
            match request {
                FileRequest::Save => {
                    if let Err(error) = sway_document::v4::save_open_graph(self.app.world_mut()) {
                        eprintln!("save failed: {error}");
                    }
                }
                FileRequest::Open => self.pending_dialog = Some(Dialog::open()),
            }
        }

        for request in view_requests {
            match request {
                ViewRequest::ToggleCamera => {
                    let world = self.app.world_mut();
                    if let Some(mut active) =
                        world.get_resource_mut::<sway_runtime::viewport::camera::ViewportCamera>()
                    {
                        *active = match *active {
                            sway_runtime::viewport::camera::ViewportCamera::Editor => {
                                sway_runtime::viewport::camera::ViewportCamera::Scene
                            }
                            sway_runtime::viewport::camera::ViewportCamera::Scene => {
                                sway_runtime::viewport::camera::ViewportCamera::Editor
                            }
                        };
                    }
                }
            }
        }

        self.poll_dialog();

        // Keeps the loop continuous: vsync (the surface is `Fifo`) paces us,
        // not this call. This also covers `begin_frame` returning `None`
        // (occluded/timeout, handled inside the presenter's `present`):
        // asking again is how the loop notices when the surface becomes
        // presentable.
        self.window.request_redraw();
    }

    /// Advances the open dialog, if any, and rebuilds the `App` against the
    /// picked file. The window and the wgpu device survive; only the `App`
    /// is dropped (`architecture`: Reloading a project is an explicit action).
    fn poll_dialog(&mut self) {
        let Some(dialog) = &mut self.pending_dialog else {
            return;
        };
        let Poll::Ready(picked) = dialog.poll() else {
            return;
        };
        self.pending_dialog = None;

        // `None` is a cancelled dialog, which is not an error.
        let Some(path) = picked else {
            return;
        };
        let directory = path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let graph_file = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled.sway.ron".into());
        self.project = ProjectSpec {
            directory,
            graph_file,
        };
        self.rebuild_app();
    }

    /// Drops the current `App` and builds a new one for `self.project`.
    ///
    /// The window, the gpu context, the viewport texture and the masonry
    /// presenter are left alone. `set_viewport_view` re-points the new world's
    /// `ManualTextureViews` at the surviving texture.
    fn rebuild_app(&mut self) {
        let size = UVec2::new(self.viewport.width, self.viewport.height);
        let mut app = (self.build_app)(&self.gpu, &self.viewport, size, &self.project);
        app.finish();
        app.cleanup();
        sway_runtime::headless::set_viewport_view(&mut app, &self.viewport, size);
        self.app = app;
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

        let title = if config.editor {
            "sway (editor)"
        } else {
            "sway"
        };
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
                (EDITOR_VIEWPORT_SIZE.height * scale_factor)
                    .round()
                    .max(1.0) as u32,
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
            &config.project,
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
            Presenter::Editor(Box::new(EditorPresenter::new(
                &gpu,
                size,
                scale_factor,
                config.commands,
                config.viewport_input,
            )))
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
            pending_dialog: None,
            build_app: config.build_app,
            project: config.project,
        });
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
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

        if let Presenter::Editor(presenter) = &mut running.presenter
            && let Some(icon) = presenter.take_cursor()
        {
            running.window.set_cursor(icon);
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
