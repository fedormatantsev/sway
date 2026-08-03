//! The node type registry.
//!
//! A node type is plugin-shaped; a node instance is an entity. `register`
//! erases `tick` to a bare fn stored here, and the tick loop dispatches
//! through it — there is no `NodeInstance` trait object.

use bevy_app::App;
use bevy_ecs::change_detection::Tick;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::reflect::AppTypeRegistry;
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;
use bevy_reflect::structs::Struct;
use bevy_reflect::{GetTypeRegistration, PartialReflect, Reflect, Typed};
use std::collections::HashMap;

use crate::compile::NodePlan;
use crate::ports::PortArena;
use crate::schema::{derive_fields, FieldKind, FieldSpec};
use crate::view::{PortView, TickCtx};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct NodeTypeId(pub u32);

pub type TickFn = fn(&mut World, Entity, &mut PortView, &TickCtx);
pub type CookFn = fn(&mut World, Entity, &PortView);
pub type PrefillFn = fn(&World, Entity, &mut PortArena, &NodePlan);
pub type SeedOutletsFn = fn(&mut PortArena, &NodePlan);
pub type InsertDefaultsFn = fn(&mut World, Entity);
pub type TickOfFn = fn(&World, Entity) -> Option<Tick>;
pub type ProducedTickFn = fn(&World, Entity) -> Option<Tick>;
/// Per inlet field: how many slots this instance has. 1 for a non-`Vec`
/// field, the instance's `Vec` length otherwise.
pub type InletLensFn = fn(&World, Entity) -> Vec<usize>;

pub trait NodeType: 'static {
    /// Everything the node consumes: authored values, event lists and
    /// products, in one struct. Also the component holding authored values.
    type Inlets: Reflect + Typed + GetTypeRegistration + Component + Default;
    /// Everything the node offers. Direction comes from which struct a field
    /// is in, so the same marker types are legal in both.
    type Outlets: Reflect + Typed + GetTypeRegistration + Default;
    type State: Component + Default;

    /// `(field name, the ordinal the node's index const uses)` for every
    /// field, inlets first then outlets. Verified against the derived fields
    /// at registration, so a field reorder fails at startup instead of
    /// silently swapping two ports.
    const ORDINALS: &'static [(&'static str, u16)];

    /// Whether `cook` is meaningful. Rust cannot distinguish a defaulted
    /// trait method from an overridden one, and the gate needs to know
    /// whether a node has a cook at all.
    const COOKS: bool = false;

    fn register(app: &mut App);
    fn tick(world: &mut World, node: Entity, ports: &mut PortView, t: &TickCtx);

    /// Reads this node's product inlets and writes its own product. Runs
    /// immediately after `tick`, only when the gate says the node is dirty.
    fn cook(_world: &mut World, _node: Entity, _ports: &PortView) {}

    /// The change tick of whatever this node's product consumers depend on.
    ///
    /// `None` — the default — means "changes to what I produce do not require
    /// my consumers to re-cook", which is correct for a material node: its
    /// consumers hold its `Handle`, and editing the material's params does
    /// not change the handle.
    fn produced_change_tick(_world: &World, _node: Entity) -> Option<Tick> {
        None
    }
}

pub struct NodeTypeEntry {
    pub name: &'static str,
    pub inlets: Vec<FieldSpec>,
    pub outlets: Vec<FieldSpec>,
    pub tick: TickFn,
    pub prefill: PrefillFn,
    pub seed_outlets: SeedOutletsFn,
    pub insert_defaults: InsertDefaultsFn,
    pub inlets_changed_tick: TickOfFn,
    pub inlet_lens: InletLensFn,
    /// `Some` iff `N::COOKS`.
    pub cook: Option<CookFn>,
    pub produced_change_tick: ProducedTickFn,
}

