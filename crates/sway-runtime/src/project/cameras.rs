//! The camera projector: one render target per camera, and what consumes it.
//!
//! Every camera in the world renders into a target of its own, sized by that
//! camera's authored `resolution` (design D1). Two cameras never share a
//! target, and no target is ever resized by the window or by an editor pane —
//! which is what makes a document's framing authored rather than incidental.
//!
//! Targets are **host-owned wgpu textures**, not Bevy `Image`s: the compositor
//! samples textures the host owns, and the texture behind a `Handle<Image>`
//! lives in the render world where the host cannot reach it. Each target is
//! registered in `ManualTextureViews` under a handle of its own, and
//! `headless::retarget_cameras` points each camera at its own handle.
//!
//! Allocation is **lazy**: a camera nothing consumes costs no VRAM. Three
//! things consume one — an `Output` node, a `Capture` node, and the editor
//! previewing it ([`EditorCameraPreview`]) — and the first two ask for the
//! authored resolution while the third asks only for the pane's pixels
//! (design D4). A camera consumed by both is allocated at the authored
//! resolution and the preview samples it down.
//!
//! What this pass publishes is what the host reads back out: which camera to
//! present ([`PresentedCamera`]) and what each capture node intends
//! ([`CaptureIntents`]). Neither is a value the graph carries — a camera
//! connection carries identity only — so the resource is how the frame loop
//! learns what the document asked for.

use bevy::camera::{ManualTextureViewHandle, RenderTarget};
use bevy::math::UVec2;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::renderer::RenderDevice;
use bevy::render::texture::{ManualTextureView, ManualTextureViews};
use sway_gpu::textures::{CameraTarget, TargetError};
use sway_graph::graph::{Graph, NodeId};

use crate::headless::VIEWPORT_HANDLE;
use crate::nodes::capture::{Capture, expand_pattern};
use crate::nodes::output::Output;
use crate::nodes::postprocess::{ColorGrade, DepthOfField, FilmGrain, is_postprocess};
use crate::nodes::protocol;
use crate::nodes::scene::Camera;
use crate::project::source_of;

/// The editor's claim on a camera: which one it is previewing, and how many
/// pixels the preview is worth drawing with.
///
/// Absent from a show build entirely. It lives here rather than in the editor
/// because the allocator has to read it and dependencies run host → domain:
/// the editor writes it, this pass reads it. `size` is already fitted to the
/// camera's aspect by the editor (the letterbox is the editor's arithmetic,
/// not the runtime's) and is in physical pixels.
///
/// A previewed camera the graph also consumes is allocated at its authored
/// resolution regardless, and the preview samples that target down.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorCameraPreview {
    pub node: Option<NodeId>,
    pub size: UVec2,
}

/// What the host must present: the camera the `Output` node names, or nothing.
///
/// `None` is a legitimate, specified state — a document with no output node,
/// or one with nothing connected, presents nothing rather than the runtime
/// choosing a camera on the author's behalf.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentedCamera(pub Option<PresentedTarget>);

/// The camera an `Output` node named, once it has a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentedTarget {
    /// The camera node, for looking its target up in [`CameraTargets`].
    pub node: NodeId,
    /// Where Bevy renders it, for anything that reasons about the render side.
    pub handle: ManualTextureViewHandle,
    /// The size that target actually is — the authored resolution, except
    /// where the editor is the only consumer and asked for pane pixels.
    pub resolution: UVec2,
}

/// What one `Capture` node intends this frame.
///
/// Intent, not action: the node publishes at tick rate and the host's frame
/// loop owns the slot clock that decides which slots become files (design D5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureIntent {
    /// The capture node itself, so a diagnostic can name it.
    pub node: NodeId,
    /// The camera it is connected to, for looking up the target to read back.
    pub camera: NodeId,
    /// The authored path pattern, project-relative and already checked to
    /// contain a frame-number run — expanding it per slot is the host's job,
    /// because the slot index is the host's to count.
    pub pattern: String,
    pub recording: bool,
    /// The camera's target size, so the host knows what it is about to write.
    pub resolution: UVec2,
}

/// Every capture node's intent, rebuilt each pass.
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct CaptureIntents(pub Vec<CaptureIntent>);

/// One camera's allocated target.
struct Allocation {
    target: CameraTarget,
    handle: ManualTextureViewHandle,
    size: UVec2,
}

/// Which camera owns which render target, and which handle it is registered
/// under.
///
/// The host reads this to get the view it composites; the runtime reads it to
/// point cameras at their targets.
#[derive(Resource, Default)]
pub struct CameraTargets {
    allocations: HashMap<NodeId, Allocation>,
    /// Handles released by a deleted or resized camera, reissued before a new
    /// one is minted so a long session does not walk the handle space.
    free_handles: Vec<ManualTextureViewHandle>,
    /// `VIEWPORT_HANDLE` is 0 and belongs to the editor camera, so camera
    /// handles start above it.
    next_handle: u32,
}

impl CameraTargets {
    /// The target a camera renders into, if it has one.
    pub fn target(&self, node: NodeId) -> Option<&CameraTarget> {
        self.allocations.get(&node).map(|entry| &entry.target)
    }

    /// The `ManualTextureViews` handle a camera's target is registered under.
    pub fn handle(&self, node: NodeId) -> Option<ManualTextureViewHandle> {
        self.allocations.get(&node).map(|entry| entry.handle)
    }

    /// How many targets are allocated. A camera nothing consumes is not one
    /// of them.
    pub fn len(&self) -> usize {
        self.allocations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.allocations.is_empty()
    }

    fn claim_handle(&mut self) -> ManualTextureViewHandle {
        if let Some(handle) = self.free_handles.pop() {
            return handle;
        }
        // `VIEWPORT_HANDLE` is `ManualTextureViewHandle(0)`.
        self.next_handle += 1;
        ManualTextureViewHandle(self.next_handle)
    }
}

/// What has already been said about a node, so nothing is said twice.
///
/// A diagnostic that repeated every frame would bury every other message in
/// the log within a second, which is why every spec requirement here says
/// "once". Forgetting a node when its problem clears is what makes a *second*
/// mistake reportable.
#[derive(Resource, Default)]
pub struct CameraDiagnostics(HashSet<(NodeId, Complaint)>);

// `Complaint` stays private: what is reported is a diagnostic, not an API.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Complaint {
    ZeroResolution,
    TooLarge,
    OutputWithNoCamera,
    CaptureWithNoCamera,
    CaptureWithNoPath,
    CaptureWithBadPath,
    PostprocessWithNoSource,
    DepthOfFieldNotOnCamera,
}

impl CameraDiagnostics {
    /// True the first time a complaint is raised about a node, and false
    /// while it keeps being raised.
    fn first_time(&mut self, node: NodeId, complaint: Complaint) -> bool {
        self.0.insert((node, complaint))
    }

