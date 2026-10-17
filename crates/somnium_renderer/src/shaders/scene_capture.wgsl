//!include "spot_cookie.wgsl"
// D2 (engine features record): a small, cheap view of the scene into a texture, for a pocket mirror, a monitor, a
// screen. A forward pass over the frame's opaque instances: albedo and emissive, the sun (scaled by how far outdoors
// the view is: there are no shadows to keep it out of a room), every lamp within reach, the environment's ambient, and
// the level's height fog lit as the froxel fog lights it. No shadows and no bounce light: a mirror's small picture of a
// dark room, not a second frame.
//!include "global_pool.wgsl"
//!include "atmosphere.wgsl"

struct Capture {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    eye: vec3<f32>,
    sun_scale: f32,
    fog_density: f32,
    ambient_scale: f32,
    fog_asymmetry: f32,
    fog_height_falloff: f32,
    fog_base_height: f32,
    fog_sky_occlusion: f32,
    /// How far the main view's fog reaches: the sky is behind that much of it.
    fog_distance: f32,
    _pad: f32,
}

/// What the environment sends along `dir`, scaled as the ambient: the sky behind everything.
fn environment(dir: vec3<f32>, mip: f32) -> vec3<f32> {
    return textureSampleLevel(env_cube, env_sampler, dir, mip).rgb * light.ibl_intensity * capture.ambient_scale;
}

@group(1) @binding(0) var<uniform> capture: Capture;
@group(1) @binding(1) var tex_sampler: sampler;
@group(1) @binding(2) var env_cube: texture_cube<f32>;
@group(1) @binding(3) var env_sampler: sampler;
@group(1) @binding(4) var transmittance_lut: texture_2d<f32>;
@group(1) @binding(5) var multiscatter_lut: texture_2d<f32>;
@group(1) @binding(6) var lut_sampler: sampler;

/// The fog's own light seen along `dir` from `at`: what it fades toward. As `volumetric.wgsl` lights its fog medium
/// (a grey scatterer): the sun through the air, by the fog's phase function, and the sky's multiple scattering, times
/// the sun's illuminance; the sky's share dimmed indoors as `fog_sky_occlusion` dims it there.
fn fog_light(dir: vec3<f32>, at: vec3<f32>) -> vec3<f32> {
    let sun_dir = normalize(light.direction);
    let r = GROUND_RADIUS + max(at.y, 0.0) * 0.001;
    if ray_hits_ground(r, sun_dir.y) {
        return vec3<f32>(0.0);
    }
    let sun = sample_transmittance(transmittance_lut, lut_sampler, r, sun_dir.y);
    let sky = sample_multiscatter(multiscatter_lut, lut_sampler, r, sun_dir.y);
    let g = clamp(capture.fog_asymmetry, -0.95, 0.95);
    let denom = 1.0 + g * g - 2.0 * g * dot(dir, sun_dir);
    let phase = (1.0 - g * g) / (4.0 * PI * max(denom * sqrt(max(denom, 1e-6)), 1e-6));
    let sky_vis = mix(1.0, capture.sun_scale, saturate(capture.fog_sky_occlusion));
    return light.color * (sun * phase * capture.sun_scale + sky * sky_vis);
}

/// The height fog's optical depth from `a` to `b`: its density falls by e every `fog_height_falloff` metres above its
/// base, integrated in closed form along the segment.
fn fog_depth(a: vec3<f32>, b: vec3<f32>) -> f32 {
    if capture.fog_density <= 0.0 {
        return 0.0;
    }
    let len = distance(a, b);
    let h = capture.fog_height_falloff;
    if h <= 0.0 {
        return capture.fog_density * len;
    }
    let ya = max(a.y - capture.fog_base_height, 0.0);
    let yb = max(b.y - capture.fog_base_height, 0.0);
    let dy = yb - ya;
    let mean = select((exp(-ya / h) - exp(-yb / h)) * h / dy, exp(-ya / h), abs(dy) < 1e-3);
    return capture.fog_density * len * mean;
}

/// `color` at `at` as the eye sees it through the fog between.
fn fogged(color: vec3<f32>, at: vec3<f32>) -> vec3<f32> {
    let through = exp(-fog_depth(capture.eye, at));
    return color * through + fog_light(normalize(at - capture.eye), capture.eye) * (1.0 - through);
}

const ENV_MAX_MIP: f32 = 5.0;
const PI: f32 = 3.14159265;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) material_id: u32,
}

