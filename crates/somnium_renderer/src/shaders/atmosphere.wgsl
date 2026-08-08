// Phase 24C: physically based atmospheric scattering (Hillaire 2020).
//
// Shared module, concatenated into whichever shader needs a sky. Everything
// here is a pure function taking its textures as parameters, so the same code
// serves the LUT-building passes and the consumers without fighting over bind
// group slots.
//
// Replaces three separate hardcoded sky gradients. Those were unit-less colour
// ramps that could not respond to the sun, which is why lowering the sun could
// never produce night — see context.md §22.1.
//
// Units: kilometres for distance, so the numbers below match the published
// coefficients directly. Radiance comes out per unit of sun illuminance, and
// callers multiply by the sun's lux to land in cd/m².

// ── Earth's atmosphere ──────────────────────────────────────────────────────
const GROUND_RADIUS: f32 = 6360.0;
const ATMOS_RADIUS:  f32 = 6460.0;

/// Rayleigh scattering at sea level (km⁻¹). Blue scatters ~5.7x more than red,
/// which is the whole reason the sky is blue and sunsets are red.
const RAYLEIGH_SCATTERING = vec3<f32>(5.802e-3, 13.558e-3, 33.1e-3);
const RAYLEIGH_SCALE_H: f32 = 8.0;

/// Mie: aerosols. Nearly colourless, strongly forward-scattering, which gives
/// the bright halo around the sun and the whiteness near the horizon.
const MIE_SCATTERING: f32 = 3.996e-3;
const MIE_EXTINCTION: f32 = 4.4e-3;
const MIE_SCALE_H:    f32 = 1.2;
const MIE_G:          f32 = 0.8;

/// Ozone absorbs but does not scatter. Without it the sky stays washed-out and
/// never reaches the deep blue of a real zenith, and twilight loses its purple.
const OZONE_ABSORPTION = vec3<f32>(0.650e-3, 1.881e-3, 0.085e-3);
const OZONE_CENTER: f32 = 25.0;
const OZONE_WIDTH:  f32 = 15.0;

/// Density of each species at `altitude` km, relative to sea level.
fn atmosphere_density(altitude: f32) -> vec3<f32> {
    let rayleigh = exp(-altitude / RAYLEIGH_SCALE_H);
    let mie      = exp(-altitude / MIE_SCALE_H);
    // Ozone is a layer, not an exponential falloff: a tent peaking around 25 km.
    let ozone    = max(0.0, 1.0 - abs(altitude - OZONE_CENTER) / OZONE_WIDTH);
    return vec3<f32>(rayleigh, mie, ozone);
}

/// Total extinction (out-scattering + absorption) at `altitude`, km⁻¹.
fn atmosphere_extinction(altitude: f32) -> vec3<f32> {
    let d = atmosphere_density(altitude);
    return RAYLEIGH_SCATTERING * d.x
         + vec3<f32>(MIE_EXTINCTION) * d.y
         + OZONE_ABSORPTION * d.z;
}

/// Rayleigh phase: symmetric, slightly stronger forward and backward.
fn rayleigh_phase(cos_theta: f32) -> f32 {
    return 3.0 / (16.0 * 3.14159265) * (1.0 + cos_theta * cos_theta);
}

/// Henyey-Greenstein phase for Mie, biased sharply forward at g = 0.8.
fn mie_phase(cos_theta: f32) -> f32 {
    let g  = MIE_G;
    let g2 = g * g;
    let denom = 1.0 + g2 - 2.0 * g * cos_theta;
    return (1.0 - g2) / (4.0 * 3.14159265 * max(denom * sqrt(max(denom, 1e-4)), 1e-4));
}

/// Distance from a point at radius `r` with zenith cosine `mu` to the top of
/// the atmosphere. Negative discriminants are clamped, so a ray that misses
/// returns 0 rather than NaN.
fn distance_to_atmosphere_top(r: f32, mu: f32) -> f32 {
    let disc = r * r * (mu * mu - 1.0) + ATMOS_RADIUS * ATMOS_RADIUS;
    return max(-r * mu + sqrt(max(disc, 0.0)), 0.0);
}

