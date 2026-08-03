//! `Displace` — element-wise displacement along `N`. Design §8.

use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::change_detection::Tick;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_math::Vec3;
use bevy_reflect::Reflect;
use sway_graph::{NodeType, PortView, Product, TickCtx, register_product};

use crate::geometry::{Attribute, Geometry};

#[derive(Reflect, Component, Default)]
pub struct DisplaceInlets {
    pub geo: Product<Geometry>,
    pub amount: f32,
    pub frequency: f32,
}

#[derive(Reflect, Default)]
pub struct DisplaceOutlets {
    pub geo: Product<Geometry>,
}

#[derive(Component, Default)]
pub struct DisplaceState;

pub struct Displace;

impl Displace {
    pub const IN_GEO: u16 = 0;
    pub const AMOUNT: u16 = 1;
    pub const FREQUENCY: u16 = 2;
    pub const OUT_GEO: u16 = 3;
}

impl NodeType for Displace {
    type Inlets = DisplaceInlets;
    type Outlets = DisplaceOutlets;
    type State = DisplaceState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("geo", Self::IN_GEO),
        ("amount", Self::AMOUNT),
        ("frequency", Self::FREQUENCY),
        ("geo", Self::OUT_GEO),
    ];
    const COOKS: bool = true;

    fn register(app: &mut App) {
        register_product::<Geometry>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, ports: &PortView) {
        let Some(source) = ports.source(Self::IN_GEO, 0) else {
            return;
        };
        // Reads and writes touch different entities, so read through the
        // world, compute into a local, then insert into self (parent §2.11).
        let Some(input) = world.get::<Geometry>(source).cloned() else {
            return;
        };
        let (amount, frequency) = world
            .get::<DisplaceInlets>(node)
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
    use crate::grid::{Grid, GridInlets, GridState};
    use sway_graph::PortArena;

    fn chain(amount: f32) -> (Geometry, Geometry) {
        let mut world = World::new();
        let src = world
            .spawn((
                GridInlets { rows: 3, cols: 3, width: 2.0, height: 2.0 },
                GridState,
            ))
            .id();
        // Cook Grid node with minimal PortView
        let mut arena = PortArena::new(0);
        let ports = PortView::new(&mut arena, 0, &[], &[], &[], &[]);
        Grid::cook(&mut world, src, &ports);

        let node = world
            .spawn((
                DisplaceInlets {
                    geo: Product::from_source(src),
                    amount,
                    frequency: 1.0
                },
                DisplaceState,
            ))
            .id();

        // Cook Displace node with PortView
        // We need to construct a PortView that supports source() calls
        let mut arena = PortArena::new(1);
        // Place the source reference in the arena
        arena.values[0] = Box::new(Product::<Geometry>::from_source(src));

        // Create minimal field spec just to make PortView work
        use sway_graph::schema::{FieldSpec, FieldKind, ProductAccess};
        use std::any::TypeId;

        // ProductAccess functions for Product<Geometry>
        let product_access = ProductAccess {
            get: |v| {
                v.try_downcast_ref::<Product<Geometry>>()
                    .and_then(|p| p.source)
            },
            set: |v, e| {
                if let Some(p) = v.try_downcast_mut::<Product<Geometry>>() {
                    p.source = e;
                }
            },
        };

        let fields = vec![FieldSpec {
            name: "geo",
            field_index: 0,
            kind: FieldKind::Product {
                access: product_access,
                capability: TypeId::of::<Geometry>(),
                spatial: false,
            },
            slot_type: TypeId::of::<Product<Geometry>>(),
            slot_type_path: "sway_graph::ports::Product<crate::geometry::Geometry>",
            variadic: false,
        }];

        let field_offsets = vec![0];
        let field_lens = vec![1];
        let connected = vec![true];

        let ports = PortView::new(&mut arena, 0, &fields, &field_offsets, &field_lens, &connected);
        Displace::cook(&mut world, node, &ports);

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
