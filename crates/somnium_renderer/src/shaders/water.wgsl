// Somnium Engine — finite lake surface and physically coherent optics (IV-D/E).
//
// Gerstner dispersion/query parity follows the published GPU Gems water pattern;
// resource/query ownership was cross-checked against Wicked Engine wiOcean and
// bevy_water (MIT / Apache-2.0). The WGSL and integration are original.

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

struct WaterFrameData {
    current_view_proj: mat4x4<f32>,
    previous_view_proj: mat4x4<f32>,
    current_time: f32,
    previous_time: f32,
    history_valid: f32,
    _pad: f32,
}

struct WaterMaterial {
    deep_color: vec4<f32>,
    shallow_color: vec4<f32>,
    edge_color: vec4<f32>,
    absorption_roughness: vec4<f32>,
    scattering_anisotropy: vec4<f32>,
    bounds: vec4<f32>,
    surface_params: vec4<f32>,
    wave_dir_a: vec2<f32>,
    wave_dir_b: vec2<f32>,
    wave_params: vec4<f32>,
    simulation_params: vec4<f32>,
    volume_params: vec4<f32>,
    wake_origin_direction: vec4<f32>,
    wake_params: vec4<f32>,
}

struct Instance {
    model: mat4x4<f32>,
    _pad: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> view: View;
@group(0) @binding(1) var depth_texture: texture_depth_2d;
@group(0) @binding(2) var<storage, read> light: DirectionalLight;
@group(0) @binding(3) var shadow_atlas: texture_depth_2d;
@group(0) @binding(4) var shadow_sampler: sampler_comparison;
@group(0) @binding(5) var env_cube: texture_cube<f32>;
@group(0) @binding(6) var env_sampler: sampler;
@group(0) @binding(7) var scene_color: texture_2d<f32>;
@group(0) @binding(8) var<uniform> frame: WaterFrameData;

@group(1) @binding(0) var<uniform> material: WaterMaterial;
@group(1) @binding(1) var body_mask: texture_2d<f32>;
@group(1) @binding(2) var body_depth: texture_2d<f32>;
@group(1) @binding(3) var shore_sdf: texture_2d<f32>;

@group(2) @binding(0) var<storage, read> instances: array<Instance>;

@group(3) @binding(0) var tex_base_color: texture_2d<f32>;
@group(3) @binding(1) var tex_normal: texture_2d<f32>;
@group(3) @binding(2) var tex_orm: texture_2d<f32>;
@group(3) @binding(3) var sampler_linear: sampler;
@group(3) @binding(4) var spectrum_displacement_large: texture_2d<f32>;
@group(3) @binding(5) var spectrum_gradient_large: texture_2d<f32>;
@group(3) @binding(6) var spectrum_displacement_small: texture_2d<f32>;
@group(3) @binding(7) var spectrum_gradient_small: texture_2d<f32>;
@group(3) @binding(8) var spectrum_displacement_fine: texture_2d<f32>;
@group(3) @binding(9) var spectrum_gradient_fine: texture_2d<f32>;

const PI: f32 = 3.14159265359;
const TAU: f32 = 6.28318530718;
const ENV_MAX_MIP: f32 = 5.0;
const F0_WATER: vec3<f32> = vec3<f32>(0.02037);

fn body_texel(uv: vec2<f32>, dimensions: vec2<u32>) -> vec2<i32> {
    let limit = vec2<f32>(dimensions - vec2<u32>(1u));
    return vec2<i32>(round(clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * limit));
}

fn mask_at(uv: vec2<f32>) -> f32 {
    return textureSample(body_mask, sampler_linear, uv).r;
}

fn depth_at(uv: vec2<f32>) -> f32 {
    let dimensions = textureDimensions(body_depth);
    let coordinate = clamp(
        uv * vec2<f32>(dimensions) - vec2<f32>(0.5),
        vec2<f32>(0.0),
        vec2<f32>(dimensions - vec2<u32>(1u)),
    );
    let lo = vec2<i32>(floor(coordinate));
    let hi = min(lo + vec2<i32>(1), vec2<i32>(dimensions) - vec2<i32>(1));
    let blend = fract(coordinate);
    let row_a = mix(
        textureLoad(body_depth, lo, 0).r,
        textureLoad(body_depth, vec2<i32>(hi.x, lo.y), 0).r,
        blend.x,
    );
    let row_b = mix(
        textureLoad(body_depth, vec2<i32>(lo.x, hi.y), 0).r,
        textureLoad(body_depth, hi, 0).r,
        blend.x,
    );
    return mix(row_a, row_b, blend.y);
}

fn sdf_at(uv: vec2<f32>) -> f32 {
    let dimensions = textureDimensions(shore_sdf);
    let coordinate = clamp(
        uv * vec2<f32>(dimensions) - vec2<f32>(0.5),
        vec2<f32>(0.0),
        vec2<f32>(dimensions - vec2<u32>(1u)),
    );
    let lo = vec2<i32>(floor(coordinate));
    let hi = min(lo + vec2<i32>(1), vec2<i32>(dimensions) - vec2<i32>(1));
    let blend = fract(coordinate);
    let row_a = mix(
        textureLoad(shore_sdf, lo, 0).r,
        textureLoad(shore_sdf, vec2<i32>(hi.x, lo.y), 0).r,
        blend.x,
    );
    let row_b = mix(
        textureLoad(shore_sdf, vec2<i32>(lo.x, hi.y), 0).r,
        textureLoad(shore_sdf, hi, 0).r,
        blend.x,
    );
    return mix(row_a, row_b, blend.y);
}

fn local_coord(uv: vec2<f32>) -> vec2<f32> {
    return mix(material.bounds.xy, material.bounds.zw, uv);
}

fn repeating_texel(p: vec2<f32>, patch_length: f32, dimensions: vec2<u32>) -> vec2<i32> {
    let uv = fract(p / patch_length + vec2<f32>(16.0));
    return vec2<i32>(floor(uv * vec2<f32>(dimensions))) % vec2<i32>(dimensions);
}

fn cubic_weights(a: f32) -> vec4<f32> {
    let a2 = a * a;
    let a3 = a2 * a;
    return vec4<f32>(
        -a3 + 3.0 * a2 - 3.0 * a + 1.0,
        3.0 * a3 - 6.0 * a2 + 4.0,
        -3.0 * a3 + 3.0 * a2 + 3.0 * a + 1.0,
        a3,
    ) / 6.0;
}

// Four-tap cubic B-spline filtering, blended with hardware bilinear according
// to the cascade's world-space texel density. This is the GodotOceanWaves/Atlas
// strategy: cubic filtering stabilizes sparsely sampled distant slopes while
// bilinear preserves resolved close detail.
fn sample_spectrum_bicubic(tex: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let dimensions = vec2<f32>(textureDimensions(tex));
    let texel = 1.0 / dimensions;
    let coordinate = fract(uv) * dimensions + vec2<f32>(0.5);
    let fraction = fract(coordinate);
    let wx = cubic_weights(fraction.x);
    let wy = cubic_weights(fraction.y);
    let gx = vec2<f32>(wx.x + wx.y, wx.z + wx.w);
    let gy = vec2<f32>(wy.x + wy.y, wy.z + wy.w);
    let hx = (vec2<f32>(wx.y, wx.w) / max(gx, vec2<f32>(1e-6))
        + vec2<f32>(-1.5, 0.5) + floor(coordinate.x)) * texel.x;
    let hy = (vec2<f32>(wy.y, wy.w) / max(gy, vec2<f32>(1e-6))
        + vec2<f32>(-1.5, 0.5) + floor(coordinate.y)) * texel.y;
    let blend = vec2<f32>(gx.x / max(gx.x + gx.y, 1e-6),
        gy.x / max(gy.x + gy.y, 1e-6));
    let row0 = mix(
        textureSampleLevel(tex, sampler_linear, vec2<f32>(hx.y, hy.y), 0.0),
        textureSampleLevel(tex, sampler_linear, vec2<f32>(hx.x, hy.y), 0.0),
        blend.x,
    );
    let row1 = mix(
        textureSampleLevel(tex, sampler_linear, vec2<f32>(hx.y, hy.x), 0.0),
        textureSampleLevel(tex, sampler_linear, vec2<f32>(hx.x, hy.x), 0.0),
        blend.x,
    );
    return mix(row0, row1, blend.y);
}

fn sample_spectrum_filtered(
    tex: texture_2d<f32>,
    p: vec2<f32>,
    patch_length: f32,
    metres_per_pixel: f32,
) -> vec4<f32> {
    let dimensions = textureDimensions(tex);
    let uv = fract(p / patch_length + vec2<f32>(16.0));
    let world_texel = patch_length / f32(dimensions.x);
    let linear = textureSampleLevel(tex, sampler_linear, uv, 0.0);
    let cubic = sample_spectrum_bicubic(tex, uv);
    let resolved = 1.0 - smoothstep(world_texel * 0.65, world_texel * 2.0,
        metres_per_pixel);
    return mix(cubic, linear, resolved);
}

fn spectral_displacement(p: vec2<f32>, shore: f32) -> vec3<f32> {
    let large = textureLoad(spectrum_displacement_large,
        repeating_texel(p, 88.0, textureDimensions(spectrum_displacement_large)), 0);
    let medium = textureLoad(spectrum_displacement_small,
        repeating_texel(p, 57.0, textureDimensions(spectrum_displacement_small)), 0);
    let combined = (large.xyz * 1.0 + medium.xyz * 0.75)
        * clamp(material.simulation_params.x, 0.0, 1.0);
    let scale = material.surface_params.z * shore;
    return vec3<f32>(combined.x, combined.z, combined.y) * scale;
}

fn spectral_gradient(p: vec2<f32>, metres_per_pixel: f32) -> vec4<f32> {
    let large = sample_spectrum_filtered(spectrum_gradient_large, p, 88.0,
        metres_per_pixel);
    let medium = sample_spectrum_filtered(spectrum_gradient_small, p, 57.0,
        metres_per_pixel);
    let fine = sample_spectrum_filtered(spectrum_gradient_fine, p, 16.0,
        metres_per_pixel);
    let blend = clamp(material.simulation_params.x, 0.0, 1.0);
    let grad = (large.xy * 1.0 + medium.xy * 1.0 + fine.xy * 0.25) * blend;
    let jacobian = max(max(large.z, medium.z), fine.z) * blend;
    let foam = (large.a + medium.a + fine.a) * blend;
    return vec4<f32>(grad, jacobian, foam);
}

struct WaveSample {
    displacement: vec3<f32>,
    normal: vec3<f32>,
    velocity: vec3<f32>,
}

fn safe_dir(v: vec2<f32>) -> vec2<f32> {
    let length_squared = dot(v, v);
    return select(vec2<f32>(1.0, 0.0), v * inverseSqrt(length_squared), length_squared > 1e-8);
}

fn rotate2(p: vec2<f32>, angle: f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}

struct WakeSample {
    slope: vec2<f32>,
    foam: f32,
}

// Compact Kelvin-wake approximation: the 19.47 degree divergent arms carry
// most of the white water, with a narrower turbulent prop wash on the centre
// line. It is driven by the same CPU vessel position/direction as buoyancy.
fn evaluate_wake(p: vec2<f32>) -> WakeSample {
    let strength = material.wake_params.y;
    if strength <= 0.0001 {
        return WakeSample(vec2<f32>(0.0), 0.0);
    }
    let direction = safe_dir(material.wake_origin_direction.zw);
    let side = vec2<f32>(-direction.y, direction.x);
    let delta = p - material.wake_origin_direction.xy;
    let behind = max(-dot(delta, direction), 0.0);
    let lateral = dot(delta, side);
    let length_limit = max(material.wake_params.z, 8.0);
    let half_width = max(material.wake_params.w, 2.0) + behind * 0.354;
    let longitudinal = smoothstep(0.0, 3.0, behind)
        * (1.0 - smoothstep(length_limit * 0.72, length_limit, behind));
    let cone = 1.0 - smoothstep(half_width * 0.72, half_width, abs(lateral));
    let arm_distance = abs(abs(lateral) - behind * 0.354);
    let arms = 1.0 - smoothstep(0.35, 1.85, arm_distance);
    let wash = exp(-abs(lateral) * 0.42) * exp(-behind * 0.028);
    let phase = sin(behind * 1.05 + abs(lateral) * 1.75 - frame.current_time * 5.2);
    let energy = longitudinal * cone * strength * clamp(material.wake_params.x / 3.0, 0.2, 1.6);
    let slope = (direction * phase * (arms * 0.18 + wash * 0.08)
        + side * sign(lateral) * arms * 0.12) * energy;
    let foam = clamp(energy * (arms * 1.15 + wash * 0.72), 0.0, 1.0);
    return WakeSample(slope, foam);
}

fn evaluate_waves(p: vec2<f32>, time: f32, shore: f32) -> WaveSample {
    let a = safe_dir(material.wave_dir_a);
    let b = safe_dir(material.wave_dir_b);
    let dirs = array<vec2<f32>, 4>(
        a,
        b,
        safe_dir(a + vec2<f32>(-b.y, b.x) * 0.35),
        safe_dir(b - vec2<f32>(-a.y, a.x) * 0.25),
    );
    let wavelengths = array<f32, 4>(
        max(material.wave_params.x, 0.5),
        max(material.wave_params.y, 0.5),
        max(material.wave_params.x, 0.5) * 0.50,
        max(material.wave_params.y, 0.5) * 0.70,
    );
    let weights = array<f32, 4>(0.55, 0.25, 0.13, 0.07);
    let total_amplitude = material.surface_params.z * shore;
    let speed = material.wave_params.z;
    let steepness = clamp(material.wave_params.w, 0.0, 0.95);
    var displacement = vec3<f32>(0.0);
    var dx = vec3<f32>(1.0, 0.0, 0.0);
    var dz = vec3<f32>(0.0, 0.0, 1.0);
    var velocity = vec3<f32>(0.0);
    for (var i = 0u; i < 4u; i = i + 1u) {
        let dir = dirs[i];
        let amplitude = total_amplitude * weights[i];
        let k = TAU / wavelengths[i];
        let omega = sqrt(9.81 * k) * speed;
        let phase = k * dot(dir, p) + omega * time;
        let s = sin(phase);
        let c = cos(phase);
        displacement += vec3<f32>(
            steepness * amplitude * dir.x * c,
            amplitude * s,
            steepness * amplitude * dir.y * c,
        );
        dx += vec3<f32>(
            -steepness * amplitude * k * dir.x * dir.x * s,
            amplitude * k * dir.x * c,
            -steepness * amplitude * k * dir.x * dir.y * s,
        );
        dz += vec3<f32>(
            -steepness * amplitude * k * dir.x * dir.y * s,
            amplitude * k * dir.y * c,
            -steepness * amplitude * k * dir.y * dir.y * s,
        );
        velocity += vec3<f32>(
            -steepness * amplitude * dir.x * omega * s,
            amplitude * omega * c,
            -steepness * amplitude * dir.y * omega * s,
        );
    }
    return WaveSample(displacement, normalize(cross(dz, dx)), velocity);
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

fn sample_shadow(world_pos: vec3<f32>, view_depth: f32) -> f32 {
    let cascade = get_cascade_index(view_depth);
    let clip = light.view_proj[cascade] * vec4<f32>(world_pos, 1.0);
    let ndc = clip.xyz / clip.w;
    let uv = atlas_uv(cascade, vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5));
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || ndc.z > 1.0 {
        return 1.0;
    }
    let texel = 1.0 / light.shadow_map_size;
    var result = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            result += textureSampleCompare(
                shadow_atlas, shadow_sampler,
                uv + vec2<f32>(f32(x), f32(y)) * texel, ndc.z,
            );
        }
    }
    return result / 9.0;
}

