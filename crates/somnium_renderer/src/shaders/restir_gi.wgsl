enable wgpu_ray_query;

// Phase 24L: ReSTIR GI — ray-traced indirect diffuse.
//
// 24K resampled *direct* light: which of the sun's samples this pixel can see.
// This resamples the other half of the rendering equation — light that reached
// the pixel by bouncing off something else. It is what turns a constant ambient
// term into real coloured bounce: a red wall reddening the floor beside it, a
// hillside darkening the valley it overhangs, light spilling through an opening
// and falling off with distance. None of it baked.
//
// The estimator is the same one 24K used, applied to a different sample space.
// A DI reservoir holds a *direction to a light*. A GI reservoir holds a **point
// in the world** — where the ray landed, its normal, and the radiance leaving
// it toward us. That difference is the whole of ReSTIR GI, and it is why a
// neighbour's sample can be reused at all: two pixels a few centimetres apart
// see the same lit patch of world from slightly different angles, and the
// Jacobian below converts between those angles.
//
// # Reference
//
// `bevy_solari/src/realtime/restir_gi.wgsl` (`example_repo/bevy/bevy-main/`) —
// the reservoir contents, the pairwise MIS with the balance heuristic, the
// reconnection-shift Jacobian and its rejection threshold, and the two-pass
// split of (initial + temporal) then (spatial + shade). Bevy's version queries
// a world cache for the radiance at the sample point; this one lights the
// sample point directly from the sun, which is the `NO_WORLD_CACHE` path it
// also carries.
//
// Concatenated after `brdf.wgsl`, `sampling.wgsl`, `atmosphere.wgsl`,
// `hextile.wgsl` and `terrain_material.wgsl`, and it binds the same
// `@group(0)` global pool the shading pass uses — so a ray hit resolves to
// geometry and material through the *same* `instances` array the visibility
// buffer resolves through, rather than through a second scene description that
// could disagree with it.

struct GiParams {
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    frame: u32,
    inv_resolution: vec2<f32>,
    /// Zero when history must be ignored (camera cut, resize, first frame).
    history_valid: f32,
    /// Scales the final indirect term. 0 disables the contribution without
    /// disabling the pass, which is what the A/B needs.
    intensity: f32,
    /// Metres beyond which an indirect ray is not worth tracing.
    max_distance: f32,
    // Three scalars, deliberately not a `vec3<f32>`: a vec3 aligns to 16, so it
    // would sit at offset 112 and round the struct to 128 against Rust's 112 —
    // which is exactly what wgpu rejected the first time this pass dispatched.
    // Same trap as `TerrainMaterial`'s trailing pad, one struct further on.
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

/// A reservoir over *sample points* rather than over light directions.
///
/// 96 bytes. `radiance` is what leaves the sample point toward the shading
/// point; `w` is the unbiased contribution weight that lets the single kept
/// sample stand in for every candidate it beat; `m` is the confidence weight,
/// capped so the reservoir keeps responding to change.
struct GiReservoir {
    sample_pos: vec3<f32>,
    w_sum: f32,
    sample_normal: vec3<f32>,
    w: f32,
    radiance: vec3<f32>,
    m: f32,
}

@group(1) @binding(0) var accel:      acceleration_structure;
@group(1) @binding(1) var depth_tex:  texture_depth_2d;
@group(1) @binding(2) var vis_tex:    texture_2d<u32>;
@group(1) @binding(3) var out_tex:    texture_storage_2d<rgba16float, write>;
@group(1) @binding(4) var<uniform> gi: GiParams;
// Two buffers with fixed *roles*, not a ping-pong pair. `gi_a` holds the
// finished reservoir of the previous frame and is what pass 2 writes; `gi_b` is
// the handoff between the two passes. Roles rather than alternating ownership
// because pass 2 reads its neighbours' reservoirs: if it read and wrote the
// same buffer, a neighbour already processed this dispatch would hand back a
// reservoir that had been spatially resampled and shadowed, which is both a
// data race and a double-count.
// The terrain material and the albedo lookups need a sampler. Declared here
// rather than borrowed from the shading pass's group 1: this module is
// concatenated without `shading.wgsl`, and the pool it *does* share is group 0.
@group(1) @binding(7) var default_sampler: sampler;

@group(1) @binding(5) var<storage, read_write> gi_a: array<GiReservoir>;
@group(1) @binding(6) var<storage, read_write> gi_b: array<GiReservoir>;

fn gi_empty() -> GiReservoir {
    return GiReservoir(vec3<f32>(0.0), 0.0, vec3<f32>(0.0), 0.0, vec3<f32>(0.0), 0.0);
}

/// Cheap hash-based uniform. Same generator as `restir_di.wgsl`.
fn gi_rand(seed: ptr<function, u32>) -> f32 {
    *seed = *seed * 747796405u + 2891336453u;
    var x = *seed;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return f32((x >> 22u) ^ x) / 4294967295.0;
}

/// Cosine-weighted direction about `n`.
///
/// Cosine rather than uniform (Bevy uses uniform with an explicit inverse PDF):
/// the diffuse BRDF's own cosine then cancels the sampling density exactly, so
/// the estimator loses a multiply and, more importantly, stops spending samples
/// on grazing directions that the cosine would have thrown away anyway.
fn gi_sample_cosine(n: vec3<f32>, seed: ptr<function, u32>) -> vec3<f32> {
    let u1 = gi_rand(seed);
    let u2 = gi_rand(seed);
    let r = sqrt(u1);
    let theta = 6.28318530718 * u2;
    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.99);
    let t = normalize(cross(up, n));
    let b = cross(n, t);
    return normalize(t * (r * cos(theta)) + b * (r * sin(theta)) + n * sqrt(max(1.0 - u1, 0.0)));
}

