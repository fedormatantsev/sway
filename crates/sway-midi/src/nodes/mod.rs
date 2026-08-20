//! Graph-model MIDI nodes.
//!
//! Registered by [`MidiPlugin`](crate::MidiPlugin): a host adds that and
//! nothing else from this crate.

pub mod midi_cc;
pub mod midi_time;

pub use midi_cc::{MidiCc, MidiCcIn, MidiCcOut};
pub use midi_time::{MidiTime, MidiTimeOut};