    /// Forgets a complaint, so it can be reported again if it recurs.
    fn cleared(&mut self, node: NodeId, complaint: Complaint) {
        self.0.remove(&(node, complaint));
    }
}

/// The camera at the start of a camera-target chain, if this node is a
/// camera or a post-process node that eventually feeds from one.
///
/// Follows each post-process node's `source` inlet. A cycle, a missing
/// source, or a node that is neither a camera nor an effect yields `None`.
/// This walk does not care whether the chain can *produce* (a `DepthOfField`
/// wired after a colour pass still names a camera); allocation uses
/// [`producing_chain`] for that.
pub fn source_camera(graph: &Graph, producer: NodeId) -> Option<NodeId> {
    let mut current = producer;
    let mut seen = HashSet::<NodeId>::new();
    loop {
        if !seen.insert(current) {
            return None;
        }
        let value = graph.get(current)?.value();
        if value.downcast_ref::<Camera>().is_some() {
            return Some(current);
        }
        if is_postprocess(value) {
            current = source_of(graph, current, protocol::SOURCE)?;
            continue;
        }
        return None;
    }
}

/// The chain from `producer` back to its camera, if every step can produce a
/// target. `DepthOfField` only produces when its immediate source is a
/// `Camera`; an effect with no source produces nothing.
pub(crate) fn producing_chain(graph: &Graph, producer: NodeId) -> Option<Vec<NodeId>> {
    let mut chain = Vec::new();
    let mut current = producer;
    let mut seen = HashSet::<NodeId>::new();
    loop {
        if !seen.insert(current) {
            return None;
        }
        let value = graph.get(current)?.value();
        if value.downcast_ref::<Camera>().is_some() {
            chain.push(current);
            return Some(chain);
        }
        if value.downcast_ref::<DepthOfField>().is_some() {
            let src = source_of(graph, current, protocol::SOURCE)?;
            if graph.get(src)?.value().downcast_ref::<Camera>().is_none() {
                return None;
            }
            chain.push(current);
            current = src;
            continue;
        }
        if value.downcast_ref::<ColorGrade>().is_some()
            || value.downcast_ref::<FilmGrain>().is_some()
        {
            let src = source_of(graph, current, protocol::SOURCE)?;
            chain.push(current);
            current = src;
            continue;
        }
        return None;
    }
}

/// Which camera-target producers something consumes this frame, and how big
/// each one's target should be.
///
/// A graph consumer needs the authored resolution; the editor previewing a
/// producer needs only the pane's pixels. A camera both consume is allocated
/// at the authored resolution, because the pixels the graph asked for are the
/// ones that have to exist (design D4). Each consumed effect shares its
/// source camera's size.
fn desired_sizes(graph: &Graph, preview: Option<&EditorCameraPreview>) -> HashMap<NodeId, UVec2> {
    let mut desired: HashMap<NodeId, UVec2> = HashMap::default();

    for (id, node) in graph.iter() {
        let consumes = node.value().downcast_ref::<Output>().is_some()
            || node.value().downcast_ref::<Capture>().is_some();
        if !consumes {
            continue;
        }
        let Some(producer) = source_of(graph, id, protocol::CAMERA) else {
            continue;
        };
        let Some(chain) = producing_chain(graph, producer) else {
            continue;
        };
        let camera = *chain.last().expect("a producing chain ends on a camera");
        let Some(resolution) = authored_resolution(graph, camera) else {
            continue;
        };
        for node in chain {
            desired.insert(node, resolution);
        }
    }

    // The preview claims only what no graph consumer already claimed: the
    // pixels a capture asked for are the ones that have to exist, and a
    // preview-sized target would be fewer of them (design D4). An effect
    // whose source camera is already claimed takes that camera's size.
    if let Some(preview) = preview
        && let Some(producer) = preview.node
        && let Some(chain) = producing_chain(graph, producer)
    {
        let camera = *chain.last().expect("a producing chain ends on a camera");
        let size = desired.get(&camera).copied().unwrap_or(preview.size);
        for node in chain {
            desired.entry(node).or_insert(size);
        }
    }

    desired
}

/// A camera node's authored resolution, or `None` if that node is not a
/// camera at all.
fn authored_resolution(graph: &Graph, node: NodeId) -> Option<UVec2> {
    Some(
        graph
            .get(node)?
            .value()
            .downcast_ref::<Camera>()?
            .inlets
            .resolution,
    )
}

/// Once-only diagnostics for post-process nodes that cannot produce a target.
fn report_postprocess_diagnostics(graph: &Graph, diagnostics: &mut CameraDiagnostics) {
    for (id, node) in graph.iter() {
        let value = node.value();
        if value.downcast_ref::<DepthOfField>().is_some() {
            match source_of(graph, id, protocol::SOURCE) {
                None => {
                    if diagnostics.first_time(id, Complaint::PostprocessWithNoSource) {
                        warn!(
                            "post-process node {id} has no source connected, so it produces no frames"
                        );
                    }
                    diagnostics.cleared(id, Complaint::DepthOfFieldNotOnCamera);
                }
                Some(src) => {
                    diagnostics.cleared(id, Complaint::PostprocessWithNoSource);
                    let on_camera = graph
                        .get(src)
                        .is_some_and(|n| n.value().downcast_ref::<Camera>().is_some());
                    if on_camera {
                        diagnostics.cleared(id, Complaint::DepthOfFieldNotOnCamera);
                    } else if diagnostics.first_time(id, Complaint::DepthOfFieldNotOnCamera) {
                        warn!(
                            "depth-of-field node {id} is not wired to a camera, so it produces no frames"
                        );
                    }
                }
            }
            continue;
        }
        if is_postprocess(value) {
            if source_of(graph, id, protocol::SOURCE).is_none() {
                if diagnostics.first_time(id, Complaint::PostprocessWithNoSource) {
                    warn!(
                        "post-process node {id} has no source connected, so it produces no frames"
                    );
                }
            } else {
                diagnostics.cleared(id, Complaint::PostprocessWithNoSource);
            }
        }
    }
}

