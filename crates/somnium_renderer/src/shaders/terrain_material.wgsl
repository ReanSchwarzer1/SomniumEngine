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

// Mirrors `terrain::GpuTerrainMaterial` (112 bytes). Every vec4 sits on a
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

fn terrain_layer_tiling(tm: TerrainMaterial, layer: u32) -> f32 {
    return tm.layer_tiling[layer / 4u][layer % 4u];
}

@group(0) @binding(11) var<storage, read> terrain_materials: array<TerrainMaterial>;

/// What the terrain material contributes to the shared `Surface`.
struct TerrainSurface {
    albedo: vec3<f32>,
    roughness: f32,
    normal: vec3<f32>,
    /// Phase 25K: real per-material ambient occlusion, packed alongside the
    /// normal. Terrain has never had this — it hardcoded a fully-open 1.0.
    occlusion: f32,
}

struct TerrainLayerSample {
    albedo: vec3<f32>,
    // Phase 25K: a real displacement map from the packed albedo's alpha, where
    // it used to be procedural noise. This is what `terrain_height_blend`
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
) -> TerrainLayerSample {
    var s: TerrainLayerSample;
    let albedo_map = tm.albedo_maps[layer / 4u][layer % 4u];
    let surface_map = tm.surface_maps[layer / 4u][layer % 4u];

    var a: vec4<f32>;
    var surf: vec4<f32>;
    if tm.hex_tiling != 0u {
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

/// Height-based blend sharpening (Phase 14C-3): the material with the greater
/// procedural height pokes through at a transition instead of soft-mixing, so
/// gravel settles into rock rather than cross-fading across it.
fn terrain_height_blend(weights: vec4<f32>, heights: vec4<f32>) -> vec4<f32> {
    let blend_sharpness = 0.25;
    let adjusted = heights * 0.4 + weights;
    let max_h = max(max(adjusted.x, adjusted.y), max(adjusted.z, adjusted.w));
    let threshold = max_h - blend_sharpness;
    let cut = max(adjusted - vec4(threshold), vec4(0.0)) * step(vec4(0.001), weights);
    return cut / max(cut.x + cut.y + cut.z + cut.w, 0.0001);
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

    // Two passes, because the height blend needs every contributing layer's
    // height before any of them can be weighted (Phase 14C-3). Only layers with
    // a meaningful weight are sampled at all — see LAYER_WEIGHT_EPSILON.
    var samples: array<TerrainLayerSample, 8>;
    var adjusted: array<f32, 8>;
    var max_adjusted = 0.0;
    for (var i = 0u; i < TERRAIN_LAYERS; i = i + 1u) {
        if weight[i] < LAYER_WEIGHT_EPSILON {
            adjusted[i] = 0.0;
            continue;
        }
        let tiling = terrain_layer_tiling(tm, i);
        samples[i] = terrain_sample_layer(
            tm, i, local_xz * tiling, world_ddx * tiling, world_ddy * tiling);
        adjusted[i] = samples[i].height * 0.4 + weight[i];
        max_adjusted = max(max_adjusted, adjusted[i]);
    }

    // Height-based sharpening: the material standing proudest at this texel
    // takes the pixel outright instead of soft-mixing across the transition.
    let threshold = max_adjusted - 0.25;
    var blend: array<f32, 8>;
    var blend_sum = 0.0;
    for (var i = 0u; i < TERRAIN_LAYERS; i = i + 1u) {
        var b = 0.0;
        if weight[i] >= LAYER_WEIGHT_EPSILON {
            b = max(adjusted[i] - threshold, 0.0);
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
        albedo += samples[i].albedo * b;
        normal_acc += samples[i].normal_ts * b;
        roughness += samples[i].roughness * b;
        occlusion += samples[i].occlusion * b;
    }
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
