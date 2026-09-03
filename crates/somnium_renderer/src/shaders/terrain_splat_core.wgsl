// Terrain data and bounded splat helpers shared by raster and ray-query paths.
//
// Keep this module deliberately small. Ray-query roots compose it without the
// full raster terrain material so backend compilers never see the painted
// weight-noise, hex-tiling, or parallax call graphs while compiling a ray hit.

// Mirrors `terrain::GpuTerrainMaterial` (2080 bytes). Every vec4 sits on a
// 16-byte offset; `tests/shaders_validate.rs` pins the complete layout.
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
    horizon_map_a: i32,
    horizon_map_b: i32,
    skyvis_map: i32,
    sky_visibility_strength: f32,
    relief_map: i32,
    relief_takeover: f32,
    weight_noise_strength: f32,
    weight_noise_scale: f32,
    macro_octave_strength: vec4<f32>,
}

@group(0) @binding(11) var<storage, read> terrain_materials: array<TerrainMaterial>;

/// Layers per terrain — must match `textures::TERRAIN_LAYER_COUNT`.
const TERRAIN_LAYERS: u32 = 32u;

/// Maximum splat layers considered by this pipeline variant.
override terrain_scan: u32 = 32u;

fn terrain_splat_groups() -> u32 {
    return (min(terrain_scan, TERRAIN_LAYERS) + 3u) / 4u;
}

fn terrain_moisture(tm: TerrainMaterial, layer: u32) -> f32 {
    return tm.layer_moisture[layer / 4u][layer % 4u];
}

/// Splat samples to normalised per-layer weights, without painted noise.
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