fn reconstruct_world(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world = view.inv_view_proj * ndc;
    return world.xyz / world.w;
}

fn project_uv(world: vec3<f32>) -> vec3<f32> {
    let clip = frame.current_view_proj * vec4<f32>(world, 1.0);
    let ndc = clip.xyz / clip.w;
    return vec3<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, ndc.z);
}

fn view_depth(world: vec3<f32>) -> f32 {
    return -(view.view_matrix * vec4<f32>(world, 1.0)).z;
}

fn edge_confidence(uv: vec2<f32>) -> f32 {
    let edge = min(min(uv.x, uv.y), min(1.0 - uv.x, 1.0 - uv.y));
    return smoothstep(0.0, 0.08, edge);
}

// Portable SSR tier: bounded linear world-space march with depth validation.
// Hidden/off-screen regions intentionally return zero confidence and fall back
// to the prefiltered environment rather than leaving reflection holes.
fn trace_ssr(origin: vec3<f32>, direction: vec3<f32>) -> vec4<f32> {
    var distance_along = 0.35;
    for (var i = 0u; i < 28u; i = i + 1u) {
        distance_along += mix(0.35, 2.5, f32(i) / 27.0);
        let ray_point = origin + direction * distance_along;
        let projected = project_uv(ray_point);
        if projected.z <= 0.0 || projected.z >= 1.0
            || any(projected.xy <= vec2<f32>(0.0)) || any(projected.xy >= vec2<f32>(1.0)) {
            break;
        }
        let dimensions = textureDimensions(depth_texture);
        let coord = body_texel(projected.xy, dimensions);
        let scene_depth = textureLoad(depth_texture, coord, 0);
        if scene_depth < 0.9999 {
            let scene_world = reconstruct_world(projected.xy, scene_depth);
            let delta = view_depth(ray_point) - view_depth(scene_world);
            let thickness = 0.12 + distance_along * 0.012;
            if delta >= 0.0 && delta < thickness {
                let confidence = edge_confidence(projected.xy)
                    * (1.0 - f32(i) / 28.0);
                let color = textureSampleLevel(scene_color, sampler_linear, projected.xy, 0.0).rgb;
                return vec4<f32>(color, confidence);
            }
        }
    }
    return vec4<f32>(0.0);
}

