//! `FrameSequence` — a folder of images published as one array texture, as a
//! graph node.
//!
//! A port of `crate::frame_sequence::FrameSequence`, which stays where it is
//! until group 9. The pure parts of that module — [`ColorSpace`],
//! [`assemble_layers`](crate::frame_sequence::assemble_layers),
//! [`sort_frames_by_name`](crate::frame_sequence::sort_frames_by_name) and
//! [`SequenceError`](crate::frame_sequence::SequenceError) — are reused
//! unchanged rather than copied: they are pure functions over `Image`, they
//! carry the whole of the folder-ordering and assembly reasoning, and they are
//! already tested there. **Group 9 must move them, not delete them.**
//!
//! The node owns its texture. Nothing hands it along a connection: an edge
//! from `outlets.sequence` carries a ZST and exists only to say the
//! connection is there and to order the two projectors (design D6).

use bevy::asset::LoadedFolder;
use bevy::ecs::world::World;
use bevy::prelude::*;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::frame_sequence::ColorSpace;
use crate::nodes::protocol::{self, ImageSequenceOut, ReflectImageSequenceNode};

/// [`FrameSequence`]'s inlets.
///
/// **Filenames must be zero-padded** — `000.png`, `001.png`, … `010.png`.
/// Order is ascending by filename and deliberately lexicographic.
#[derive(Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct FrameSequenceIn {
    pub folder: String,
    /// How the stored bytes are meant to be read. One node kind serves both
    /// the colour run and the depth run, so this cannot be inferred from
    /// where the sequence is connected.
    pub color_space: ColorSpace,
}

/// [`FrameSequence`]'s state. Not authored, not serialized.
#[derive(Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct FrameSequenceState {
    /// The published array texture, or `Handle::default()` while nothing has
    /// been published.
    pub texture: Handle<Image>,
    /// The published texture's actual layer count. Derived from what loaded,
    /// never authored, so a partly-loaded sequence can never be sampled out
    /// of range.
    pub layers: u32,
    /// The folder path `folder` was enumerated for. Compared against the
    /// inlet so that editing the *colour space* re-assembles without
    /// restarting the folder load.
    pub folder_path: String,
    /// The strong folder handle. It has to live somewhere: dropping it
    /// unloads the folder *and* every frame in it.
    pub folder: Handle<LoadedFolder>,
    /// Set when something that could change the outcome happened; cleared
    /// when an assembly is attempted. The anti-spam mechanism — an attempt
    /// that finds frames still in flight must not retry every frame.
    pub pending: bool,
    /// The last diagnostic reported, so a permanent error logs once rather
    /// than once per attempt.
    pub reported: Option<String>,
}

/// A run of images loaded from one folder and published as a single layered
/// texture, one layer per image.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, ImageSequenceNode, Default)]
pub struct FrameSequence {
    pub inlets: FrameSequenceIn,
    pub state: FrameSequenceState,
    pub outlets: ImageSequenceOut,
}

impl NodeKind for FrameSequence {
    /// Nothing: enumerating a folder and assembling an array texture needs
    /// the `AssetServer` and `ResMut<Assets<Image>>`, which `&World` cannot
    /// give. The projector does it.
    fn evaluate(&mut self, _world: &World) {}
}

impl protocol::ImageSequenceNode for FrameSequence {
    fn texture(&self) -> &Handle<Image> {
        &self.state.texture
    }

    fn layers(&self) -> u32 {
        self.state.layers
    }
}
