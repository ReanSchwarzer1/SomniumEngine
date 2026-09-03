// Shared ray-hit resolution (Phase VV-D).
//
// Extracted from `gi_trace()` so ReSTIR GI and Halcyon water reflections
// resolve a triangle through the same instance → barycentric → material path.
// Concatenated after a module that `enable`s `wgpu_ray_query` and declares
// `accel` plus `default_sampler`; `global_pool.wgsl` and
// `terrain_splat_core.wgsl` supply the scene arrays and bounded terrain-hit
// helpers. Declarations are order-independent.

/// What a ray hit turned out to be.
struct RtHit {
    hit: bool,
    pos: vec3<f32>,
    normal: vec3<f32>,
    albedo: vec3<f32>,
    emissive: vec3<f32>,
    roughness: f32,
    metallic: f32,
    /// Ray `t` of this intersection. 0 on a true miss. A cutout reject keeps
    /// the card's `t` so `rt_trace` can continue through the hole.
    t: f32,
}

fn rt_miss(origin: vec3<f32>) -> RtHit {
    var out: RtHit;
    out.hit = false;
    out.pos = origin;
    out.normal = vec3<f32>(0.0, 1.0, 0.0);
    out.albedo = vec3<f32>(0.0);
    out.emissive = vec3<f32>(0.0);
    out.roughness = 1.0;
    out.metallic = 0.0;
    out.t = 0.0;
    return out;
}

/// Blend the terrain's per-layer mean albedos by the splat weights.
///
/// The full composite is what the shading pass does per pixel and is far too
/// much for a bounce or a reflection ray. The means cost two texture reads
/// and are right to within each layer's own variation.
fn rt_terrain_albedo(terrain_index: u32, world_pos: vec3<f32>) -> vec3<f32> {
    let tm = terrain_materials[terrain_index];
    let uv = (world_pos.xz - tm.terrain_origin) * tm.inv_world_size;

    // Accumulated straight out of the splat samples: no scan array, no
    // strongest-four, no normalise pass.
    //
    // # Why this shape, and not the shading path's
    //
    // Two earlier attempts to bound this pipeline's compilation failed, and
    // both failed the same way — they removed *code* from the ray path without
    // removing the two things that are expensive to inline into a ray-query
    // entry point:
    //
    // - `terrain_unpack_splats` returns `array<f32, 32>`. A 128-byte local,
    //   dynamically indexed, becomes scratch memory in a ray shader.
    // - `terrain_strongest_four` is a 32-iteration insertion sort with four
    //   branches and eight conditional moves per iteration. Its bound is an
    //   `override`, so it is a compile-time constant and unrolls completely:
    //   roughly 256 branches inlined into the hit.
    //
    // Neither buys anything here. This function blends per-layer *mean*
    // albedos precisely because the full composite is far too much for a
    // bounce ray — and once you are averaging means, ranking them and keeping
    // the top four is work whose result you then throw away by averaging
    // anyway. Summing all thirty-two weighted means is fewer instructions,
    // allocates nothing, and is *more* accurate than the top-four
    // approximation it replaces.
    //
    // Eight iterations, one bindless fetch each, no local array, no sort.
    var c = vec3<f32>(0.0);
    var moisture = 0.0;
    var total = 0.0;
    for (var g = 0u; g < 8u; g = g + 1u) {
        let id = tm.splat_maps[g / 4u][g % 4u];
        if id >= 0 {
            let s = textureSampleLevel(textures[id], default_sampler, uv, 0.0);
            let base = g * 4u;
            c += tm.layer_albedo[base + 0u].rgb * s.x
                + tm.layer_albedo[base + 1u].rgb * s.y
                + tm.layer_albedo[base + 2u].rgb * s.z
                + tm.layer_albedo[base + 3u].rgb * s.w;
            moisture += terrain_moisture(tm, base + 0u) * s.x
                + terrain_moisture(tm, base + 1u) * s.y
                + terrain_moisture(tm, base + 2u) * s.z
                + terrain_moisture(tm, base + 3u) * s.w;
            total += s.x + s.y + s.z + s.w;
        }
    }

    // Splat weights are stored unnormalised, so the divide is the normalise the
    // unpack used to do. An unpainted texel has no albedo to report and returns
    // black rather than a division by zero.
    let inv = 1.0 / max(total, 0.0001);
    let wet = saturate(tm.wetness * moisture * inv);
    return c * inv * mix(1.0, tm.wetness_darken, wet);
}

