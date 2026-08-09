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
    layer_tiling: vec4<f32>,
    // xy = brush world XZ, z = radius, w = mode
    // (0 off, 1 sculpt, 2 layer paint, 3 foliage).
    brush: vec4<f32>,
    albedo_maps: vec4<i32>,
    surface_maps: vec4<i32>,
    _reserved_maps: vec4<i32>,
    terrain_origin: vec2<f32>,
    inv_world_size: vec2<f32>,
    splat_map: i32,
    cliff_layer: u32,
    /// Phase 25F: non-zero applies stochastic hex-tiling to the layer maps.
    hex_tiling: u32,
    _pad1: u32,
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
    let albedo_map = tm.albedo_maps[layer];
    let surface_map = tm.surface_maps[layer];

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
    let map = textures[tm.albedo_maps[layer]];
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

    var weights = textureSample(textures[tm.splat_map], default_sampler, splat_uv);
    weights = weights / max(weights.x + weights.y + weights.z + weights.w, 0.0001);

    // All four layers are sampled unconditionally: a texture fetch behind a
    // per-pixel branch has no uniform derivative to work with.
    let local_xz = world_pos.xz - tm.terrain_origin;
    let s0 = terrain_sample_layer(
        tm, 0u, local_xz * tm.layer_tiling.x,
        world_ddx * tm.layer_tiling.x, world_ddy * tm.layer_tiling.x);
    let s1 = terrain_sample_layer(
        tm, 1u, local_xz * tm.layer_tiling.y,
        world_ddx * tm.layer_tiling.y, world_ddy * tm.layer_tiling.y);
    let s2 = terrain_sample_layer(
        tm, 2u, local_xz * tm.layer_tiling.z,
        world_ddx * tm.layer_tiling.z, world_ddy * tm.layer_tiling.z);
    let s3 = terrain_sample_layer(
        tm, 3u, local_xz * tm.layer_tiling.w,
        world_ddx * tm.layer_tiling.w, world_ddy * tm.layer_tiling.w);

    let w = terrain_height_blend(
        weights, vec4(s0.height, s1.height, s2.height, s3.height));

    var albedo = s0.albedo * w.x + s1.albedo * w.y + s2.albedo * w.z + s3.albedo * w.w;
    let normal_ts = normalize(
        s0.normal_ts * w.x + s1.normal_ts * w.y + s2.normal_ts * w.z + s3.normal_ts * w.w,
    );
    var roughness =
        s0.roughness * w.x + s1.roughness * w.y + s2.roughness * w.z + s3.roughness * w.w;
    let occlusion =
        s0.occlusion * w.x + s1.occlusion * w.y + s2.occlusion * w.z + s3.occlusion * w.w;

    // Triplanar cliff projection on steep slopes (Phase 14C step 4).
    let steepness = 1.0 - abs(geo_normal.y);
    let cliff_blend = smoothstep(0.45, 0.7, steepness);
    let cliff = terrain_triplanar_albedo(
        tm,
        tm.cliff_layer,
        world_pos - vec3(tm.terrain_origin.x, 0.0, tm.terrain_origin.y),
        geo_normal,
        tm.layer_tiling[tm.cliff_layer],
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
