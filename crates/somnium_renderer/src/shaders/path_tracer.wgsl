//!include "spot_cookie.wgsl"
//!include "rt_hit.wgsl"
//!include "global_pool.wgsl"
//!include "brdf.wgsl"
//!include "sampling.wgsl"
//!include "terrain_splat_core.wgsl"
//!include "wind.wgsl"

enable wgpu_ray_query;

// Real-time path tracer (17 Oct 2026; replaces the 24O accumulator in lighting_extra.wgsl, which was a Lambert-only,
// sun-only progressive average that froze the simulation while it gathered).
//
// One path a pixel a frame, for the denoiser in path_denoise.wgsl to accumulate (reprojected) and filter:
// - surfaces: Burley/Hammon diffuse and GGX specular through `evaluate_brdf`, the lobe picked by its weight, the
//   specular direction from GGX's visible normals (Heitz 2018);
// - next-event estimation: the sun and the moon with a shadow ray each, and one lamp picked by resampled importance
//   (Talbot 2005: up to eight candidates weighed by their unshadowed light here) with a shadow ray;
// - emission where a path lands on a glowing surface; the sky where it leaves;
// - Russian roulette after the second bounce, and a clamp on what any one bounce may add (fireflies).
//
// Mode 0 traces the light that reaches each surface after bouncing, which is what the raster's environment, probes
// and GI only approximate (29 Oct 2026). It starts from the surface the raster itself lit: shading.wgsl writes the
// finished material at every pixel while this runs (albedo with its paint, decals and terrain layers, metal, the
// normal-mapped normal, roughness, the material's own occlusion), keeps its direct light (the sun's and the lamps'
// traced shadows included) and the sky's light (its fill and its reflection, under GTAO), and leaves the rest of its
// ambient (probes, GI, reflections of the scene) to this; the composite in path_denoise.wgsl adds this back. Before, the
// tracer rebuilt each surface from vertex normals, a flat roughness and the bare albedo texture, and lit it all itself,
// so normal maps, roughness and metal maps, vertex paint, baked light and decals were gone (the developer, 29 Oct).
// A surface whose light is baked into it (`VertexPaint.baked_light`, the PS2 levels) receives nothing: its light is
// the bake, as the raster has it. The light is written demodulated (divided by the albedo) so the denoiser blurs
// light, not texture.
//
// Mode 1, ray-traced reflections (17 Oct 2026; engine features record D4): the raster lights the frame as usual and
// only glossy surfaces (roughness up to `max_rough`) trace one ray along their GGX lobe, lit at its hit by the same
// sun, moon and lamp sampling; what it brings back is the light arriving along the reflection, which shading puts in
// place of the environment's specular (its `lighting_extra` specular bit). The denoiser keeps a short history and
// filters a mirror not at all, a rough floor more.

struct PtParams {
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    frame: u32,
    bounces: u32,
    lamp_count: u32,
    firefly: f32,
    env_intensity: f32,
    /// 0 the whole picture, 1 reflections only.
    mode: u32,
    max_rough: f32,
    _pad: vec2<u32>,
}

@group(1) @binding(0) var accel: acceleration_structure;
@group(1) @binding(1) var env_cube: texture_cube<f32>;
@group(1) @binding(2) var env_sampler: sampler;
@group(1) @binding(3) var default_sampler: sampler;
@group(1) @binding(4) var<uniform> pt: PtParams;
/// The primary surface's light divided by its albedo (rgb); a = 1 on a surface, 0 where the view sees the sky.
@group(1) @binding(5) var out_light: texture_storage_2d<rgba16float, write>;
/// The primary surface's normal (xyz) and its distance from the eye (w; 0 for the sky and for what is not traced).
@group(1) @binding(6) var out_gbuf: texture_storage_2d<rgba16float, write>;
/// What the filtered light is multiplied back by (mode 0: the albedo it was divided by, the material's occlusion and
/// the fog's transmittance; mode 1: white, with the roughness in a).
@group(1) @binding(7) var out_albedo: texture_storage_2d<rgba16float, write>;
/// The raster's visibility buffer (instance + 1, triangle): the surface each pixel sees.
@group(1) @binding(9) var vis_tex: texture_2d<u32>;
/// Mode 0, the raster's finished surface at each pixel (shading.wgsl `path_surface_*`): albedo and metallic; the
/// shading normal and roughness; the material's own occlusion (negative: its light is baked, trace nothing) and the
/// fog's transmittance.
@group(1) @binding(10) var surface_albedo: texture_2d<f32>;
@group(1) @binding(11) var surface_normal: texture_2d<f32>;
@group(1) @binding(12) var surface_extra: texture_2d<f32>;

