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
use std::process::ExitCode;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use bevy::app::App;
use bevy::math::UVec2;
use crossbeam_channel::Sender;
use sway_editor::{FileRequest, ViewRequest};
use sway_gpu::{
    Compositor, GpuContext, ReadbackPool, ViewportTexture, VsyncPreference, WindowSurface,
};
use sway_editor::edit::EditorEdit;
use sway_viewport_input::ViewportInput;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::capture::{CaptureDrain, write_png};
use crate::presenter::{EDITOR_VIEWPORT_SIZE, EditorPresenter, ShowPresenter};

/// The show's fixed frame rate (design D5a).
///
/// One rate for the whole process, independent of the display and independent
/// of whether anything is capturing — a rate that changed when a recording
/// started would make every timing observation depend on whether someone
/// happened to be recording.
pub const SHOW_FPS: u32 = 60;

/// Paces the frame loop against real time.
///
/// A wall-clock deadline per frame, not a count of refreshes: 144 Hz is not a
/// multiple of 60, and a nominal 60 Hz panel is usually 59.94.
///
/// A late frame is late, not two frames at once — [`Self::advance`] pushes the
/// next deadline out from *now* whenever the last one has already passed,
/// rather than letting a backlog accumulate into a burst of catch-up frames.
struct FramePace {
    interval: Duration,
    next: Option<Instant>,
}

impl FramePace {
    fn new(fps: u32) -> Self {
        Self {
            interval: Duration::from_secs_f64(1.0 / f64::from(fps.max(1))),
            next: None,
        }
    }

    /// How long to wait before the next frame may start, if at all.
    fn wait(&self, now: Instant) -> Option<Duration> {
        let next = self.next?;
        (next > now).then(|| next - now)
    }

    /// Records that a frame started at `now`, and sets the next deadline.
    fn advance(&mut self, now: Instant) {
        let next = match self.next {
            // On time: keep the phase, so the rate does not drift with frame
            // duration.
            Some(next) if next + self.interval > now => next + self.interval,
            // Late (or the first frame): start the next interval from here.
            // Anything else would render ahead to make up the difference.
            _ => now + self.interval,
        };
        self.next = Some(next);
    }
}

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
    /// Whether to stop waiting for the display's refresh (`--no-vsync`).
    pub no_vsync: bool,
    /// Where to write one image of the whole window before exiting, if the
    /// run was asked for one (`--capture-window`).
    pub capture_window: Option<PathBuf>,
}

/// The whole-window capture in progress, if this run was asked for one.
///
/// It settles by observation rather than by a frame count (design D8): the
/// image is written once every asset has resolved, the graph has projected at
/// least once, and two consecutive readbacks of the window are byte-identical.
/// That last condition is what defends against the asynchronous
/// pipeline-compilation trap `headless.rs` documents — the wrong-clear-colour
/// frames it found are stable frame to frame only *after* the upscaling
/// pipeline is ready.
struct WindowCapture {
    path: PathBuf,
    pool: ReadbackPool,
    previous: Option<Vec<u8>>,
    frames: u32,
    /// The readback ticket last issued, so a completed one can be matched to
    /// the frame it came from.
    next_ticket: u64,
}

/// A generous bound on how long a capture may wait to settle, in frames — in
/// the spirit of `headless.rs`'s own `MAX_UPDATES = 300`, which was measured
/// against a cold shader cache. Hitting it is a diagnostic and a failure exit,
/// never a written file.
const CAPTURE_FRAME_CAP: u32 = 900;

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
    /// The show's fixed rate, paced against real time.
    pace: FramePace,
    /// The slot clock, the readback pool and the writer thread.
    capture: CaptureDrain,
    /// The one-shot whole-window capture, if this run was asked for one.
    window_capture: Option<WindowCapture>,
    /// Set once the run has decided how it ends. `None` while it is still
    /// running; the event loop exits as soon as it is `Some`.
    outcome: Option<ExitCode>,
}

