use bevy::prelude::{DirectionalLight, PointLight};
use bevy_ecs::schedule::IntoScheduleConfigs;

mod beat;
mod envelope;
pub mod field_wire;
mod lfo;
mod math;
mod mesh_asset;
pub mod nodes;
mod osc;
mod outputs;
mod pbr_material;
mod plane_mesh;
mod scene;
mod spatial;
mod value;
#[cfg(test)]
mod wire_testing;

pub use beat::*;
pub use envelope::*;
pub use lfo::*;
pub use math::*;
pub use mesh_asset::*;
pub use nodes::GraphNodesPlugin;
pub use osc::*;
pub use outputs::*;
pub use pbr_material::*;
pub use plane_mesh::*;
pub use scene::*;
pub use spatial::*;
pub use value::*;

/// The implemented wire-model slice.
pub struct WireNodesPlugin;

impl bevy_app::Plugin for WireNodesPlugin {
    fn build(&self, app: &mut bevy_app::App) {
        sway_graph::register_behaviour_type::<Oscillator>(app);
        sway_graph::register_wire_type::<TimeFrom>(app);
        sway_graph::register_wire_type::<AmplitudeFrom>(app);
        sway_graph::register_wire_type::<TranslationFrom>(app);
        sway_graph::register_wire_type::<RotationFrom>(app);
        sway_graph::register_wire_type::<ScaleFrom>(app);
        sway_graph::register_wire_type::<bevy::prelude::ChildOf>(app);
        sway_graph::register_behaviour_type::<Vec3Value>(app);
        sway_graph::register_wire_type::<Vec3XFrom>(app);
        sway_graph::register_wire_type::<Vec3YFrom>(app);
        sway_graph::register_wire_type::<Vec3ZFrom>(app);
        sway_graph::register_behaviour_type::<Math>(app);
        sway_graph::register_behaviour_type::<Remap>(app);
        sway_graph::register_wire_type::<MathAFrom>(app);
        sway_graph::register_wire_type::<MathBFrom>(app);
        sway_graph::register_wire_type::<RemapInputFrom>(app);

        // What a project document may name (M4). Short names, not type paths.
        app.register_type::<Waveform>();
        app.register_type::<MathOp>();
        app.register_type::<FloatOut>();
        app.register_type::<Vec3Out>();
        app.register_type::<MaterialOut>();
        app.register_type::<bevy::prelude::MeshMaterial3d<bevy::prelude::StandardMaterial>>();
        sway_graph::register_authorable::<Oscillator>(app, "Oscillator");
        sway_graph::register_authorable::<bevy::prelude::Transform>(app, "Transform");
        sway_graph::register_authorable::<sway_graph::EditorPos>(app, "EditorPos");
        sway_graph::register_authorable::<Vec3Value>(app, "Vec3");
        sway_graph::register_authorable::<Math>(app, "Math");
        sway_graph::register_authorable::<Remap>(app, "Remap");

        sway_graph::register_authorable::<SceneCamera>(app, "SceneCamera");
        // Bevy's own types, registered directly: both already carry
        // #[reflect(Component, Default)] and both already require Transform.
        // #[require(EditorPos)] cannot be added to a foreign type, so a light
        // with no authored EditorPos lands on the canvas's fallback grid.
        sway_graph::register_authorable::<DirectionalLight>(app, "DirectionalLight");
        sway_graph::register_authorable::<PointLight>(app, "PointLight");

        sway_graph::register_authorable::<MeshAsset>(app, "MeshAsset");
        app.add_systems(
            bevy_app::Update,
            load_mesh_assets
                .run_if(bevy_ecs::prelude::resource_exists::<bevy::prelude::AssetServer>),
        );

        sway_graph::register_authorable::<PlaneMesh>(app, "PlaneMesh");
        app.add_systems(
            bevy_app::Update,
            build_plane_meshes.run_if(
                bevy_ecs::prelude::resource_exists::<bevy::prelude::Assets<bevy::prelude::Mesh>>,
            ),
        );

        sway_graph::register_wire_type::<MaterialFrom>(app);
        sway_graph::register_authorable::<PbrMaterial>(app, "PbrMaterial");
        app.add_systems(
            bevy_app::Update,
            sync_pbr_materials.run_if(
                bevy_ecs::prelude::resource_exists::<
                    bevy::prelude::Assets<bevy::prelude::StandardMaterial>,
                >,
            ),
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
                "DirectionalLight",
                "EditorPos",
                "Math",
                "MeshAsset",
                "Oscillator",
                "PbrMaterial",
                "PlaneMesh",
                "PointLight",
                "Remap",
                "SceneCamera",
                "Transform",
                "Vec3",
            ]
        );
    }
}