fn pt_rand(seed: ptr<function, u32>) -> f32 {
    *seed = *seed * 747796405u + 2891336453u;
    var x = *seed;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return f32((x >> 22u) ^ x) / 4294967295.0;
}

fn pt_luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

/// The raster lamps' falloff (shading.wgsl `smooth_distance_attenuation`), so a path-traced lamp is as bright.
fn pt_attenuation(dist: f32, range: f32, radius: f32) -> f32 {
    let ratio = dist / range;
    let ratio4 = ratio * ratio * ratio * ratio;
    let factor = saturate(1.0 - ratio4);
    let floor_r = max(radius, 0.08);
    return (factor * factor) / max(dist * dist, floor_r * floor_r);
}

fn pt_basis(n: vec3<f32>) -> mat3x3<f32> {
    var t = vec3<f32>(1.0, 0.0, 0.0);
    if abs(n.y) < 0.9 {
        t = vec3<f32>(0.0, 1.0, 0.0);
    }
    let tangent = normalize(cross(t, n));
    let bitangent = cross(n, tangent);
    return mat3x3<f32>(tangent, n, bitangent);
}

/// Unshadowed light from one lamp at `p` through the surface, and the point a shadow ray must reach.
struct LampSample {
    light: vec3<f32>,
    to: vec3<f32>,
}

fn pt_lamp(ll: GpuLocalLight, s: Surface, p: vec3<f32>) -> LampSample {
    var out: LampSample;
    out.light = vec3<f32>(0.0);
    var at = ll.position_ws;
    if ll.light_type == 4u {
        // A tube: its nearest point.
        let axis = normalize(ll.direction_ws);
        let half_len = max(ll._pad1, 0.05);
        let a = ll.position_ws - axis * half_len;
        let ab = axis * half_len * 2.0;
        let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
        at = a + ab * t;
    }
    out.to = at;
    let d = at - p;
    let dist = length(d);
    if dist > ll.range || dist < 1e-4 {
        return out;
    }
    let l = d / dist;
    if dot(s.normal, l) <= 0.0 {
        return out;
    }
    var atten = pt_attenuation(dist, ll.range, ll.radius);
    if ll.light_type == 1u {
        atten *= spot_shape(ll.direction_ws, -l, ll.spot_cos_outer, ll.spot_cos_inner, ll._pad2);
    } else if ll.light_type == 2u || ll.light_type == 3u {
        // A panel or a disc lights the side it faces.
        atten *= max(dot(-l, normalize(ll.direction_ws)), 0.0) * 2.0;
    }
    out.light = evaluate_brdf(s, l) * ll.color * atten;
    return out;
}

/// Whether nothing that casts a shadow stands between `origin` and `origin + dir * dist`: only shadow casters (TLAS
/// mask 0x1, as the raster's lamp rays use: a lamp's diffuser and housing are not casters and must not shadow their
/// own light), alpha-tested holes passed through, the first real hit enough.
fn pt_clear(origin: vec3<f32>, dir: vec3<f32>, dist: f32) -> bool {
    if dist <= 0.02 {
        return true;
    }
    var rq: ray_query;
    rayQueryInitialize(&rq, accel, RayDesc(0x6u, 0x1u, 0.01, dist - 0.02, origin, dir));
    while rayQueryProceed(&rq) {
        if rt_candidate_is_covered(rayQueryGetCandidateIntersection(&rq)) {
            rayQueryConfirmIntersection(&rq);
        }
    }
    return rayQueryGetCommittedIntersection(&rq).kind == RAY_QUERY_INTERSECTION_NONE;
}

