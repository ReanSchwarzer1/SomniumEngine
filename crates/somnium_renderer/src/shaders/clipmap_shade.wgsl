// Somnium Engine — Terrain clipmap shading (Phase DF).
//
// Group 2 is 2D-array views of the caches generate paints as color attachments
// (UE5 RVT / this engine's G-buffer pattern). Compute storage writes of those
// arrays sampled as black on Vulkan (Dbg 32). Dummy 1×1 until a terrain exists.
//
// Concatenated after `terrain_material.wgsl` and before `shading.wgsl`.

@group(2) @binding(0) var clipmap_detail_albedo: texture_2d_array<f32>;
@group(2) @binding(1) var clipmap_detail_surface: texture_2d_array<f32>;
@group(2) @binding(2) var clipmap_macro_albedo: texture_2d_array<f32>;
@group(2) @binding(3) var clipmap_macro_normal: texture_2d_array<f32>;

fn clipmap_vec2_from_packed(v: array<vec4<f32>, 4>, ring: u32) -> vec2<f32> {
    let packed = v[ring / 2u];
    let o = (ring % 2u) * 2u;
    return vec2<f32>(packed[o], packed[o + 1u]);
}

fn clipmap_macro_vec2(v: array<vec4<f32>, 2>, ring: u32) -> vec2<f32> {
    let packed = v[ring / 2u];
    let o = (ring % 2u) * 2u;
    return vec2<f32>(packed[o], packed[o + 1u]);
}

fn clipmap_uv(center: vec2<f32>, origin: vec2<f32>, tpm: f32, size: f32, world_xz: vec2<f32>) -> vec2<f32> {
    let extent = size / max(tpm, 0.0001);
    let logical = (world_xz - center) / extent + vec2<f32>(0.5);
    return fract(logical + origin);
}

/// Inset so the square cache's corners are unused. O3DE `m_validDetailClipmapRadius`.
const CLIPMAP_SAFE: f32 = 0.96;
/// Blend to the next ring over this many texels (O3DE `m_clipmapBlendSize` = 256).
const CLIPMAP_BLEND_TEXELS: f32 = 256.0;

fn clipmap_half_extent(size: f32, tpm: f32) -> f32 {
    return size / max(tpm, 0.0001) * 0.5;
}

fn clipmap_dist_m(center: vec2<f32>, world_xz: vec2<f32>) -> f32 {
    return length(world_xz - center);
}

fn clipmap_covers(center: vec2<f32>, world_xz: vec2<f32>, half: f32, inset: f32) -> bool {
    return clipmap_dist_m(center, world_xz) < half * inset;
}

fn clipmap_edge_w(center: vec2<f32>, world_xz: vec2<f32>, half: f32, tpm: f32) -> f32 {
    let dist_texels = clipmap_dist_m(center, world_xz) * max(tpm, 0.0001);
    let valid = half * CLIPMAP_SAFE * max(tpm, 0.0001);
    let begin = max(valid - CLIPMAP_BLEND_TEXELS, 0.0);
    return saturate((dist_texels - begin) / max(CLIPMAP_BLEND_TEXELS, 1.0));
}

fn clipmap_wrap_i(v: i32, size: i32) -> i32 {
    var m = v % size;
    if m < 0 {
        m += size;
    }
    return m;
}

/// Toroidal bilinear. Hardware Repeat + anisotropy on `default_sampler` smears
/// the wrap seam into the streak bands at square ring edges (Dbg 32).
fn clipmap_load4(tex: texture_2d_array<f32>, uv: vec2<f32>, ring: u32, size: f32) -> vec4<f32> {
    let s = i32(size);
    let exact = uv * size - vec2<f32>(0.5);
    let i0 = i32(floor(exact.x));
    let j0 = i32(floor(exact.y));
    let f = fract(exact);
    let p00 = vec2<i32>(clipmap_wrap_i(i0, s), clipmap_wrap_i(j0, s));
    let p10 = vec2<i32>(clipmap_wrap_i(i0 + 1, s), p00.y);
    let p01 = vec2<i32>(p00.x, clipmap_wrap_i(j0 + 1, s));
    let p11 = vec2<i32>(p10.x, p01.y);
    let r = i32(ring);
    let c00 = textureLoad(tex, p00, r, 0);
    let c10 = textureLoad(tex, p10, r, 0);
    let c01 = textureLoad(tex, p01, r, 0);
    let c11 = textureLoad(tex, p11, r, 0);
    return mix(mix(c00, c10, f.x), mix(c01, c11, f.x), f.y);
}

