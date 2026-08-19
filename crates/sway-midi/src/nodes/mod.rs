//! Graph-model MIDI nodes.

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
