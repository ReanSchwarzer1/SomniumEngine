//! Parse and validate every shader module the renderer builds.
//!
//! wgpu compiles WGSL when the pipeline is created, which is at startup on a
//! machine with a GPU — so until now a typo, a stale struct mirror or an
//! out-of-order declaration surfaced as a first-frame crash and nowhere in CI.
//! naga is wgpu's own front end, so running it over the same concatenations the
//! passes assemble catches that class of error in `cargo test`.
//!
//! This validates the *modules*, not the pipelines: bind-group layout mismatches
//! and vertex-format disagreements still need a device.

use naga::valid::{Capabilities, ValidationFlags, Validator};

const BRDF: &str = include_str!("../src/shaders/brdf.wgsl");
const SAMPLING: &str = include_str!("../src/shaders/sampling.wgsl");
const ATMOSPHERE: &str = include_str!("../src/shaders/atmosphere.wgsl");
const HEXTILE: &str = include_str!("../src/shaders/hextile.wgsl");
const TERRAIN_MATERIAL: &str = include_str!("../src/shaders/terrain_material.wgsl");
const SHADING: &str = include_str!("../src/shaders/shading.wgsl");
const VISIBILITY: &str = include_str!("../src/shaders/visibility.wgsl");
const SHADOW: &str = include_str!("../src/shaders/shadow.wgsl");
const TRANSPARENT: &str = include_str!("../src/shaders/transparent.wgsl");

/// Parse and validate one module, panicking with naga's own diagnostic.
fn check(label: &str, source: &str) {
    let module = match naga::front::wgsl::parse_str(source) {
        Ok(m) => m,
        Err(e) => panic!("{label} failed to parse:\n{}", e.emit_to_string(source)),
    };
    // The renderer's bindless pools need these; without them validation rejects
    // the binding arrays every pass depends on.
    let capabilities = Capabilities::all();
    let mut validator = Validator::new(ValidationFlags::all(), capabilities);
    if let Err(e) = validator.validate(&module) {
        panic!("{label} failed validation:\n{}", e.emit_to_string(source));
    }
}

#[test]
fn the_shading_module_validates() {
    // The exact concatenation `ShadingPass::new` builds. Phase 25A-2 added
    // terrain_material.wgsl to it, which is also the check that terrain's
    // material functions can see `textures` and `default_sampler` even though
    // those are declared in a later file — module-scope declarations in WGSL
    // are order-independent, and this is what proves it rather than assuming.
    check(
        "shading",
        &format!("{BRDF}\n{SAMPLING}\n{ATMOSPHERE}\n{HEXTILE}\n{TERRAIN_MATERIAL}\n{SHADING}"),
    );
}

#[test]
fn the_visibility_module_validates() {
    check("visibility", VISIBILITY);
}

#[test]
fn the_shadow_module_validates() {
    check("shadow", SHADOW);
}

#[test]
fn the_transparent_module_validates() {
    check("transparent", &format!("{BRDF}\n{TRANSPARENT}"));
}