/// Same, to the ground. Only meaningful when the ray actually hits.
fn distance_to_ground(r: f32, mu: f32) -> f32 {
    let disc = r * r * (mu * mu - 1.0) + GROUND_RADIUS * GROUND_RADIUS;
    return max(-r * mu - sqrt(max(disc, 0.0)), 0.0);
}

fn ray_hits_ground(r: f32, mu: f32) -> bool {
    return mu < 0.0 && (r * r * (mu * mu - 1.0) + GROUND_RADIUS * GROUND_RADIUS) >= 0.0;
}

// ── Transmittance LUT parameterisation (Bruneton) ────────────────────────────
//
// A plain (altitude, angle) mapping wastes almost all its resolution: nearly
// every interesting variation happens within a couple of degrees of the
// horizon. This warps the domain so texels concentrate there.

fn transmittance_r_mu_to_uv(r: f32, mu: f32) -> vec2<f32> {
    let h   = sqrt(max(ATMOS_RADIUS * ATMOS_RADIUS - GROUND_RADIUS * GROUND_RADIUS, 0.0));
    let rho = sqrt(max(r * r - GROUND_RADIUS * GROUND_RADIUS, 0.0));
    let d   = distance_to_atmosphere_top(r, mu);
    let d_min = ATMOS_RADIUS - r;
    let d_max = rho + h;
    return vec2<f32>((d - d_min) / max(d_max - d_min, 1e-6), rho / max(h, 1e-6));
}

fn transmittance_uv_to_r_mu(uv: vec2<f32>) -> vec2<f32> {
    let h   = sqrt(max(ATMOS_RADIUS * ATMOS_RADIUS - GROUND_RADIUS * GROUND_RADIUS, 0.0));
    let rho = h * uv.y;
    let r   = sqrt(rho * rho + GROUND_RADIUS * GROUND_RADIUS);
    let d_min = ATMOS_RADIUS - r;
    let d_max = rho + h;
    let d = d_min + uv.x * (d_max - d_min);
    var mu = 1.0;
    if d > 0.0 {
        mu = (h * h - rho * rho - d * d) / (2.0 * r * d);
    }
    return vec2<f32>(r, clamp(mu, -1.0, 1.0));
}

/// Ray-march the optical depth to the top of the atmosphere. Only used when
/// building the LUT; everything else samples the result.
fn compute_transmittance(r: f32, mu: f32) -> vec3<f32> {
    let steps = 40;
    let dist  = distance_to_atmosphere_top(r, mu);
    let dt    = dist / f32(steps);
    var optical_depth = vec3<f32>(0.0);
    for (var i = 0; i < steps; i = i + 1) {
        let t  = (f32(i) + 0.5) * dt;
        // Law of cosines along the ray, giving radius at the sample point.
        let ri = sqrt(max(t * t + 2.0 * r * mu * t + r * r, 0.0));
        optical_depth += atmosphere_extinction(ri - GROUND_RADIUS) * dt;
    }
    return exp(-optical_depth);
}

fn sample_transmittance(
    lut: texture_2d<f32>,
    samp: sampler,
    r: f32,
    mu: f32,
) -> vec3<f32> {
    return textureSampleLevel(lut, samp, transmittance_r_mu_to_uv(r, mu), 0.0).rgb;
}

// ── Multiple scattering ─────────────────────────────────────────────────────
//
// Single scattering alone leaves the sky far too dark and shadowed sides of
// the world nearly black. Hillaire's approximation: assume light that has
// bounced more than once is isotropic and altitude-dependent only, so it fits
// in a small 2-D table indexed by altitude and sun angle.

fn multiscatter_uv(r: f32, mu_sun: f32) -> vec2<f32> {
    return vec2<f32>(
        mu_sun * 0.5 + 0.5,
        clamp((r - GROUND_RADIUS) / (ATMOS_RADIUS - GROUND_RADIUS), 0.0, 1.0),
    );
}

