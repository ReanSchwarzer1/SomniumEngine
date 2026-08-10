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
const ATMOSPHERE_VOL: &str = include_str!("../src/shaders/volumetric.wgsl");
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
fn the_volumetric_module_validates() {
    // The froxel volume for aerial perspective and fog (24U/25I), which is
    // concatenated after the atmosphere so it can reuse its density, phase and
    // LUT helpers rather than defining a second atmosphere.
    check("volumetric", &format!("{ATMOSPHERE}
{ATMOSPHERE_VOL}"));
}

/// The WGSL side of the CPU/GPU struct mirrors, checked against the Rust side.
///
/// `material/pool.rs` asserts `GpuTerrainMaterial`'s `repr(C)` offsets, which
/// only ever proved half of the agreement — the WGSL half was a comment. This
/// closes it: naga computes the same layout the GPU will, so a member whose
/// alignment differs between the two languages fails here instead of silently
/// decoding the wrong words. Phase 25E hit exactly that with a trailing
/// `vec3<u32>` pad, which aligns to 16 in WGSL and to 4 in Rust.
#[test]
fn the_terrain_material_struct_matches_the_rust_layout() {
    let source =
        format!("{BRDF}\n{SAMPLING}\n{ATMOSPHERE}\n{HEXTILE}\n{TERRAIN_MATERIAL}\n{SHADING}");
    let module = naga::front::wgsl::parse_str(&source).expect("shading module parses");

    let (_, ty) = module
        .types
        .iter()
        .find(|(_, t)| t.name.as_deref() == Some("TerrainMaterial"))
        .expect("TerrainMaterial is declared");

    let naga::TypeInner::Struct { members, span } = &ty.inner else {
        panic!("TerrainMaterial is not a struct");
    };

    assert_eq!(*span, 272, "WGSL size disagrees with GpuTerrainMaterial");

    // Only the members whose offsets the Rust test also pins. Checking every
    // one would just restate the declaration; these are the ones where a
    // vec2/vec3/array alignment rule could move something.
    let offset = |name: &str| {
        members
            .iter()
            .find(|m| m.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no member {name}"))
            .offset
    };
    assert_eq!(offset("layer_tiling"), 0);
    assert_eq!(offset("brush"), 32);
    assert_eq!(offset("albedo_maps"), 48);
    assert_eq!(offset("surface_maps"), 80);
    assert_eq!(offset("terrain_origin"), 112);
    assert_eq!(offset("inv_world_size"), 120);
    assert_eq!(offset("splat_map"), 128);
    assert_eq!(offset("hex_tiling"), 140);
    assert_eq!(offset("layer_height_scale"), 144);
    assert_eq!(offset("layer_blend_width"), 176);
    assert_eq!(offset("layer_weight_clamp"), 208);
    assert_eq!(offset("height_blend"), 240);
    assert_eq!(offset("macro_map"), 244);
    assert_eq!(offset("macro_strength"), 252);
    assert_eq!(offset("detail_fade_start"), 256);
    assert_eq!(offset("detail_fade_end"), 260);
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
