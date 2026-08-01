# Virtual MIDI destination (Ableton)

**Date:** 2026-08-01  
**Status:** Accepted

## Goal

While sway runs, Ableton Live (or any CoreMIDI client) can send MIDI to a
virtual destination named **Sway**. Hardware and IAC sources continue to work
via the existing `--midi <filter>` source connections. Both paths share one
`read_proc` → `Sender<MidiEvent>` → `MidiInbox` pipeline; the graph is unchanged.

## Design

- Use CoreMIDI `MIDIDestinationCreate` (a virtual **destination** — apps send
  *to* it). Colloquially this is the “virtual MIDI port.”
- Always create destination `"Sway"` inside `open_input`.
- Same MIDI client and boxed sender for the destination and the input port.
- `--midi` still connects matching external sources; empty filter connects all.
- No persistent `kMIDIPropertyUniqueID`, no midir, no parser / hotplug work.

## Lifecycle

1. `MIDIClientCreate`
2. `MIDIInputPortCreate` (for source listening)
3. `MIDIDestinationCreate` (`"Sway"`, same `read_proc` / refcon)
4. Set `kMIDIPropertyUniqueID` (`'SWAY'`) and non-zero
   `kMIDIPropertyAdvanceScheduleTimeMuSec` so DAWs deliver ASAP (0 causes
   CoreMIDI to hold scheduled packets; Ableton often looks like a black hole)
5. Connect filtered sources via `MIDIPortConnectSource`
6. On drop: `MIDIEndpointDispose(dest)` → `MIDIPortDispose` → `MIDIClientDispose`,
   then free the boxed sender

## Ableton setup

1. Run sway (destination appears only while the process is alive)
2. Preferences → Link/Tempo/MIDI → MIDI Ports → Output **Sway** → enable **Track**
3. MIDI track: MIDI To → **Sway**, channel 1 (MIDI channel 0)
4. Arm/play notes — sway logs `midi in: ...` on receipt

## Out of scope

- System Real-Time / running-status parsing (M3)
- Hotplug (M6)
- Windows / Linux
