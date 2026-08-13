enable wgpu_ray_query;

// Phase 24M/N/O: world-space radiance cache, scene specular GI, path tracer.
// Concatenated with rt_hit.wgsl + global_pool + brdf + sampling + atmosphere
// + hextile + terrain_material, same as ReSTIR GI.

struct ExtraParams {
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    frame: u32,
    origin: vec3<f32>,
    cell_size: f32,
    flags: u32,
    intensity: f32,
    spec_rough: f32,
    path_bounces: u32,
    inv_res: vec2<f32>,
    history_valid: f32,
    half_cells: f32,
}

@group(1) @binding(0) var accel: acceleration_structure;
@group(1) @binding(1) var depth_tex: texture_depth_2d;
@group(1) @binding(2) var vis_tex: texture_2d<u32>;
@group(1) @binding(3) var gi_tex: texture_2d<f32>;
@group(1) @binding(4) var env_cube: texture_cube<f32>;
@group(1) @binding(5) var env_sampler: sampler;
@group(1) @binding(6) var cache_history: texture_3d<f32>;
@group(1) @binding(7) var cache_out: texture_storage_3d<rgba16float, write>;
@group(1) @binding(8) var aux_history: texture_2d<f32>;
@group(1) @binding(9) var aux_out: texture_storage_2d<rgba16float, write>;
@group(1) @binding(10) var<uniform> extra: ExtraParams;
@group(1) @binding(11) var default_sampler: sampler;
@group(1) @binding(12) var<storage, read_write> sh_probes: array<vec4<f32>>;

fn extra_rand(seed: ptr<function, u32>) -> f32 {
    *seed = *seed * 747796405u + 2891336453u;
    var x = *seed;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return f32((x >> 22u) ^ x) / 4294967295.0;
}

fn reconstruct_world(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world = extra.inv_view_proj * ndc;
    return world.xyz / world.w;
}

fn cache_coord(pos: vec3<f32>, dims: vec3<u32>) -> vec3<i32> {
    let local = (pos - extra.origin) / extra.cell_size + vec3<f32>(extra.half_cells);
    return vec3<i32>(clamp(local, vec3<f32>(0.0), vec3<f32>(dims) - vec3<f32>(1.0)));
}

fn vis_normal(coord: vec2<i32>) -> vec3<f32> {
    let vis = textureLoad(vis_tex, coord, 0);
    if vis.x == 0u {
        return vec3<f32>(0.0, 1.0, 0.0);
    }
    let inst = instances[vis.x - 1u];
    let base = inst.index_offset + vis.y * 3u;
    let a = vertices[inst.vertex_offset + indices[base + 0u]];
    let b = vertices[inst.vertex_offset + indices[base + 1u]];
    let c = vertices[inst.vertex_offset + indices[base + 2u]];
    let p0 = (inst.model * vec4<f32>(a.pos_x, a.pos_y, a.pos_z, 1.0)).xyz;
    let p1 = (inst.model * vec4<f32>(b.pos_x, b.pos_y, b.pos_z, 1.0)).xyz;
    let p2 = (inst.model * vec4<f32>(c.pos_x, c.pos_y, c.pos_z, 1.0)).xyz;
    var n = normalize(cross(p1 - p0, p2 - p0));
    if dot(n, extra.camera_pos - p0) < 0.0 {
        n = -n;
    }
    return n;
}

// ── 24M: splat this frame's GI into the world cache ──────────────────────────

@compute @workgroup_size(8, 8, 1)
fn cache_splat(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(cache_out);
    if gid.x >= dims.x || gid.y >= dims.y || gid.z >= dims.z {
        return;
    }
    let uvw = (vec3<f32>(gid) + 0.5) / vec3<f32>(dims);
    var hist = vec4<f32>(0.0);
    if extra.history_valid > 0.5 {
        hist = textureSampleLevel(cache_history, default_sampler, uvw, 0.0);
    }
    textureStore(cache_out, gid, vec4<f32>(hist.rgb * 0.92, hist.a * 0.92));
}

