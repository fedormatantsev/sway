mod envelope;
mod lfo;
mod material;
mod math;
mod midi;
mod scene;

pub use envelope::*;
pub use lfo::*;
pub use material::*;
pub use math::*;
pub use midi::*;
pub use scene::*;

#[cfg(test)]
mod tests {
    use bevy_app::App;
    use sway_graph::{GraphPlugin, NodeTypeRegistry};

    use super::*;

    #[test]
    fn signal_nodes_plugin_registers_all_eight_nodes() {
        let mut app = App::new();
        app.add_plugins((GraphPlugin, SignalNodesPlugin));
        let registry = app.world().resource::<NodeTypeRegistry>();
        for name in [
            core::any::type_name::<MidiNote>(),
            core::any::type_name::<MidiCC>(),
            core::any::type_name::<LFO>(),
            core::any::type_name::<Envelope>(),
            core::any::type_name::<Math>(),
            core::any::type_name::<Remap>(),
            core::any::type_name::<Switch>(),
            core::any::type_name::<Select>(),
        ] {
            assert!(registry.id_of(name).is_some(), "{name} must be registered");
        }
    }

    #[test]
    fn enum_defaults_are_the_first_variants() {
        assert_eq!(Waveform::default(), Waveform::Sine);
        assert_eq!(MathOp::default(), MathOp::Add);
        assert_eq!(NoteField::default(), NoteField::Note);
    }
}
