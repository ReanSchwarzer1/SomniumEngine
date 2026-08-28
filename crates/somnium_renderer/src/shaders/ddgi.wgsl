// MORROWIND-AB: portable SDF-backed diffuse probe updates.
// Intentionally contains no ray-query extension or acceleration structure.

struct DdgiParams {
    origin: vec3<f32>,
    spacing: f32,
    light_dir: vec3<f32>,
    intensity: f32,
    light_color: vec3<f32>,
    hysteresis: f32,
    update_start: u32,
    update_budget: u32,
    valid_lo: u32,
    valid_hi: u32,
}

@group(0) @binding(0) var scene_sdf: texture_3d<f32>;
@group(0) @binding(1) var env_cube: texture_cube<f32>;
@group(0) @binding(2) var env_sampler: sampler;
@group(0) @binding(3) var<uniform> ddgi: DdgiParams;
@group(0) @binding(4) var<storage, read_write> sh_probes: array<vec4<f32>>;

const PROBE_GRID: u32 = 4u;
const PROBE_COUNT: u32 = 64u;
const SH_COEFFS: u32 = 9u;
const RAYS: u32 = 64u;
const PI4: f32 = 12.5663706144;

fn fibonacci_dir(i: u32, rotation: f32) -> vec3<f32> {
    let z = 1.0 - 2.0 * (f32(i) + 0.5) / f32(RAYS);
    let r = sqrt(max(1.0 - z * z, 0.0));
    let phi = 2.39996323 * f32(i) + rotation;
    return vec3<f32>(cos(phi) * r, sin(phi) * r, z);
}

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

fn sdf_uvw(p: vec3<f32>) -> vec3<f32> {
    let dims = vec3<f32>(textureDimensions(scene_sdf));
    return (p - ddgi.origin) / (ddgi.spacing * dims) + vec3<f32>(0.5);
}

fn sdf_at(p: vec3<f32>) -> f32 {
    let uvw = sdf_uvw(p);
    if any(uvw <= vec3<f32>(0.0)) || any(uvw >= vec3<f32>(1.0)) {
        return ddgi.spacing * 8.0;
    }
    return abs(textureSampleLevel(scene_sdf, env_sampler, uvw, 0.0).a);
}

fn sun_visible(origin: vec3<f32>) -> f32 {
    let dir = normalize(ddgi.light_dir);
    var t = ddgi.spacing * 0.5;
    for (var step = 0u; step < 20u; step++) {
        let d = sdf_at(origin + dir * t);
        if d < ddgi.spacing * 0.25 { return 0.0; }
        t += max(d, ddgi.spacing * 0.2);
        if t > ddgi.spacing * 24.0 { break; }
    }
    return 1.0;
}

fn sdf_normal(p: vec3<f32>) -> vec3<f32> {
    let e = ddgi.spacing * 0.5;
    return normalize(vec3<f32>(
        sdf_at(p + vec3<f32>(e, 0.0, 0.0)) - sdf_at(p - vec3<f32>(e, 0.0, 0.0)),
        sdf_at(p + vec3<f32>(0.0, e, 0.0)) - sdf_at(p - vec3<f32>(0.0, e, 0.0)),
        sdf_at(p + vec3<f32>(0.0, 0.0, e)) - sdf_at(p - vec3<f32>(0.0, 0.0, e))));
}

fn trace_radiance(origin: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    var t = ddgi.spacing * 0.35;
    for (var step = 0u; step < 32u; step++) {
        let p = origin + dir * t;
        let d = sdf_at(p);
        if d < ddgi.spacing * 0.3 {
            let n = sdf_normal(p);
            let payload = textureSampleLevel(scene_sdf, env_sampler, sdf_uvw(p), 0.0);
            let albedo = max(payload.rgb, vec3<f32>(0.04));
            let sun = max(dot(n, normalize(ddgi.light_dir)), 0.0)
                * sun_visible(p + n * ddgi.spacing * 0.4) * ddgi.light_color;
            let sky = textureSampleLevel(env_cube, env_sampler, n, 5.0).rgb;
            return (sun + sky * 0.25) * albedo * 0.318309886;
        }
        t += max(d, ddgi.spacing * 0.2);
        if t > ddgi.spacing * 32.0 { break; }
    }
    // Base IBL already supplies sky irradiance at the shaded point. Probes
    // contribute only bounced light from geometry, so misses carry no energy.
    return vec3<f32>(0.0);
}

@compute @workgroup_size(1, 1, 1)
fn update_probes(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= ddgi.update_budget { return; }
    let probe = (ddgi.update_start + gid.x) % PROBE_COUNT;
    let xyz = vec3<u32>(probe % PROBE_GRID, (probe / PROBE_GRID) % PROBE_GRID,
                        probe / (PROBE_GRID * PROBE_GRID));
    let centre = vec3<f32>(f32(PROBE_GRID - 1u)) * 0.5;
    let pos = ddgi.origin + (vec3<f32>(xyz) - centre) * ddgi.spacing;
    var coeff: array<vec3<f32>, 9>;
    let weight = PI4 / f32(RAYS);
    let rotation = f32(ddgi.update_start % 64u) * 0.09817477;
    for (var ray = 0u; ray < RAYS; ray++) {
        let dir = fibonacci_dir(ray, rotation);
        let radiance = trace_radiance(pos, dir) * ddgi.intensity;
        let basis = sh_y(dir);
        for (var c = 0u; c < SH_COEFFS; c++) {
            coeff[c] += radiance * basis[c] * weight;
        }
    }
    let base = probe * SH_COEFFS;
    let valid_word = select(ddgi.valid_lo, ddgi.valid_hi, probe >= 32u);
    let valid_bit = 1u << (probe & 31u);
    let blend = select(0.0, ddgi.hysteresis, (valid_word & valid_bit) != 0u);
    for (var c = 0u; c < SH_COEFFS; c++) {
        let old = sh_probes[base + c].rgb;
        sh_probes[base + c] = vec4<f32>(mix(coeff[c], old, blend), 1.0);
    }
}
