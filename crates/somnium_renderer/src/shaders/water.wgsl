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
    // Per cascade: inverse tile length in x and y, then the displacement and
    // normal weights that cascade contributes.
    cascade_scales: array<vec4<f32>, 3>,
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
@group(0) @binding(9) var reflection_tex: texture_2d_array<f32>;
@group(0) @binding(10) var reflection_sampler: sampler;

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

/// Cubic filtering stabilizes a cascade whose texels are larger than a pixel;
/// hardware bilinear is both cheaper and sharper once the map out-resolves the
/// screen. The crossover is the cascade's own texel density, so a coarse map
/// and a fine map each pick the right filter at the same distance.
fn sample_spectrum_filtered(tex: texture_2d<f32>, uv: vec2<f32>, inverse_tile: f32) -> vec4<f32> {
    let texels_per_metre = f32(textureDimensions(tex).x) * inverse_tile;
    let linear = textureSampleLevel(tex, sampler_linear, uv, 0.0);
    let cubic = sample_spectrum_bicubic(tex, uv);
    return mix(cubic, linear, min(1.0, texels_per_metre * 0.1));
}

/// World displacement summed over the cascades that are allowed to move
/// geometry. Channels are `(x, up, z)` straight out of the transform.
fn spectral_displacement(p: vec2<f32>, shore: f32) -> vec3<f32> {
    var displacement = vec3<f32>(0.0);
    let scales_0 = material.cascade_scales[0];
    let scales_1 = material.cascade_scales[1];
    let scales_2 = material.cascade_scales[2];
    displacement += textureSampleLevel(spectrum_displacement_large, sampler_linear,
        p * scales_0.xy, 0.0).xyz * scales_0.z;
    displacement += textureSampleLevel(spectrum_displacement_small, sampler_linear,
        p * scales_1.xy, 0.0).xyz * scales_1.z;
    displacement += textureSampleLevel(spectrum_displacement_fine, sampler_linear,
        p * scales_2.xy, 0.0).xyz * scales_2.z;
    let scale = material.surface_params.z * shore
        * clamp(material.simulation_params.x, 0.0, 1.0);
    return displacement * scale;
}