@compute @workgroup_size(8, 8, 1)
fn cache_from_screen(@builtin(global_invocation_id) gid: vec3<u32>) {
    let screen = textureDimensions(depth_tex);
    if gid.x >= screen.x || gid.y >= screen.y {
        return;
    }
    let coord = vec2<i32>(gid.xy);
    let depth = textureLoad(depth_tex, coord, 0);
    if depth >= 1.0 {
        return;
    }
    let uv = (vec2<f32>(coord) + 0.5) * extra.inv_res;
    let pos = reconstruct_world(uv, depth);
    let gi = textureLoad(gi_tex, coord, 0);
    if gi.a < 0.5 && (extra.flags & 16u) == 0u {
        return;
    }
    var rad = gi.rgb;
    if gi.a < 0.5 {
        rad = textureSampleLevel(env_cube, env_sampler, vis_normal(coord), 5.0).rgb * extra.intensity;
    }
    let cell = cache_coord(pos, textureDimensions(cache_out));
    // Races are acceptable for a cache: last writer wins, temporal blend
    // in cache_splat damps the flicker.
    textureStore(cache_out, vec3<u32>(cell), vec4<f32>(rad, 1.0));
}

// ── 24N: scene-wide specular GI (SSR then RT) ────────────────────────────────

@compute @workgroup_size(8, 8, 1)
fn specular_gi(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(aux_out);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(dims);
    let full = vec2<i32>(uv * vec2<f32>(textureDimensions(depth_tex)));
    let depth = textureLoad(depth_tex, full, 0);
    if depth >= 1.0 {
        textureStore(aux_out, gid.xy, vec4<f32>(0.0));
        return;
    }
    let pos = reconstruct_world(uv, depth);
    let n = vis_normal(full);
    let v = normalize(extra.camera_pos - pos);
    var dir = reflect(-v, n);
    if extra.spec_rough > 0.04 {
        var seed = gid.x * 1973u + gid.y * 9277u + extra.frame * 26699u;
        dir = normalize(dir + (vec3<f32>(extra_rand(&seed), extra_rand(&seed), extra_rand(&seed)) - 0.5) * extra.spec_rough);
    }

    var radiance = vec3<f32>(0.0);
    var conf = 0.0;

    // Short SSR: if the mirror lands on screen with nearby depth, take it.
    var ss_hit = false;
    var t_ss = 0.4;
    for (var i = 0u; i < 12u; i++) {
        let p = pos + dir * t_ss;
        let clip = view.view_proj * vec4<f32>(p, 1.0);
        if clip.w <= 0.0 { break; }
        let ndc = clip.xy / clip.w;
        let suv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if any(suv < vec2<f32>(0.0)) || any(suv > vec2<f32>(1.0)) { break; }
        let scoord = vec2<i32>(suv * vec2<f32>(textureDimensions(depth_tex)));
        let sd = textureLoad(depth_tex, scoord, 0);
        let sw = reconstruct_world(suv, sd);
        if sd < 1.0 && abs(length(sw - extra.camera_pos) - length(p - extra.camera_pos)) < t_ss * 0.15 {
            radiance = textureLoad(gi_tex, scoord, 0).rgb
                + textureSampleLevel(env_cube, env_sampler, dir, extra.spec_rough * 5.0).rgb * 0.15;
            conf = 0.65;
            ss_hit = true;
            break;
        }
        t_ss *= 1.45;
    }

    if !ss_hit && (extra.flags & 2u) != 0u {
        let hit = rt_trace(pos + n * 0.05, dir, 0.05, 400.0);
        if hit.hit {
            radiance = hit.albedo * max(dot(hit.normal, normalize(light.direction)), 0.0) * light.color
                + hit.emissive;
            conf = 1.0;
        } else {
            radiance = textureSampleLevel(env_cube, env_sampler, dir, extra.spec_rough * 5.0).rgb;
            conf = 0.35;
        }
    }

    if extra.history_valid > 0.5 {
        let px = 1.0 / vec2<f32>(textureDimensions(aux_out));
        var acc = textureSampleLevel(aux_history, default_sampler, uv, 0.0);
        acc += textureSampleLevel(aux_history, default_sampler, uv + vec2<f32>(px.x, 0.0), 0.0);
        acc += textureSampleLevel(aux_history, default_sampler, uv - vec2<f32>(px.x, 0.0), 0.0);
        acc += textureSampleLevel(aux_history, default_sampler, uv + vec2<f32>(0.0, px.y), 0.0);
        acc += textureSampleLevel(aux_history, default_sampler, uv - vec2<f32>(0.0, px.y), 0.0);
        acc *= 0.2;
        radiance = mix(radiance, acc.rgb, 0.85);
        conf = max(conf, acc.a * 0.85);
    }
    textureStore(aux_out, gid.xy, vec4<f32>(radiance * extra.intensity, conf));
}

