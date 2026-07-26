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