/// Light reaching the surface straight from the sun, the moon and one resampled lamp, shadowed.
fn pt_direct(s: Surface, p: vec3<f32>, geo_n: vec3<f32>, seed: ptr<function, u32>) -> vec3<f32> {
    var total = vec3<f32>(0.0);
    let origin = p + geo_n * 0.02;
    let sun_dir = normalize(light.direction);
    if dot(s.normal, sun_dir) > 0.0 && pt_luma(light.color) > 1e-6 {
        let lit = evaluate_brdf(s, sun_dir) * light.color;
        if pt_luma(lit) > 1e-7 && pt_clear(origin, sun_dir, 4000.0) {
            total += lit;
        }
    }
    let sun_illuminance = pt_luma(light.color);
    let moon_strength = saturate(1.0 - sun_illuminance / 10.0);
    if moon_strength > 0.0 && light.moon_intensity > 0.0 {
        let moon_dir = normalize(light.moon_direction);
        if dot(s.normal, moon_dir) > 0.0 {
            let lit = evaluate_brdf(s, moon_dir) * vec3<f32>(0.55, 0.72, 1.0) * light.moon_intensity * moon_strength;
            if pt_luma(lit) > 1e-7 && pt_clear(origin, moon_dir, 4000.0) {
                total += lit;
            }
        }
    }
    // One lamp of the level's, picked from up to eight candidates in proportion to their light here.
    let count = min(pt.lamp_count, cluster_params.num_local_lights);
    if count > 0u {
        let m = min(count, 8u);
        var chosen = 0u;
        var chosen_weight = 0.0;
        var weights = 0.0;
        for (var k = 0u; k < m; k++) {
            let index = min(u32(pt_rand(seed) * f32(count)), count - 1u);
            let w = pt_luma(pt_lamp(local_lights[index], s, p).light);
            weights += w;
            if w > 0.0 && pt_rand(seed) * weights < w {
                chosen = index;
                chosen_weight = w;
            }
        }
        if chosen_weight > 0.0 {
            let lamp = pt_lamp(local_lights[chosen], s, p);
            // Resampled estimate: f / p_hat * (sum of candidate weights) / M * (lamps).
            let scale = (weights / f32(m)) * f32(count) / chosen_weight;
            let d = lamp.to - origin;
            let dist = length(d);
            // Stopped 6 cm short of the lamp, as the raster's lamp rays are (shading.wgsl `practical_visibility`).
            if pt_clear(origin, d / max(dist, 1e-4), dist - 0.06) {
                total += lamp.light * scale;
            }
        }
    }
    return total;
}

/// GGX visible-normal sample about +Y (Heitz 2018), in the basis's frame.
fn pt_sample_vndf(v_local: vec3<f32>, alpha: f32, u: vec2<f32>) -> vec3<f32> {
    // Work in the y-up frame: x, z across, y up.
    let vh = normalize(vec3<f32>(alpha * v_local.x, v_local.y, alpha * v_local.z));
    let lensq = vh.x * vh.x + vh.z * vh.z;
    var t1 = vec3<f32>(1.0, 0.0, 0.0);
    if lensq > 1e-7 {
        t1 = vec3<f32>(-vh.z, 0.0, vh.x) / sqrt(lensq);
    }
    let t2 = cross(vh, t1);
    let r = sqrt(u.x);
    let phi = 6.2831853 * u.y;
    let p1 = r * cos(phi);
    var p2 = r * sin(phi);
    let s = 0.5 * (1.0 + vh.y);
    p2 = (1.0 - s) * sqrt(max(1.0 - p1 * p1, 0.0)) + s * p2;
    let nh = t1 * p1 + t2 * p2 + vh * sqrt(max(0.0, 1.0 - p1 * p1 - p2 * p2));
    return normalize(vec3<f32>(alpha * nh.x, max(0.0, nh.y), alpha * nh.z));
}

