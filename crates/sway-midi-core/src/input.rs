//! Packet parsing and port connection. CoreMIDI invokes `read_proc` on its own
//! high-priority thread, so the only thing done there is a channel send.

use crate::ffi::*;
use crate::message::{MidiMessage, TimedMidi};
use crate::parser::StreamParser;
use crossbeam_channel::Sender;
use std::ffi::c_void;

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
    // MIDIInputPortCreate / MIDIDestinationCreate. `Drop for MidiInput`
    // disposes the CoreMIDI destination and port (and client) before the
    // `Box<Sender>` it points at is freed, so as long as this callback can
    // only run while those are alive, the pointer is valid here. `list` is
    // owned by CoreMIDI for the duration of the call.
    unsafe {
        let tx = &*(refcon as *const Sender<TimedMidi>);
        let n = (*list).num_packets;
        let mut pkt = (&raw const (*list).packet) as *const MIDIPacket;
        for _ in 0..n {
            let len = ((*pkt).length as usize).min(256);
            let data = std::slice::from_raw_parts((&raw const (*pkt).data) as *const u8, len);
            let host_time = (*pkt).time_stamp;
            // One parser per packet: CoreMIDI packets hold complete messages,
            // and connection-scoped state would let the hardware port and the
            // virtual destination corrupt each other's running status.
            let mut parser = StreamParser::new();
            for &byte in data {
                if let Some((status, data1, data2)) = parser.push(byte) {
                    // NOTE: `send` on an unbounded channel can allocate (to
                    // grow the internal buffer) and this runs on CoreMIDI's
                    // high-priority real-time thread (see module doc).
                    // Acceptable through M3; revisit if this ever glitches.
                    let _ = tx.send(TimedMidi {
                        host_time,
                        message: MidiMessage::from_bytes(status, data1, data2),
                    });
                }
            }
            pkt = next_packet(pkt);
        }
    }
}

/// Display name of the virtual CoreMIDI destination published for other apps
/// (e.g. Ableton Live MIDI To).
pub const VIRTUAL_DESTINATION_NAME: &str = "Sway";

/// Stable CoreMIDI unique ID for the virtual destination (`'SWAY'`).
/// Persisting this across launches lets Ableton keep a saved MIDI To routing.
const VIRTUAL_DESTINATION_UNIQUE_ID: i32 = 0x5357_4159;

/// Non-zero advance schedule (µs). With 0, CoreMIDI holds DAW packets until
/// their timestamp; Ableton then often appears to send into a black hole.
/// Non-zero means we receive ASAP and schedule via `MidiInbox` timestamps.
const VIRTUAL_DESTINATION_ADVANCE_SCHEDULE_US: i32 = 100_000;

/// Keeps the CoreMIDI client, input port, virtual destination, and the boxed
/// sender the callback points at alive. Dropping it disposes CoreMIDI objects
/// first (so `read_proc` can no longer be invoked), then frees the boxed sender.
pub struct MidiInput {
    _client: MIDIClientRef,
    _port: MIDIPortRef,
    _dest: MIDIEndpointRef,
    _tx: Box<Sender<TimedMidi>>,
}

impl Drop for MidiInput {
    fn drop(&mut self) {
        // SAFETY: `_dest`, `_port`, and `_client` were created together in
        // `open_input` and are only ever disposed here. Rust drops struct
        // fields in declaration order after `Drop::drop` returns, so disposing
        // the destination and port (which stops CoreMIDI from invoking
        // `read_proc` with this refcon) and the client here, before `_tx` is
        // freed below, is what guarantees `read_proc` never dereferences a
        // dangling `Box<Sender>`.
        unsafe {
            MIDIEndpointDispose(self._dest);
            MIDIPortDispose(self._port);
            MIDIClientDispose(self._client);
        }
    }
}

/// Opens MIDI input: publishes a virtual destination named
/// [`VIRTUAL_DESTINATION_NAME`], connects every source whose display name
/// contains `filter`, and streams events into `tx`. An empty filter connects
/// every source.
pub fn open_input(filter: &str, tx: Sender<TimedMidi>) -> Result<MidiInput, OSStatus> {
    let client_name = CfString::new("sway");
    let port_name = CfString::new("sway-in");
    let dest_name = CfString::new(VIRTUAL_DESTINATION_NAME);

    let mut client: MIDIClientRef = 0;
    // SAFETY: `client_name` outlives the call; `client` is a valid out slot.
    let st = unsafe { MIDIClientCreate(client_name.0, None, std::ptr::null_mut(), &mut client) };
    if st != 0 {
        return Err(st);
    }

    let tx = Box::new(tx);
    let refcon = (&*tx) as *const Sender<TimedMidi> as *mut c_void;
    let mut port: MIDIPortRef = 0;
    // SAFETY: `refcon` points into the Box returned inside `MidiInput`, which
    // the caller must keep alive for the lifetime of the port and destination.
    let st = unsafe { MIDIInputPortCreate(client, port_name.0, read_proc, refcon, &mut port) };
    if st != 0 {
        // SAFETY: `client` was just created above and is otherwise unused;
        // dispose it so it is not leaked on this error path.
        unsafe { MIDIClientDispose(client) };
        return Err(st);
    }

    let mut dest: MIDIEndpointRef = 0;
    // SAFETY: same `refcon` lifetime as the input port; `dest` is a valid out
    // slot. On failure we dispose the port and client created above.
    let st = unsafe { MIDIDestinationCreate(client, dest_name.0, read_proc, refcon, &mut dest) };
    if st != 0 {
        unsafe {
            MIDIPortDispose(port);
            MIDIClientDispose(client);
        }
        return Err(st);
    }

    // Best-effort destination properties for DAW clients. Failures here are
    // non-fatal: the destination still exists and can receive MIDISend.
    unsafe {
        let _ = MIDIObjectSetIntegerProperty(
            dest,
            kMIDIPropertyUniqueID,
            VIRTUAL_DESTINATION_UNIQUE_ID,
        );
        let _ = MIDIObjectSetIntegerProperty(
            dest,
            kMIDIPropertyAdvanceScheduleTimeMuSec,
            VIRTUAL_DESTINATION_ADVANCE_SCHEDULE_US,
        );
    }

    // SAFETY: enumeration plus connect against a port we just created.
    unsafe {
        let n = MIDIGetNumberOfSources();
        for i in 0..n {
            let ep = MIDIGetSource(i);
            // NOTE: a source whose name could not be read is treated as "does
            // not match the filter" (`unwrap_or(false)`). `object_display_name`
            // returns `None` not just when CoreMIDI has no name to give, but
            // also when `CFStringGetCString` overflows its 256-byte buffer.
            // So a device with a sufficiently long display name silently
            // connects nothing when a non-empty `--midi` filter is given,
            // with no error surfaced.
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
        _dest: dest,
        _tx: tx,
    })
}