impl NodeTypeEntry {
    /// Resolves a field ordinal in the node's one flat space: inlets first,
    /// then outlets. Returns the spec and whether it is an inlet.
    pub fn field(&self, ordinal: u16) -> Option<(&FieldSpec, bool)> {
        let o = ordinal as usize;
        if o < self.inlets.len() {
            Some((&self.inlets[o], true))
        } else {
            self.outlets.get(o - self.inlets.len()).map(|f| (f, false))
        }
    }

    pub fn field_count(&self) -> usize {
        self.inlets.len() + self.outlets.len()
    }
}

#[derive(Resource, Default)]
pub struct NodeTypeRegistry {
    entries: Vec<NodeTypeEntry>,
    by_name: HashMap<&'static str, NodeTypeId>,
}

impl NodeTypeRegistry {
    pub fn get(&self, id: NodeTypeId) -> Option<&NodeTypeEntry> {
        self.entries.get(id.0 as usize)
    }
    pub fn id_of(&self, name: &str) -> Option<NodeTypeId> {
        self.by_name.get(name).copied()
    }
}

pub fn register_node_type<N: NodeType>(app: &mut App) -> NodeTypeId {
    N::register(app);

    app.init_resource::<AppTypeRegistry>();
    {
        let registry = app.world().resource::<AppTypeRegistry>().clone();
        let mut w = registry.write();
        w.register::<N::Inlets>();
        w.register::<N::Outlets>();
    }

    let (inlets, outlets) = {
        let registry = app.world().resource::<AppTypeRegistry>().clone();
        let r = registry.read();
        (
            derive_fields::<N::Inlets>(&r)
                .unwrap_or_else(|e| panic!("{}: {e}", core::any::type_name::<N>())),
            derive_fields::<N::Outlets>(&r)
                .unwrap_or_else(|e| panic!("{}: {e}", core::any::type_name::<N>())),
        )
    };

    check_ordinals::<N>(&inlets, &outlets);
    check_outlets::<N>(&outlets);

    let entry = NodeTypeEntry {
        name: core::any::type_name::<N>(),
        inlets,
        outlets,
        tick: N::tick,
        prefill: prefill_of::<N>,
        seed_outlets: seed_outlets_of::<N>,
        insert_defaults: insert_defaults_of::<N>,
        inlets_changed_tick: inlets_changed_tick_of::<N>,
        inlet_lens: inlet_lens_of::<N>,
        cook: if N::COOKS { Some(N::cook as CookFn) } else { None },
        produced_change_tick: N::produced_change_tick,
    };

    app.init_resource::<NodeTypeRegistry>();
    let mut reg = app.world_mut().resource_mut::<NodeTypeRegistry>();
    let id = NodeTypeId(reg.entries.len() as u32);
    reg.by_name.insert(entry.name, id);
    reg.entries.push(entry);
    id
}

/// The startup guard: the node's index consts must agree with the derived
/// field ordinals. Inlets occupy 0..inlets.len(); outlets follow.
///
/// Element indices are *not* declared — they are positional within their
/// field, and a `Vec` field's length is per-instance.
fn check_ordinals<N: NodeType>(inlets: &[FieldSpec], outlets: &[FieldSpec]) {
    let node = core::any::type_name::<N>();
    let expected: Vec<(&'static str, u16)> = inlets
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name, i as u16))
        .chain(
            outlets
                .iter()
                .enumerate()
                .map(|(i, f)| (f.name, (inlets.len() + i) as u16)),
        )
        .collect();

    let mut matched = vec![false; N::ORDINALS.len()];
    for &(name, want) in &expected {
        if let Some((index, _)) = N::ORDINALS
            .iter()
            .enumerate()
            .find(|(index, entry)| !matched[*index] && **entry == (name, want))
        {
            matched[index] = true;
            continue;
        }
        match N::ORDINALS
            .iter()
            .enumerate()
            .find(|(index, (declared, _))| !matched[*index] && declared == &name)
        {
            Some((_, (_, got))) => panic!(
                "{node}: field `{name}` is ordinal {want}, but ORDINALS declares {got} \
                 — a field was reordered, or the const is stale"
            ),
            None => panic!(
                "{node}: field `{name}` is undeclared in ORDINALS (expected ordinal {want})"
            ),
        }
    }
    for (index, (name, _)) in N::ORDINALS.iter().enumerate() {
        if !matched[index] {
            panic!("{node}: ORDINALS declares `{name}`, which is not a field");
        }
    }
}

