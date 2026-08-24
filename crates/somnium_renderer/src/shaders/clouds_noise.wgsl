// Somnium Engine — Phase CONTROL-M: cloud noise and weather-map generation.
//
// Three compute entry points, all run rarely: the two 3-D noise volumes once
// per process, the weather map only when its authored parameters change.
// Nothing here runs per frame, which is why it can afford to be readable.
//
// ## References
//
// - Schneider & Vos, *The Real-Time Volumetric Cloudscapes of Horizon Zero
//   Dawn* (SIGGRAPH 2015) — the Perlin–Worley base eroded by higher-frequency
//   Worley, and the reason the base channel is a *remap* of Perlin by Worley
//   rather than a blend: the Worley term carves the billowy interior that
//   Perlin alone cannot produce.
// - Schneider, *Nubis* (SIGGRAPH 2017) — the weather map as a top-down field
//   sampled by world XZ.
//
// ## What is Somnium's own decision, stated as such
//
// The plan (§6.3) records that the widely quoted 128³/32³ resolutions and the
// coverage/type/precipitation channel split **could not be verified** from
// Guerrilla's own material, and refuses to launder a community reconstruction
// as a citation. So: **these resolutions and this channel assignment are
// Somnium's design decision.** 128³ base and 32³ detail are chosen because
// they are 8 MB and 128 KB at RGBA8, which is a rounding error next to the
// terrain atlas, and because a lower base loses the silhouette. The channel
// split is corroborated independently by Unity HDRP's cloud map, which is a
// top-down coverage-and-type field.

struct NoiseParams {
    /// Weather-map coverage bias, `0..1`.
    coverage: f32,
    /// Weather-map cloud-type bias, `0..1` — stratus through cumulonimbus.
    cloud_type: f32,
    /// Precipitation bias, `0..1`.
    precipitation: f32,
    /// Placement seed. Changing it reshuffles the weather field.
    seed: f32,
    /// Metres of world per weather-map texel, so the field has a real scale.
    weather_metres: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0) var<uniform> np: NoiseParams;
@group(0) @binding(1) var base_out:    texture_storage_3d<rgba8unorm, write>;
@group(0) @binding(2) var detail_out:  texture_storage_3d<rgba8unorm, write>;
@group(0) @binding(3) var weather_out: texture_storage_2d<rgba8unorm, write>;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Hashes
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn hash13(p3_in: vec3<f32>) -> f32 {
    var p3 = fract(p3_in * 0.1031);
    p3 += dot(p3, p3.zyx + 31.32);
    return fract((p3.x + p3.y) * p3.z);
}

fn hash33(p3_in: vec3<f32>) -> vec3<f32> {
    var p3 = fract(p3_in * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 += dot(p3, p3.yxz + 33.33);
    return fract((p3.xxy + p3.yxx) * p3.zyx);
}

fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tiling noise
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Every generator here takes a cell count and wraps its lattice modulo that
// count, so the resulting volume tiles seamlessly. A cloud volume that does not
// tile shows a hard seam every time the wind carries the sample point past the
// texture's edge, and it is invisible in a still screenshot and glaring the
// moment anything moves.

fn wrap3(v: vec3<f32>, period: f32) -> vec3<f32> {
    return v - floor(v / period) * period;
}

/// Inverted Worley (cellular) noise: 1 at a feature point, falling to 0.
fn worley(p: vec3<f32>, cells: f32) -> f32 {
    let pc = p * cells;
    let id = floor(pc);
    let f = pc - id;
    var nearest = 1.0e9;
    for (var z = -1; z <= 1; z = z + 1) {
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let offset = vec3<f32>(f32(x), f32(y), f32(z));
                let cell = wrap3(id + offset, cells);
                let feature = hash33(cell) + offset;
                nearest = min(nearest, length(feature - f));
            }
        }
    }
    return 1.0 - clamp(nearest, 0.0, 1.0);
}

/// Three octaves of Worley, each twice the frequency and half the weight.
fn worley_fbm(p: vec3<f32>, cells: f32) -> f32 {
    return worley(p, cells) * 0.625
        + worley(p, cells * 2.0) * 0.25
        + worley(p, cells * 4.0) * 0.125;
}

/// Tiling gradient (Perlin) noise.
fn perlin(p: vec3<f32>, cells: f32) -> f32 {
    let pc = p * cells;
    let id = floor(pc);
    let f = pc - id;
    // Quintic fade: C² continuous, so a lit cloud has no visible lattice.
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);

    var total = 0.0;
    for (var z = 0; z <= 1; z = z + 1) {
        for (var y = 0; y <= 1; y = y + 1) {
            for (var x = 0; x <= 1; x = x + 1) {
                let corner = vec3<f32>(f32(x), f32(y), f32(z));
                let cell = wrap3(id + corner, cells);
                let gradient = normalize(hash33(cell) * 2.0 - 1.0);
                let weight = mix(1.0 - u, u, corner);
                total += dot(gradient, f - corner) * weight.x * weight.y * weight.z;
            }
        }
    }
    return total;
}

fn perlin_fbm(p: vec3<f32>, cells: f32) -> f32 {
    return perlin(p, cells) * 0.5
        + perlin(p, cells * 2.0) * 0.25
        + perlin(p, cells * 4.0) * 0.125
        + perlin(p, cells * 8.0) * 0.0625;
}

