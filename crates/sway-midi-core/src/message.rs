#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiMessage {
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8, velocity: u8 },
    Control { channel: u8, cc: u8, value: u8 },
    Clock,
    Start,
    Continue,
    Stop,
    SongPosition { sixteenths: u16 },
    Other { status: u8, data1: u8, data2: u8 },
}

impl MidiMessage {
    pub fn from_bytes(status: u8, data1: u8, data2: u8) -> Self {
        match status {
            0xF8 => Self::Clock,
            0xFA => Self::Start,
            0xFB => Self::Continue,
            0xFC => Self::Stop,
            0xF2 => {
                let sixteenths = u16::from(data2) << 7 | u16::from(data1);
                Self::SongPosition { sixteenths }
            }
            s if s & 0xF0 == 0x90 => {
                let channel = s & 0x0F;
                if data2 == 0 {
                    Self::NoteOff {
                        channel,
                        note: data1,
                        velocity: 0,
                    }
                } else {
                    Self::NoteOn {
                        channel,
                        note: data1,
                        velocity: data2,
                    }
                }
            }
            s if s & 0xF0 == 0x80 => Self::NoteOff {
                channel: s & 0x0F,
                note: data1,
                velocity: data2,
            },
            s if s & 0xF0 == 0xB0 => Self::Control {
                channel: s & 0x0F,
                cc: data1,
                value: data2,
            },
            s => Self::Other {
                status: s,
                data1,
                data2,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedMidi {
    pub host_time: u64,
    pub message: MidiMessage,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_velocity_note_on_is_note_off() {
        assert_eq!(
            MidiMessage::from_bytes(0x90, 60, 0),
            MidiMessage::NoteOff {
                channel: 0,
                note: 60,
                velocity: 0
            }
        );
    }

    #[test]
    fn note_on_carries_channel() {
        assert_eq!(
            MidiMessage::from_bytes(0x91, 64, 100),
            MidiMessage::NoteOn {
                channel: 1,
                note: 64,
                velocity: 100
            }
        );
    }

    #[test]
    fn clock_start_stop_continue_and_spp() {
        assert_eq!(MidiMessage::from_bytes(0xF8, 0, 0), MidiMessage::Clock);
        assert_eq!(MidiMessage::from_bytes(0xFA, 0, 0), MidiMessage::Start);
        assert_eq!(MidiMessage::from_bytes(0xFB, 0, 0), MidiMessage::Continue);
        assert_eq!(MidiMessage::from_bytes(0xFC, 0, 0), MidiMessage::Stop);
        assert_eq!(
            MidiMessage::from_bytes(0xF2, 8, 0),
            MidiMessage::SongPosition { sixteenths: 8 }
        );
    }
}
