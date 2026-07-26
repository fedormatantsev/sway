# M0 Walking Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A MIDI note from the Octatrack changes a cube's colour on a fullscreen HDMI display, driven by a hardcoded Rust graph ticking in `FixedUpdate`.

**Architecture:** Two crates. `sway-midi` owns direct CoreMIDI FFI and pushes timestamped events down a `crossbeam-channel` from CoreMIDI's own callback thread. `sway-app` is a Bevy binary whose graph tick is a single exclusive system in `FixedUpdate` draining that channel into a `GraphState` resource, with an `Update` system applying that state to the cube's material. No file format, no editor, no node abstraction — those arrive at M2 and M4.

**Tech Stack:** Rust 2024 edition, Bevy 0.19, crossbeam-channel, CoreMIDI and CoreFoundation via hand-written FFI (no `midir`, no `coremidi-rs`).

## Global Constraints

- **Bevy pinned to `0.19`** — exact, workspace-wide. §2.8 of the spec requires one Bevy version across the tree; M1b adds Vello and the pin becomes load-bearing.
- **No MIDI wrapper crates.** CoreMIDI is bound directly via `extern "C"`. `midir` and `coremidi-rs` are out.
- **`sway-midi` must not depend on Bevy.** It is plain Rust plus FFI, so its tests run without an ECS.
- **The graph tick is one exclusive system in `FixedUpdate`** (spec §2.6). Not a per-type system, not `Update`.
- **Every `unsafe` block carries a `// SAFETY:` comment** stating the invariant being relied on.
- **Platform: macOS only** for M0. CoreMIDI is a macOS framework; the packet-walk alignment is `cfg`-gated on `target_arch`.
- **Tests must not require MIDI hardware.** Hardware appears only in the manual verification steps.
- **Bevy 0.19 renamed buffered events to messages.** `AssetEvent` derives `Message`, and the collection resource is `Messages<M>`, not `Events<M>`. Older examples and LLM recall will say `Events`; that will not compile.

**Verification note:** every code block in this plan was compiled and every test run against Bevy 0.19 before the plan was written. Where the obvious approach did not work — `mpsc::Receiver` not being `Sync`, frame 0 running no fixed tick, `Events` vs `Messages` — the working version is what appears here.

## File Structure

```
Cargo.toml                  workspace manifest, pinned versions
crates/
  sway-midi/
    Cargo.toml
    src/lib.rs              public API: MidiEvent, list_sources, open_input
    src/ffi.rs              raw CoreMIDI + CoreFoundation declarations, CfString
    src/input.rs            read callback, packet walking, port connection
  sway-app/
    Cargo.toml
    src/main.rs             Bevy app wiring, CLI arg, window setup
    src/graph.rs            GraphState, graph_tick exclusive system
    src/scene.rs            cube/camera/light spawn, colour application
```

`sway-midi` splits FFI declarations from behaviour so the unsafe surface is one readable file. `sway-app` splits the graph from the scene because M2 moves `graph.rs` into `sway-graph` wholesale.

---

### Task 1: Workspace skeleton

**Files:**
- Create: `Cargo.toml` (workspace root, replacing the current single-crate manifest)
- Create: `crates/sway-midi/Cargo.toml`
- Create: `crates/sway-midi/src/lib.rs`
- Create: `crates/sway-app/Cargo.toml`
- Create: `crates/sway-app/src/main.rs`
- Delete: `src/main.rs`

**Interfaces:**
- Consumes: nothing
- Produces: workspace with members `sway-midi` and `sway-app`; `sway_midi` is a library crate, `sway-app` a binary named `sway`.

- [ ] **Step 1: Replace the root manifest with a workspace**

```toml
[workspace]
resolver = "3"
members = ["crates/sway-midi", "crates/sway-app"]

[workspace.package]
edition = "2024"
version = "0.1.0"

[workspace.dependencies]
bevy = "=0.19.0"
crossbeam-channel = "0.5"
sway-midi = { path = "crates/sway-midi" }
```

- [ ] **Step 2: Create the sway-midi manifest**

```toml
[package]
name = "sway-midi"
edition.workspace = true
version.workspace = true

[dependencies]
crossbeam-channel.workspace = true
```

- [ ] **Step 3: Create a placeholder sway-midi lib**

`crates/sway-midi/src/lib.rs`:

