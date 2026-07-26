//! Direct CoreMIDI input for sway. No wrapper crates: the FFI is in `ffi`.

pub mod ffi;

pub mod input;

pub use input::{open_input, MidiEvent, MidiInput};

/// Lists every CoreMIDI source by index and display name, for preflight output.
pub fn list_sources() -> Vec<(usize, String)> {
    // SAFETY: plain enumeration; no pointer is retained across the call.
    let n = unsafe { ffi::MIDIGetNumberOfSources() };
    (0..n)
        .map(|i| {
            // SAFETY: `i` is below the count returned above.
            let ep = unsafe { ffi::MIDIGetSource(i) };
            (
                i,
                ffi::object_display_name(ep).unwrap_or_else(|| format!("<source {i}>")),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerating_sources_does_not_crash() {
        // A machine with no MIDI hardware returns an empty list; the point is
        // that the FFI round-trip and CFString decoding are sound.
        let sources = list_sources();
        for (i, name) in &sources {
            assert!(!name.is_empty(), "source {i} has an empty name");
        }
    }

    use crate::ffi::{MIDIPacket, MIDIPacketList};
    use std::ffi::c_void;

    /// Builds a two-packet MIDIPacketList by hand and drives the read callback
    /// over it. This exercises the alignment-sensitive packet walk, which is
    /// the only part of the FFI that silently produces garbage when wrong.
    #[test]
    fn read_proc_parses_multiple_packets() {
        let (tx, rx) = crossbeam_channel::unbounded::<MidiEvent>();
        let tx = Box::new(tx);

        let mut buf = vec![0u8; 4096];
        // SAFETY: `buf` is far larger than two packets and is 8-byte aligned
        // enough for the fields we write; we only read back what we wrote.
        unsafe {
            let list = buf.as_mut_ptr() as *mut MIDIPacketList;
            (*list).num_packets = 2;

            let p1 = (&raw mut (*list).packet) as *mut MIDIPacket;
            (*p1).time_stamp = 111;
            (*p1).length = 3;
            (*p1).data[0] = 0x90;
            (*p1).data[1] = 60;
            (*p1).data[2] = 100;

            let p2 = crate::input::next_packet(p1) as *mut MIDIPacket;
            (*p2).time_stamp = 222;
            (*p2).length = 3;
            (*p2).data[0] = 0x90;
            (*p2).data[1] = 64;
            (*p2).data[2] = 80;

            crate::input::read_proc(
                list,
                (&*tx) as *const crossbeam_channel::Sender<MidiEvent> as *mut c_void,
                std::ptr::null_mut(),
            );
        }

        let a = rx.try_recv().expect("first packet");
        let b = rx.try_recv().expect("second packet");
        assert_eq!(
            (a.status, a.data1, a.data2, a.host_time),
            (0x90, 60, 100, 111)
        );
        assert_eq!((b.status, b.data1, b.data2, b.host_time), (0x90, 64, 80, 222));
        assert!(rx.try_recv().is_err(), "exactly two events expected");
    }

    /// Opens an input with a filter that matches no source (or every source,
    /// on a machine with none present) and drops it immediately. This
    /// exercises the `Drop for MidiInput` path — port and client disposal —
    /// without needing real MIDI hardware, and without ever reaching
    /// `read_proc` since nothing is connected.
    #[test]
    fn dropping_an_unmatched_input_does_not_crash() {
        let (tx, _rx) = crossbeam_channel::unbounded::<MidiEvent>();
        let input = crate::input::open_input("no-such-source-xyz", tx).expect("open_input");
        drop(input);
    }
}
