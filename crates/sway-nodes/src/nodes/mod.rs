//! Graph-model node kinds (`redesign-graph-model` tasks 4.1-4.3).
//!
//! Each kind is one `#[derive(Reflect)]` struct with exactly the fields
//! `inlets`, `state`, `outlets` (design D3), registered with
//! [`sway_graph::graph::RegisterNodeKind`] by [`GraphNodesPlugin`].

use bevy_app::{App, Plugin};
use sway_graph::graph::RegisterNodeKind;

pub mod envelope;
pub mod math;
pub mod osc;
pub mod value;

pub use envelope::{Envelope, EnvelopeIn, EnvelopeOut, EnvelopeState};
pub use math::{Math, MathIn, MathOut, Remap, RemapIn, RemapOut};
pub use osc::{Oscillator, OscillatorIn, OscillatorOut, Waveform};
pub use value::{Vec3, Vec3In, Vec3Out};

/// Registers every new-model node kind declared in this module tree, plus
/// their part types (so the editor and the document serializer can reach
/// them by path) and the shared enums a document may reference.
pub struct GraphNodesPlugin;

impl Plugin for GraphNodesPlugin {
    fn build(&self, app: &mut App) {
        app.register_node_kind::<Vec3>()
            .register_type::<Vec3In>()
            .register_type::<Vec3Out>()
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
            .register_type::<crate::math::MathOp>();
    }
}

#[cfg(test)]
mod tests {
    use bevy_app::App;
    use bevy_ecs::reflect::AppTypeRegistry;
    use sway_graph::graph::registry::registered_node_kinds;

    use super::*;

    #[test]
    fn the_plugin_registers_every_new_node_kind_with_a_unique_short_name() {
        let mut app = App::new();
        app.add_plugins(GraphNodesPlugin);

        let registry = app.world().resource::<AppTypeRegistry>().clone();
        let registry = registry.read();
        let kinds = registered_node_kinds(&registry);

        let mut short_names: Vec<&str> = kinds
            .iter()
            .map(|path| path.rsplit("::").next().unwrap_or(path))
            .collect();
        let before_dedup = short_names.len();
        short_names.sort_unstable();
        short_names.dedup();
        assert_eq!(
            short_names.len(),
            before_dedup,
            "every new node kind's short name must be unique: {kinds:?}"
        );

        for expected in ["Vec3", "Math", "Remap", "Oscillator", "Envelope"] {
            assert!(
                short_names.contains(&expected),
                "missing `{expected}` in {short_names:?}"
            );
        }
    }
}