// ── 24O: unbiased path tracer (accumulate, not in the default frame) ─────────

@compute @workgroup_size(8, 8, 1)
fn path_trace(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(aux_out);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(dims);
    var seed = gid.x * 1103515245u + gid.y * 134775813u + extra.frame * 214013u + 1u;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let near = extra.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    let far = extra.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    var origin = extra.camera_pos;
    var dir = normalize(far.xyz / far.w - near.xyz / near.w);
    var throughput = vec3<f32>(1.0);
    var radiance = vec3<f32>(0.0);

    let bounces = extra.path_bounces;
    for (var b = 0u; b < bounces; b++) {
        let hit = rt_trace(origin, dir, 0.02, 500.0);
        if !hit.hit {
            radiance += throughput * textureSampleLevel(env_cube, env_sampler, dir, 0.0).rgb;
            break;
        }
        radiance += throughput * hit.emissive;
        let l = normalize(light.direction);
        let ndl = max(dot(hit.normal, l), 0.0);
        if ndl > 0.0 {
            let shadow = rt_trace(hit.pos + hit.normal * 0.03, l, 0.03, 4000.0);
            if !shadow.hit {
                radiance += throughput * hit.albedo * (1.0 / 3.14159265) * light.color * ndl;
            }
        }
        var u1 = extra_rand(&seed);
        var u2 = extra_rand(&seed);
        let r = sqrt(u1);
        let phi = 6.2831853 * u2;
        let hemi = vec3<f32>(r * cos(phi), sqrt(max(1.0 - u1, 0.0)), r * sin(phi));
        var t = vec3<f32>(1.0, 0.0, 0.0);
        if abs(hit.normal.y) < 0.9 {
            t = vec3<f32>(0.0, 1.0, 0.0);
        }
        t = normalize(cross(t, hit.normal));
        let btan = cross(hit.normal, t);
        dir = normalize(t * hemi.x + hit.normal * hemi.y + btan * hemi.z);
        throughput *= hit.albedo;
        origin = hit.pos + hit.normal * 0.03;
        if max(throughput.x, max(throughput.y, throughput.z)) < 0.01 {
            break;
        }
    }

    var outc = vec4<f32>(radiance, 1.0);
    if extra.history_valid > 0.5 {
        let prev = textureSampleLevel(aux_history, default_sampler, uv, 0.0);
        let f = 1.0 / (f32(extra.frame % 1024u) + 1.0);
        outc = vec4<f32>(mix(prev.rgb, radiance, max(f, 0.02)), 1.0);
    }
    textureStore(aux_out, gid.xy, outc);
}

const PROBE_GRID: u32 = 4u;
const SH_COEFFS: u32 = 9u;
const SH_SAMPLES: u32 = 32u;

