// Somnium Engine — Terrain material (Phase 14C, folded into the shared shading
// path by Phase 25A-2).
//
// This is what is left of `terrain.wgsl` after the move: splatmap-weighted
// blending of four PBR layers, height-based blend sharpening, a triplanar cliff
// projection on steep slopes, and the editor's brush cursor ring. Its copies of
// `sample_shadow`, the cascade selection and the clustered-light lookup are
// gone — terrain shades in `shading.wgsl` now, so each of those exists once,
// which is the entire point of the sub-phase.
//
// Concatenated ahead of `shading.wgsl` and reads that file's `textures` binding
// array and `default_sampler`. Layer maps reach the bindless array as
// single-layer views of the same `texture_2d_array`s the terrain has always
// owned, so nothing is copied to get here.
//
// References:
// - bevy_triplanar_splatting (example_repo/bevy-plugins/) — array-texture splat
//   sampling + triplanar weight blending.

//!include "terrain_splat_core.wgsl"

// Pipeline overrides. Defaults keep the full path (clipmap generate, naga).
// The shading PSO sets these so unused hex/POM code is deleted — runtime
// uniforms do not change occupancy, which is why the Details checkboxes
// never moved Shading ms.
override enable_hex: bool = true;
override enable_pom: bool = true;
/// False when every terrain queued this frame shades through the clipmap
/// (Phase DF). The cache already holds strongest-four + hex + height-blend, so
/// `evaluate_terrain_material` becomes unreachable and the backend drops it —
/// along with the 8 splat fetches, the 32-entry scan arrays and the POM march
/// it would otherwise contribute to register pressure for nothing.
///
/// Must stay `true` unless the CPU has checked the *same* condition
/// `TerrainClipmap::fill_gpu` writes into `clipmap_enabled`, or a terrain will
/// find neither path.
override enable_live_terrain: bool = true;

/// Below this weight a layer cannot change the result, so it is not sampled.
///
/// This is what makes eight layers cheaper than the four used to be. Splat
/// weights are sparse — two or three materials meet at any given texel and the
/// rest are zero — so gating turns a fixed 16 samples (and 48 with hex-tiling)
/// into the four or six that actually contribute. It is only legal because the
/// terrain path samples with explicit derivatives throughout: `textureSampleGrad`
/// has no uniformity requirement, where `textureSample` inside this branch
/// would be undefined.
const LAYER_WEIGHT_EPSILON: f32 = 0.002;

/// The gate `LAYER_WEIGHT_EPSILON` rises to past `detail_fade_end` (Phase 25D).
///
/// 0.2 admits at most four layers and in practice one or two, because splat
/// weights sum to one and a far pixel is almost always inside a single
/// material. Higher was tempting and wrong: at 0.5 only one layer can ever
/// survive, so a genuine 51/49 boundary snaps to one material and the seam
/// crawls as the camera moves.
const FAR_LAYER_EPSILON: f32 = 0.2;

fn terrain_layer_tiling(tm: TerrainMaterial, layer: u32) -> f32 {
    return tm.layer_tiling[layer / 4u][layer % 4u];
}

fn terrain_height_scale(tm: TerrainMaterial, layer: u32) -> f32 {
    return tm.layer_height_scale[layer / 4u][layer % 4u];
}

/// Transition-band width, floored so the depth blend never divides by zero.
fn terrain_blend_width(tm: TerrainMaterial, layer: u32) -> f32 {
    return max(tm.layer_blend_width[layer / 4u][layer % 4u], 0.001);
}

fn terrain_weight_clamp(tm: TerrainMaterial, layer: u32) -> f32 {
    return tm.layer_weight_clamp[layer / 4u][layer % 4u];
}

fn terrain_parallax_depth(tm: TerrainMaterial, layer: u32) -> f32 {
    return tm.layer_parallax[layer / 4u][layer % 4u];
}

/// Value noise on the integer lattice, smoothstep-interpolated.
///
/// Shared by TSUSHIMA-G's weight perturbation and TSUSHIMA-H's macro octaves,
/// which want the same field at very different scales — metres for one,
/// kilometres for the other.
///
/// The hash is an integer bit-mix, not the usual `fract(sin(dot(p, k)) * n)`.
/// That one-liner is fine for a screen-space dither and wrong here: its period
/// collapses once the argument is large, and world coordinates on these maps
/// reach a couple of thousand metres. Its failure mode is diagonal banding at
/// exactly the low frequencies this exists to supply.
fn terrain_noise_hash(cell: vec2<i32>) -> f32 {
    // `bitcast`, not `u32(...)`: a value conversion of a negative i32 is not
    // defined to wrap, and `local_xz` is negative for anything off the
    // terrain's positive quadrant.
    var h = bitcast<u32>(cell.x) * 0x8DA6B343u ^ bitcast<u32>(cell.y) * 0xD8242BA5u;
    h ^= h >> 16u;
    h = h * 0x85EBCA6Bu;
    h ^= h >> 13u;
    h = h * 0xC2B2AE35u;
    h ^= h >> 16u;
    return f32(h >> 8u) / 16777216.0;
}

fn terrain_value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    // Smoothstep rather than a straight lerp. Linear interpolation of a value
    // lattice has a discontinuous derivative at every cell edge, and a
    // discontinuous derivative in a field that perturbs a material *boundary*
    // is a straight line in the boundary — a grid, which is the one artifact
    // this whole sub-phase exists to remove.
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a00 = terrain_noise_hash(c);
    let a10 = terrain_noise_hash(c + vec2<i32>(1, 0));
    let a01 = terrain_noise_hash(c + vec2<i32>(0, 1));
    let a11 = terrain_noise_hash(c + vec2<i32>(1, 1));
    return mix(mix(a00, a10, u.x), mix(a01, a11, u.x), u.y);
}

/// Two octaves of value noise for one layer's splat weight (Phase TSUSHIMA-G).
///
/// Indexed by **world position**, never by UV and never by anything derived
/// from the camera. A noise field that moves with the view crawls; the mirror
/// of that mistake — indexing a dither by UV rather than by screen position —
/// is written up at length in this file's stochastic-filtering comment. Here
/// the world is the stable frame, because the boundary being perturbed is a
/// property of the ground and not of who is looking at it.
///
/// The per-layer offset is what makes this break a boundary rather than move
/// it. Perturb every layer with the same field and the weights rise and fall
/// together, the ranking between them never changes, and the whole edge just
/// translates. Two layers that disagree about where the noise is interlock.
fn terrain_weight_noise(world_xz: vec2<f32>, layer: u32, scale: f32) -> f32 {
    let o = vec2<f32>(f32(layer) * 13.37, f32(layer) * 7.77);
    let n0 = terrain_value_noise((world_xz + o) * scale);
    let n1 = terrain_value_noise((world_xz + o) * scale * 5.43);
    return (n0 - 0.5) + (n1 - 0.5) * 0.5;
}

/// Perturb normalised splat weights, in place, before strongest-four runs.
///
/// **Before** selection is the whole point: a perturbation applied afterwards
/// can only wobble an edge the four winners already drew, where one applied
/// beforehand can change *which* four win. That is the difference between a
/// wobbly oval and an interlocked one.
///
/// Scaled by `4·w·(1−w)`, which is zero at both ends and one at w = 0.5, so a
/// fully-painted area and a bare area both stay exactly put and only the
/// transition band moves. Without the envelope, noise punches holes in the
/// middle of solid ground: an author who painted a road gets gravel in it.
///
/// Renormalises afterwards. The envelope is symmetric but the noise sample is
/// not, so the perturbed weights do not sum to one on their own, and
/// `discarded` downstream is defined as the weight the strongest-four
/// selection threw away. Letting that drift makes a debug channel lie about
/// the selection it exists to report.
fn terrain_perturb_weights(
    tm: TerrainMaterial,
    weight: ptr<function, array<f32, 32>>,
    local_xz: vec2<f32>,
) {
    let amount = tm.weight_noise_strength;
    let scale = max(tm.weight_noise_scale, 1.0e-4);
    var total = 0.0;
    for (var i = 0u; i < terrain_scan; i = i + 1u) {
        var w = (*weight)[i];
        // Splat weights are sparse — two or three layers meet at any texel and
        // the rest are exactly zero. Skipping those is what keeps this from
        // costing 32 two-octave evaluations a pixel, and the branch is coherent
        // across a warp because neighbouring texels agree about which layers
        // are painted there.
        if w > 0.0 && w < 1.0 {
            w = saturate(
                w + terrain_weight_noise(local_xz, i, scale) * amount * w * (1.0 - w) * 4.0);
            (*weight)[i] = w;
        }
        total += w;
    }
    let inv = 1.0 / max(total, 0.0001);
    for (var i = 0u; i < terrain_scan; i = i + 1u) {
        (*weight)[i] = (*weight)[i] * inv;
    }
}