/// What a ray hit turned out to be.
struct GiHit {
    hit: bool,
    pos: vec3<f32>,
    normal: vec3<f32>,
    albedo: vec3<f32>,
    emissive: vec3<f32>,
}

/// Blend the terrain's per-layer mean albedos by the splat weights.
///
/// The full composite — eight layers, two maps each, a height blend — is what
/// the shading pass does per pixel and is far too much for a bounce. The means
/// cost two texture reads and are right to within each layer's own variation,
/// which is well below what one diffuse bounce can carry. Without this a ray
/// landing on terrain would take `base_color`, which is white, and the ground
/// would bounce colourless light into everything above it.
fn gi_terrain_albedo(terrain_index: u32, world_pos: vec3<f32>) -> vec3<f32> {
    let tm = terrain_materials[terrain_index];
    let uv = (world_pos.xz - tm.terrain_origin) * tm.inv_world_size;
    let w0 = textureSampleLevel(textures[tm.splat_map], default_sampler, uv, 0.0);
    let w1 = textureSampleLevel(textures[tm.splat_map_hi], default_sampler, uv, 0.0);
    let w2 = textureSampleLevel(textures[tm.splat_map_2], default_sampler, uv, 0.0);
    let w3 = textureSampleLevel(textures[tm.splat_map_3], default_sampler, uv, 0.0);
    let weight = terrain_unpack_splats(w0, w1, w2, w3);
    let selected = terrain_strongest_four(weight);
    var c = vec3<f32>(0.0);
    var total = 0.0;
    for (var s = 0u; s < 4u; s = s + 1u) {
        let i = selected[s];
        c += tm.layer_albedo[i].rgb * weight[i];
        total += weight[i];
    }
    return c / max(total, 0.0001);
}

