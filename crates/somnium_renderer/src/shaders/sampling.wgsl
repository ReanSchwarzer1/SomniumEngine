// Phase 24G: shared sampling patterns.
//
// Every stochastic technique that follows — PCSS, GTAO, TAA jitter, contact
// shadows — needs to pick sample positions, and picking them badly is what makes
// a one-sample-per-pixel effect look like static rather than like softness.
// Gathering the patterns here means they get chosen once and reused, instead of
// each pass inventing its own and drifting.
//
// The goal throughout is *blue* noise: error spread evenly across neighbouring
// pixels so a spatial filter or a temporal accumulator can resolve it. White
// noise clumps, and clumps survive filtering as blotches.

const SAMPLING_PI: f32 = 3.14159265;
const GOLDEN_ANGLE: f32 = 2.39996323;

/// Interleaved gradient noise (Jorge Jimenez).
///
/// One multiply-add and a fract, and the result is close enough to blue noise
/// for per-pixel rotation angles. This is the workhorse: rotating a fixed
/// sample pattern per pixel turns visible banding into fine dithering that a
/// blur or TAA removes.
fn interleaved_gradient_noise(pixel: vec2<f32>, frame: u32) -> f32 {
    // Advancing the pixel coordinate per frame decorrelates successive frames,
    // so TAA sees a new pattern each time rather than accumulating one bias.
    let p = pixel + 5.588238 * f32(frame % 64u);
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

/// Vogel disk: `count` points spiralling out with even area density.
///
/// Preferred over Poisson tables (which Somnium would have to ship and index)
/// and over uniform grids (which alias into visible rings). `rotation` comes
/// from [`interleaved_gradient_noise`], which is what turns the fixed spiral
/// into per-pixel noise.
fn vogel_disk_sample(index: u32, count: u32, rotation: f32) -> vec2<f32> {
    // The +0.5 offset keeps the first sample off the exact centre, where it
    // would carry no information about the neighbourhood.
    let radius = sqrt((f32(index) + 0.5) / f32(count));
    let theta = f32(index) * GOLDEN_ANGLE + rotation;
    return vec2<f32>(radius * cos(theta), radius * sin(theta));
}

/// Cosine-weighted hemisphere direction around `normal`.
///
/// Cosine weighting matches the Lambert term, so the samples are already
/// distributed the way the integral wants and no per-sample weight is needed.
fn cosine_hemisphere(normal: vec3<f32>, u: vec2<f32>) -> vec3<f32> {
    let r = sqrt(u.x);
    let theta = 2.0 * SAMPLING_PI * u.y;
    let disk = vec2<f32>(r * cos(theta), r * sin(theta));
    let z = sqrt(max(1.0 - u.x, 0.0));

    // Frisvad's branchless orthonormal basis. The sign trick avoids the
    // singularity a naive cross-with-up hits when the normal points up.
    let s = select(-1.0, 1.0, normal.z >= 0.0);
    let a = -1.0 / (s + normal.z);
    let b = normal.x * normal.y * a;
    let tangent = vec3<f32>(1.0 + s * normal.x * normal.x * a, s * b, -s * normal.x);
    let bitangent = vec3<f32>(b, s + normal.y * normal.y * a, -normal.y);

    return normalize(tangent * disk.x + bitangent * disk.y + normal * z);
}

/// R2 low-discrepancy sequence (Roberts). The 2-D analogue of the golden ratio.
///
/// Used for TAA's sub-pixel jitter: it fills the pixel more evenly than Halton
/// at low sample counts, so the image converges in fewer frames.
fn r2_sequence(index: u32) -> vec2<f32> {
    // Plastic-number reciprocals — the 2-D generalisation of 1/phi.
    const A1: f32 = 0.7548776662;
    const A2: f32 = 0.5698402910;
    return fract(vec2<f32>(A1, A2) * f32(index + 1u));
}

/// Halton sequence element, base 2 and 3.
///
/// Kept alongside R2 because Halton is what most published TAA jitter tables
/// use, so matching it makes comparisons against reference implementations
/// meaningful.
fn halton_2_3(index: u32) -> vec2<f32> {
    var x = 0.0;
    var f = 0.5;
    var i = index + 1u;
    while i > 0u {
        x += f * f32(i % 2u);
        i = i / 2u;
        f = f * 0.5;
    }

    var y = 0.0;
    var f3 = 1.0 / 3.0;
    var j = index + 1u;
    while j > 0u {
        y += f3 * f32(j % 3u);
        j = j / 3u;
        f3 = f3 / 3.0;
    }

    return vec2<f32>(x, y);
}