/// The same weights with TSUSHIMA-G's boundary perturbation applied.
///
/// # Why this is a separate function and not a flag
///
/// `terrain_perturb_weights` is a `terrain_scan`-iteration loop containing two
/// value-noise evaluations, and `terrain_scan` is an `override` — a
/// compile-time constant. A backend can therefore fully unroll it: 32
/// iterations times two octaves times four lattice hashes permits 256 inlined
/// hash bodies in whatever calls this.
///
/// In the raster shading pass that is affordable and was measured to be.
/// Inlined into a **ray-query** pipeline it is not: `rt_hit.wgsl` is composed
/// into `restir_gi.wgsl`, `lighting_extra.wgsl` and `water_reflection.wgsl`, and
/// NVIDIA's Vulkan driver compiling ReSTIR-GI's `initial_and_temporal` with
/// this in the hit path reached **47 GB of private memory** and never finished
/// startup. Guarding one pipeline just moved the explosion to the next
/// ray-query root that composes this file.
///
/// So the split is not stylistic. The traced paths call the plain unpack. This
/// can make a painted boundary slightly less irregular in secondary lighting,
/// but the raster path keeps the authored perturbation and startup compilation
/// remains bounded.
fn terrain_unpack_splats_painted(
    s: array<vec4<f32>, 8>,
    tm: TerrainMaterial,
    local_xz: vec2<f32>,
) -> array<f32, 32> {
    var w = terrain_unpack_splats(s);
    // Strength 0 is the exact identity — not "noise scaled to nothing", the
    // original weights untouched and two loops not run.
    if tm.weight_noise_strength > 0.0 {
        terrain_perturb_weights(tm, &w, local_xz);
    }
    return w;
}

fn ts_to_surfgrad(n_ts: vec3<f32>, tangent: vec3<f32>, bitangent: vec3<f32>) -> vec3<f32> {
    let g = n_ts.xy / max(n_ts.z, 0.2);
    return tangent * g.x + bitangent * g.y;
}

fn resolve_surfgrad(n_geo: vec3<f32>, g: vec3<f32>) -> vec3<f32> {
    return normalize(n_geo - g);
}

// ── Parallax occlusion mapping (Phase 25H) ───────────────────────────────────
//
// Terrain is the surface most often seen at a grazing angle, and that is exactly
// where a normal map stops working: it shades a flat plane as though it had
// relief, but the relief never *moves* against the surface, so the ground reads
// as a photograph lying on glass. Parallax fixes the one thing a normal map
// cannot — it displaces where each texel appears, so a pebble occludes the
// crack behind it and the whole surface gains depth as the camera moves.
//
// # Working in metres, not UV
//
// The usual formulation marches in tangent-space UV. Somnium's terrain has a
// world-aligned tangent frame (tangent is +X projected onto the surface, see
// `evaluate_terrain_material`) and **eight layers with different tiling**, so a
// UV offset would mean something different for every layer. Marching in world
// XZ metres instead gives one offset that is correct for all of them: each
// layer converts it with its own tiling exactly as it converts the position.
//
// # Reference
//
// - `bevy/crates/bevy_pbr/src/render/parallax_mapping.wgsl` — steep parallax
//   plus the single-lookup POM refinement, and the reason every fetch is
//   `textureSampleLevel`: a `textureSample` inside a loop needs derivatives,
//   which forces the compiler to unroll a loop whose bound is dynamic.
// - `o3de/.../ShaderLib/Atom/Features/ParallaxMapping.azsli` —
//   `AdvancedParallaxMapping`'s march toward the light, which is what makes
//   relief look lit rather than merely displaced.

/// One height sample of `layer` at a texture coordinate.
fn terrain_parallax_height(tm: TerrainMaterial, layer: u32, uv: vec2<f32>) -> f32 {
    let map = tm.albedo_maps[layer / 4u][layer % 4u];
    if map < 0 {
        return 0.5;
    }
    // Level 0 explicitly. Inside a march the derivatives are meaningless — the
    // taps walk along a ray, not across the screen — and asking for them would
    // both pick a wrong mip and force the loop to unroll.
    return textureSampleLevel(textures[map], default_sampler, uv, 0.0).a;
}

/// Steep parallax plus a POM refinement, in whatever frame the caller samples
/// the height map in.
///
/// The march is the behaviour; the frame is the interface. Both of this file's
/// parallax paths — the heightfield's world-XZ one and the cliff projection's
/// plane-local one — reduce to "start at the peak, step along the ray until you
/// are under the surface, interpolate the last two". Writing that twice is how
/// the two copies end up disagreeing about the refinement, which is the one
/// part nobody re-reads.
///
/// `uv` is where the surface would be with no displacement, `step_uv` is one
/// layer's worth of lateral travel in the same units, and the result is the
/// offset to add to `uv`.
fn terrain_parallax_march(
    tm: TerrainMaterial,
    layer: u32,
    uv: vec2<f32>,
    step_uv: vec2<f32>,
    layers: f32,
) -> vec2<f32> {
    let layer_depth = 1.0 / layers;
    var offset = vec2<f32>(0.0);
    var ray_depth = 0.0;
    // The height map is 1 at the peak; the ray starts at the peak and descends.
    var surface = 1.0 - terrain_parallax_height(tm, layer, uv);

    var i = 0.0;
    loop {
        if surface <= ray_depth || i >= layers {
            break;
        }
        offset += step_uv;
        ray_depth += layer_depth;
        surface = 1.0 - terrain_parallax_height(tm, layer, uv + offset);
        i = i + 1.0;
    }

    // POM refinement: one extra lookup, interpolating between the step that
    // crossed the surface and the one before it. Relief mapping's binary search
    // is more exact and costs a lookup per bisection; at ground-detail depths
    // the difference is below a pixel.
    let prev_offset = offset - step_uv;
    let after = surface - ray_depth;
    let before = (1.0 - terrain_parallax_height(tm, layer, uv + prev_offset))
        - ray_depth + layer_depth;
    let denom = after - before;
    let weight = select(0.0, after / denom, abs(denom) > 1e-6);
    return mix(offset, prev_offset, clamp(weight, 0.0, 1.0));
}

/// World-XZ offset from steep parallax plus a POM refinement.
///
/// `view_ts` is the direction *toward the camera* in the surface's tangent
/// frame: xy along (tangent, bitangent), z along the normal.
fn terrain_parallax_offset(
    tm: TerrainMaterial,
    layer: u32,
    local_xz: vec2<f32>,
    tiling: f32,
    view_ts: vec3<f32>,
    tangent_xz: vec2<f32>,
    bitangent_xz: vec2<f32>,
    depth: f32,
    steps: f32,
) -> vec2<f32> {
    if steps < 4.0 {
        return vec2<f32>(0.0);
    }
    // Grazing angles need more steps and shallower ones fewer, because the ray
    // crosses more of the height field per unit of depth. Bevy interpolates the
    // count the same way, and clamps away from zero so a surface parallel to
    // the view does not divide by its own vanishing z.
    let steepness = max(abs(view_ts.z), 0.05);
    let layers = max(mix(steps, 1.0, steepness), 1.0);
    let layer_depth = 1.0 / layers;

    // How far to step per layer, in metres along the surface. The ray moves
    // *against* the view direction as it sinks into the surface.
    let step_ts = -view_ts.xy / steepness * depth * layer_depth;
    let step_xz = tangent_xz * step_ts.x + bitangent_xz * step_ts.y;

    // Into the layer's texture coordinate and back out again. Scaling both the
    // start point and the step by the same non-zero scalar and dividing the
    // result by it is exact, and it is what lets the march be written once for
    // a path that thinks in metres and one that thinks in texture space.
    //
    // A zero tiling means the whole layer is one texel, which has no relief to
    // displace, so the offset is zero rather than an infinity.
    if abs(tiling) <= 1e-6 {
        return vec2<f32>(0.0);
    }
    return terrain_parallax_march(
        tm, layer, local_xz * tiling, step_xz * tiling, layers) / tiling;
}

/// Parallax offset inside one projection plane's own texture coordinate.
///
/// `view_pl` is the direction toward the camera resolved into that plane's
/// frame: xy along the two world axes the coordinate is built from, in that
/// order, and z along the plane's outward normal. `depth` is in the same
/// coordinate as `uv`, which is why the caller multiplies the layer's metre
/// depth by the tiling before passing it.
///
/// No tangent frame appears anywhere in here, and that is the point — the
/// coordinate *is* the parametrisation the texture is laid out in, so a step in
/// it needs no basis to be meaningful. It is the heightfield path, which has to
/// convert a tangent-space ray into world XZ first, that needs one.
fn terrain_projected_offset(
    tm: TerrainMaterial,
    layer: u32,
    uv: vec2<f32>,
    view_pl: vec3<f32>,
    depth: f32,
    steps: f32,
) -> vec2<f32> {
    if steps < 4.0 {
        return vec2<f32>(0.0);
    }
    let steepness = max(abs(view_pl.z), 0.05);
    let layers = max(mix(steps, 1.0, steepness), 1.0);
    let step_uv = -view_pl.xy / steepness * depth * (1.0 / layers);
    return terrain_parallax_march(tm, layer, uv, step_uv, layers);
}

