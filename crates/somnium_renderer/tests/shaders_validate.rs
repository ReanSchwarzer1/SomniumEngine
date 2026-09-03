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
use somnium_renderer::shaders::{Shaders, define};

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

/// Validate the actual shading variant, including DREAMS-B's conditional STF.
#[test]
fn the_dreams_stochastic_filter_variant_validates() {
    let shaders = Shaders::new();
    let source = shaders
        .source("shading.wgsl", define::DREAMS_STF)
        .expect("DREAMS_STF must be a registered variant");
    check("shading.wgsl+DREAMS_STF", &source);
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

    assert_eq!(*span, 2080, "WGSL size disagrees with GpuTerrainMaterial");

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
    // TSUSHIMA-G lands in the two words TSUSHIMA-B/C padded, and TSUSHIMA-H's
    // vec4 has to be 16-byte aligned or every `array<vec4<_>>` above it moves.
    assert_eq!(offset("weight_noise_strength"), 2056);
    assert_eq!(offset("macro_octave_strength"), 2064);
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

/// Stochastic terrain filtering must ask the texture how big it is.
///
/// DREAMS-B shipped `max(length(ddx), length(ddy)) * 1024.0` under the comment
/// "authored terrain banks are 1024² today". They are not:
/// `choose_runtime_resolutions` loads hero layers 0-15 at 2048 and only drops
/// them to 1024 when the BC7 budget is exceeded, and the shipped maps log
/// `0-15 at 2048, 16-31 at 1024`. A hardcoded 1024 halves the footprint of
/// every hero layer, which is exactly one mip level too sharp, and a single
/// stochastic tap has no trilinear blend to hide it.
///
/// A source check rather than a render check because the symptom is temporal
/// shimmer at distance, which a still frame cannot show and which a moving
/// capture measures at the same order as its own noise.
#[test]
fn stochastic_terrain_filtering_reads_the_texture_size() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders/terrain_material.wgsl"),
    )
    .expect("terrain_material.wgsl");

    let body = source
        .split_once("fn terrain_stochastic_sample(")
        .expect("terrain_stochastic_sample exists")
        .1;
    let body = &body[..body.find("\n}").unwrap_or(body.len())];

    assert!(
        body.contains("textureDimensions("),
        "the LOD must come from the texture, not from an assumed resolution"
    );
    for assumed in ["1024.0", "2048.0", "512.0"] {
        assert!(
            !body.contains(assumed),
            "`{assumed}` is a hardcoded bank resolution; hero and extra layers \
             differ and the budget can change both"
        );
    }
}

/// Every registered debug view is reachable in the shader.
///
/// `somnium_ui` owns the list of views and `somnium_renderer` owns the shader
/// that branches on their codes, so neither crate could check the two agree.
/// The guard that stood in for it asserted the highest code was 33 -- a
/// hard-coded number that says nothing about whether a branch exists, and that
/// fails on the next view added whether or not the shader was updated.
///
/// A view with no branch renders the ordinary lit image, so the menu entry
/// silently does nothing.
#[test]
fn every_registered_debug_view_has_a_branch_in_the_shader() {
    const SHADING: &str = include_str!("../src/shaders/shading.wgsl");
    let source = SHADING;
    // Branches read `dbg > N.5 && dbg < M.5`; M is the code they select.
    let mut branched: Vec<i32> = Vec::new();
    for (index, _) in source.match_indices("dbg < ") {
        let rest = &source[index + "dbg < ".len()..];
        let end = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(rest.len());
        if let Ok(value) = rest[..end].parse::<f32>() {
            branched.push((value - 0.5).round() as i32);
        }
    }
    let missing: Vec<&str> = somnium_ui::debug::DEBUG_VIEWS
        .iter()
        .filter(|view| view.code > 0.5 && !branched.contains(&(view.code as i32)))
        .map(|view| view.id)
        .collect();
    assert!(
        missing.is_empty(),
        "registered but not branched on in shading.wgsl, so the menu entry          renders the ordinary lit image: {missing:?}"
    );
}

