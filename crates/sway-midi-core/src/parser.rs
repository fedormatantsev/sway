//! A MIDI byte-stream parser.
//!
//! M0 walked each `MIDIPacket` in fixed three-byte strides. That is wrong for
//! three separate reasons and each one costs this milestone: System Real-Time
//! messages are one byte and may appear *between the bytes of another
//! message*, Program Change and Channel Pressure carry one data byte, and
//! running status omits repeated status bytes entirely. Clock, start, stop and
//! continue are all System Real-Time, so under the old stride the transport
//! received nothing at all.
//!
//! State is per packet, not per connection: CoreMIDI packets hold complete
//! messages (SysEx excepted, and skipped here), and one parser shared across
//! the input port and the virtual destination would let two sources corrupt
//! each other's running status.

/// MIDI clock, 24 per quarter note.
pub const CLOCK: u8 = 0xF8;
/// Start playback from the beginning.
pub const START: u8 = 0xFA;
/// Resume playback from the current position.
pub const CONTINUE: u8 = 0xFB;
/// Stop playback.
pub const STOP: u8 = 0xFC;
/// Song Position Pointer: 14-bit count of sixteenth notes, LSB first.
pub const SONG_POSITION: u8 = 0xF2;

/// How many data bytes a status byte expects.
fn data_len(status: u8) -> usize {
    match status & 0xF0 {
        0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => 2,
        0xC0 | 0xD0 => 1,
        0xF0 => match status {
            0xF1 | 0xF3 => 1,
            SONG_POSITION => 2,
            _ => 0,
        },
        _ => 0,
    }
}

/// Parses a MIDI byte stream one byte at a time.
#[derive(Debug, Default)]
pub struct StreamParser {
    /// The status whose data bytes are being collected.
    current: Option<u8>,
    /// The last channel status, reused when a data byte arrives with no
    /// status of its own. System Common clears it; System Real-Time does not.
    running: Option<u8>,
    data: [u8; 2],
    have: usize,
    in_sysex: bool,
}

impl StreamParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one byte. Returns `Some((status, data1, data2))` when a message
    /// completes; `data2` is 0 for messages carrying one data byte.
    pub fn push(&mut self, byte: u8) -> Option<(u8, u8, u8)> {
        // System Real-Time: one byte, may appear anywhere, changes nothing.
        if byte >= 0xF8 {
            return Some((byte, 0, 0));
        }

        if byte >= 0x80 {
            self.have = 0;
            match byte {
                0xF0 => {
                    self.in_sysex = true;
                    self.current = None;
                    self.running = None;
                    return None;
                }
                0xF7 => {
                    self.in_sysex = false;
                    self.current = None;
                    return None;
                }
                _ => {}
            }
            self.in_sysex = false;
            // System Common clears running status; a channel status sets it.
            self.running = (byte < 0xF0).then_some(byte);
            if data_len(byte) == 0 {
                self.current = None;
                return Some((byte, 0, 0));
            }
            self.current = Some(byte);
            return None;
        }

        if self.in_sysex {
            return None;
        }

        let status = self.current.or(self.running)?;
        self.current = Some(status);
        self.data[self.have] = byte;
        self.have += 1;
        if self.have < data_len(status) {
            return None;
        }

        let message = (
            status,
            self.data[0],
            if data_len(status) == 2 {
                self.data[1]
            } else {
                0
            },
        );
        self.have = 0;
        // A channel status stays current (running status); System Common
        // does not repeat.
        if status >= 0xF0 {
            self.current = None;
        }
        Some(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds a byte slice and collects every completed message.
    fn parse(bytes: &[u8]) -> Vec<(u8, u8, u8)> {
        let mut parser = StreamParser::new();
        bytes.iter().filter_map(|&b| parser.push(b)).collect()
    }

    #[test]
    fn a_lone_clock_byte_is_a_message() {
        // THE M0 BUG. A packet holding one 0xF8 never entered the old
        // three-byte stride, so MIDI clock reached the app never.
        assert_eq!(parse(&[CLOCK]), vec![(CLOCK, 0, 0)]);
    }

    #[test]
    fn real_time_bytes_interrupt_a_message_without_corrupting_it() {
        // System Real-Time may appear between any two bytes of another
        // message and must not disturb it — this is why a stride parser
        // cannot be patched to handle clock.
        assert_eq!(
            parse(&[0x90, 60, CLOCK, 100]),
            vec![(CLOCK, 0, 0), (0x90, 60, 100)]
        );
    }

    #[test]
    fn running_status_repeats_the_previous_status() {
        assert_eq!(
            parse(&[0x90, 60, 100, 62, 90]),
            vec![(0x90, 60, 100), (0x90, 62, 90)]
        );
    }

    #[test]
    fn two_byte_messages_complete_after_one_data_byte() {
        // Program Change and Channel Pressure carry one data byte. The old
        // stride would have eaten the following status byte as data.
        assert_eq!(
            parse(&[0xC0, 5, 0xD0, 64]),
            vec![(0xC0, 5, 0), (0xD0, 64, 0)]
        );
    }

    #[test]
    fn a_song_position_pointer_carries_both_data_bytes() {
        // 14-bit, LSB first: 8 sixteenths = two beats.
        assert_eq!(parse(&[SONG_POSITION, 8, 0]), vec![(SONG_POSITION, 8, 0)]);
    }

    #[test]
    fn system_common_clears_running_status() {
        // After a Song Select, a bare data byte is not a note-on.
        assert_eq!(
            parse(&[0x90, 60, 100, 0xF3, 2, 62, 90]),
            vec![(0x90, 60, 100), (0xF3, 2, 0),]
        );
    }

    #[test]
    fn sysex_is_skipped_and_does_not_swallow_what_follows() {
        assert_eq!(
            parse(&[0xF0, 1, 2, 3, 0xF7, 0x90, 60, 100]),
            vec![(0x90, 60, 100)]
        );
    }

    #[test]
    fn real_time_bytes_pass_through_a_sysex_block() {
        // A clock inside a SysEx dump still has to reach the transport.
        assert_eq!(parse(&[0xF0, 1, CLOCK, 2, 0xF7]), vec![(CLOCK, 0, 0)]);
    }

    #[test]
    fn transport_commands_are_one_byte_messages() {
        assert_eq!(
            parse(&[START, STOP, CONTINUE]),
            vec![(START, 0, 0), (STOP, 0, 0), (CONTINUE, 0, 0)]
        );
    }

    #[test]
    fn a_stray_data_byte_before_any_status_is_dropped() {
        assert_eq!(parse(&[60, 100]), Vec::new());
    }
}