/// How much of the relief shadows itself from the sun (Phase 25H).
///
/// Ported from O3DE's `AdvancedParallaxMapping`: from the point the view ray
/// actually hit, march *toward the light* through the same height field; every
/// step that ends up under the surface darkens the result. This is what turns a
/// displaced texture into lit relief — without it a pebble moves correctly and
/// is still lit as though nothing were beside it.
///
/// Returns 1 for fully lit.
fn terrain_parallax_shadow(
    tm: TerrainMaterial,
    layer: u32,
    local_xz: vec2<f32>,
    tiling: f32,
    light_ts: vec3<f32>,
    tangent_xz: vec2<f32>,
    bitangent_xz: vec2<f32>,
    depth: f32,
    steps: u32,
) -> f32 {
    if steps == 0u || light_ts.z <= 0.05 {
        // The sun is at or below the surface's horizon; the geometric N·L term
        // has already taken this pixel to black and a relief shadow on top of
        // it would only be a second, wrong darkening.
        return 1.0;
    }
    // Into the layer's texture coordinate once, as `terrain_parallax_offset`
    // does: the height fetch is shared with the view march and takes a texture
    // coordinate, not metres.
    let uv = local_xz * tiling;
    let start = 1.0 - terrain_parallax_height(tm, layer, uv);
    let step = 1.0 / f32(steps);
    let step_ts = light_ts.xy / light_ts.z * depth * step;
    let step_uv = (tangent_xz * step_ts.x + bitangent_xz * step_ts.y) * tiling;

    var occlusion = 0.0;
    var offset = vec2<f32>(0.0);
    var ray = start;
    for (var i = 0u; i < steps; i = i + 1u) {
        offset += step_uv;
        ray -= step;
        let h = 1.0 - terrain_parallax_height(tm, layer, uv + offset);
        // Weighted by how far along the march it is, as O3DE does: an occluder
        // right beside the point casts a harder shadow than one at the far end
        // of the trace, which is what keeps the contact edge sharp.
        occlusion = max(occlusion, (ray - h) * (1.0 - f32(i) * step));
    }
    return saturate(1.0 - occlusion);
}

/// Layer texture reads the current fragment issued, for debug mode 12.
///
/// A private rather than a field on `Surface`: it exists only to be looked at,
/// and threading it through the shared surface struct would put a debug counter
/// in the path of every mesh in the scene.
/// The fragment's pixel coordinate, for anything that needs screen-space noise.
///
/// Set once at the top of `fs_main`. Stochastic filtering has to index its
/// dither by *screen position*, not by surface position: neighbouring pixels
/// must disagree so the result resolves, and the same pixel must agree with
/// itself frame to frame while the camera is still, or the surface flickers.
var<private> terrain_screen_pixel: vec2<u32> = vec2<u32>(0u);
var<private> terrain_taps: u32 = 0u;
var<private> terrain_discarded: f32 = 0.0;
var<private> terrain_selected_rgb: vec3<f32> = vec3<f32>(0.0);
var<private> terrain_weight_rgb: vec3<f32> = vec3<f32>(0.0);
var<private> terrain_wetness_factor: f32 = 0.0;
var<private> terrain_cliff_blend_dbg: f32 = 0.0;
var<private> terrain_dominant_albedo: vec3<f32> = vec3<f32>(0.0);
var<private> terrain_wet_f0: f32 = 0.0;
/// Phase DF: which detail ring the clipmap path picked (debug mode 33).
var<private> terrain_clipmap_ring: f32 = 0.0;
/// Which stage of the clipmap stack actually produced this pixel (debug 34).
///
/// 0 = a detail ring, 1 = a macro ring, 2 = the flat macro-map fallback,
/// 3 = the constant colour when even the macro map is missing, -1 = the
/// clipmap path did not run.
///
/// `terrain_clipmap_ring` cannot answer this. It reports the outermost detail
/// ring as 7/7 = 1.0 and "no ring at all" as 1.0 as well, so debug 33 draws the
/// two in the same white. The band artifact is a question about *which stage*,
/// and for three investigations the only instrument available could not see it.
var<private> terrain_clipmap_source: f32 = -1.0;
/// MORROWIND-AD clipmap-generation-only bindings: albedo atlas, surface atlas,
/// page table, physical atlas edge. Normal shading leaves the sentinel intact.
var<private> terrain_virtual_texture: vec4<i32> = vec4<i32>(-1, -1, -1, 0);

/// Phase 25H: the relief self-shadow term, read by the shading pass.
var<private> terrain_parallax_shadow_factor: f32 = 1.0;

/// Pi, local to this module.
///
/// `brdf.wgsl` also declares `PI`, and this file is composed into two roots
/// with different dependency sets: `shading.wgsl` pulls in `brdf.wgsl` and
/// `clipmap_gen.wgsl` does not. Borrowing the name compiles in one root and
/// fails to parse in the other, which is exactly what the validation test
/// caught. A distinct name works in both and collides in neither.
const TERRAIN_PI: f32 = 3.14159265359;

/// Phase TSUSHIMA-C: the landscape-scale bent normal, world space.
///
/// Written by `terrain_sky_visibility` and read once, in the shading pass's
/// terrain branch. Defaults to zero, which the reader treats as "not written"
/// — a valid bent direction always has a positive Y.
var<private> terrain_sky_bent: vec3<f32> = vec3<f32>(0.0);

// ── Baked terrain visibility (Phase TSUSHIMA-B and TSUSHIMA-C) ───────────────
//
// Both functions read maps baked by `terrain::horizon` and are called from
// `shading.wgsl`'s terrain branch rather than from `evaluate_terrain_material`.
// That placement is deliberate: the quantities are properties of the
// *heightfield*, not of the material, so putting them here would mean writing
// them twice — once in the live path and once in `evaluate_clipmap_material` —
// and the clipmap path would inevitably drift.
//
// Neither is behind a pipeline override, unlike hex and POM. The reason is
// cost shape: those gate a multi-step march, where leaving the code resident
// costs occupancy on every terrain pixel whether or not the march runs. These
// are two texture fetches and a compare. The CPU unbinds the maps (`-1`) when
// the feature is off, which is the same sentinel `macro_map` has always used,
// and the branch is coherent across the whole draw because every pixel of a
// terrain reads the same material.

/// Terrain self-shadowing from the baked horizon map (TSUSHIMA-B).
///
/// Two fetches and a compare, at **any** distance. `SHADOW_DISTANCE` is a
/// compile-time 100 m and the cascades stop there; this does not stop at all,
/// which is the entire point — a landscape's structure at range is almost
/// entirely long shadow, and without it hills read as shapes with paint on
/// them.
///
/// `light_dir` points *toward* the sun, matching `light.direction`.
fn terrain_horizon_shadow(
    tm: TerrainMaterial,
    splat_uv: vec2<f32>,
    light_dir: vec3<f32>,
    sun_angular_radius: f32,
) -> f32 {
    if tm.horizon_map_a < 0 || tm.horizon_map_b < 0 {
        return 1.0;
    }
    let hxz = light_dir.xz;
    let horiz = length(hxz);
    if horiz < 1e-4 {
        return 1.0;  // sun at the zenith: nothing can occlude it
    }
    let sun_elev = atan2(light_dir.y, horiz);
    if sun_elev <= 0.0 {
        // Below the horizon. The sun's own illuminance has already gone to
        // zero through `sun::transmittance`, so returning 0 here is agreeing
        // with a decision made on the CPU rather than making a new one.
        return 0.0;
    }

    // Bearing in [0, 8), matching `horizon::DIRS`: measured from +X toward +Z.
    //
    // Both bracketing azimuths are sampled and interpolated. Taking only the
    // nearest is what makes the shadow edge snap between compass bearings as
    // the sun turns, and it is the most-reported artifact of this technique.
    let bearing = atan2(hxz.y, hxz.x) * (4.0 / TERRAIN_PI) + 8.0;
    let b0 = i32(floor(bearing)) & 7;
    let b1 = (b0 + 1) & 7;
    let f = fract(bearing);

    // Bindless *indices* cross the function boundary, never the textures:
    // pulling a texture out of a binding array and passing it segfaults naga's
    // SPIR-V backend during pipeline creation, with no diagnostic.
    let lo = textureSampleLevel(textures[tm.horizon_map_a], default_sampler, splat_uv, 0.0);
    let hi = textureSampleLevel(textures[tm.horizon_map_b], default_sampler, splat_uv, 0.0);
    let packed = array<f32, 8>(lo.r, lo.g, lo.b, lo.a, hi.r, hi.g, hi.b, hi.a);
    let angle = mix(packed[b0], packed[b1], f) * (TERRAIN_PI * 0.5);

    // Softened by the sun's angular radius rather than by a magic constant.
    // A 0.53-degree disc has a real penumbra, `light.sun_angular_radius`
    // already carries it, and it is the same value `evaluate_brdf_area` widens
    // the specular lobe by — so the two agree by construction rather than by
    // being tuned to match.
    let softness = max(sun_angular_radius, 0.002);
    return smoothstep(angle - softness, angle + softness, sun_elev);
}

/// What the baked relief chain says about this pixel (TSUSHIMA-E).
///
/// `.xyz` is the filtered heightfield normal in world space, `.w` is the
/// length of the unnormalised mean that produced it — Toksvig's measure of how
/// much the normals it averaged disagreed. A `.w` of 1 means they agreed and
/// the surface really is that smooth; lower means the level threw relief away,
/// and `widen_roughness_toksvig` puts it back as roughness.
///
/// Sampled with explicit gradients: the terrain path uses `textureSampleGrad`
/// throughout precisely so it can sample inside non-uniform control flow, and
/// this is called from the same place.
fn terrain_relief_normal(
    tm: TerrainMaterial,
    splat_uv: vec2<f32>,
    splat_ddx: vec2<f32>,
    splat_ddy: vec2<f32>,
) -> vec4<f32> {
    if tm.relief_map < 0 {
        return vec4<f32>(0.0, 1.0, 0.0, 1.0);
    }
    let s = textureSampleGrad(
        textures[tm.relief_map], default_sampler, splat_uv, splat_ddx, splat_ddy);
    let xz = s.rg * 2.0 - 1.0;
    // Y is reconstructed rather than stored. On a heightfield the normal's Y
    // is always positive, so the sign is never in question and the channel is
    // free to carry the length instead — which is the channel that matters.
    let y = sqrt(max(1.0 - dot(xz, xz), 0.0));
    return vec4<f32>(xz.x, y, xz.y, s.b);
}