/// Smith G1 for GGX, `alpha` = roughness squared.
fn pt_g1(n_dot_x: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    return 2.0 * n_dot_x / (n_dot_x + sqrt(a2 + (1.0 - a2) * n_dot_x * n_dot_x));
}

/// Perspective-correct barycentrics of the triangle (`c*` its clip positions) at `ndc`: in screen space as the raster
/// does, or where the camera ray meets the triangle's plane when a vertex is near or behind the eye.
fn pt_bary(p0: vec3<f32>, p1: vec3<f32>, p2: vec3<f32>, c0: vec4<f32>, c1: vec4<f32>, c2: vec4<f32>, near_eye: bool,
           ndc: vec2<f32>) -> vec3<f32> {
    if near_eye {
        let near = view.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
        let far = view.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
        let ro = near.xyz / near.w;
        let rd = far.xyz / far.w - ro;
        let e1 = p1 - p0;
        let e2 = p2 - p0;
        let pv = cross(rd, e2);
        let det = dot(e1, pv);
        if abs(det) > 1e-20 {
            let tv = ro - p0;
            let u = dot(tv, pv) / det;
            let v = dot(rd, cross(tv, e1)) / det;
            return vec3<f32>(1.0 - u - v, u, v);
        }
    }
    let n0 = c0.xy / c0.w;
    let n1 = c1.xy / c1.w;
    let n2 = c2.xy / c2.w;
    let det = (n1.y - n2.y) * (n0.x - n2.x) + (n2.x - n1.x) * (n0.y - n2.y);
    let w0 = ((n1.y - n2.y) * (ndc.x - n2.x) + (n2.x - n1.x) * (ndc.y - n2.y)) / det;
    let w1 = ((n2.y - n0.y) * (ndc.x - n2.x) + (n0.x - n2.x) * (ndc.y - n2.y)) / det;
    var b = vec3<f32>(w0 / c0.w, w1 / c1.w, (1.0 - w0 - w1) / c2.w);
    b /= (b.x + b.y + b.z);
    if !all(b == b) {
        return vec3<f32>(1.0 / 3.0);
    }
    return b;
}