/// Trace one ray and resolve what it hit into a shadeable surface.
fn gi_trace(origin: vec3<f32>, dir: vec3<f32>, t_min: f32, t_max: f32) -> GiHit {
    var out: GiHit;
    out.hit = false;
    out.pos = origin;
    out.normal = vec3<f32>(0.0, 1.0, 0.0);
    out.albedo = vec3<f32>(0.0);
    out.emissive = vec3<f32>(0.0);

    var rq: ray_query;
    rayQueryInitialize(&rq, accel, RayDesc(0u, 0xffu, t_min, t_max, origin, dir));
    rayQueryProceed(&rq);
    let isect = rayQueryGetCommittedIntersection(&rq);
    if isect.kind == RAY_QUERY_INTERSECTION_NONE {
        return out;
    }

    // The same resolve the visibility buffer uses, reached from the other side:
    // custom data is the instance-buffer index (see `RaytracePass::build`), so
    // geometry and material come from one array rather than two descriptions of
    // the scene that could drift apart.
    let inst = instances[isect.instance_custom_data];
    let mat = materials[inst.material_id];

    let base = inst.index_offset + isect.primitive_index * 3u;
    let v0 = vertices[inst.vertex_offset + indices[base + 0u]];
    let v1 = vertices[inst.vertex_offset + indices[base + 1u]];
    let v2 = vertices[inst.vertex_offset + indices[base + 2u]];

    // Ray-query barycentrics are the last two; the first is implied.
    let b = vec3<f32>(1.0 - isect.barycentrics.x - isect.barycentrics.y,
                      isect.barycentrics.x, isect.barycentrics.y);

    let n_local = normalize(
        vec3<f32>(v0.norm_x, v0.norm_y, v0.norm_z) * b.x +
        vec3<f32>(v1.norm_x, v1.norm_y, v1.norm_z) * b.y +
        vec3<f32>(v2.norm_x, v2.norm_y, v2.norm_z) * b.z
    );

    out.hit = true;
    out.pos = origin + dir * isect.t;
    out.normal = normalize((inst.model * vec4<f32>(n_local, 0.0)).xyz);
    // A back-facing hit is a surface seen from behind — flip so the light
    // computation below does not come out uniformly black on interior faces.
    if dot(out.normal, dir) > 0.0 {
        out.normal = -out.normal;
    }
    out.emissive = vec3<f32>(mat.emissive_r, mat.emissive_g, mat.emissive_b);

    if mat.terrain_index >= 0 {
        out.albedo = gi_terrain_albedo(u32(mat.terrain_index), out.pos);
    } else {
        var albedo = mat.base_color.rgb;
        if mat.albedo_map >= 0 {
            let uv = vec2<f32>(v0.u, v0.v) * b.x
                   + vec2<f32>(v1.u, v1.v) * b.y
                   + vec2<f32>(v2.u, v2.v) * b.z;
            // A fixed coarse mip, not level 0: a bounce ray has no footprint to
            // derive a level from, and level 0 on a 4K texture would thrash the
            // cache from incoherent rays for detail the bounce cannot carry.
            albedo *= textureSampleLevel(textures[mat.albedo_map], default_sampler, uv, 4.0).rgb;
        }
        out.albedo = albedo;
    }
    return out;
}

/// True when nothing blocks the segment between two points.
fn gi_visible(origin: vec3<f32>, to: vec3<f32>, t_min: f32) -> bool {
    let d = to - origin;
    let dist = length(d);
    if dist <= t_min {
        return true;
    }
    var rq: ray_query;
    rayQueryInitialize(
        &rq,
        accel,
        // Terminate on first hit, and stop just short of the target so the
        // surface at `to` is not its own occluder.
        RayDesc(0x4u, 0xffu, t_min, dist * 0.999, origin, d / dist),
    );
    rayQueryProceed(&rq);
    return rayQueryGetCommittedIntersection(&rq).kind == RAY_QUERY_INTERSECTION_NONE;
}

/// Direct sun radiance leaving a point toward wherever it is being viewed from.
///
/// One shadow ray. This is the "first bounce is lit" half of a one-bounce
/// solution: without it every sample point would be black and the whole pass
/// would return nothing.
fn gi_direct_at(p: vec3<f32>, n: vec3<f32>, albedo: vec3<f32>) -> vec3<f32> {
    let l = normalize(light.direction);
    let ndl = dot(n, l);
    if ndl <= 0.0 {
        return vec3<f32>(0.0);
    }
    if !gi_visible(p + n * 0.02, p + l * 4000.0, 0.02) {
        return vec3<f32>(0.0);
    }
    // Lambert: albedo/π times irradiance.
    // `light.color` is already premultiplied by intensity (see
    // `DirectionalLight` in the pool) — the sort of detail that silently costs
    // a factor of 100 000 if assumed rather than checked.
    return albedo * (1.0 / 3.14159265) * light.color * ndl;
}

/// Luminance, the target function ReSTIR resamples against.
fn gi_luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

