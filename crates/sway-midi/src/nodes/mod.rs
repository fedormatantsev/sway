//! Graph-model MIDI nodes.
//!
//! Registered by [`MidiPlugin`](crate::MidiPlugin): a host adds that and
//! nothing else from this crate.

pub mod midi_cc;
pub mod midi_notes;
pub mod midi_time;
pub mod on_midi_note;

pub use midi_cc::{MidiCc, MidiCcIn, MidiCcOut};
pub use midi_notes::{MidiNotes, MidiNotesIn, MidiNotesOut, NoteEvent};
pub use midi_time::{MidiTime, MidiTimeOut};
pub use on_midi_note::{OnMidiNote, OnMidiNoteIn, OnMidiNoteOut, parse_note_name};
