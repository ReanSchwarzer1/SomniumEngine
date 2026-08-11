// Somnium Engine Phase IV-G finite-water medium composition.
// Original WGSL: ray-segment submersion and near-plane coverage avoid a
// binary fullscreen switch. No Brown-Conrady or stylized reference helpers.

struct View {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    view_matrix: mat4x4<f32>,
    camera_pos: vec3<f32>,
    _padding: f32,
    time: f32,
    _pad1: vec2<f32>,
}

struct DirectionalLight {
    direction: vec3<f32>,
    _pad0: f32,
    color: vec3<f32>,
    _pad1: f32,
    view_proj: array<mat4x4<f32>, 4>,
    cascade_splits: vec4<f32>,
    shadow_map_size: f32,
    ibl_intensity: f32,
    sun_angular_radius: f32,
    _pad2_z: f32,
    moon_direction: vec3<f32>,
    moon_intensity: f32,
}

struct UnderwaterParams {
    model: mat4x4<f32>,
    inverse_model: mat4x4<f32>,
    bounds: vec4<f32>,
    absorption: vec4<f32>,
    scattering: vec4<f32>,
    surface: vec4<f32>, // amplitude, clarity, max depth, caustic strength
    wave_dirs: vec4<f32>,
    wave: vec4<f32>, // wavelength A/B, speed, steepness
    frame: vec4<f32>, // time, camera signed distance, enabled, padding
}

@group(0) @binding(0) var scene_color: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var opaque_depth: texture_depth_2d;
@group(0) @binding(3) var<storage, read> view: View;
@group(0) @binding(4) var<storage, read> light: DirectionalLight;
@group(0) @binding(5) var body_mask: texture_2d<f32>;
@group(0) @binding(6) var body_depth: texture_2d<f32>;
@group(0) @binding(7) var<uniform> params: UnderwaterParams;

const PI: f32 = 3.14159265359;
const TAU: f32 = 6.28318530718;

fn safe_dir(value: vec2<f32>) -> vec2<f32> {
    return select(vec2<f32>(1.0, 0.0), normalize(value), dot(value, value) > 1e-7);
}

fn wave_height(p: vec2<f32>) -> f32 {
    let a = safe_dir(params.wave_dirs.xy);
    let b = safe_dir(params.wave_dirs.zw);
    let dirs = array<vec2<f32>, 4>(a, b,
        safe_dir(a + vec2<f32>(-b.y, b.x) * 0.35),
        safe_dir(b - vec2<f32>(-a.y, a.x) * 0.25));
    let lengths = array<f32, 4>(max(params.wave.x, 0.5), max(params.wave.y, 0.5),
        max(params.wave.x * 0.5, 0.5), max(params.wave.y * 0.7, 0.5));
    let weights = array<f32, 4>(0.55, 0.25, 0.13, 0.07);
    var result = 0.0;
    for (var i = 0u; i < 4u; i++) {
        let k = TAU / lengths[i];
        let omega = sqrt(9.81 * k) * params.wave.z;
        result += sin(k * dot(dirs[i], p) + omega * params.frame.x)
            * params.surface.x * weights[i];
    }
    return result;
}

fn local_uv(local: vec3<f32>) -> vec2<f32> {
    let size = max(params.bounds.zw - params.bounds.xy, vec2<f32>(1e-5));
    return local.xz / size + vec2<f32>(0.5);
}

fn mask_at(local: vec3<f32>) -> f32 {
    let uv = local_uv(local);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) { return 0.0; }
    let dimensions = textureDimensions(body_mask);
    let coord = vec2<i32>(round(uv * vec2<f32>(dimensions - vec2<u32>(1u))));
    return textureLoad(body_mask, coord, 0).r;
}

fn depth_at(local: vec3<f32>) -> f32 {
    let uv = clamp(local_uv(local), vec2<f32>(0.0), vec2<f32>(1.0));
    let dimensions = textureDimensions(body_depth);
    let coord = vec2<i32>(round(uv * vec2<f32>(dimensions - vec2<u32>(1u))));
    return textureLoad(body_depth, coord, 0).r;
}

fn reconstruct_world(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world = view.inv_view_proj * clip;
    return world.xyz / world.w;
}

fn hg(g: f32, cosine: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(max(1.0 + g2 - 2.0 * g * cosine, 1e-4), 1.5));
}