/// Widen roughness by the normal variance a mip level discarded.
///
/// From the filtered normal's length, the von Mises-Fisher concentration is
/// `k = l(3 - l²)/(1 - l²)` and the equivalent added roughness variance is
/// `1/(2k)`. Alpha adds in *variance* space, not in roughness space, which is
/// why this squares in and roots out twice.
///
/// The double root is not a typo and it is the easiest thing here to get
/// wrong. `D_GGX` takes **perceptual** roughness `r` and computes `a = r*r`,
/// `a2 = a*a` — so its `a2` is `r⁴`, the standard `alpha` is `r²`, and
/// `alpha²` is `r⁴`. Filtering happens in `alpha²`, so recovering `r` is
/// `pow(alpha2, 0.25)`. Getting it wrong is invisible in a still and obvious
/// in motion.
fn widen_roughness_toksvig(roughness: f32, len: f32) -> f32 {
    let l = clamp(len, 0.0, 0.9999);
    let l2 = l * l;
    let kappa = l * (3.0 - l2) / max(1.0 - l2, 1e-4);
    let variance = 1.0 / max(2.0 * kappa, 1e-4);
    let alpha = roughness * roughness;
    return sqrt(sqrt(clamp(alpha * alpha + 2.0 * variance, 0.0, 1.0)));
}

/// Baked sky visibility and the landscape-scale bent normal (TSUSHIMA-C).
///
/// Returns the cosine-weighted fraction of the sky this point can see, and
/// leaves the average unoccluded direction in `terrain_sky_bent`.
///
/// This is **not** GI. No bounce, no colour bleeding, no ReSTIR interaction.
/// It is "how much sky can this point see", a fixed geometric property of the
/// heightfield, and it is the reason valleys are darker than ridges in every
/// photograph of a landscape ever taken. Nothing in the renderer knew it:
/// GTAO is screen-space and radius-bounded, the per-layer AO is texture-scale,
/// and the SH probe volume is 4x4x4 over the whole view.
fn terrain_sky_visibility(tm: TerrainMaterial, splat_uv: vec2<f32>) -> f32 {
    if tm.skyvis_map < 0 {
        return 1.0;
    }
    let sv = textureSampleLevel(textures[tm.skyvis_map], default_sampler, splat_uv, 0.0);
    let bent = sv.rgb * 2.0 - 1.0;
    if dot(bent, bent) > 1e-4 {
        terrain_sky_bent = normalize(bent);
    }
    // Strength retreats toward fully-open rather than toward black, so 0 is
    // the exact identity and the slider has a meaningful zero.
    return mix(1.0, sv.a, clamp(tm.sky_visibility_strength, 0.0, 1.0));
}

/// The worst case a pixel can pay — four selected layers, two maps each, three
/// hex taps, plus a biplanar cliff (4 extra). Debug mode 12 scales by this.
const TERRAIN_MAX_TAPS: f32 = 36.0;

/// What the terrain material contributes to the shared `Surface`.
struct TerrainSurface {
    albedo: vec3<f32>,
    /// Phase 25H: how much the relief shadows itself from the sun. 1 is lit.
    /// Applied to the direct term rather than to `occlusion`, which is an
    /// indirect quantity — mixing them would darken the sky's contribution with
    /// a shadow the sun casts.
    parallax_shadow: f32,
    /// Layer texture reads this pixel actually issued (Phase 25D). Carried for
    /// debug mode 12 and for nothing else — it is what makes "detail cost
    /// scales with screen area" a measurement rather than a claim.
    taps: u32,
    /// Splat weight dropped by strongest-four (XV-D debug mode 18).
    discarded: f32,
    /// First three selected layer indices / 15 (debug mode 19).
    selected_rgb: vec3<f32>,
    /// Raw strongest-four weights of the first three (debug mode 20).
    weight_rgb: vec3<f32>,
    roughness: f32,
    normal: vec3<f32>,
    /// Phase 25K: real per-material ambient occlusion, packed alongside the
    /// normal. Terrain has never had this — it hardcoded a fully-open 1.0.
    occlusion: f32,
}

struct TerrainLayerSample {
    albedo: vec3<f32>,
    // Phase 25K: a real displacement map from the packed albedo's alpha, where
    // it used to be procedural noise. This is what `terrain_append_height`
    // consumes, and what makes gravel settle into rock rather than cross-fade
    // across it.
    height: f32,
    normal_ts: vec3<f32>,
    roughness: f32,
    occlusion: f32,
}

const TERRAIN_VT_PAGE_SIZE: u32 = 128u;

fn terrain_vt_table_entry(mip: u32, page: vec2<u32>, source_size: u32) -> u32 {
    var offset = 0u;
    for (var level = 0u; level < mip; level = level + 1u) {
        let pages = max(1u, (max(1u, source_size >> level) + TERRAIN_VT_PAGE_SIZE - 1u) / TERRAIN_VT_PAGE_SIZE);
        offset += pages * pages;
    }
    let pages = max(1u, (max(1u, source_size >> mip) + TERRAIN_VT_PAGE_SIZE - 1u) / TERRAIN_VT_PAGE_SIZE);
    return offset + page.y * pages + page.x;
}

/// Resolve a logical source sample through the bounded physical atlas. A
/// missing fine page walks to a resident ancestor; an entirely cold cache uses
/// the already-computed mean layer material.
fn terrain_sample_virtual(
    tm: TerrainMaterial,
    layer: u32,
    uv: vec2<f32>,
    ddx_uv: vec2<f32>,
    ddy_uv: vec2<f32>,
) -> TerrainLayerSample {
    var out: TerrainLayerSample;
    let source_size = select(1024u, 2048u, layer < 16u);
    let max_mip = u32(log2(f32(source_size)));
    let footprint = max(length(ddx_uv), length(ddy_uv)) * f32(source_size);
    var mip = min(u32(max(floor(log2(max(footprint, 1.0))), 0.0)), max_mip);
    let source_uv = fract(uv);
    loop {
        let mip_size = max(1u, source_size >> mip);
        let pages = max(1u, (mip_size + TERRAIN_VT_PAGE_SIZE - 1u) / TERRAIN_VT_PAGE_SIZE);
        let texel = min(vec2<u32>(source_uv * f32(mip_size)), vec2<u32>(mip_size - 1u));
        let page = min(texel / TERRAIN_VT_PAGE_SIZE, vec2<u32>(pages - 1u));
        let entry = terrain_vt_table_entry(mip, page, source_size);
        let mapped = textureLoad(
            textures[terrain_virtual_texture.z],
            vec2<i32>(i32(entry), i32(layer)),
            0,
        );
        if mapped.b > 0.5 {
            let slot = vec2<f32>(round(mapped.rg * 255.0));
            let local = vec2<f32>(texel - page * TERRAIN_VT_PAGE_SIZE) + 0.5;
            // The paired physical atlases are exactly 64 MiB: 64x32 BC7
            // pages. `w` carries the width; the fixed 2:1 shape avoids growing
            // a square allocation past the authored budget.
            let atlas_extent = vec2<f32>(
                f32(terrain_virtual_texture.w),
                f32(terrain_virtual_texture.w) * 0.5,
            );
            let atlas_uv = (slot * f32(TERRAIN_VT_PAGE_SIZE) + local) / atlas_extent;
            let a = textureSampleLevel(
                textures[terrain_virtual_texture.x], default_sampler, atlas_uv, 0.0);
            let surf = textureSampleLevel(
                textures[terrain_virtual_texture.y], default_sampler, atlas_uv, 0.0);
            out.albedo = a.rgb;
            out.height = a.a;
            out.roughness = surf.b;
            out.occlusion = surf.a;
            let nxy = surf.rg * 2.0 - 1.0;
            out.normal_ts = vec3<f32>(nxy, sqrt(max(1.0 - dot(nxy, nxy), 0.0)));
            return out;
        }
        if mip >= max_mip {
            break;
        }
        mip += 1u;
    }
    out.albedo = max(tm.layer_albedo[layer].rgb, vec3<f32>(0.02));
    out.height = 0.5;
    out.normal_ts = vec3<f32>(0.0, 0.0, 1.0);
    out.roughness = 0.8;
    out.occlusion = 1.0;
    return out;
}