/// Allocates, resizes and releases camera targets, and registers each in
/// `ManualTextureViews`.
///
/// Runs before `headless::retarget_cameras`, which is what actually points a
/// camera at the handle allocated here.
pub fn allocate_camera_targets(
    graph: Res<Graph>,
    device: Res<RenderDevice>,
    preview: Option<Res<EditorCameraPreview>>,
    mut targets: ResMut<CameraTargets>,
    mut views: ResMut<ManualTextureViews>,
    mut diagnostics: ResMut<CameraDiagnostics>,
) {
    let desired = desired_sizes(&graph, preview.as_deref());
    report_postprocess_diagnostics(&graph, &mut diagnostics);

    // Release first, so a resized camera's handle is available to be reissued
    // to its own replacement rather than growing the handle space every edit.
    let stale: Vec<NodeId> = targets
        .allocations
        .iter()
        .filter(|(node, entry)| desired.get(*node) != Some(&entry.size))
        .map(|(node, _)| *node)
        .collect();
    for node in stale {
        if let Some(entry) = targets.allocations.remove(&node) {
            views.remove(&entry.handle);
            targets.free_handles.push(entry.handle);
        }
    }

    for (node, size) in &desired {
        if targets.allocations.contains_key(node) {
            continue;
        }
        match CameraTarget::new(device.wgpu_device(), size.x, size.y) {
            Ok(target) => {
                diagnostics.cleared(*node, Complaint::ZeroResolution);
                diagnostics.cleared(*node, Complaint::TooLarge);
                let handle = targets.claim_handle();
                // Bevy writes through the sRGB view, so `view_format` must be
                // the sRGB one — the same pairing `set_viewport_view` makes.
                views.insert(
                    handle,
                    ManualTextureView {
                        texture_view: target.bevy_view.clone().into(),
                        size: *size,
                        view_format: TextureFormat::Rgba8UnormSrgb,
                    },
                );
                targets.allocations.insert(
                    *node,
                    Allocation {
                        target,
                        handle,
                        size: *size,
                    },
                );
            }
            // No target, so the camera renders nothing — never a target of
            // some other size, which would put a differently-resolved image
            // on stage without saying so.
            Err(error) => {
                let complaint = match error {
                    TargetError::ZeroResolution { .. } => Complaint::ZeroResolution,
                    TargetError::TooLarge { .. } => Complaint::TooLarge,
                };
                if diagnostics.first_time(*node, complaint) {
                    warn!("camera {node} renders nothing: {error}");
                }
            }
        }
    }
}

/// Publishes [`PresentedCamera`] and [`CaptureIntents`] from the graph.
///
/// Runs after [`allocate_camera_targets`], so a camera named here already has
/// the target the host is about to read.
pub fn publish_camera_consumers(
    graph: Res<Graph>,
    targets: Res<CameraTargets>,
    mut presented: ResMut<PresentedCamera>,
    mut intents: ResMut<CaptureIntents>,
    mut diagnostics: ResMut<CameraDiagnostics>,
) {
    let mut next_presented = None;
    let mut next_intents = Vec::new();

    for (id, node) in graph.iter() {
        if node.value().downcast_ref::<Output>().is_some() {
            match source_of(&graph, id, protocol::CAMERA) {
                Some(camera) => {
                    diagnostics.cleared(id, Complaint::OutputWithNoCamera);
                    if let (Some(handle), Some(target)) =
                        (targets.handle(camera), targets.target(camera))
                    {
                        next_presented = Some(PresentedTarget {
                            node: camera,
                            handle,
                            resolution: UVec2::new(target.width, target.height),
                        });
                    }
                }
                None => {
                    // The migration depends on this message: a document that
                    // filled the window before this change now presents
                    // nothing until its camera is wired here.
                    if diagnostics.first_time(id, Complaint::OutputWithNoCamera) {
                        warn!(
                            "output node {id} has no camera target connected, so nothing is presented — \
                             connect a camera-target producer's `camera` outlet to it"
                        );
                    }
                }
            }
            continue;
        }

        let Some(capture) = node.value().downcast_ref::<Capture>() else {
            continue;
        };

        let Some(camera) = source_of(&graph, id, protocol::CAMERA) else {
            if diagnostics.first_time(id, Complaint::CaptureWithNoCamera) {
                warn!("capture node {id} has no camera target connected, so it writes nothing");
            }
            continue;
        };
        diagnostics.cleared(id, Complaint::CaptureWithNoCamera);

        if capture.inlets.path.is_empty() {
            if diagnostics.first_time(id, Complaint::CaptureWithNoPath) {
                warn!("capture node {id} has an empty path, so it writes nothing");
            }
            continue;
        }
        diagnostics.cleared(id, Complaint::CaptureWithNoPath);

        // Checked once here rather than per slot, so a pattern that can never
        // name a frame is reported before a run starts rather than sixty
        // times a second during one.
        if let Err(error) = expand_pattern(&capture.inlets.path, 0) {
            if diagnostics.first_time(id, Complaint::CaptureWithBadPath) {
                warn!("capture node {id} writes nothing: {error}");
            }
            continue;
        }
        diagnostics.cleared(id, Complaint::CaptureWithBadPath);

        let Some(target) = targets.target(camera) else {
            // The camera has no target — already reported by the allocator,
            // which names the camera and the reason.
            continue;
        };

        next_intents.push(CaptureIntent {
            node: id,
            camera,
            pattern: capture.inlets.path.clone(),
            recording: capture.inlets.recording,
            resolution: UVec2::new(target.width, target.height),
        });
    }

    // Never write an equal value (architecture §7): both resources are read
    // by the host every frame and a needless write dirties them.
    if presented.0 != next_presented {
        presented.0 = next_presented;
    }
    if intents.0 != next_intents {
        intents.0 = next_intents;
    }
}

