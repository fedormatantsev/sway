//! What the editor needs to highlight. Spec §3.3.
//!
//! Everything here is computed at rebuild — authoring time — so a show pays
//! nothing for it.

use bevy_ecs::entity::Entity;
use bevy_ecs::resource::Resource;

#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct GraphDiagnostics {
    /// Entities in a cycle. They still run, appended after the acyclic part.
    pub cycles: Vec<Entity>,
    /// `(producer, wire name)` — the producer lacks the wire's `Source`.
    pub missing_source: Vec<(Entity, &'static str)>,
    /// `(consumer, wire name)` — the consumer lacks the wire's `Target`.
    pub missing_target: Vec<(Entity, &'static str)>,
}

impl GraphDiagnostics {
    pub fn is_clean(&self) -> bool {
        self.cycles.is_empty() && self.missing_source.is_empty() && self.missing_target.is_empty()
    }
}
