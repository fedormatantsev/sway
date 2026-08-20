//! The `Capture` node: writing a camera's frames to image files.
//!
//! **Not a scene node**, for the same reason [`Output`](super::output::Output)
//! is not: no pose, no `children`, no `SceneNodeOut`, so the closed scene-node
//! set is untouched and a mesh or material connection is refused by schema.
//!
//! The node is a pure function of its inlets and its `state` is empty: it
//! publishes *intent* — which camera, where to write, and whether it is
//! recording — and the host's frame loop owns the slot clock that turns that
//! intent into files. That division is what keeps the capture rate off the
//! graph's tick rate (design D5), and it is why `recording` can become an
//! event-driven inlet later with no change here.

use bevy::ecs::world::World;
use bevy::reflect::Reflect;
use bevy::reflect::std_traits::ReflectDefault;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::nodes::protocol::CameraTarget;

/// Why a path pattern cannot be turned into a filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathPatternError {
    /// The pattern says nothing about where the frame's number goes. Choosing
    /// a numbering scheme on the author's behalf is exactly what the `nodes`
    /// spec forbids, so this is a refusal rather than a fallback.
    NoFrameNumber,
}

impl core::fmt::Display for PathPatternError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoFrameNumber => f.write_str(
                "the path says nothing about where the frame number goes — write a run of \
                 '#' where it should appear, as in \"frames/shot_####.png\"",
            ),
        }
    }
}

impl core::error::Error for PathPatternError {}

/// Expands a path pattern for one capture slot.
///
/// The first run of `#` in the pattern is replaced by `slot`, zero-padded to
/// the length of that run — so `"out_####.png"` at slot 7 is
/// `"out_0007.png"`. A slot too large for the run widens it rather than being
/// truncated: a wrong number is worse than an untidy one. Any later run is
/// left alone, because one number per name is what a sequence means.
///
/// The pattern must contain such a run. The node must not choose a directory,
/// a filename or a numbering scheme of its own, so a pattern that names no
/// place for the number is refused.
pub fn expand_pattern(pattern: &str, slot: u64) -> Result<String, PathPatternError> {
    let bytes = pattern.as_bytes();
    let start = bytes
        .iter()
        .position(|byte| *byte == b'#')
        .ok_or(PathPatternError::NoFrameNumber)?;
    let width = bytes[start..]
        .iter()
        .take_while(|byte| **byte == b'#')
        .count();
    Ok(format!(
        "{}{:0width$}{}",
        &pattern[..start],
        slot,
        &pattern[start + width..],
    ))
}

/// [`Capture`]'s inlets.
#[derive(Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct CaptureIn {
    /// The camera port. A marker inlet: pure schema, non-variadic — the
    /// projector reads the edge, never this field.
    pub camera: CameraTarget,
    /// Where files go, including where each frame's number appears. Relative
    /// to the project directory, like every other path a graph names.
    pub path: String,
    /// Defaults to false, so opening a project never writes a file. A plain
    /// bool today — toggled in the inspector — precisely so that driving it
    /// from an event later needs no change to this node.
    pub recording: bool,
}

/// Writes a connected camera's frames to image files while recording.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct Capture {
    pub inlets: CaptureIn,
    /// Empty, and deliberately so: the run's slot counter lives with the
    /// host-side drain that owns the clock, not here, which keeps this node's
    /// tick a pure function of its inlets.
    pub state: (),
    pub outlets: (),
}

impl NodeKind for Capture {
    fn evaluate(&mut self, _world: &World) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_of_hashes_becomes_the_zero_padded_slot() {
        assert_eq!(
            expand_pattern("frames/shot_####.png", 7).unwrap(),
            "frames/shot_0007.png"
        );
        assert_eq!(
            expand_pattern("frames/shot_####.png", 0).unwrap(),
            "frames/shot_0000.png"
        );
    }

    #[test]
    fn the_padding_width_is_the_length_of_the_run() {
        assert_eq!(expand_pattern("a_#.png", 5).unwrap(), "a_5.png");
        assert_eq!(expand_pattern("a_######.png", 5).unwrap(), "a_000005.png");
        // A slot too big for the run widens rather than truncates: a file
        // named for the wrong frame would silently corrupt the timeline.
        assert_eq!(expand_pattern("a_##.png", 1234).unwrap(), "a_1234.png");
    }

    #[test]
    fn a_pattern_with_no_run_is_refused() {
        // Appending a number, or picking a suffix, would be the node choosing
        // a numbering scheme — which the `nodes` spec forbids.
        assert_eq!(
            expand_pattern("frames/shot.png", 3),
            Err(PathPatternError::NoFrameNumber)
        );
        assert_eq!(expand_pattern("", 0), Err(PathPatternError::NoFrameNumber));
    }

    #[test]
    fn only_the_first_run_is_expanded() {
        // One number per name. A directory named with hashes stays as it was
        // written rather than becoming a second copy of the frame number.
        assert_eq!(
            expand_pattern("take_##/frame_####.png", 12).unwrap(),
            "take_12/frame_####.png"
        );
    }

    #[test]
    fn recording_defaults_to_off() {
        // Opening a project must never write a file.
        assert!(!CaptureIn::default().recording);
    }

    #[test]
    fn a_capture_declares_its_three_ports_and_nothing_a_scene_node_has() {
        use bevy::reflect::{Typed, structs::Struct};
        let bevy::reflect::TypeInfo::Struct(info) = CaptureIn::type_info() else {
            panic!("CaptureIn is a struct");
        };
        let names: Vec<&str> = info.iter().map(|field| field.name()).collect();
        assert_eq!(names, vec!["camera", "path", "recording"]);

        let inlets = CaptureIn::default();
        assert!(inlets.field("mesh").is_none(), "a capture draws nothing");
        assert!(inlets.field("material").is_none());
        assert!(
            inlets.field("children").is_none(),
            "a capture is not a placement, so nothing sits under it"
        );
        assert!(inlets.field("translation").is_none(), "and it has no pose");
        assert!(inlets.field("rotation").is_none());
        assert!(inlets.field("scale").is_none());
    }

    #[test]
    fn a_capture_offers_itself_as_nothing() {
        use bevy::reflect::{TypeInfo, Typed};
        let TypeInfo::Struct(info) = <Capture as Typed>::type_info() else {
            panic!("Capture is a struct");
        };
        let outlets = info
            .field("outlets")
            .expect("every node kind has an outlets part");
        assert_eq!(outlets.type_path(), "()");
    }
}