//!if DREAMS_STF
/// DREAMS-B stochastic mip selection. Trilinear filtering evaluates both
/// neighbouring mips for every lookup; this chooses one with the same expected
/// value and lets TAA integrate the noise. The shared Slang-cooked rank atlas
/// supplies the decision, so terrain does not grow a private hash function.
fn terrain_stochastic_sample(
    map: i32,
    uv: vec2<f32>,
    ddx: vec2<f32>,
    ddy: vec2<f32>,
    layer: u32,
) -> vec4<f32> {
    // The bank resolution is **not** a constant, and assuming it was is the
    // defect this line replaced. `choose_runtime_resolutions` loads hero
    // layers 0-15 at 2048 and extra layers 16-31 at 1024, and drops the hero
    // set to 1024 only when the BC7 budget is exceeded. On the shipped maps it
    // logs `0-15 at 2048, 16-31 at 1024`.
    //
    // A hardcoded 1024 therefore halves the footprint of every hero layer,
    // which is exactly **one mip level too sharp**. Trilinear would have
    // filtered it; a single stochastic tap does not, so the terrain
    // under-filters and shimmers, worst at distance where the true LOD is
    // highest. Asking the texture is both correct and free.
    let size = vec2<f32>(textureDimensions(textures[map], 0));
    let rho = max(length(ddx * size), length(ddy * size));
    let lod = max(log2(max(rho, 1.0)), 0.0);
    let lower = floor(lod);
    // Indexed by **screen** position, not by `uv`.
    //
    // Indexing the dither by texture coordinate was the shimmer. TAA jitters
    // the sample position by a fraction of a pixel every frame, which moves
    // `uv`, which moves `floor(uv * 64.0)` across a tile boundary, which flips
    // the decision -- and a flipped decision here is a whole mip level. A
    // stationary camera over static terrain therefore changed 1.99% of the
    // frame every frame, against 0.44% with stochastic filtering off.
    //
    // It also gave the technique nothing to resolve against: one 64x64 tile
    // spread over texture space means every pixel inside a tile shares a
    // decision, so there is no high-frequency detail for TAA's neighbourhood to
    // average. Screen indexing makes adjacent pixels disagree, which is the
    // whole premise of filtering stochastically.
    //
    // The per-layer shift stays. It decorrelates the two to four layers blended
    // at one pixel, and the layer index is a property of the surface, so it
    // costs no temporal stability.
    let shifted = terrain_screen_pixel + vec2<u32>(layer * 17u, layer * 29u);
    let index = (shifted.y & 63u) * 64u + (shifted.x & 63u);
    let decision = f32(grain_words[index / 4u][index & 3u] & 255u) / 255.0;
    let chosen = lower + select(0.0, 1.0, decision < fract(lod));
    return textureSampleLevel(textures[map], default_sampler, uv, chosen);
}
//!endif

/// Sample one layer at `uv`, with `ddx`/`ddy` its screen-space derivatives.
///
/// Phase 25F: albedo and normal go through the hex-tiled path, roughness does
/// not. That is a deliberate cut rather than an oversight — repetition is
/// visible in colour and in the way light catches surface detail, and barely at
/// all in how rough a surface is, so the third sample set buys the least per
/// tap. Three taps per map is the whole cost of the technique.
fn terrain_sample_layer(
    tm: TerrainMaterial,
    layer: u32,
    uv: vec2<f32>,
    ddx: vec2<f32>,
    ddy: vec2<f32>,
    hex: bool,
) -> TerrainLayerSample {
    if terrain_virtual_texture.w > 0 {
        return terrain_sample_virtual(tm, layer, uv, ddx, ddy);
    }
    var s: TerrainLayerSample;
    let albedo_map = tm.albedo_maps[layer / 4u][layer % 4u];
    let surface_map = tm.surface_maps[layer / 4u][layer % 4u];

    // Hero-bank mode deliberately leaves layers 16..31 at -1. Their splat
    // groups are also unbound, but guard the final sampling boundary as well:
    // a stale/corrupt splat texel must never turn -1 into an out-of-bounds
    // bindless texture access (the source of intermittent white terrain).
    if albedo_map < 0 || surface_map < 0 {
        s.albedo = max(tm.layer_albedo[layer].rgb, vec3<f32>(0.02));
        s.height = 0.5;
        s.normal_ts = vec3<f32>(0.0, 0.0, 1.0);
        s.roughness = 0.8;
        s.occlusion = 1.0;
        return s;
    }

    var a: vec4<f32>;
    var surf: vec4<f32>;
    if hex {
        // One simplex grid and three taps for **both** maps. They depend only
        // on the UV and its derivatives, so building them per map cost a layer
        // two grids and six taps where three do.
        let h = hex_taps(uv, ddx, ddy);
        a = hex_sample_with(albedo_map, h);
        let hs = hex_sample_packed_surface_with(surface_map, h);
        s.albedo = a.rgb;
        s.height = a.a;
        s.roughness = hs.roughness;
        s.occlusion = hs.occlusion;
        s.normal_ts = hs.normal_ts;
        return s;
    } else {
//!if DREAMS_STF
        if (cluster_params.shading_mode & 32u) != 0u {
            a = terrain_stochastic_sample(albedo_map, uv, ddx, ddy, layer);
            surf = terrain_stochastic_sample(surface_map, uv, ddx, ddy, layer + 32u);
        } else {
            a = textureSampleGrad(textures[albedo_map], default_sampler, uv, ddx, ddy);
            surf = textureSampleGrad(textures[surface_map], default_sampler, uv, ddx, ddy);
        }
//!else
        a = textureSampleGrad(textures[albedo_map], default_sampler, uv, ddx, ddy);
        surf = textureSampleGrad(textures[surface_map], default_sampler, uv, ddx, ddy);
//!endif
    }

    s.albedo = a.rgb;
    s.height = a.a;
    s.roughness = surf.b;
    s.occlusion = surf.a;

    // Phase 25K: only XY are stored; Z is reconstructed. Exact for a unit
    // normal, and it is what BC5 compression would force anyway — so the
    // packing costs nothing and saves a channel.
    let nxy = surf.rg * 2.0 - 1.0;
    s.normal_ts = vec3<f32>(nxy, sqrt(max(1.0 - dot(nxy, nxy), 0.0)));
    return s;
}

/// Fold one layer's height into its weight (Phase 25E).
///
/// Ported from O3DE's `AppendHeightToWeight`. The clamp is the whole trick: a
/// layer's relief only counts in full once it has real coverage, so a 3% sliver
/// of gravel with a tall height map cannot out-rank the grass that is actually
/// painted here. Without it the height map behaves like a second splatmap
/// nobody authored.
fn terrain_append_height(
    tm: TerrainMaterial,
    layer: u32,
    weight: f32,
    height: f32,
) -> f32 {
    let h = height * terrain_height_scale(tm, layer);
    return weight + h * min(1.0, terrain_weight_clamp(tm, layer) * weight);
}

// ── The macro tier (Phase 25D) ───────────────────────────────────────────────
//
// Eight materials can describe a texel of ground but not a landscape: every
// patch of grass is the same patch of grass, and at distance the layers
// converge to their own mean and the terrain goes uniform. The macro map
// carries the frequencies no tiling texture reaches — hundreds of metres — and
// the detail composite is blended against it, which is O3DE's macro/detail
// split (`TerrainMacroHelpers.azsli`, `GetDetailColor`).
//
// Its texels are display-referred and centred on 0.5, so the blend happens in
// the same approximately-perceptual space Phase 25E already averages albedo in
// — between the `sqrt` and the squaring. A uniform 0.5 map is the identity for
// the overlay mode, which is what makes "no macro map" and "strength 0" agree.

const MACRO_MULTIPLY: u32 = 0u;
const MACRO_LERP: u32 = 1u;
const MACRO_LINEAR_LIGHT: u32 = 2u;
const MACRO_OVERLAY: u32 = 3u;

/// Ported from O3DE's `ApplyTextureBlend` (`BlendUtility.azsli`). `detail` and
/// `macro_c` are both perceptual-space values.
fn terrain_macro_blend(
    detail: vec3<f32>,
    macro_c: vec3<f32>,
    mode: u32,
    factor: f32,
) -> vec3<f32> {
    if mode == MACRO_MULTIPLY {
        return mix(detail, detail * macro_c * 2.0, factor);
    }
    if mode == MACRO_LERP {
        return mix(detail, macro_c, factor);
    }
    var blended = detail;
    if mode == MACRO_LINEAR_LIGHT {
        blended = clamp(detail + 2.0 * macro_c - 1.0, vec3(0.0), vec3(1.0));
    } else {
        // Overlay: screen where the detail is light, multiply where it is dark,
        // so the detail keeps its own structure and takes the macro's colour.
        let hi = 1.0 - (1.0 - 2.0 * (detail - 0.5)) * (1.0 - macro_c);
        let lo = 2.0 * detail * macro_c;
        blended = select(lo, hi, detail > vec3(0.5));
    }
    return mix(detail, blended, factor);
}

/// Sample the macro map at a terrain-global UV, returning colour and the
/// per-texel strength its alpha carries.
///
/// Falls back to the blend's identity — 0.5 at zero strength — when no macro
/// map is bound, so the branch below it needs no second path.
fn terrain_macro_sample(
    tm: TerrainMaterial,
    splat_uv: vec2<f32>,
    splat_ddx: vec2<f32>,
    splat_ddy: vec2<f32>,
) -> vec4<f32> {
    if tm.macro_map < 0 {
        return vec4<f32>(0.5, 0.5, 0.5, 0.0);
    }
    let m = textureSampleGrad(
        textures[tm.macro_map], default_sampler, splat_uv, splat_ddx, splat_ddy);
    return vec4<f32>(m.rgb, m.a * tm.macro_strength);
}