```rust
//! Direct CoreMIDI input for sway. No wrapper crates: the FFI is in `ffi`.
```

- [ ] **Step 4: Create the sway-app manifest**

```toml
[package]
name = "sway-app"
edition.workspace = true
version.workspace = true

[[bin]]
name = "sway"
path = "src/main.rs"

[dependencies]
bevy.workspace = true
crossbeam-channel.workspace = true
sway-midi.workspace = true
```

- [ ] **Step 5: Create a placeholder binary**

`crates/sway-app/src/main.rs`:

```rust
fn main() {
    println!("sway");
}
```

- [ ] **Step 6: Remove the old single-crate source**

```bash
rm -rf src
```

- [ ] **Step 7: Verify the workspace builds**

Run: `cargo build`
Expected: `Finished` with no errors. First build compiles Bevy and takes several minutes.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock crates .gitignore
git rm -r --cached src 2>/dev/null || true
git commit -m "chore: workspace skeleton with sway-midi and sway-app"
```

---

### Task 2: CoreMIDI FFI declarations and source enumeration

**Files:**
- Create: `crates/sway-midi/src/ffi.rs`
- Modify: `crates/sway-midi/src/lib.rs`

**Interfaces:**
- Consumes: Task 1's crate layout
- Produces:
  - `sway_midi::list_sources() -> Vec<(usize, String)>` — index and display name of every CoreMIDI source
  - `ffi` module types used by Task 3: `MIDIPacketList`, `MIDIPacket`, `MIDIClientRef`, `MIDIPortRef`, `MIDIEndpointRef`, `OSStatus`, `MIDIReadProc`, `CfString`, and the `extern` functions `MIDIClientCreate`, `MIDIInputPortCreate`, `MIDIPortConnectSource`, `MIDIGetNumberOfSources`, `MIDIGetSource`

- [ ] **Step 1: Write the failing test**

Append to `crates/sway-midi/src/lib.rs`:

```rust
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
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p sway-midi`
Expected: FAIL — `cannot find function 'list_sources' in this scope`

- [ ] **Step 3: Write the FFI declarations**

`crates/sway-midi/src/ffi.rs`:

```rust
//! Raw CoreMIDI and CoreFoundation declarations. Everything unsafe about the
//! MIDI layer is confined to this file and `input.rs`.

use std::ffi::{c_char, c_void, CString};

pub type OSStatus = i32;
pub type MIDIObjectRef = u32;
pub type MIDIClientRef = MIDIObjectRef;
pub type MIDIPortRef = MIDIObjectRef;
pub type MIDIEndpointRef = MIDIObjectRef;
pub type ItemCount = usize;
pub type CFStringRef = *const c_void;
pub type CFAllocatorRef = *const c_void;

pub type MIDINotifyProc = extern "C" fn(*const c_void, *mut c_void);
pub type MIDIReadProc = extern "C" fn(*const MIDIPacketList, *mut c_void, *mut c_void);

#[repr(C)]
pub struct MIDIPacketList {
    pub num_packets: u32,
    pub packet: [MIDIPacket; 1],
}

#[repr(C)]
pub struct MIDIPacket {
    pub time_stamp: u64,
    pub length: u16,
    pub data: [u8; 256],
}

#[link(name = "CoreMIDI", kind = "framework")]
unsafe extern "C" {
    pub fn MIDIClientCreate(
        name: CFStringRef,
        notify_proc: Option<MIDINotifyProc>,
        notify_refcon: *mut c_void,
        out_client: *mut MIDIClientRef,
    ) -> OSStatus;

    pub fn MIDIInputPortCreate(
        client: MIDIClientRef,
        port_name: CFStringRef,
        read_proc: MIDIReadProc,
        refcon: *mut c_void,
        out_port: *mut MIDIPortRef,
    ) -> OSStatus;

    pub fn MIDIPortConnectSource(
        port: MIDIPortRef,
        source: MIDIEndpointRef,
        conn_refcon: *mut c_void,
    ) -> OSStatus;

    pub fn MIDIGetNumberOfSources() -> ItemCount;
    pub fn MIDIGetSource(index: ItemCount) -> MIDIEndpointRef;
    pub fn MIDIObjectGetStringProperty(
        obj: MIDIObjectRef,
        property: CFStringRef,
        out: *mut CFStringRef,
    ) -> OSStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub fn CFStringCreateWithCString(
        alloc: CFAllocatorRef,
        cstr: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    pub fn CFStringGetCString(
        s: CFStringRef,
        buffer: *mut c_char,
        size: isize,
        encoding: u32,
    ) -> bool;
    pub fn CFRelease(cf: *const c_void);
    pub static kMIDIPropertyDisplayName: CFStringRef;
}

pub const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

/// Owns a CFString and releases it on drop.
pub struct CfString(pub CFStringRef);

impl CfString {
    pub fn new(s: &str) -> Self {
        let c = CString::new(s).expect("no interior NUL");
        // SAFETY: `c` outlives the call and CF copies the bytes; a null
        // allocator selects the default one.
        let r = unsafe {
            CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), K_CF_STRING_ENCODING_UTF8)
        };
        CfString(r)
    }
}

