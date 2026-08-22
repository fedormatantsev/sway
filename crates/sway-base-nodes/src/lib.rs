//! The base node kinds: the value and signal nodes every project starts from.
//!
//! Every one of them is a pure function of its own inlets and state — a node
//! that advances over time takes that time as an inlet. A handle inlet is
//! resolved through the occurrence arena; this crate still reads no clock and
//! no MIDI.
//!
//! Render-coupled kinds (meshes, materials, scene nodes) live in
//! `sway-runtime`. MIDI time lives in `sway-midi`.

pub mod nodes;

use bevy_app::{App, Plugin};
use sway_events::RegisterEventHandle;
use sway_graph::graph::RegisterNodeKind;

pub use nodes::curve_sampler::curve_sampler_value;
pub use nodes::math::{math_value, remap_value};
pub use nodes::{
    CurveKeys, CurveSampler, CurveSamplerIn, CurveSamplerOut, MakeVec3, MakeVec3In, MakeVec3Out,
    Math, MathIn, MathOp, MathOut, Remap, RemapIn, RemapOut, Timer, TimerIn, TimerOut, TimerState,
    Trigger,
};

/// The whole domain, in one plugin: every base node kind, its part types (so
/// the editor and the document serializer can reach them by path) and the
/// shared enums a document may reference.
///
/// A host adds this and nothing else from this crate. Registering [`Trigger`]
/// and `EventHandle<Trigger>` is this plugin's job — a host that adds it
/// registers neither on the domain's behalf.
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
            .register_node_kind::<CurveSampler>()
            .register_type::<CurveSamplerIn>()
            .register_type::<CurveSamplerOut>()
            .register_type::<CurveKeys>()
            .register_node_kind::<Timer>()
            .register_type::<TimerIn>()
            .register_type::<TimerState>()
            .register_type::<TimerOut>()
            .register_type::<Trigger>()
            .register_event_handle::<Trigger>()
            .register_type::<MathOp>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_defaults_are_the_first_variants() {
        assert_eq!(MathOp::default(), MathOp::Add);
    }
}
