//! Turning capture intent into files, on the host's own clock.
//!
//! The graph publishes *intent* — which camera, where to write, and whether
//! recording is true — and this module owns the part the graph cannot: the
//! slot clock (design D5). A slot is `1/60` of show time counted from the run's
//! start, show time is wall time, and the slot index **is** the file number, so
//! a sequence played back at the capture rate matches the show's own timing
//! rather than the rate frames happened to be produced at.
//!
//! Three rules follow from "keeping up with the external clock outranks
//! completing a capture":
//!
//! - **Nothing here blocks.** Readback is asynchronous (`sway_gpu::readback`)
//!   and encoding happens on a writer thread behind a bounded channel.
//! - **Saturation drops.** A pool with no free buffer, or a writer channel
//!   with no room, costs that slot — counted, and reported when the run ends.
//! - **A slow frame repeats rather than reslots.** Crossing several slots in
//!   one frame writes this frame's pixels to every one of them, so a render
//!   rate below the capture rate costs duplicate images rather than distorted
//!   timing. A dropped slot leaves its number unused, because renumbering
//!   would move every later frame earlier in time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossbeam_channel::{Sender, TrySendError, bounded};
use sway_gpu::{ReadbackPool, ReadbackRefused};
use sway_graph::graph::NodeId;
use sway_runtime::nodes::expand_pattern;
use sway_runtime::{CameraTargets, CaptureIntents};

/// Files per second of show time. Fixed for now; design records making it an
/// inlet as the next step, and the node is already shaped so that adding one
/// changes nothing else.
pub const CAPTURE_RATE: u32 = 60;

/// How many readbacks may be in flight at once. Small on purpose: a deep pool
/// answers a disk that cannot keep up by consuming memory, where the specified
/// answer is to drop the slot and say so.
const POOL_DEPTH: usize = 4;

/// How many encoded frames may be queued for the writer thread.
const WRITER_QUEUE: usize = 8;