impl Drop for CfString {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: we own this reference, created in `new`.
            unsafe { CFRelease(self.0) };
        }
    }
}

/// Reads a CoreMIDI object's display name.
pub fn object_display_name(obj: MIDIObjectRef) -> Option<String> {
    let mut out: CFStringRef = std::ptr::null();
    // SAFETY: `kMIDIPropertyDisplayName` is a CF constant owned by the
    // framework; `out` is a valid slot we own.
    let status = unsafe { MIDIObjectGetStringProperty(obj, kMIDIPropertyDisplayName, &mut out) };
    if status != 0 || out.is_null() {
        return None;
    }
    let mut buf = [0 as c_char; 256];
    // SAFETY: `out` is a valid CFStringRef from the call above; `buf` is sized.
    let ok = unsafe {
        CFStringGetCString(
            out,
            buf.as_mut_ptr(),
            buf.len() as isize,
            K_CF_STRING_ENCODING_UTF8,
        )
    };
    // SAFETY: MIDIObjectGetStringProperty returned an owned reference.
    unsafe { CFRelease(out) };
    if !ok {
        return None;
    }
    // SAFETY: CFStringGetCString NUL-terminated `buf` on success.
    Some(
        unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned(),
    )
}
```

- [ ] **Step 4: Wire the module and implement `list_sources`**

Replace the contents of `crates/sway-midi/src/lib.rs` above the `tests` module with:

```rust
//! Direct CoreMIDI input for sway. No wrapper crates: the FFI is in `ffi`.

pub mod ffi;

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
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p sway-midi`
Expected: PASS, `1 passed`

- [ ] **Step 6: Commit**

```bash
git add crates/sway-midi
git commit -m "feat(midi): CoreMIDI FFI declarations and source enumeration"
```

---

### Task 3: Packet parsing and input connection

**Files:**
- Create: `crates/sway-midi/src/input.rs`
- Modify: `crates/sway-midi/src/lib.rs`

**Interfaces:**
- Consumes: everything Task 2 produced from `ffi`
- Produces:
  - `sway_midi::MidiEvent { status: u8, data1: u8, data2: u8, host_time: u64 }` — `Debug + Clone + Copy + PartialEq`
  - `sway_midi::MidiInput` — RAII guard; dropping it drops the boxed sender, so it must be held for as long as events are wanted
  - `sway_midi::open_input(filter: &str, tx: Sender<MidiEvent>) -> Result<MidiInput, OSStatus>` — connects every source whose display name contains `filter`; an empty filter connects all

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `crates/sway-midi/src/lib.rs`:

```rust
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
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p sway-midi`
Expected: FAIL — `could not find 'input' in the crate root`

- [ ] **Step 3: Write the input module**

`crates/sway-midi/src/input.rs`:

```rust
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
```

- [ ] **Step 4: Re-export from the crate root**

Insert into `crates/sway-midi/src/lib.rs`, directly after `pub mod ffi;`:

```rust
pub mod input;

