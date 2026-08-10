use bevy_ecs::schedule::IntoScheduleConfigs;

mod beat;
mod envelope;
mod field_wire;
mod lfo;
mod math;
mod mesh_asset;
mod midi;
mod osc;
mod outputs;
mod pbr_material;
mod scene;
mod spatial;
mod transport;
mod value;
#[cfg(test)]
mod wire_testing;

pub use beat::*;
pub use envelope::*;
pub use lfo::*;
pub use math::*;
pub use mesh_asset::*;
pub use midi::*;
pub use osc::*;
pub use outputs::*;
pub use pbr_material::*;
pub use scene::*;
pub use spatial::*;
pub use transport::*;
pub use value::*;

/// The implemented wire-model slice.
pub struct WireNodesPlugin;

impl bevy_app::Plugin for WireNodesPlugin {
    fn build(&self, app: &mut bevy_app::App) {
        sway_graph::register_behaviour::<Lfo>(app, lfo_behaviour);
        sway_graph::register_wire::<AmplitudeFrom>(app);
        sway_graph::register_wire::<TranslationFrom>(app);
        sway_graph::register_wire::<RotationFrom>(app);
        sway_graph::register_wire::<ScaleFrom>(app);
        sway_graph::register_wire::<bevy::prelude::ChildOf>(app);
        sway_graph::register_behaviour::<Vec3Value>(app, vec3_behaviour);
        sway_graph::register_wire::<Vec3XFrom>(app);
        sway_graph::register_wire::<Vec3YFrom>(app);
        sway_graph::register_wire::<Vec3ZFrom>(app);
        sway_graph::register_behaviour::<Math>(app, math_behaviour);
        sway_graph::register_behaviour::<Remap>(app, remap_behaviour);
        sway_graph::register_wire::<MathAFrom>(app);
        sway_graph::register_wire::<MathBFrom>(app);
        sway_graph::register_wire::<RemapInputFrom>(app);

        // What a project document may name (M4). Short names, not type paths.
        app.register_type::<Waveform>();
        app.register_type::<MathOp>();
        sway_graph::register_authorable::<Lfo>(app, "Lfo");
        sway_graph::register_authorable::<FloatOut>(app, "FloatOut");
        sway_graph::register_authorable::<Vec3Out>(app, "Vec3Out");
        sway_graph::register_authorable::<bevy::prelude::Transform>(app, "Transform");
        sway_graph::register_authorable::<sway_graph::EditorPos>(app, "EditorPos");
        sway_graph::register_authorable::<Vec3Value>(app, "Vec3");
        sway_graph::register_authorable::<Math>(app, "Math");
        sway_graph::register_authorable::<Remap>(app, "Remap");

        sway_graph::register_authorable::<MeshAsset>(app, "MeshAsset");
        app.add_systems(
            bevy_app::Update,
            load_mesh_assets.run_if(bevy_ecs::prelude::resource_exists::<bevy::prelude::AssetServer>),
        );

        sway_graph::register_wire::<MaterialFrom>(app);
        sway_graph::register_authorable::<PbrMaterial>(app, "PbrMaterial");
        app.add_systems(
            bevy_app::Update,
            sync_pbr_materials.run_if(bevy_ecs::prelude::resource_exists::<
                bevy::prelude::Assets<bevy::prelude::StandardMaterial>,
            >),
        );
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
            vec![
                "EditorPos", "FloatOut", "Lfo", "Math", "MeshAsset", "PbrMaterial", "Remap",
                "Transform", "Vec3", "Vec3Out"
            ]
        );
    }
}