/// One image to encode and write.
struct WriteJob {
    path: PathBuf,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

/// Encodes and writes on a thread of its own, so neither PNG compression nor a
/// slow disk is ever on the frame loop.
struct Writer {
    jobs: Sender<WriteJob>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Writer {
    fn new() -> Self {
        let (jobs, rx) = bounded::<WriteJob>(WRITER_QUEUE);
        let handle = std::thread::Builder::new()
            .name("sway capture writer".into())
            .spawn(move || {
                for job in rx {
                    if let Err(error) = write_png(&job.path, job.width, job.height, &job.pixels) {
                        eprintln!("capture: could not write {}: {error}", job.path.display());
                    }
                }
            })
            .expect("could not start the capture writer thread");
        Self {
            jobs,
            handle: Some(handle),
        }
    }

    /// Queues a write, or reports that there was no room. Never blocks: a full
    /// queue means the disk is behind the clock, and the show does not wait
    /// for the disk.
    fn try_write(&self, job: WriteJob) -> Result<(), ()> {
        match self.jobs.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => Err(()),
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // Dropping the sender ends the thread's loop; joining then lets the
        // queued frames finish rather than losing them at exit.
        let (dead, _) = bounded(1);
        let jobs = std::mem::replace(&mut self.jobs, dead);
        drop(jobs);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Encodes `pixels` as a PNG at `path`, atomically.
///
/// Written to a temporary name in the destination's own directory and renamed
/// into place, so a failure — a full disk, a killed process — leaves no
/// partial file where a complete one is expected. Same directory because a
/// rename across filesystems is a copy, and a copy is not atomic.
pub fn write_png(path: &Path, width: u32, height: u32, pixels: &[u8]) -> std::io::Result<()> {
    let Some(directory) = path.parent() else {
        return Err(std::io::Error::other(format!(
            "\"{}\" names no directory to write into",
            path.display()
        )));
    };
    if !directory.as_os_str().is_empty() {
        std::fs::create_dir_all(directory)?;
    }

    let temporary = path.with_extension(format!(
        "{}.partial",
        path.extension()
            .map(|extension| extension.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));

    let result = (|| -> std::io::Result<()> {
        let image =
            image::RgbaImage::from_raw(width, height, pixels.to_vec()).ok_or_else(|| {
                std::io::Error::other(format!(
                    "{} bytes is not a {width}x{height} RGBA image",
                    pixels.len()
                ))
            })?;
        image
            .save_with_format(&temporary, image::ImageFormat::Png)
            .map_err(std::io::Error::other)?;
        std::fs::rename(&temporary, path)
    })();

    if result.is_err() {
        // Best effort: the point is that no partial file is left behind, and
        // there is nothing useful to do if even the cleanup fails.
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// One capture node's run: when it started, which slot comes next, and what it
/// has lost.
struct Run {
    started: Instant,
    next_slot: u64,
    dropped: u64,
    written: u64,
}

/// A readback that has been issued and the slots its pixels are owed to.
struct Pending {
    node: NodeId,
    pattern: String,
    directory: PathBuf,
    /// One or more: several when a slow frame crossed several slots at once,
    /// which is what makes a repeat a repeat rather than a reslot.
    slots: Vec<u64>,
}

/// Decides which slots a run has crossed since it was last asked.
///
/// Pure, and separated from the frame loop for exactly that reason: the
/// timeline is the part of capture that has to be right, and it is the part a
/// device-backed test cannot pin down.
///
/// Returns the slots owed a frame, and the next slot to start from. Empty when
/// no boundary has been crossed — the ordinary answer when the loop is running
/// faster than the capture rate.
fn slots_crossed(elapsed: Duration, next_slot: u64, rate: u32) -> (Vec<u64>, u64) {
    let current = (elapsed.as_secs_f64() * f64::from(rate)).floor() as u64;
    if current < next_slot {
        return (Vec::new(), next_slot);
    }
    ((next_slot..=current).collect(), current + 1)
}

/// The host's side of capture: the slot clock, the readback pool and the
/// writer thread.
pub struct CaptureDrain {
    pool: ReadbackPool,
    writer: Writer,
    runs: HashMap<NodeId, Run>,
    pending: HashMap<u64, Pending>,
    next_ticket: u64,
}

impl CaptureDrain {
    pub fn new(device: &sway_gpu::wgpu::Device, queue: &sway_gpu::wgpu::Queue) -> Self {
        Self {
            pool: ReadbackPool::new(device, queue, POOL_DEPTH),
            writer: Writer::new(),
            runs: HashMap::new(),
            pending: HashMap::new(),
            next_ticket: 0,
        }
    }

    /// One frame's worth of capture, run after `app.update()` has returned and
    /// the frame's render commands are submitted.
    ///
    /// `now` is passed in rather than read here so the caller's frame clock and
    /// the slot clock are literally the same instant.
    pub fn frame(
        &mut self,
        intents: &CaptureIntents,
        targets: &CameraTargets,
        project_directory: &Path,
        now: Instant,
    ) {
        // A run that stopped recording — or whose node went away — reports
        // what it lost and ends. Its outstanding readbacks are still written:
        // they are frames of the run that just finished.
        let recording: Vec<NodeId> = intents
            .0
            .iter()
            .filter(|intent| intent.recording)
            .map(|intent| intent.node)
            .collect();
        let ended: Vec<NodeId> = self
            .runs
            .keys()
            .copied()
            .filter(|node| !recording.contains(node))
            .collect();
        for node in ended {
            if let Some(run) = self.runs.remove(&node) {
                report_run_end(node, &run);
            }
        }

        for intent in &intents.0 {
            if !intent.recording {
                continue;
            }
            let Some(target) = targets.target(intent.camera) else {
                // The camera has no target; the runtime has already said why,
                // naming the camera.
                continue;
            };

            // Each false -> true edge begins a run whose first file is
            // numbered zero, overwriting whatever a previous run left there.
            let run = self.runs.entry(intent.node).or_insert_with(|| {
                eprintln!(
                    "capture: {} recording to {} at {}x{}",
                    intent.node,
                    project_directory.join(&intent.pattern).display(),
                    intent.resolution.x,
                    intent.resolution.y,
                );
                Run {
                    started: now,
                    next_slot: 0,
                    dropped: 0,
                    written: 0,
                }
            });

            let (slots, next_slot) = slots_crossed(
                now.saturating_duration_since(run.started),
                run.next_slot,
                CAPTURE_RATE,
            );
            run.next_slot = next_slot;
            if slots.is_empty() {
                continue;
            }

            let ticket = self.next_ticket;
            match self.pool.request(target.texture(), ticket) {
                Ok(()) => {
                    self.next_ticket += 1;
                    self.pending.insert(
                        ticket,
                        Pending {
                            node: intent.node,
                            pattern: intent.pattern.clone(),
                            directory: project_directory.to_path_buf(),
                            slots,
                        },
                    );
                }
                Err(ReadbackRefused::Saturated) | Err(ReadbackRefused::UnsupportedFormat) => {
                    // Every slot this frame owed a file loses it, and leaves
                    // its number unused.
                    run.dropped += slots.len() as u64;
                }
            }
        }

        // Whatever finished since last frame, written to every slot it owes.
        for readback in self.pool.collect() {
            let Some(pending) = self.pending.remove(&readback.tag) else {
                continue;
            };
            for slot in pending.slots {
                let Ok(name) = expand_pattern(&pending.pattern, slot) else {
                    // Already reported once by the runtime; a pattern that
                    // cannot name a frame never becomes an intent.
                    continue;
                };
                let job = WriteJob {
                    path: pending.directory.join(name),
                    width: readback.width,
                    height: readback.height,
                    pixels: readback.pixels.clone(),
                };
                let outcome = self.writer.try_write(job);
                if let Some(run) = self.runs.get_mut(&pending.node) {
                    match outcome {
                        Ok(()) => run.written += 1,
                        Err(()) => run.dropped += 1,
                    }
                }
            }
        }
    }

    /// How many readbacks are still outstanding.
    ///
    /// The frame loop never waits on this — waiting is the thing capture must
    /// not do — so it exists only for the tests, which want a run drained
    /// rather than measuring this machine's GPU latency.
    #[cfg(test)]
    pub fn outstanding(&self) -> usize {
        self.pool.in_flight()
    }

    /// Ends every run in progress, reporting what each lost. Called when the
    /// process is shutting down or the project is being replaced, so a run cut
    /// short still says how complete it was.
    pub fn finish(&mut self) {
        for (node, run) in self.runs.drain() {
            report_run_end(node, &run);
        }
    }
}

impl Drop for CaptureDrain {
    fn drop(&mut self) {
        self.finish();
    }
}

fn report_run_end(node: NodeId, run: &Run) {
    if run.dropped == 0 {
        eprintln!("capture: {node} finished, {} frames written", run.written);
    } else {
        // A recording with holes must not be mistaken for a complete one. The
        // numbering is slot-based, so the holes are findable after the fact.
        eprintln!(
            "capture: {node} finished, {} frames written, {} slots dropped",
            run.written, run.dropped
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_starts_by_capturing_slot_zero() {
        let (slots, next) = slots_crossed(Duration::ZERO, 0, 60);
        assert_eq!(slots, vec![0]);
        assert_eq!(next, 1);
    }

    #[test]
    fn a_frame_inside_the_same_slot_captures_nothing() {
        // The loop can run faster than the capture rate; that must not mean
        // more files than the timeline has room for.
        let (slots, next) = slots_crossed(Duration::from_micros(8_000), 1, 60);
        assert!(slots.is_empty());
        assert_eq!(next, 1, "and the next slot is still owed");
    }

    /// Frame times for a loop running at `fps`, each placed *inside* a frame
    /// rather than on its boundary.
    ///
    /// A boundary is exactly where binary floating point is ambiguous — 2/30
    /// of a second is 3.9999999999999996 sixtieths, not 4 — so a test clocked
    /// on boundaries measures `f64` rounding rather than the slot arithmetic.
    /// A real loop never lands exactly on one either.
    fn mid_frame_times(fps: u64, frames: u64) -> impl Iterator<Item = Duration> {
        let period = 1_000_000_000 / fps;
        (0..frames).map(move |frame| Duration::from_nanos(frame * period + period / 2))
    }

    #[test]
    fn one_second_of_show_time_holds_the_capture_rate_worth_of_slots() {
        // "The graph ticks at 120 Hz with recording true for one second of
        // show time -> about 60 files, not about 120." The tick rate never
        // enters this arithmetic at all, which is the point.
        let mut next = 0;
        let mut all = Vec::new();
        for elapsed in mid_frame_times(60, 60) {
            let (slots, after) = slots_crossed(elapsed, next, 60);
            assert_eq!(slots.len(), 1, "one frame per slot at the capture rate");
            all.extend(slots);
            next = after;
        }
        assert_eq!(all, (0..60).collect::<Vec<u64>>());
    }

    #[test]
    fn a_render_rate_below_the_capture_rate_repeats_rather_than_reslots() {
        // "Frames rendered at 30 Hz while capturing at 60: each rendered frame
        // appears in about two consecutively numbered files, and one second of
        // show time still spans about 60 numbers."
        let mut next = 0;
        let mut all = Vec::new();
        for elapsed in mid_frame_times(30, 30) {
            let (slots, after) = slots_crossed(elapsed, next, 60);
            assert!(
                slots.len() <= 2,
                "a 30 Hz loop owes at most two slots a frame, got {slots:?}"
            );
            all.extend(slots);
            next = after;
        }
        // Consecutive and gapless, spanning about 60 numbers: the numbering is
        // a timeline, so playback at 60 lasts as long as the show did.
        assert_eq!(all, (0..59).collect::<Vec<u64>>());
    }

    #[test]
    fn a_render_rate_above_the_capture_rate_does_not_write_more_files() {
        // "A fast display does not speed the show up" has a capture
        // counterpart: a loop running at 120 still owes 60 slots a second.
        let mut next = 0;
        let mut total = 0;
        for elapsed in mid_frame_times(120, 120) {
            let (slots, after) = slots_crossed(elapsed, next, 60);
            total += slots.len();
            next = after;
        }
        assert_eq!(total, 60);
    }

    #[test]
    fn a_long_stall_owes_every_slot_it_crossed_and_numbers_them_consecutively() {
        // Half a second lost in one frame: the slots it crossed still get
        // files (of the same, most recent frame), so playback timing holds.
        let (slots, next) = slots_crossed(Duration::from_millis(500), 0, 60);
        assert_eq!(slots.len(), 31, "slots 0..=30");
        assert_eq!(slots.first(), Some(&0));
        assert_eq!(slots.last(), Some(&30));
        assert_eq!(next, 31);
    }

    #[test]
    fn the_clock_never_runs_backwards() {
        // A `next_slot` already past the elapsed time (a run restarted, or a
        // clock that did not advance) owes nothing rather than a negative
        // range that would panic.
        let (slots, next) = slots_crossed(Duration::ZERO, 5, 60);
        assert!(slots.is_empty());
        assert_eq!(next, 5);
    }

    #[test]
    fn a_png_is_written_whole_or_not_at_all() {
        let directory =
            std::env::temp_dir().join(format!("sway-capture-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("frame_0000.png");

        let pixels = vec![255u8; 4 * 4 * 4];
        write_png(&path, 4, 4, &pixels).expect("a 4x4 image writes");
        assert!(path.exists());
        let decoded = image::open(&path).expect("and reads back");
        assert_eq!((decoded.width(), decoded.height()), (4, 4));

        // A byte count that is not the image's is refused, and leaves no
        // half-written file where a complete one is expected.
        let short = directory.join("frame_0001.png");
        write_png(&short, 4, 4, &pixels[..8]).expect_err("a truncated buffer is not an image");
        assert!(!short.exists(), "no partial file");
        assert!(
            !short.with_extension("png.partial").exists(),
            "and no leftover temporary"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_path_that_cannot_be_written_is_reported_and_leaves_nothing() {
        // The `app` spec's failure case for `--capture-window`: a diagnostic
        // naming the path, and no partial file. The root directory is not
        // writable by an ordinary user, so this is a real refusal rather than
        // a simulated one.
        let path = Path::new("/sway-capture-should-not-be-writable/frame.png");
        let error = write_png(path, 2, 2, &[0u8; 16]).expect_err("the root is not writable");
        assert!(!path.exists());
        // The caller prints the path alongside this; what the error itself has
        // to carry is why.
        assert!(
            !error.to_string().is_empty(),
            "the failure says something about itself"
        );
    }
}

/// The whole capture path against a real device: a graph, a camera target, the
/// readback pool and the writer thread, writing real PNGs to a temp directory.
///
/// No window and no display — which is the point. The manual verification this
/// stands in for cannot be run without a person at the machine, and the
/// properties that matter (files appear, at the camera's authored resolution,
/// numbered by slot, restarting at zero on a second run) do not need one.
#[cfg(test)]
mod run_tests {
    use super::*;
    use bevy::prelude::*;
    use sway_graph::graph::{Graph, Node, Port};
    use sway_runtime::nodes::{Camera, CameraIn, Capture, CaptureIn, protocol};

    struct Fixture {
        app: App,
        drain: CaptureDrain,
        directory: PathBuf,
        capture: NodeId,
        base: Instant,
        /// Kept alive: the app's `ManualTextureViews` holds views of it.
        _gpu: sway_gpu::GpuContext,
        _viewport: sway_gpu::ViewportTexture,
    }

    fn fixture(name: &str, resolution: UVec2) -> Fixture {
        let directory = std::env::temp_dir().join(format!(
            "sway-capture-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a temp directory");

        let gpu = sway_gpu::GpuContext::new(None);
        let size = UVec2::new(64, 64);
        let viewport = sway_gpu::ViewportTexture::new(&gpu.device, size.x, size.y);
        let mut app = sway_runtime::headless::build_app(&gpu, &viewport, size, &directory);
        app.add_plugins(sway_runtime::RuntimePlugin);
        app.finish();
        app.cleanup();

        let (camera, capture) = {
            let mut graph = app.world_mut().resource_mut::<Graph>();
            let camera = graph.insert(Node::of(Camera {
                inlets: CameraIn {
                    resolution,
                    ..Default::default()
                },
                ..Default::default()
            }));
            let capture = graph.insert(Node::of(Capture {
                inlets: CaptureIn {
                    path: "shot_####.png".into(),
                    recording: true,
                    ..Default::default()
                },
                ..Default::default()
            }));
            graph
                .connect(
                    Port::new(camera, protocol::CAMERA),
                    Port::new(capture, protocol::CAMERA),
                    0,
                )
                .expect("a camera connects to a capture node");
            (camera, capture)
        };
        let _ = camera;

        let drain = CaptureDrain::new(&gpu.device, &gpu.queue);
        Fixture {
            app,
            drain,
            directory,
            capture,
            base: Instant::now(),
            _gpu: gpu,
            _viewport: viewport,
        }
    }

    impl Fixture {
        /// One frame of the real loop: update, then drain at `at`.
        fn step(&mut self, at: Instant) {
            self.app.update();
            let intents = self.app.world().resource::<CaptureIntents>().clone();
            let targets = self.app.world().resource::<CameraTargets>();
            self.drain.frame(&intents, targets, &self.directory, at);
        }

        /// Runs `slots` capture slots and waits for each readback to land, so
        /// the test measures the numbering rather than this machine's GPU
        /// latency. A real run drops instead of waiting; nothing here changes
        /// that behaviour, it only avoids provoking it.
        fn record(&mut self, slots: u64) {
            for slot in 0..slots {
                let at = self.base + Duration::from_nanos(slot * 16_666_667 + 8_000_000);
                self.step(at);
                for _ in 0..500 {
                    if self.drain.outstanding() == 0 {
                        break;
                    }
                    self.step(at);
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }

        fn set_recording(&mut self, recording: bool) {
            self.app
                .world_mut()
                .resource_mut::<Graph>()
                .get_mut(self.capture)
                .expect("the capture node")
                .value_mut()
                .downcast_mut::<Capture>()
                .expect("a capture node")
                .inlets
                .recording = recording;
        }

        /// Every file in the directory, sorted by name.
        fn files(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.directory)
                .expect("the directory exists")
                .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
                .collect();
            names.sort();
            names
        }

        /// Waits for the writer thread to catch up, then lists the files.
        fn settled_files(&mut self, expected: usize) -> Vec<String> {
            for _ in 0..500 {
                if self.files().len() >= expected {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            self.files()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn a_run_writes_one_file_per_slot_numbered_from_zero() {
        let mut f = fixture("numbering", UVec2::new(64, 48));
        f.record(5);
        let files = f.settled_files(5);
        assert_eq!(
            files,
            vec![
                "shot_0000.png",
                "shot_0001.png",
                "shot_0002.png",
                "shot_0003.png",
                "shot_0004.png",
            ],
            "the slot index is the file number, zero-padded to the pattern's run"
        );
    }

    #[test]
    fn files_carry_the_cameras_authored_resolution_not_the_panes() {
        // The viewport texture in this fixture is 64x64; the camera is
        // authored at 96x54, and that is what has to land on disk.
        let mut f = fixture("resolution", UVec2::new(96, 54));
        f.record(2);
        f.settled_files(2);
        let image = image::open(f.directory.join("shot_0000.png")).expect("a written frame");
        assert_eq!((image.width(), image.height()), (96, 54));
    }

    #[test]
    fn a_second_run_restarts_the_numbering_and_overwrites() {
        let mut f = fixture("restart", UVec2::new(32, 32));
        f.record(3);
        assert_eq!(f.settled_files(3).len(), 3);

        // Clearing the flag ends the run and stops writing; the files already
        // written are left as they are.
        f.set_recording(false);
        let at = f.base + Duration::from_millis(100);
        f.step(at);
        f.step(at);
        assert_eq!(
            f.files().len(),
            3,
            "no further files after the flag cleared"
        );

        // Deleting them makes the overwrite observable: a second run whose
        // first frame is numbered zero recreates exactly that file.
        for name in f.files() {
            std::fs::remove_file(f.directory.join(name)).expect("removable");
        }
        assert!(f.files().is_empty());

        f.set_recording(true);
        f.base += Duration::from_millis(500);
        f.record(2);
        assert_eq!(
            f.settled_files(2),
            vec!["shot_0000.png", "shot_0001.png"],
            "the second run's first frame is numbered zero again"
        );
    }

    #[test]
    fn recording_defaults_to_off_so_opening_a_project_writes_nothing() {
        let mut f = fixture("off", UVec2::new(32, 32));
        f.set_recording(false);
        f.record(4);
        assert!(f.files().is_empty());
    }
}
