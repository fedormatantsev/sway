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

        // What a project document may name (M4). Short names, not type paths.
        app.register_type::<Waveform>();
        sway_graph::register_authorable::<Lfo>(app, "Lfo");
        sway_graph::register_authorable::<FloatOut>(app, "FloatOut");
        sway_graph::register_authorable::<Vec3Out>(app, "Vec3Out");
        sway_graph::register_authorable::<bevy::prelude::Transform>(app, "Transform");
        sway_graph::register_authorable::<sway_graph::EditorPos>(app, "EditorPos");
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

    #[test]
    fn the_plugin_registers_every_authorable_component() {
        let mut app = bevy_app::App::new();
        app.add_plugins(sway_graph::WiresPlugin)
            .add_plugins(WireNodesPlugin);

        let registry = app.world().resource::<sway_graph::ComponentDocRegistry>();
        let mut names: Vec<&str> = registry.entries.iter().map(|e| e.name).collect();
        names.sort();
        assert_eq!(
            names,
            vec!["EditorPos", "FloatOut", "Lfo", "Transform", "Vec3Out"]
        );
    }
}
