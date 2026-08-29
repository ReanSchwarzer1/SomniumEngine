//! Parse and validate every shader module the renderer builds.
//!
//! wgpu compiles WGSL when the pipeline is created, which is at startup on a
//! machine with a GPU — so a typo, a stale struct mirror or an out-of-order
//! declaration would surface as a first-frame crash and nowhere in CI. naga is
//! wgpu's own front end, so running it over the sources the passes assemble
//! catches that class of error in `cargo test`.
//!
//! This validates the *modules*, not the pipelines: bind-group layout
//! mismatches and vertex-format disagreements still need a device.
//!
//! # What MORROWIND-C changed here, and why it matters
//!
//! Every composed module used to be a `format!` in this file **mirroring** a
//! `format!` in a pass constructor. Two copies of an ordering that had to
//! agree, with nothing enforcing that they did — and `restir_gi.rs`'s own
//! comment said *"`tests/shaders_validate.rs` pins this exact concatenation"*,
//! which is a description of a convention, not a mechanism.
//!
//! Composition now lives in the `.wgsl` files as `//!include` directives, and
//! this file resolves them through the same [`Shaders`] registry the renderer
//! uses. **There is one description of what a shader is made of**, and this
//! test validates it rather than a copy of it. A `//!include` that names a
//! missing file, a cycle, or a typo in a `//!if` fails here.

use naga::valid::{Capabilities, ValidationFlags, Validator};
use somnium_renderer::shaders::Shaders;

// Modules that compose nothing still validate on their own, so their text is
// still read directly. Everything with dependencies goes through `Shaders`.
const SPD: &str = include_str!("../src/shaders/spd.wgsl");
const VELOCITY: &str = include_str!("../src/shaders/velocity.wgsl");
const MOTION_BLUR: &str = include_str!("../src/shaders/motion_blur.wgsl");
const CAS: &str = include_str!("../src/shaders/cas.wgsl");
const PRESENT: &str = include_str!("../src/shaders/present.wgsl");
const VISIBILITY: &str = include_str!("../src/shaders/visibility.wgsl");
const SHADOW: &str = include_str!("../src/shaders/shadow.wgsl");
const WATER: &str = include_str!("../src/shaders/water.wgsl");
const WATER_SPECTRUM: &str = include_str!("../src/shaders/water_spectrum.wgsl");
const UNDERWATER: &str = include_str!("../src/shaders/underwater.wgsl");
const CLOUDS_NOISE: &str = include_str!("../src/shaders/clouds_noise.wgsl");
const CLOUDS_COMPOSITE: &str = include_str!("../src/shaders/clouds_composite.wgsl");

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

/// Resolve a composed root through the same registry the renderer uses.
fn composed(name: &str) -> String {
    Shaders::new().source_or_panic(name)
}

/// **Every composed root validates.**
///
/// This is MORROWIND-C's acceptance test and it replaces thirteen hand-written
/// concatenations that mirrored thirteen others. Adding a shader is one line in
/// `shaders.rs` and its `//!include` header; nothing here has to be updated to
/// match, which is the point — the previous arrangement could drift and this
/// one cannot.
#[test]
fn every_composed_root_validates() {
    let shaders = Shaders::new();
    for root in [
        "shading.wgsl",
        "restir_gi.wgsl",
        "lighting_extra.wgsl",
        "water_reflection.wgsl",
        "census.wgsl",
        "classify.wgsl",
        "clipmap_gen.wgsl",
        "volumetric.wgsl",
        "clouds.wgsl",
        "atmosphere_lut.wgsl",
        "ibl_gen.wgsl",
        "dof.wgsl",
        "gtao.wgsl",
        // MORROWIND-AC: three entry points in one module, and the one that
        // matters is `fs_weights` — its two search loops are the shape naga
        // is fussiest about.
        "smaa.wgsl",
        // MORROWIND-AC: `transparent.wgsl` now has two fragment entry points
        // sharing one `shade`, and this is what proves the MRT one composes.
        "transparent.wgsl",
        "oit_composite.wgsl",
        // MORROWIND-U. Composes global_pool.wgsl for `Vertex`, which is the
        // point: the posed vertices it writes have the same layout as every
        // other vertex in the pool, because they are in the same pool.
        "skinning.wgsl",
    ] {
        check(root, &shaders.source_or_panic(root));
    }
}

