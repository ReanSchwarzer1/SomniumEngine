// MORROWIND-C: composition is declared here rather than assembled by a
// `format!` of `include_str!` calls at this pass's construction site. The
// resolver (`somnium_shader`) emits each module once, in this order, and
// hoists every `enable` above everything.
//!include "atmosphere.wgsl"

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
    /// Cloud-deck coverage, 0..1.
    overcast: f32,
    /// Direction TOWARD the sun, plus its colour.
    sun_direction: vec4<f32>,
    /// `.rgb` sun colour scaled by illuminance; `.w` sky-dome luminance scale.
    sun_color: vec4<f32>,
    /// The moonless night (`NightSky`): x glow cd/m², y aurora, z Halo Road, w Thread. All zero draws nothing.
    night: vec4<f32>,
    /// `.rgb` the glow's colour.
    night_tint: vec4<f32>,
}

/// The Undersun's night (`NightSky`, Rimeborn): no moon. A glow over the dome, brighter overhead and toward the hidden
/// sun; the Halo Road, the Lid plates' lit edges, as a dashed ring through the sun across its bearing, with the gap of
/// the Drowning's fallen plate; the Thread, a thin line from the horizon below the sun up toward it; aurora, folded
/// curtains low on the side away from the sun, green over red. At cubemap resolution all of it is soft, as faint
/// things in a dark sky are.
fn undersun_night(dir: vec3<f32>, sun_dir: vec3<f32>) -> vec3<f32> {
    let glow = params.night.x;
    if glow <= 0.0 {
        return vec3<f32>(0.0);
    }
    let up = max(dir.y, 0.0);
    let above = smoothstep(-0.06, 0.02, dir.y);
    let toward_sun = max(dot(dir, sun_dir), 0.0);
    var c = params.night_tint.rgb * glow * (0.3 + 0.7 * sqrt(up) + 0.6 * toward_sun * toward_sun) * above;

    let sun_flat = normalize(vec3<f32>(sun_dir.x, 0.0, sun_dir.z) + vec3<f32>(1.0e-4, 0.0, 0.0));
    let across = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), sun_flat));
    let dir_flat = normalize(vec3<f32>(dir.x, 0.0, dir.z) + vec3<f32>(1.0e-4, 0.0, 0.0));

    // The Halo Road.
    let ring_normal = normalize(cross(sun_dir, across));
    let off_ring = dot(dir, ring_normal);
    let around = atan2(dot(dir, across), dot(dir, sun_dir));
    let plate = fract(around * 14.0 / 6.2831853);
    let seams = smoothstep(0.0, 0.06, plate) * smoothstep(0.0, 0.06, 1.0 - plate);
    let fallen = smoothstep(0.08, 0.16, abs(around - 2.2));
    let road = exp(-off_ring * off_ring / (2.0 * 0.007 * 0.007)) * seams * fallen * above;
    c += vec3<f32>(1.0, 0.9, 0.78) * glow * 7.0 * params.night.z * road;

    // The Thread.
    let off_plane = dot(dir, across);
    let sun_el = asin(clamp(sun_dir.y, -1.0, 1.0));
    let el = asin(clamp(dir.y, -1.0, 1.0));
    let sunward = smoothstep(0.0, 0.2, dot(dir_flat, sun_flat));
    let rising = smoothstep(-0.01, 0.03, el) * (1.0 - smoothstep(sun_el - 0.08, sun_el, el)) * (0.5 + 0.5 * (1.0 - el / max(sun_el, 0.05)));
    let thread = exp(-off_plane * off_plane / (2.0 * 0.0045 * 0.0045)) * rising * sunward;
    c += vec3<f32>(0.82, 0.9, 1.0) * glow * 9.0 * params.night.w * thread;

    // Aurora.
    let away = smoothstep(0.0, 0.5, -dot(dir_flat, sun_flat));
    let az = atan2(dir.x, dir.z);
    let folds = 0.55 + 0.45 * sin(az * 9.0 + 2.0 * sin(az * 3.7));
    let green = vec3<f32>(0.2, 1.0, 0.45) * smoothstep(0.03, 0.12, dir.y) * (1.0 - smoothstep(0.22, 0.5, dir.y));
    let red = vec3<f32>(1.0, 0.22, 0.28) * 0.35 * smoothstep(0.2, 0.45, dir.y) * (1.0 - smoothstep(0.45, 0.75, dir.y));
    c += (green + red) * folds * away * glow * 5.0 * params.night.y;
    return c;
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

    // Phase 25M-2: raymarch_sky calculates unit-less atmospheric scattering per lux of sun.
    // Multiply by sun_illuminance (which carries the sun's lux attenuated by atmosphere transmittance).
    // Transmittance fades smoothly across twilight (via HORIZON_FADE in sun.rs), causing the sky to
    // transition naturally from daylight (8000 cd/m²) -> sunset (500 cd/m²) -> night airglow floor.
    let sun_illuminance = params.sun_color.w;

    var radiance = raymarch_sky(
        transmittance_lut, multiscatter_lut, atmos_sampler,
        view_pos, ray_dir, sun_dir, 32,
    ) * sun_illuminance;

    // A cloud deck replaces the blue dome with the CIE overcast sky: neutral,
    // no sun disc, three times as bright at the zenith as at the horizon, and
    // lit from below only by the ground. Without this every reflection and all
    // ambient light under a grey sky came from a clear blue one. Its zenith is
    // tied to the clear zenith, so exposure holds as the deck closes.
    let cover = smoothstep(0.35, 0.95, params.overcast);
    if cover > 0.0 {
        let zenith = raymarch_sky(
            transmittance_lut, multiscatter_lut, atmos_sampler,
            view_pos, vec3<f32>(0.0, 1.0, 0.0), sun_dir, 16,
        ) * sun_illuminance;
        let lz = dot(zenith, vec3<f32>(0.2126, 0.7152, 0.0722)) * 1.6;
        let dome = select(0.2, (1.0 + 2.0 * ray_dir.y) / 3.0, ray_dir.y >= 0.0);
        radiance = mix(radiance, vec3<f32>(0.97, 1.0, 1.03) * lz * dome, cover);
    }

    let moon_dir = -sun_dir;
    let moon_strength = saturate(1.0 - sun_illuminance / 10.0);
    radiance += night_sky_ambient(ray_dir, moon_dir, moon_strength);
    radiance += undersun_night(ray_dir, sun_dir);

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
