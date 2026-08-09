//! GPU-free WGSL validation.
//!
//! M1 is otherwise verified entirely by eye, so this is the only automated
//! feedback on shader correctness. naga parses and validates WGSL without a
//! device, which catches syntax and type errors on any machine.
//!
//! Guarantees:
//! - Every `.wgsl` file directly under `assets/shaders` either parses and
//!   validates with naga, or is listed in `PREPROCESSOR_SHADERS` below.
//! - A shader that uses Bevy's `#import` preprocessor (which naga cannot
//!   parse) is only excused from validation if it is named in
//!   `PREPROCESSOR_SHADERS`. Adding a name there is a deliberate, reviewed
//!   act — an unexplained skip fails the test instead of being printed and
//!   forgotten.
//! - Every name listed in `PREPROCESSOR_SHADERS` is checked against what is
//!   actually on disk, so a stale entry (renamed/deleted file) fails loudly
//!   instead of masquerading as "nothing to validate."
//! - Shaders in subdirectories are not supported: the walk is non-recursive,
//!   and the test fails if a subdirectory is found, rather than silently
//!   ignoring its contents.

/// Shaders that use Bevy's `#import` preprocessor and therefore cannot be
/// parsed by naga. Adding a name here is a deliberate decision to give up
/// automated validation for that file — it should be visible in review.
///
/// Only referenced by the validation test below (naga is a dev-dependency),
/// so it is `cfg(test)` to avoid a dead-code warning in normal builds.
#[cfg(test)]
const PREPROCESSOR_SHADERS: &[&str] =
    &["point_cloud.wgsl", "sprite_layer.wgsl", "sprite_depth_spike.wgsl"];

#[cfg(test)]
mod tests {
    use super::PREPROCESSOR_SHADERS;
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
    fn uses_bevy_preprocessor_classifies_correctly() {
        assert!(uses_bevy_preprocessor("#import bevy_pbr::mesh_functions\n"));
        assert!(uses_bevy_preprocessor("#define_import_path my_shader\n"));
        assert!(!uses_bevy_preprocessor(
            "@fragment fn fragment() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }"
        ));
    }

    #[test]
    fn every_shader_parses_and_validates() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/shaders");
        let mut checked = 0;
        let mut seen_allowlisted = Vec::new();
        let mut failures = Vec::new();
        let mut subdirs = Vec::new();

        for entry in std::fs::read_dir(&dir).expect("assets/shaders must exist") {
            let path = entry.unwrap().path();

            if path.is_dir() {
                subdirs.push(path.file_name().unwrap().to_string_lossy().to_string());
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("wgsl") {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let src = std::fs::read_to_string(&path).unwrap();

            if uses_bevy_preprocessor(&src) {
                if PREPROCESSOR_SHADERS.contains(&name.as_str()) {
                    seen_allowlisted.push(name);
                } else {
                    failures.push(format!(
                        "{name}: uses Bevy's #import preprocessor, which naga cannot parse, \
                         but is not in PREPROCESSOR_SHADERS. Either drop the #import lines so \
                         this shader can be validated, or add \"{name}\" to \
                         PREPROCESSOR_SHADERS in shader_validation.rs as a deliberate, \
                         reviewed decision to skip it."
                    ));
                }
                continue;
            }
            match validate_wgsl(&name, &src) {
                Ok(()) => checked += 1,
                Err(e) => failures.push(e),
            }
        }

        assert!(
            subdirs.is_empty(),
            "nested shader directories are not supported by this harness (found: {subdirs:?}); \
             flatten assets/shaders or extend the walk to recurse"
        );

        for expected in PREPROCESSOR_SHADERS {
            assert!(
                seen_allowlisted.iter().any(|n| n == expected),
                "PREPROCESSOR_SHADERS lists \"{expected}\" but no such file was found under \
                 assets/shaders; the file may have been renamed or the path is wrong"
            );
        }

        if !seen_allowlisted.is_empty() {
            println!("NOT VALIDATED (allowlisted bevy preprocessor imports): {seen_allowlisted:?}");
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