/// The clipmap ships off, and the two places that decide so must agree.
///
/// `somnium_ui` owns the toggle default and `somnium_renderer` owns
/// `TerrainClipmap::env_default_enabled`. They disagreed: the ring constructor
/// said "off until DF-E gates pass" while `debug_toggles` said on, and since
/// `apply_debug_toggles` force-writes the field from the toggle, the toggle
/// won. Anything that reads one and expects the other is wrong about what
/// ships.
#[test]
fn the_clipmap_ships_off_in_both_places() {
    // Cleared here because this process may inherit either variable from a
    // capture run, and the question is what an unset environment does.
    unsafe {
        std::env::remove_var("SOMNIUM_TERRAIN_CLIPMAP");
    }
    assert!(
        !somnium_ui::debug::DebugToggles::from_env().is_on("terrain_clipmap"),
        "the `terrain_clipmap` toggle is on by default again;          `apply_debug_toggles` writes it straight into every ring"
    );
    assert!(
        !somnium_renderer::terrain::clipmap::TerrainClipmap::env_default_enabled(),
        "`env_default_enabled` is on by default again"
    );
}

/// Stochastic mip selection ships off.
///
/// It is the one application of stochastic filtering that cannot pay: a
/// fractional `textureSampleLevel` is trilinear in the sampler at no extra
/// cost, so picking one of the two mips trades a filtered tap for an
/// unfiltered one and buys nothing. Measured at 1.99% of pixels moving between
/// consecutive frames on a stationary camera, against 0.43% for trilinear.
#[test]
fn stochastic_mip_selection_ships_off() {
    unsafe {
        std::env::remove_var("SOMNIUM_DREAMS_STF");
    }
    assert!(
        !somnium_ui::debug::DebugToggles::from_env().is_on("dreams_stf"),
        "`dreams_stf` is on by default again; it costs 4.5x the temporal          instability of the hardware trilinear it replaces"
    );
}

/// Specular antialiasing must run after **everything** that writes the shading
/// normal or the roughness.
///
/// It shipped in TSUSHIMA-E above the terrain branch, which then overwrote both
/// outright — so every terrain pixel computed the filter and discarded it, and
/// the relief normal and decals overwrote it again after that. The comment at
/// the call site claimed it ran last; only the comment did.
///
/// A source-order test rather than an image test because the failure is
/// invisible in a still: a discarded roughness widening looks exactly like a
/// surface that did not need one, right up until it aliases in motion.
#[test]
fn specular_aa_runs_after_every_normal_and_roughness_writer() {
    let source = composed("shading.wgsl");
    let aa = source
        .find("if enable_specular_aa {")
        .expect("the specular AA block is still there");

    // Every later writer of `surface.normal` / `surface.roughness` in the
    // fragment path. If a new one is added below the filter, this fails and
    // the fix is to move the filter, not to delete the line from this list.
    for writer in [
        "surface.roughness = terrain.roughness;",
        "surface.normal = terrain.normal;",
        "widen_roughness_toksvig(surface.roughness, relief.w)",
        "apply_decals(&surface, hit_point, decal_froxel);",
    ] {
        let at = source
            .find(writer)
            .unwrap_or_else(|| panic!("writer moved or was renamed: {writer}"));
        assert!(
            at < aa,
            "`{writer}` runs after specular AA, so the filtered roughness is discarded"
        );
    }

    // And before `f0`, which is derived from roughness-adjacent state and is
    // the first consumer downstream.
    let f0 = source
        .find("surface.f0       = mix(vec3<f32>(0.04)")
        .expect("f0 derivation is still there");
    assert!(aa < f0, "specular AA must run before f0 is derived");
}

