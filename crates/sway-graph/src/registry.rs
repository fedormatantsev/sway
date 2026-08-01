//! The node type registry. Spec §3.
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
use bevy_reflect::{GetTypeRegistration, PartialReflect, Reflect, TypePath, Typed};
use std::collections::HashMap;

use crate::compile::NodePlan;
use crate::ports::PortArena;
use crate::schema::{derive_schema, SchemaHalf};
use crate::view::{PortView, TickCtx};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct NodeTypeId(pub u32);

pub type TickFn = fn(&mut World, Entity, &mut PortView, &TickCtx);
pub type PrefillFn = fn(&World, Entity, &mut PortArena, &NodePlan);
pub type SeedOutputsFn = fn(&mut PortArena, &NodePlan);
pub type InsertDefaultsFn = fn(&mut World, Entity);
pub type TickOfFn = fn(&World, Entity) -> Option<Tick>;
pub type CookFn = fn(&mut World, Entity, &crate::view::SlotView);
pub type ProducedTickFn = fn(&World, Entity) -> Option<Tick>;

pub trait NodeType: 'static {
    type Params: Reflect + Typed + GetTypeRegistration + Component + Default;
    type Outputs: Reflect + Typed + GetTypeRegistration + Default;
    /// Named, typed `Feeds` inputs. `NoSlots` when the node has none (§4).
    type Slots: Reflect + Typed + GetTypeRegistration + Default;
    /// The capability a `Feeds` edge *from* this node carries. `()` means the
    /// node cannot be a `Feeds` source. Bounded `TypePath`, not `Reflect`:
    /// the structure pass needs identity and a name, nothing more (§4).
    type Produces: TypePath + Send + Sync + 'static;
    type State: Component + Default;

    /// `(field name, the ordinal the node's index const uses)` for every
    /// port. Verified against the reflect-derived schema at registration, so
    /// a field reorder fails at startup instead of silently swapping two
    /// ports (spec §3).
    const PORT_ORDINALS: &'static [(&'static str, u16)];
    /// The same guard for slots. Empty for a node with no slots.
    const SLOT_ORDINALS: &'static [(&'static str, u16)] = &[];
    /// Does this node carry a `Transform`, i.e. may it appear in the scene
    /// tree? Parenting a non-spatial node is a compile error (§4).
    const SPATIAL: bool = false;
    /// Whether `cook` is meaningful. Rust cannot distinguish a defaulted
    /// trait method from an overridden one, and the gate needs to know
    /// whether a node has a cook at all rather than calling an empty one on
    /// every dirty node (§4).
    const COOKS: bool = false;

    fn register(app: &mut App);
    fn tick(world: &mut World, node: Entity, ports: &mut PortView, t: &TickCtx);

    /// Reads this node's `Feeds` sources and writes its own product. Runs in
    /// `cook_order`, only when the gate says the node is dirty (§6, §7).
    fn cook(_world: &mut World, _node: Entity, _slots: &crate::view::SlotView) {}

    /// The change tick of whatever this node's `Feeds` consumers depend on.
    ///
    /// `None` — the default — means "changes to what I produce do not require
    /// my consumers to re-cook", which is correct for a material node: its
    /// consumers hold its `Handle`, and editing the material's params does
    /// not change the handle. A geometry operator overrides this with its own
    /// `Geometry` change tick. Keeping it node-supplied is what lets the
    /// engine gate on geometry without `sway-graph` knowing `Geometry`
    /// exists (§6).
    fn produced_change_tick(_world: &World, _node: Entity) -> Option<Tick> {
        None
    }
}

#[derive(Clone, Debug, Default)]
pub struct NodeSchema {
    pub inputs: SchemaHalf,
    pub outputs: SchemaHalf,
}

impl NodeSchema {
    /// Continuous inputs then continuous outputs, contiguous (spec §4).
    pub fn continuous_len(&self) -> usize {
        self.inputs.continuous.len() + self.outputs.continuous.len()
    }
    pub fn events_len(&self) -> usize {
        self.inputs.events.len() + self.outputs.events.len()
    }
}