/// Low-frequency value variance at the scales between the tile and the terrain
/// (Phase TSUSHIMA-H).
///
/// Real ground varies at 1 m, 10 m, 100 m and 1 km. This material had variance
/// at the tile size and at the terrain size and at nothing in between, which is
/// what makes a hillside read as one printed sheet of gravel however good the
/// gravel is.
///
/// **Multiplied, not lerped**, so the octaves compose: a kilometre-wide band
/// and a ten-metre mottle should both be visible in the same square metre, and
/// a lerp lets the finer one erase the coarser one wherever it is strong.
///
/// Centred on 1.0, so a strength of 0 is the exact identity rather than a
/// half-strength grey — the same property the macro map's 0.5 fallback has, for
/// the same reason.
fn terrain_macro_octaves(world_xz: vec2<f32>, strength: vec3<f32>) -> f32 {
    // Cycles per metre: ~1 km, ~100 m, ~10 m. Fixed rather than uniform,
    // because these are the design — the three bands the material was missing —
    // and the strengths are what an author actually wants to reach for.
    let a = terrain_value_noise(world_xz * 0.001);
    let b = terrain_value_noise(world_xz * 0.010);
    let c = terrain_value_noise(world_xz * 0.100);
    return (1.0 + (a - 0.5) * strength.x)
         * (1.0 + (b - 0.5) * strength.y)
         * (1.0 + (c - 0.5) * strength.z);
}

/// How much of the per-pixel layer budget survives at `distance` metres.
///
/// 0 close up, 1 past `detail_fade_end`. This is Phase 25D's answer to the
/// clipmap's stated goal — detail cost scaling with screen area rather than
/// world area. A pixel a kilometre away covers metres of ground and averages
/// layers that are individually indistinguishable in it; paying eight layers
/// times two maps times three hex taps for that is 48 texture reads to compute
/// a colour a single layer would have given.
fn terrain_detail_fade(tm: TerrainMaterial, distance: f32) -> f32 {
    return clamp(
        (distance - tm.detail_fade_start) / max(tm.detail_fade_end - tm.detail_fade_start, 1.0),
        0.0,
        1.0,
    );
}

fn terrain_sample_projected_maps(
    tm: TerrainMaterial,
    layer: u32,
    uv: vec2<f32>,
    ddx: vec2<f32>,
    ddy: vec2<f32>,
) -> TerrainLayerSample {
    if terrain_virtual_texture.w > 0 {
        return terrain_sample_virtual(tm, layer, uv, ddx, ddy);
    }
    var s: TerrainLayerSample;
    let albedo_map = tm.albedo_maps[layer / 4u][layer % 4u];
    let surface_map = tm.surface_maps[layer / 4u][layer % 4u];
    if albedo_map < 0 || surface_map < 0 {
        s.albedo = max(tm.layer_albedo[layer].rgb, vec3<f32>(0.02));
        s.height = 0.5;
        s.normal_ts = vec3<f32>(0.0, 0.0, 1.0);
        s.roughness = 0.8;
        s.occlusion = 1.0;
        return s;
    }
    let a = textureSampleGrad(textures[albedo_map], default_sampler, uv, ddx, ddy);
    let surf = textureSampleGrad(textures[surface_map], default_sampler, uv, ddx, ddy);
    s.albedo = a.rgb;
    s.height = a.a;
    s.roughness = surf.b;
    s.occlusion = surf.a;
    let nxy = surf.rg * 2.0 - 1.0;
    s.normal_ts = vec3<f32>(nxy, sqrt(max(1.0 - dot(nxy, nxy), 0.0)));
    return s;
}

/// Full-PBR biplanar (default) or triplanar (debug) projection, with
/// world-space parallax (Phase TSUSHIMA-I).
///
/// # Why this needed its own march
///
/// `evaluate_terrain_material` disables the heightfield POM wherever
/// `cliff_blend >= 0.05`, and the reason was correct: that march is UV-space —
/// it walks the terrain's world-XZ parametrisation — and the cliff projection
/// is not. On a vertical face the XZ parametrisation is degenerate, so a march
/// along it either goes nowhere or smears. Godot makes the same exclusion.
///
/// The consequence was that the steepest ground in the scene, the ground a
/// player is most often standing right next to, was the one surface with no
/// depth at all: a photograph of a rock face on a flat triangle.
///
/// The fix is not to make the XZ march work. It is to march in the frame the
/// projection actually samples in. Each plane's texture coordinate is two world
/// axes scaled by the tiling, so a displacement in that coordinate has an exact
/// physical meaning and the ray direction resolves into it with two dot
/// products. `terrain_parallax_march` is then the same march the heightfield
/// uses, because it was written to take a coordinate rather than a metre.
fn terrain_projected_pbr(
    tm: TerrainMaterial,
    layer: u32,
    world_pos: vec3<f32>,
    n: vec3<f32>,
    tiling: f32,
    world_ddx: vec2<f32>,
    world_ddy: vec2<f32>,
) -> TerrainLayerSample {
    let p = world_pos * tiling;
    let dpdx = vec3<f32>(world_ddx.x, 0.0, world_ddx.y) * tiling;
    let dpdy = vec3<f32>(world_ddy.x, 0.0, world_ddy.y) * tiling;
    let k = max(tm.projection_sharpness, 1.0);
    var w = pow(abs(n), vec3(k));
    // Drop the weakest axis for biplanar; keep all three for the debug path.
    if tm.projection_mode == 0u {
        if w.x <= w.y && w.x <= w.z {
            w.x = 0.0;
        } else if w.y <= w.z {
            w.y = 0.0;
        } else {
            w.z = 0.0;
        }
    }
    w = w / max(w.x + w.y + w.z, 1e-4);

    // `world_pos` here is terrain-*local*: XZ has the origin subtracted, Y has
    // not. The camera has to make the same move or the ray points somewhere
    // else entirely.
    let camera_local = view.camera_pos - vec3<f32>(tm.terrain_origin.x, 0.0, tm.terrain_origin.y);
    let to_camera = camera_local - world_pos;
    // Step count from the same two things the heightfield march uses, computed
    // here rather than passed in: the two call sites — live terrain and the
    // clipmap's shading pass — would otherwise each carry a copy of the rule
    // and one of them would eventually stop matching.
    let pom_steps = select(
        0.0,
        f32(tm.parallax_steps) * (1.0 - terrain_detail_fade(tm, length(to_camera))),
        enable_pom && tm.parallax_steps >= 4u,
    );
    let pom_depth = terrain_parallax_depth(tm, layer) * tiling;
    let v = normalize(to_camera);
    // Marching a plane that contributes a few percent buys nothing and costs a
    // full march, so the gate is far above the 0.001 the sample gate uses.
    let pom_min_weight = 0.05;

    var out: TerrainLayerSample;
    out.albedo = vec3(0.0);
    out.height = 0.0;
    out.roughness = 0.0;
    out.occlusion = 0.0;
    out.normal_ts = vec3(0.0, 0.0, 1.0);
    var n_world = vec3(0.0);

    if w.x > 0.001 {
        var uv_x = p.zy;
        if pom_steps >= 4.0 && w.x > pom_min_weight && pom_depth > 0.0 {
            uv_x += terrain_projected_offset(
                tm, layer, uv_x, vec3<f32>(v.z, v.y, v.x * sign(n.x)), pom_depth, pom_steps);
        }
        let s = terrain_sample_projected_maps(
            tm, layer, uv_x, dpdx.zy, dpdy.zy);
        out.albedo += s.albedo * w.x;
        out.height += s.height * w.x;
        out.roughness += s.roughness * w.x;
        out.occlusion += s.occlusion * w.x;
        let t = vec3(0.0, 1.0, 0.0);
        let b = vec3(0.0, 0.0, 1.0) * sign(n.x);
        n_world += normalize(t * s.normal_ts.x + b * s.normal_ts.y + vec3(sign(n.x), 0.0, 0.0) * s.normal_ts.z) * w.x;
    }
    if w.y > 0.001 {
        var uv_y = p.xz;
        if pom_steps >= 4.0 && w.y > pom_min_weight && pom_depth > 0.0 {
            uv_y += terrain_projected_offset(
                tm, layer, uv_y, vec3<f32>(v.x, v.z, v.y * sign(n.y)), pom_depth, pom_steps);
        }
        let s = terrain_sample_projected_maps(
            tm, layer, uv_y, dpdx.xz, dpdy.xz);
        out.albedo += s.albedo * w.y;
        out.height += s.height * w.y;
        out.roughness += s.roughness * w.y;
        out.occlusion += s.occlusion * w.y;
        let t = vec3(1.0, 0.0, 0.0);
        let b = vec3(0.0, 0.0, 1.0) * sign(n.y);
        n_world += normalize(t * s.normal_ts.x + b * s.normal_ts.y + vec3(0.0, sign(n.y), 0.0) * s.normal_ts.z) * w.y;
    }
    if w.z > 0.001 {
        var uv_z = p.xy;
        if pom_steps >= 4.0 && w.z > pom_min_weight && pom_depth > 0.0 {
            uv_z += terrain_projected_offset(
                tm, layer, uv_z, vec3<f32>(v.x, v.y, v.z * sign(n.z)), pom_depth, pom_steps);
        }
        let s = terrain_sample_projected_maps(
            tm, layer, uv_z, dpdx.xy, dpdy.xy);
        out.albedo += s.albedo * w.z;
        out.height += s.height * w.z;
        out.roughness += s.roughness * w.z;
        out.occlusion += s.occlusion * w.z;
        let t = vec3(1.0, 0.0, 0.0);
        let b = vec3(0.0, 1.0, 0.0) * sign(n.z);
        n_world += normalize(t * s.normal_ts.x + b * s.normal_ts.y + vec3(0.0, 0.0, sign(n.z)) * s.normal_ts.z) * w.z;
    }
    n_world = normalize(n_world);
    // Store as a tangent-space perturbation against +Y so the caller can
    // compose it with the heightfield TBN via surface gradients.
    out.normal_ts = vec3(n_world.x, n_world.z, max(n_world.y, 0.2));
    return out;
}