/// The surface the raster drew at this pixel, as a ray hit: the visibility buffer names the instance and the triangle,
/// the camera ray through the pixel meets that triangle (barycentrics), and its material is read as `rt_resolve` reads
/// it, its albedo at full detail. Small props are not in the TLAS (renderer.rs `TLAS_MIN_RADIUS`), so tracing the
/// primary ray would see through them; this sees exactly what the raster does. Untextured (mode 0, which takes the
/// raster's own surface), only its place and its geometric normal are worked out.
fn pt_primary(coord: vec2<i32>, origin: vec3<f32>, ndc: vec2<f32>, textured: bool) -> RtHit {
    let vis = textureLoad(vis_tex, coord, 0);
    if vis.x == 0u {
        return rt_miss(origin);
    }
    var out: RtHit;
    let inst = instances[vis.x - 1u];
    let mat = materials[inst.material_id];
    let base = inst.index_offset + vis.y * 3u;
    let v0 = vertices[inst.vertex_offset + indices[base + 0u]];
    let v1 = vertices[inst.vertex_offset + indices[base + 1u]];
    let v2 = vertices[inst.vertex_offset + indices[base + 2u]];
    var p0 = (inst.model * vec4<f32>(v0.pos_x, v0.pos_y, v0.pos_z, 1.0)).xyz;
    var p1 = (inst.model * vec4<f32>(v1.pos_x, v1.pos_y, v1.pos_z, 1.0)).xyz;
    var p2 = (inst.model * vec4<f32>(v2.pos_x, v2.pos_y, v2.pos_z, 1.0)).xyz;
    // The triangle as the visibility pass drew it, wind included (shading.wgsl does the same).
    if mat.terrain_index < 0 {
        let pivot = inst.model[3].xyz;
        p0 += wind_offset(p0, pivot, mat.wind_bend, mat.wind_flutter, view.wind_time.x, view.wind, view.camera_pos);
        p1 += wind_offset(p1, pivot, mat.wind_bend, mat.wind_flutter, view.wind_time.x, view.wind, view.camera_pos);
        p2 += wind_offset(p2, pivot, mat.wind_bend, mat.wind_flutter, view.wind_time.x, view.wind, view.camera_pos);
    }
    // Barycentrics exactly as the raster finds them (shading.wgsl `vis_barycentric`, `ray_barycentric` near the eye),
    // here and one pixel across and down, for the texture's gradients: sampled at full detail everywhere, a floor or a
    // wall at a grazing angle aliases.
    let c0 = view.view_proj * vec4<f32>(p0, 1.0);
    let c1 = view.view_proj * vec4<f32>(p1, 1.0);
    let c2 = view.view_proj * vec4<f32>(p2, 1.0);
    let near_eye = min(c0.w, min(c1.w, c2.w)) < 1.0;
    let b = pt_bary(p0, p1, p2, c0, c1, c2, near_eye, ndc);
    out.hit = true;
    out.pos = p0 * b.x + p1 * b.y + p2 * b.z;
    // The eye's true direction to the point: the line meets the triangle at the same point whichever way `dir`
    // runs, but the normal must be turned by the real one (17 Oct: a ray unprojected through the reversed, infinite
    // projection came out backwards, every normal faced away and no lamp lit anything).
    let toward = normalize(out.pos - origin);
    out.t = length(out.pos - origin);
    let n_local = normalize(
        vec3<f32>(v0.norm_x, v0.norm_y, v0.norm_z) * b.x +
        vec3<f32>(v1.norm_x, v1.norm_y, v1.norm_z) * b.y +
        vec3<f32>(v2.norm_x, v2.norm_y, v2.norm_z) * b.z);
    out.normal = normalize((inst.model * vec4<f32>(n_local, 0.0)).xyz);
    if dot(out.normal, toward) > 0.0 {
        out.normal = -out.normal;
    }
    out.emissive = vec3<f32>(mat.emissive_r, mat.emissive_g, mat.emissive_b);
    out.roughness = clamp(mat.roughness, 0.04, 1.0);
    out.metallic = clamp(mat.metallic, 0.0, 1.0);
    out.terrain_index = mat.terrain_index;
    out.albedo = mat.base_color.rgb;
    if !textured {
        return out;
    }
    let step = 2.0 / vec2<f32>(textureDimensions(vis_tex));
    let bx = pt_bary(p0, p1, p2, c0, c1, c2, near_eye, ndc + vec2<f32>(step.x, 0.0));
    let by = pt_bary(p0, p1, p2, c0, c1, c2, near_eye, ndc - vec2<f32>(0.0, step.y));
    let t0 = vec2<f32>(v0.u, v0.v);
    let t1 = vec2<f32>(v1.u, v1.v);
    let t2 = vec2<f32>(v2.u, v2.v);
    let uv = t0 * b.x + t1 * b.y + t2 * b.z;
    let uv_dx = (t0 * bx.x + t1 * bx.y + t2 * bx.z) - uv;
    let uv_dy = (t0 * by.x + t1 * by.y + t2 * by.z) - uv;
    // A glow drawn in its texture (a screen, a lamp's panel), as the raster draws it (shading.wgsl, Phase 24T).
    if mat.emissive_map >= 0 {
        out.emissive *= textureSampleGrad(textures[mat.emissive_map], default_sampler, uv, uv_dx, uv_dy).rgb;
    }
    if mat.terrain_index >= 0 {
        out.albedo = rt_terrain_albedo(u32(mat.terrain_index), out.pos);
        out.roughness = 0.55;
        out.metallic = 0.0;
    } else if mat.albedo_map >= 0 {
        out.albedo *= textureSampleGrad(textures[mat.albedo_map], default_sampler, uv, uv_dx, uv_dy).rgb;
    }
    return out;
}

