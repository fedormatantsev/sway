//! GPU-free WGSL validation.
//!
//! M1 is otherwise verified entirely by eye, so this is the only automated
//! feedback on shader correctness. naga parses and validates WGSL without a
//! device, which catches syntax and type errors on any machine.
//!
//! Limitation: naga does not understand Bevy's `#import` preprocessor, so
//! shaders using it are skipped. Skips are printed rather than silent — an
//! unvalidated shader should be visible, not forgotten.

#[cfg(test)]
mod tests {
    use std::path::Path;

    fn validate_wgsl(name: &str, src: &str) -> Result<(), String> {
        let module = naga::front::wgsl::parse_str(src)
            .map_err(|e| format!("{name}: parse failed:\n{}", e.emit_to_string(src)))?;

        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );

        validator
            .validate(&module)
            .map_err(|e| format!("{name}: validation failed: {e:?}"))?;

        Ok(())
    }

    fn uses_bevy_preprocessor(src: &str) -> bool {
        src.lines().any(|l| {
            let t = l.trim_start();
            t.starts_with("#import") || t.starts_with("#define_import_path")
        })
    }

    #[test]
    fn every_shader_parses_and_validates() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/shaders");
        let mut checked = 0;
        let mut skipped = Vec::new();
        let mut failures = Vec::new();

        for entry in std::fs::read_dir(&dir).expect("assets/shaders must exist") {
            let path = entry.unwrap().path();
            if path.extension().and_then(|s| s.to_str()) != Some("wgsl") {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let src = std::fs::read_to_string(&path).unwrap();

            if uses_bevy_preprocessor(&src) {
                skipped.push(name);
                continue;
            }
            match validate_wgsl(&name, &src) {
                Ok(()) => checked += 1,
                Err(e) => failures.push(e),
            }
        }

        if !skipped.is_empty() {
            println!("NOT VALIDATED (bevy preprocessor imports): {skipped:?}");
        }
        println!("validated {checked} shader(s)");
        assert!(failures.is_empty(), "shader validation failed:\n{}", failures.join("\n\n"));
    }

    #[test]
    fn validator_rejects_a_type_error() {
        // Guards the harness itself: if this ever passes, the validator has
        // been neutered and the test above is worthless.
        let bad = "@fragment fn fragment() -> @location(0) vec4<f32> { return vec3<f32>(1.0, 0.0, 0.0); }";
        assert!(validate_wgsl("bad", bad).is_err());
    }
}