fn sample_multiscatter(
    lut: texture_2d<f32>,
    samp: sampler,
    r: f32,
    mu_sun: f32,
) -> vec3<f32> {
    return textureSampleLevel(lut, samp, multiscatter_uv(r, mu_sun), 0.0).rgb;
}

/// One ray-march through the atmosphere, returning radiance per unit of sun
/// illuminance. `ray_origin` is in planet-centred km.
fn raymarch_sky(
    transmittance_lut: texture_2d<f32>,
    multiscatter_lut:  texture_2d<f32>,
    samp:              sampler,
    ray_origin:        vec3<f32>,
    ray_dir:           vec3<f32>,
    sun_dir:           vec3<f32>,
    steps:             i32,
) -> vec3<f32> {
    let r  = length(ray_origin);
    let mu = dot(ray_origin, ray_dir) / r;

    // March to whichever boundary comes first.
    var march_len = distance_to_atmosphere_top(r, mu);
    if ray_hits_ground(r, mu) {
        march_len = distance_to_ground(r, mu);
    }
    if march_len <= 0.0 {
        return vec3<f32>(0.0);
    }

    let cos_theta = dot(ray_dir, sun_dir);
    let phase_r = rayleigh_phase(cos_theta);
    let phase_m = mie_phase(cos_theta);

    var luminance    = vec3<f32>(0.0);
    var transmittance = vec3<f32>(1.0);
    let dt = march_len / f32(steps);

    for (var i = 0; i < steps; i = i + 1) {
        let t = (f32(i) + 0.5) * dt;
        let pos = ray_origin + ray_dir * t;
        let ri  = length(pos);
        let alt = ri - GROUND_RADIUS;

        let density    = atmosphere_density(alt);
        let extinction = atmosphere_extinction(alt);
        let scatter_r  = RAYLEIGH_SCATTERING * density.x;
        let scatter_m  = MIE_SCATTERING * density.y;

        let mu_sun = dot(pos, sun_dir) / ri;

        // Shadow of the planet itself: no direct sun below the horizon.
        var sun_visibility = 1.0;
        if ray_hits_ground(ri, mu_sun) {
            sun_visibility = 0.0;
        }
        let sun_transmittance =
            sample_transmittance(transmittance_lut, samp, ri, mu_sun) * sun_visibility;

        let in_scatter_single = sun_transmittance
            * (scatter_r * phase_r + vec3<f32>(scatter_m * phase_m));

        // Multiply-scattered light is treated as isotropic, so it takes the
        // scattering coefficients without a phase function.
        let ms = sample_multiscatter(multiscatter_lut, samp, ri, mu_sun);
        let in_scatter_multi = ms * (scatter_r + vec3<f32>(scatter_m));

        let in_scatter = in_scatter_single + in_scatter_multi;

        // Energy-conserving integration of a segment with constant extinction
        // (Hillaire): analytic rather than a Riemann sum, so 32 steps suffice.
        let step_transmittance = exp(-extinction * dt);
        let safe_extinction = max(extinction, vec3<f32>(1e-7));
        let integrated = (in_scatter - in_scatter * step_transmittance) / safe_extinction;

        luminance += transmittance * integrated;
        transmittance *= step_transmittance;
    }

    return luminance;
}

// ── Phase 24D: what is left when the sun goes down ──────────────────────────

/// Cheap 3-D hash, for star placement.
fn star_hash(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.3183099 + vec3<f32>(0.1, 0.1, 0.1));
    q += dot(q, q.yzx + 19.19);
    return fract((q.x + q.y) * q.z);
}

