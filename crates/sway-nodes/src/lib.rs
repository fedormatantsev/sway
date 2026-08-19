//! Built-in value nodes and the pure functions they evaluate.
//!
//! Render-coupled kinds (meshes, materials, scene nodes) live in
//! `sway-runtime`. MIDI time lives in `sway-midi`.

mod beat;
mod envelope;
mod lfo;
mod math;
pub mod nodes;

pub use beat::*;
pub use envelope::*;
pub use lfo::*;
pub use math::*;
pub use nodes::GraphNodesPlugin;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_defaults_are_the_first_variants() {
        assert_eq!(Waveform::default(), Waveform::Sine);
        assert_eq!(MathOp::default(), MathOp::Add);
        assert_eq!(NoteField::default(), NoteField::Note);
        assert_eq!(Division::default(), Division::Beat);
    }
}