pub use input::{open_input, MidiEvent, MidiInput};
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p sway-midi`
Expected: PASS, `2 passed`

- [ ] **Step 6: Commit**

```bash
git add crates/sway-midi
git commit -m "feat(midi): packet walking and CoreMIDI port connection"
```

---

### Task 4: The hardcoded graph

**Files:**
- Create: `crates/sway-app/src/graph.rs`
- Modify: `crates/sway-app/src/main.rs`

**Interfaces:**
- Consumes: `sway_midi::MidiEvent`
- Produces:
  - `GraphState { level: f32 }` — a Bevy `Resource`, `Default + Debug + PartialEq`
  - `MidiRx(pub Receiver<MidiEvent>)` — a Bevy `Resource`
  - `graph_tick(world: &mut World)` — the exclusive system for `FixedUpdate`
  - `DECAY_PER_SEC: f32`
  - `TICK_HZ: f64`

**Note on the test harness:** `TimeUpdateStrategy::FixedTimesteps(1)` makes each `app.update()` run `FixedUpdate` exactly once — except the first, where the accumulator is still empty. The helper therefore burns one warm-up update, and `warm_up_update_ran_no_ticks` pins that behaviour so it fails loudly if a Bevy upgrade changes it.

- [ ] **Step 1: Declare the module first**

`sway-app` is a binary-only crate, so `graph.rs` is not compiled at all until
`main.rs` declares it. Declaring the module before writing the test is what
makes the next step a genuine red: without it, `cargo test` reports `0 tests`
rather than a compile error.

Replace `crates/sway-app/src/main.rs` with:

```rust
mod graph;