/// Returns `(slope x, slope z, horizontal stretch, accumulated foam)`. Slope is
/// weighted per cascade; foam is summed, because a texel that is folding in two
/// cascades at once is whiter than one folding in either alone.
fn spectral_gradient(p: vec2<f32>) -> vec4<f32> {
    let scales_0 = material.cascade_scales[0];
    let scales_1 = material.cascade_scales[1];
    let scales_2 = material.cascade_scales[2];
    let large = sample_spectrum_filtered(spectrum_gradient_large, p * scales_0.xy, scales_0.x);
    let medium = sample_spectrum_filtered(spectrum_gradient_small, p * scales_1.xy, scales_1.x);
    let fine = sample_spectrum_filtered(spectrum_gradient_fine, p * scales_2.xy, scales_2.x);
    let blend = clamp(material.simulation_params.x, 0.0, 1.0);
    let grad = (large.xy * scales_0.w + medium.xy * scales_1.w + fine.xy * scales_2.w) * blend;
    let stretch = large.z + medium.z + fine.z;
    // Foam takes no part in the Gerstner/spectral crossfade. The analytic waves
    // produce none at all, so scaling it by the blend would only mean that
    // dialling back the spectrum quietly erases whitecaps that did form.
    let foam = large.a + medium.a + fine.a;
    return vec4<f32>(grad, stretch, foam);
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

// ── Reference ocean BRDF ──────────────────────────────────────────────────
//
// A rough sea does not behave like the smooth dielectric a plain Schlick term
// describes: at grazing angles the microfacets shadow each other long before
// the surface can act as a mirror. The GodotOceanWaves reference folds that
// into a roughness-dependent Fresnel whose grazing value stays below a tenth
// rather than reaching one, which is what keeps the horizon reading as water
// instead of chrome. The exponent and denominator are an empirical fit.

fn ocean_fresnel(cosine: f32, roughness: f32) -> f32 {
    let exponent = 5.0 * exp(-2.69 * roughness);
    let falloff = pow(1.0 - cosine, exponent) / (1.0 + 22.7 * pow(roughness, 1.5));
    return mix(falloff, 1.0, F0_WATER.x);
}

/// Fresnel for the environment reflection, which is a different quantity from
/// the one above despite sharing a name.
///
/// `ocean_fresnel` deliberately stays below a tenth so a rough sea does not
/// mirror the sun at grazing angles. Applying that same curve to the sky
/// reflection is what makes water read as wet stone: almost all of the colour a
/// viewer calls "sea" is reflected sky, and it has to climb towards a mirror
/// near the horizon. Roughness caps how far it gets, so foam still scatters
/// instead of reflecting.
fn environment_fresnel(cosine: f32, roughness: f32) -> f32 {
    let grazing = max(1.0 - roughness, F0_WATER.x);
    return F0_WATER.x + (grazing - F0_WATER.x) * pow(1.0 - cosine, 5.0);
}

/// Smith height-correlated masking, in the approximate form the reference uses.
/// `alpha` is taken as the roughness directly rather than its square.
fn smith_masking_shadowing(cos_theta: f32, alpha: f32) -> f32 {
    let sin_theta = sqrt(max(1.0 - cos_theta * cos_theta, 1e-6));
    let a = cos_theta / max(alpha * sin_theta, 1e-6);
    let a_squared = a * a;
    if a >= 1.6 {
        return 0.0;
    }
    return (1.0 - 1.259 * a + 0.396 * a_squared) / (3.535 * a + 2.181 * a_squared);
}

fn ggx_distribution(cos_theta: f32, alpha: f32) -> f32 {
    let alpha_squared = alpha * alpha;
    let d = 1.0 + (alpha_squared - 1.0) * cos_theta * cos_theta;
    return alpha_squared / max(PI * d * d, 1e-8);
}

/// Direct sun or moon highlight under the reference model. The `+ 0.1` in the
/// denominator is the reference's own guard against the grazing singularity.
fn ocean_specular(
    n: vec3<f32>,
    v: vec3<f32>,
    l: vec3<f32>,
    radiance: vec3<f32>,
    roughness: f32,
    fresnel: f32,
) -> vec3<f32> {
    let ndotl = max(dot(n, l), 2e-5);
    let ndotv = max(dot(n, v), 2e-5);
    if dot(n, l) <= 0.0 {
        return vec3<f32>(0.0);
    }
    let halfway = normalize(l + v);
    let light_mask = smith_masking_shadowing(ndotv, roughness);
    let view_mask = smith_masking_shadowing(ndotl, roughness);
    let distribution = ggx_distribution(dot(n, halfway), roughness);
    let geometry = 1.0 / (1.0 + light_mask + view_mask);
    return radiance * fresnel * distribution * geometry / (4.0 * ndotv + 0.1);
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

struct ShadeOutput {
    @location(0) color: vec4<f32>,
}

struct PrepassOutput {
    @location(0) surface: vec4<f32>,
    @location(1) velocity: vec2<f32>,
    @location(2) roughness: f32,
}

fn upsample_rt(uv: vec2<f32>, coverage: f32, layer: i32) -> vec4<f32> {
    let dims = vec2<f32>(textureDimensions(reflection_tex));
    if dims.x < 1.5 {
        return vec4<f32>(0.0);
    }
    let pixel = uv * dims - vec2<f32>(0.5);
    let base = vec2<i32>(floor(pixel));
    let frac = fract(pixel);
    var acc = vec4<f32>(0.0);
    var weight = 0.0;
    for (var y = 0; y <= 1; y = y + 1) {
        for (var x = 0; x <= 1; x = x + 1) {
            let tap = textureLoad(reflection_tex, base + vec2<i32>(x, y), layer, 0);
            let bilinear = select(frac.x, 1.0 - frac.x, x == 0) * select(frac.y, 1.0 - frac.y, y == 0);
            let w = bilinear * max(tap.a, 0.05) * coverage;
            acc += tap * w;
            weight += w;
        }
    }
    if weight < 1e-5 {
        return vec4<f32>(0.0);
    }
    return acc / weight;
}

@fragment
fn fs_main(input: VertexOutput, @builtin(front_facing) front_facing: bool) -> ShadeOutput {
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
    let spectrum = spectral_gradient(water_coord);
    let wake = evaluate_wake(water_coord);
    let water_view_depth = view_depth(input.world_position);

    // A displaced vertex surface can keep its large silhouette waves, while
    // its sub-pixel slope energy must migrate into roughness. Otherwise the
    // regular Gerstner bands turn into a distant cross-hatch under highlights.
    let shortest_wave = max(min(material.wave_params.x, material.wave_params.y) * 0.70, 0.5);
    let slope_resolve = 1.0 - smoothstep(shortest_wave * 0.035, shortest_wave * 0.18,
        metres_per_pixel);
    let distance_resolve = 1.0 - smoothstep(120.0, 420.0, water_view_depth);
    // Slope decays exponentially towards the horizon instead of being clipped
    // at a fixed range. Far water is not calm — its waves are simply smaller
    // than a pixel, and flattening the normal is what stops them aliasing into
    // a shimmering band.
    let slope_strength = mix(0.05, 1.0, exp(-water_view_depth * 0.011));
    let spectral_slope = spectrum.xy * slope_strength;
    let spectrum_normal = normalize(vec3<f32>(-spectral_slope.x, 1.0, -spectral_slope.y));
    let wake_normal = normalize(vec3<f32>(-wake.slope.x, 1.0, -wake.slope.y));
    let spectral_weight = clamp(material.simulation_params.x, 0.0, 1.0);
    let combined_wave_normal = normalize(mix(
        normalize(mix(wave.normal, spectrum_normal, spectral_weight)),
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
    var refracted = textureSampleLevel(scene_color, sampler_linear, refr_uv, 0.0).rgb;
    let rt_refr = upsample_rt(screen_uv, coverage, 1);
    refracted = mix(refracted, rt_refr.rgb, rt_refr.a);

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
    // Light that entered the water, scattered off what is suspended in it, and
    // came back out. The single-scattering albedo is far more saturated than
    // the absorption tint, which is exactly why a deep sea reads as blue-green
    // rather than black: absorption says what is *removed*, and this says what
    // comes back. It needs no visible bed, so unlike refraction below it
    // survives into open water — without it the surface goes bland the moment
    // it is too deep to see through.
    let single_scatter = (vec3<f32>(1.0) - transmittance)
        * sigma_s / max(sigma_t, vec3<f32>(1e-5))
        * ((sun_scatter + moon_scatter) / PI + env_up);
    let tint = mix(material.shallow_color.rgb, material.deep_color.rgb,
        smoothstep(0.5, max(material.surface_params.y * 6.0, 2.0), authored_depth));
    var transmitted = refracted * tint * transmittance;

    // Light entering the back of a crest and leaving towards the viewer. The
    // term peaks when looking into the sun through a raised wave, which is what
    // gives a backlit swell its green translucency. It is consumed by the
    // diffuse response below rather than added to the refracted path, so it is
    // not counted twice.
    let sss_modifier = vec3<f32>(0.9, 1.15, 0.85);
    // Both terms describe light that has travelled through the water towards
    // the eye, so both are gated on the viewer facing the sun. The reference
    // leaves the second one ungated, which in a physically scaled pipeline lays
    // a constant blue-green wash over the whole sea and goes badly discoloured
    // under a low sun, where the wash is lit by orange light.
    let into_sun = pow(max(dot(sun_l, -v), 0.0), 4.0);
    let sss_height = max(0.0, input.wave_height + 2.5) * into_sun
        * pow(max(0.5 - 0.5 * dot(sun_l, n), 0.0), 3.0);
    let sss_near = 0.5 * pow(ndotv, 2.0) * into_sun;

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
    // Whitecap coverage. The 0.75 scale and the distance fade are the
    // reference's: foam is a thin film that stops resolving long before the
    // waves carrying it do, so it has to disappear faster than the geometry.
    let crest_foam = smoothstep(0.0, 1.0, spectrum.a * 0.75)
        * exp(-water_view_depth * 0.0075)
        * smoothstep(0.5, 2.0, authored_depth);
    let foam_amount = clamp(max(max(shore_foam, crest_foam), wake.foam), 0.0, 1.0);
    let wet_band = (1.0 - smoothstep(0.0, max(material.surface_params.y * 2.5, 1.5),
        shore_distance)) * (0.35 + foam_amount * 0.65);
    transmitted *= 1.0 - wet_band * 0.16;

    // Foam is a diffuse dielectric scatterer, not a bright emitter. Its colour
    // is a warm off-white well below unity so lit whitecaps land in range
    // rather than clipping the moment the sun hits them.
    let foam_color = vec3<f32>(0.502, 0.412, 0.355);

    let roughness_map = 0.5 * (
        textureSample(tex_orm, sampler_linear, detail_uv_a).g
        + textureSample(tex_orm, sampler_linear, detail_uv_b).g * 0.65
        + textureSample(tex_orm, sampler_linear, detail_uv_c).g * 0.35) / 1.5;
    let unresolved_energy = 1.0 - min(slope_resolve, distance_resolve);
    let distance_roughness = unresolved_energy * 0.28;

    // Fresnel is evaluated against the authored base roughness, then roughness
    // itself responds to foam. Doing it in this order matches the reference and
    // keeps the two from chasing each other.
    let base_roughness = clamp(material.absorption_roughness.w, 0.04, 1.0);
    var fresnel_scalar = ocean_fresnel(ndotv, base_roughness);
    // A back-facing water fragment means one of two very different things. Seen
    // from below the surface it is the underside, and past the critical angle
    // it becomes a perfect mirror — the Snell window. Seen from above it is
    // just a wave that choppiness has folded over, and treating that as a
    // mirror turns the fold into a white shard of reflected sky. Only the
    // camera's own position tells the two apart.
    let viewed_from_below = view.camera_pos.y < input.world_position.y;
    if !front_facing && viewed_from_below {
        let eta = 1.333;
        let sin_transmitted_sq = eta * eta * (1.0 - ndotv * ndotv);
        fresnel_scalar = mix(fresnel_scalar, 1.0,
            smoothstep(0.96, 1.02, sin_transmitted_sq));
    }
    // Two different quantities share the name "roughness" here. The authored
    // one above characterises the microfacet distribution and stays fixed. This
    // one only decides how blurred the reflection is, and foam pushes it up
    // because a whitecap scatters the sky rather than mirroring it.
    let reflection_roughness = clamp(
        (1.0 - fresnel_scalar) * foam_amount + 0.4
            + roughness_map * 0.08 + distance_roughness,
        0.04,
        1.0,
    );

    let reflection_dir = reflect(-v, n);
    let environment = textureSampleLevel(env_cube, env_sampler, reflection_dir,
        reflection_roughness * ENV_MAX_MIP).rgb * light.ibl_intensity;
    let ssr = trace_ssr(input.world_position + n * 0.05, reflection_dir);
    let ssr_weight = ssr.a * clamp(material.surface_params.w, 0.0, 1.0);
    let rt = upsample_rt(screen_uv, coverage, 0);
    let rt_strength = clamp(material.volume_params.z, 0.0, 1.0);
    // SSR owns the near field where it is confident. The traced ray fills the
    // rest; the environment cube is the miss. `rt.a` is hit confidence.
    let traced = mix(environment, rt.rgb, rt.a * rt_strength);
    var reflected = mix(traced, ssr.rgb, ssr_weight);

    let debug_mode = material.volume_params.w;
    if debug_mode > 0.5 && debug_mode < 1.5 {
        // VV-A: SSR hit (green), miss (red), brightness is confidence.
        let hit = vec3<f32>(0.08, 0.85, 0.18) * ssr.a;
        let miss = vec3<f32>(0.85, 0.12, 0.10) * (1.0 - ssr.a);
        reflected = hit + miss;
    } else if debug_mode > 1.5 {
        // Source: SSR (blue), RT hit (yellow), environment (magenta).
        let ssr_c = vec3<f32>(0.15, 0.35, 0.95) * ssr_weight;
        let rt_c = vec3<f32>(0.95, 0.82, 0.12) * (1.0 - ssr_weight) * rt.a * rt_strength;
        let env_c = vec3<f32>(0.85, 0.15, 0.75) * (1.0 - ssr_weight) * (1.0 - rt.a * rt_strength);
        reflected = ssr_c + rt_c + env_c;
    }

    // How much of the surface response is reflection rather than everything
    // underneath it. This is the split that decides whether the sea looks like
    // water, so it uses the grazing-mirror curve rather than the suppressed one
    // the direct sun highlight needs.
    let reflectance = environment_fresnel(ndotv, reflection_roughness);
    let submerged = 1.0 - reflectance;

    // Forward scatter through a crest, which only exists when looking towards
    // the sun. Its tint is the water's own, so it belongs with the volume.
    //
    // The reference writes the surrounding diffuse as a bare `0.5 * ndotl`
    // because its light colour is around one. Somnium's sun carries physical
    // intensity, so the same expression would make the sea a mid-grey diffuse
    // reflector and wash it out. Everything here is weighted by an actual
    // albedo and normalised by pi, the convention the rest of the engine's
    // direct lighting already uses.
    let sun_mask = smith_masking_shadowing(max(dot(n, v), 2e-5), base_roughness);
    let scatter_albedo = material.shallow_color.rgb * sss_modifier;
    let crest_glow = scatter_albedo * (sss_height + sss_near) / (1.0 + sun_mask)
        * light.color * shadow / PI;

    // Refraction is the only part that needs the bed to be visible, so it is
    // the only part that fades out with depth. The volume scatter and the crest
    // glow carry open water on their own.
    let shallow_transmission = 1.0 - smoothstep(0.5, 6.0, authored_depth);
    let below_surface = transmitted * shallow_transmission + single_scatter + crest_glow;

    // Where foam covers the surface it replaces the water body entirely: a
    // whitecap is air and droplets, and nothing below it reaches the eye.
    let sun_ndotl = max(dot(n, sun_l), 0.0);
    let moon_ndotl = max(dot(n, moon_l), 0.0);
    let foam_response = foam_color
        * (env_up + (light.color * shadow * sun_ndotl + moon_color * moon_ndotl) / PI);

    let direct = ocean_specular(n, v, sun_l, light.color * shadow, base_roughness, fresnel_scalar)
        + ocean_specular(n, v, moon_l, moon_color, base_roughness, fresnel_scalar);

    let final_color = mix(below_surface, foam_response, foam_amount) * submerged
        + reflected * reflectance
        + direct;

    if debug_mode > 0.5 {
        return ShadeOutput(vec4<f32>(min(reflected, vec3<f32>(60000.0)), coverage));
    }

    return ShadeOutput(
        vec4<f32>(min(final_color, vec3<f32>(60000.0)), coverage),
    );
}

@fragment
fn fs_prepass(input: VertexOutput, @builtin(front_facing) front_facing: bool) -> PrepassOutput {
    let sdf_cells = sdf_at(input.uv);
    let cell_metres = (material.bounds.z - material.bounds.x)
        / f32(max(textureDimensions(shore_sdf).x, 1u));
    let signed_shore_distance = sdf_cells * cell_metres;
    let coverage_width = max(fwidth(signed_shore_distance), cell_metres * 0.35);
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
    let spectrum = spectral_gradient(water_coord);
    let wake = evaluate_wake(water_coord);
    let water_view_depth = view_depth(input.world_position);
    let shortest_wave = max(min(material.wave_params.x, material.wave_params.y) * 0.70, 0.5);
    let slope_resolve = 1.0 - smoothstep(shortest_wave * 0.035, shortest_wave * 0.18,
        metres_per_pixel);
    let distance_resolve = 1.0 - smoothstep(120.0, 420.0, water_view_depth);
    let slope_strength = mix(0.05, 1.0, exp(-water_view_depth * 0.011));
    let spectral_slope = spectrum.xy * slope_strength;
    let spectrum_normal = normalize(vec3<f32>(-spectral_slope.x, 1.0, -spectral_slope.y));
    let wake_normal = normalize(vec3<f32>(-wake.slope.x, 1.0, -wake.slope.y));
    let spectral_weight = clamp(material.simulation_params.x, 0.0, 1.0);
    let combined_wave_normal = normalize(mix(
        normalize(mix(wave.normal, spectrum_normal, spectral_weight)),
        wake_normal,
        clamp(material.wake_params.y * 0.55, 0.0, 0.75),
    ));
    let resolved_wave_normal = normalize(mix(vec3<f32>(0.0, 1.0, 0.0), combined_wave_normal,
        min(slope_resolve, distance_resolve)));
    let texture_time = frame.current_time;
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
    let detail_strength = 0.24 * (1.0 - smoothstep(75.0, 360.0, water_view_depth));
    var n = normalize(mix(resolved_wave_normal, mapped, detail_strength));
    let v = normalize(view.camera_pos - input.world_position);
    if dot(n, v) < 0.0 { n = -n; }
    let ndotv = max(dot(n, v), 1e-4);
    let foam_width = max(material.surface_params.y * 8.0, 6.0);
    let foam_noise = clamp(
        textureSample(tex_orm, sampler_linear, detail_uv_b * 0.63).r * 0.65
        + textureSample(tex_normal, sampler_linear, detail_uv_a * 0.47).x * 0.35,
        0.0,
        1.0,
    );
    let shore_distance = max(sdf_cells, 0.0) * cell_metres;
    let breaker_distance = 1.1 + 0.75 * sin(frame.current_time * 0.72
        + dot(water_coord, vec2<f32>(0.071, 0.043)));
    let breaker = 1.0 - smoothstep(0.45, 2.0, abs(shore_distance - breaker_distance));
    let shore_band = 1.0 - smoothstep(0.0, foam_width, shore_distance);
    let coord = vec2<i32>(input.clip_pos.xy);
    let base_depth = textureLoad(depth_texture, coord, 0);
    let base_has_backdrop = base_depth < 0.9999;
    let screen_size = vec2<f32>(textureDimensions(depth_texture));
    let screen_uv = input.clip_pos.xy / screen_size;
    let base_world = reconstruct_world(screen_uv, min(base_depth, 0.9999));
    let backdrop_distance = select(authored_depth, distance(input.world_position, base_world), base_has_backdrop);
    let depth_contact = select(
        0.0,
        1.0 - smoothstep(0.04, 1.35, backdrop_distance),
        base_has_backdrop,
    );
    let shore_foam = max(
        shore_band * clamp(0.28 + breaker * 0.82 + foam_noise * 0.38, 0.0, 1.0),
        depth_contact * (0.62 + foam_noise * 0.28),
    );
    let crest_foam = smoothstep(0.0, 1.0, spectrum.a * 0.75)
        * exp(-water_view_depth * 0.0075)
        * smoothstep(0.5, 2.0, authored_depth);
    let foam_amount = clamp(max(max(shore_foam, crest_foam), wake.foam), 0.0, 1.0);
    let roughness_map = 0.5 * (
        textureSample(tex_orm, sampler_linear, detail_uv_a).g
        + textureSample(tex_orm, sampler_linear, detail_uv_b).g * 0.65
        + textureSample(tex_orm, sampler_linear, detail_uv_c).g * 0.35) / 1.5;
    let unresolved_energy = 1.0 - min(slope_resolve, distance_resolve);
    let distance_roughness = unresolved_energy * 0.28;
    let base_roughness = clamp(material.absorption_roughness.w, 0.04, 1.0);
    var fresnel_scalar = ocean_fresnel(ndotv, base_roughness);
    let viewed_from_below = view.camera_pos.y < input.world_position.y;
    if !front_facing && viewed_from_below {
        let eta = 1.333;
        let sin_transmitted_sq = eta * eta * (1.0 - ndotv * ndotv);
        fresnel_scalar = mix(fresnel_scalar, 1.0,
            smoothstep(0.96, 1.02, sin_transmitted_sq));
    }
    let reflection_roughness = clamp(
        (1.0 - fresnel_scalar) * foam_amount + 0.4
            + roughness_map * 0.08 + distance_roughness,
        0.04,
        1.0,
    );
    return PrepassOutput(
        vec4<f32>(n.xz * 0.5 + 0.5, min(water_view_depth, 60000.0), coverage),
        clamp(input.screen_velocity, vec2<f32>(-1.0), vec2<f32>(1.0)),
        reflection_roughness,
    );
}
