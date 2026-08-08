// Phase 24C: builds the transmittance and multiple-scattering LUTs.
//
// Concatenated after `atmosphere.wgsl`, which supplies the physics. Both LUTs
// depend only on the atmosphere's own parameters — not on the sun or the
// camera — so they are built once at startup and never touched again.

@group(0) @binding(0) var transmittance_src: texture_2d<f32>;
@group(0) @binding(1) var lut_sampler:       sampler;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0)       uv:   vec2<f32>,
}

/// Full-screen triangle, shared by both LUT passes.
@vertex
fn vs_lut(@builtin(vertex_index) vid: u32) -> VOut {
    var xs = array<f32, 3>(-1.0,  3.0, -1.0);
    var ys = array<f32, 3>(-1.0, -1.0,  3.0);
    let p  = vec2<f32>(xs[vid], ys[vid]);
    return VOut(vec4<f32>(p, 0.0, 1.0), vec2<f32>((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5));
}

/// Transmittance: how much sunlight survives from a point to space.
@fragment
fn fs_transmittance(in: VOut) -> @location(0) vec4<f32> {
    let r_mu = transmittance_uv_to_r_mu(in.uv);
    return vec4<f32>(compute_transmittance(r_mu.x, r_mu.y), 1.0);
}

/// Number of directions sampled per texel of the multiple-scattering LUT.
const MS_DIRECTIONS: i32 = 64;
const MS_STEPS: i32 = 20;

/// Multiple scattering, following Hillaire's second-order-plus-geometric-series
/// approximation.
///
/// For each altitude and sun angle, light arriving from every direction is
/// integrated assuming isotropic phase, giving both the second-order term and
/// the fraction that would scatter again. Summing the geometric series then
/// stands in for every remaining order at once, which is what makes this
/// affordable: one small table instead of an unbounded recursion.
@fragment
fn fs_multiscatter(in: VOut) -> @location(0) vec4<f32> {
    let mu_sun = in.uv.x * 2.0 - 1.0;
    let r = mix(GROUND_RADIUS, ATMOS_RADIUS, in.uv.y);
    let sun_dir = vec3<f32>(0.0, mu_sun, sqrt(max(1.0 - mu_sun * mu_sun, 0.0)));
    let pos = vec3<f32>(0.0, r, 0.0);

    var luminance_total = vec3<f32>(0.0);
    var scatter_total   = vec3<f32>(0.0);

    for (var d = 0; d < MS_DIRECTIONS; d = d + 1) {
        // Fibonacci sphere: even coverage without needing a random source.
        let fi = (f32(d) + 0.5) / f32(MS_DIRECTIONS);
        let cos_phi = 1.0 - 2.0 * fi;
        let sin_phi = sqrt(max(1.0 - cos_phi * cos_phi, 0.0));
        let theta = 3.14159265 * (1.0 + 2.2360679) * f32(d);
        let ray_dir = vec3<f32>(cos(theta) * sin_phi, cos_phi, sin(theta) * sin_phi);

        let mu = dot(normalize(pos), ray_dir);
        var march_len = distance_to_atmosphere_top(r, mu);
        let hits_ground = ray_hits_ground(r, mu);
        if hits_ground {
            march_len = distance_to_ground(r, mu);
        }
        if march_len <= 0.0 {
            continue;
        }

        let dt = march_len / f32(MS_STEPS);
        var transmittance = vec3<f32>(1.0);
        var luminance     = vec3<f32>(0.0);
        var scattered     = vec3<f32>(0.0);

        for (var i = 0; i < MS_STEPS; i = i + 1) {
            let t   = (f32(i) + 0.5) * dt;
            let p   = pos + ray_dir * t;
            let ri  = length(p);
            let alt = ri - GROUND_RADIUS;

            let density    = atmosphere_density(alt);
            let extinction = atmosphere_extinction(alt);
            let scattering = RAYLEIGH_SCATTERING * density.x
                           + vec3<f32>(MIE_SCATTERING * density.y);

            let mu_s = dot(p, sun_dir) / ri;
            var sun_vis = 1.0;
            if ray_hits_ground(ri, mu_s) {
                sun_vis = 0.0;
            }
            let sun_t = sample_transmittance(transmittance_src, lut_sampler, ri, mu_s) * sun_vis;

            let step_t = exp(-extinction * dt);
            let safe_e = max(extinction, vec3<f32>(1e-7));

            // Isotropic phase (1/4pi) — the defining assumption of the approximation.
            let in_scatter = scattering * sun_t * (1.0 / (4.0 * 3.14159265));
            luminance += transmittance * (in_scatter - in_scatter * step_t) / safe_e;

            // Track scattering without the sun term: this is the fraction that
            // feeds the next order.
            let bounce = scattering * (1.0 / (4.0 * 3.14159265));
            scattered += transmittance * (bounce - bounce * step_t) / safe_e;

            transmittance *= step_t;
        }

        // Light bouncing off the ground contributes too; a mid grey albedo is
        // a reasonable stand-in for an unknown world.
        if hits_ground {
            const GROUND_ALBEDO: f32 = 0.3;
            let p  = pos + ray_dir * march_len;
            let ri = length(p);
            let mu_s = dot(p, sun_dir) / ri;
            if mu_s > 0.0 {
                let sun_t = sample_transmittance(transmittance_src, lut_sampler, ri, mu_s);
                luminance += transmittance * sun_t * mu_s * GROUND_ALBEDO / 3.14159265;
            }
        }

        // 4pi / N is the solid angle each sampled direction represents.
        let weight = 4.0 * 3.14159265 / f32(MS_DIRECTIONS);
        luminance_total += luminance * weight;
        scatter_total   += scattered * weight;
    }

    // Geometric series over all remaining scattering orders. `scatter_total`
    // stays well below 1 for a real atmosphere, so this converges.
    let ms = luminance_total / max(vec3<f32>(1.0) - scatter_total, vec3<f32>(1e-4));
    return vec4<f32>(ms, 1.0);
}