/// Rescale `v` from `[old_lo, old_hi]` into `[new_lo, new_hi]`.
///
/// The single most load-bearing helper in the whole technique: Schneider's
/// shape model is a chain of remaps, and getting one of them backwards
/// produces clouds that look plausible in a thumbnail and have no interior.
fn remap(v: f32, old_lo: f32, old_hi: f32, new_lo: f32, new_hi: f32) -> f32 {
    return new_lo + (v - old_lo) / max(old_hi - old_lo, 1e-6) * (new_hi - new_lo);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Entry points
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 128³ base volume.
///
/// - **R** — Perlin–Worley: `perlin_fbm` remapped by `worley_fbm`, which is
///   the shape a cumulus silhouette needs.
/// - **G, B, A** — three increasing Worley frequencies, used by the march to
///   erode the base into billows.
@compute @workgroup_size(4, 4, 4)
fn base_noise(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(base_out);
    if gid.x >= dims.x || gid.y >= dims.y || gid.z >= dims.z {
        return;
    }
    let p = (vec3<f32>(gid) + 0.5) / vec3<f32>(dims);

    // Perlin is signed; the shape model wants it in 0..1.
    let perlin_signed = perlin_fbm(p, 4.0);
    let perlin01 = clamp(perlin_signed * 0.5 + 0.5, 0.0, 1.0);
    let worley_low = worley_fbm(p, 4.0);
    // Schneider's remap: the Worley field becomes the *floor* the Perlin field
    // is rescaled against, which is what carves the billows.
    let perlin_worley = clamp(remap(perlin01, worley_low - 1.0, 1.0, 0.0, 1.0), 0.0, 1.0);

    textureStore(
        base_out,
        vec3<i32>(gid),
        vec4<f32>(
            perlin_worley,
            worley_fbm(p, 8.0),
            worley_fbm(p, 16.0),
            worley_fbm(p, 32.0),
        ),
    );
}

/// 32³ detail volume: three Worley octaves used only at the cloud's edges.
@compute @workgroup_size(4, 4, 4)
fn detail_noise(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(detail_out);
    if gid.x >= dims.x || gid.y >= dims.y || gid.z >= dims.z {
        return;
    }
    let p = (vec3<f32>(gid) + 0.5) / vec3<f32>(dims);
    textureStore(
        detail_out,
        vec3<i32>(gid),
        vec4<f32>(
            worley_fbm(p, 4.0),
            worley_fbm(p, 8.0),
            worley_fbm(p, 16.0),
            1.0,
        ),
    );
}

/// Two-dimensional tiling value noise, for the weather field.
fn value2(p: vec2<f32>, cells: f32, seed: f32) -> f32 {
    let pc = p * cells;
    let id = floor(pc);
    let f = pc - id;
    let u = f * f * (3.0 - 2.0 * f);
    let wrap = vec2<f32>(cells, cells);
    let a = hash12(((id + vec2<f32>(0.0, 0.0)) % wrap) + seed);
    let b = hash12(((id + vec2<f32>(1.0, 0.0)) % wrap) + seed);
    let c = hash12(((id + vec2<f32>(0.0, 1.0)) % wrap) + seed);
    let d = hash12(((id + vec2<f32>(1.0, 1.0)) % wrap) + seed);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn value2_fbm(p: vec2<f32>, seed: f32) -> f32 {
    return value2(p, 4.0, seed) * 0.5
        + value2(p, 8.0, seed + 11.0) * 0.25
        + value2(p, 16.0, seed + 23.0) * 0.15
        + value2(p, 32.0, seed + 37.0) * 0.10;
}

/// The weather map: a top-down field sampled by world XZ.
///
/// - **R** — coverage. How much of the sky this column has cloud in.
/// - **G** — cloud type, `0` stratus … `1` cumulonimbus. Selects the vertical
///   density profile in the march.
/// - **B** — precipitation. Read by CONTROL-N; the march uses it only to
///   darken the base of a raining cloud.
/// - **A** — unused, reserved.
///
/// Generated procedurally from the authored biases, and **paintable**: the
/// texture is a `COPY_DST` so a brush can write into it without regenerating.
@compute @workgroup_size(8, 8, 1)
fn weather_map(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(weather_out);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(dims);

    // Coverage is the authored bias remapped through a noise field rather than
    // multiplied by it: at coverage 1 the sky must be *solid*, and a multiply
    // can never reach solid because the noise never reaches one everywhere.
    let field = value2_fbm(uv, np.seed);
    let coverage = clamp(remap(field, 1.0 - np.coverage, 1.0, 0.0, 1.0), 0.0, 1.0);

    // Type is a slower field, so a bank of cumulus does not turn into stratus
    // texel by texel.
    let type_field = value2(uv, 3.0, np.seed + 71.0);
    let cloud_type = clamp(np.cloud_type * 0.7 + type_field * 0.3, 0.0, 1.0);

    // Precipitation follows the thickest parts, which is where it actually
    // comes from: a raining column is a deep column.
    let precip = clamp(np.precipitation * coverage * cloud_type * 1.5, 0.0, 1.0);

    textureStore(
        weather_out,
        vec2<i32>(gid.xy),
        vec4<f32>(coverage, cloud_type, precip, 1.0),
    );
}