fn fresnel_schlick(cosine: f32) -> vec3<f32> {
    return F0_WATER + (vec3<f32>(1.0) - F0_WATER) * pow(1.0 - cosine, 5.0);
}

fn hg_phase(g: f32, cos_theta: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(max(1.0 + g2 - 2.0 * g * cos_theta, 1e-4), 1.5));
}

fn direct_specular(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, radiance: vec3<f32>, roughness: f32) -> vec3<f32> {
    let ndotl = max(dot(n, l), 0.0);
    let ndotv = max(dot(n, v), 1e-4);
    if ndotl <= 0.0 { return vec3<f32>(0.0); }
    let h = normalize(v + l);
    let ndoth = max(dot(n, h), 0.0);
    let vdoth = max(dot(v, h), 0.0);
    let alpha = roughness * roughness;
    let alpha2 = alpha * alpha;
    let widened = clamp(alpha + light.sun_angular_radius * 0.5, alpha, 1.0);
    let widened2 = widened * widened;
    let denominator = ndoth * ndoth * (widened2 - 1.0) + 1.0;
    let d = widened2 / max(PI * denominator * denominator, 1e-6);
    let k = alpha * 0.5;
    let gv = ndotv / (ndotv * (1.0 - k) + k);
    let gl = ndotl / (ndotl * (1.0 - k) + k);
    let f = fresnel_schlick(vdoth);
    let energy = alpha2 / max(widened2, 1e-8);
    return d * gv * gl * f * radiance * ndotl * energy / max(4.0 * ndotv * ndotl, 1e-4);
}

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @builtin(instance_index) instance_index: u32,
}

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) screen_velocity: vec2<f32>,
    @location(3) wave_height: f32,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let instance = instances[input.instance_index];
    let depth = depth_at(input.uv);
    let shore = smoothstep(0.25, 2.0, depth);
    let p = local_coord(input.uv);
    let current_wave = evaluate_waves(p, frame.current_time, shore);
    let previous_wave = evaluate_waves(p, frame.previous_time, shore);
    let spectrum_wave = spectral_displacement(p, shore);
    let base_world = (instance.model * vec4<f32>(input.position, 1.0)).xyz;
    let current_world = base_world + (instance.model
        * vec4<f32>(current_wave.displacement + spectrum_wave, 0.0)).xyz;
    // Spectral displacement history remains visual-only in this tier; reusing
    // the current sample avoids inventing a false whole-wave motion vector.
    let previous_world = base_world + (instance.model
        * vec4<f32>(previous_wave.displacement + spectrum_wave, 0.0)).xyz;
    let current_clip = frame.current_view_proj * vec4<f32>(current_world, 1.0);
    let previous_clip = frame.previous_view_proj * vec4<f32>(previous_world, 1.0);
    let current_ndc = current_clip.xy / current_clip.w;
    let previous_ndc = previous_clip.xy / previous_clip.w;
    var output: VertexOutput;
    output.world_position = current_world;
    output.uv = input.uv;
    // IV-K3: Pass wave height to fragment for SSS calculation.
    output.wave_height = (current_wave.displacement + spectrum_wave).y;
    output.screen_velocity = select(
        vec2<f32>(0.0),
        (previous_ndc - current_ndc) * vec2<f32>(0.5, -0.5),
        frame.history_valid > 0.5 && previous_clip.w > 0.0,
    );
    output.clip_pos = view.view_proj * vec4<f32>(current_world, 1.0);
    return output;
}

