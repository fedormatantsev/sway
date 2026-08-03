//! `PortView` — a node's scoped window onto the arena.
//!
//! Indices are the node's own **field ordinals** (`Gain::GAIN`,
//! `Emitter::OUT_PULSE`, ...) and, for a `Vec` field, an element index.
//! `PortView` resolves them against the node's own base internally, which is
//! what stops a node reaching another node's slots by arithmetic.

use bevy_ecs::entity::Entity;
use bevy_reflect::Reflect;

use crate::ports::{Events, Occurrence, PortArena};
use crate::schema::{FieldKind, FieldSpec};

/// Context shared by every node ticked this frame.
pub struct TickCtx {
    /// The fixed timestep, in seconds.
    pub dt: f32,
    /// Absolute start of this tick's window, in seconds.
    pub tick_start: f64,
    /// Monotonically increasing tick counter, starting at 0.
    pub tick_index: u64,
}

/// Scoped to one node: field ordinals are resolved against its base here.
pub struct PortView<'a> {
    arena: &'a mut PortArena,
    base: usize,
    fields: &'a [FieldSpec],
    field_offsets: &'a [usize],
    field_lens: &'a [usize],
    connected: &'a [bool],
}

impl<'a> PortView<'a> {
    pub fn new(
        arena: &'a mut PortArena,
        base: usize,
        fields: &'a [FieldSpec],
        field_offsets: &'a [usize],
        field_lens: &'a [usize],
        connected: &'a [bool],
    ) -> Self {
        Self { arena, base, fields, field_offsets, field_lens, connected }
    }

    fn slot(&self, field: u16, index: u16) -> usize {
        let f = field as usize;
        assert!(
            f < self.field_lens.len(),
            "PortView: field ordinal {field} is out of range for this node's {} fields",
            self.field_lens.len()
        );
        let len = self.field_lens[f];
        assert!(
            (index as usize) < len,
            "PortView: element {index} is out of range for field `{}`, which has {len} slot(s)",
            self.fields[f].name
        );
        self.base + self.field_offsets[f] + index as usize
    }

    /// How many slots a field has: 1, or the instance's `Vec` length.
    pub fn len(&self, field: u16) -> usize {
        self.field_lens[field as usize]
    }

    pub fn is_empty(&self, field: u16) -> bool {
        self.len(field) == 0
    }

    /// Whether an edge drives this slot. False means it holds its authored
    /// value.
    pub fn is_connected(&self, field: u16, index: u16) -> bool {
        let slot = self.slot(field, index) - self.base;
        self.connected.get(slot).copied().unwrap_or(false)
    }

    /// Reads a non-`Vec` field's value.
    ///
    /// A compiled graph guarantees the slot holds exactly `T`, so a downcast
    /// failure here means the compiler failed to catch a type mismatch. The
    /// panic is deliberate: the tick is documented infallible for valid
    /// graphs.
    pub fn read<T: Reflect + Clone>(&self, field: u16) -> T {
        self.read_at(field, 0)
    }

    pub fn read_at<T: Reflect + Clone>(&self, field: u16, index: u16) -> T {
        let slot = self.slot(field, index);
        self.arena.values[slot]
            .try_downcast_ref::<T>()
            .unwrap_or_else(|| {
                panic!(
                    "PortView::read: field `{}`[{index}] does not hold a `{}` — the compiler \
                     should have caught this type mismatch before the tick ran",
                    self.fields[field as usize].name,
                    core::any::type_name::<T>()
                )
            })
            .clone()
    }

    /// Overwrites a non-`Vec` field's slot. Immediate — a node later in
    /// compiled order sees this within the same tick.
    pub fn write<T: Reflect>(&mut self, field: u16, value: T) {
        self.write_at(field, 0, value);
    }

    pub fn write_at<T: Reflect>(&mut self, field: u16, index: u16, value: T) {
        let slot = self.slot(field, index);
        self.arena.values[slot] = Box::new(value);
    }

    /// This tick's occurrences on an event field. Empty if nothing arrived.
    pub fn events<T: Reflect>(&self, field: u16) -> &[Occurrence<T>] {
        self.events_at(field, 0)
    }

    pub fn events_at<T: Reflect>(&self, field: u16, index: u16) -> &[Occurrence<T>] {
        let slot = self.slot(field, index);
        &self.arena.values[slot]
            .try_downcast_ref::<Events<T>>()
            .unwrap_or_else(|| {
                panic!(
                    "PortView::events: field `{}`[{index}] does not hold an `Events<{}>`",
                    self.fields[field as usize].name,
                    core::any::type_name::<T>()
                )
            })
            .occurrences
    }

    /// Appends an occurrence to an event field's slot for this tick.
    pub fn emit<T: Reflect>(&mut self, field: u16, offset: f32, value: T) {
        let slot = self.slot(field, 0);
        self.arena.values[slot]
            .try_downcast_mut::<Events<T>>()
            .unwrap_or_else(|| {
                panic!(
                    "PortView::emit: field `{}` does not hold an `Events<{}>`",
                    self.fields[field as usize].name,
                    core::any::type_name::<T>()
                )
            })
            .occurrences
            .push(Occurrence { offset, value });
    }

    /// The entity feeding a `Product` field's slot, or `None` if unconnected.
    pub fn source(&self, field: u16, index: u16) -> Option<Entity> {
        let slot = self.slot(field, index);
        let FieldKind::Product { access, .. } = self.fields[field as usize].kind else {
            panic!(
                "PortView::source: field `{}` is not a product",
                self.fields[field as usize].name
            );
        };
        (access.get)(&*self.arena.values[slot])
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use super::*;
    use crate::ports::PortArena;
    use crate::schema::derive_fields;
    use crate::test_nodes::GainInlets;
    use bevy_reflect::TypeRegistry;

    fn gain_fields() -> Vec<FieldSpec> {
        let mut registry = TypeRegistry::new();
        registry.register::<GainInlets>();
        derive_fields::<GainInlets>(&registry).expect("fields")
    }

    #[test]
    fn an_out_of_range_field_cannot_cross_a_node_boundary() {
        let mut arena = PortArena::new(4);
        arena.values[3] = Box::new(41.0_f32);
        let fields = gain_fields();

        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut view = PortView::new(&mut arena, 0, &fields, &[0, 1], &[1, 1], &[false, false]);
            view.write(9, 99.0_f32);
        }));

        assert!(result.is_err(), "a field outside the node must panic");
        assert_eq!(
            arena.values[3].try_downcast_ref::<f32>(),
            Some(&41.0),
            "another node's slot must remain untouched"
        );
    }

    #[test]
    fn an_out_of_range_element_cannot_cross_a_node_boundary() {
        let mut arena = PortArena::new(4);
        arena.values[3] = Box::new(41.0_f32);
        let fields = gain_fields();

        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut view = PortView::new(&mut arena, 0, &fields, &[0, 1], &[1, 1], &[false, false]);
            // `gain` has one slot; element 2 is past it and into `bias`.
            view.write_at(0, 2, 99.0_f32);
        }));

        assert!(result.is_err(), "an element past a field's length must panic");
        assert_eq!(arena.values[3].try_downcast_ref::<f32>(), Some(&41.0));
    }
}