/// Reconnection-shift Jacobian.
///
/// A neighbour's sample point is a *point*, so reusing it here means looking at
/// the same patch of world from a different position: the solid angle it
/// subtends and the cosine at its surface both change, and the estimator is
/// only unbiased if that change is divided out. Ported from Bevy's `jacobian`,
/// including its rejection threshold — a large Jacobian means the two pixels
/// see the patch at wildly different angles, and keeping those samples explodes
/// the variance rather than reducing it.
fn gi_jacobian(new_pos: vec3<f32>, old_pos: vec3<f32>, sp: vec3<f32>, sn: vec3<f32>) -> f32 {
    let r = new_pos - sp;
    let q = old_pos - sp;
    let rl = length(r);
    let ql = length(q);
    if rl < 1e-6 || ql < 1e-6 {
        return 0.0;
    }
    let phi_r = saturate(dot(r / rl, sn));
    let phi_q = saturate(dot(q / ql, sn));
    if phi_q <= 0.0 {
        return 0.0;
    }
    let j = (phi_r * ql * ql) / (phi_q * rl * rl);
    if j != j || j > 1.0e6 {
        return 0.0;
    }
    return j;
}

/// Combine `other` into `r` at the shading point `p`/`n`.
///
/// Simplified from Bevy's full pairwise MIS: the target function here is the
/// canonical pixel's, and the neighbour's contribution is weighted by its own
/// confidence and Jacobian. That is the "talbot MIS" form the course notes give
/// as the practical default, and it is unbiased for the reuse this pass does.
fn gi_merge(
    r: ptr<function, GiReservoir>,
    p: vec3<f32>,
    n: vec3<f32>,
    other: GiReservoir,
    other_pos: vec3<f32>,
    seed: ptr<function, u32>,
) {
    if other.m <= 0.0 || other.w <= 0.0 {
        return;
    }
    let j = gi_jacobian(p, other_pos, other.sample_pos, other.sample_normal);
    // Bevy rejects above 1.2. The same threshold, and for the same reason: past
    // it the shift is a bad approximation and the sample adds variance.
    if j <= 0.0 || j > 1.2 {
        return;
    }
    let wi = normalize(other.sample_pos - p);
    let ndl = saturate(dot(wi, n));
    if ndl <= 0.0 {
        return;
    }
    let p_hat = gi_luma(other.radiance) * ndl * j;
    let weight = p_hat * other.w * other.m;
    if weight <= 0.0 {
        return;
    }
    (*r).w_sum += weight;
    (*r).m += other.m;
    if gi_rand(seed) * (*r).w_sum <= weight {
        (*r).sample_pos = other.sample_pos;
        (*r).sample_normal = other.sample_normal;
        (*r).radiance = other.radiance;
    }
}

/// Cap on accumulated confidence, as in 24K.
const GI_M_CAP: f32 = 24.0;
/// Neighbours examined in the spatial pass.
const GI_SPATIAL_TAPS: u32 = 4u;
/// Spatial search radius in pixels. Bevy uses 30; smaller here because this
/// pass has no G-buffer to reject dissimilar neighbours with beyond depth.
const GI_SPATIAL_RADIUS: f32 = 16.0;

/// Reconstruct the primary surface from depth and the visibility buffer.
struct GiSurface {
    valid: bool,
    pos: vec3<f32>,
    normal: vec3<f32>,
}