/// Resolve a committed ray-query intersection against the global pool.
fn rt_resolve(origin: vec3<f32>, dir: vec3<f32>, isect: RayIntersection) -> RtHit {
    if isect.kind == RAY_QUERY_INTERSECTION_NONE {
        return rt_miss(origin);
    }

    var out: RtHit;
    // Custom data is the instance-buffer index (see `RaytracePass::build`), so
    // geometry and material come from one array rather than two descriptions of
    // the scene that could drift apart.
    let inst = instances[isect.instance_custom_data];
    let mat = materials[inst.material_id];

    let base = inst.index_offset + isect.primitive_index * 3u;
    let v0 = vertices[inst.vertex_offset + indices[base + 0u]];
    let v1 = vertices[inst.vertex_offset + indices[base + 1u]];
    let v2 = vertices[inst.vertex_offset + indices[base + 2u]];

    let b = vec3<f32>(
        1.0 - isect.barycentrics.x - isect.barycentrics.y,
        isect.barycentrics.x,
        isect.barycentrics.y,
    );

    let n_local = normalize(
        vec3<f32>(v0.norm_x, v0.norm_y, v0.norm_z) * b.x +
        vec3<f32>(v1.norm_x, v1.norm_y, v1.norm_z) * b.y +
        vec3<f32>(v2.norm_x, v2.norm_y, v2.norm_z) * b.z
    );

    out.hit = true;
    out.t = isect.t;
    out.pos = origin + dir * isect.t;
    out.normal = normalize((inst.model * vec4<f32>(n_local, 0.0)).xyz);
    if dot(out.normal, dir) > 0.0 {
        out.normal = -out.normal;
    }
    out.emissive = vec3<f32>(mat.emissive_r, mat.emissive_g, mat.emissive_b);
    out.roughness = clamp(mat.roughness, 0.04, 1.0);
    out.metallic = clamp(mat.metallic, 0.0, 1.0);

    if mat.terrain_index >= 0 {
        out.albedo = rt_terrain_albedo(u32(mat.terrain_index), out.pos);
        out.roughness = 0.55;
        out.metallic = 0.0;
    } else {
        var albedo = mat.base_color.rgb;
        if mat.albedo_map >= 0 {
            let uv = vec2<f32>(v0.u, v0.v) * b.x
                   + vec2<f32>(v1.u, v1.v) * b.y
                   + vec2<f32>(v2.u, v2.v) * b.z;
            // MASK cards keep blade colour only where the atlas is opaque;
            // the rest is near-black. Closest-hit without an any-hit shader
            // would shade that background as a solid surface, which reads as
            // pinpricks on water when RT reflections are on.
            if mat.alpha_cutoff > 0.0 {
                let texel = textureSampleLevel(
                    textures[mat.albedo_map], default_sampler, uv, 0.0);
                if texel.a < mat.alpha_cutoff {
                    var skip = rt_miss(origin);
                    skip.t = isect.t;
                    return skip;
                }
                albedo *= texel.rgb;
            } else {
                albedo *= textureSampleLevel(
                    textures[mat.albedo_map], default_sampler, uv, 4.0).rgb;
            }
        }
        out.albedo = albedo;
    }
    return out;
}

fn rt_trace(origin: vec3<f32>, dir: vec3<f32>, t_min: f32, t_max: f32) -> RtHit {
    var t = t_min;
    for (var hop = 0u; hop < 4u; hop++) {
        var rq: ray_query;
        rayQueryInitialize(&rq, accel, RayDesc(0u, 0xffu, t, t_max, origin, dir));
        rayQueryProceed(&rq);
        let hit = rt_resolve(origin, dir, rayQueryGetCommittedIntersection(&rq));
        if hit.hit {
            return hit;
        }
        // True miss has t = 0. A cutout reject keeps the card's t so the
        // next hop can see whatever is behind the hole.
        if hit.t <= t + 1e-4 {
            return rt_miss(origin);
        }
        t = hit.t + 0.002;
    }
    return rt_miss(origin);
}
