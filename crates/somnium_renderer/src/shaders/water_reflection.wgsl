enable wgpu_ray_query;

// Phase VV — Halcyon: hardware ray-traced water reflections.
//
// Traces a half-resolution mirror / GGX ray from the water G-buffer against
// the scene TLAS, shades the hit with sun (cascade sample) plus IBL, then
// temporally accumulates. The water fragment shader bilateral-upsamples this
// target and blends it with screen-space tracing on confidence.
//
// Concatenated with `rt_hit.wgsl`, `global_pool.wgsl`, `brdf.wgsl`,
// `hextile.wgsl`, and `terrain_material.wgsl`.

struct ReflectParams {
    inv_view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    camera_pos: vec3<f32>,
    frame: u32,
    inv_half_res: vec2<f32>,
    history_valid: f32,
    rt_strength: f32,
    roughness_skip: f32,
    enabled: f32,
    refract_enabled: f32,
    _pad1: f32,
}

@group(1) @binding(0) var accel: acceleration_structure;
@group(1) @binding(1) var water_surface: texture_2d<f32>;
@group(1) @binding(2) var water_roughness: texture_2d<f32>;
@group(1) @binding(3) var velocity_tex: texture_2d<f32>;
@group(1) @binding(4) var env_cube: texture_cube<f32>;
@group(1) @binding(5) var env_sampler: sampler;
@group(1) @binding(6) var shadow_atlas: texture_depth_2d;
@group(1) @binding(7) var shadow_sampler: sampler_comparison;
@group(1) @binding(8) var history_tex: texture_2d_array<f32>;
@group(1) @binding(9) var out_tex: texture_storage_2d_array<rgba16float, write>;
@group(1) @binding(10) var<uniform> params: ReflectParams;
@group(1) @binding(11) var default_sampler: sampler;

const ENV_MAX_MIP: f32 = 5.0;
const IOR_WATER: f32 = 1.333;

fn reflect_rand(seed: ptr<function, u32>) -> f32 {
    *seed = *seed * 747796405u + 2891336453u;
    var x = *seed;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return f32((x >> 22u) ^ x) / 4294967295.0;
}

fn reconstruct_normal_toward(encoded: vec2<f32>, toward: vec3<f32>) -> vec3<f32> {
    let xz = encoded * 2.0 - 1.0;
    let y = sqrt(max(1.0 - dot(xz, xz), 0.0));
    let above = normalize(vec3<f32>(xz.x, y, xz.y));
    let below = normalize(vec3<f32>(xz.x, -y, xz.y));
    // The prepass orients the water normal toward the camera. x/z alone do
    // not contain the sign of y, so choose the hemisphere that preserves that
    // convention instead of always reconstructing an upward normal.
    return select(below, above, dot(above, toward) >= dot(below, toward));
}

fn world_from_uv_view_depth(uv: vec2<f32>, view_depth: f32) -> vec3<f32> {
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 1.0, 1.0);
    let far_h = params.inv_view_proj * ndc;
    let far_w = far_h.xyz / far_h.w;
    let ray = normalize(far_w - params.camera_pos);
    let ray_view = (params.view * vec4<f32>(ray, 0.0)).xyz;
    let t = view_depth / max(-ray_view.z, 1e-4);
    return params.camera_pos + ray * t;
}

fn get_cascade_index(view_depth: f32) -> u32 {
    if view_depth < light.cascade_splits.x { return 0u; }
    if view_depth < light.cascade_splits.y { return 1u; }
    if view_depth < light.cascade_splits.z { return 2u; }
    return 3u;
}

fn atlas_uv(cascade: u32, uv: vec2<f32>) -> vec2<f32> {
    let offsets = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0), vec2<f32>(0.5, 0.0),
        vec2<f32>(0.0, 0.5), vec2<f32>(0.5, 0.5),
    );
    return uv * 0.5 + offsets[cascade];
}

