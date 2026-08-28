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

// Mirrors `terrain::GpuTerrainMaterial` (2032 bytes, Phase DF). Every vec4 sits
// on a 16-byte offset; see the Rust struct for why that is load-bearing.
struct TerrainMaterial {
    layer_tiling: array<vec4<f32>, 8>,
    brush: vec4<f32>,
    albedo_maps: array<vec4<i32>, 8>,
    surface_maps: array<vec4<i32>, 8>,
    terrain_origin: vec2<f32>,
    inv_world_size: vec2<f32>,
    splat_maps: array<vec4<i32>, 2>,
    cliff_layer: u32,
    hex_tiling: u32,
    height_blend: u32,
    macro_map: i32,
    layer_height_scale: array<vec4<f32>, 8>,
    layer_blend_width: array<vec4<f32>, 8>,
    layer_weight_clamp: array<vec4<f32>, 8>,
    layer_parallax: array<vec4<f32>, 8>,
    macro_mode: u32,
    macro_strength: f32,
    detail_fade_start: f32,
    detail_fade_end: f32,
    layer_albedo: array<vec4<f32>, 32>,
    parallax_steps: u32,
    parallax_shadow_steps: u32,
    projection_sharpness: f32,
    projection_mode: u32,
    layer_moisture: array<vec4<f32>, 8>,
    wetness: f32,
    wetness_darken: f32,
    wetness_gloss: f32,
    wetness_f0: f32,
    // Phase DF: nested material clipmaps. 2032 bytes total.
    clipmap_enabled: u32,
    clipmap_rings: u32,
    clipmap_size: f32,
    clipmap_debug: u32,
    clipmap_albedo: array<vec4<i32>, 2>,
    clipmap_surface: array<vec4<i32>, 2>,
    clipmap_center: array<vec4<f32>, 4>,
    clipmap_origin: array<vec4<f32>, 4>,
    clipmap_tpm: array<vec4<f32>, 2>,
    clipmap_macro_albedo: vec4<i32>,
    clipmap_macro_normal: vec4<i32>,
    clipmap_macro_center: array<vec4<f32>, 2>,
    clipmap_macro_origin: array<vec4<f32>, 2>,
    clipmap_macro_tpm: vec4<f32>,
    clipmap_macro_rings: u32,
    clipmap_macro_size: f32,
    clipmap_detail_ready: u32,
    clipmap_macro_ready: u32,
}

/// Layers per terrain — must match `textures::TERRAIN_LAYER_COUNT`.
const TERRAIN_LAYERS: u32 = 32u;

// Pipeline overrides. Defaults keep the full path (clipmap generate, naga).
// The shading PSO sets these so unused hex/POM code is deleted — runtime
// uniforms do not change occupancy, which is why the Details checkboxes
// never moved Shading ms.
override enable_hex: bool = true;
override enable_pom: bool = true;
override terrain_scan: u32 = 32u;
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

fn terrain_splat_groups() -> u32 {
    return (min(terrain_scan, TERRAIN_LAYERS) + 3u) / 4u;
}

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

fn terrain_moisture(tm: TerrainMaterial, layer: u32) -> f32 {
    return tm.layer_moisture[layer / 4u][layer % 4u];
}

fn terrain_unpack_splats(s: array<vec4<f32>, 8>) -> array<f32, 32> {
    var w = array<f32, 32>();
    var total = 0.0;
    for (var g = 0u; g < terrain_splat_groups(); g = g + 1u) {
        let v = s[g];
        let base = g * 4u;
        w[base + 0u] = v.x;
        w[base + 1u] = v.y;
        w[base + 2u] = v.z;
        w[base + 3u] = v.w;
        total += v.x + v.y + v.z + v.w;
    }
    total = max(total, 0.0001);
    for (var i = 0u; i < terrain_scan; i = i + 1u) {
        w[i] = w[i] / total;
    }
    return w;
}