impl Running {
    fn redraw(&mut self) {
        // The show's own pace, ahead of everything else: a wall-clock deadline
        // rather than whatever the display happens to deliver. `Fifo` still
        // blocks underneath by default, so this is a floor unless `--no-vsync`
        // lifted it (design D5a).
        let now = Instant::now();
        if let Some(wait) = self.pace.wait(now) {
            std::thread::sleep(wait);
        }
        let frame_started = Instant::now();
        self.pace.advance(frame_started);

        // The whole-window capture reads back the *presented* surface texture,
        // which only exists inside a frame — so the request is handed down to
        // the presenter rather than made here. Only once the scene has
        // settled enough to be worth reading (design D8).
        let scene_ready = self.window_capture.is_some() && scene_has_been_projected(&self.app);
        let window_readback: Option<(u64, &mut ReadbackPool)> = if scene_ready {
            self.window_capture.as_mut().map(|capture| {
                let ticket = capture.next_ticket;
                capture.next_ticket += 1;
                (ticket, &mut capture.pool)
            })
        } else {
            None
        };

        match &mut self.presenter {
            Presenter::Show(presenter) => presenter.present(
                &mut self.app,
                &self.gpu,
                &self.surface,
                &mut self.compositor,
                window_readback,
            ),
            Presenter::Editor(presenter) => presenter.present(
                &mut self.app,
                &self.gpu,
                &self.surface,
                &mut self.viewport,
                &mut self.compositor,
                window_readback,
            ),
        }

        // The capture drain, after `app.update()` has returned and the frame's
        // render commands are submitted — which is the one place in the
        // process where that is true. A main-world Bevy system reading a
        // target back would read an indeterminate frame, because with
        // pipelined rendering frame N's render runs alongside frame N+1's main
        // schedule.
        {
            let world = self.app.world();
            let intents = world.resource::<sway_runtime::CaptureIntents>().clone();
            let targets = world.resource::<sway_runtime::CameraTargets>();
            self.capture
                .frame(&intents, targets, &self.project.directory, frame_started);
        }

        self.poll_window_capture();

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
                // An index into the list the toolbar was last given, which the
                // presenter built and can resolve — the editor deliberately
                // knows nothing about what a camera is.
                ViewRequest::SelectCamera(index) => {
                    let Presenter::Editor(presenter) = &self.presenter else {
                        continue;
                    };
                    let Some(choice) = presenter.camera_choice(index) else {
                        continue;
                    };
                    let world = self.app.world_mut();
                    if let Some(mut active) =
                        world.get_resource_mut::<sway_editor_viewport::ViewportCamera>()
                    {
                        // Never write an equal value: this is editor state
                        // read every frame.
                        if *active != choice {
                            *active = choice;
                        }
                    }
                }
            }
        }

        self.poll_dialog();

        // Keeps the loop continuous. The rate comes from `FramePace` above
        // (and, by default, from the vsync'd present underneath it), not from
        // this call. This also covers `begin_frame` returning `None`
        // (occluded/timeout, handled inside the presenter's `present`):
        // asking again is how the loop notices when the surface becomes
        // presentable.
        self.window.request_redraw();
    }

    /// Advances the whole-window capture, if this run has one.
    ///
    /// Writes the file and sets the run's outcome the moment two consecutive
    /// readbacks agree; gives up with a diagnostic and a failure outcome once
    /// the frame cap is reached, leaving no file behind either way.
    fn poll_window_capture(&mut self) {
        if self.outcome.is_some() {
            // Already decided. winit can deliver another redraw between
            // `exit()` and the loop actually ending, and reporting the same
            // failure twice reads as two failures.
            return;
        }
        let Some(capture) = &mut self.window_capture else {
            return;
        };
        capture.frames += 1;

        for readback in capture.pool.collect() {
            let settled = capture.previous.as_deref() == Some(readback.pixels.as_slice());
            capture.previous = Some(readback.pixels.clone());
            if !settled {
                continue;
            }

            self.outcome = Some(
                match write_png(
                    &capture.path,
                    readback.width,
                    readback.height,
                    &readback.pixels,
                ) {
                    Ok(()) => {
                        eprintln!(
                            "captured the window ({}x{}) to {}",
                            readback.width,
                            readback.height,
                            capture.path.display()
                        );
                        ExitCode::SUCCESS
                    }
                    Err(error) => {
                        eprintln!(
                            "could not write the window capture to {}: {error}",
                            capture.path.display()
                        );
                        ExitCode::FAILURE
                    }
                },
            );
            return;
        }

        if capture.frames >= CAPTURE_FRAME_CAP {
            eprintln!(
                "the window did not settle within {CAPTURE_FRAME_CAP} frames, so nothing was \
                 written to {}",
                capture.path.display()
            );
            self.outcome = Some(ExitCode::FAILURE);
        }
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

/// Whether the scene is far enough along to be worth reading the window back.
///
/// Every asset the project references has loaded and the graph has been
/// projected at least once (`architecture`: evaluation waits for assets). Not
/// sufficient on its own — the frame may still be a placeholder cleared to the
/// wrong colour while a pipeline compiles — which is why the caller also
/// requires two consecutive readbacks to agree.
fn scene_has_been_projected(app: &App) -> bool {
    app.world()
        .get_resource::<crate::ProjectedFrames>()
        .is_some_and(|projected| projected.0 > 0)
}

struct Shell {
    config: Option<ShellConfig>,
    running: Option<Running>,
    /// How the run ended, once it has. Read by [`run`] after the event loop
    /// returns, so the exit status alone distinguishes success from failure.
    outcome: ExitCode,
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

        let vsync = if config.no_vsync {
            VsyncPreference::DontWait
        } else {
            VsyncPreference::Wait
        };
        let surface = WindowSurface::new(
            &gpu.instance,
            &gpu.device,
            &gpu.adapter,
            window.clone(),
            vsync,
        );
        if config.no_vsync && surface.present_mode() == sway_gpu::wgpu::PresentMode::Fifo {
            // The surface offers neither `Mailbox` nor `Immediate`. The show
            // starts anyway, waiting for the refresh as it does by default —
            // failing to start over a presentation preference would be worse.
            eprintln!(
                "--no-vsync could not be honoured: this surface only offers Fifo, so the show \
                 still waits for the display's refresh"
            );
        }

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

        let capture = CaptureDrain::new(&gpu.device, &gpu.queue);
        let window_capture = config.capture_window.map(|path| WindowCapture {
            path,
            // Two buffers: this compares consecutive frames, so exactly two
            // ever matter, and a deeper pool would only delay the comparison.
            pool: ReadbackPool::new(&gpu.device, &gpu.queue, 2),
            previous: None,
            frames: 0,
            next_ticket: 0,
        });
        if window_capture.is_some() && !surface.readable() {
            eprintln!(
                "this surface cannot be read back, so the window cannot be captured; nothing \
                 was written"
            );
            self.outcome = ExitCode::FAILURE;
            event_loop.exit();
            return;
        }

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
            pace: FramePace::new(SHOW_FPS),
            capture,
            window_capture,
            outcome: None,
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
            WindowEvent::RedrawRequested => {
                running.redraw();
                // A one-shot capture ends the run the moment it has decided.
                if let Some(outcome) = running.outcome.take() {
                    // Every run in progress reports what it lost before the
                    // process goes; the writer thread drains on drop.
                    running.capture.finish();
                    self.outcome = outcome;
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }
}

/// Runs the shell. Blocks until the window is closed or a one-shot capture
/// finishes.
///
/// Returns the process's exit status, which alone distinguishes a capture that
/// worked from one that did not (`app`: "the exit status alone distinguishes
/// the two").
pub fn run(config: ShellConfig) -> ExitCode {
    let event_loop = EventLoop::new().expect("could not create the winit event loop");
    let mut shell = Shell {
        config: Some(config),
        running: None,
        outcome: ExitCode::SUCCESS,
    };
    event_loop
        .run_app(&mut shell)
        .expect("shell event loop exited with an error");
    // Dropping the running state ends any capture run and joins the writer
    // thread, so queued frames are on disk before the process exits.
    drop(shell.running.take());
    shell.outcome
}

#[cfg(test)]
mod pace_tests {
    use super::*;

    #[test]
    fn the_first_frame_does_not_wait() {
        let pace = FramePace::new(SHOW_FPS);
        assert_eq!(pace.wait(Instant::now()), None);
    }

    #[test]
    fn a_frame_that_finishes_early_waits_out_the_rest_of_the_interval() {
        // A 144 Hz display must not make the show render at 144: the deadline
        // is the show's, not the panel's.
        let mut pace = FramePace::new(SHOW_FPS);
        let start = Instant::now();
        pace.advance(start);

        let early = start + Duration::from_millis(7);
        let wait = pace.wait(early).expect("still inside the interval");
        assert!(
            wait > Duration::from_millis(9) && wait < Duration::from_millis(10),
            "expected roughly 9.7ms left of a 16.7ms frame, got {wait:?}"
        );
    }

    #[test]
    fn an_on_time_frame_keeps_the_phase_rather_than_drifting() {
        // Counting from the deadline, not from when the frame happened to
        // start, is what keeps 60 frames inside a second instead of 59.
        let mut pace = FramePace::new(SHOW_FPS);
        let start = Instant::now();
        pace.advance(start);
        let first_deadline = pace.next.expect("set");

        // A frame that started a hair after its deadline, but well inside the
        // next interval.
        pace.advance(first_deadline + Duration::from_micros(200));
        assert_eq!(
            pace.next,
            Some(first_deadline + pace.interval),
            "the next deadline is one interval on from the last, not from now"
        );
    }

    #[test]
    fn a_late_frame_is_not_made_up_for() {
        // "One frame takes longer than the fixed interval to produce -> the
        // frame after it is not rendered early to compensate." Rendering ahead
        // would turn one slow frame into a burst.
        let mut pace = FramePace::new(SHOW_FPS);
        let start = Instant::now();
        pace.advance(start);

        let very_late = start + Duration::from_millis(100);
        pace.advance(very_late);
        assert_eq!(
            pace.next,
            Some(very_late + pace.interval),
            "the next deadline starts from now, so no catch-up frames are owed"
        );
        assert_eq!(
            pace.wait(very_late),
            Some(pace.interval),
            "and the frame after it still waits a full interval"
        );
    }

    #[test]
    fn the_interval_is_the_show_rate_not_a_refresh_count() {
        let pace = FramePace::new(SHOW_FPS);
        assert!(
            (pace.interval.as_secs_f64() - 1.0 / 60.0).abs() < 1e-9,
            "got {:?}",
            pace.interval
        );
    }
}