/// Cascade sample at the hit, not a second shadow ray. Measured cheaper than a
/// visibility query at reflection resolution, and the cascade already agrees
/// with the raster path the blend has to match (VV-D).
fn sample_cascade_shadow(world_pos: vec3<f32>, view_depth: f32) -> f32 {
    let cascade = get_cascade_index(view_depth);
    let clip = light.view_proj[cascade] * vec4<f32>(world_pos, 1.0);
    if clip.w <= 0.0 {
        return 1.0;
    }
    let ndc = clip.xyz / clip.w;
    // Validate against the cascade's local tile before converting to atlas
    // space. Every out-of-frustum coordinate can look valid after `* 0.5 +
    // tile_offset`, which made unrelated atlas texels become dancing black
    // shadow blotches in rough reflections.
    let local_uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if any(local_uv < vec2<f32>(0.0)) || any(local_uv > vec2<f32>(1.0))
        || ndc.z < 0.0 || ndc.z > 1.0
    {
        return 1.0;
    }
    let uv = atlas_uv(cascade, local_uv);
    return textureSampleCompareLevel(shadow_atlas, shadow_sampler, uv, ndc.z);
}

fn orthonormal_basis(n: vec3<f32>) -> mat3x3<f32> {
    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.99);
    let t = normalize(cross(up, n));
    let b = cross(n, t);
    return mat3x3<f32>(t, b, n);
}

/// Karis / UE4 GGX importance sampling of the half-vector (VV-E).
fn sample_ggx_h(xi: vec2<f32>, n: vec3<f32>, roughness: f32) -> vec3<f32> {
    let a = roughness * roughness;
    let phi = 6.28318530718 * xi.x;
    let cos_theta = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
    let sin_theta = sqrt(max(1.0 - cos_theta * cos_theta, 0.0));
    let h_local = vec3<f32>(sin_theta * cos(phi), sin_theta * sin(phi), cos_theta);
    return normalize(orthonormal_basis(n) * h_local);
}

fn shade_hit(hit: RtHit, wo: vec3<f32>) -> vec3<f32> {
    var surface: Surface;
    surface.albedo = hit.albedo;
    surface.roughness = hit.roughness;
    surface.metallic = hit.metallic;
    surface.normal = hit.normal;
    surface.view_dir = wo;
    surface.f0 = mix(vec3<f32>(0.04), hit.albedo, hit.metallic);
    surface.occlusion = 1.0;
    surface.bent_normal = hit.normal;

    let view_depth = max(-(params.view * vec4<f32>(hit.pos, 1.0)).z, 0.05);
    let shadow = sample_cascade_shadow(hit.pos, view_depth);
    let sun_l = normalize(light.direction);
    let sun = evaluate_brdf(surface, sun_l) * light.color * shadow;

    let n_dot_v = max(dot(hit.normal, wo), 1e-4);
    let irradiance = textureSampleLevel(env_cube, env_sampler, hit.normal, ENV_MAX_MIP).rgb;
    let kd = (vec3<f32>(1.0) - surface.f0) * (1.0 - hit.metallic);
    let diffuse = irradiance * hit.albedo * kd;
    let r = reflect(-wo, hit.normal);
    let prefiltered = textureSampleLevel(
        env_cube,
        env_sampler,
        r,
        hit.roughness * ENV_MAX_MIP,
    ).rgb;
    let spec_f = surface.f0 + (vec3<f32>(1.0) - surface.f0) * pow(1.0 - n_dot_v, 5.0);
    let specular = prefiltered * spec_f;
    let ibl = (diffuse + specular) * light.ibl_intensity;

    return sun + ibl + hit.emissive;
}

struct TemporalResult {
    color: vec4<f32>,
    // luminance mean, luminance squared mean, sample count, hit distance
    moments: vec4<f32>,
}

