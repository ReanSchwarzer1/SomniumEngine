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

// Mirrors `terrain::GpuTerrainMaterial` (256 bytes). Every vec4 sits on a
// 16-byte offset; see the Rust struct for why that is load-bearing.
struct TerrainMaterial {
    // Phase 25L: eight layers. Arrays of vec4 rather than a flat array, so the
    // WGSL layout matches Rust's `repr(C)` packing without per-element padding
    // — WGSL gives a bare `array<f32, 8>` a 16-byte stride.
    layer_tiling: array<vec4<f32>, 2>,
    // xy = brush world XZ, z = radius, w = mode
    // (0 off, 1 sculpt, 2 layer paint, 3 foliage).
    brush: vec4<f32>,
    albedo_maps: array<vec4<i32>, 2>,
    surface_maps: array<vec4<i32>, 2>,
    terrain_origin: vec2<f32>,
    inv_world_size: vec2<f32>,
    splat_map: i32,
    splat_map_hi: i32,
    cliff_layer: u32,
    /// Phase 25F: non-zero applies stochastic hex-tiling to the layer maps.
    hex_tiling: u32,
    /// Phase 25E per-layer blend parameters. Same vec4-pair packing as
    /// `layer_tiling`, and for the same alignment reason.
    layer_height_scale: array<vec4<f32>, 2>,
    layer_blend_width: array<vec4<f32>, 2>,
    layer_weight_clamp: array<vec4<f32>, 2>,
    /// Non-zero runs the height-weighted blend; zero is the plain splat blend.
    height_blend: u32,
    /// Phase 25D: the macro tier.
    macro_map: i32,
    macro_mode: u32,
    macro_strength: f32,
    /// Metres over which the per-pixel layer budget falls away.
    detail_fade_start: f32,
    detail_fade_end: f32,
    // Three scalars, deliberately not a `vec3<u32>`: a vec3 aligns to 16, so it
    // would land at offset 256 and give the struct a 272-byte stride against
    // Rust's 256 — every terrain past index 0 would then decode from the wrong
    // words. Same trap as the one the header warns about, one field further on.
    _pad0: u32,
    _pad1: u32,
}

/// Layers per terrain — must match `textures::TERRAIN_LAYER_COUNT`.
const TERRAIN_LAYERS: u32 = 8u;

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

@group(0) @binding(11) var<storage, read> terrain_materials: array<TerrainMaterial>;

/// Layer texture reads the current fragment issued, for debug mode 12.
///
/// A private rather than a field on `Surface`: it exists only to be looked at,
/// and threading it through the shared surface struct would put a debug counter
/// in the path of every mesh in the scene.
var<private> terrain_taps: u32 = 0u;

/// The worst case a pixel can pay — eight layers, two maps each, three hex taps
/// per map. Debug mode 12 scales by this so the heatmap reads as a fraction of
/// the old fixed cost.
const TERRAIN_MAX_TAPS: f32 = 48.0;

/// What the terrain material contributes to the shared `Surface`.
struct TerrainSurface {
    albedo: vec3<f32>,
    /// Layer texture reads this pixel actually issued (Phase 25D). Carried for
    /// debug mode 12 and for nothing else — it is what makes "detail cost
    /// scales with screen area" a measurement rather than a claim.
    taps: u32,
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
    var s: TerrainLayerSample;
    let albedo_map = tm.albedo_maps[layer / 4u][layer % 4u];
    let surface_map = tm.surface_maps[layer / 4u][layer % 4u];

