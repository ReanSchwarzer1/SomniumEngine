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
/// Linear + Repeat + **anisotropy 1**, and only ever used at an explicit LOD.
///
/// The earlier hardware-sampling attempt reached for `default_sampler`, which
/// runs at `anisotropy_clamp: 16` for terrain's grazing angles. An anisotropic
/// tap takes a *footprint*, and a footprint that straddles the toroidal seam
/// reads texels belonging to the far side of the world — which is what showed
/// up as streak bands at the ring edges in Dbg 32. It was the anisotropy, not
/// the wrap: with no footprint and an explicit level this is exactly the 2×2
/// bilinear the manual version was computing by hand.
@group(2) @binding(4) var clipmap_sampler: sampler;

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

/// Toroidal bilinear, in one instruction.
///
/// This was four `textureLoad`s plus the wrap arithmetic and the two mixes,
/// per image — eight taps for one cache lookup, and sixteen for any pixel
/// inside the ring-blend band, which is roughly three quarters of a ring's
/// area. `Repeat` addressing performs the same wrap in the sampler, and an
/// explicit level keeps it legal in non-uniform control flow (no derivatives
/// are taken), so nothing about the result changes.
fn clipmap_sample(tex: texture_2d_array<f32>, uv: vec2<f32>, ring: u32) -> vec4<f32> {
    return textureSampleLevel(tex, clipmap_sampler, uv, i32(ring), 0.0);
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
    let a = clipmap_sample(clipmap_detail_albedo, uv, ring);
    let s = clipmap_sample(clipmap_detail_surface, uv, ring);
    // An ungenerated texel is exactly zero — wgpu zero-fills a new texture, and
    // a ring can briefly hold a strip generate has not reached. Generate always
    // writes occlusion into alpha, and a real blend of layer AO maps is never
    // 0, so zero alpha means "no data" rather than "fully occluded". Reporting
    // it as data cost the surface all of its indirect light and read as a black
    // wedge with straight edges; reporting it as invalid falls through to the
    // next coarser ring.
    if s.a <= 0.0 {
        return t;
    }
    // Squared back to linear: the cache stores albedo perceptually so an 8-bit
    // channel can resolve dark ground. See `clipmap_gen.wgsl`.
    t.albedo = a.rgb * a.rgb;
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
    let a = clipmap_sample(clipmap_macro_albedo, uv, ring);
    let n = clipmap_sample(clipmap_macro_normal, uv, ring);
    // Zero alpha is an ungenerated texel; see `clipmap_tap_detail`.
    if n.a <= 0.0 {
        return t;
    }
    t.albedo = a.rgb * a.rgb;
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
    // Walk **outward** to the next detail ring that actually has data.
    //
    // `clipmap_pick_detail_ring` only knows that a ring is flagged ready and
    // geometrically covers this position. The tap additionally rejects texels
    // generate has not written, and dropping straight to the macro stack on
    // that rejection is what turned a strip of missing detail into a flat,
    // dark band with straight edges: the macro rings are coarse, and if they
    // miss too the fallback is a constant colour with no normal at all — which
    // is why the band had no ripples in it while the sand around it did.
    //
    // The loop almost always exits on its first iteration, and the rings it
    // skips reject on the cheap `ready` / `covers` tests before sampling.
    if ring < tm.clipmap_rings {
        for (var r = ring; r < tm.clipmap_rings; r = r + 1u) {
            let probe = clipmap_tap_detail(tm, sample_xz, r);
            if probe.valid {
                tap = probe;
                ring = r;
                break;
            }
        }
        terrain_clipmap_ring = select(
            1.0,
            f32(ring) / max(f32(tm.clipmap_rings - 1u), 1.0),
            ring < tm.clipmap_rings,
        );
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
            // Squared for the same reason `terrain_macro_blend` squares after
            // mixing: the unique-colour map is authored perceptually. Reading
            // it as linear made this fallback noticeably brighter than the
            // cache it stands in for.
            let m = textureSampleLevel(
                textures[tm.macro_map], default_sampler, splat_uv_fb, 0.0).rgb;
            tap.albedo = m * m;
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
