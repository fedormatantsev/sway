use bevy::prelude::*;
use sway_graph::EditorPos;
use sway_nodes::FloatOut;

use crate::Transport;

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
#[require(FloatOut, EditorPos)]
pub struct MidiTime;

pub fn write_midi_time(
    transport: Res<Transport>,
    mut midi_times: Query<&mut FloatOut, With<MidiTime>>,
) {
    let value = FloatOut(transport.ppq as f32);
    for mut output in &mut midi_times {
        output.set_if_neq(value);
    }
}