/// Two limits on `Outlets`, both deliberate (design §12):
///
/// - at most one product outlet, which is what keeps `produced_change_tick` a
///   per-node function rather than a per-outlet table;
/// - no `Vec` outlets, because nothing needs a variable number of outputs and
///   allowing it would put a per-instance count on the source side of every
///   edge.
fn check_outlets<N: NodeType>(outlets: &[FieldSpec]) {
    let node = core::any::type_name::<N>();
    let mut products = outlets
        .iter()
        .filter(|f| matches!(f.kind, FieldKind::Product { .. }));
    if let (Some(first), Some(second)) = (products.next(), products.next()) {
        panic!(
            "{node}: outlets `{}` and `{}` are both products, and a node may have at most one \
             — `produced_change_tick` is per node, not per outlet",
            first.name, second.name
        );
    }
    if let Some(variadic) = outlets.iter().find(|f| f.variadic) {
        panic!(
            "{node}: outlet `{}` is a Vec, which is not supported — a node's outlet count is \
             fixed by its type",
            variadic.name
        );
    }
}

/// Copies every **unconnected** inlet slot from the node's `Inlets` component
/// into the arena. The authored-versus-driven rule: `Inlets` is never written
/// by the graph, so a disconnect returns the slot to its authored value.
fn prefill_of<N: NodeType>(world: &World, node: Entity, arena: &mut PortArena, plan: &NodePlan) {
    let Some(inlets) = world.get::<N::Inlets>(node) else {
        return;
    };
    let inlets: &dyn Struct = inlets.reflect_ref().as_struct().expect("Inlets is a struct");

    for (ordinal, spec) in plan.fields[..plan.inlet_field_count].iter().enumerate() {
        let field = inlets
            .field_at(spec.field_index)
            .expect("field_index came from this type's own fields");
        let offset = plan.field_offsets[ordinal];
        let len = plan.field_lens[ordinal];

        for index in 0..len {
            if plan.connected[offset + index] {
                continue;
            }
            let value = if spec.variadic {
                field
                    .reflect_ref()
                    .as_list()
                    .expect("a variadic field reflects as a list")
                    .get(index)
                    .expect("compile sized this field from the same list")
            } else {
                field
            };
            arena.values[plan.base + offset + index] = clone_authored(value);
        }
    }
}

fn clone_authored(value: &dyn PartialReflect) -> Box<dyn PartialReflect> {
    value
        .reflect_clone()
        .unwrap_or_else(|error| {
            panic!(
                "could not clone `{}` while prefilling an inlet: {error:?}",
                value.reflect_type_path()
            )
        })
        .into_partial_reflect()
}

/// Writes each outlet's default into its slot, and seeds a product outlet
/// with the node's own entity — which is constant for the life of a compiled
/// graph, so the node's `tick` never has to write it.
fn seed_outlets_of<N: NodeType>(arena: &mut PortArena, plan: &NodePlan) {
    let outlets = N::Outlets::default();
    let outlets: &dyn Struct = outlets.reflect_ref().as_struct().expect("Outlets is a struct");

    for (offset_index, spec) in plan.fields[plan.inlet_field_count..].iter().enumerate() {
        let ordinal = plan.inlet_field_count + offset_index;
        let slot = plan.base + plan.field_offsets[ordinal];
        let value = outlets
            .field_at(spec.field_index)
            .expect("field_index came from this type's own fields");
        arena.values[slot] = clone_authored(value);

        if let FieldKind::Product { access, .. } = spec.kind {
            (access.set)(&mut *arena.values[slot], Some(plan.entity));
        }
    }
}