@vertex
fn vs_main(@builtin(vertex_index) v_idx: u32, @builtin(instance_index) inst_idx: u32) -> VOut {
    // Programmable vertex pulling from the global pool, as the visibility pass does.
    let instance = instances[inst_idx];
    let index = indices[instance.index_offset + v_idx];
    let vertex = vertices[instance.vertex_offset + index];
    let world = instance.model * vec4<f32>(vertex.pos_x, vertex.pos_y, vertex.pos_z, 1.0);
    var out: VOut;
    out.clip = capture.view_proj * world;
    out.world_pos = world.xyz;
    out.normal = (instance.model * vec4<f32>(vertex.norm_x, vertex.norm_y, vertex.norm_z, 0.0)).xyz;
    out.uv = vec2<f32>(vertex.u, vertex.v);
    out.material_id = instance.material_id;
    return out;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    // Derivatives at top level, where control flow is uniform (as visibility.wgsl does).
    let ddx_uv = dpdx(in.uv);
    let ddy_uv = dpdy(in.uv);
    let material = materials[in.material_id];
    var albedo = material.base_color.rgb;
    var alpha = material.base_color.a;
    if material.albedo_map >= 0 {
        let s = textureSampleGrad(textures[material.albedo_map], tex_sampler, in.uv, ddx_uv, ddy_uv);
        albedo *= s.rgb;
        alpha *= s.a;
    }
    if material.alpha_cutoff > 0.0 && alpha < material.alpha_cutoff {
        discard;
    }

    // Two-sided: whichever face it sees faces it.
    var n = normalize(in.normal);
    let to_eye = capture.eye - in.world_pos;
    if dot(n, to_eye) < 0.0 {
        n = -n;
    }

    var light_in = textureSampleLevel(env_cube, env_sampler, n, ENV_MAX_MIP).rgb * light.ibl_intensity * capture.ambient_scale;
    light_in += light.color * max(dot(n, normalize(light.direction)), 0.0) / PI * capture.sun_scale;
    for (var i = 0u; i < cluster_params.num_local_lights; i++) {
        let ll = local_lights[i];
        var to_light = ll.position_ws - in.world_pos;
        if ll.light_type == 4u {
            // A tube: the nearest point of its axis.
            let axis = normalize(ll.direction_ws);
            let along = clamp(dot(in.world_pos - ll.position_ws, axis), -ll._pad1, ll._pad1);
            to_light = ll.position_ws + axis * along - in.world_pos;
        }
        let dist = length(to_light);
        if dist > ll.range || dist < 1.0e-4 {
            continue;
        }
        let l = to_light / dist;
        let ratio = dist / ll.range;
        let window = saturate(1.0 - ratio * ratio * ratio * ratio);
        var atten = window * window / max(dist * dist, 0.0064);
        if ll.light_type == 1u {
            atten *= spot_shape(ll.direction_ws, -l, ll.spot_cos_outer, ll.spot_cos_inner, ll._pad2);
        } else if ll.light_type == 2u || ll.light_type == 3u {
            // Panels and discs light one side.
            atten *= saturate(dot(-l, normalize(ll.direction_ws)) * 4.0);
        }
        light_in += ll.color * atten * max(dot(n, l), 0.0) / PI;
    }

    var emissive = vec3<f32>(material.emissive_r, material.emissive_g, material.emissive_b);
    if material.emissive_map >= 0 {
        emissive *= textureSampleGrad(textures[material.emissive_map], tex_sampler, in.uv, ddx_uv, ddy_uv).rgb;
    }
    let color = fogged(albedo * (1.0 - 0.8 * material.metallic) * light_in + emissive, in.world_pos);
    // Finite always: one NaN would sit in the picture until the next capture.
    let finite = select(vec3<f32>(0.0), color, color == color);
    return vec4<f32>(min(finite, vec3<f32>(60000.0)), 1.0);
}

// The sky behind everything: a triangle over the whole target at the far plane, drawn first.
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: SkyOut;
    out.clip = vec4<f32>(xy, 1.0, 1.0);
    out.ndc = xy;
    return out;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let far = capture.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far.xyz / far.w - capture.eye);
    let sky = fogged(environment(dir, 2.0), capture.eye + dir * capture.fog_distance);
    return vec4<f32>(select(vec3<f32>(0.0), sky, sky == sky), 1.0);
}