/// Deterministic strongest-four. Lower index wins ties.
///
/// One pass with the running top four held in scalars, not four passes over a
/// dynamically indexed `array<bool, 32>` companion. The old shape cost 4×32
/// iterations, a 128-byte by-value copy of `weight` at the call, and a second
/// 32-entry array that no GPU keeps in registers — every terrain pixel paid
/// scratch-memory traffic for a selection sort of at most four winners.
///
/// The tie break is unchanged: comparisons are strict, so an equal weight
/// arriving at a higher index never displaces the one already held.
fn terrain_strongest_four(weight: ptr<function, array<f32, 32>>) -> array<u32, 4> {
    var b0 = -1.0;
    var b1 = -1.0;
    var b2 = -1.0;
    var b3 = -1.0;
    var i0 = 0u;
    var i1 = 0u;
    var i2 = 0u;
    var i3 = 0u;
    for (var i = 0u; i < terrain_scan; i = i + 1u) {
        let w = (*weight)[i];
        if w > b0 {
            b3 = b2; i3 = i2;
            b2 = b1; i2 = i1;
            b1 = b0; i1 = i0;
            b0 = w;  i0 = i;
        } else if w > b1 {
            b3 = b2; i3 = i2;
            b2 = b1; i2 = i1;
            b1 = w;  i1 = i;
        } else if w > b2 {
            b3 = b2; i3 = i2;
            b2 = w;  i2 = i;
        } else if w > b3 {
            b3 = w;  i3 = i;
        }
    }
    return array<u32, 4>(i0, i1, i2, i3);
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

/// One height sample of `layer` at a world-XZ offset from `local_xz`.
fn terrain_parallax_height(
    tm: TerrainMaterial,
    layer: u32,
    local_xz: vec2<f32>,
    tiling: f32,
) -> f32 {
    let map = tm.albedo_maps[layer / 4u][layer % 4u];
    if map < 0 {
        return 0.5;
    }
    // Level 0 explicitly. Inside a march the derivatives are meaningless — the
    // taps walk along a ray, not across the screen — and asking for them would
    // both pick a wrong mip and force the loop to unroll.
    return textureSampleLevel(textures[map], default_sampler, local_xz * tiling, 0.0).a;
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

    var offset = vec2<f32>(0.0);
    var ray_depth = 0.0;
    // The height map is 1 at the peak; the ray starts at the peak and descends.
    var surface = 1.0 - terrain_parallax_height(tm, layer, local_xz, tiling);

    var i = 0.0;
    loop {
        if surface <= ray_depth || i >= layers {
            break;
        }
        offset += step_xz;
        ray_depth += layer_depth;
        surface = 1.0 - terrain_parallax_height(tm, layer, local_xz + offset, tiling);
        i = i + 1.0;
    }

    // POM refinement: one extra lookup, interpolating between the step that
    // crossed the surface and the one before it. Relief mapping's binary search
    // is more exact and costs a lookup per bisection; at ground-detail depths
    // the difference is below a pixel.
    let prev_offset = offset - step_xz;
    let after = surface - ray_depth;
    let before = (1.0 - terrain_parallax_height(tm, layer, local_xz + prev_offset, tiling))
        - ray_depth + layer_depth;
    let denom = after - before;
    let weight = select(0.0, after / denom, abs(denom) > 1e-6);
    return mix(offset, prev_offset, clamp(weight, 0.0, 1.0));
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
    let start = 1.0 - terrain_parallax_height(tm, layer, local_xz, tiling);
    let step = 1.0 / f32(steps);
    let step_ts = light_ts.xy / light_ts.z * depth * step;
    let step_xz = tangent_xz * step_ts.x + bitangent_xz * step_ts.y;

    var occlusion = 0.0;
    var offset = vec2<f32>(0.0);
    var ray = start;
    for (var i = 0u; i < steps; i = i + 1u) {
        offset += step_xz;
        ray -= step;
        let h = 1.0 - terrain_parallax_height(tm, layer, local_xz + offset, tiling);
        // Weighted by how far along the march it is, as O3DE does: an occluder
        // right beside the point casts a harder shadow than one at the far end
        // of the trace, which is what keeps the contact edge sharp.
        occlusion = max(occlusion, (ray - h) * (1.0 - f32(i) * step));
    }
    return saturate(1.0 - occlusion);
}

@group(0) @binding(11) var<storage, read> terrain_materials: array<TerrainMaterial>;

/// Layer texture reads the current fragment issued, for debug mode 12.
///
/// A private rather than a field on `Surface`: it exists only to be looked at,
/// and threading it through the shared surface struct would put a debug counter
/// in the path of every mesh in the scene.
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
/// MORROWIND-AD clipmap-generation-only bindings: albedo atlas, surface atlas,
/// page table, physical atlas edge. Normal shading leaves the sentinel intact.
var<private> terrain_virtual_texture: vec4<i32> = vec4<i32>(-1, -1, -1, 0);

/// Phase 25H: the relief self-shadow term, read by the shading pass.
var<private> terrain_parallax_shadow_factor: f32 = 1.0;

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
        a = textureSampleGrad(textures[albedo_map], default_sampler, uv, ddx, ddy);
        surf = textureSampleGrad(textures[surface_map], default_sampler, uv, ddx, ddy);
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

/// Full-PBR biplanar (default) or triplanar (debug) projection.
/// Height is used for blending only — no POM on this path.
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

    var out: TerrainLayerSample;
    out.albedo = vec3(0.0);
    out.height = 0.0;
    out.roughness = 0.0;
    out.occlusion = 0.0;
    out.normal_ts = vec3(0.0, 0.0, 1.0);
    var n_world = vec3(0.0);

    if w.x > 0.001 {
        let s = terrain_sample_projected_maps(
            tm, layer, p.zy, dpdx.zy, dpdy.zy);
        out.albedo += s.albedo * w.x;
        out.height += s.height * w.x;
        out.roughness += s.roughness * w.x;
        out.occlusion += s.occlusion * w.x;
        let t = vec3(0.0, 1.0, 0.0);
        let b = vec3(0.0, 0.0, 1.0) * sign(n.x);
        n_world += normalize(t * s.normal_ts.x + b * s.normal_ts.y + vec3(sign(n.x), 0.0, 0.0) * s.normal_ts.z) * w.x;
    }
    if w.y > 0.001 {
        let s = terrain_sample_projected_maps(
            tm, layer, p.xz, dpdx.xz, dpdy.xz);
        out.albedo += s.albedo * w.y;
        out.height += s.height * w.y;
        out.roughness += s.roughness * w.y;
        out.occlusion += s.occlusion * w.y;
        let t = vec3(1.0, 0.0, 0.0);
        let b = vec3(0.0, 0.0, 1.0) * sign(n.y);
        n_world += normalize(t * s.normal_ts.x + b * s.normal_ts.y + vec3(0.0, sign(n.y), 0.0) * s.normal_ts.z) * w.y;
    }
    if w.z > 0.001 {
        let s = terrain_sample_projected_maps(
            tm, layer, p.xy, dpdx.xy, dpdy.xy);
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
    var splat_s = terrain_fetch_splats(tm, splat_uv, splat_ddx, splat_ddy);
    var weight = terrain_unpack_splats(splat_s);
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
    let local_xz = world_xz - tm.terrain_origin;
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
    var splat_s = terrain_fetch_splats(tm, splat_uv, splat_ddx, splat_ddy);
    var weight = terrain_unpack_splats(splat_s);
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

    let local_xz = world_pos.xz - tm.terrain_origin;
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
