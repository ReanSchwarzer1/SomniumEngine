// Phase 19A: environment cubemap generation.
//
// Two entry points, run once at startup (and again if the sun moves):
//
//   fs_sky       — renders the procedural sky into mip 0 of a cubemap.
//   fs_prefilter — GGX-prefilters mip 0 into mips 1..N, one roughness per mip,
//                  which is the "prefiltered environment map" half of Karis'
//                  split-sum approximation (SIGGRAPH 2013, *Real Shading in
//                  Unreal Engine 4*).
//
// The sky is captured from the same procedural function the shading pass draws
// as background, so reflections always match the visible sky — no HDRI asset
// needed, and it stays correct when the sun direction changes.

struct GenParams {
    /// Cube face being rendered, 0..5 (+X, -X, +Y, -Y, +Z, -Z).
    face: u32,
    /// Target roughness for this mip (prefilter only).
    roughness: f32,
    /// Source mip resolution, for the sample-count heuristic.
    _src_size: f32,
    _pad: f32,
    /// Direction TOWARD the sun, plus its colour.
    sun_direction: vec4<f32>,
    /// `.rgb` sun colour scaled by illuminance; `.w` sky-dome luminance scale.
    sun_color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: GenParams;
// Phase 24C: atmosphere LUTs, built once by `pass/atmosphere.rs`.
@group(0) @binding(3) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(4) var multiscatter_lut:  texture_2d<f32>;
@group(0) @binding(5) var atmos_sampler:     sampler;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0)       uv:   vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VOut {
    var xs = array<f32, 3>(-1.0,  3.0, -1.0);
    var ys = array<f32, 3>(-1.0, -1.0,  3.0);
    let p  = vec2(xs[vid], ys[vid]);
    let uv = vec2((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    return VOut(vec4(p, 0.0, 1.0), uv);
}

/// Map a face index + face UV to a world-space direction.
/// Standard Vulkan/D3D cube convention; the vertical flips matter or the sky
/// ends up mirrored in the reflections.
fn cube_dir(face: u32, uv: vec2<f32>) -> vec3<f32> {
    let st = uv * 2.0 - vec2(1.0);
    var d: vec3<f32>;
    switch face {
        case 0u:  { d = vec3( 1.0,  -st.y, -st.x); } // +X
        case 1u:  { d = vec3(-1.0,  -st.y,  st.x); } // -X
        case 2u:  { d = vec3( st.x,  1.0,   st.y); } // +Y
        case 3u:  { d = vec3( st.x, -1.0,  -st.y); } // -Y
        case 4u:  { d = vec3( st.x, -st.y,  1.0);  } // +Z
        default:  { d = vec3(-st.x, -st.y, -1.0);  } // -Z
    }
    return normalize(d);
}

/// The procedural sky, matching `shading.wgsl`'s background so reflections
/// agree with what the camera sees.
fn sky(ray_dir: vec3<f32>) -> vec3<f32> {
    // Phase 24C. This was three hardcoded colour constants; it is now a real
    // ray-march through a Rayleigh/Mie/ozone atmosphere. The difference that
    // matters is not the daytime look — it is that everything below now falls
    // out of the sun's own position and intensity, so dusk, night and the
    // reddening of a low sun are consequences rather than special cases.
    let sun_dir = normalize(params.sun_direction.xyz);

    // Camera altitude above sea level, in km. Held slightly off the ground so
    // the view ray never starts inside the planet.
    let view_pos = vec3<f32>(0.0, GROUND_RADIUS + 0.5, 0.0);

    // `.w` carries the sun's illuminance in lux (Phase 24A).
    let sun_illuminance = params.sun_color.w;

    var radiance = raymarch_sky(
        transmittance_lut, multiscatter_lut, atmos_sampler,
        view_pos, ray_dir, sun_dir, 32,
    ) * sun_illuminance;

    // Sharp features (sun disc, moon disc, stars) are deliberately absent
    // here and drawn over the background instead — see `sky_detail`. Keeping
    // them out also avoids double-counting the sun, whose specular highlight
    // the shading pass already computes from the analytic light.
    // Night fades in on the sun's *illuminance*, not its elevation. Dimming a
    // light and moving it below the horizon are different things, and the dial
    // in the inspector is intensity — so keying off elevation meant turning the
    // sun down to moonlight left a starless sky. 10 lux is roughly civil
    // twilight, the point where the eye starts picking out stars.
    let moon_dir = -sun_dir;
    let moon_strength = saturate(1.0 - sun_illuminance / 10.0);
    radiance += night_sky_ambient(ray_dir, moon_dir, moon_strength);

    return min(radiance, vec3<f32>(60000.0));
}

@fragment
fn fs_sky(in: VOut) -> @location(0) vec4<f32> {
    return vec4(sky(cube_dir(params.face, in.uv)), 1.0);
}

// ─── Prefilter ───────────────────────────────────────────────────────────────

@group(0) @binding(1) var src_cube: texture_cube<f32>;
@group(0) @binding(2) var src_samp: sampler;

const PI: f32 = 3.14159265359;
const SAMPLE_COUNT: u32 = 64u;

/// Van der Corput radical inverse — the low-discrepancy half of Hammersley.
fn radical_inverse_vdc(bits_in: u32) -> f32 {
    var bits = bits_in;
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return f32(bits) * 2.3283064365386963e-10;
}

fn hammersley(i: u32, n: u32) -> vec2<f32> {
    return vec2(f32(i) / f32(n), radical_inverse_vdc(i));
}

/// Sample a half-vector from the GGX distribution around `n`.
fn importance_sample_ggx(xi: vec2<f32>, n: vec3<f32>, roughness: f32) -> vec3<f32> {
    let a = roughness * roughness;
    let phi = 2.0 * PI * xi.x;
    let cos_theta = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
    let sin_theta = sqrt(1.0 - cos_theta * cos_theta);

    let h_tangent = vec3(sin_theta * cos(phi), sin_theta * sin(phi), cos_theta);

    // Build a tangent frame around n, switching the reference axis when n is
    // nearly parallel to Z (otherwise the cross product degenerates).
    var up_vec = vec3(0.0, 0.0, 1.0);
    if abs(n.z) >= 0.999 {
        up_vec = vec3(1.0, 0.0, 0.0);
    }
    let tangent   = normalize(cross(up_vec, n));
    let bitangent = cross(n, tangent);
    return normalize(tangent * h_tangent.x + bitangent * h_tangent.y + n * h_tangent.z);
}

@fragment
fn fs_prefilter(in: VOut) -> @location(0) vec4<f32> {
    let n = cube_dir(params.face, in.uv);
    // The split-sum approximation assumes the view direction equals the normal,
    // which is what makes a single prefiltered map usable from any angle.
    let v = n;

    var total  = vec3(0.0);
    var weight = 0.0;
    for (var i = 0u; i < SAMPLE_COUNT; i = i + 1u) {
        let xi = hammersley(i, SAMPLE_COUNT);
        let h  = importance_sample_ggx(xi, n, params.roughness);
        let l  = normalize(2.0 * dot(v, h) * h - v);

        let n_dot_l = dot(n, l);
        if n_dot_l > 0.0 {
            // Explicit LOD: this is inside a data-dependent branch, so
            // implicit-derivative sampling would be illegal in WGSL.
            total  += textureSampleLevel(src_cube, src_samp, l, 0.0).rgb * n_dot_l;
            weight += n_dot_l;
        }
    }
    return vec4(total / max(weight, 1e-4), 1.0);
}
