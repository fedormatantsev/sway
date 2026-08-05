//! The wire-model demo. Spec §5.1.
//!
//! ```text
//! Lfo A ──amplitude──▶ Lfo B ──translation.y──▶ cube B
//!       └─translation.y──▶ cube A            (fan-out)
//! group ──ChildOf──▶ cube A, cube B
//! ```
//!
//! Geometry does not flow through the graph in this slice: each cube's mesh
//! is built once here. Asset flow is the follow-up spec's problem.

use bevy::prelude::*;
use sway_nodes::{AmplitudeFrom, FloatOut, Lfo, TranslationYFrom, Waveform};

pub fn setup_demo_graph(world: &mut World) {
    let mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(0.6, 0.6, 0.6));
    let material = world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.6, 0.7, 0.9),
            ..default()
        });

    // The modulator: a slow, half-amplitude sine.
    let modulator = world
        .spawn((
            Name::new("Lfo A (modulator)"),
            Lfo { beats: 8.0, shape: Waveform::Sine, phase: 0.0, amplitude: 0.5 },
            FloatOut::default(),
        ))
        .id();

    // The modulated LFO: its amplitude is driven by A, so it must compute
    // between two propagations. This is what makes the order load-bearing.
    let carrier = world
        .spawn((
            Name::new("Lfo B (carrier)"),
            Lfo { beats: 2.0, shape: Waveform::Sine, phase: 0.0, amplitude: 0.0 },
            FloatOut::default(),
            AmplitudeFrom(modulator),
        ))
        .id();

    let group = world
        .spawn((Name::new("group"), Transform::default(), Visibility::default()))
        .id();

    world.spawn((
        Name::new("cube A"),
        Mesh3d(mesh.clone()),
        MeshMaterial3d(material.clone()),
        Transform::from_xyz(-0.8, 0.0, 0.0),
        Visibility::default(),
        ChildOf(group),
        TranslationYFrom(modulator),
    ));

    world.spawn((
        Name::new("cube B"),
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_xyz(0.8, 0.0, 0.0),
        Visibility::default(),
        ChildOf(group),
        TranslationYFrom(carrier),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .add_plugins((sway_graph::WiresPlugin, sway_nodes::WireNodesPlugin));
        app
    }

    #[test]
    fn the_demo_is_built_from_wire_components() {
        let mut app = app();
        setup_demo_graph(app.world_mut());

        assert_eq!(
            app.world_mut().query::<&Lfo>().iter(app.world()).count(),
            2
        );
        assert_eq!(
            app.world_mut().query::<&TranslationYFrom>().iter(app.world()).count(),
            2
        );
        assert_eq!(
            app.world_mut().query::<&ChildOf>().iter(app.world()).count(),
            2
        );
    }
}