fn caustic_pattern(p: vec2<f32>, time: f32) -> f32 {
    let a = sin(dot(p, vec2<f32>(0.83, 0.56)) * 1.7 + time * 1.15);
    let b = sin(dot(p, vec2<f32>(-0.31, 0.95)) * 2.1 - time * 0.92);
    let c = sin(dot(p, vec2<f32>(0.69, -0.72)) * 3.3 + time * 0.47);
    return pow(clamp((a + b + c) * 0.18 + 0.52, 0.0, 1.0), 7.0);
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let positions = array<vec2<f32>, 3>(vec2<f32>(-1.0, -3.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0));
    var output: VertexOutput;
    output.position = vec4<f32>(positions[index], 0.0, 1.0);
    output.uv = positions[index] * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let original = textureSampleLevel(scene_color, linear_sampler, input.uv, 0.0);
    if params.frame.z < 0.5 { return original; }
    let camera_local = (params.inverse_model * vec4<f32>(view.camera_pos, 1.0)).xyz;
    let near_world = reconstruct_world(input.uv, 0.0001);
    let near_local = (params.inverse_model * vec4<f32>(near_world, 1.0)).xyz;
    let near_signed = near_local.y - wave_height(near_local.xz);
    // A per-pixel near-plane classification: when the near plane intersects
    // the displaced surface, adjacent rays transition independently.
    let near_submersion = smoothstep(0.035, -0.035, near_signed) * mask_at(near_local);
    if near_submersion <= 0.0001 { return original; }

    let depth = textureLoad(opaque_depth, vec2<i32>(input.position.xy), 0);
    let ray = normalize(near_world - view.camera_pos);
    let endpoint_world = select(reconstruct_world(input.uv, min(depth, 0.9999)),
        view.camera_pos + ray * 240.0, depth >= 0.9999);
    let endpoint_local = (params.inverse_model * vec4<f32>(endpoint_world, 1.0)).xyz;
    let start_signed = camera_local.y - wave_height(camera_local.xz);
    let end_signed = endpoint_local.y - wave_height(endpoint_local.xz);
    let total_distance = distance(view.camera_pos, endpoint_world);
    var submerged_fraction = 0.0;
    if start_signed <= 0.0 {
        submerged_fraction = select(clamp(-start_signed / max(end_signed - start_signed, 1e-5), 0.0, 1.0),
            1.0, end_signed <= 0.0);
    } else if end_signed < 0.0 {
        let crossing = start_signed / max(start_signed - end_signed, 1e-5);
        submerged_fraction = 1.0 - clamp(crossing, 0.0, 1.0);
    }
    let body_coverage = max(mask_at(camera_local), mask_at(endpoint_local));
    let path_length = min(total_distance * submerged_fraction, 80.0) * body_coverage;
    if path_length <= 0.0001 { return original; }

    let clarity_scale = mix(1.8, 0.45, clamp(params.surface.y, 0.0, 1.0));
    let sigma_a = max(params.absorption.rgb * clarity_scale, vec3<f32>(1e-5));
    let sigma_s = max(params.scattering.rgb * clarity_scale, vec3<f32>(0.0));
    let sigma_t = sigma_a + sigma_s;
    let transmittance = exp(-sigma_t * path_length);
    let sun_dir = normalize(light.direction);
    let phase = hg(clamp(params.scattering.a, -0.8, 0.8), dot(ray, sun_dir));
    let depth_below = max(-end_signed, 0.0);
    let shaft_modulation = 0.78 + 0.22 * sin(dot(endpoint_world.xz,
        vec2<f32>(0.071, 0.113)) + params.frame.x * 0.31);
    let illumination = light.color * phase * shaft_modulation
        + vec3<f32>(max(light.moon_intensity, 0.0)) * 0.08;
    let inscatter = (vec3<f32>(1.0) - transmittance)
        * sigma_s / max(sigma_t, vec3<f32>(1e-5)) * illumination;
    var color = original.rgb * transmittance + inscatter;

    // Portable projected caustics: only opaque submerged receivers, faded by
    // water depth, travelled path, and turbidity. Sky and above-water pixels
    // can never receive them.
    if depth < 0.9999 && end_signed < 0.0 && mask_at(endpoint_local) > 0.5 {
        let bed_depth = depth_at(endpoint_local);
        let turbidity = dot(sigma_t, vec3<f32>(0.333333));
        let caustic_fade = exp(-depth_below * 0.24 - path_length * turbidity * 0.18)
            * smoothstep(0.15, 1.0, bed_depth);
        color += light.color * caustic_pattern(endpoint_world.xz, params.frame.x)
            * caustic_fade * params.surface.w * 0.018;
    }
    return vec4<f32>(mix(original.rgb, color, near_submersion), original.a);
}