fn main() {
    println!("sway");
}
```

- [ ] **Step 2: Write the failing tests**

`crates/sway-app/src/graph.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use crossbeam_channel::{Receiver, Sender};

    fn note_on(vel: u8) -> MidiEvent {
        MidiEvent {
            status: 0x90,
            data1: 60,
            data2: vel,
            host_time: 0,
        }
    }

    /// Headless app running FixedUpdate exactly once per `app.update()`.
    ///
    /// Frame 0 runs no fixed tick — the accumulator is empty until real time
    /// has advanced once — so one warm-up update is burned here.
    fn headless() -> (Sender<MidiEvent>, App) {
        let (tx, rx): (Sender<MidiEvent>, Receiver<MidiEvent>) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
            .insert_resource(MidiRx(rx))
            .init_resource::<GraphState>()
            .add_systems(FixedUpdate, graph_tick);
        app.update();
        (tx, app)
    }

    fn level(app: &App) -> f32 {
        app.world().resource::<GraphState>().level
    }

    #[test]
    fn warm_up_update_ran_no_ticks() {
        let (_tx, app) = headless();
        assert_eq!(level(&app), 0.0);
    }

    #[test]
    fn note_on_sets_level_then_one_tick_of_decay() {
        let (tx, mut app) = headless();
        tx.send(note_on(127)).unwrap();
        app.update();
        let expected = 1.0 - (1.0 / TICK_HZ as f32) * DECAY_PER_SEC;
        assert!(
            (level(&app) - expected).abs() < 1e-6,
            "expected {expected}, got {}",
            level(&app)
        );
    }

    #[test]
    fn velocity_scales_level() {
        let (tx, mut app) = headless();
        tx.send(note_on(64)).unwrap();
        app.update();
        assert!(level(&app) > 0.4 && level(&app) < 0.55, "got {}", level(&app));
    }

    #[test]
    fn level_decays_to_zero_and_clamps() {
        let (tx, mut app) = headless();
        tx.send(note_on(127)).unwrap();
        for _ in 0..500 {
            app.update();
        }
        assert_eq!(level(&app), 0.0, "decay must clamp, not go negative");
    }

    #[test]
    fn note_off_is_ignored() {
        let (tx, mut app) = headless();
        tx.send(MidiEvent {
            status: 0x80,
            data1: 60,
            data2: 100,
            host_time: 0,
        })
        .unwrap();
        app.update();
        assert_eq!(level(&app), 0.0);
    }

    #[test]
    fn zero_velocity_note_on_is_ignored() {
        // Many devices send note-on with velocity 0 instead of note-off.
        let (tx, mut app) = headless();
        tx.send(note_on(0)).unwrap();
        app.update();
        assert_eq!(level(&app), 0.0);
    }

    #[test]
    fn identical_input_gives_bit_identical_output() {
        let run = || {
            let (tx, mut app) = headless();
            let mut trace = Vec::new();
            for i in 0..40 {
                if i == 0 {
                    tx.send(note_on(100)).unwrap();
                }
                if i == 5 {
                    tx.send(note_on(64)).unwrap();
                }
                app.update();
                trace.push(level(&app).to_bits());
            }
            trace
        };
        assert_eq!(run(), run(), "same input must give bit-identical output");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p sway-app`
Expected: FAIL to compile — `cannot find type 'GraphState' in this scope` and similar. If you instead see `0 tests`, Step 1 was skipped.

- [ ] **Step 4: Write the graph**

Prepend to `crates/sway-app/src/graph.rs`, above the `tests` module:

```rust
//! The M0 graph: one hardcoded node. Replaced wholesale by `sway-graph` at M2.

use bevy::prelude::*;
use crossbeam_channel::Receiver;
use sway_midi::MidiEvent;

/// Graph tick rate. Spec §7 leaves the final number to M2 measurement; 120 Hz
/// is comfortably above frame rate and divides evenly into common tempos.
pub const TICK_HZ: f64 = 120.0;

/// How fast `level` falls back to zero, in units per second.
pub const DECAY_PER_SEC: f32 = 2.0;

/// The receiving end of the CoreMIDI channel.
#[derive(Resource)]
pub struct MidiRx(pub Receiver<MidiEvent>);

/// The entire graph state for M0.
#[derive(Resource, Default, Debug, PartialEq)]
pub struct GraphState {
    /// 0.0 to 1.0, set by note velocity and decaying toward zero.
    pub level: f32,
}

/// The graph tick: one exclusive system in `FixedUpdate` (spec §2.6).
///
/// Drains every MIDI event that arrived since the last tick, applies note-ons,
/// then decays. Decay uses the fixed timestep rather than frame delta, so the
/// result depends only on how many ticks ran.
pub fn graph_tick(world: &mut World) {
    let mut notes: Vec<MidiEvent> = Vec::new();
    if let Some(rx) = world.get_resource::<MidiRx>() {
        while let Ok(e) = rx.0.try_recv() {
            // Note-on with non-zero velocity. Many devices spell note-off as
            // note-on with velocity 0.
            if e.status & 0xF0 == 0x90 && e.data2 > 0 {
                notes.push(e);
            }
        }
    }
    let dt = world.resource::<Time<Fixed>>().delta_secs();
    let mut state = world.resource_mut::<GraphState>();
    for e in notes {
        state.level = e.data2 as f32 / 127.0;
    }
    state.level = (state.level - dt * DECAY_PER_SEC).max(0.0);
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p sway-app`
Expected: PASS, `7 passed`

- [ ] **Step 6: Commit**

```bash
git add crates/sway-app
git commit -m "feat(app): hardcoded graph tick with golden-trace determinism tests"
```

---

### Task 5: Scene and colour application

**Files:**
- Create: `crates/sway-app/src/scene.rs`
- Modify: `crates/sway-app/src/main.rs`

**Interfaces:**
- Consumes: `graph::GraphState`
- Produces:
  - `Cube` — marker component
  - `setup_scene(...)` — a `Startup` system spawning cube, camera, and light
  - `apply_level(...)` — an `Update` system writing `GraphState::level` into the cube's material

**Note on the material write:** `Assets::get_mut` marks the asset changed by the act of being called, so an unconditional write would re-upload the material every frame. Spec §2.11 requires read-compare-then-write, which is what `apply_level` does.

- [ ] **Step 1: Declare the module first**

Same reason as Task 4: `scene.rs` is not compiled until `main.rs` declares it,
so without this the next step reports `0 tests` instead of failing.

Replace `crates/sway-app/src/main.rs` with:

```rust
mod graph;
mod scene;

fn main() {
    println!("sway");
}
```

- [ ] **Step 2: Write the failing test**

`crates/sway-app/src/scene.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::GraphState;

    /// Headless app with assets but no renderer, enough to exercise the
    /// material write path.
    fn headless() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_resource::<GraphState>()
            .add_systems(Update, apply_level);
        app
    }

    fn spawn_cube(app: &mut App) -> Handle<StandardMaterial> {
        let handle = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: Color::BLACK,
                ..default()
            });
        app.world_mut()
            .spawn((MeshMaterial3d(handle.clone()), Cube));
        handle
    }

    /// Drains asset-modified notifications since the last call. Note that in
    /// Bevy 0.19 `AssetEvent` is a `Message`, so the collection is `Messages`,
    /// not `Events`.
    fn count_modified(app: &mut App) -> usize {
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<StandardMaterial>>>()
            .drain()
            .filter(|e| matches!(e, AssetEvent::Modified { .. }))
            .count()
    }

    #[test]
    fn level_drives_base_color() {
        let mut app = headless();
        let handle = spawn_cube(&mut app);

        app.world_mut().resource_mut::<GraphState>().level = 1.0;
        app.update();

        let materials = app.world().resource::<Assets<StandardMaterial>>();
        let colour = materials.get(&handle).unwrap().base_color;
        assert_eq!(colour, colour_for_level(1.0));
    }

    #[test]
    fn changed_level_modifies_the_asset() {
        let mut app = headless();
        let _handle = spawn_cube(&mut app);

        app.world_mut().resource_mut::<GraphState>().level = 0.5;
        app.update();

        assert!(
            count_modified(&mut app) > 0,
            "a real colour change must write through"
        );
    }

    #[test]
    fn unchanged_level_does_not_touch_the_asset() {
        let mut app = headless();
        let _handle = spawn_cube(&mut app);

        app.world_mut().resource_mut::<GraphState>().level = 0.5;
        app.update();
        let _ = count_modified(&mut app);

        // Same level again: apply_level must short-circuit before get_mut.
        app.update();
        assert_eq!(
            count_modified(&mut app),
            0,
            "apply_level must not rewrite an unchanged colour"
        );
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p sway-app`
Expected: FAIL to compile — `cannot find function 'apply_level' in this scope`. If you instead see only Task 4's 7 tests passing, Step 1 was skipped.

- [ ] **Step 4: Write the scene module**

Prepend to `crates/sway-app/src/scene.rs`, above the `tests` module:

```rust
//! The M0 scene: one cube, one camera, one light. Replaced by graph-authored
//! scene nodes at M5 (spec §2.10).

use crate::graph::GraphState;
use bevy::prelude::*;

/// Marks the cube whose colour the graph drives.
#[derive(Component)]
pub struct Cube;

/// The colour for a given graph level. Pulled out so tests can assert against
/// it without duplicating the formula.
pub fn colour_for_level(level: f32) -> Color {
    Color::srgb(level, 0.1, 1.0 - level)
}

pub fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(1.5, 1.5, 1.5))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: colour_for_level(0.0),
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, 0.0),
        Cube,
    ));
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 2.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 6000.0,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