/// Mode 0: the raster's own finished surface at this pixel (shading.wgsl `path_surface_*`), seen along `view_dir`.
fn pt_raster_surface(coord: vec2<u32>, geo_n: vec3<f32>, view_dir: vec3<f32>) -> Surface {
    let a = textureLoad(surface_albedo, coord, 0);
    let nr = textureLoad(surface_normal, coord, 0);
    var s: Surface;
    s.albedo = saturate(select(vec3<f32>(0.5), a.rgb, a.rgb == a.rgb));
    s.metallic = saturate(select(0.0, a.a, a.a == a.a));
    s.roughness = clamp(select(1.0, nr.w, nr.w == nr.w), 0.045, 1.0);
    var n = geo_n;
    if all(nr.xyz == nr.xyz) && dot(nr.xyz, nr.xyz) > 0.25 {
        n = normalize(nr.xyz);
    }
    // A normal map can tilt the normal past the edge of what the eye sees; brought back just in front of it.
    let n_dot_v = dot(n, view_dir);
    if n_dot_v < 0.01 {
        n = normalize(n + view_dir * (0.01 - n_dot_v));
    }
    s.normal = n;
    s.view_dir = view_dir;
    s.f0 = mix(vec3<f32>(0.04), s.albedo, s.metallic);
    s.occlusion = 1.0;
    s.bent_normal = n;
    return s;
}

fn pt_surface(hit: RtHit, view_dir: vec3<f32>) -> Surface {
    var s: Surface;
    s.albedo = hit.albedo;
    s.roughness = clamp(hit.roughness, 0.045, 1.0);
    s.metallic = saturate(hit.metallic);
    s.normal = hit.normal;
    s.view_dir = view_dir;
    s.f0 = mix(vec3<f32>(0.04), hit.albedo, s.metallic);
    s.occlusion = 1.0;
    s.bent_normal = hit.normal;
    return s;
}

/// The chance of following the specular lobe rather than the diffuse.
fn pt_specular_chance(s: Surface) -> f32 {
    let spec = pt_luma(F_Schlick(s.f0, max(dot(s.normal, s.view_dir), 0.0)));
    let diff = pt_luma(s.albedo) * (1.0 - s.metallic);
    return clamp(spec / max(spec + diff, 1e-4), 0.1, 0.9);
}

/// The pdf of having picked `l` (both lobes together, as one sample picks either).
fn pt_pdf(s: Surface, l: vec3<f32>, p_spec: f32) -> f32 {
    let n_dot_l = dot(s.normal, l);
    if n_dot_l <= 0.0 {
        return 0.0;
    }
    let h = normalize(s.view_dir + l);
    let n_dot_v = max(dot(s.normal, s.view_dir), 1e-4);
    let alpha = s.roughness * s.roughness;
    let spec = pt_g1(n_dot_v, alpha) * D_GGX(max(dot(s.normal, h), 0.0), s.roughness) / (4.0 * n_dot_v);
    let diff = n_dot_l / 3.14159265;
    return p_spec * spec + (1.0 - p_spec) * diff;
}

