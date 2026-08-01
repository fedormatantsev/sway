//! `Displace` — element-wise displacement along `N`. Design §8.

use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::change_detection::Tick;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_math::Vec3;
use bevy_reflect::Reflect;
use sway_graph::{
    NoOutputs, NodeType, PortView, Slot, SlotView, TickCtx, register_slot,
};

use crate::geometry::{Attribute, Geometry};

#[derive(Reflect, Default)]
pub struct DisplaceSlots {
    pub geo: Slot<Geometry>,
}

#[derive(Reflect, Component, Default)]
pub struct DisplaceParams {
    pub amount: f32,
    pub frequency: f32,
}

#[derive(Component, Default)]
pub struct DisplaceState;

pub struct Displace;

impl Displace {
    pub const AMOUNT: u16 = 0;
    pub const FREQUENCY: u16 = 1;
    pub const IN_GEO: u16 = 0;
}

impl NodeType for Displace {
    type Params = DisplaceParams;
    type Outputs = NoOutputs;
    type Slots = DisplaceSlots;
    type Produces = Geometry;
    type State = DisplaceState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] =
        &[("amount", Self::AMOUNT), ("frequency", Self::FREQUENCY)];
    const SLOT_ORDINALS: &'static [(&'static str, u16)] = &[("geo", Self::IN_GEO)];
    const COOKS: bool = true;

    fn register(app: &mut App) {
        register_slot::<Geometry>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, slots: &SlotView) {
        let Some(source) = slots.source(Self::IN_GEO) else {
            return;
        };
        // Reads and writes touch different entities, so read through the
        // world, compute into a local, then insert into self (parent §2.11).
        let Some(input) = world.get::<Geometry>(source).cloned() else {
            return;
        };
        let (amount, frequency) = world
            .get::<DisplaceParams>(node)
            .map(|p| (p.amount, p.frequency))
            .unwrap_or((0.0, 1.0));

        let Some(positions) = input.get("P").and_then(|a| a.as_vec3()) else {
            return;
        };
        let normals = input.get("N").and_then(|a| a.as_vec3());

        let displaced: Vec<Vec3> = positions
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let n = normals.map(|n| n[i]).unwrap_or(Vec3::Y);
                let f = (p.x * frequency).sin() * (p.z * frequency).sin();
                *p + n * (amount * f)
            })
            .collect();

        // Cloning the input carries every other attribute through as a
        // refcount bump; only `P` becomes a new buffer (design §5).
        let mut out = input;
        out.set("P", Attribute::Vec3(Arc::new(displaced)));
        world.entity_mut(node).insert(out);
    }

    fn produced_change_tick(world: &World, node: Entity) -> Option<Tick> {
        world
            .get_entity(node)
            .ok()?
            .get_change_ticks::<Geometry>()
            .map(|t| t.changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::{Grid, GridParams, GridState};

    fn chain(amount: f32) -> (Geometry, Geometry) {
        let mut world = World::new();
        let src = world
            .spawn((
                GridParams { rows: 3, cols: 3, width: 2.0, height: 2.0 },
                GridState,
            ))
            .id();
        Grid::cook(&mut world, src, &SlotView::new(&[]));

        let node = world
            .spawn((
                DisplaceParams { amount, frequency: 1.0 },
                DisplaceState,
            ))
            .id();
        let slots = [Some(sway_graph::SlotSource { entity: src, plan_index: 0 })];
        Displace::cook(&mut world, node, &SlotView::new(&slots));

        (
            world.get::<Geometry>(src).cloned().unwrap(),
            world.get::<Geometry>(node).cloned().unwrap(),
        )
    }

    #[test]
    fn an_untouched_attribute_is_shared_not_copied() {
        // Design §5: the refcount-bump claim, asserted rather than described.
        let (src, out) = chain(0.5);
        let (Some(Attribute::Vec3(a)), Some(Attribute::Vec3(b))) = (src.get("N"), out.get("N"))
        else {
            panic!("N must be a Vec3 attribute on both");
        };
        assert!(Arc::ptr_eq(a, b), "N passed through must not be copied");
    }

    #[test]
    fn positions_are_a_new_buffer() {
        let (src, out) = chain(0.5);
        let (Some(Attribute::Vec3(a)), Some(Attribute::Vec3(b))) = (src.get("P"), out.get("P"))
        else {
            panic!("P must be a Vec3 attribute on both");
        };
        assert!(!Arc::ptr_eq(a, b), "P was rewritten, so it must be its own buffer");
    }

    #[test]
    fn zero_amount_leaves_positions_unmoved() {
        let (src, out) = chain(0.0);
        assert_eq!(
            src.get("P").and_then(|a| a.as_vec3()).cloned(),
            out.get("P").and_then(|a| a.as_vec3()).cloned()
        );
    }

    #[test]
    fn displacement_follows_the_normal() {
        let (src, out) = chain(1.0);
        let before = src.get("P").and_then(|a| a.as_vec3()).unwrap();
        let after = out.get("P").and_then(|a| a.as_vec3()).unwrap();
        let moved = before.iter().zip(after.iter()).any(|(b, a)| b != a);
        assert!(moved, "a non-zero amount must move at least one point");
        for (b, a) in before.iter().zip(after.iter()) {
            assert_eq!(a.x, b.x, "displacement is along N (+Y), not in-plane");
            assert_eq!(a.z, b.z);
        }
    }
}
