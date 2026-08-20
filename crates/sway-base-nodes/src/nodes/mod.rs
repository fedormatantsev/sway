//! The base node kinds, one module each.
//!
//! Each kind is one `#[derive(Reflect)]` struct with exactly the fields
//! `inlets`, `state`, `outlets` (design D3), registered with
//! [`sway_graph::graph::RegisterNodeKind`] by
//! [`BaseNodesPlugin`](crate::BaseNodesPlugin).

pub mod envelope;
pub mod make_vec3;
pub mod math;
pub mod osc;

pub use envelope::{Envelope, EnvelopeIn, EnvelopeOut, EnvelopeState};
pub use make_vec3::{MakeVec3, MakeVec3In, MakeVec3Out};
pub use math::{Math, MathIn, MathOp, MathOut, Remap, RemapIn, RemapOut};
pub use osc::{Oscillator, OscillatorIn, OscillatorOut, Waveform};

#[cfg(test)]
mod tests {
    use bevy_app::App;
    use bevy_ecs::reflect::AppTypeRegistry;
    use sway_graph::graph::registry::registered_node_kinds;

    use crate::BaseNodesPlugin;

    fn short_names(app: &App) -> Vec<&'static str> {
        let registry = app.world().resource::<AppTypeRegistry>().clone();
        let registry = registry.read();
        registered_node_kinds(&registry)
            .iter()
            .map(|path| path.rsplit("::").next().unwrap_or(path))
            .collect()
    }

    #[test]
    fn the_plugin_registers_every_base_node_kind_with_a_unique_short_name() {
        // A document keys node kinds by short name, so two kinds sharing one
        // is a load that cannot be resolved.
        let mut app = App::new();
        app.add_plugins(BaseNodesPlugin);

        let mut names = short_names(&app);
        let before_dedup = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            before_dedup,
            "every base node kind's short name must be unique: {names:?}"
        );

        for expected in ["MakeVec3", "Math", "Remap", "Oscillator", "Envelope"] {
            assert!(
                names.contains(&expected),
                "missing `{expected}` in {names:?}"
            );
        }
    }

    #[test]
    fn a_node_kind_is_not_named_for_the_type_it_produces() {
        // `MakeVec3`'s outlet is a `Vec3`; the kind must not carry that name.
        let mut app = App::new();
        app.add_plugins(BaseNodesPlugin);
        assert!(
            !short_names(&app).contains(&"Vec3"),
            "a node kind must not take its output type's name"
        );
    }
}
