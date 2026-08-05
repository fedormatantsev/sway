mod beat;
mod envelope;
mod lfo;
mod material;
mod math;
mod mesh;
mod midi;
mod osc;
mod outputs;
mod scene;
mod spatial;
mod transport;

pub use beat::*;
pub use envelope::*;
pub use lfo::*;
pub use material::*;
pub use math::*;
pub use mesh::*;
pub use midi::*;
pub use osc::*;
pub use outputs::*;
pub use scene::*;
pub use spatial::*;
pub use transport::*;

/// Registers the M2b scene node set.
pub struct SceneNodesPlugin;

impl bevy_app::Plugin for SceneNodesPlugin {
    fn build(&self, app: &mut bevy_app::App) {
        sway_graph::register_node_type::<Group>(app);
        sway_graph::register_node_type::<Rgb>(app);
        sway_graph::register_node_type::<StandardMaterialNode>(app);
        sway_graph::register_node_type::<MeshNode>(app);
    }
}

/// The wire-model slice. Spec §5.1.
pub struct WireNodesPlugin;

impl bevy_app::Plugin for WireNodesPlugin {
    fn build(&self, app: &mut bevy_app::App) {
        sway_graph::register_behaviour::<Lfo>(app, lfo_behaviour);
        sway_graph::register_wire::<AmplitudeFrom>(app);
        sway_graph::register_wire::<TranslationYFrom>(app);
        sway_graph::register_wire::<bevy::prelude::ChildOf>(app);
    }
}

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
        assert_eq!(Division::default(), Division::Beat);
    }
}
