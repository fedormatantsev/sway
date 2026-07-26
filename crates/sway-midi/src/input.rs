//! Packet parsing and port connection. CoreMIDI invokes `read_proc` on its own
//! high-priority thread, so the only thing done there is a channel send.

use crate::ffi::*;
use crossbeam_channel::Sender;
use std::ffi::c_void;

/// A single three-byte MIDI message with the host time CoreMIDI reported.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MidiEvent {
    pub status: u8,
    pub data1: u8,
    pub data2: u8,
    /// CoreMIDI host time, in mach absolute time units. Converting to a wall
    /// clock needs `mach_timebase_info`, which M0 does not need — sub-tick
    /// offsets arrive with the transport at M3.
    pub host_time: u64,
}

/// Walks to the next packet in a `MIDIPacketList`.
///
/// On aarch64 packets are padded to a 4-byte boundary; on x86_64 they are
/// packed. Getting this wrong yields garbage after the first packet, so it is
/// `cfg`-gated rather than assumed.
///
/// SAFETY: `pkt` must point at a valid packet that has at least one more
/// packet following it in the same list.
pub(crate) unsafe fn next_packet(pkt: *const MIDIPacket) -> *const MIDIPacket {
    unsafe {
        let len = (*pkt).length as usize;
        let end = (&raw const (*pkt).data) as *const u8;
        let end = end.add(len) as usize;
        #[cfg(target_arch = "aarch64")]
        let end = (end + 3) & !3;
        end as *const MIDIPacket
    }
}

/// The CoreMIDI read callback. Runs on CoreMIDI's thread.
pub(crate) extern "C" fn read_proc(
    list: *const MIDIPacketList,
    refcon: *mut c_void,
    _src: *mut c_void,
) {
    // SAFETY: `refcon` is the boxed Sender pointer handed to
    // MIDIInputPortCreate, kept alive by the `MidiInput` guard for as long as
    // the port exists. `list` is owned by CoreMIDI for the duration of the call.
    unsafe {
        let tx = &*(refcon as *const Sender<MidiEvent>);
        let n = (*list).num_packets;
        let mut pkt = (&raw const (*list).packet) as *const MIDIPacket;
        for _ in 0..n {
            let len = ((*pkt).length as usize).min(256);
            let data = std::slice::from_raw_parts((&raw const (*pkt).data) as *const u8, len);
            let host_time = (*pkt).time_stamp;
            let mut i = 0;
            while i + 2 < len {
                let status = data[i];
                if status & 0x80 != 0 {
                    let _ = tx.send(MidiEvent {
                        status,
                        data1: data[i + 1],
                        data2: data[i + 2],
                        host_time,
                    });
                }
                i += 3;
            }
            pkt = next_packet(pkt);
        }
    }
}

/// Keeps the CoreMIDI client, port, and the boxed sender the callback points
/// at alive. Dropping it releases the sender, after which the callback must not
/// run again — so it must outlive the app.
pub struct MidiInput {
    _client: MIDIClientRef,
    _port: MIDIPortRef,
    _tx: Box<Sender<MidiEvent>>,
}

/// Opens every source whose display name contains `filter` and streams events
/// into `tx`. An empty filter connects every source.
pub fn open_input(filter: &str, tx: Sender<MidiEvent>) -> Result<MidiInput, OSStatus> {
    let client_name = CfString::new("sway");
    let port_name = CfString::new("sway-in");

    let mut client: MIDIClientRef = 0;
    // SAFETY: `client_name` outlives the call; `client` is a valid out slot.
    let st = unsafe { MIDIClientCreate(client_name.0, None, std::ptr::null_mut(), &mut client) };
    if st != 0 {
        return Err(st);
    }

    let tx = Box::new(tx);
    let refcon = (&*tx) as *const Sender<MidiEvent> as *mut c_void;
    let mut port: MIDIPortRef = 0;
    // SAFETY: `refcon` points into the Box returned inside `MidiInput`, which
    // the caller must keep alive for the lifetime of the port.
    let st = unsafe { MIDIInputPortCreate(client, port_name.0, read_proc, refcon, &mut port) };
    if st != 0 {
        return Err(st);
    }

    // SAFETY: enumeration plus connect against a port we just created.
    unsafe {
        let n = MIDIGetNumberOfSources();
        for i in 0..n {
            let ep = MIDIGetSource(i);
            let matches = filter.is_empty()
                || object_display_name(ep)
                    .map(|s| s.contains(filter))
                    .unwrap_or(false);
            if matches {
                MIDIPortConnectSource(port, ep, std::ptr::null_mut());
            }
        }
    }

    Ok(MidiInput {
        _client: client,
        _port: port,
        _tx: tx,
    })
}