/// TSUSHIMA-G's perturbation is worth nothing unless it runs before selection.
///
/// A perturbation applied after `terrain_strongest_four` can only wobble an
/// edge the four winners have already drawn. Applied before, it can change
/// *which* four win, which is what turns an oval into an interlocked boundary.
/// Both are one line apart in the source and produce pictures that differ only
/// in ways nobody notices until the terrain is in front of them — which is the
/// exact shape of the specular-AA bug this file also pins.
#[test]
fn weight_noise_is_applied_before_strongest_four() {
    let source = composed("shading.wgsl");

    // Inside the painted unpack, not at the call sites: two call sites means
    // two chances for one to be missed.
    let unpack = source
        .find("fn terrain_unpack_splats_painted(")
        .expect("terrain_unpack_splats_painted is still there");
    let perturb = source
        .find("terrain_perturb_weights(tm, &w, local_xz);")
        .expect("the perturbation call is still inside the painted unpack");
    let unpack_end = source[unpack..]
        .find("\n}")
        .expect("terrain_unpack_splats_painted has a closing brace")
        + unpack;
    assert!(
        perturb > unpack && perturb < unpack_end,
        "the perturbation left the painted unpack, so a call site can now miss it"
    );

    // And every caller unpacks before it selects.
    let mut sites = 0usize;
    for (at, _) in source.match_indices("terrain_unpack_splats_painted(splat_s, tm, local_xz)") {
        let select = source[at..]
            .find("terrain_strongest_four(&weight)")
            .map(|o| o + at)
            .expect("a call site that unpacks but never selects");
        assert!(
            at < select,
            "weights are selected before they are perturbed"
        );
        sites += 1;
    }
    assert_eq!(
        sites, 2,
        "shading.wgsl composes the live and clipmap-generate paths; \
         a third or a missing one means a path changed shape"
    );
}

