use bevy_ecs::resource::Resource;

/// MIDI channels, in the protocol's own 0–15 numbering.
const CHANNELS: usize = 16;
/// Controller numbers, 0–127.
const CONTROLLERS: usize = 128;

/// The last raw Control Change value seen on every channel/controller pair
/// this session, published for nodes to read during evaluation — the same
/// role [`Transport`](crate::Transport) plays for time (design D1).
///
/// The snapshot outlives a project: controller position is live hardware
/// state, not a document field, so opening another project does not clear it
/// (design D5).
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct MidiControls {
    values: [[u8; CONTROLLERS]; CHANNELS],
}

impl Default for MidiControls {
    fn default() -> Self {
        Self {
            values: [[0; CONTROLLERS]; CHANNELS],
        }
    }
}

impl MidiControls {
    /// The last raw value on `channel` / `cc`, 0 until a matching Control
    /// Change has arrived. Inlets are `f32` (design D3), so the address is
    /// truncated toward zero and then clamped — `as` does both, and maps NaN
    /// to 0.
    pub fn get(&self, channel: f32, cc: f32) -> u8 {
        let channel = (channel as i32).clamp(0, CHANNELS as i32 - 1) as usize;
        let cc = (cc as i32).clamp(0, CONTROLLERS as i32 - 1) as usize;
        self.values[channel][cc]
    }

    /// Records a Control Change. Last write wins. Addresses come from the
    /// parser's status nibble and data byte; clamping keeps a malformed data
    /// byte from indexing out of the table.
    pub fn set(&mut self, channel: u8, cc: u8, value: u8) {
        let channel = usize::from(channel).min(CHANNELS - 1);
        let cc = usize::from(cc).min(CONTROLLERS - 1);
        self.values[channel][cc] = value;
    }
}

#[cfg(test)]
mod tests {
    use super::MidiControls;

    #[test]
    fn every_cell_starts_at_zero() {
        let controls = MidiControls::default();
        assert_eq!(controls.get(0.0, 0.0), 0);
        assert_eq!(controls.get(15.0, 127.0), 0);
    }

    #[test]
    fn a_written_cell_reads_back() {
        let mut controls = MidiControls::default();
        controls.set(3, 74, 99);
        assert_eq!(controls.get(3.0, 74.0), 99);
    }

    #[test]
    fn the_last_write_wins() {
        let mut controls = MidiControls::default();
        controls.set(0, 1, 10);
        controls.set(0, 1, 20);
        assert_eq!(controls.get(0.0, 1.0), 20);
    }

    #[test]
    fn an_address_is_truncated_toward_zero_then_clamped() {
        let mut controls = MidiControls::default();
        controls.set(0, 0, 3);
        controls.set(0, 1, 42);
        controls.set(15, 127, 7);

        assert_eq!(controls.get(0.9, 1.9), 42, "truncated, not rounded");
        assert_eq!(controls.get(20.0, 200.0), 7, "clamped to 15 / 127");
        assert_eq!(controls.get(-3.0, -3.0), 3, "clamped to 0 / 0");
    }

    #[test]
    fn a_nan_address_reads_channel_zero_controller_zero() {
        let mut controls = MidiControls::default();
        controls.set(0, 0, 55);
        assert_eq!(controls.get(f32::NAN, f32::NAN), 55);
    }

    #[test]
    fn a_malformed_data_byte_clamps_instead_of_panicking() {
        let mut controls = MidiControls::default();
        controls.set(200, 200, 5);
        assert_eq!(controls.get(15.0, 127.0), 5);
    }
}