struct FragmentOutput {
    @location(0) color: vec4<f32>,
    // Encoded XZ normal, linear view depth, coverage.
    @location(1) surface: vec4<f32>,
    @location(2) velocity: vec2<f32>,
}

@fragment
fn fs_main(input: VertexOutput, @builtin(front_facing) front_facing: bool) -> FragmentOutput {
    // Reconstruct a continuous zero contour from the signed shoreline field.
    // A binary filtered mask still exposes its square texel grid at oblique
    // shorelines; the bilinear SDF contour is stable in world space and gets
    // one derivative-scaled antialiasing band on top.
    let sdf_cells = sdf_at(input.uv);
    let cell_metres = (material.bounds.z - material.bounds.x)
        / f32(max(textureDimensions(shore_sdf).x, 1u));
    let signed_shore_distance = sdf_cells * cell_metres;
    let coverage_width = max(fwidth(signed_shore_distance), cell_metres * 0.35);
    // Extend the surface slightly beneath opaque terrain. Terrain depth owns
    // the visible intersection, as in Unreal's dilated WaterInfo mesh and
    // Wicked Engine's broad ocean surface. This closes sub-cell mask/terrain
    // mismatches without allowing water to draw over dry ground.
    let under_terrain_guard = 1.5;
    let coverage = smoothstep(
        -under_terrain_guard - coverage_width,
        -under_terrain_guard + coverage_width,
        signed_shore_distance,
    );
    if coverage <= 0.001 {
        discard;
    }
    let authored_depth = max(depth_at(input.uv), 0.025);
    let shore = smoothstep(0.25, 2.0, authored_depth);
    let water_coord = local_coord(input.uv);
    let wave = evaluate_waves(water_coord, frame.current_time, shore);
    let metres_per_pixel = max(length(dpdx(water_coord)), length(dpdy(water_coord)));
    let spectrum = spectral_gradient(water_coord, metres_per_pixel);
    let wake = evaluate_wake(water_coord);
    let water_view_depth = view_depth(input.world_position);

    // A displaced vertex surface can keep its large silhouette waves, while
    // its sub-pixel slope energy must migrate into roughness. Otherwise the
    // regular Gerstner bands turn into a distant cross-hatch under highlights.
    let shortest_wave = max(min(material.wave_params.x, material.wave_params.y) * 0.70, 0.5);
    let slope_resolve = 1.0 - smoothstep(shortest_wave * 0.035, shortest_wave * 0.18,
        metres_per_pixel);
    let distance_resolve = 1.0 - smoothstep(120.0, 420.0, water_view_depth);
    let spectrum_normal = normalize(vec3<f32>(-spectrum.x * material.surface_params.z * 0.42,
        1.0, -spectrum.y * material.surface_params.z * 0.42));
    let wake_normal = normalize(vec3<f32>(-wake.slope.x, 1.0, -wake.slope.y));
    let combined_wave_normal = normalize(mix(
        normalize(mix(wave.normal, spectrum_normal, 0.38)),
        wake_normal,
        clamp(material.wake_params.y * 0.55, 0.0, 0.75),
    ));
    let resolved_wave_normal = normalize(mix(vec3<f32>(0.0, 1.0, 0.0), combined_wave_normal,
        min(slope_resolve, distance_resolve)));

    let texture_time = frame.current_time;
    // Scale-separated, rotated samples prevent the obvious 16-28 m repeating
    // grid visible in the old material. Fine detail fades first with distance.
    let detail_uv_a = rotate2(water_coord, 0.31) * 0.008
        + vec2<f32>(0.004, 0.003) * texture_time;
    let detail_uv_b = rotate2(water_coord, -0.67) * 0.021
        + vec2<f32>(-0.008, 0.010) * texture_time;
    let detail_uv_c = rotate2(water_coord, 1.13) * 0.057
        + vec2<f32>(0.015, -0.011) * texture_time;
    let normal_a = textureSample(tex_normal, sampler_linear, detail_uv_a).xyz * 2.0 - 1.0;
    let normal_b = textureSample(tex_normal, sampler_linear, detail_uv_b).xyz * 2.0 - 1.0;
    let normal_c = textureSample(tex_normal, sampler_linear, detail_uv_c).xyz * 2.0 - 1.0;
    let detail_normal = normalize(normal_a * 0.52 + normal_b * 0.31 + normal_c * 0.17);
    let tangent = normalize(vec3<f32>(resolved_wave_normal.y, -resolved_wave_normal.x, 0.0));
    let bitangent = normalize(cross(resolved_wave_normal, tangent));
    let mapped = normalize(mat3x3<f32>(tangent, bitangent, resolved_wave_normal) * detail_normal);
    // Move sub-pixel wave energy into roughness with distance instead of
    // letting unresolved normals become a white moiré pattern at the horizon.
    let detail_strength = 0.24 * (1.0 - smoothstep(75.0, 360.0, water_view_depth));
    var n = normalize(mix(resolved_wave_normal, mapped, detail_strength));
    let v = normalize(view.camera_pos - input.world_position);
    if dot(n, v) < 0.0 { n = -n; }
    let ndotv = max(dot(n, v), 1e-4);
    let screen_size = vec2<f32>(textureDimensions(depth_texture));
    let screen_uv = input.clip_pos.xy / screen_size;
    let coord = vec2<i32>(input.clip_pos.xy);
    let base_depth = textureLoad(depth_texture, coord, 0);
    let base_has_backdrop = base_depth < 0.9999;
    let base_world = reconstruct_world(screen_uv, min(base_depth, 0.9999));

    // Refract, then reject samples that are sky, in front of the surface, or
    // reveal foreground. Invalid displacement falls back to the unperturbed UV.
    let refract_offset = n.xz * (0.010 + 0.022 * shore) / (1.0 + water_view_depth * 0.015);
    let candidate_uv = clamp(screen_uv + refract_offset, vec2<f32>(0.0), vec2<f32>(1.0));
    let candidate_coord = body_texel(candidate_uv, textureDimensions(depth_texture));
    let candidate_depth = textureLoad(depth_texture, candidate_coord, 0);
    let candidate_world = reconstruct_world(candidate_uv, min(candidate_depth, 0.9999));
    let candidate_valid = candidate_depth < 0.9999
        && view_depth(candidate_world) > water_view_depth + 0.03;
    let refr_uv = select(screen_uv, candidate_uv, candidate_valid);
    let refracted = textureSampleLevel(scene_color, sampler_linear, refr_uv, 0.0).rgb;

    let backdrop_distance = select(authored_depth, distance(input.world_position, base_world), base_has_backdrop);
    let path_length = clamp(min(backdrop_distance, authored_depth / max(abs(dot(n, v)), 0.18)), 0.0, 60.0);
    let clarity_scale = mix(1.8, 0.45, clamp(material.surface_params.x, 0.0, 1.0));
    let sigma_a = max(material.absorption_roughness.rgb * clarity_scale, vec3<f32>(1e-5));
    let sigma_s = max(material.scattering_anisotropy.rgb * clarity_scale, vec3<f32>(0.0));
    let sigma_t = sigma_a + sigma_s;
    let transmittance = exp(-sigma_t * path_length);

    let sun_l = normalize(light.direction);
    let moon_l = normalize(light.moon_direction);
    let shadow = sample_shadow(input.world_position, water_view_depth);
    let phase_g = clamp(material.scattering_anisotropy.a, -0.8, 0.8);
    let sun_scatter = light.color * max(dot(n, sun_l), 0.0) * shadow
        * hg_phase(phase_g, dot(-v, sun_l));
    let moon_color = vec3<f32>(max(light.moon_intensity, 0.0));
    let moon_scatter = moon_color * max(dot(n, moon_l), 0.0)
        * hg_phase(phase_g, dot(-v, moon_l));
    let env_up = textureSampleLevel(env_cube, env_sampler, vec3<f32>(0.0, 1.0, 0.0), ENV_MAX_MIP).rgb
        * light.ibl_intensity;
    let single_scatter = (vec3<f32>(1.0) - transmittance)
        * sigma_s / max(sigma_t, vec3<f32>(1e-5))
        * (sun_scatter + moon_scatter + env_up);
    let tint = mix(material.shallow_color.rgb, material.deep_color.rgb,
        smoothstep(0.5, max(material.surface_params.y * 6.0, 2.0), authored_depth));
    var transmitted = refracted * tint * transmittance + single_scatter;

    // IV-K3: GodotOceanWaves SSS model. Forward-scattered light through wave
    // peaks produces a green-tinted glow visible when looking toward the sun.
    // The old code multiplied by 0.000004, effectively zeroing the effect.
    let sss_modifier = vec3<f32>(0.9, 1.15, 0.85); // green-shift for underwater
    let sss_height = 1.0 * max(0.0, input.wave_height + 2.5)
        * pow(max(dot(sun_l, -v), 0.0), 4.0)
        * pow(max(0.5 - 0.5 * dot(sun_l, n), 0.0), 3.0);
    let sss_near = 0.5 * pow(ndotv, 2.0);
    let crest_scatter = sss_modifier
        * (sss_height + sss_near)
        * light.color * shadow * (1.0 - fresnel_schlick(ndotv).x);
    transmitted += crest_scatter * (vec3<f32>(1.0) - transmittance);

    let shore_distance = max(sdf_cells, 0.0) * cell_metres;
    let foam_width = max(material.surface_params.y * 8.0, 6.0);
    let foam_noise = clamp(
        textureSample(tex_orm, sampler_linear, detail_uv_b * 0.63).r * 0.65
        + textureSample(tex_normal, sampler_linear, detail_uv_a * 0.47).x * 0.35,
        0.0,
        1.0,
    );
    let breaker_distance = 1.1 + 0.75 * sin(frame.current_time * 0.72
        + dot(water_coord, vec2<f32>(0.071, 0.043)));
    let breaker = 1.0 - smoothstep(0.45, 2.0, abs(shore_distance - breaker_distance));
    let shore_band = 1.0 - smoothstep(0.0, foam_width, shore_distance);
    // Wicked Engine derives shore foam from the difference between water and
    // opaque scene depth. Combine that actual contact signal with the authored
    // SDF band, so residual one-metre terrain facets blend into foam instead of
    // reading as a geometric cut-out.
    let depth_contact = select(
        0.0,
        1.0 - smoothstep(0.04, 1.35, backdrop_distance),
        base_has_backdrop,
    );
    let shore_foam = max(
        shore_band * clamp(0.28 + breaker * 0.82 + foam_noise * 0.38, 0.0, 1.0),
        depth_contact * (0.62 + foam_noise * 0.28),
    );
    let crest_foam = smoothstep(0.0, 1.0, spectrum.a * 1.35) * smoothstep(0.5, 2.0, authored_depth);
    let foam_amount = clamp(max(max(shore_foam, crest_foam), wake.foam), 0.0, 1.0);
    let wet_band = (1.0 - smoothstep(0.0, max(material.surface_params.y * 2.5, 1.5),
        shore_distance)) * (0.35 + foam_amount * 0.65);
    transmitted *= 1.0 - wet_band * 0.16;
    // IV-K3: Bright, rough foam — not a faint tint. Reference uses white foam
    // albedo with full Lambertian lighting and distance fade.
    let foam_albedo = vec3<f32>(0.85, 0.88, 0.82);
    let foam_light = foam_albedo
        * (env_up + light.color * shadow * max(dot(n, sun_l), 0.0) + moon_color * max(dot(n, moon_l), 0.0))
        * exp(-water_view_depth * 0.0075);
    transmitted = mix(transmitted, foam_light, foam_amount);

    let roughness_map = 0.5 * (
        textureSample(tex_orm, sampler_linear, detail_uv_a).g
        + textureSample(tex_orm, sampler_linear, detail_uv_b).g * 0.65
        + textureSample(tex_orm, sampler_linear, detail_uv_c).g * 0.35) / 1.5;
    let unresolved_energy = 1.0 - min(slope_resolve, distance_resolve);
    let distance_roughness = unresolved_energy * 0.28;
    // IV-K3: Raise base roughness from ~0.06 → ~0.40, matching GodotOceanWaves.
    // Foam pushes roughness further up to ~0.85 for rough whitecap appearance.
    let roughness = clamp(
        0.40 + roughness_map * 0.08 + distance_roughness + foam_amount * 0.45,
        0.04,
        0.95,
    );
    let reflection_dir = reflect(-v, n);
    let environment = textureSampleLevel(env_cube, env_sampler, reflection_dir, roughness * ENV_MAX_MIP).rgb
        * light.ibl_intensity;
    let ssr = trace_ssr(input.world_position + n * 0.05, reflection_dir);
    let ssr_weight = ssr.a * clamp(material.surface_params.w, 0.0, 1.0);
    let reflected = mix(environment, ssr.rgb, ssr_weight);
    var fresnel = fresnel_schlick(ndotv);
    if !front_facing {
        // Water-to-air transmission reaches a critical angle. Past it the
        // underside becomes a full reflection (the Snell-window/TIR boundary).
        let eta = 1.333;
        let sin_transmitted_sq = eta * eta * (1.0 - ndotv * ndotv);
        fresnel = mix(fresnel, vec3<f32>(1.0), smoothstep(0.96, 1.02,
            sin_transmitted_sq));
    }
    let direct = direct_specular(n, v, sun_l, light.color * shadow, roughness)
        + direct_specular(n, v, moon_l, moon_color, max(roughness, 0.06));
    let final_color = transmitted * (vec3<f32>(1.0) - fresnel) + reflected * fresnel + direct;

    return FragmentOutput(
        vec4<f32>(min(final_color, vec3<f32>(60000.0)), coverage),
        vec4<f32>(n.xz * 0.5 + 0.5, min(water_view_depth, 60000.0), coverage),
        clamp(input.screen_velocity, vec2<f32>(-1.0), vec2<f32>(1.0)),
    );
}
