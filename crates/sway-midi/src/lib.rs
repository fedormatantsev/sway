//! Direct CoreMIDI input for sway. No wrapper crates: the FFI is in `ffi`.

pub mod ffi;

pub mod input;

pub use input::{MidiEvent, MidiInput, VIRTUAL_DESTINATION_NAME, open_input};

/// Returns the current mach absolute host time in CoreMIDI's timestamp units.
pub fn host_time_now() -> u64 {
    // SAFETY: `mach_absolute_time` takes no pointers and has no preconditions.
    unsafe { ffi::mach_absolute_time() }
}

/// Converts CoreMIDI's mach absolute host time to seconds.
pub fn host_time_to_secs(host_time: u64) -> f64 {
    let mut info = ffi::MachTimebaseInfo { numer: 0, denom: 0 };
    // SAFETY: `info` is a valid writable out-parameter.
    let status = unsafe { ffi::mach_timebase_info(&mut info) };
    assert_eq!(status, 0, "mach_timebase_info failed");
    host_time as f64 * f64::from(info.numer) / f64::from(info.denom) / 1_000_000_000.0
}

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

/// Lists every CoreMIDI destination by index and display name, for preflight
/// output (includes other apps' destinations and, while sway holds an open
/// input, the virtual [`VIRTUAL_DESTINATION_NAME`] endpoint).
pub fn list_destinations() -> Vec<(usize, String)> {
    // SAFETY: plain enumeration; no pointer is retained across the call.
    let n = unsafe { ffi::MIDIGetNumberOfDestinations() };
    (0..n)
        .map(|i| {
            // SAFETY: `i` is below the count returned above.
            let ep = unsafe { ffi::MIDIGetDestination(i) };
            (
                i,
                ffi::object_display_name(ep).unwrap_or_else(|| format!("<destination {i}>")),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_time_conversion_is_monotonic_and_matches_timebase() {
        let mut info = ffi::MachTimebaseInfo { numer: 0, denom: 0 };
        // SAFETY: `info` is a valid writable out-parameter.
        assert_eq!(unsafe { ffi::mach_timebase_info(&mut info) }, 0);

        // One denominator's worth of host ticks converts to `numer`
        // nanoseconds by the documented mach timebase ratio.
        let ticks = u64::from(info.denom);
        let expected_secs = f64::from(info.numer) / 1_000_000_000.0;
        let converted = host_time_to_secs(ticks);
        assert!((converted - expected_secs).abs() < f64::EPSILON);
        assert!(host_time_to_secs(ticks + 1) > converted);
    }

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

        // Backed by a `Vec<u32>` rather than `Vec<u8>` so the allocation is
        // 4-byte aligned, matching CoreMIDI's real packet-list buffers (which
        // are packed to 4 per MIDIServices.h). A `Vec<u8>` is only guaranteed
        // 1-byte aligned and would not reproduce the alignment this layout
        // depends on.
        let mut buf_u32 = vec![0u32; 1024];
        let buf = unsafe {
            std::slice::from_raw_parts_mut(buf_u32.as_mut_ptr() as *mut u8, buf_u32.len() * 4)
        };
        // SAFETY: `buf` is far larger than two packets and is 4-byte aligned,
        // matching `MIDIPacketList`'s required alignment; we only read back
        // what we wrote.
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
        assert_eq!(
            (b.status, b.data1, b.data2, b.host_time),
            (0x90, 64, 80, 222)
        );
        assert!(rx.try_recv().is_err(), "exactly two events expected");
    }

    /// Opens an input with a filter that matches no source (or every source,
    /// on a machine with none present) and drops it immediately. This
    /// exercises the `Drop for MidiInput` path — destination, port, and
    /// client disposal — without needing real MIDI hardware, and without
    /// ever reaching `read_proc` since nothing is connected.
    #[test]
    fn dropping_an_unmatched_input_does_not_crash() {
        let (tx, _rx) = crossbeam_channel::unbounded::<MidiEvent>();
        let input = crate::input::open_input("no-such-source-xyz", tx).expect("open_input");
        drop(input);
    }

    /// While an input is held open, the virtual destination must appear in
    /// CoreMIDI's destination list under [`VIRTUAL_DESTINATION_NAME`].
    #[test]
    fn open_input_publishes_virtual_destination() {
        let (tx, _rx) = crossbeam_channel::unbounded::<MidiEvent>();
        let input = crate::input::open_input("no-such-source-xyz", tx).expect("open_input");
        let destinations = list_destinations();
        assert!(
            destinations
                .iter()
                .any(|(_, name)| name.contains(VIRTUAL_DESTINATION_NAME)),
            "expected a destination containing {VIRTUAL_DESTINATION_NAME:?}, got {destinations:?}"
        );
        drop(input);
    }

    /// Ableton (and other DAWs) send to virtual destinations via MIDISend.
    /// If our destination callback never fires for those packets, Live will
    /// show "Sway" in MIDI To but the cube will never move.
    #[test]
    fn virtual_destination_receives_midisend_note_on() {
        use crate::ffi::{
            MIDIClientCreate, MIDIClientDispose, MIDIOutputPortCreate, MIDIPacket,
            MIDIPacketList, MIDIPortDispose, MIDISend, CfString,
        };
        use std::time::Duration;

        let (tx, rx) = crossbeam_channel::unbounded::<MidiEvent>();
        let input = crate::input::open_input("no-such-source-xyz", tx).expect("open_input");

        let dest = list_destinations()
            .into_iter()
            .find(|(_, name)| name.contains(VIRTUAL_DESTINATION_NAME))
            .map(|(i, _)| unsafe { crate::ffi::MIDIGetDestination(i) })
            .expect("Sway destination must be published");

        let client_name = CfString::new("sway-midi-test-out");
        let port_name = CfString::new("sway-midi-test-out-port");
        let mut client = 0;
        let mut port = 0;
        unsafe {
            assert_eq!(
                MIDIClientCreate(client_name.0, None, std::ptr::null_mut(), &mut client),
                0
            );
            assert_eq!(MIDIOutputPortCreate(client, port_name.0, &mut port), 0);

            let mut list = MIDIPacketList {
                num_packets: 1,
                packet: [MIDIPacket {
                    time_stamp: host_time_now(),
                    length: 3,
                    data: [0; 256],
                }],
            };
            list.packet[0].data[0] = 0x90;
            list.packet[0].data[1] = 60;
            list.packet[0].data[2] = 100;

            assert_eq!(MIDISend(port, dest, &list), 0);
        }

        let event = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("virtual destination must deliver MIDISend note-on to read_proc");
        assert_eq!((event.status, event.data1, event.data2), (0x90, 60, 100));

        unsafe {
            MIDIPortDispose(port);
            MIDIClientDispose(client);
        }
        drop(input);
    }

    #[test]
    fn enumerating_destinations_does_not_crash() {
        let destinations = list_destinations();
        for (i, name) in &destinations {
            assert!(!name.is_empty(), "destination {i} has an empty name");
        }
    }

    /// Pins `MIDIPacket`/`MIDIPacketList` to the exact layout CoreMIDI uses,
    /// independent of anything this crate does with them.
    ///
    /// The numbers below are measured from the macOS SDK: `MIDIServices.h`
    /// wraps both structs in `#pragma pack(push, 4)` (line 446, popped at
    /// 613). A C program compiled against the SDK reports:
    ///   MIDIPacket:     size=268 align=4, .timeStamp@0 .length@8 .data@10
    ///   MIDIPacketList: size=272 align=4, .numPackets@0 .packet@4
    /// `read_proc_parses_multiple_packets` only checks that `next_packet`'s
    /// stride agrees with the *Rust* struct definition, so it cannot catch a
    /// mismatch between the Rust definition and the real CoreMIDI ABI; this
    /// test is what actually pins the layout.
    #[test]
    fn midi_packet_layout_matches_core_midi_abi() {
        use crate::ffi::{MIDIPacket, MIDIPacketList};

        assert_eq!(std::mem::size_of::<MIDIPacket>(), 268);
        assert_eq!(std::mem::align_of::<MIDIPacket>(), 4);
        assert_eq!(std::mem::offset_of!(MIDIPacket, time_stamp), 0);
        assert_eq!(std::mem::offset_of!(MIDIPacket, length), 8);
        assert_eq!(std::mem::offset_of!(MIDIPacket, data), 10);

        assert_eq!(std::mem::size_of::<MIDIPacketList>(), 272);
        assert_eq!(std::mem::align_of::<MIDIPacketList>(), 4);
        assert_eq!(std::mem::offset_of!(MIDIPacketList, num_packets), 0);
        assert_eq!(std::mem::offset_of!(MIDIPacketList, packet), 4);
    }
}