fn reflect_luma(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn temporal_seed(current: vec4<f32>, hit_distance: f32) -> TemporalResult {
    let luma = reflect_luma(current.rgb);
    return TemporalResult(current, vec4<f32>(luma, luma * luma, 1.0, hit_distance));
}

fn accumulate(
    uv: vec2<f32>,
    load_coord: vec2<i32>,
    full_dims: vec2<i32>,
    view_depth: f32,
    roughness: f32,
    normal: vec3<f32>,
    layer: i32,
    current: vec4<f32>,
    hit_distance: f32,
) -> TemporalResult {
    if params.history_valid < 0.5 {
        return temporal_seed(current, hit_distance);
    }
    let velocity = textureLoad(velocity_tex, load_coord, 0).xy;
    let prev_uv = uv + velocity;
    if any(prev_uv < vec2<f32>(0.0)) || any(prev_uv > vec2<f32>(1.0)) {
        return temporal_seed(current, hit_distance);
    }
    let prev = textureSampleLevel(history_tex, default_sampler, prev_uv, layer, 0.0);
    // Layer 2 is the previous frame's water guide, written alongside the
    // reflection result. Sampling today's water G-buffer at prev_uv compares
    // two current-frame pixels and cannot reject disocclusions.
    let guide = textureSampleLevel(history_tex, default_sampler, prev_uv, 2, 0.0);
    let prev_normal = reconstruct_normal_toward(guide.rg, normal);
    let depth_ok = abs(guide.b - view_depth) < max(0.025 * view_depth, 0.2);
    let normal_ok = dot(prev_normal, normal) > 0.92;
    let roughness_ok = abs(guide.a - roughness) < 0.12;
    let finite_ok = all(prev.rgb == prev.rgb);
    let prev_moments = textureSampleLevel(history_tex, default_sampler, prev_uv, 3 + layer, 0.0);
    let distance_ok = (hit_distance <= 0.0 && prev_moments.a <= 0.0)
        || (hit_distance > 0.0 && prev_moments.a > 0.0
            && abs(prev_moments.a - hit_distance) < max(hit_distance * 0.2, 0.75));
    let valid = guide.a > 0.0 && depth_ok && normal_ok && roughness_ok
        && finite_ok && distance_ok && all(prev_moments == prev_moments);

    if !valid {
        return temporal_seed(current, hit_distance);
    }

    // Variance-guided clipping keeps a rare bright/dark hit from becoming a
    // persistent dancing blotch. The current sample supplies the local centre;
    // prior moments supply a stable scale when one ray cannot estimate a 3x3
    // current-frame neighbourhood on its own.
    let variance = max(prev_moments.y - prev_moments.x * prev_moments.x, 0.0);
    let sigma = sqrt(variance);
    let current_luma = reflect_luma(current.rgb);
    let radius = max(2.0 * sigma, 0.1 + current_luma * 0.5);
    let history_luma = max(reflect_luma(prev.rgb), 1e-5);
    let clipped_luma = clamp(history_luma, max(current_luma - radius, 0.0), current_luma + radius);
    let stable_prev = vec4<f32>(max(prev.rgb, vec3<f32>(0.0)) * clipped_luma / history_luma, prev.a);

    let old_count = clamp(prev_moments.z, 1.0, 32.0);
    let new_count = min(old_count + 1.0, 32.0);
    let current_weight = 1.0 / new_count;
    let max_history = mix(0.84, 0.93, roughness);
    let history_weight = min(1.0 - current_weight, max_history);
    let color = mix(current, stable_prev, history_weight);
    let mean = mix(prev_moments.x, current_luma, current_weight);
    let mean2 = mix(prev_moments.y, current_luma * current_luma, current_weight);
    return TemporalResult(color, vec4<f32>(mean, mean2, new_count, hit_distance));
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let half_dims = textureDimensions(out_tex);
    if gid.x >= half_dims.x || gid.y >= half_dims.y {
        return;
    }
    let half_coord = vec2<i32>(gid.xy);
    let full_dims = vec2<i32>(textureDimensions(water_surface));
    let full_coord = vec2<i32>(
        i32(gid.x * 2u) + 1,
        i32(gid.y * 2u) + 1,
    );
    let load_coord = min(full_coord, full_dims - vec2<i32>(1));
    let uv = (vec2<f32>(load_coord) + 0.5) / vec2<f32>(full_dims);

    if params.enabled < 0.5 && params.refract_enabled < 0.5 {
        textureStore(out_tex, half_coord, 0, vec4<f32>(0.0));
        textureStore(out_tex, half_coord, 1, vec4<f32>(0.0));
        textureStore(out_tex, half_coord, 2, vec4<f32>(0.0));
        textureStore(out_tex, half_coord, 3, vec4<f32>(0.0));
        textureStore(out_tex, half_coord, 4, vec4<f32>(0.0));
        return;
    }

    let g = textureLoad(water_surface, load_coord, 0);
    let coverage = g.a;
    if coverage < 0.01 {
        textureStore(out_tex, half_coord, 0, vec4<f32>(0.0));
        textureStore(out_tex, half_coord, 1, vec4<f32>(0.0));
        textureStore(out_tex, half_coord, 2, vec4<f32>(0.0));
        textureStore(out_tex, half_coord, 3, vec4<f32>(0.0));
        textureStore(out_tex, half_coord, 4, vec4<f32>(0.0));
        return;
    }

    let roughness = clamp(textureLoad(water_roughness, load_coord, 0).r, 0.04, 1.0);
    let view_depth = g.b;
    let world = world_from_uv_view_depth(uv, view_depth);
    let wo = normalize(params.camera_pos - world);
    let n = reconstruct_normal_toward(g.rg, wo);

    var reflect_result = vec4<f32>(0.0);
    var reflect_distance = 0.0;
    var reflect_moments = vec4<f32>(0.0);
    if params.enabled >= 0.5 {
        if roughness >= params.roughness_skip {
            let mirror = reflect(-wo, n);
            let env = textureSampleLevel(env_cube, env_sampler, mirror, roughness * ENV_MAX_MIP).rgb
                * light.ibl_intensity;
            reflect_result = vec4<f32>(env, 0.0);
        } else {
            var seed = gid.x * 1973u + gid.y * 9277u + params.frame * 26699u;
            var dir: vec3<f32>;
            if roughness < 0.08 {
                dir = reflect(-wo, n);
            } else {
                let xi = vec2<f32>(reflect_rand(&seed), reflect_rand(&seed));
                let h = sample_ggx_h(xi, n, roughness);
                dir = reflect(-wo, h);
                if dot(dir, n) <= 0.0 {
                    dir = reflect(-wo, n);
                }
            }

            let origin = world + n * 0.05;
            let hit = rt_trace(origin, dir, 0.05, 4000.0);
            if hit.hit {
                reflect_distance = distance(hit.pos, origin);
                let lit = shade_hit(hit, -dir);
                if all(lit == lit) {
                    reflect_result = vec4<f32>(max(lit, vec3<f32>(0.0)), 1.0);
                } else {
                    reflect_result = vec4<f32>(
                        textureSampleLevel(env_cube, env_sampler, dir, roughness * ENV_MAX_MIP).rgb
                            * light.ibl_intensity,
                        0.0,
                    );
                }
            } else {
                reflect_result = vec4<f32>(
                    textureSampleLevel(env_cube, env_sampler, dir, roughness * ENV_MAX_MIP).rgb
                        * light.ibl_intensity,
                    0.0,
                );
            }
            let temporal = accumulate(
                uv, load_coord, full_dims, view_depth, roughness, n, 0,
                reflect_result, reflect_distance,
            );
            reflect_result = temporal.color;
            reflect_moments = temporal.moments;
        }
    }
    textureStore(out_tex, half_coord, 0, reflect_result);
    if reflect_moments.z <= 0.0 {
        reflect_moments = temporal_seed(reflect_result, reflect_distance).moments;
    }
    textureStore(out_tex, half_coord, 3, reflect_moments);

    var refract_result = vec4<f32>(0.0);
    var refract_distance = 0.0;
    var refract_moments = vec4<f32>(0.0);
    if params.refract_enabled >= 0.5 && roughness < params.roughness_skip {
        // Air→water from above (eta < 1); water→air from below (Snell window).
        let from_below = params.camera_pos.y < world.y;
        let eta = select(1.0 / IOR_WATER, IOR_WATER, from_below);
        var rdir = refract(-wo, n, eta);
        if dot(rdir, rdir) > 1e-8 {
            rdir = normalize(rdir);
            let rorigin = world - n * 0.05;
            let rhit = rt_trace(rorigin, rdir, 0.05, 4000.0);
            if rhit.hit {
                refract_distance = distance(rhit.pos, rorigin);
                let lit = shade_hit(rhit, -rdir);
                if all(lit == lit) {
                    refract_result = vec4<f32>(max(lit, vec3<f32>(0.0)), 1.0);
                }
            }
            let temporal = accumulate(
                uv, load_coord, full_dims, view_depth, roughness, n, 1,
                refract_result, refract_distance,
            );
            refract_result = temporal.color;
            refract_moments = temporal.moments;
        }
    }
    textureStore(out_tex, half_coord, 1, refract_result);
    if refract_moments.z <= 0.0 {
        refract_moments = temporal_seed(refract_result, refract_distance).moments;
    }
    textureStore(out_tex, half_coord, 4, refract_moments);
    textureStore(
        out_tex,
        half_coord,
        2,
        vec4<f32>(n.xz * 0.5 + vec2<f32>(0.5), view_depth, roughness),
    );
}