/// A star field in the given direction, in cd/m².
///
/// Quantising the direction into cells and placing at most one star per cell
/// keeps density even; without that, stars clump badly toward the poles of
/// whatever parameterisation is used.
fn star_field(dir: vec3<f32>) -> vec3<f32> {
    let cell_scale = 340.0;
    let cell = floor(dir * cell_scale);
    let h = star_hash(cell);
    if h < 0.987 {
        return vec3<f32>(0.0);
    }

    // Position the star within its cell so the grid never becomes visible.
    let offset = vec3<f32>(star_hash(cell + 11.0), star_hash(cell + 23.0), star_hash(cell + 37.0));
    let star_dir = normalize((cell + offset) / cell_scale);
    let falloff = pow(max(dot(dir, star_dir), 0.0), 40000.0);

    // Rough spectral spread: most stars read white, some warm, a few blue.
    let tint_pick = star_hash(cell + 53.0);
    let tint = mix(vec3<f32>(1.0, 0.85, 0.7), vec3<f32>(0.75, 0.85, 1.0), tint_pick);
    let brightness = mix(0.002, 0.02, star_hash(cell + 71.0));
    return tint * brightness * falloff;
}

/// Low-frequency night light: airglow plus the moon's scattered halo.
///
/// This is the part that belongs in the environment cubemap. At 256² per face a
/// texel spans about a third of a degree, so anything sharper than that smears
/// into a blob — which is exactly what stars did on the first attempt.
fn night_sky_ambient(dir: vec3<f32>, moon_dir: vec3<f32>, moon_strength: f32) -> vec3<f32> {
    let halo = pow(max(dot(dir, moon_dir), 0.0), 700.0) * 6.0 * moon_strength;
    const AIRGLOW = vec3<f32>(0.0002, 0.00025, 0.0004);
    return vec3<f32>(halo) + AIRGLOW;
}

/// High-frequency sky detail: the sun disc, the moon disc and the stars.
///
/// Drawn analytically over the background at full screen resolution rather than
/// baked into the cubemap, for two reasons. Resolution is one — a half-degree
/// disc cannot survive a third-of-a-degree texel. The other is energy: the
/// shading pass already computes the sun's specular highlight from the analytic
/// light, so a sun disc in the cubemap would be counted a second time through
/// the IBL specular term.
fn sky_detail(
    dir: vec3<f32>,
    sun_dir: vec3<f32>,
    sun_illuminance: f32,
    moon_dir: vec3<f32>,
    moon_strength: f32,
) -> vec3<f32> {
    var result = vec3<f32>(0.0);

    // cos(0.2666°) — the sun's true angular radius.
    const SUN_COS_RADIUS: f32 = 0.99998916;
    let cos_sun = dot(dir, sun_dir);
    if cos_sun > SUN_COS_RADIUS {
        // Limb darkening: the disc is dimmer at its edge, which is what stops
        // it reading as a flat sticker pasted on the sky.
        let edge = saturate((cos_sun - SUN_COS_RADIUS) / (1.0 - SUN_COS_RADIUS));
        let limb = 0.4 + 0.6 * sqrt(edge);
        result += vec3<f32>(1.0, 0.95, 0.85) * sun_illuminance * 0.6 * limb;
    }

    const MOON_COS_RADIUS: f32 = 0.9999897;
    if dot(dir, moon_dir) > MOON_COS_RADIUS {
        result += vec3<f32>(2500.0, 2450.0, 2300.0) * moon_strength;
    }

    return result + star_field(dir) * moon_strength;
}

/// Moon disc and the airglow floor, in cd/m²./// Moon disc and the airglow floor, in cd/m².
///
/// A real night sky is not black — a full moon is about 2 500 cd/m² across a
/// half-degree disc, and even a moonless sky glows faintly. Without this the
/// scene simply goes to zero and auto-exposure chases noise.
fn night_sky(dir: vec3<f32>, moon_dir: vec3<f32>, moon_illuminance: f32) -> vec3<f32> {
    // cos(0.26°) — the moon subtends almost exactly the same angle as the sun.
    let cos_moon = 0.9999897;
    let d = dot(dir, moon_dir);
    var moon = vec3<f32>(0.0);
    if d > cos_moon {
        moon = vec3<f32>(2500.0, 2450.0, 2300.0) * moon_illuminance;
    }
    // Soft glow around the moon, from scattering in the air.
    let halo = pow(max(d, 0.0), 700.0) * 6.0 * moon_illuminance;

    const AIRGLOW = vec3<f32>(0.0002, 0.00025, 0.0004);
    return moon + vec3<f32>(halo) + AIRGLOW + star_field(dir);
}
