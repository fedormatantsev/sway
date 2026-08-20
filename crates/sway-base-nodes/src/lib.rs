//! The base node kinds: the value and signal nodes every project starts from.
//!
//! Every one of them is a pure function of its own inlets and state — a node
//! that advances over time takes that time as an inlet — so this crate reads
//! nothing outside the graph and needs no clock.
//!
//! Render-coupled kinds (meshes, materials, scene nodes) live in
//! `sway-runtime`. MIDI time lives in `sway-midi`.

pub mod nodes;

use bevy_app::{App, Plugin};
use sway_graph::graph::RegisterNodeKind;

pub use nodes::{
    Envelope, EnvelopeIn, EnvelopeOut, EnvelopeState, MakeVec3, MakeVec3In, MakeVec3Out, Math,
    MathIn, MathOp, MathOut, Oscillator, OscillatorIn, OscillatorOut, Remap, RemapIn, RemapOut,
    Waveform,
};
pub use nodes::envelope::{EnvelopeParams, adsr_unscaled};
pub use nodes::math::{math_value, remap_value};
pub use nodes::osc::oscillator_value;

/// The whole domain, in one plugin: every base node kind, its part types (so
/// the editor and the document serializer can reach them by path) and the
/// shared enums a document may reference.
///
/// A host adds this and nothing else from this crate.
pub struct BaseNodesPlugin;

impl Plugin for BaseNodesPlugin {
    fn build(&self, app: &mut App) {
        app.register_node_kind::<MakeVec3>()
            .register_type::<MakeVec3In>()
            .register_type::<MakeVec3Out>()
            .register_node_kind::<Math>()
            .register_type::<MathIn>()
            .register_type::<MathOut>()
            .register_node_kind::<Remap>()
            .register_type::<RemapIn>()
            .register_type::<RemapOut>()
            .register_node_kind::<Oscillator>()
            .register_type::<OscillatorIn>()
            .register_type::<OscillatorOut>()
            .register_node_kind::<Envelope>()
            .register_type::<EnvelopeIn>()
            .register_type::<EnvelopeState>()
            .register_type::<EnvelopeOut>()
            // Shared enums a document or the inspector may address by path
            // (e.g. `inlets.shape`).
            .register_type::<Waveform>()
            .register_type::<MathOp>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_defaults_are_the_first_variants() {
        assert_eq!(Waveform::default(), Waveform::Sine);
        assert_eq!(MathOp::default(), MathOp::Add);
    }
}