/// Points every camera at the texture it renders into.
///
/// Three cases, and the third is why this is one system rather than two:
/// a camera the graph produced renders into its own target; a camera the graph
/// produced that has *no* target renders nowhere; anything else — the editor's
/// own camera, the gizmo overlay camera — renders into the viewport texture,
/// which is sized by the pane rather than by the graph.
///
/// "Renders nowhere" is `RenderTarget::Window`, whose primary window does not
/// exist in this process: Bevy cannot resolve it, so the camera is skipped
/// during extraction and neither draws nor clears. That is exactly the
/// specified behaviour for a camera whose target could not be produced, and
/// it is reached without a second system writing `Camera::is_active` behind
/// the editor's back.
pub fn retarget_cameras(
    map: Option<Res<crate::project::NodeEntities>>,
    targets: Option<Res<CameraTargets>>,
    mut cameras: Query<(Entity, &mut RenderTarget), With<Camera3d>>,
) {
    for (entity, mut target) in &mut cameras {
        let node = map.as_ref().and_then(|map| map.node(entity));
        let desired = match node {
            Some(node) => match targets.as_ref().and_then(|targets| targets.handle(node)) {
                Some(handle) => RenderTarget::TextureView(handle),
                None => RenderTarget::Window(bevy::window::WindowRef::Primary),
            },
            None => RenderTarget::TextureView(VIEWPORT_HANDLE),
        };

        // Neither `RenderTarget` nor `WindowRef` derives `PartialEq`, so
        // idempotence is checked by matching the variant and, where it has
        // one, its handle — rather than by comparing whole targets.
        let settled = match (&*target, &desired) {
            (RenderTarget::TextureView(have), RenderTarget::TextureView(want)) => have == want,
            (
                RenderTarget::Window(bevy::window::WindowRef::Primary),
                RenderTarget::Window(bevy::window::WindowRef::Primary),
            ) => true,
            _ => false,
        };
        if !settled {
            *target = desired;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::capture::CaptureIn;
    use crate::nodes::scene::CameraIn;
    use bevy::reflect::{Reflect, TypePath};
    use sway_graph::graph::{Node, Port};

    fn insert<T: Reflect + TypePath>(graph: &mut Graph, value: T) -> NodeId {
        graph.insert(Node::of(value))
    }

    fn camera(resolution: UVec2) -> Camera {
        Camera {
            inlets: CameraIn {
                resolution,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn connect(graph: &mut Graph, from: NodeId, to: NodeId) {
        graph
            .connect(
                Port::new(from, protocol::CAMERA),
                Port::new(to, protocol::CAMERA),
                0,
            )
            .expect("a camera connects to a consumer");
    }

    fn feed(graph: &mut Graph, from: NodeId, to: NodeId) {
        graph
            .connect(
                Port::new(from, protocol::CAMERA),
                Port::new(to, protocol::SOURCE),
                0,
            )
            .expect("a producer feeds an effect");
    }

    #[test]
    fn a_camera_nothing_consumes_is_not_allocated() {
        // Lazy allocation is what keeps a document with four 4K cameras from
        // costing four 4K targets before anything looks at them.
        let mut graph = Graph::default();
        insert(&mut graph, camera(UVec2::new(1920, 1080)));
        assert!(desired_sizes(&graph, None).is_empty());
    }

    #[test]
    fn each_consumed_camera_asks_for_its_own_authored_size() {
        let mut graph = Graph::default();
        let cam_a = insert(&mut graph, camera(UVec2::new(1920, 1080)));
        let cam_b = insert(&mut graph, camera(UVec2::new(512, 512)));
        let output = insert(&mut graph, Output::default());
        let capture = insert(&mut graph, Capture::default());
        connect(&mut graph, cam_a, output);
        connect(&mut graph, cam_b, capture);

        let desired = desired_sizes(&graph, None);
        assert_eq!(desired.get(&cam_a), Some(&UVec2::new(1920, 1080)));
        assert_eq!(desired.get(&cam_b), Some(&UVec2::new(512, 512)));
        assert_eq!(desired.len(), 2, "neither one's size affects the other");
    }

    #[test]
    fn a_preview_only_camera_asks_for_the_panes_pixels_not_the_authored_ones() {
        // "The preview costs the pane's pixels, not the camera's" — a 4K
        // camera previewed in a 640x360 pane must not allocate 4K.
        let mut graph = Graph::default();
        let cam = insert(&mut graph, camera(UVec2::new(3840, 2160)));
        let preview = EditorCameraPreview {
            node: Some(cam),
            size: UVec2::new(640, 360),
        };

        let desired = desired_sizes(&graph, Some(&preview));
        assert_eq!(desired.get(&cam), Some(&UVec2::new(640, 360)));
    }

    #[test]
    fn a_camera_the_graph_also_consumes_keeps_its_authored_size_while_previewed() {
        // The pixels a capture asked for are the ones that have to exist; the
        // preview samples that one target down (design D4).
        let mut graph = Graph::default();
        let cam = insert(&mut graph, camera(UVec2::new(1920, 1080)));
        let capture = insert(&mut graph, Capture::default());
        connect(&mut graph, cam, capture);
        let preview = EditorCameraPreview {
            node: Some(cam),
            size: UVec2::new(640, 360),
        };

        let desired = desired_sizes(&graph, Some(&preview));
        assert_eq!(desired.get(&cam), Some(&UVec2::new(1920, 1080)));
    }

    #[test]
    fn previewing_something_that_is_not_a_camera_allocates_nothing() {
        // A stale preview selection — the node was deleted and its id reused
        // by something else — must not conjure a target.
        let mut graph = Graph::default();
        let output = insert(&mut graph, Output::default());
        let preview = EditorCameraPreview {
            node: Some(output),
            size: UVec2::new(640, 360),
        };
        assert!(desired_sizes(&graph, Some(&preview)).is_empty());
    }

    #[test]
    fn a_capture_inlet_change_does_not_move_the_cameras_size() {
        // "Consuming a camera does not change it": connecting and
        // disconnecting leaves the authored resolution alone.
        let mut graph = Graph::default();
        let cam = insert(&mut graph, camera(UVec2::new(1280, 720)));
        let capture = insert(
            &mut graph,
            Capture {
                inlets: CaptureIn {
                    path: "out_####.png".into(),
                    recording: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        connect(&mut graph, cam, capture);
        assert_eq!(
            desired_sizes(&graph, None).get(&cam),
            Some(&UVec2::new(1280, 720))
        );

        let edge = graph.edges()[0].id;
        assert!(graph.disconnect(edge));
        assert!(desired_sizes(&graph, None).is_empty());
        assert_eq!(
            authored_resolution(&graph, cam),
            Some(UVec2::new(1280, 720))
        );
    }

    #[test]
    fn a_complaint_is_raised_once_and_again_only_after_it_clears() {
        // Testing the once-ness, not the wording: a diagnostic that repeated
        // every frame would bury the log, and one that never repeated would
        // hide a second, different mistake.
        let mut diagnostics = CameraDiagnostics::default();
        let node = NodeId::new(1, 0);
        assert!(diagnostics.first_time(node, Complaint::ZeroResolution));
        assert!(!diagnostics.first_time(node, Complaint::ZeroResolution));
        assert!(!diagnostics.first_time(node, Complaint::ZeroResolution));

        // A different complaint about the same node is its own message.
        assert!(diagnostics.first_time(node, Complaint::TooLarge));

        diagnostics.cleared(node, Complaint::ZeroResolution);
        assert!(
            diagnostics.first_time(node, Complaint::ZeroResolution),
            "a problem that comes back is reportable again"
        );
    }

    #[test]
    fn source_camera_walks_a_three_node_chain() {
        let mut graph = Graph::default();
        let cam = insert(&mut graph, camera(UVec2::new(1920, 1080)));
        let dof = insert(&mut graph, DepthOfField::default());
        let grade = insert(&mut graph, ColorGrade::default());
        feed(&mut graph, cam, dof);
        feed(&mut graph, dof, grade);

        assert_eq!(source_camera(&graph, cam), Some(cam));
        assert_eq!(source_camera(&graph, dof), Some(cam));
        assert_eq!(source_camera(&graph, grade), Some(cam));
    }

    #[test]
    fn source_camera_is_none_when_the_source_is_missing() {
        let mut graph = Graph::default();
        let grade = insert(&mut graph, ColorGrade::default());
        assert_eq!(source_camera(&graph, grade), None);
    }

    #[test]
    fn a_consumed_color_grade_asks_for_the_cameras_size() {
        let mut graph = Graph::default();
        let cam = insert(&mut graph, camera(UVec2::new(1920, 1080)));
        let grade = insert(&mut graph, ColorGrade::default());
        let output = insert(&mut graph, Output::default());
        feed(&mut graph, cam, grade);
        connect(&mut graph, grade, output);

        let desired = desired_sizes(&graph, None);
        assert_eq!(desired.get(&cam), Some(&UVec2::new(1920, 1080)));
        assert_eq!(desired.get(&grade), Some(&UVec2::new(1920, 1080)));
    }

    #[test]
    fn an_unwired_effect_asks_for_nothing_even_when_consumed() {
        let mut graph = Graph::default();
        let grade = insert(&mut graph, ColorGrade::default());
        let output = insert(&mut graph, Output::default());
        connect(&mut graph, grade, output);
        assert!(desired_sizes(&graph, None).is_empty());
    }

    #[test]
    fn depth_of_field_after_a_color_pass_asks_for_nothing() {
        let mut graph = Graph::default();
        let cam = insert(&mut graph, camera(UVec2::new(1920, 1080)));
        let grade = insert(&mut graph, ColorGrade::default());
        let dof = insert(&mut graph, DepthOfField::default());
        let output = insert(&mut graph, Output::default());
        feed(&mut graph, cam, grade);
        feed(&mut graph, grade, dof);
        connect(&mut graph, dof, output);
        assert!(
            desired_sizes(&graph, None).is_empty(),
            "DoF cannot produce, so the chain allocates nothing"
        );
    }

    #[test]
    fn previewing_a_grade_asks_for_the_panes_pixels_for_the_whole_chain() {
        let mut graph = Graph::default();
        let cam = insert(&mut graph, camera(UVec2::new(3840, 2160)));
        let grade = insert(&mut graph, ColorGrade::default());
        feed(&mut graph, cam, grade);
        let preview = EditorCameraPreview {
            node: Some(grade),
            size: UVec2::new(640, 360),
        };
        let desired = desired_sizes(&graph, Some(&preview));
        assert_eq!(desired.get(&cam), Some(&UVec2::new(640, 360)));
        assert_eq!(desired.get(&grade), Some(&UVec2::new(640, 360)));
    }
}

/// Allocation against a real device.
///
/// `RuntimePlugin`'s chain, a real `wgpu::Device` and a real
/// `ManualTextureViews` — no window, no renderer, no `RenderPlugin`. That is
/// enough to prove targets are actually allocated, registered, resized and
/// released, which the pure `desired_sizes` tests above deliberately do not
/// touch.
#[cfg(test)]
mod device_tests {
    use super::*;
    use crate::nodes::capture::CaptureIn;
    use crate::nodes::scene::CameraIn;
    use crate::project::RuntimePlugin;
    use bevy::asset::AssetPlugin;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::reflect::TypePath;
    use sway_graph::graph::{Node, Port};

    fn app() -> App {
        let gpu = sway_gpu::GpuContext::new(None);
        let mut app = App::new();
        app.add_plugins((bevy::app::TaskPoolPlugin::default(), AssetPlugin::default()));
        app.init_asset::<Mesh>();
        app.init_asset::<Image>();
        app.init_asset::<StandardMaterial>();
        app.init_asset::<crate::nodes::sprite_material::SpriteMaterialAsset>();
        app.add_plugins(RuntimePlugin);
        app.insert_resource(RenderDevice::from(gpu.device.clone()))
            .init_resource::<ManualTextureViews>();
        app
    }

    fn insert<T: Reflect + TypePath>(app: &mut App, value: T) -> NodeId {
        app.world_mut()
            .resource_mut::<Graph>()
            .insert(Node::of(value))
    }

    fn connect(app: &mut App, from: NodeId, to: NodeId) {
        app.world_mut()
            .resource_mut::<Graph>()
            .connect(
                Port::new(from, protocol::CAMERA),
                Port::new(to, protocol::CAMERA),
                0,
            )
            .expect("a camera connects to a consumer");
    }

    fn feed(app: &mut App, from: NodeId, to: NodeId) {
        app.world_mut()
            .resource_mut::<Graph>()
            .connect(
                Port::new(from, protocol::CAMERA),
                Port::new(to, protocol::SOURCE),
                0,
            )
            .expect("a producer feeds an effect");
    }

    fn camera(resolution: UVec2) -> Camera {
        Camera {
            inlets: CameraIn {
                resolution,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn adding_a_camera_does_not_disturb_an_existing_one() {
        let mut app = app();
        let first = insert(&mut app, camera(UVec2::new(1920, 1080)));
        let output = insert(&mut app, Output::default());
        connect(&mut app, first, output);
        app.update();

        let handle_before = app.world().resource::<CameraTargets>().handle(first);
        assert!(handle_before.is_some(), "the presented camera got a target");

        let second = insert(&mut app, camera(UVec2::new(512, 512)));
        let capture = insert(&mut app, Capture::default());
        connect(&mut app, second, capture);
        app.update();

        let targets = app.world().resource::<CameraTargets>();
        let first_target = targets.target(first).expect("still allocated");
        assert_eq!(
            (first_target.width, first_target.height),
            (1920, 1080),
            "the first camera's target kept its size"
        );
        assert_eq!(
            targets.handle(first),
            handle_before,
            "and its handle, so nothing re-pointed at a destroyed texture"
        );
        let second_target = targets.target(second).expect("allocated");
        assert_eq!((second_target.width, second_target.height), (512, 512));
        assert_ne!(targets.handle(first), targets.handle(second));
    }

    #[test]
    fn editing_a_resolution_replaces_only_that_cameras_target() {
        let mut app = app();
        let edited = insert(&mut app, camera(UVec2::new(1920, 1080)));
        let steady = insert(&mut app, camera(UVec2::new(800, 600)));
        let output = insert(&mut app, Output::default());
        let capture = insert(&mut app, Capture::default());
        connect(&mut app, edited, output);
        connect(&mut app, steady, capture);
        app.update();

        let steady_handle = app.world().resource::<CameraTargets>().handle(steady);

        app.world_mut()
            .resource_mut::<Graph>()
            .get_mut(edited)
            .expect("still there")
            .value_mut()
            .downcast_mut::<Camera>()
            .expect("a camera")
            .inlets
            .resolution = UVec2::new(1280, 720);
        app.update();

        let targets = app.world().resource::<CameraTargets>();
        let resized = targets.target(edited).expect("reallocated");
        assert_eq!((resized.width, resized.height), (1280, 720));
        let unchanged = targets.target(steady).expect("untouched");
        assert_eq!((unchanged.width, unchanged.height), (800, 600));
        assert_eq!(targets.handle(steady), steady_handle);

        // The reader sees the new size without the project being reopened.
        let presented = app
            .world()
            .resource::<PresentedCamera>()
            .0
            .expect("presented");
        assert_eq!(presented.resolution, UVec2::new(1280, 720));

        // The old registration went with the old texture rather than being
        // left pointing at a destroyed one.
        let views = app.world().resource::<ManualTextureViews>();
        let handle = targets.handle(edited).expect("has a handle");
        assert_eq!(
            views.get(&handle).map(|view| view.size),
            Some(UVec2::new(1280, 720))
        );
    }

    #[test]
    fn a_zero_resolution_produces_no_target_and_every_other_camera_still_renders() {
        let mut app = app();
        let broken = insert(&mut app, camera(UVec2::new(1920, 0)));
        let fine = insert(&mut app, camera(UVec2::new(640, 360)));
        let capture = insert(&mut app, Capture::default());
        let output = insert(&mut app, Output::default());
        connect(&mut app, broken, capture);
        connect(&mut app, fine, output);
        app.update();

        let targets = app.world().resource::<CameraTargets>();
        assert!(
            targets.target(broken).is_none(),
            "no target of some other size"
        );
        assert!(targets.target(fine).is_some(), "the others still render");
    }

    #[test]
    fn a_resolution_the_device_cannot_allocate_renders_nothing_and_is_reported_once() {
        let gpu = sway_gpu::GpuContext::new(None);
        let over = gpu.device.limits().max_texture_dimension_2d + 1;

        let mut app = app();
        let impossible = insert(&mut app, camera(UVec2::new(over, 1080)));
        let fine = insert(&mut app, camera(UVec2::new(640, 360)));
        let output = insert(&mut app, Output::default());
        let capture = insert(
            &mut app,
            Capture {
                inlets: CaptureIn {
                    path: "shot_####.png".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        connect(&mut app, impossible, output);
        connect(&mut app, fine, capture);
        app.update();

        let targets = app.world().resource::<CameraTargets>();
        assert!(targets.target(impossible).is_none(), "it renders nothing");
        assert!(
            targets.target(fine).is_some(),
            "every other camera still renders"
        );
        assert_eq!(
            app.world().resource::<PresentedCamera>().0,
            None,
            "and nothing is presented in its place"
        );

        // Once, not per frame — the point of the test. A complaint repeated
        // sixty times a second would bury every other message in the log.
        let after_first = app.world().resource::<CameraDiagnostics>().0.len();
        assert_eq!(after_first, 1);
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(app.world().resource::<CameraDiagnostics>().0.len(), 1);
    }

    #[test]
    fn deleting_a_camera_releases_its_target_and_reissues_its_handle() {
        let mut app = app();
        let first = insert(&mut app, camera(UVec2::new(640, 360)));
        let output = insert(&mut app, Output::default());
        connect(&mut app, first, output);
        app.update();
        let handle = app
            .world()
            .resource::<CameraTargets>()
            .handle(first)
            .expect("allocated");

        app.world_mut().resource_mut::<Graph>().remove(first);
        app.update();

        assert!(app.world().resource::<CameraTargets>().is_empty());
        assert!(
            app.world()
                .resource::<ManualTextureViews>()
                .get(&handle)
                .is_none(),
            "a released handle must not still name a destroyed texture"
        );

        // And the handle comes back rather than the space growing per edit.
        let second = insert(&mut app, camera(UVec2::new(640, 360)));
        connect(&mut app, second, output);
        app.update();
        assert_eq!(
            app.world().resource::<CameraTargets>().handle(second),
            Some(handle)
        );
    }

    #[test]
    fn the_wired_camera_is_what_is_presented_and_rewiring_changes_it() {
        let mut app = app();
        let first = insert(&mut app, camera(UVec2::new(640, 360)));
        let second = insert(&mut app, camera(UVec2::new(320, 200)));
        let output = insert(&mut app, Output::default());
        connect(&mut app, first, output);
        app.update();
        assert_eq!(
            app.world().resource::<PresentedCamera>().0.map(|p| p.node),
            Some(first)
        );

        // Connecting a second camera replaces the first rather than failing:
        // the `camera` inlet is non-variadic.
        connect(&mut app, second, output);
        app.update();
        assert_eq!(
            app.world().resource::<PresentedCamera>().0.map(|p| p.node),
            Some(second)
        );
        assert_eq!(
            app.world()
                .resource::<Graph>()
                .edges_into(output)
                .filter(|edge| edge.dst.path == protocol::CAMERA)
                .count(),
            1,
            "the output node holds one connection"
        );
        assert!(
            app.world()
                .resource::<CameraTargets>()
                .target(first)
                .is_none(),
            "the camera nothing consumes any more gave its target back"
        );
    }

    #[test]
    fn one_camera_serves_several_consumers_and_still_renders_once() {
        // "All three receive the same frames at the camera's authored
        // resolution — AND the camera renders once, not once per consumer."
        // One target and one entity is what "once" means here: a second of
        // either would be a second render pass of the same view.
        let mut app = app();
        let cam = insert(&mut app, camera(UVec2::new(1920, 1080)));
        let output = insert(&mut app, Output::default());
        let first_capture = insert(
            &mut app,
            Capture {
                inlets: CaptureIn {
                    path: "a_####.png".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let second_capture = insert(
            &mut app,
            Capture {
                inlets: CaptureIn {
                    path: "b_####.png".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        connect(&mut app, cam, output);
        connect(&mut app, cam, first_capture);
        connect(&mut app, cam, second_capture);
        // And the editor previewing it at a fraction of the size, which must
        // not shrink the target the graph consumers asked for (design D4).
        app.insert_resource(EditorCameraPreview {
            node: Some(cam),
            size: UVec2::new(320, 180),
        });
        app.update();

        let targets = app.world().resource::<CameraTargets>();
        assert_eq!(targets.len(), 1, "one target, not one per consumer");
        let target = targets.target(cam).expect("allocated");
        assert_eq!((target.width, target.height), (1920, 1080));

        let presented = app
            .world()
            .resource::<PresentedCamera>()
            .0
            .expect("presented");
        assert_eq!(presented.resolution, UVec2::new(1920, 1080));
        let intents = &app.world().resource::<CaptureIntents>().0;
        assert_eq!(intents.len(), 2);
        assert!(
            intents
                .iter()
                .all(|intent| intent.resolution == UVec2::new(1920, 1080)),
            "every consumer sees the same frames at the same resolution"
        );
    }

    #[test]
    fn a_preview_only_camera_is_allocated_at_the_fitted_size_and_still_renders_once() {
        let mut app = app();
        let cam = insert(&mut app, camera(UVec2::new(1920, 1080)));
        app.insert_resource(EditorCameraPreview {
            node: Some(cam),
            size: UVec2::new(640, 360),
        });
        app.update();

        let targets = app.world().resource::<CameraTargets>();
        assert_eq!(targets.len(), 1);
        let target = targets.target(cam).expect("allocated");
        assert_eq!(
            (target.width, target.height),
            (640, 360),
            "the preview costs the pane's pixels, not the camera's"
        );
        assert_eq!(
            app.world().resource::<PresentedCamera>().0,
            None,
            "previewing is not presenting"
        );
    }

    #[test]
    fn a_document_with_no_output_node_presents_nothing() {
        let mut app = app();
        insert(&mut app, camera(UVec2::new(640, 360)));
        app.update();
        assert_eq!(app.world().resource::<PresentedCamera>().0, None);
        assert!(app.world().resource::<CameraTargets>().is_empty());
    }

    #[test]
    fn a_capture_publishes_its_camera_path_and_flag() {
        let mut app = app();
        let cam = insert(&mut app, camera(UVec2::new(1920, 1080)));
        let capture = insert(
            &mut app,
            Capture {
                inlets: CaptureIn {
                    path: "frames/shot_####.png".into(),
                    recording: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        connect(&mut app, cam, capture);
        app.update();

        assert_eq!(
            app.world().resource::<CaptureIntents>().0,
            vec![CaptureIntent {
                node: capture,
                camera: cam,
                pattern: "frames/shot_####.png".into(),
                recording: true,
                // Files carry the camera's authored resolution, never a pane's.
                resolution: UVec2::new(1920, 1080),
            }]
        );
    }

    #[test]
    fn a_capture_authored_the_way_the_editor_authors_one_publishes_its_intent() {
        // The palette creates a node through `Graph::create` and the inspector
        // edits it through `Graph::set_field` — neither goes near
        // `Node::of(Capture { .. })`, which is how every other test here builds
        // one. This walks the authoring path a person actually takes, so a
        // node kind that cannot be created or edited reflectively fails here
        // rather than in someone's session.
        let mut app = app();
        let registry = app.world().resource::<AppTypeRegistry>().clone();
        let registry = registry.read();

        let cam = insert(&mut app, camera(UVec2::new(1280, 720)));
        let capture = {
            let mut graph = app.world_mut().resource_mut::<Graph>();
            let capture = graph
                .create(&registry, <Capture as bevy::reflect::TypePath>::type_path())
                .expect("the palette can create a capture node");
            assert_eq!(
                graph.set_field(capture, "path", &"grabs/frame_####.png".to_string()),
                sway_graph::graph::FieldWrite::Written,
                "the inspector can write the path"
            );
            assert_eq!(
                graph.set_field(capture, "recording", &true),
                sway_graph::graph::FieldWrite::Written,
                "and the recording flag"
            );
            capture
        };
        connect(&mut app, cam, capture);
        app.update();

        assert_eq!(
            app.world().resource::<CaptureIntents>().0,
            vec![CaptureIntent {
                node: capture,
                camera: cam,
                pattern: "grabs/frame_####.png".into(),
                recording: true,
                resolution: UVec2::new(1280, 720),
            }]
        );
    }

    #[test]
    fn a_capture_with_no_camera_or_no_path_publishes_no_intent() {
        let mut app = app();
        let unwired = insert(
            &mut app,
            Capture {
                inlets: CaptureIn {
                    path: "shot_####.png".into(),
                    recording: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let cam = insert(&mut app, camera(UVec2::new(64, 64)));
        let pathless = insert(
            &mut app,
            Capture {
                inlets: CaptureIn {
                    recording: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        connect(&mut app, cam, pathless);
        app.update();

        assert!(app.world().resource::<CaptureIntents>().0.is_empty());
        let _ = unwired;

        // Reported once, not per frame: three more frames add no complaints.
        let after_first = app.world().resource::<CameraDiagnostics>().0.len();
        app.update();
        app.update();
        app.update();
        assert_eq!(
            app.world().resource::<CameraDiagnostics>().0.len(),
            after_first,
            "a repeated frame must not be a repeated diagnostic"
        );
        assert_eq!(after_first, 2, "one complaint per broken capture node");
    }

    #[test]
    fn an_output_naming_no_camera_is_reported_once() {
        // The migration depends on this message: a pre-change document
        // presents nothing until its camera is wired to an output node.
        let mut app = app();
        insert(&mut app, Output::default());
        app.update();
        assert_eq!(app.world().resource::<PresentedCamera>().0, None);
        assert_eq!(app.world().resource::<CameraDiagnostics>().0.len(), 1);

        app.update();
        app.update();
        assert_eq!(
            app.world().resource::<CameraDiagnostics>().0.len(),
            1,
            "still once, after three frames"
        );
    }

    #[test]
    fn a_camera_the_graph_produced_renders_into_its_own_target_and_others_into_the_viewport() {
        // The three cases `retarget_cameras` distinguishes, in one world.
        let mut app = app();
        let cam = insert(&mut app, camera(UVec2::new(640, 360)));
        let unconsumed = insert(&mut app, camera(UVec2::new(640, 360)));
        let output = insert(&mut app, Output::default());
        connect(&mut app, cam, output);
        app.update();

        // The editor's own camera: no graph node produced it.
        let editor_camera = app.world_mut().spawn(Camera3d::default()).id();
        app.world_mut()
            .run_system_once(retarget_cameras)
            .expect("the system runs");

        let targets = app.world().resource::<CameraTargets>();
        let handle = targets.handle(cam).expect("allocated");
        let map = app.world().resource::<crate::project::NodeEntities>();
        let consumed_entity = map.entity(cam).expect("a camera is a scene node");
        let unconsumed_entity = map.entity(unconsumed).expect("still a scene node");

        assert!(
            matches!(
                app.world().get::<RenderTarget>(consumed_entity),
                Some(RenderTarget::TextureView(h)) if *h == handle
            ),
            "a consumed camera renders into its own target"
        );
        assert!(
            matches!(
                app.world().get::<RenderTarget>(unconsumed_entity),
                Some(RenderTarget::Window(_))
            ),
            "a camera with no target renders nowhere, not into the viewport"
        );
        assert!(
            matches!(
                app.world().get::<RenderTarget>(editor_camera),
                Some(RenderTarget::TextureView(h)) if *h == VIEWPORT_HANDLE
            ),
            "a camera the graph did not produce renders into the pane-sized viewport"
        );
    }

    #[test]
    fn a_color_grade_on_the_output_allocates_two_targets_of_the_cameras_size() {
        let mut app = app();
        let cam = insert(&mut app, camera(UVec2::new(1920, 1080)));
        let grade = insert(&mut app, ColorGrade::default());
        let output = insert(&mut app, Output::default());
        feed(&mut app, cam, grade);
        connect(&mut app, grade, output);
        app.update();

        let targets = app.world().resource::<CameraTargets>();
        let cam_t = targets.target(cam).expect("camera allocated");
        let grade_t = targets.target(grade).expect("grade allocated");
        assert_eq!((cam_t.width, cam_t.height), (1920, 1080));
        assert_eq!((grade_t.width, grade_t.height), (1920, 1080));
        assert_ne!(targets.handle(cam), targets.handle(grade));
    }

    #[test]
    fn disconnecting_the_output_releases_the_grade_and_the_camera() {
        let mut app = app();
        let cam = insert(&mut app, camera(UVec2::new(1920, 1080)));
        let grade = insert(&mut app, ColorGrade::default());
        let output = insert(&mut app, Output::default());
        feed(&mut app, cam, grade);
        connect(&mut app, grade, output);
        app.update();
        assert_eq!(app.world().resource::<CameraTargets>().len(), 2);

        let edge = {
            let graph = app.world().resource::<Graph>();
            graph
                .edges()
                .iter()
                .find(|e| e.dst.node == output)
                .expect("output connection")
                .id
        };
        app.world_mut().resource_mut::<Graph>().disconnect(edge);
        app.update();
        assert!(app.world().resource::<CameraTargets>().is_empty());
    }

    #[test]
    fn editing_the_camera_resolution_resizes_the_grade_too() {
        let mut app = app();
        let cam = insert(&mut app, camera(UVec2::new(1920, 1080)));
        let grade = insert(&mut app, ColorGrade::default());
        let output = insert(&mut app, Output::default());
        feed(&mut app, cam, grade);
        connect(&mut app, grade, output);
        app.update();

        app.world_mut()
            .resource_mut::<Graph>()
            .get_mut(cam)
            .expect("still there")
            .value_mut()
            .downcast_mut::<Camera>()
            .expect("a camera")
            .inlets
            .resolution = UVec2::new(1280, 720);
        app.update();

        let targets = app.world().resource::<CameraTargets>();
        let cam_t = targets.target(cam).expect("camera");
        let grade_t = targets.target(grade).expect("grade");
        assert_eq!((cam_t.width, cam_t.height), (1280, 720));
        assert_eq!((grade_t.width, grade_t.height), (1280, 720));
    }

    #[test]
    fn branching_keeps_the_camera_target_and_the_grain_target() {
        let mut app = app();
        let cam = insert(&mut app, camera(UVec2::new(800, 600)));
        let grain = insert(&mut app, FilmGrain::default());
        let output = insert(&mut app, Output::default());
        let capture = insert(&mut app, Capture::default());
        connect(&mut app, cam, output);
        feed(&mut app, cam, grain);
        connect(&mut app, grain, capture);
        app.update();

        let targets = app.world().resource::<CameraTargets>();
        assert!(targets.target(cam).is_some());
        assert!(targets.target(grain).is_some());
        assert_ne!(targets.handle(cam), targets.handle(grain));
    }

    #[test]
    fn an_unwired_consumed_grade_gets_no_target() {
        let mut app = app();
        let grade = insert(&mut app, ColorGrade::default());
        let output = insert(&mut app, Output::default());
        connect(&mut app, grade, output);
        app.update();
        assert!(
            app.world()
                .resource::<CameraTargets>()
                .target(grade)
                .is_none()
        );
    }

    #[test]
    fn depth_of_field_after_a_grade_gets_no_target() {
        let mut app = app();
        let cam = insert(&mut app, camera(UVec2::new(1920, 1080)));
        let grade = insert(&mut app, ColorGrade::default());
        let dof = insert(&mut app, DepthOfField::default());
        let output = insert(&mut app, Output::default());
        feed(&mut app, cam, grade);
        feed(&mut app, grade, dof);
        connect(&mut app, dof, output);
        app.update();
        let targets = app.world().resource::<CameraTargets>();
        assert!(targets.target(dof).is_none());
        assert!(targets.target(grade).is_none());
    }

    #[test]
    fn output_wired_to_film_grain_publishes_the_grain_handle() {
        let mut app = app();
        let cam = insert(&mut app, camera(UVec2::new(800, 600)));
        let grain = insert(&mut app, FilmGrain::default());
        let output = insert(&mut app, Output::default());
        feed(&mut app, cam, grain);
        connect(&mut app, grain, output);
        app.update();

        let targets = app.world().resource::<CameraTargets>();
        let grain_handle = targets.handle(grain).expect("grain allocated");
        let presented = app
            .world()
            .resource::<PresentedCamera>()
            .0
            .expect("presented");
        assert_eq!(presented.node, grain);
        assert_eq!(presented.handle, grain_handle);
        assert_eq!(presented.resolution, UVec2::new(800, 600));
    }

    #[test]
    fn capture_wired_to_color_grade_publishes_the_grade_handle() {
        let mut app = app();
        let cam = insert(&mut app, camera(UVec2::new(1920, 1080)));
        let grade = insert(&mut app, ColorGrade::default());
        let capture = insert(
            &mut app,
            Capture {
                inlets: CaptureIn {
                    path: "grabs/frame_####.png".into(),
                    recording: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        feed(&mut app, cam, grade);
        connect(&mut app, grade, capture);
        app.update();

        let targets = app.world().resource::<CameraTargets>();
        let grade_handle = targets.handle(grade).expect("grade allocated");
        let intents = &app.world().resource::<CaptureIntents>().0;
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].camera, grade);
        assert_eq!(intents[0].resolution, UVec2::new(1920, 1080));
        assert_eq!(targets.handle(intents[0].camera), Some(grade_handle));
    }

    #[test]
    fn branching_output_and_capture_publish_two_different_handles() {
        let mut app = app();
        let cam = insert(&mut app, camera(UVec2::new(800, 600)));
        let grain = insert(&mut app, FilmGrain::default());
        let output = insert(&mut app, Output::default());
        let capture = insert(
            &mut app,
            Capture {
                inlets: CaptureIn {
                    path: "grabs/frame_####.png".into(),
                    recording: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        connect(&mut app, cam, output);
        feed(&mut app, cam, grain);
        connect(&mut app, grain, capture);
        app.update();

        let presented = app
            .world()
            .resource::<PresentedCamera>()
            .0
            .expect("presented");
        let intent = &app.world().resource::<CaptureIntents>().0[0];
        assert_eq!(presented.node, cam);
        assert_eq!(intent.camera, grain);
        assert_ne!(
            presented.handle,
            app.world()
                .resource::<CameraTargets>()
                .handle(grain)
                .unwrap()
        );
        assert_eq!(
            presented.handle,
            app.world().resource::<CameraTargets>().handle(cam).unwrap()
        );
    }
}