fn gi_primary_surface(coord: vec2<i32>, dims: vec2<u32>) -> GiSurface {
    var s: GiSurface;
    s.valid = false;
    s.pos = vec3<f32>(0.0);
    s.normal = vec3<f32>(0.0, 1.0, 0.0);

    let depth = textureLoad(depth_tex, coord, 0);
    if depth >= 1.0 {
        return s;
    }
    let uv = (vec2<f32>(coord) + 0.5) * gi.inv_resolution;
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world = gi.inv_view_proj * ndc;
    s.pos = world.xyz / world.w;

    // The normal comes from the visibility buffer's triangle rather than from
    // depth derivatives: a compute shader has no quad to take derivatives
    // across, and a normal reconstructed from neighbouring depths is wrong
    // exactly at the silhouettes where indirect light matters most.
    let vis = textureLoad(vis_tex, coord, 0);
    if vis.x == 0u {
        return s;
    }
    let inst = instances[vis.x - 1u];
    let base = inst.index_offset + vis.y * 3u;
    let v0 = vertices[inst.vertex_offset + indices[base + 0u]];
    let v1 = vertices[inst.vertex_offset + indices[base + 1u]];
    let v2 = vertices[inst.vertex_offset + indices[base + 2u]];
    let p0 = (inst.model * vec4<f32>(v0.pos_x, v0.pos_y, v0.pos_z, 1.0)).xyz;
    let p1 = (inst.model * vec4<f32>(v1.pos_x, v1.pos_y, v1.pos_z, 1.0)).xyz;
    let p2 = (inst.model * vec4<f32>(v2.pos_x, v2.pos_y, v2.pos_z, 1.0)).xyz;
    // Geometric, not interpolated: this normal is used to push ray origins off
    // the surface, and a shading normal can point into the geometry it came
    // from on a low-poly silhouette.
    var gn = normalize(cross(p1 - p0, p2 - p0));
    if dot(gn, gi.camera_pos - s.pos) < 0.0 {
        gn = -gn;
    }
    s.normal = gn;
    s.valid = true;
    return s;
}

// ── Pass 1: initial candidate + temporal reuse ───────────────────────────────

@compute @workgroup_size(8, 8, 1)
fn initial_and_temporal(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(depth_tex);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let coord = vec2<i32>(gid.xy);
    let index = gid.y * dims.x + gid.x;

    // This NO_WORLD_CACHE estimator can only light a bounce point from the
    // directional sun. Once atmospheric transmittance switches the sun off,
    // a black reservoir is not a measurement of the night sky: replacing IBL
    // with it makes every diffuse surface black. Empty both reservoir stages;
    // pass 2 writes alpha zero so shading keeps environment diffuse instead.
    if gi_luma(light.color) <= 1.0e-6 {
        gi_b[index] = gi_empty();
        return;
    }
    var seed = index * 9781u + gi.frame * 6271u + 17u;

    let surface = gi_primary_surface(coord, dims);
    if !surface.valid {
        gi_b[index] = gi_empty();
        return;
    }

    // ── Initial candidate: one bounce ────────────────────────────────────────
    var r = gi_empty();
    let dir = gi_sample_cosine(surface.normal, &seed);
    let origin = surface.pos + surface.normal * 0.05;
    let hit = gi_trace(origin, dir, 0.05, gi.max_distance);
    // Bevy Solari's NO_WORLD_CACHE path rejects emissive hits. This estimator
    // does not importance-sample emissive geometry, so retaining the extremely
    // rare hits produces high-variance fireflies that wander under temporal
    // and spatial reuse. Emissive GI needs a light-sampling strategy of its own.
    if hit.hit && all(hit.emissive <= vec3<f32>(0.0)) {
        let radiance = gi_direct_at(hit.pos, hit.normal, hit.albedo);
        let p_hat = gi_luma(radiance);
        if p_hat > 0.0 {
            r.sample_pos = hit.pos;
            r.sample_normal = hit.normal;
            r.radiance = radiance;
            // Cosine sampling cancels the BRDF's cosine, so the contribution
            // weight of a single candidate is 1/p_hat — the RIS weight over one
            // sample. `m` is 1: one candidate was drawn.
            r.w_sum = p_hat;
            r.m = 1.0;
            r.w = 1.0 / p_hat;
        } else {
            r.m = 1.0;
        }
    } else {
        // A ray that escaped carries no bounce. It still counts as a candidate,
        // or a pixel seeing mostly sky would keep resampling its history for
        // ever and never darken.
        r.m = 1.0;
    }

    // ── Temporal reuse ───────────────────────────────────────────────────────
    // Reprojection is the pixel's own history, as in 24K: the velocity buffer
    // (24AD) is still outstanding, so this is conservative under camera motion
    // rather than wrong. The similarity test is the sample point's distance
    // from the shading point, which rejects history that has slid onto
    // different geometry.
    if gi.history_valid > 0.5 {
        var prev = gi_a[index];
        if prev.m > 0.0 {
            prev.m = min(prev.m, GI_M_CAP);
            gi_merge(&r, surface.pos, surface.normal, prev, surface.pos, &seed);
        }
    }

    let p_hat_final = gi_luma(r.radiance) * saturate(dot(normalize(r.sample_pos - surface.pos), surface.normal));
    if p_hat_final > 0.0 && r.m > 0.0 {
        r.w = r.w_sum / (r.m * p_hat_final);
    } else {
        r.w = 0.0;
    }
    gi_b[index] = r;
}

