//!include "global_pool.wgsl"
//!include "brdf.wgsl"
// The preview studio (`pass/preview_studio.rs`): a few draws, lit and tone-mapped into a small target of their own.
// The material editor's turning sphere is one; an asset's thumbnail is another.
//
// Geometry comes from the global pool and the material from the bindless material buffer, as for every other pass, so
// the preview cannot disagree with the level about what a material is. Light is the deferred shading's own BRDF
// (`evaluate_brdf`, with this frame's energy terms) under a fixed three-light rig turned with the camera, plus the
// engine's prefiltered environment with its brightness normalised: a night level must not hand the studio a black sky,
// and a noon one must not wash it out. No shadows, fog or bounce light.

struct Studio {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    eye: vec3<f32>,
    exposure: f32,
    /// Linear colour behind the subject (the panel it sits in).
    backdrop: vec3<f32>,
    /// Mean luminance the environment is scaled to.
    env_level: f32,
    /// Key, fill and rim: xyz the direction toward the light (world space), then its colour.
    light_dir: array<vec4<f32>, 3>,
    light_color: array<vec4<f32>, 3>,
}

@group(1) @binding(0) var<uniform> studio: Studio;
@group(1) @binding(1) var<storage, read> draws: array<Instance>;
@group(1) @binding(2) var tex_sampler: sampler;
@group(1) @binding(3) var env_cube: texture_cube<f32>;
@group(1) @binding(4) var env_sampler: sampler;

const ENV_MAX_MIP: f32 = 5.0;
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
/// `Instance._padding` bit: the draw's material is alpha-blended (the renderer's `material_blend`).
const DRAW_BLENDED: u32 = 1u;

/// What scales the environment to `studio.env_level`: its blurriest mip, averaged over the six axes.
fn environment_gain() -> f32 {
    var sum = 0.0;
    sum += dot(textureSampleLevel(env_cube, env_sampler, vec3<f32>(1.0, 0.0, 0.0), ENV_MAX_MIP).rgb, LUMA);
    sum += dot(textureSampleLevel(env_cube, env_sampler, vec3<f32>(-1.0, 0.0, 0.0), ENV_MAX_MIP).rgb, LUMA);
    sum += dot(textureSampleLevel(env_cube, env_sampler, vec3<f32>(0.0, 1.0, 0.0), ENV_MAX_MIP).rgb, LUMA);
    sum += dot(textureSampleLevel(env_cube, env_sampler, vec3<f32>(0.0, -1.0, 0.0), ENV_MAX_MIP).rgb, LUMA);
    sum += dot(textureSampleLevel(env_cube, env_sampler, vec3<f32>(0.0, 0.0, 1.0), ENV_MAX_MIP).rgb, LUMA);
    sum += dot(textureSampleLevel(env_cube, env_sampler, vec3<f32>(0.0, 0.0, -1.0), ENV_MAX_MIP).rgb, LUMA);
    return min(studio.env_level / max(sum / 6.0, 1.0e-5), 1.0e4);
}

/// The environment along `dir`, half desaturated (a coloured sky should tint a preview, not dye it) and scaled.
fn environment(dir: vec3<f32>, mip: f32, gain: f32) -> vec3<f32> {
    let raw = textureSampleLevel(env_cube, env_sampler, dir, mip).rgb;
    return mix(vec3<f32>(dot(raw, LUMA)), raw, 0.5) * gain;
}

/// ACES filmic (Narkowicz), as `postprocess.wgsl` has it.
fn aces_film(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + vec3<f32>(0.03))) / (x * (2.43 * x + vec3<f32>(0.59)) + vec3<f32>(0.14)),
        vec3<f32>(0.0), vec3<f32>(1.0));
}

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) material_id: u32,
    @location(4) @interpolate(flat) flags: u32,
    @location(5) @interpolate(flat) env_gain: f32,
}

@vertex
fn vs_main(@builtin(vertex_index) v_idx: u32, @builtin(instance_index) draw_idx: u32) -> VOut {
    let draw = draws[draw_idx];
    let index = indices[draw.index_offset + v_idx];
    let vertex = vertices[draw.vertex_offset + index];
    let world = draw.model * vec4<f32>(vertex.pos_x, vertex.pos_y, vertex.pos_z, 1.0);
    var out: VOut;
    out.clip = studio.view_proj * world;
    out.world_pos = world.xyz;
    out.normal = (draw.model * vec4<f32>(vertex.norm_x, vertex.norm_y, vertex.norm_z, 0.0)).xyz;
    out.uv = vec2<f32>(vertex.u, vertex.v);
    out.material_id = draw.material_id;
    out.flags = draw._padding;
    out.env_gain = environment_gain();
    return out;
}