/// No ray-query pipeline may reach the splat-weight perturbation.
///
/// This is a memory guard, not a correctness one, and it is worth a test
/// because the failure is invisible in source and catastrophic at runtime.
///
/// `terrain_perturb_weights` is a `terrain_scan`-iteration loop holding two
/// value-noise evaluations, and `terrain_scan` is a compile-time `override`.
/// Routing `rt_hit.wgsl` through it took NVIDIA's Vulkan driver to **47 GB of
/// private memory** compiling ReSTIR-GI's `initial_and_temporal`, and the engine
/// never finished starting. Guarding one pipeline only moved the explosion to
/// the next root that composes this file.
///
/// The raster path retains the painted perturbation. Secondary-ray albedo uses
/// the bounded plain weights, accepting a small boundary mismatch rather than
/// making the application unable to start.
#[test]
fn ray_query_terrain_hit_uses_the_bounded_splat_unpack() {
    for root in [
        "restir_gi.wgsl",
        "lighting_extra.wgsl",
        "water_reflection.wgsl",
    ] {
        let source = composed(root);
        assert!(
            !source.contains("fn terrain_perturb_weights("),
            "{root} still composes the full painted terrain material"
        );
        let start = source
            .find("fn rt_terrain_albedo(")
            .unwrap_or_else(|| panic!("{root} no longer composes the shared terrain-hit path"));
        let end = source[start..]
            .find("\n/// Resolve a committed ray-query intersection")
            .map(|offset| start + offset)
            .unwrap_or_else(|| panic!("could not isolate rt_terrain_albedo in {root}"));
        // Code only. This function's own comments name the constructs banned
        // below in order to explain why they are banned, and a substring scan
        // cannot tell an explanation from a call.
        let terrain_hit: String = source[start..end]
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("
");
        let terrain_hit = terrain_hit.as_str();
        assert!(
            !terrain_hit.contains("terrain_unpack_splats_painted("),
            "{root}'s terrain hit calls the perturbing unpack, recreating the \
             47 GB startup compilation."
        );
        // Stricter than "use the plain unpack", because the plain unpack was
        // not bounded enough. It returns `array<f32, 32>` — a 128-byte local
        // that becomes scratch in a ray shader — and its caller then ran
        // `terrain_strongest_four`, a 32-iteration insertion sort whose bound
        // is an `override` and therefore unrolls to roughly 256 branches. Both
        // survived the earlier module split, which is why that split did not
        // fix the compile. A bounce ray averages per-layer means and needs
        // neither.
        for banned in [
            "terrain_unpack_splats(",
            "terrain_strongest_four(",
            "array<f32, 32>",
        ] {
            assert!(
                !terrain_hit.contains(banned),
                "{root}'s terrain hit reaches `{banned}`, putting a scan array \
                 or an unrolled sort back into a ray-query entry point"
            );
        }
    }
}

/// TSUSHIMA-H's octaves compose with the macro blend only in its own space.
///
/// `terrain_macro_blend`'s overlay and linear-light modes are defined against a
/// perceptual operand, and the albedo is squared back to linear immediately
/// after. An octave multiply that landed below the squaring would be a second,
/// independent gain on linear radiance fighting the blend instead of composing
/// with it — and it would look almost right, which is the problem.
#[test]
fn macro_octaves_land_between_the_macro_blend_and_the_squaring() {
    let source = composed("shading.wgsl");
    let mut found = 0usize;
    for (blend, _) in source.match_indices("albedo = terrain_macro_blend(") {
        let rest = &source[blend..];
        let octaves = rest
            .find("terrain_macro_octaves(local_xz, tm.macro_octave_strength.xyz)")
            .expect("the macro blend is no longer followed by the octaves");
        let square = rest
            .find("albedo = albedo * albedo;")
            .expect("the macro blend is no longer followed by the squaring");
        assert!(
            octaves < square,
            "the octaves are applied to linear albedo, not to the perceptual value \
             `terrain_macro_blend` works in"
        );
        found += 1;
    }
    assert_eq!(
        found, 2,
        "the live and clipmap-generate paths each blend a macro map; \
         a terrain shades through whichever the distance picks and they must agree"
    );
}

/// TSUSHIMA-I's cliff parallax must actually reach the sampler.
///
/// The failure this pins has happened twice in this phase already: a filter
/// that is computed into a local and then not used, with a comment above it
/// saying it is. Here it would be an offset added to a `uv_*` that the sample
/// call below does not read, and the picture would be the pre-TSUSHIMA cliff —
/// correct-looking, flat, and indistinguishable from a cliff that was never
/// meant to have parallax.
#[test]
fn cliff_parallax_reaches_the_projected_sampler() {
    let source = composed("shading.wgsl");
    let start = source
        .find("fn terrain_projected_pbr(")
        .expect("terrain_projected_pbr is still there");
    let end = source[start..]
        .find("\n}")
        .expect("terrain_projected_pbr has a closing brace")
        + start;
    let body = &source[start..end];

    // One plane per world axis, each marching in its own coordinate and each
    // sampling the coordinate it marched.
    for plane in ["uv_x", "uv_y", "uv_z"] {
        let offset = body
            .find(&format!("{plane} += terrain_projected_offset("))
            .unwrap_or_else(|| panic!("{plane} no longer takes a parallax offset"));
        let sampled = body
            .find(&format!("tm, layer, {plane},"))
            .unwrap_or_else(|| panic!("{plane} is no longer what the maps are sampled at"));
        assert!(
            offset < sampled,
            "{plane} is sampled before it is displaced, so the march is discarded"
        );
    }

    // And the heightfield march it replaces is still excluded on cliffs — two
    // parallax solutions applied to one pixel would displace it twice.
    assert!(
        source.contains("let allow_pom = cliff_blend < 0.05;"),
        "the UV-space march is no longer excluded on cliffs"
    );
}

/// Both parallax paths share one march.
///
/// The steep-parallax loop and its single-lookup refinement are the subtle
/// part, and the refinement is the half nobody re-reads. Two copies is two
/// chances for one of them to keep a bug the other lost.
#[test]
fn one_parallax_march_serves_both_frames() {
    let source = composed("shading.wgsl");
    assert_eq!(
        source.matches("fn terrain_parallax_march(").count(),
        1,
        "the march is declared more than once"
    );
    for caller in ["terrain_parallax_offset", "terrain_projected_offset"] {
        let start = source
            .find(&format!("fn {caller}("))
            .unwrap_or_else(|| panic!("{caller} is gone"));
        let end = source[start..].find("\n}").unwrap() + start;
        let body = &source[start..end];
        assert!(
            body.contains("terrain_parallax_march("),
            "{caller} no longer delegates to the shared march"
        );
        assert!(!body.contains("loop {"), "{caller} grew a march of its own");
    }
}
