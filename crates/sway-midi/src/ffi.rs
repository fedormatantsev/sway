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

// Apple's MIDIServices.h wraps these in `#pragma pack(push, 4)` (line 446,
// popped at 613), so a plain `#[repr(C)]` is wrong here: the `u64 time_stamp`
// would force 8-byte alignment on the struct and shift every field after it
// relative to what CoreMIDI actually writes. `packed(4)` reproduces the SDK's
// pack-to-4 behaviour: fields keep their natural alignment capped at 4 bytes,
// matching the measured C layout (MIDIPacket size=268 align=4, .timeStamp@0
// .length@8 .data@10; MIDIPacketList size=272 align=4, .numPackets@0
// .packet@4). See the layout test in `lib.rs` for the numbers this must
// satisfy.
#[repr(C, packed(4))]
pub struct MIDIPacketList {
    pub num_packets: u32,
    pub packet: [MIDIPacket; 1],
}

#[repr(C, packed(4))]
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

    pub fn MIDIPortDispose(port: MIDIPortRef) -> OSStatus;
    pub fn MIDIClientDispose(client: MIDIClientRef) -> OSStatus;

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