struct TerrainGenerated {
    albedo: vec4<f32>,
    surface: vec4<f32>,
}

/// A tangent that remains finite even when the geometric normal is ±X.
/// Projecting a fixed X axis collapses to zero at exactly those normals, then
/// normalize() spreads NaNs through POM, normal mapping, and ultimately HDR.
fn terrain_stable_tangent(n: vec3<f32>) -> vec3<f32> {
    let reference = select(
        vec3<f32>(1.0, 0.0, 0.0),
        vec3<f32>(0.0, 0.0, 1.0),
        abs(n.x) > 0.9,
    );
    let projected = reference - n * dot(reference, n);
    return projected * inverseSqrt(max(dot(projected, projected), 1e-8));
}

fn terrain_fetch_splats(
    tm: TerrainMaterial,
    splat_uv: vec2<f32>,
    splat_ddx: vec2<f32>,
    splat_ddy: vec2<f32>,
) -> array<vec4<f32>, 8> {
    var splat_s = array<vec4<f32>, 8>();
    for (var g = 0u; g < terrain_splat_groups(); g = g + 1u) {
        let id = tm.splat_maps[g / 4u][g % 4u];
        if id >= 0 {
            splat_s[g] = textureSampleGrad(
                textures[id], default_sampler, splat_uv, splat_ddx, splat_ddy);
        }
    }
    return splat_s;
}

fn terrain_generate_texel(
    terrain_index: u32,
    world_xz: vec2<f32>,
    world_ddx: vec2<f32>,
    world_ddy: vec2<f32>,
    hex: bool,
) -> TerrainGenerated {
    let tm = terrain_materials[terrain_index];
    let splat_uv = (world_xz - tm.terrain_origin) * tm.inv_world_size;
    let splat_ddx = world_ddx * tm.inv_world_size;
    let splat_ddy = world_ddy * tm.inv_world_size;
    let local_xz = world_xz - tm.terrain_origin;
    var splat_s = terrain_fetch_splats(tm, splat_uv, splat_ddx, splat_ddy);
    var weight = terrain_unpack_splats_painted(splat_s, tm, local_xz);
    let selected = terrain_strongest_four(&weight);
    var kept = 0.0;
    for (var s = 0u; s < 4u; s = s + 1u) {
        kept += weight[selected[s]];
    }
    kept = max(kept, 0.0001);
    // Renormalise the four survivors and nothing else. The old form built a
    // second `array<f32, 32>`, wrote all 32 slots, and then only ever read the
    // four selected indices back out.
    var sel_w = array<f32, 4>(
        weight[selected[0]] / kept,
        weight[selected[1]] / kept,
        weight[selected[2]] / kept,
        weight[selected[3]] / kept,
    );
    let geo_normal = vec3<f32>(0.0, 1.0, 0.0);
    let tangent = vec3<f32>(1.0, 0.0, 0.0);
    let bitangent = vec3<f32>(0.0, 0.0, 1.0);
    let epsilon = LAYER_WEIGHT_EPSILON;

    var samples: array<TerrainLayerSample, 4>;
    var adjusted: array<f32, 4>;
    for (var s = 0u; s < 4u; s = s + 1u) {
        let i = selected[s];
        if sel_w[s] < epsilon {
            adjusted[s] = 0.0;
            continue;
        }
        let tiling = terrain_layer_tiling(tm, i);
        samples[s] = terrain_sample_layer(
            tm, i, local_xz * tiling, world_ddx * tiling, world_ddy * tiling, hex);
        if tm.height_blend != 0u {
            adjusted[s] = terrain_append_height(tm, i, sel_w[s], samples[s].height);
        } else {
            adjusted[s] = sel_w[s];
        }
    }
    var max_w = 0.0;
    var min_depth = -1e30;
    for (var s = 0u; s < 4u; s = s + 1u) {
        let i = selected[s];
        if sel_w[s] < epsilon {
            continue;
        }
        max_w = max(max_w, adjusted[s]);
        min_depth = max(min_depth, adjusted[s] - terrain_blend_width(tm, i));
    }
    var blend: array<f32, 4>;
    var blend_sum = 0.0;
    for (var s = 0u; s < 4u; s = s + 1u) {
        var b = 0.0;
        let i = selected[s];
        if sel_w[s] >= epsilon {
            let local_min = max(min_depth, max_w - terrain_blend_width(tm, i));
            b = max((adjusted[s] - local_min) / max(max_w - local_min, 1e-4), 0.0);
        }
        blend[s] = b;
        blend_sum += b;
    }
    blend_sum = max(blend_sum, 0.0001);
    var albedo = vec3<f32>(0.0);
    var n_ts = vec3<f32>(0.0, 0.0, 1.0);
    var roughness = 0.0;
    var occlusion = 0.0;
    var height = 0.0;
    var moisture = 0.0;
    for (var s = 0u; s < 4u; s = s + 1u) {
        let b = blend[s] / blend_sum;
        if b <= 0.0 {
            continue;
        }
        albedo += sqrt(samples[s].albedo) * b;
        n_ts += samples[s].normal_ts * b;
        roughness += samples[s].roughness * b;
        occlusion += samples[s].occlusion * b;
        height += samples[s].height * b;
        moisture += terrain_moisture(tm, selected[s]) * b;
    }
    let macro_c = terrain_macro_sample(tm, splat_uv, splat_ddx, splat_ddy);
    albedo = terrain_macro_blend(albedo, macro_c.rgb, tm.macro_mode, macro_c.a);
    // Same octaves, same world frame, same place in the chain as the live path.
    // A terrain shades through whichever of the two the distance picks, and a
    // clipmap ring that disagreed with live terrain about the tint would put a
    // visible ring on the ground at the handover.
    albedo = albedo * terrain_macro_octaves(local_xz, tm.macro_octave_strength.xyz);
    albedo = albedo * albedo;
    let wet = saturate(tm.wetness * moisture);
    albedo *= mix(1.0, tm.wetness_darken, wet);
    roughness = mix(roughness, roughness * tm.wetness_gloss, wet);
    n_ts = normalize(n_ts);
    var packed: TerrainGenerated;
    // Alpha is not sampled as height by the clipmap shading path. Preserve the
    // exact wet factor there instead so dielectric F0 matches live terrain.
    packed.albedo = vec4<f32>(albedo, wet);
    packed.surface = vec4<f32>(n_ts.xy * 0.5 + 0.5, roughness, occlusion);
    return packed;
}

