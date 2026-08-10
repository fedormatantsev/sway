//! `PbrMaterial` — a material as its own node.

use bevy::prelude::*;
use bevy_ecs::change_detection::DetectChangesMut;
use sway_graph::EditorPos;

use crate::field_wire::field_wire;

/// A PBR material as a node. Colours are `Vec3` rather than `Color` because
/// roadmap D5 makes every colour inlet a `Vec3` wire, and the field a wire
/// writes has to be the type the wire carries. They are read as sRGB — what an
/// author types — and converted on the way to the asset.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
#[require(MaterialOut, EditorPos)]
pub struct PbrMaterial {
    pub base_color: Vec3,
    pub emissive: Vec3,
    pub metallic: f32,
    pub roughness: f32,
}

impl Default for PbrMaterial {
    fn default() -> Self {
        Self {
            base_color: Vec3::splat(0.8),
            emissive: Vec3::ZERO,
            metallic: 0.0,
            roughness: 0.5,
        }
    }
}

impl PbrMaterial {
    pub fn to_standard_material(&self) -> StandardMaterial {
        StandardMaterial {
            base_color: Color::srgb(self.base_color.x, self.base_color.y, self.base_color.z),
            emissive: LinearRgba::rgb(self.emissive.x, self.emissive.y, self.emissive.z),
            metallic: self.metallic,
            perceptual_roughness: self.roughness,
            ..default()
        }
    }
}

/// The outlet, in the sense of architecture §2: an entity is a material
/// producer because it has one of these. Not authorable — a handle has no
/// business round-tripping through a document.
#[derive(Component, Default, Debug, Clone, PartialEq)]
pub struct MaterialOut(pub Handle<StandardMaterial>);

/// An ordinary `Changed<T>` system. The comparison the "never write an equal
/// value" rule asks for happens upstream, on `PbrMaterial` itself: this body
/// only runs when that component actually changed, so the asset write is
/// already guarded.
pub fn sync_pbr_materials(
    mut assets: ResMut<Assets<StandardMaterial>>,
    mut nodes: Query<(&PbrMaterial, &mut MaterialOut), Changed<PbrMaterial>>,
) {
    for (node, mut out) in &mut nodes {
        let desired = node.to_standard_material();
        // Mutating in place is what makes sharing work: every mesh already
        // holding this handle picks the edit up.
        if let Some(mut existing) = assets.get_mut(&out.0) {
            *existing = desired;
        } else {
            out.set_if_neq(MaterialOut(assets.add(desired)));
        }
    }
}

field_wire!(
    /// Hands a material node's asset to a mesh. Sourced from `MaterialOut`
    /// rather than from `MeshMaterial3d` so the editor's legality rule stays
    /// exact — every mesh carries a `MeshMaterial3d`, and sourcing from that
    /// would make every mesh look like a legal material producer.
    MaterialFrom / DrivesMaterial,
    MaterialOut => MeshMaterial3d<StandardMaterial>,
    "material",
    |t| &mut t.0,
    |s| s.0.clone()
);

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;
    use bevy::prelude::*;
    use crate::wire_testing::assert_writes_only_on_change;
    use sway_graph::propagate_of;

    fn material_app() -> App {
        let mut app = App::new();
        app.add_plugins((bevy::app::TaskPoolPlugin::default(), AssetPlugin::default()));
        app.init_asset::<StandardMaterial>();
        app.add_systems(Update, sync_pbr_materials);
        app
    }

    #[test]
    fn a_material_node_publishes_a_handle_to_its_own_asset() {
        let mut app = material_app();
        let node = app
            .world_mut()
            .spawn(PbrMaterial {
                base_color: Vec3::new(0.6, 0.7, 0.9),
                metallic: 0.25,
                ..default()
            })
            .id();

        app.update();

        let handle = app.world().get::<MaterialOut>(node).expect("required").0.clone();
        assert_ne!(handle, Handle::default(), "an asset was created");
        let material = app
            .world()
            .resource::<Assets<StandardMaterial>>()
            .get(&handle)
            .expect("the handle resolves");
        assert_eq!(material.metallic, 0.25);
    }

    #[test]
    fn editing_a_material_mutates_the_asset_in_place() {
        // In place, not replaced: every mesh already holding this handle must
        // see the edit, which is the whole reason a material is its own node.
        let mut app = material_app();
        let node = app.world_mut().spawn(PbrMaterial::default()).id();
        app.update();
        let before = app.world().get::<MaterialOut>(node).expect("required").0.clone();

        app.world_mut().get_mut::<PbrMaterial>(node).expect("present").metallic = 1.0;
        app.update();

        let after = app.world().get::<MaterialOut>(node).expect("required").0.clone();
        assert_eq!(before, after, "the handle must not change under an edit");
        assert_eq!(
            app.world()
                .resource::<Assets<StandardMaterial>>()
                .get(&after)
                .map(|m| m.metallic),
            Some(1.0)
        );
    }

    #[test]
    fn the_material_wire_hands_the_same_handle_to_two_meshes() {
        let mut app = material_app();
        let node = app.world_mut().spawn(PbrMaterial::default()).id();
        app.update();

        let a = app
            .world_mut()
            .spawn(MeshMaterial3d::<StandardMaterial>::default())
            .id();
        let b = app
            .world_mut()
            .spawn(MeshMaterial3d::<StandardMaterial>::default())
            .id();
        propagate_of::<MaterialFrom>(app.world_mut(), node, a);
        propagate_of::<MaterialFrom>(app.world_mut(), node, b);

        let expected = app.world().get::<MaterialOut>(node).expect("required").0.clone();
        assert_eq!(
            app.world().get::<MeshMaterial3d<StandardMaterial>>(a).map(|m| m.0.clone()),
            Some(expected.clone())
        );
        assert_eq!(
            app.world().get::<MeshMaterial3d<StandardMaterial>>(b).map(|m| m.0.clone()),
            Some(expected)
        );
    }

    #[test]
    fn the_material_wire_never_writes_an_equal_value() {
        let mut assets = Assets::<StandardMaterial>::default();
        let one = assets.add(StandardMaterial::default());
        let two = assets.add(StandardMaterial::default());
        assert_writes_only_on_change::<MaterialFrom>(
            MaterialOut(one),
            MaterialOut(two),
            MeshMaterial3d::<StandardMaterial>::default(),
        );
    }

    #[test]
    fn material_parameters_reach_the_standard_material() {
        // Carried over from the deleted material.rs.
        let material = PbrMaterial {
            base_color: Vec3::ONE,
            emissive: Vec3::ZERO,
            metallic: 0.25,
            roughness: 0.75,
        }
        .to_standard_material();
        assert_eq!(material.base_color, Color::srgb(1.0, 1.0, 1.0));
        assert_eq!(material.metallic, 0.25);
        assert_eq!(material.perceptual_roughness, 0.75);
    }
}