/// Writes the graph's level into the cube's material.
///
/// Reads and compares before calling `get_mut`, because `get_mut` marks the
/// asset modified purely by being called — an unconditional write would
/// re-upload the material every frame (spec §2.11).
pub fn apply_level(
    state: Res<GraphState>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    q: Query<&MeshMaterial3d<StandardMaterial>, With<Cube>>,
) {
    let want = colour_for_level(state.level);
    for handle in &q {
        let Some(current) = materials.get(&handle.0) else {
            continue;
        };
        if current.base_color == want {
            continue;
        }
        if let Some(mut mat) = materials.get_mut(&handle.0) {
            mat.base_color = want;
        }
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p sway-app`
Expected: PASS, `10 passed`

- [ ] **Step 6: Commit**

```bash
git add crates/sway-app
git commit -m "feat(app): cube scene and change-gated material write"
```

---

### Task 6: Wire the binary, fullscreen output, and preflight

**Files:**
- Modify: `crates/sway-app/src/main.rs`

**Interfaces:**
- Consumes: `graph::{GraphState, MidiRx, graph_tick, TICK_HZ}`, `scene::{setup_scene, apply_level}`, `sway_midi::{open_input, list_sources}`
- Produces: the `sway` binary. CLI: `sway [--monitor N] [--midi SUBSTRING] [--windowed] [--list]`

- [ ] **Step 1: Write the whole binary**

Replace `crates/sway-app/src/main.rs` with:

```rust
mod graph;
mod scene;

use bevy::prelude::*;
use bevy::window::{Monitor, MonitorSelection, WindowMode};
use graph::{graph_tick, GraphState, MidiRx, TICK_HZ};
use scene::{apply_level, setup_scene};

struct Args {
    monitor: usize,
    midi_filter: String,
    windowed: bool,
    list_only: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        monitor: 0,
        midi_filter: String::new(),
        windowed: false,
        list_only: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--monitor" => {
                args.monitor = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .expect("--monitor needs a number");
            }
            "--midi" => {
                args.midi_filter = it.next().expect("--midi needs a substring");
            }
            "--windowed" => args.windowed = true,
            "--list" => args.list_only = true,
            other => panic!("unknown argument: {other}"),
        }
    }
    args
}

