// ── View Uniform ─────────────────────────────────────────────────────────────
struct View {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    view_matrix:   mat4x4<f32>,
    camera_pos:    vec3<f32>,
    _padding:      f32,
    time:          f32,
    _pad1:         vec2<f32>,
}

@group(0) @binding(0) var<storage, read> view: View;
@group(0) @binding(1) var depth_texture: texture_depth_2d;

// ── Water Component ─────────────────────────────────────────────────────────
struct WaterMaterial {
    deep_color: vec4<f32>,
    shallow_color: vec4<f32>,
    edge_color: vec4<f32>,
    clarity: f32,
    edge_scale: f32,
    amplitude: f32,
    coord_scale: vec2<f32>,
    coord_offset: vec2<f32>,
    wave_dir_a: vec2<f32>,
    wave_dir_b: vec2<f32>,
    wave_blend: f32,
}

@group(1) @binding(0) var<uniform> material: WaterMaterial;

// ── Instance Data ───────────────────────────────────────────────────────────
struct Instance {
    model: mat4x4<f32>,
    _pad: vec4<f32>, // matching alignment
}

@group(2) @binding(0) var<storage, read> instances: array<Instance>;

// ── Textures ────────────────────────────────────────────────────────────────
@group(3) @binding(0) var tex_base_color: texture_2d<f32>;
@group(3) @binding(1) var tex_normal: texture_2d<f32>;
@group(3) @binding(2) var tex_orm: texture_2d<f32>;
@group(3) @binding(3) var sampler_linear: sampler;

// -- Sun, shadows, environment, scene colour (Phase 22) ----------------------
// Layout mirrors `DirectionalLight` in shading.wgsl so the same GPU buffer
// serves both passes. The water used to invent its own light vector, which is
// why its highlight never agreed with the rest of the frame.
struct DirectionalLight {
    direction:       vec3<f32>,   // points TOWARD the sun
    _pad0:           f32,
    color:           vec3<f32>,
    _pad1:           f32,
    view_proj:       array<mat4x4<f32>, 4>,
    cascade_splits:  vec4<f32>,
    shadow_map_size: f32,
    ibl_intensity:   f32,
    sun_angular_radius:         f32,
    _pad2_z:         f32,
}

@group(0) @binding(2) var<storage, read> light: DirectionalLight;
@group(0) @binding(3) var shadow_atlas:   texture_depth_2d;
@group(0) @binding(4) var shadow_sampler: sampler_comparison;
@group(0) @binding(5) var env_cube:       texture_cube<f32>;
@group(0) @binding(6) var env_sampler:    sampler;
@group(0) @binding(7) var scene_color:    texture_2d<f32>;

const PI: f32 = 3.14159265;
/// Highest mip of the prefiltered environment cubemap (matches ibl.rs).
const ENV_MAX_MIP: f32 = 5.0;
/// Beyond this many metres of water the transmitted light is entirely
/// scattering. Capping stops open water -- where the depth buffer still holds
/// the far plane -- from reporting a depth of thousands of units.
const MAX_WATER_DEPTH: f32 = 60.0;
/// Spectral absorption of clear seawater, roughly in the observed ratio: red is
/// swallowed first, blue travels furthest. Scaled by the material's clarity.
const ABSORPTION: vec3<f32> = vec3<f32>(0.45, 0.10, 0.045);

fn get_cascade_index(view_depth: f32) -> u32 {
    if view_depth < light.cascade_splits.x { return 0u; }
    if view_depth < light.cascade_splits.y { return 1u; }
    if view_depth < light.cascade_splits.z { return 2u; }
    return 3u;
}

fn atlas_uv(cascade: u32, uv: vec2<f32>) -> vec2<f32> {
    var offsets = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0), vec2<f32>(0.5, 0.0),
        vec2<f32>(0.0, 0.5), vec2<f32>(0.5, 0.5),
    );
    return uv * 0.5 + offsets[cascade];
}

/// 3x3 PCF against the cascade atlas. Same scheme as shading.wgsl so the water
/// and the geometry under it agree on where the shadow edge falls.
fn sample_shadow(world_pos: vec3<f32>, view_depth: f32) -> f32 {
    let cascade    = get_cascade_index(view_depth);
    let light_clip = light.view_proj[cascade] * vec4<f32>(world_pos, 1.0);
    let ndc        = light_clip.xyz / light_clip.w;

    let uv            = vec2<f32>(ndc.x * 0.5 + 0.5, 1.0 - (ndc.y * 0.5 + 0.5));
    let atlas_coord   = atlas_uv(cascade, uv);
    let compare_depth = ndc.z;

    if any(atlas_coord < vec2<f32>(0.0)) || any(atlas_coord > vec2<f32>(1.0)) || compare_depth > 1.0 {
        return 1.0;
    }

    let texel_size = 1.0 / light.shadow_map_size;
    var shadow = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
            shadow += textureSampleCompare(
                shadow_atlas, shadow_sampler,
                atlas_coord + offset, compare_depth,
            );
        }
    }
    return shadow / 9.0;
}