@fragment
fn fs_main(in: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // Derivatives at top level, where control flow is uniform.
    let ddx_uv = dpdx(in.uv);
    let ddy_uv = dpdy(in.uv);
    let dp1 = dpdx(in.world_pos);
    let dp2 = dpdy(in.world_pos);
    // The level's BRDF energy terms, as `shading.wgsl` sets them this frame.
    brdf_multiscatter = (cluster_params.shading_mode & 64u) != 0u;
    brdf_rough_diffuse = (cluster_params.shading_mode & 128u) != 0u;

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
    var roughness = material.roughness;
    var metallic = material.metallic;
    if material.metallic_roughness_map >= 0 {
        let mr = textureSampleGrad(textures[material.metallic_roughness_map], tex_sampler, in.uv, ddx_uv, ddy_uv);
        roughness = mr.g;
        metallic = mr.b;
    }
    roughness = clamp(roughness, 0.05, 1.0);
    var occlusion = 1.0;
    if material.occlusion_map >= 0 {
        occlusion = textureSampleGrad(textures[material.occlusion_map], tex_sampler, in.uv, ddx_uv, ddy_uv).r;
    }

    // Whichever face the camera sees faces it (a plane, a two-sided leaf, the back of a glass ball).
    var geo_n = normalize(in.normal);
    if !front {
        geo_n = -geo_n;
    }
    var n = geo_n;
    if material.normal_map >= 0 {
        // Tangent frame from screen derivatives, as `transparent.wgsl` builds it; a degenerate UV keeps the mesh normal.
        let det = ddx_uv.x * ddy_uv.y - ddy_uv.x * ddx_uv.y;
        let t_raw = (dp1 * ddy_uv.y - dp2 * ddx_uv.y) * sign(det);
        let t_ortho = t_raw - geo_n * dot(t_raw, geo_n);
        if abs(det) > 1.0e-12 && dot(t_ortho, t_ortho) > 1.0e-16 {
            let t = normalize(t_ortho);
            let b = cross(geo_n, t) * sign(det);
            let tn = textureSampleGrad(textures[material.normal_map], tex_sampler, in.uv, ddx_uv, ddy_uv).xyz * 2.0 - 1.0;
            n = normalize((t * tn.x + b * tn.y) * material.normal_scale + geo_n * max(tn.z, 0.05));
        }
    }

    let v = normalize(studio.eye - in.world_pos);
    var surface: Surface;
    surface.albedo = albedo;
    surface.roughness = roughness;
    surface.metallic = metallic;
    surface.normal = n;
    surface.view_dir = v;
    surface.f0 = mix(vec3<f32>(0.04), albedo, metallic);
    surface.occlusion = occlusion;
    surface.bent_normal = n;

    var direct = vec3<f32>(0.0);
    for (var i = 0u; i < 3u; i++) {
        direct += evaluate_brdf(surface, normalize(studio.light_dir[i].xyz)) * studio.light_color[i].rgb;
    }
    // Split-sum environment: the prefiltered mip for the roughness, and the analytic scale and bias.
    let n_dot_v = max(dot(n, v), 1.0e-3);
    let ab = env_brdf_ab(roughness, n_dot_v);
    let specular_weight = surface.f0 * ab.x + ab.y;
    let r = reflect(-v, n);
    let specular_env = environment(r, roughness * ENV_MAX_MIP, in.env_gain) * specular_weight;
    let diffuse_env = environment(n, ENV_MAX_MIP, in.env_gain) * albedo * (1.0 - metallic)
        * (vec3<f32>(1.0) - specular_weight);
    var emissive = vec3<f32>(material.emissive_r, material.emissive_g, material.emissive_b);
    if material.emissive_map >= 0 {
        emissive *= textureSampleGrad(textures[material.emissive_map], tex_sampler, in.uv, ddx_uv, ddy_uv).rgb;
    }

    if (in.flags & DRAW_BLENDED) != 0u {
        // Glass as `transparent.wgsl` draws it: the surface's own light weighted by its coverage, the reflection by
        // Fresnel, and the rest of the backdrop let through. Premultiplied, so the blend adds the light and keeps
        // `1 - a` of what is behind; an 8-bit target clamps before blending, so dividing by `a` would lose light.
        let fresnel = surface.f0 + (vec3<f32>(1.0) - surface.f0) * pow(1.0 - n_dot_v, 5.0);
        let lit = (direct + diffuse_env) * alpha + specular_env * occlusion + emissive;
        let through = dot((vec3<f32>(1.0) - fresnel) * (1.0 - alpha), vec3<f32>(0.3333));
        let a = clamp(1.0 - through, 0.0, 1.0);
        let color = aces_film(lit * studio.exposure);
        return vec4<f32>(select(vec3<f32>(0.0), color, color == color), a);
    }

    let hdr = direct + (diffuse_env + specular_env) * occlusion + emissive;
    let color = aces_film(hdr * studio.exposure);
    // Finite always: one NaN would sit in the picture until the next edit.
    return vec4<f32>(select(vec3<f32>(0.0), color, color == color), 1.0);
}

// The backdrop: the panel's colour, lifted a little toward the middle so the subject sits in a pool of light.
struct BackdropOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_backdrop(@builtin(vertex_index) i: u32) -> BackdropOut {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: BackdropOut;
    out.clip = vec4<f32>(xy, 1.0, 1.0);
    out.ndc = xy;
    return out;
}

@fragment
fn fs_backdrop(in: BackdropOut) -> @location(0) vec4<f32> {
    let lift = 1.0 + 0.9 * (1.0 - smoothstep(0.0, 1.25, length(in.ndc * vec2<f32>(1.0, 1.15) + vec2<f32>(0.0, 0.12))));
    return vec4<f32>(studio.backdrop * lift, 1.0);
}