/// Finest **ready** ring whose interior still covers `world_xz`. Unfilled
/// rings are skipped so a blob-empty finest cannot hide a coarser ring that
/// already painted this frame.
fn clipmap_pick_detail_ring(tm: TerrainMaterial, world_xz: vec2<f32>) -> u32 {
    for (var r = 0u; r < tm.clipmap_rings; r = r + 1u) {
        if (tm.clipmap_detail_ready & (1u << r)) == 0u {
            continue;
        }
        let tpm = tm.clipmap_tpm[r / 4u][r % 4u];
        let half = clipmap_half_extent(tm.clipmap_size, tpm);
        let c = clipmap_vec2_from_packed(tm.clipmap_center, r);
        if clipmap_covers(c, world_xz, half, CLIPMAP_SAFE) {
            return r;
        }
    }
    return tm.clipmap_rings;
}

fn clipmap_pick_macro_ring(tm: TerrainMaterial, world_xz: vec2<f32>) -> u32 {
    var ring = tm.clipmap_macro_rings;
    for (var r = 0u; r < tm.clipmap_macro_rings; r = r + 1u) {
        if (tm.clipmap_macro_ready & (1u << r)) == 0u {
            continue;
        }
        let tpm = tm.clipmap_macro_tpm[r];
        let half = clipmap_half_extent(tm.clipmap_macro_size, tpm);
        let c = clipmap_macro_vec2(tm.clipmap_macro_center, r);
        if clipmap_covers(c, world_xz, half, CLIPMAP_SAFE) {
            ring = r;
            break;
        }
    }
    return ring;
}

struct ClipmapTap {
    albedo: vec3<f32>,
    roughness: f32,
    occlusion: f32,
    nxy: vec2<f32>,
    valid: bool,
}

fn clipmap_tap_detail(tm: TerrainMaterial, world_xz: vec2<f32>, ring: u32) -> ClipmapTap {
    var t: ClipmapTap;
    t.valid = false;
    t.albedo = vec3<f32>(0.0);
    t.roughness = 0.8;
    t.occlusion = 1.0;
    t.nxy = vec2<f32>(0.0);
    if ring >= tm.clipmap_rings || (tm.clipmap_detail_ready & (1u << ring)) == 0u {
        return t;
    }
    let tpm = tm.clipmap_tpm[ring / 4u][ring % 4u];
    let c = clipmap_vec2_from_packed(tm.clipmap_center, ring);
    let half = clipmap_half_extent(tm.clipmap_size, tpm);
    if !clipmap_covers(c, world_xz, half, CLIPMAP_SAFE) {
        return t;
    }
    let uv = clipmap_uv(
        c,
        clipmap_vec2_from_packed(tm.clipmap_origin, ring),
        tpm,
        tm.clipmap_size,
        world_xz,
    );
    let a = clipmap_load4(clipmap_detail_albedo, uv, ring, tm.clipmap_size);
    let s = clipmap_load4(clipmap_detail_surface, uv, ring, tm.clipmap_size);
    t.albedo = a.rgb;
    t.roughness = s.b;
    t.occlusion = s.a;
    t.nxy = s.rg * 2.0 - 1.0;
    t.valid = true;
    return t;
}

fn clipmap_tap_macro(tm: TerrainMaterial, world_xz: vec2<f32>, ring: u32) -> ClipmapTap {
    var t: ClipmapTap;
    t.valid = false;
    t.albedo = vec3<f32>(0.0);
    t.roughness = 0.8;
    t.occlusion = 1.0;
    t.nxy = vec2<f32>(0.0);
    if ring >= tm.clipmap_macro_rings || (tm.clipmap_macro_ready & (1u << ring)) == 0u {
        return t;
    }
    let tpm = tm.clipmap_macro_tpm[ring];
    let c = clipmap_macro_vec2(tm.clipmap_macro_center, ring);
    let half = clipmap_half_extent(tm.clipmap_macro_size, tpm);
    if !clipmap_covers(c, world_xz, half, CLIPMAP_SAFE) {
        return t;
    }
    let uv = clipmap_uv(
        c,
        clipmap_macro_vec2(tm.clipmap_macro_origin, ring),
        tpm,
        tm.clipmap_macro_size,
        world_xz,
    );
    let a = clipmap_load4(clipmap_macro_albedo, uv, ring, tm.clipmap_macro_size);
    let n = clipmap_load4(clipmap_macro_normal, uv, ring, tm.clipmap_macro_size);
    t.albedo = a.rgb;
    t.roughness = n.b;
    t.occlusion = n.a;
    t.nxy = n.rg * 2.0 - 1.0;
    t.valid = true;
    return t;
}