    var a: vec4<f32>;
    var surf: vec4<f32>;
    if hex {
        a = hex_sample(albedo_map, uv, ddx, ddy);
        surf = hex_sample(surface_map, uv, ddx, ddy);
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
fn terrain_macro_sample(tm: TerrainMaterial, splat_uv: vec2<f32>) -> vec4<f32> {
    if tm.macro_map < 0 {
        return vec4<f32>(0.5, 0.5, 0.5, 0.0);
    }
    let m = textureSample(textures[tm.macro_map], default_sampler, splat_uv);
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

/// Triplanar sample of one layer's albedo: project along the three axes and
/// blend by a sharpened normal weight. Used on cliffs, where a heightfield's
/// top-down UV stretches into vertical streaks.
fn terrain_triplanar_albedo(
    tm: TerrainMaterial,
    layer: u32,
    world_pos: vec3<f32>,
    n: vec3<f32>,
    tiling: f32,
) -> vec3<f32> {
    var w = pow(abs(n), vec3(4.0));
    w = w / (w.x + w.y + w.z);
    let map = textures[tm.albedo_maps[layer / 4u][layer % 4u]];
    let cx = textureSample(map, default_sampler, world_pos.zy * tiling).rgb;
    let cy = textureSample(map, default_sampler, world_pos.xz * tiling).rgb;
    let cz = textureSample(map, default_sampler, world_pos.xy * tiling).rgb;
    return cx * w.x + cy * w.y + cz * w.z;
}

/// Evaluate the terrain surface at a world-space point.
///
/// `splat_uv` is the terrain-global [0,1] coordinate carried in the chunk
/// vertices, which the shading pass interpolates like any other UV — one of the
/// things that made moving terrain into the visibility buffer cheap.
/// `world_ddx` / `world_ddy` are the screen-space derivatives of the world
/// position, taken in the caller where control flow is uniform. The layer UVs
/// are `local_xz * tiling`, so their derivatives are these scaled by the same
/// rate — which the hex-tiled path needs explicitly, since each of its taps
/// reads a different part of the texture and implicit derivatives would be
/// computed across that discontinuity.
fn evaluate_terrain_material(
    terrain_index: u32,
    world_pos: vec3<f32>,
    geo_normal: vec3<f32>,
    splat_uv: vec2<f32>,
    world_ddx: vec2<f32>,
    world_ddy: vec2<f32>,
) -> TerrainSurface {
    let tm = terrain_materials[terrain_index];

    // Eight weights from two RGBA splatmaps, normalised together.
    let w_lo = textureSample(textures[tm.splat_map], default_sampler, splat_uv);
    let w_hi = textureSample(textures[tm.splat_map_hi], default_sampler, splat_uv);
    let total = max(
        w_lo.x + w_lo.y + w_lo.z + w_lo.w + w_hi.x + w_hi.y + w_hi.z + w_hi.w,
        0.0001,
    );
    var weight = array<f32, 8>(
        w_lo.x / total, w_lo.y / total, w_lo.z / total, w_lo.w / total,
        w_hi.x / total, w_hi.y / total, w_hi.z / total, w_hi.w / total,
    );

    let local_xz = world_pos.xz - tm.terrain_origin;

    // Phase 25D: the per-pixel layer budget falls with distance. What comes
    // off is *layers*, not taps per layer.
    //
    // Dropping hex-tiling past the fade was tried first, on the reasoning that
    // three taps per map exist to hide a repetition already below a pixel at
    // that range. It is wrong, and the A/B showed it as a hard lattice across
    // the whole mid-ground: at distance a 4 m tile is a few pixels wide, so the
    // repetition does not vanish — it beats against the pixel grid and becomes
    // *more* visible than it is close up. Hex-tiling is what removes that, and
    // it earns its taps furthest away. The layer gate was doing nearly all of
    // the saving anyway.
    let view_distance = distance(world_pos, view.camera_pos);
    let fade = terrain_detail_fade(tm, view_distance);
    let epsilon = mix(LAYER_WEIGHT_EPSILON, FAR_LAYER_EPSILON, fade);
    let hex = tm.hex_tiling != 0u;

    // Sample every layer that can still affect the pixel. Nothing below the
    // gate may be read afterwards, because `samples[i]` for a skipped layer
    // holds whatever the previous iteration left there.
    var samples: array<TerrainLayerSample, 8>;
    var adjusted: array<f32, 8>;
    var taps = 0u;
    for (var i = 0u; i < TERRAIN_LAYERS; i = i + 1u) {
        if weight[i] < epsilon {
            adjusted[i] = 0.0;
            continue;
        }
        let tiling = terrain_layer_tiling(tm, i);
        samples[i] = terrain_sample_layer(
            tm, i, local_xz * tiling, world_ddx * tiling, world_ddy * tiling, hex);
        taps += select(2u, 6u, hex);
        if tm.height_blend != 0u {
            adjusted[i] = terrain_append_height(tm, i, weight[i], samples[i].height);
        } else {
            adjusted[i] = weight[i];
        }
    }

    // Depth blend (Phase 25E, O3DE `GetDetailSurface`). Only materials within
    // their own transition band of the winner contribute, and because the band
    // is measured on weights that already carry each layer's relief, the
    // boundary follows the rock's crevices rather than a contour of the
    // splatmap. `min_depth` lets the widest-blending material in the set widen
    // the band for everything it meets, which is what keeps snow soft against
    // rock instead of the harder material dictating both edges.
    var max_w = 0.0;
    var min_depth = -1e30;
    for (var i = 0u; i < TERRAIN_LAYERS; i = i + 1u) {
        if weight[i] < epsilon {
            continue;
        }
        max_w = max(max_w, adjusted[i]);
        min_depth = max(min_depth, adjusted[i] - terrain_blend_width(tm, i));
    }

    var blend: array<f32, 8>;
    var blend_sum = 0.0;
    for (var i = 0u; i < TERRAIN_LAYERS; i = i + 1u) {
        var b = 0.0;
        if weight[i] >= epsilon {
            let local_min = max(min_depth, max_w - terrain_blend_width(tm, i));
            b = max((adjusted[i] - local_min) / max(max_w - local_min, 1e-4), 0.0);
        }
        blend[i] = b;
        blend_sum += b;
    }
    blend_sum = max(blend_sum, 0.0001);

    var albedo = vec3<f32>(0.0);
    var normal_acc = vec3<f32>(0.0);
    var roughness = 0.0;
    var occlusion = 0.0;
    for (var i = 0u; i < TERRAIN_LAYERS; i = i + 1u) {
        let b = blend[i] / blend_sum;
        if b <= 0.0 {
            continue;
        }
        // Phase 25E: albedo is averaged in an approximately perceptual space
        // (O3DE does the same, `sqrt` in and squared out). A weighted mean of
        // *linear* albedo between two materials of different luminance sits
        // below both when read back through the display transform, so a seam
        // that should be a texture boundary shows up as a dark band along it.
        albedo += sqrt(samples[i].albedo) * b;
        normal_acc += samples[i].normal_ts * b;
        roughness += samples[i].roughness * b;
        occlusion += samples[i].occlusion * b;
    }

    // Phase 25D: the macro tier goes in here, while the albedo is still in the
    // perceptual space the layers were averaged in — which is also the space
    // O3DE performs its overlay and linear-light modes in, and the reason a
    // macro texel of 0.5 is the identity.
    let macro_c = terrain_macro_sample(tm, splat_uv);
    albedo = terrain_macro_blend(albedo, macro_c.rgb, tm.macro_mode, macro_c.a);

    albedo = albedo * albedo;
    let normal_ts = normalize(normal_acc);

    // Triplanar cliff projection on steep slopes (Phase 14C step 4).
    let steepness = 1.0 - abs(geo_normal.y);
    let cliff_blend = smoothstep(0.45, 0.7, steepness);
    let cliff = terrain_triplanar_albedo(
        tm,
        tm.cliff_layer,
        world_pos - vec3(tm.terrain_origin.x, 0.0, tm.terrain_origin.y),
        geo_normal,
        terrain_layer_tiling(tm, tm.cliff_layer),
    );
    albedo = mix(albedo, cliff, cliff_blend);
    roughness = mix(roughness, 0.8, cliff_blend);

    // Tangent basis for an up-facing heightfield: tangent along +X projected
    // onto the surface (terrain UVs are axis-aligned by construction). Derived
    // here rather than taken from the shading pass's UV-delta TBN, whose
    // determinant collapses on a grid whose UVs are perfectly axis-aligned.
    let tangent = normalize(vec3<f32>(1.0, 0.0, 0.0) - geo_normal * geo_normal.x);
    let bitangent = cross(geo_normal, tangent);
    let tbn = mat3x3<f32>(tangent, bitangent, geo_normal);

    var out: TerrainSurface;
    out.albedo = albedo;
    out.taps = taps;
    out.roughness = max(roughness, 0.05);
    out.occlusion = occlusion;
    // Generated normal maps use Z-up tangent space; remap to TBN (x→T, y→B).
    out.normal = normalize(
        tbn * normalize(vec3(normal_ts.x, normal_ts.y, max(normal_ts.z, 0.2))));
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