/// Mode 1: the light arriving at a glossy primary surface along one sample of its reflection lobe. A surface rougher
/// than `max_rough` writes nothing (distance 0), and shading keeps the environment's specular there.
fn pt_reflect(coord: vec2<u32>, primary: RtHit, eye: vec3<f32>, seed: ptr<function, u32>) {
    let s = pt_surface(primary, -normalize(primary.pos - eye));
    var n = primary.normal;
    if !all(n == n) || dot(n, n) < 0.5 {
        n = vec3<f32>(0.0, 1.0, 0.0);
    }
    var dist = length(primary.pos - eye);
    if !(dist == dist) || dist > 1.0e6 {
        dist = 1.0;
    }
    let glossy = s.roughness <= pt.max_rough;
    textureStore(out_gbuf, coord, vec4<f32>(n, select(0.0, dist, glossy)));
    // The roughness rides in the albedo's alpha, for the filter (path_denoise.wgsl) and the fade at `max_rough`.
    textureStore(out_albedo, coord, vec4<f32>(1.0, 1.0, 1.0, s.roughness));
    if !glossy {
        textureStore(out_light, coord, vec4<f32>(0.0));
        return;
    }
    let basis = pt_basis(s.normal);
    let v_local = transpose(basis) * s.view_dir;
    let h = basis * pt_sample_vndf(v_local, s.roughness * s.roughness, vec2<f32>(pt_rand(seed), pt_rand(seed)));
    var l = reflect(-s.view_dir, h);
    if dot(l, s.normal) <= 0.0 {
        l = reflect(-s.view_dir, s.normal);
    }
    let hit = rt_trace(primary.pos + n * 0.02, l, 0.01, 1000.0);
    var radiance: vec3<f32>;
    if hit.hit {
        let hs = pt_surface(hit, -l);
        radiance = pt_direct(hs, hit.pos, hit.normal, seed) + hit.emissive;
        // The light bounced about the room at the hit, which the reflection cannot trace further: the environment's
        // diffuse, as the raster's own fallback has it.
        let kd = (vec3<f32>(1.0) - hs.f0) * (1.0 - hs.metallic);
        radiance += textureSampleLevel(env_cube, env_sampler, hit.normal, 5.0).rgb * hs.albedo * kd * light.ibl_intensity;
    } else {
        radiance = textureSampleLevel(env_cube, env_sampler, l, s.roughness * 5.0).rgb * light.ibl_intensity;
    }
    let safe = select(vec3<f32>(0.0), radiance, radiance == radiance);
    textureStore(out_light, coord, vec4<f32>(min(safe, vec3<f32>(60000.0)), 1.0));
}

