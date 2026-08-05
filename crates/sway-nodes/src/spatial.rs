//! Wires into the scene. Spec §2.3.

use bevy::prelude::*;
use bevy_ecs::change_detection::DetectChangesMut;
use sway_graph::Wire;

use crate::outputs::FloatOut;

#[derive(Component)]
#[relationship(relationship_target = DrivesTranslationY)]
pub struct TranslationYFrom(#[entities] pub Entity);

#[derive(Component)]
#[relationship_target(relationship = TranslationYFrom)]
pub struct DrivesTranslationY(Vec<Entity>);

impl Wire for TranslationYFrom {
    type Source = FloatOut;
    type Target = Transform;
    const NAME: &'static str = "translation.y";

    fn propagate(src: &FloatOut, dst: Mut<Transform>) {
        dst.map_unchanged(|t| &mut t.translation.y).set_if_neq(src.0);
    }
}