/// Logs every monitor once, so choosing `--monitor N` does not require
/// guessing.
///
/// This must run in `Update`, not `Startup`. Bevy spawns `Monitor` entities
/// from `create_monitors`, which winit calls from its event-loop resume
/// handler — after `Startup` has already run, so a `Startup` query sees an
/// empty world. The `Local` latch makes it fire once, on the first frame where
/// monitors actually exist.
fn log_monitors(monitors: Query<&Monitor>, mut logged: Local<bool>) {
    if *logged || monitors.is_empty() {
        return;
    }
    *logged = true;
    for (i, m) in monitors.iter().enumerate() {
        info!(
            "monitor {i}: {} {}x{} @ {:?} mHz",
            m.name.as_deref().unwrap_or("<unnamed>"),
            m.physical_width,
            m.physical_height,
            m.refresh_rate_millihertz,
        );
    }
}

fn main() {
    let args = parse_args();

    let sources = sway_midi::list_sources();
    if sources.is_empty() {
        eprintln!("no CoreMIDI sources found");
    } else {
        eprintln!("CoreMIDI sources:");
        for (i, name) in &sources {
            eprintln!("  {i}: {name}");
        }
    }
    if args.list_only {
        return;
    }

    let (tx, rx) = crossbeam_channel::unbounded();
    // Held for the process lifetime: dropping it closes the port and frees the
    // sender the CoreMIDI callback points at.
    let _midi = match sway_midi::open_input(&args.midi_filter, tx) {
        Ok(conn) => Some(conn),
        Err(status) => {
            eprintln!("could not open MIDI input (OSStatus {status}); continuing without MIDI");
            None
        }
    };

    let mode = if args.windowed {
        WindowMode::Windowed
    } else {
        WindowMode::BorderlessFullscreen(MonitorSelection::Index(args.monitor))
    };

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                mode,
                title: "sway".into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
        .insert_resource(MidiRx(rx))
        .init_resource::<GraphState>()
        .add_systems(Startup, setup_scene)
        .add_systems(FixedUpdate, graph_tick)
        .add_systems(Update, (apply_level, log_monitors))
        .run();
}
```

- [ ] **Step 2: Verify it builds and the test suite still passes**

Run: `cargo test --workspace && cargo build`
Expected: all tests pass, `Finished`

- [ ] **Step 3: Manually verify MIDI enumeration**

Run: `cargo run -p sway-app -- --list`
Expected: a list of CoreMIDI sources. With the Octatrack connected, its name appears. If the list is empty with hardware plugged in, check macOS **Audio MIDI Setup → MIDI Studio** before suspecting the code.

- [ ] **Step 4: Manually verify the windowed path**

Run: `cargo run -p sway-app -- --windowed`
Expected: a window with a dark blue cube. Monitor indices are printed to the log. Play notes on the Octatrack: the cube flashes toward red on each note and fades back to blue. Higher velocity gives a brighter flash.

- [ ] **Step 5: Manually verify fullscreen on the external display**

With the HDMI display connected, note its index from Step 4's log, then run:

`cargo run -p sway-app --release -- --monitor 1`

Expected: fullscreen borderless output on the external display, same note-reactive behaviour. Quit with Cmd-Q.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-app
git commit -m "feat(app): fullscreen output, monitor selection, and MIDI preflight"
```

---

## Exit criteria

Matches the spec's M0 exit — *"Octatrack plugged in, something on screen moves in time"* — plus the three things M0 exists to prove:

- **MIDI IO thread** — Tasks 2 and 3, with packet walking tested without hardware.
- **`FixedUpdate` tick position** — Task 4, with `TimeUpdateStrategy::FixedTimesteps(1)` making the tick count exact and the determinism test asserting bit-identical output.
- **Fullscreen on an external display** — Task 6, Step 5.

## Deliberately not in M0

Per the spec: no project file, no editor, no node abstraction, no port arena, no transport or clock sync, no scene graph nodes. `host_time` is captured but not converted to a wall clock — that lands with the transport at M3. Duplicate-dependency CI (spec §2.8) waits for M1b, when Vello arrives and the wgpu pin starts to matter.