fn clipmap_blend_taps(a: ClipmapTap, b: ClipmapTap, w: f32) -> ClipmapTap {
    if !b.valid || w <= 0.0 {
        return a;
    }
    if !a.valid {
        return b;
    }
    var t = a;
    t.albedo = mix(a.albedo, b.albedo, w);
    t.roughness = mix(a.roughness, b.roughness, w);
    t.occlusion = mix(a.occlusion, b.occlusion, w);
    t.nxy = mix(a.nxy, b.nxy, w);
    t.valid = true;
    return t;
}

fn evaluate_clipmap_material(
    tm: TerrainMaterial,
    world_pos: vec3<f32>,
    geo_normal: vec3<f32>,
    splat_uv: vec2<f32>,
    world_ddx: vec2<f32>,
    world_ddy: vec2<f32>,
) -> TerrainSurface {
    _ = splat_uv;
    let tangent = normalize(vec3<f32>(1.0, 0.0, 0.0) - geo_normal * geo_normal.x);
    let bitangent = cross(geo_normal, tangent);
    let steepness = 1.0 - abs(geo_normal.y);
    let cliff_blend = smoothstep(0.45, 0.7, steepness);
    let world_xz = world_pos.xz;
    var ring = clipmap_pick_detail_ring(tm, world_xz);
    terrain_clipmap_ring = select(
        1.0,
        f32(ring) / max(f32(tm.clipmap_rings - 1u), 1.0),
        ring < tm.clipmap_rings,
    );

    let parallax_shadow = 1.0;
    let sample_xz = world_xz;
    var tap: ClipmapTap;
    tap.albedo = vec3<f32>(0.0);
    tap.roughness = 0.8;
    tap.occlusion = 1.0;
    tap.nxy = vec2<f32>(0.0);
    tap.valid = false;
    if ring < tm.clipmap_rings {
        tap = clipmap_tap_detail(tm, sample_xz, ring);
        let tpm = tm.clipmap_tpm[ring / 4u][ring % 4u];
        let c = clipmap_vec2_from_packed(tm.clipmap_center, ring);
        let half = clipmap_half_extent(tm.clipmap_size, tpm);
        let w = clipmap_edge_w(c, sample_xz, half, tpm);
        if w > 0.0 {
            if ring + 1u < tm.clipmap_rings {
                tap = clipmap_blend_taps(tap, clipmap_tap_detail(tm, sample_xz, ring + 1u), w);
            } else {
                let mr = clipmap_pick_macro_ring(tm, sample_xz);
                tap = clipmap_blend_taps(tap, clipmap_tap_macro(tm, sample_xz, mr), w);
            }
        }
    }
    if !tap.valid {
        let mr = clipmap_pick_macro_ring(tm, sample_xz);
        tap = clipmap_tap_macro(tm, sample_xz, mr);
    }
    if !tap.valid {
        let splat_uv_fb = (world_xz - tm.terrain_origin) * tm.inv_world_size;
        if tm.macro_map >= 0 {
            tap.albedo = textureSampleLevel(
                textures[tm.macro_map], default_sampler, splat_uv_fb, 0.0).rgb;
        } else {
            tap.albedo = vec3<f32>(0.35, 0.32, 0.28);
        }
        tap.roughness = 0.8;
        tap.occlusion = 1.0;
        tap.nxy = vec2<f32>(0.0);
        tap.valid = true;
    }
    var albedo = tap.albedo;
    var roughness = tap.roughness;
    var occlusion = tap.occlusion;
    let nxy = tap.nxy;
    var n_ts = vec3<f32>(nxy, sqrt(max(1.0 - dot(nxy, nxy), 0.0)));
    var surfgrad = ts_to_surfgrad(n_ts, tangent, bitangent);
    var taps = 2u;

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

    terrain_wetness_factor = 0.0;
    terrain_cliff_blend_dbg = cliff_blend;
    terrain_dominant_albedo = albedo;
    terrain_wet_f0 = 0.0;
    terrain_discarded = 0.0;
    terrain_selected_rgb = vec3<f32>(0.0);
    terrain_weight_rgb = vec3<f32>(0.0);

    var out: TerrainSurface;
    out.albedo = albedo;
    out.taps = taps;
    out.discarded = 0.0;
    out.selected_rgb = vec3<f32>(terrain_clipmap_ring);
    out.weight_rgb = vec3<f32>(0.0);
    out.parallax_shadow = parallax_shadow;
    out.roughness = max(roughness, 0.05);
    out.occlusion = occlusion;
    out.normal = resolve_surfgrad(geo_normal, surfgrad);
    return out;
}