fn insert_defaults_of<N: NodeType>(world: &mut World, node: Entity) {
    if world.get::<N::Inlets>(node).is_none() {
        world.entity_mut(node).insert(N::Inlets::default());
    }
    if world.get::<N::State>(node).is_none() {
        world.entity_mut(node).insert(N::State::default());
    }
}

fn inlets_changed_tick_of<N: NodeType>(world: &World, node: Entity) -> Option<Tick> {
    world
        .get_entity(node)
        .ok()?
        .get_change_ticks::<N::Inlets>()
        .map(|t| t.changed)
}

fn inlet_lens_of<N: NodeType>(world: &World, node: Entity) -> Vec<usize> {
    let Some(inlets) = world.get::<N::Inlets>(node) else {
        return Vec::new();
    };
    let inlets: &dyn Struct = inlets.reflect_ref().as_struct().expect("Inlets is a struct");
    (0..inlets.field_len())
        .map(|i| {
            let field = inlets.field_at(i).expect("index below field_len");
            match field.reflect_ref().as_list() {
                Ok(list) => list.len(),
                Err(_) => 1,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::App;
    use bevy_ecs::component::Component;
    use bevy_reflect::Reflect;

    #[derive(Reflect, Component, Default)]
    struct ProbeInlets {
        gain: f32,
    }

    #[derive(Component, Default)]
    struct ProbeState;

    #[derive(Reflect, Default)]
    struct ProbeOutlets {
        bias: f32,
        value: f32,
    }

    struct Probe;

    impl Probe {
        const GAIN: u16 = 0;
        const BIAS: u16 = 1;
        const OUT_VALUE: u16 = 2;
    }

    impl NodeType for Probe {
        type Inlets = ProbeInlets;
        type Outlets = ProbeOutlets;
        type State = ProbeState;

        const ORDINALS: &'static [(&'static str, u16)] = &[
            ("gain", Probe::GAIN),
            ("bias", Probe::BIAS),
            ("value", Probe::OUT_VALUE),
        ];

        fn register(_app: &mut App) {}

        fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
    }

    /// A panic payload from `panic!("{}", format_args!(...))` (which is what
    /// `panic!("...{x}...")` desugars to) may arrive as `&str` rather than
    /// `String`, depending on how the panic message was constructed. Try
    /// both rather than assuming one.
    fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
        if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else {
            String::new()
        }
    }

    #[test]
    fn registration_derives_both_halves_and_lays_out_ordinals_per_kind() {
        let mut app = App::new();
        let id = register_node_type::<Probe>(&mut app);
        let reg = app.world().resource::<NodeTypeRegistry>();
        let entry = reg.get(id).expect("registered");

        assert_eq!(entry.inlets.len(), 1);
        assert_eq!(entry.outlets.len(), 2);
        assert_eq!(entry.field_count(), 3);
    }

    #[test]
    fn a_wrong_ordinal_fails_registration_and_names_the_field() {
        struct Bad;
        impl NodeType for Bad {
            type Inlets = ProbeInlets;
            type Outlets = ProbeOutlets;
            type State = ProbeState;
            // "bias" is ordinal 1, not 0. This is exactly the mistake a
            // field reorder makes, and it must not reach the tick loop.
            const ORDINALS: &'static [(&'static str, u16)] =
                &[("gain", 0), ("bias", 0), ("value", 2)];
            fn register(_app: &mut App) {}
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
        }

        let mut app = App::new();
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            register_node_type::<Bad>(&mut app)
        }))
        .unwrap_err();
        let msg = panic_message(&*err);
        assert!(msg.contains("bias"), "must name the field: {msg}");
        assert!(msg.contains('1'), "must state the correct ordinal: {msg}");
    }

    #[test]
    fn a_missing_ordinal_declaration_fails_registration() {
        // The other half of the guard: declaring fewer consts than there are
        // fields means some field has no name in node code at all.
        struct Incomplete;
        impl NodeType for Incomplete {
            type Inlets = ProbeInlets;
            type Outlets = ProbeOutlets;
            type State = ProbeState;
            const ORDINALS: &'static [(&'static str, u16)] = &[("gain", 0)];
            fn register(_app: &mut App) {}
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
        }

        let mut app = App::new();
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            register_node_type::<Incomplete>(&mut app)
        }))
        .unwrap_err();
        let msg = panic_message(&*err);
        assert!(msg.contains("bias") || msg.contains("undeclared"), "{msg}");
    }

    #[test]
    fn an_ordinal_naming_a_nonexistent_field_fails_registration() {
        // The third half of the guard: ORDINALS naming something that isn't
        // a field at all (typo, stale const after a field rename).
        struct Phantom;
        impl NodeType for Phantom {
            type Inlets = ProbeInlets;
            type Outlets = ProbeOutlets;
            type State = ProbeState;
            const ORDINALS: &'static [(&'static str, u16)] = &[
                ("gain", 0),
                ("bias", 1),
                ("value", 2),
                ("does_not_exist", 99),
            ];
            fn register(_app: &mut App) {}
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
        }

        let mut app = App::new();
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            register_node_type::<Phantom>(&mut app)
        }))
        .unwrap_err();
        let msg = panic_message(&*err);
        assert!(msg.contains("does_not_exist"), "must name the bogus entry: {msg}");
    }

    #[test]
    fn matching_inlet_and_outlet_names_are_checked_by_name_and_ordinal() {
        #[derive(Reflect, Component, Default)]
        struct SameNameInlets {
            value: f32,
        }

        #[derive(Reflect, Default)]
        struct SameNameOutlets {
            value: f32,
        }

        struct SameName;
        impl NodeType for SameName {
            type Inlets = SameNameInlets;
            type Outlets = SameNameOutlets;
            type State = ProbeState;
            const ORDINALS: &'static [(&'static str, u16)] = &[("value", 0), ("value", 1)];
            fn register(_app: &mut App) {}
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
        }

        let mut app = App::new();
        register_node_type::<SameName>(&mut app);
    }

    #[test]
    fn two_product_outlets_fail_registration() {
        // design §12: at most one product outlet per node, because
        // produced_change_tick is per node.
        use crate::ports::{Product, Spatial};
        use crate::schema::register_product;

        #[derive(Reflect, Default)]
        struct TwoProducts {
            a: Product<Spatial>,
            b: Product<Spatial>,
        }

        struct Bad;
        impl NodeType for Bad {
            type Inlets = ProbeInlets;
            type Outlets = TwoProducts;
            type State = ProbeState;
            const ORDINALS: &'static [(&'static str, u16)] =
                &[("gain", 0), ("a", 1), ("b", 2)];
            fn register(app: &mut App) {
                register_product::<Spatial>(app);
            }
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
        }

        let mut app = App::new();
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            register_node_type::<Bad>(&mut app)
        }))
        .unwrap_err();
        let msg = panic_message(&*err);
        assert!(msg.contains("at most one"), "{msg}");
    }

    #[test]
    fn a_vec_outlet_fails_registration() {
        #[derive(Reflect, Default)]
        struct VecOutlet {
            values: Vec<f32>,
        }

        struct Bad;
        impl NodeType for Bad {
            type Inlets = ProbeInlets;
            type Outlets = VecOutlet;
            type State = ProbeState;
            const ORDINALS: &'static [(&'static str, u16)] = &[("gain", 0), ("values", 1)];
            fn register(_app: &mut App) {}
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
        }

        let mut app = App::new();
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            register_node_type::<Bad>(&mut app)
        }))
        .unwrap_err();
        let msg = panic_message(&*err);
        assert!(msg.contains("values"), "must name the outlet: {msg}");
    }
}