fn sh_y(n: vec3<f32>) -> array<f32, 9> {
    var y: array<f32, 9>;
    y[0] = 0.282095;
    y[1] = 0.488603 * n.y;
    y[2] = 0.488603 * n.z;
    y[3] = 0.488603 * n.x;
    y[4] = 1.092548 * n.x * n.y;
    y[5] = 1.092548 * n.y * n.z;
    y[6] = 0.315392 * (3.0 * n.z * n.z - 1.0);
    y[7] = 1.092548 * n.x * n.z;
    y[8] = 0.546274 * (n.x * n.x - n.y * n.y);
    return y;
}

fn fibonacci_dir(i: u32, n: u32) -> vec3<f32> {
    let z = 1.0 - 2.0 * (f32(i) + 0.5) / f32(n);
    let r = sqrt(max(1.0 - z * z, 0.0));
    let phi = 2.399963229728653 * f32(i);
    return vec3<f32>(cos(phi) * r, sin(phi) * r, z);
}

@compute @workgroup_size(4, 4, 4)
fn bake_probes(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= PROBE_GRID || gid.y >= PROBE_GRID || gid.z >= PROBE_GRID {
        return;
    }
    let uvw = (vec3<f32>(gid) + 0.5) / f32(PROBE_GRID);
    let pos = extra.origin + (uvw * extra.half_cells * 2.0 - extra.half_cells) * extra.cell_size;
    var sh0 = vec3<f32>(0.0);
    var sh1 = vec3<f32>(0.0);
    var sh2 = vec3<f32>(0.0);
    var sh3 = vec3<f32>(0.0);
    var sh4 = vec3<f32>(0.0);
    var sh5 = vec3<f32>(0.0);
    var sh6 = vec3<f32>(0.0);
    var sh7 = vec3<f32>(0.0);
    var sh8 = vec3<f32>(0.0);
    let weight = 12.5663706144 / f32(SH_SAMPLES);
    for (var i = 0u; i < SH_SAMPLES; i++) {
        let dir = fibonacci_dir(i, SH_SAMPLES);
        var col = textureSampleLevel(env_cube, env_sampler, dir, 4.0).rgb;
        if (extra.flags & 1u) != 0u {
            let local = (pos - extra.origin) / extra.cell_size + vec3<f32>(extra.half_cells);
            let dims = vec3<f32>(textureDimensions(cache_history));
            let uvw_c = clamp(local / max(dims, vec3<f32>(1.0)), vec3<f32>(0.0), vec3<f32>(1.0));
            col += textureSampleLevel(cache_history, default_sampler, uvw_c, 0.0).rgb;
        }
        let y = sh_y(dir);
        let wcol = col * weight * extra.intensity;
        sh0 += wcol * y[0];
        sh1 += wcol * y[1];
        sh2 += wcol * y[2];
        sh3 += wcol * y[3];
        sh4 += wcol * y[4];
        sh5 += wcol * y[5];
        sh6 += wcol * y[6];
        sh7 += wcol * y[7];
        sh8 += wcol * y[8];
    }
    let base = (gid.x + gid.y * PROBE_GRID + gid.z * PROBE_GRID * PROBE_GRID) * SH_COEFFS;
    sh_probes[base + 0u] = vec4<f32>(sh0, 1.0);
    sh_probes[base + 1u] = vec4<f32>(sh1, 1.0);
    sh_probes[base + 2u] = vec4<f32>(sh2, 1.0);
    sh_probes[base + 3u] = vec4<f32>(sh3, 1.0);
    sh_probes[base + 4u] = vec4<f32>(sh4, 1.0);
    sh_probes[base + 5u] = vec4<f32>(sh5, 1.0);
    sh_probes[base + 6u] = vec4<f32>(sh6, 1.0);
    sh_probes[base + 7u] = vec4<f32>(sh7, 1.0);
    sh_probes[base + 8u] = vec4<f32>(sh8, 1.0);
}