fn evaluate_terrain_material(
    terrain_index: u32,
    world_pos: vec3<f32>,
    geo_normal: vec3<f32>,
    splat_uv: vec2<f32>,
    world_ddx: vec2<f32>,
    world_ddy: vec2<f32>,
) -> TerrainSurface {
    let tm = terrain_materials[terrain_index];
    let splat_ddx = world_ddx * tm.inv_world_size;
    let splat_ddy = world_ddy * tm.inv_world_size;
    // Hoisted above the unpack because TSUSHIMA-G's perturbation is indexed by
    // world position and runs inside it.
    let local_xz = world_pos.xz - tm.terrain_origin;
    var splat_s = terrain_fetch_splats(tm, splat_uv, splat_ddx, splat_ddy);
    var weight = terrain_unpack_splats_painted(splat_s, tm, local_xz);
    let selected = terrain_strongest_four(&weight);
    var kept = 0.0;
    for (var s = 0u; s < 4u; s = s + 1u) {
        kept += weight[selected[s]];
    }
    let discarded = 1.0 - kept;
    let selected_rgb = vec3<f32>(
        f32(selected[0]), f32(selected[1]), f32(selected[2])) / 31.0;
    let weight_rgb = vec3<f32>(
        weight[selected[0]], weight[selected[1]], weight[selected[2]]);
    kept = max(kept, 0.0001);
    // Renormalise the four survivors and nothing else — see the same change in
    // `terrain_generate_texel`. Everything below reads `weight` only at the
    // selected indices, so the 32-slot `gated` array and its 32-iteration
    // rewrite were pure scratch traffic on every terrain pixel.
    var sel_w = array<f32, 4>(
        weight[selected[0]] / kept,
        weight[selected[1]] / kept,
        weight[selected[2]] / kept,
        weight[selected[3]] / kept,
    );

    let view_distance = distance(world_pos, view.camera_pos);
    let fade = terrain_detail_fade(tm, view_distance);
    let epsilon = mix(LAYER_WEIGHT_EPSILON, FAR_LAYER_EPSILON, fade);
    // Hex / POM flags must stay uniform (`tm.hex_tiling`, `tm.parallax_steps`).
    // ANDing them with a per-pixel fade or cliff test makes the whole `if`
    // varying; DXC then flattens the march and the Details checkbox appears
    // to work while the samples still run. Aerial cut and the toggle both
    // zero those uniforms on the CPU. Do **not** reintroduce a close/far
    // sample-path mix — warps pay the union, and walking got slower.
    let hex = enable_hex && tm.hex_tiling != 0u;

    let tangent = terrain_stable_tangent(geo_normal);
    let bitangent = cross(geo_normal, tangent);

    let steepness = 1.0 - abs(geo_normal.y);
    let cliff_blend = smoothstep(0.45, 0.7, steepness);
    // Projected cliffs cannot POM — the march is UV-space and the projection
    // is world-space. Godot makes the same exclusion.
    let allow_pom = cliff_blend < 0.05;

    var parallax_shadow = 1.0;
    var march_xz = vec2<f32>(0.0);
    // Uniform kill first so "Parallax off" is a real skip, not a flattened
    // 1-step view march plus the 8-step shadow that `gpu_material` used to
    // leave running. Fade and cliffs only apply when the feature is on.
    if enable_pom && tm.parallax_steps >= 4u {
        let parallax_steps = f32(tm.parallax_steps) * (1.0 - fade);
        // Fewer than four remaining steps is a mip-0 march for relief the pixel
        // cannot resolve. Near ground keeps the full 24-step count.
        if allow_pom && parallax_steps >= 4.0 {
            var dominant = selected[0];
            var best = -1.0;
            for (var s = 0u; s < 4u; s = s + 1u) {
                if sel_w[s] > best {
                    best = sel_w[s];
                    dominant = selected[s];
                }
            }
            let depth = terrain_parallax_depth(tm, dominant);
            if depth > 0.0 {
                let tiling = terrain_layer_tiling(tm, dominant);
                let v = normalize(view.camera_pos - world_pos);
                let view_ts = vec3<f32>(dot(v, tangent), dot(v, bitangent), dot(v, geo_normal));
                march_xz = terrain_parallax_offset(
                    tm, dominant, local_xz, tiling, view_ts,
                    tangent.xz, bitangent.xz, depth, parallax_steps,
                );
                let l = normalize(light.direction);
                let light_ts = vec3<f32>(dot(l, tangent), dot(l, bitangent), dot(l, geo_normal));
                parallax_shadow = terrain_parallax_shadow(
                    tm, dominant, local_xz + march_xz, tiling, light_ts,
                    tangent.xz, bitangent.xz, depth, tm.parallax_shadow_steps,
                );
                parallax_shadow = mix(parallax_shadow, 1.0, fade);
            }
        }
    }
    let parallax_xz = local_xz + march_xz;

    var samples: array<TerrainLayerSample, 4>;
    var adjusted: array<f32, 4>;
    var taps = 0u;
    for (var s = 0u; s < 4u; s = s + 1u) {
        let i = selected[s];
        if sel_w[s] < epsilon {
            adjusted[s] = 0.0;
            continue;
        }
        let tiling = terrain_layer_tiling(tm, i);
        samples[s] = terrain_sample_layer(
            tm, i, parallax_xz * tiling, world_ddx * tiling, world_ddy * tiling, hex);
        taps += select(2u, 6u, hex);
        if tm.height_blend != 0u {
            adjusted[s] = terrain_append_height(tm, i, sel_w[s], samples[s].height);
        } else {
            adjusted[s] = sel_w[s];
        }
    }

    var max_w = 0.0;
    var min_depth = -1e30;
    for (var s = 0u; s < 4u; s = s + 1u) {
        let i = selected[s];
        if sel_w[s] < epsilon {
            continue;
        }
        max_w = max(max_w, adjusted[s]);
        min_depth = max(min_depth, adjusted[s] - terrain_blend_width(tm, i));
    }

    var blend: array<f32, 4>;
    var blend_sum = 0.0;
    for (var s = 0u; s < 4u; s = s + 1u) {
        var b = 0.0;
        let i = selected[s];
        if sel_w[s] >= epsilon {
            let local_min = max(min_depth, max_w - terrain_blend_width(tm, i));
            b = max((adjusted[s] - local_min) / max(max_w - local_min, 1e-4), 0.0);
        }
        blend[s] = b;
        blend_sum += b;
    }
    blend_sum = max(blend_sum, 0.0001);

    var albedo = vec3<f32>(0.0);
    var surfgrad = vec3<f32>(0.0);
    var roughness = 0.0;
    var occlusion = 0.0;
    for (var s = 0u; s < 4u; s = s + 1u) {
        let b = blend[s] / blend_sum;
        if b <= 0.0 {
            continue;
        }
        albedo += sqrt(samples[s].albedo) * b;
        surfgrad += ts_to_surfgrad(samples[s].normal_ts, tangent, bitangent) * b;
        roughness += samples[s].roughness * b;
        occlusion += samples[s].occlusion * b;
    }

    let macro_c = terrain_macro_sample(tm, splat_uv, splat_ddx, splat_ddy);
    albedo = terrain_macro_blend(albedo, macro_c.rgb, tm.macro_mode, macro_c.a);
    // TSUSHIMA-H, in the same approximately-perceptual space the macro blend
    // above already works in — between the `sqrt` and the squaring, not after.
    // Applied later it would be a second, independent gain on linear radiance
    // fighting the blend rather than composing with it, and the overlay and
    // linear-light modes are defined against a perceptual operand.
    albedo = albedo * terrain_macro_octaves(local_xz, tm.macro_octave_strength.xyz);
    albedo = albedo * albedo;

    if cliff_blend > 0.0 {
        let local_pos = world_pos - vec3(tm.terrain_origin.x, 0.0, tm.terrain_origin.y);
        let cliff = terrain_projected_pbr(
            tm,
            tm.cliff_layer,
            local_pos,
            geo_normal,
            terrain_layer_tiling(tm, tm.cliff_layer),
            world_ddx,
            world_ddy,
        );
        taps += select(4u, 6u, tm.projection_mode != 0u);
        albedo = mix(albedo, cliff.albedo, cliff_blend);
        roughness = mix(roughness, cliff.roughness, cliff_blend);
        occlusion = mix(occlusion, cliff.occlusion, cliff_blend);
        let cliff_grad = ts_to_surfgrad(normalize(cliff.normal_ts), tangent, bitangent);
        surfgrad = mix(surfgrad, cliff_grad, cliff_blend);
    }

    var moisture = 0.0;
    for (var s = 0u; s < 4u; s = s + 1u) {
        let b = blend[s] / blend_sum;
        if b > 0.0 {
            moisture += terrain_moisture(tm, selected[s]) * b;
        }
    }
    if cliff_blend > 0.0 {
        moisture = mix(moisture, terrain_moisture(tm, tm.cliff_layer), cliff_blend);
    }
    let wet = saturate(tm.wetness * moisture);
    albedo *= mix(1.0, tm.wetness_darken, wet);
    roughness = mix(roughness, roughness * tm.wetness_gloss, wet);
    terrain_wetness_factor = wet;
    terrain_cliff_blend_dbg = cliff_blend;
    var dom = vec3<f32>(0.0);
    for (var s = 0u; s < 4u; s = s + 1u) {
        if blend[s] / blend_sum > 0.0 {
            dom = samples[s].albedo;
            break;
        }
    }
    terrain_dominant_albedo = dom;
    terrain_wet_f0 = tm.wetness_f0 * wet;

    var out: TerrainSurface;
    out.albedo = albedo;
    out.taps = taps;
    out.discarded = discarded;
    out.selected_rgb = selected_rgb;
    out.weight_rgb = weight_rgb;
    out.parallax_shadow = parallax_shadow;
    out.roughness = max(roughness, 0.05);
    out.occlusion = occlusion;
    out.normal = resolve_surfgrad(geo_normal, surfgrad);
    return out;
}

/// The editor's brush cursor ring, drawn in-shader so it follows the terrain
/// contour exactly rather than floating as a flat decal.
///
/// Applied after lighting, which is why it takes the shaded colour: it is an
/// overlay, not a material property.
fn terrain_brush_overlay(
    terrain_index: u32,
    world_pos: vec3<f32>,
    shaded: vec3<f32>,
) -> vec3<f32> {
    let brush = terrain_materials[terrain_index].brush;
    if brush.w < 0.5 {
        return shaded;
    }
    let d = distance(world_pos.xz, brush.xy);
    let ring_width = max(brush.z * 0.04, 0.15);
    let ring = 1.0 - smoothstep(0.0, ring_width, abs(d - brush.z));
    let fill = (1.0 - smoothstep(brush.z * 0.85, brush.z, d)) * 0.08;
    var cursor_color = vec3<f32>(0.2, 1.0, 0.3);       // sculpt
    if brush.w > 2.5 {
        cursor_color = vec3<f32>(1.0, 0.65, 0.15);     // foliage
    } else if brush.w > 1.5 {
        cursor_color = vec3<f32>(0.25, 0.55, 1.0);     // layer paint
    }

    // Scaled by the scene's own brightness rather than a fixed multiplier: the
    // ring used to be `cursor_color * 2.0`, written when the sun was an
    // arbitrary 5. Against a 100 000 lux sun that is black, and against a
    // moonlit scene it is a searchlight.
    let scene_level = max(dot(shaded, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-4);
    return mix(shaded, cursor_color * scene_level * 4.0, clamp(ring * 0.8 + fill, 0.0, 1.0));
}