pub struct NodeTypeEntry {
    pub name: &'static str,
    pub schema: NodeSchema,
    pub tick: TickFn,
    pub prefill: PrefillFn,
    pub seed_outputs: SeedOutputsFn,
    pub insert_defaults: InsertDefaultsFn,
    pub params_changed_tick: TickOfFn,
    pub slots: Vec<crate::slots::SlotField>,
    pub produces: core::any::TypeId,
    pub produces_path: &'static str,
    pub spatial: bool,
    /// `Some` iff `N::COOKS`.
    pub cook: Option<CookFn>,
    pub produced_change_tick: ProducedTickFn,
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
        w.register::<N::Params>();
        w.register::<N::Outputs>();
        w.register::<N::Slots>();
    }

    let schema = {
        let registry = app.world().resource::<AppTypeRegistry>().clone();
        let r = registry.read();
        NodeSchema {
            inputs: derive_schema::<N::Params>(&r)
                .unwrap_or_else(|e| panic!("{}: {e}", core::any::type_name::<N>())),
            outputs: derive_schema::<N::Outputs>(&r)
                .unwrap_or_else(|e| panic!("{}: {e}", core::any::type_name::<N>())),
        }
    };

    check_ordinals::<N>(&schema);

    let slots = {
        let registry = app.world().resource::<AppTypeRegistry>().clone();
        let r = registry.read();
        crate::slots::derive_slots::<N::Slots>(&r)
            .unwrap_or_else(|e| panic!("{}: {e}", core::any::type_name::<N>()))
    };

    check_slot_ordinals::<N>(&slots);

    let entry = NodeTypeEntry {
        name: core::any::type_name::<N>(),
        schema,
        tick: N::tick,
        prefill: prefill_of::<N>,
        seed_outputs: seed_outputs_of::<N>,
        insert_defaults: insert_defaults_of::<N>,
        params_changed_tick: params_changed_tick_of::<N>,
        slots,
        produces: core::any::TypeId::of::<N::Produces>(),
        produces_path: <N::Produces as TypePath>::type_path(),
        spatial: N::SPATIAL,
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

/// Spec §3's startup guard: the node's index consts must agree with the
/// reflect-derived per-kind ordinals.
fn check_ordinals<N: NodeType>(schema: &NodeSchema) {
    let node = core::any::type_name::<N>();
    let mut expected: Vec<(&'static str, u16)> = Vec::new();
    for (i, f) in schema.inputs.continuous.iter().enumerate() {
        expected.push((f.name, i as u16));
    }
    for (i, f) in schema.outputs.continuous.iter().enumerate() {
        expected.push((f.name, (schema.inputs.continuous.len() + i) as u16));
    }
    for (i, f) in schema.inputs.events.iter().enumerate() {
        expected.push((f.name, i as u16));
    }
    for (i, f) in schema.outputs.events.iter().enumerate() {
        expected.push((f.name, (schema.inputs.events.len() + i) as u16));
    }

    let mut matched = vec![false; N::PORT_ORDINALS.len()];
    for &(name, want) in &expected {
        if let Some((index, _)) = N::PORT_ORDINALS
            .iter()
            .enumerate()
            .find(|(index, entry)| !matched[*index] && **entry == (name, want))
        {
            matched[index] = true;
            continue;
        }
        match N::PORT_ORDINALS
            .iter()
            .enumerate()
            .find(|(index, (declared, _))| !matched[*index] && declared == &name)
        {
            Some((_, (_, got))) => panic!(
                "{node}: port `{name}` is ordinal {want}, but PORT_ORDINALS declares {got} \
                 — a field was reordered, or the const is stale"
            ),
            None => panic!(
                "{node}: port `{name}` is undeclared in PORT_ORDINALS (expected ordinal {want})"
            ),
        }
    }
    for (index, (name, _)) in N::PORT_ORDINALS.iter().enumerate() {
        if !matched[index] {
            panic!("{node}: PORT_ORDINALS declares `{name}`, which is not a port");
        }
    }
}

/// The slot half of §3's startup guard. Slots occupy one flat space in field
/// order, so this is simpler than `check_ordinals` — but the hazard is the
/// same, and a swapped slot silently feeds a material into a geometry input.
fn check_slot_ordinals<N: NodeType>(slots: &[crate::slots::SlotField]) {
    let node = core::any::type_name::<N>();
    for (ordinal, slot) in slots.iter().enumerate() {
        let want = ordinal as u16;
        match N::SLOT_ORDINALS.iter().find(|(name, _)| *name == slot.name) {
            Some(&(_, got)) if got == want => {}
            Some(&(_, got)) => panic!(
                "{node}: slot `{}` is ordinal {want}, but SLOT_ORDINALS declares {got} \
                 — a field was reordered, or the const is stale",
                slot.name
            ),
            None => panic!(
                "{node}: slot `{}` is undeclared in SLOT_ORDINALS (expected ordinal {want})",
                slot.name
            ),
        }
    }
    for (name, _) in N::SLOT_ORDINALS {
        if !slots.iter().any(|s| s.name == *name) {
            panic!("{node}: SLOT_ORDINALS declares `{name}`, which is not a slot");
        }
    }
}

/// Copies every **unconnected** continuous input from the node's `Params`
/// component into its arena slots. Spec §4's authored-versus-driven rule:
/// `Params` is never written by the graph, so a disconnect returns the port
/// to its authored value.
fn prefill_of<N: NodeType>(world: &World, node: Entity, arena: &mut PortArena, plan: &NodePlan) {
    let Some(params) = world.get::<N::Params>(node) else {
        return;
    };
    let params: &dyn Struct = params.reflect_ref().as_struct().expect("Params is a struct");
    for (ordinal, field) in plan.schema.inputs.continuous.iter().enumerate() {
        if plan.connected_continuous[ordinal] {
            continue;
        }
        let value = params
            .field_at(field.field_index)
            .expect("field_index came from this type's own schema");
        arena.continuous[plan.continuous_base + ordinal] = value
            .reflect_clone()
            .unwrap_or_else(|error| {
                panic!(
                    "could not clone `{}` while prefilling a node parameter: {error:?}",
                    value.reflect_type_path()
                )
            })
            .into_partial_reflect();
    }
}

fn seed_outputs_of<N: NodeType>(arena: &mut PortArena, plan: &NodePlan) {
    let outputs = N::Outputs::default();
    let outputs: &dyn Struct = outputs.reflect_ref().as_struct().expect("Outputs is a struct");
    let output_base = plan.continuous_base + plan.schema.inputs.continuous.len();
    for (ordinal, field) in plan.schema.outputs.continuous.iter().enumerate() {
        let value = outputs
            .field_at(field.field_index)
            .expect("field_index came from this type's own schema");
        arena.continuous[output_base + ordinal] = value
            .reflect_clone()
            .unwrap_or_else(|error| {
                panic!(
                    "could not clone `{}` while seeding a node output: {error:?}",
                    value.reflect_type_path()
                )
            })
            .into_partial_reflect();
    }
}

fn insert_defaults_of<N: NodeType>(world: &mut World, node: Entity) {
    if world.get::<N::Params>(node).is_none() {
        world.entity_mut(node).insert(N::Params::default());
    }
    if world.get::<N::State>(node).is_none() {
        world.entity_mut(node).insert(N::State::default());
    }
}

fn params_changed_tick_of<N: NodeType>(world: &World, node: Entity) -> Option<Tick> {
    world
        .get_entity(node)
        .ok()?
        .get_change_ticks::<N::Params>()
        .map(|t| t.changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::Event;
    use bevy_app::App;
    use bevy_ecs::component::Component;
    use bevy_reflect::Reflect;

    #[derive(Reflect, Default, Debug, Clone, PartialEq)]
    struct NoteMsg {
        note: u8,
        velocity: u8,
    }

    #[derive(Reflect, Component, Default)]
    struct ProbeParams {
        gain: f32,
        trigger: Event<NoteMsg>,
        bias: f32,
    }

    #[derive(Reflect, Default)]
    struct ProbeOut {
        value: f32,
    }

    #[derive(Component, Default)]
    struct ProbeState;

    struct Probe;

    impl Probe {
        const GAIN: u16 = 0;
        const BIAS: u16 = 1;
        const OUT_VALUE: u16 = 2; // inputs then outputs, within the kind
        const TRIGGER: u16 = 0; // event space is separate
    }

    impl NodeType for Probe {
        type Params = ProbeParams;
        type Outputs = ProbeOut;
        type Slots = crate::slots::NoSlots;
        type Produces = ();
        type State = ProbeState;

        const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
            ("gain", Probe::GAIN),
            ("bias", Probe::BIAS),
            ("value", Probe::OUT_VALUE),
            ("trigger", Probe::TRIGGER),
        ];

        fn register(app: &mut App) {
            crate::schema::register_event_port::<NoteMsg>(app);
        }

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

        assert_eq!(entry.schema.inputs.continuous.len(), 2);
        assert_eq!(entry.schema.inputs.events.len(), 1);
        assert_eq!(entry.schema.outputs.continuous.len(), 1);
        // Spec §4: continuous inputs then continuous outputs, contiguous.
        assert_eq!(entry.schema.continuous_len(), 3);
        assert_eq!(entry.schema.events_len(), 1);
    }

    #[test]
    fn a_wrong_ordinal_fails_registration_and_names_the_field() {
        struct Bad;
        impl NodeType for Bad {
            type Params = ProbeParams;
            type Outputs = ProbeOut;
            type Slots = crate::slots::NoSlots;
            type Produces = ();
            type State = ProbeState;
            // "bias" is continuous #1, not #0. This is exactly the mistake a
            // field reorder makes, and it must not reach the tick loop.
            const PORT_ORDINALS: &'static [(&'static str, u16)] =
                &[("gain", 0), ("bias", 0), ("value", 2), ("trigger", 0)];
            fn register(app: &mut App) {
                crate::schema::register_event_port::<NoteMsg>(app);
            }
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
        // ports means some port has no name in node code at all.
        struct Incomplete;
        impl NodeType for Incomplete {
            type Params = ProbeParams;
            type Outputs = ProbeOut;
            type Slots = crate::slots::NoSlots;
            type Produces = ();
            type State = ProbeState;
            const PORT_ORDINALS: &'static [(&'static str, u16)] = &[("gain", 0)];
            fn register(app: &mut App) {
                crate::schema::register_event_port::<NoteMsg>(app);
            }
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
    fn an_ordinal_naming_a_nonexistent_port_fails_registration() {
        // The third half of the guard: PORT_ORDINALS naming something that
        // isn't a port at all (typo, stale const after a field rename).
        struct Phantom;
        impl NodeType for Phantom {
            type Params = ProbeParams;
            type Outputs = ProbeOut;
            type Slots = crate::slots::NoSlots;
            type Produces = ();
            type State = ProbeState;
            const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
                ("gain", 0),
                ("bias", 1),
                ("value", 2),
                ("trigger", 0),
                ("does_not_exist", 99),
            ];
            fn register(app: &mut App) {
                crate::schema::register_event_port::<NoteMsg>(app);
            }
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
    fn matching_input_and_output_names_are_checked_by_name_and_ordinal() {
        #[derive(Reflect, Component, Default)]
        struct SameNameParams {
            value: f32,
        }

        #[derive(Reflect, Default)]
        struct SameNameOutputs {
            value: f32,
        }

        struct SameName;
        impl NodeType for SameName {
            type Params = SameNameParams;
            type Outputs = SameNameOutputs;
            type Slots = crate::slots::NoSlots;
            type Produces = ();
            type State = ProbeState;
            const PORT_ORDINALS: &'static [(&'static str, u16)] = &[("value", 0), ("value", 1)];
            fn register(_app: &mut App) {}
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
        }

        let mut app = App::new();
        register_node_type::<SameName>(&mut app);
    }

    #[test]
    fn a_node_type_registers_its_slots_capability_and_cook_flag() {
        use crate::slots::{NoSlots, Slot, register_slot};
        use bevy_reflect::TypePath;

        #[derive(TypePath)]
        struct Blob;

        #[derive(Reflect, Default)]
        struct ConsumerSlots {
            input: Slot<Blob>,
        }

        struct Producer;
        impl NodeType for Producer {
            type Params = ProbeParams;
            type Outputs = ProbeOut;
            type Slots = NoSlots;
            type Produces = Blob;
            type State = ProbeState;
            const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
                ("gain", 0),
                ("bias", 1),
                ("value", 2),
                ("trigger", 0),
            ];
            const COOKS: bool = true;
            fn register(app: &mut App) {
                crate::schema::register_event_port::<NoteMsg>(app);
            }
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
            fn cook(_w: &mut World, _n: Entity, _s: &crate::view::SlotView) {}
        }

        struct Consumer;
        impl NodeType for Consumer {
            type Params = ProbeParams;
            type Outputs = ProbeOut;
            type Slots = ConsumerSlots;
            type Produces = ();
            type State = ProbeState;
            const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
                ("gain", 0),
                ("bias", 1),
                ("value", 2),
                ("trigger", 0),
            ];
            const SLOT_ORDINALS: &'static [(&'static str, u16)] = &[("input", 0)];
            const SPATIAL: bool = true;
            fn register(app: &mut App) {
                crate::schema::register_event_port::<NoteMsg>(app);
                register_slot::<Blob>(app);
            }
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
        }

        let mut app = App::new();
        let producer = register_node_type::<Producer>(&mut app);
        let consumer = register_node_type::<Consumer>(&mut app);
        let reg = app.world().resource::<NodeTypeRegistry>();

        let p = reg.get(producer).expect("registered");
        assert!(p.slots.is_empty());
        assert_eq!(p.produces, core::any::TypeId::of::<Blob>());
        assert!(p.cook.is_some(), "COOKS = true must store the cook fn");
        assert!(!p.spatial);

        let c = reg.get(consumer).expect("registered");
        assert_eq!(c.slots.len(), 1);
        assert_eq!(c.slots[0].name, "input");
        assert_eq!(c.slots[0].capability, core::any::TypeId::of::<Blob>());
        assert_eq!(c.produces, core::any::TypeId::of::<()>());
        assert!(c.cook.is_none(), "COOKS defaults false — no cook stored");
        assert!(c.spatial);
    }

    #[test]
    fn a_wrong_slot_ordinal_fails_registration_and_names_the_slot() {
        use crate::slots::{Slot, register_slot};
        use bevy_reflect::TypePath;

        #[derive(TypePath)]
        struct Blob;

        #[derive(Reflect, Default)]
        struct TwoSlots {
            first: Slot<Blob>,
            second: Slot<Blob>,
        }

        struct Bad;
        impl NodeType for Bad {
            type Params = ProbeParams;
            type Outputs = ProbeOut;
            type Slots = TwoSlots;
            type Produces = ();
            type State = ProbeState;
            const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
                ("gain", 0),
                ("bias", 1),
                ("value", 2),
                ("trigger", 0),
            ];
            // `second` is slot #1, not #0 — the mistake a field reorder makes.
            const SLOT_ORDINALS: &'static [(&'static str, u16)] =
                &[("first", 0), ("second", 0)];
            fn register(app: &mut App) {
                crate::schema::register_event_port::<NoteMsg>(app);
                register_slot::<Blob>(app);
            }
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
        }

        let mut app = App::new();
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            register_node_type::<Bad>(&mut app)
        }))
        .unwrap_err();
        let msg = panic_message(&*err);
        assert!(msg.contains("second"), "must name the slot: {msg}");
    }
}