// ── Noise Functions ─────────────────────────────────────────────────────────
fn random2d(v: vec2<f32>) -> f32 {
    return fract(sin(dot(v, vec2<f32>(12.9898,78.233))) * 43758.5453123);
}

fn random2di(v: vec2<f32>) -> f32 {
    return random2d(floor(v));
}

fn cubic_hermite_curve_2d(x: vec2<f32>) -> vec2<f32> {
    return smoothstep(vec2<f32>(0.0), vec2<f32>(1.0), x);
}

fn vnoise2d(v: vec2<f32>) -> f32 {
    let i = floor(v);
    let f = fract(v);
    let a = random2di(i);
    let b = random2di(i + vec2<f32>(1.0, 0.0));
    let c = random2di(i + vec2<f32>(0.0, 1.0));
    let d = random2di(i + vec2<f32>(1.0, 1.0));
    let u = cubic_hermite_curve_2d(f);
    return mix(a, b, u.x) + (c - a) * u.y * (1.0 - u.x) + (d - b) * u.x * u.y;
}

fn fbm_half(v2: vec2<f32>) -> f32 {
    let m2 = mat2x2<f32>(vec2<f32>(0.8, 0.6), vec2<f32>(-0.6, 0.8));
    var p = v2;
    var f = 0.5000 * vnoise2d(p); p = m2 * p * 2.02;
    f = f + 0.2500 * vnoise2d(p);
    return f / 0.9375;
}

fn fbm(v2: vec2<f32>) -> f32 {
    let m2 = mat2x2<f32>(vec2<f32>(0.8, 0.6), vec2<f32>(-0.6, 0.8));
    var p = v2;
    var f = 0.5000 * vnoise2d(p); p = m2 * p * 2.02;
    f = f + 0.2500 * vnoise2d(p); p = m2 * p * 2.03;
    f = f + 0.1250 * vnoise2d(p); p = m2 * p * 2.01;
    f = f + 0.0625 * vnoise2d(p);
    return f / 0.9375;
}

// ── Water Functions ─────────────────────────────────────────────────────────
fn wave(p: vec2<f32>) -> f32 {
    let time = view.time * 0.5 + 23.0;
    let time_x = time / 1.0;
    let time_y = time / 0.5;
    let wave_len_x = 2.0;
    let wave_len_y = 5.0;
    let wave_y = cos(p.y / wave_len_y + time_y);
    let wave_x = smoothstep(1.0, 0.0, abs(sin(p.x / wave_len_x + wave_y + time_x)));
    let n = fbm(p) / 2.0 - 1.0;
    return wave_x + n;
}

fn sample_directional_wave(p: vec2<f32>, time: f32, dir: vec2<f32>) -> f32 {
    let rotated_p = vec2<f32>(
        -(p.x * dir.x + p.y * dir.y),
        p.y * dir.x - p.x * dir.y
    );
    var result = wave((rotated_p - time) * 0.3) * 0.3;
    result = result + wave((rotated_p + time) * 0.4) * 0.3;
    result = result + wave((rotated_p + time) * 0.5) * 0.2;
    result = result + wave((rotated_p - time) * 0.6) * 0.2;
    return result;
}

const FADE_IN: f32 = 0.85;

fn get_wave_height(p: vec2<f32>) -> f32 {
    let time = view.time / 2.0;
    var wave_b = sample_directional_wave(p, time, material.wave_dir_b);
    if material.wave_blend < FADE_IN {
        let wave_a = sample_directional_wave(p, time, material.wave_dir_a);
        let blend = smoothstep(0.0, FADE_IN, material.wave_blend);
        wave_b = mix(wave_a, wave_b, blend);
    }
    return material.amplitude * wave_b;
}

fn uv_to_coord(uv: vec2<f32>) -> vec2<f32> {
    return material.coord_offset + (uv * material.coord_scale);
}