/// The shading module, kept as its own test because it is the acceptance case.
///
/// Phase 25A-2 added `terrain_material.wgsl` to the composition, which is also
/// the check that terrain's material functions can see `textures` and
/// `default_sampler` even though those are declared in a different file —
/// module-scope declarations in WGSL are order-independent, and this is what
/// proves it rather than assuming.
#[test]
fn the_shading_module_validates() {
    check("shading", &composed("shading.wgsl"));
}

/// Phase CONTROL-M. The march reuses `sample_transmittance`,
/// `sample_multiscatter` and `ray_hits_ground`, which is what proves the clouds
/// and the sky read the same atmosphere rather than each carrying a copy — now
/// declared by `//!include "atmosphere.wgsl"` at the top of `clouds.wgsl`
/// rather than by the order of two `include_str!` calls in `CloudPass::new`.
#[test]
fn the_cloud_modules_validate() {
    check("clouds_noise", CLOUDS_NOISE);
    check("clouds", &composed("clouds.wgsl"));
    check("clouds_composite", CLOUDS_COMPOSITE);
}

/// Phase DOOM-B/C. The census and the classifier share `pixel_class.wgsl`,
/// which is the structural guarantee that a tile is routed by the same test
/// that counted it — and both read `instances`, `materials` and `view` from the
/// same global pool the shading pass does, so a census cannot classify a pixel
/// differently from the pass it is describing.
#[test]
fn the_census_and_classify_modules_validate() {
    check("census", &composed("census.wgsl"));
    check("classify", &composed("classify.wgsl"));
}

#[test]
fn the_clipmap_generate_module_validates() {
    check("clipmap_gen", &composed("clipmap_gen.wgsl"));
}

/// The froxel volume for aerial perspective and fog (24U/25I), which composes
/// the atmosphere so it reuses its density, phase and LUT helpers rather than
/// defining a second atmosphere.
#[test]
fn the_volumetric_module_validates() {
    check("volumetric", &composed("volumetric.wgsl"));
}

/// Phase 24L. The GI pass binds the same `@group(0)` pool the shading pass
/// does, which is the point: a ray hit and a visibility-buffer hit resolve
/// through one description of the scene, not two that could drift apart.
///
/// `enable wgpu_ray_query;` is hoisted by the resolver, so the old requirement
/// that `restir_gi.wgsl` be concatenated *first* has stopped being a rule
/// somebody has to remember.
#[test]
fn the_restir_gi_module_validates() {
    check("restir_gi", &composed("restir_gi.wgsl"));
}

#[test]
fn the_lighting_extra_module_validates() {
    check("lighting_extra", &composed("lighting_extra.wgsl"));
}

/// Phase VV. Same modules `WaterReflectionPass::new` builds, now declared in
/// `water_reflection.wgsl` itself.
#[test]
fn the_water_reflection_module_validates() {
    check("water_reflection", &composed("water_reflection.wgsl"));
}

/// The standalone post and utility modules. Each declares its own bindings
/// and pulls in nothing, so each validates alone — and every one of them has
/// already caught something: a reserved keyword in SPD, a reserved parameter
/// name in the GI module, three struct-field mismatches.
#[test]
fn the_standalone_post_modules_validate() {
    check("spd", SPD);
    check("velocity", VELOCITY);
    check("motion_blur", MOTION_BLUR);
    check("cas", CAS);
    check("present", PRESENT);
}

#[test]
fn the_visibility_module_validates() {
    check("visibility", VISIBILITY);
}