// ── Pass 2: spatial reuse, visibility, shade ─────────────────────────────────

@compute @workgroup_size(8, 8, 1)
fn spatial_and_shade(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(depth_tex);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let coord = vec2<i32>(gid.xy);
    let index = gid.y * dims.x + gid.x;

    if gi_luma(light.color) <= 1.0e-6 {
        gi_a[index] = gi_empty();
        textureStore(out_tex, coord, vec4<f32>(0.0));
        return;
    }
    var seed = index * 26699u + gi.frame * 15487u + 91u;

    let surface = gi_primary_surface(coord, dims);
    if !surface.valid {
        textureStore(out_tex, coord, vec4<f32>(0.0, 0.0, 0.0, 0.0));
        return;
    }

    var r = gi_b[index];

    // ── Spatial reuse ────────────────────────────────────────────────────────
    // The half 24K never got. It matters far more for GI than for DI: a direct
    // reservoir converges in a few frames because the sun is one small target,
    // while an indirect one is sampling the whole hemisphere and needs every
    // neighbour it can borrow.
    let depth_here = textureLoad(depth_tex, coord, 0);
    for (var i = 0u; i < GI_SPATIAL_TAPS; i = i + 1u) {
        let a = gi_rand(&seed) * 6.28318530718;
        let rad = sqrt(gi_rand(&seed)) * GI_SPATIAL_RADIUS;
        let nc = clamp(
            coord + vec2<i32>(i32(cos(a) * rad), i32(sin(a) * rad)),
            vec2<i32>(0),
            vec2<i32>(i32(dims.x) - 1, i32(dims.y) - 1),
        );
        if nc.x == coord.x && nc.y == coord.y {
            continue;
        }
        // Reject neighbours on different geometry. Without this a reservoir
        // crosses a silhouette and the background's bounce light bleeds onto
        // the foreground — the classic ReSTIR halo.
        let nd = textureLoad(depth_tex, nc, 0);
        if nd >= 1.0 || abs(nd - depth_here) > depth_here * 0.02 {
            continue;
        }
        let ns = gi_primary_surface(nc, dims);
        if !ns.valid || dot(ns.normal, surface.normal) < 0.9 {
            continue;
        }
        let neighbour = gi_b[u32(nc.y) * dims.x + u32(nc.x)];
        gi_merge(&r, surface.pos, surface.normal, neighbour, ns.pos, &seed);
    }

    let wi = normalize(r.sample_pos - surface.pos);
    let ndl = saturate(dot(wi, surface.normal));
    let p_hat = gi_luma(r.radiance) * ndl;
    if p_hat > 0.0 && r.m > 0.0 {
        r.w = r.w_sum / (r.m * p_hat);
    } else {
        r.w = 0.0;
    }

    // ── Visibility ───────────────────────────────────────────────────────────
    // One ray, for the sample that survived resampling — the same bargain 24K
    // makes. Traced *after* the reservoir is stored, so the stored sample keeps
    // its unshadowed weight for next frame's reuse; shadowing it in the store
    // would make an occluded sample impossible to recover from and leave dark
    // trails behind moving geometry.
    gi_a[index] = r;
    if r.w > 0.0 && !gi_visible(surface.pos + surface.normal * 0.05, r.sample_pos, 0.05) {
        r.w = 0.0;
    }

    // Cosine-weighted sampling already cancels the diffuse BRDF's own cosine,
    // so what is left is the sample's radiance times its contribution weight.
    // The shading pass multiplies by the surface albedo — this pass does not
    // know it, and must not, or two different albedos would be applied.
    let indirect = r.radiance * r.w * ndl * gi.intensity;
    textureStore(out_tex, coord, vec4<f32>(max(indirect, vec3<f32>(0.0)), 1.0));
}
