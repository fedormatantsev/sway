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

/// The implemented wire-model slice.
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
    use super::*;

    #[test]
    fn enum_defaults_are_the_first_variants() {
        assert_eq!(Waveform::default(), Waveform::Sine);
        assert_eq!(MathOp::default(), MathOp::Add);
        assert_eq!(NoteField::default(), NoteField::Note);
        assert_eq!(Division::default(), Division::Beat);
    }
}