#[test]
fn the_shadow_module_validates() {
    check("shadow", SHADOW);
}

/// The forward transparent pass, which composes nothing.
///
/// This test used to validate `{BRDF}` concatenated with `{TRANSPARENT}` — a
/// pairing `TransparentPass::new` never built. It compiles `transparent.wgsl`
/// alone, and the module calls none of `brdf.wgsl`'s three functions. The test
/// was over-approximating, and MORROWIND-C found it by making the test resolve
/// what the pass actually builds. Two descriptions of one shader will drift;
/// that is the whole argument for having one.
#[test]
fn the_transparent_module_validates() {
    check("transparent", &composed("transparent.wgsl"));
}

#[test]
fn the_phase_iv_water_modules_validate() {
    check("water", WATER);
    check("water_spectrum", WATER_SPECTRUM);
    check("underwater", UNDERWATER);
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
    let source = composed("shading.wgsl");
    let module = naga::front::wgsl::parse_str(&source).expect("shading module parses");

    let (_, ty) = module
        .types
        .iter()
        .find(|(_, t)| t.name.as_deref() == Some("TerrainMaterial"))
        .expect("TerrainMaterial is declared");

    let naga::TypeInner::Struct { members, span } = &ty.inner else {
        panic!("TerrainMaterial is not a struct");
    };

    assert_eq!(*span, 2032, "WGSL size disagrees with GpuTerrainMaterial");

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
    assert_eq!(offset("brush"), 128);
    assert_eq!(offset("albedo_maps"), 144);
    assert_eq!(offset("surface_maps"), 272);
    assert_eq!(offset("terrain_origin"), 400);
    assert_eq!(offset("inv_world_size"), 408);
    assert_eq!(offset("splat_maps"), 416);
    assert_eq!(offset("hex_tiling"), 452);
    assert_eq!(offset("layer_height_scale"), 464);
    assert_eq!(offset("layer_blend_width"), 592);
    assert_eq!(offset("layer_weight_clamp"), 720);
    assert_eq!(offset("layer_parallax"), 848);
    assert_eq!(offset("macro_mode"), 976);
    assert_eq!(offset("macro_strength"), 980);
    assert_eq!(offset("detail_fade_start"), 984);
    assert_eq!(offset("detail_fade_end"), 988);
    assert_eq!(offset("layer_albedo"), 992);
    assert_eq!(offset("parallax_steps"), 1504);
    assert_eq!(offset("projection_sharpness"), 1512);
    assert_eq!(offset("layer_moisture"), 1520);
    assert_eq!(offset("wetness"), 1648);
    assert_eq!(offset("clipmap_enabled"), 1664);
    assert_eq!(offset("clipmap_albedo"), 1680);
    assert_eq!(offset("clipmap_center"), 1744);
    assert_eq!(offset("clipmap_tpm"), 1872);
    assert_eq!(offset("clipmap_macro_rings"), 2016);
}

/// The `enable` directives survive composition and end up first.
///
/// `restir_gi.wgsl` and `lighting_extra.wgsl` both declare
/// `enable wgpu_ray_query;`, and WGSL requires every `enable` to precede every
/// declaration. Before this, the rule was satisfied by concatenating those two
/// files *first* and leaving a comment explaining why — which is a rule
/// somebody has to remember, in two places, forever. The resolver hoists them
/// instead, and this is the check that it does.
#[test]
fn enable_directives_are_hoisted_to_the_top_of_a_composed_module() {
    for root in [
        "restir_gi.wgsl",
        "lighting_extra.wgsl",
        "water_reflection.wgsl",
    ] {
        let source = composed(root);
        let first = source
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default();
        assert!(
            first.trim_start().starts_with("enable "),
            "{root}: expected an `enable` first, found `{first}`"
        );
        assert_eq!(
            source.matches("enable wgpu_ray_query;").count(),
            1,
            "{root}: a duplicated `enable` is a parse error"
        );
    }
}