// ── Vertex Shader ───────────────────────────────────────────────────────────
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
    @location(2) base_world_position: vec3<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    let instance = instances[in.instance_index];
    let world_pos_4 = instance.model * vec4<f32>(in.position, 1.0);
    
    let w_pos = uv_to_coord(in.uv);
    let height = get_wave_height(w_pos);
    
    // Normal is straight up initially
    let world_position = world_pos_4.xyz + vec3<f32>(0.0, 1.0, 0.0) * height;
    
    var out: VertexOutput;
    out.world_position = world_position;
    out.uv = in.uv;
    out.base_world_position = world_pos_4.xyz;
    out.clip_pos = view.view_proj * vec4<f32>(world_position, 1.0);
    return out;
}

// ── Fragment Shader ─────────────────────────────────────────────────────────

// Convert depth texture value to linear View-Z.
fn depth_ndc_to_view_z(ndc_depth: f32) -> f32 {
    return 0.0; // we'll implement this properly below.
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let w_pos = uv_to_coord(in.uv);
    let height = get_wave_height(w_pos);
    
    // Reconstruct true per-pixel world position to fix triangle faceting
    let true_world_position = in.base_world_position + vec3<f32>(0.0, 1.0, 0.0) * height;
    
    // Compute analytical normal
    let delta = 0.5;
    let height_dx = get_wave_height(w_pos + vec2<f32>(delta, 0.0));
    let height_dz = get_wave_height(w_pos + vec2<f32>(0.0, delta));
    let world_normal = normalize(vec3<f32>(height - height_dx, delta, height - height_dz));
    
    let tangent = normalize(vec3<f32>(delta, height_dx - height, 0.0));
    let bitangent = normalize(vec3<f32>(0.0, height_dz - height, delta));
    let tbn = mat3x3<f32>(tangent, bitangent, world_normal);
    
    // Dual panning to break up tiling
    let time_offset1 = view.time * vec2<f32>(0.015, 0.01);
    let time_offset2 = view.time * vec2<f32>(-0.01, 0.02);
    
    let tex_uv1 = w_pos * 0.4 + time_offset1;
    let tex_uv2 = w_pos * 0.3 + time_offset2;
    
    // Sample textures twice and blend
    let base_color1 = textureSample(tex_base_color, sampler_linear, tex_uv1).rgb;
    let base_color2 = textureSample(tex_base_color, sampler_linear, tex_uv2).rgb;
    let base_color = mix(base_color1, base_color2, 0.5);
    
    let normal_map1 = textureSample(tex_normal, sampler_linear, tex_uv1).xyz * 2.0 - 1.0;
    let normal_map2 = textureSample(tex_normal, sampler_linear, tex_uv2).xyz * 2.0 - 1.0;
    let normal_map = normalize(normal_map1 + normal_map2);
    
    let orm1 = textureSample(tex_orm, sampler_linear, tex_uv1);
    let orm2 = textureSample(tex_orm, sampler_linear, tex_uv2);
    let orm = mix(orm1, orm2, 0.5);
    
    // Mix the geometric normal with the normal map for a balanced look
    let raw_pbr_normal = normalize(tbn * normal_map);
    let pbr_normal = normalize(mix(world_normal, raw_pbr_normal, 0.6)); // softened normal map intensity
    
    // -- Scene behind the surface --------------------------------------------
    let tex_coords  = vec2<i32>(in.clip_pos.xy);
    let screen_size = vec2<f32>(textureDimensions(depth_texture));
    let screen_uv   = in.clip_pos.xy / screen_size;

    let opaque_depth_ndc = textureLoad(depth_texture, tex_coords, 0);
    let ndc_opaque = vec4<f32>(screen_uv.x * 2.0 - 1.0, 1.0 - screen_uv.y * 2.0, opaque_depth_ndc, 1.0);
    var world_opaque = view.inv_view_proj * ndc_opaque;
    world_opaque = world_opaque / world_opaque.w;

    // The depth buffer clears to 1.0, so anything at the far plane means there
    // is no geometry under the water at all. That case needs saying out loud:
    // otherwise the "refracted" sample is the sky, and enough of it survives in
    // the blue channel to turn open ocean into a swimming pool.
    let has_backdrop = opaque_depth_ndc < 0.9999;

    // Distance the view ray travels through water before hitting anything.
    // `raw` still drives the shoreline term, which wants the true value; the
    // capped one drives absorption, where an uncapped far-plane reading used to
    // force every open-water pixel to the same flat "infinitely deep" colour.
    let raw_depth_diff = select(
        MAX_WATER_DEPTH,
        max(distance(true_world_position, world_opaque.xyz), 0.0),
        has_backdrop,
    );
    let depth_diff = min(raw_depth_diff, MAX_WATER_DEPTH);

    // -- Surface vectors -------------------------------------------------------
    let n = pbr_normal;
    let v = normalize(view.camera_pos - true_world_position);
    let l = normalize(light.direction);
    let h = normalize(l + v);

    let ndotl = max(dot(n, l), 0.0);
    let ndotv = max(dot(n, v), 1e-4);
    let ndoth = max(dot(n, h), 0.0);
    let vdoth = max(dot(v, h), 0.0);

    let view_pos   = view.view_matrix * vec4<f32>(true_world_position, 1.0);
    let view_depth = -view_pos.z; // right-handed: Z is negative in front of camera
    let shadow     = sample_shadow(true_world_position, view_depth);

    // Sky irradiance for the diffuse and scattering terms: the roughest mip of
    // the prefiltered environment, looked up straight up.
    let sky_irradiance = textureSampleLevel(
        env_cube, env_sampler, vec3<f32>(0.0, 1.0, 0.0), ENV_MAX_MIP,
    ).rgb * light.ibl_intensity;

    // -- Transmitted: what comes up through the surface ------------------------
    // Refraction offsets the scene lookup by the surface normal's horizontal
    // component, so whatever is under the water wobbles with the waves. The
    // divisor keeps distant water from smearing.
    let refract_offset = n.xz * 0.03 / (1.0 + view_depth * 0.02);
    let refr_uv    = clamp(screen_uv + refract_offset, vec2<f32>(0.0), vec2<f32>(1.0));
    let refracted  = textureSampleLevel(scene_color, sampler_linear, refr_uv, 0.0).rgb;

    // Beer-Lambert, per channel: red is absorbed first, which is what gives deep
    // water its blue-green cast instead of a flat painted-on blue.
    // With no backdrop nothing is transmitted at all — all of it is scattering.
    let absorption = select(
        vec3<f32>(0.0),
        exp(-depth_diff * max(material.clarity, 0.0) * ABSORPTION),
        has_backdrop,
    );

    // Subsurface scattering: light that entered, bounced and came back out. The
    // base-colour texture modulates it by luminance, so the artwork still adds
    // detail without fighting the water's hue.
    let detail  = 0.75 + 0.5 * dot(base_color, vec3<f32>(0.3333));
    let scatter = material.deep_color.rgb * detail
        * (light.color * ndotl * shadow * 0.25 + sky_irradiance * 0.5);

    var transmitted = mix(scatter, refracted * material.shallow_color.rgb, absorption);

    // Shoreline foam where the water meets geometry.
    let foam = material.edge_color.rgb * (light.color * shadow * 0.25 + sky_irradiance * 0.5);
    transmitted = mix(foam, transmitted, smoothstep(0.0, max(material.edge_scale, 1e-3), raw_depth_diff));

    // -- Reflected: sky and sun off the surface --------------------------------
    let roughness = clamp(orm.g, 0.02, 1.0);
    let a  = roughness * roughness;
    let a2 = a * a;

    // GGX normal distribution.
    let d_denom  = ndoth * ndoth * (a2 - 1.0) + 1.0;
    let dist_ggx = a2 / (PI * d_denom * d_denom);

    // Smith geometry term. This was missing, so the specular had no shadowing or
    // masking and the trailing `* 2.0` was compensating for it.
    let k   = a * 0.5;
    let g_v = ndotv / (ndotv * (1.0 - k) + k);
    let g_l = ndotl / (ndotl * (1.0 - k) + k);
    let geom = g_v * g_l;

    // Water's index of refraction gives F0 = 0.02.
    let f0     = vec3<f32>(0.02);
    let f_spec = f0 + (1.0 - f0) * pow(1.0 - vdoth, 5.0);
    // The reflection uses the view angle, not the half-vector: looking straight
    // down you see into the water, at grazing angles it turns into a mirror.
    // This Fresnel split is most of what makes water read as water.
    let f_env  = f0 + (1.0 - f0) * pow(1.0 - ndotv, 5.0);

    let r   = reflect(-v, n);
    let env = textureSampleLevel(env_cube, env_sampler, r, roughness * ENV_MAX_MIP).rgb
        * light.ibl_intensity;

    // Sun glint. Clamped because a near-mirror surface produces enormous values
    // on a handful of pixels, which read as fireflies once bloom gets hold of them.
    let sun_spec = min(
        (dist_ggx * geom * f_spec) / max(4.0 * ndotv * ndotl, 1e-4) * light.color * ndotl * shadow,
        vec3<f32>(40.0),
    );

    let final_color = mix(transmitted, env, f_env) + sun_spec;
    return vec4<f32>(final_color, 1.0);
}
