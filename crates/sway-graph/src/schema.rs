//! Deriving a node's port schema from its `Params` / `Outputs` types.
//!
//! Spec §3: the schema is derived from the types, never written beside them.
//! A plain field is a continuous port; a field typed `Event<T>` is an event
//! port whose payload is `T`.

use core::any::TypeId;
use core::fmt;

use bevy_app::App;
use bevy_reflect::{FromType, Reflect, TypePath, TypeRegistry, Typed};

use crate::ports::Event;

/// Type data marking a type as an event-port marker, and carrying the
/// payload type the port actually transports.
#[derive(Clone)]
pub struct ReflectEventPort {
    pub payload: TypeId,
    pub payload_path: &'static str,
}

impl<T: Reflect + TypePath> FromType<Event<T>> for ReflectEventPort {
    fn from_type() -> Self {
        Self {
            payload: TypeId::of::<T>(),
            payload_path: T::type_path(),
        }
    }
}

/// Registers `Event<T>` and its `ReflectEventPort` data. A node type with an
/// `Event<T>` port must call this in its `register`.
pub fn register_event_port<T>(app: &mut App)
where
    T: Reflect + TypePath + Typed + bevy_reflect::FromReflect + bevy_reflect::GetTypeRegistration,
{
    let registry = app
        .world()
        .resource::<bevy_ecs::reflect::AppTypeRegistry>()
        .clone();
    let mut registry = registry.write();
    registry.register::<T>();
    registry.register::<Event<T>>();
    registry.register_type_data::<Event<T>, ReflectEventPort>();
}

/// One port, as derived from one struct field.
#[derive(Clone, Debug, PartialEq)]
pub struct PortField {
    pub name: &'static str,
    /// Index of the field in the reflect struct — what `Struct::field_at`
    /// takes. Not the port ordinal, which is per-kind.
    pub field_index: usize,
    /// For a continuous port, the field's type. For an event port, the
    /// *payload* type, so edge validation compares like with like.
    pub type_id: TypeId,
    pub type_path: &'static str,
}

/// The ports derived from one struct — either a node's inputs (`Params`) or
/// its outputs (`Outputs`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SchemaHalf {
    pub continuous: Vec<PortField>,
    pub events: Vec<PortField>,
}

#[derive(Debug)]
pub enum SchemaError {
    NotAStruct { type_path: &'static str },
    UnregisteredEventField { type_path: &'static str, field: &'static str },
    UnregisteredSlotField {
        type_path: &'static str,
        field: &'static str,
    },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAStruct { type_path } => write!(
                f,
                "`{type_path}` must be a struct to derive a port schema from it"
            ),
            Self::UnregisteredEventField { type_path, field } => write!(
                f,
                "`{type_path}.{field}` looks like an event port but its type is not \
                 registered as one — call `register_event_port::<Payload>(app)` in \
                 this node type's `register`"
            ),
            Self::UnregisteredSlotField { type_path, field } => write!(
                f,
                "`{type_path}.{field}` looks like a Feeds slot but its type is not \
                 registered as one — call `register_slot::<Capability>(app)` in this \
                 node type's `register`"
            ),
        }
    }
}

impl core::error::Error for SchemaError {}

pub fn derive_schema<T: Typed>(registry: &TypeRegistry) -> Result<SchemaHalf, SchemaError> {
    let info = T::type_info();
    let s = info.as_struct().map_err(|_| SchemaError::NotAStruct {
        type_path: info.type_path(),
    })?;

    let mut half = SchemaHalf::default();
    for i in 0..s.field_len() {
        let field = s.field_at(i).expect("index below field_len");
        match registry.get_type_data::<ReflectEventPort>(field.type_id()) {
            Some(ev) => half.events.push(PortField {
                name: field.name(),
                field_index: i,
                type_id: ev.payload,
                type_path: ev.payload_path,
            }),
            None => {
                // An `Event<_>` field whose type data is missing would land
                // here and become a continuous port of a zero-sized type,
                // which no edge could usefully drive. Catch it by type path
                // rather than letting it through silently.
                if is_event_marker_path(field.type_path()) {
                    return Err(SchemaError::UnregisteredEventField {
                        type_path: info.type_path(),
                        field: field.name(),
                    });
                }
                half.continuous.push(PortField {
                    name: field.name(),
                    field_index: i,
                    type_id: field.type_id(),
                    type_path: field.type_path(),
                });
            }
        }
    }
    Ok(half)
}

/// Recognises `sway_graph::ports::Event<..>` by path.
///
/// This exists only to turn a forgotten `register_event_port` into a clear
/// error instead of a silently useless port. The *authoritative* test for an
/// event port is the `ReflectEventPort` type data above; this is the
/// diagnostic for its absence.
fn is_event_marker_path(path: &str) -> bool {
    path.starts_with("sway_graph::ports::Event<")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::Event;
    use bevy_reflect::{Reflect, TypeRegistry};

    #[derive(Reflect, Default, Debug, Clone, PartialEq)]
    struct NoteMsg {
        note: u8,
        velocity: u8,
    }

    #[derive(Reflect, Default)]
    struct MixedParams {
        hz: f32,
        trigger: Event<NoteMsg>,
        amplitude: f32,
    }

    fn registry() -> TypeRegistry {
        let mut r = TypeRegistry::new();
        r.register::<NoteMsg>();
        r.register::<MixedParams>();
        r.register::<Event<NoteMsg>>();
        r.register_type_data::<Event<NoteMsg>, ReflectEventPort>();
        r
    }

    #[test]
    fn field_type_decides_port_kind_and_ordinals_are_per_kind() {
        let s = derive_schema::<MixedParams>(&registry()).expect("schema");

        // Spec §3: field type decides kind. Spec §4: ordinals are per-kind,
        // so `amplitude` is continuous #1 even though it is field #2.
        assert_eq!(
            s.continuous.iter().map(|f| f.name).collect::<Vec<_>>(),
            vec!["hz", "amplitude"]
        );
        assert_eq!(s.continuous[1].field_index, 2);
        assert_eq!(s.events.iter().map(|f| f.name).collect::<Vec<_>>(), vec!["trigger"]);
    }

    #[test]
    fn event_port_carries_its_payload_type_not_the_marker_type() {
        let s = derive_schema::<MixedParams>(&registry()).expect("schema");

        // Edge validation compares payloads, so the schema must report
        // NoteMsg here — not Event<NoteMsg>.
        assert_eq!(s.events[0].type_id, core::any::TypeId::of::<NoteMsg>());
        assert_ne!(s.events[0].type_id, core::any::TypeId::of::<Event<NoteMsg>>());
    }

    #[test]
    fn an_unregistered_event_field_is_an_error_not_a_continuous_port() {
        // The failure this prevents: a node author adds an Event<T> field but
        // forgets register_event_port, and it silently becomes a continuous
        // port of a zero-sized type that no edge can ever usefully drive.
        let mut r = TypeRegistry::new();
        r.register::<NoteMsg>();
        r.register::<MixedParams>();
        r.register::<Event<NoteMsg>>();
        // deliberately NOT register_type_data::<_, ReflectEventPort>

        let err = derive_schema::<MixedParams>(&r).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("trigger"), "message must name the field: {msg}");
        assert!(msg.contains("register_event_port"), "message must say the fix: {msg}");
    }

    #[test]
    fn a_non_struct_params_type_is_rejected() {
        let mut r = TypeRegistry::new();
        r.register::<f32>();
        let err = derive_schema::<f32>(&r).unwrap_err();
        assert!(err.to_string().contains("struct"), "{err}");
    }
}