@compute @workgroup_size(8, 8, 1)
fn path_trace(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(out_light);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(dims);
    var seed = (gid.x * 1973u + gid.y * 9277u + pt.frame * 26699u) | 1u;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    // The pixel's ray: through a point at middle depth. The renderer's projection is reversed and infinite, so depth
    // 0 unprojects to a point at infinity whose w comes out negative, which turned every ray round (17 Oct: the
    // surfaces faced away and no lamp lit them); 0.5 is a finite point in front of the eye in either convention.
    // The raster's own camera (global pool `view`), so the tracer works in exactly the space the scene is drawn in.
    let mid = view.inv_view_proj * vec4<f32>(ndc, 0.5, 1.0);
    var dir = normalize(mid.xyz / mid.w - view.camera_pos);
    var origin = view.camera_pos;

    let primary = pt_primary(vec2<i32>(gid.xy), origin, ndc, pt.mode == 1u);
    // Nothing traced here: the sky (the raster draws it), or in mode 0 a surface whose light is baked into it.
    if !primary.hit || (pt.mode == 0u && textureLoad(surface_extra, gid.xy, 0).x < 0.0) {
        textureStore(out_light, gid.xy, vec4<f32>(0.0, 0.0, 0.0, 0.0));
        textureStore(out_gbuf, gid.xy, vec4<f32>(0.0, 1.0, 0.0, 0.0));
        textureStore(out_albedo, gid.xy, vec4<f32>(1.0, 1.0, 1.0, 0.0));
        return;
    }

    if pt.mode == 1u {
        pt_reflect(gid.xy, primary, origin, &seed);
        return;
    }

    // From here the ray is the eye's true one to the surface it sees (`pt_primary`), and the surface is the raster's.
    dir = normalize(primary.pos - origin);
    var surface = pt_raster_surface(gid.xy, primary.normal, -dir);
    // The denoiser stops at edges where neighbours' normals differ, so it gets the smooth normal: the normal-mapped one
    // (the BRDF's) stopped it at every brick, stone and plank, which stayed noisy (F's check, 29 Oct).
    let primary_normal = primary.normal;
    // The primary surface's albedo the light is divided by: what it reflects of diffuse light and of specular.
    let demod = max(surface.albedo * (1.0 - surface.metallic) + surface.f0, vec3<f32>(0.04));
    var radiance = vec3<f32>(0.0);
    var throughput = vec3<f32>(1.0);
    var hit = primary;
    let bounces = clamp(pt.bounces, 1u, 8u);
    for (var b = 0u; b < bounces; b++) {
        let geo_n = hit.normal;
        if b > 0u {
            // At each surface the path reaches: light straight from the sun, the moon and a lamp, and its own glow.
            // (The primary's own direct light and glow are the raster's.)
            var add = throughput * (pt_direct(surface, hit.pos, geo_n, &seed) + hit.emissive);
            let l = pt_luma(add);
            if l > pt.firefly {
                add *= pt.firefly / l;
            }
            radiance += add;
        }
        // The next direction: one lobe, picked by its weight.
        let p_spec = pt_specular_chance(surface);
        let basis = pt_basis(surface.normal);
        var l: vec3<f32>;
        if pt_rand(&seed) < p_spec {
            let v_local = transpose(basis) * surface.view_dir;
            let h_local = pt_sample_vndf(v_local, surface.roughness * surface.roughness, vec2<f32>(pt_rand(&seed), pt_rand(&seed)));
            let h = basis * h_local;
            l = reflect(-surface.view_dir, h);
        } else {
            l = cosine_hemisphere(surface.normal, vec2<f32>(pt_rand(&seed), pt_rand(&seed)));
        }
        let pdf = pt_pdf(surface, l, p_spec);
        // A direction a normal map tilted under the surface itself goes nowhere.
        if pdf <= 1e-6 || dot(l, geo_n) <= 0.0 {
            break;
        }
        throughput *= evaluate_brdf(surface, l) / pdf;
        // Russian roulette after the second bounce.
        if b >= 1u {
            let keep = clamp(max(throughput.x, max(throughput.y, throughput.z)), 0.05, 0.95);
            if pt_rand(&seed) > keep {
                break;
            }
            throughput /= keep;
        }
        origin = hit.pos + geo_n * 0.02;
        dir = l;
        hit = rt_trace(origin, dir, 0.01, 1000.0);
        if !hit.hit {
            // The sky the primary itself sees, its fill and its reflection, is the raster's (shading.wgsl keeps both
            // under the tracer), so only sky reached after a bounce off the scene is added here.
            if b == 0u {
                break;
            }
            var sky = throughput * textureSampleLevel(env_cube, env_sampler, dir, 3.0).rgb * light.ibl_intensity;
            let sl = pt_luma(sky);
            if sl > pt.firefly {
                sky *= pt.firefly / sl;
            }
            radiance += sky;
            break;
        }
        surface = pt_surface(hit, -dir);
    }

    // Nothing that is not a finite number leaves here: one NaN in the history would spread through the filter
    // to the whole picture in a second or two.
    var n = primary_normal;
    if !all(n == n) || dot(n, n) < 0.5 {
        n = vec3<f32>(0.0, 1.0, 0.0);
    }
    var dist = length(primary.pos - view.camera_pos);
    if !(dist == dist) || dist > 1.0e6 {
        dist = 1.0;
    }
    let safe = select(vec3<f32>(0.0), radiance, radiance == radiance);
    textureStore(out_light, gid.xy, vec4<f32>(min(safe / demod, vec3<f32>(60000.0)), 1.0));
    textureStore(out_gbuf, gid.xy, vec4<f32>(n, dist));
    // Multiplied back after the filter, so their detail stays sharp: the albedo, the material's own occlusion (a
    // crevice the geometry does not have, which no ray can find) and the fog's transmittance (its light is already in
    // the raster's picture).
    let extra = textureLoad(surface_extra, gid.xy, 0);
    let back = demod * saturate(extra.x) * saturate(extra.y);
    textureStore(out_albedo, gid.xy, vec4<f32>(select(vec3<f32>(0.0), back, back == back), 1.0));
}
