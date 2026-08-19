//! The new graph-model node kinds for `sway-midi` (`redesign-graph-model`
//! task 4.3), landing beside `crate::midi_time::MidiTime` (the old
//! entity/component version) at the crate root. Nothing here replaces
//! anything else in this crate.

pub mod midi_time;

pub use midi_time::{MidiTime, MidiTimeOut};

use bevy_app::{App, Plugin};
use sway_graph::graph::RegisterNodeKind;

/// Registers `MidiTime` and its outlets type.
pub struct MidiGraphNodesPlugin;

impl Plugin for MidiGraphNodesPlugin {
    fn build(&self, app: &mut App) {
        app.register_node_kind::<MidiTime>()
            .register_type::<MidiTimeOut>();
    }
}
