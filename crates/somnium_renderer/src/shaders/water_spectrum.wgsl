// Somnium Engine Phase IV-K: deterministic multi-cascade inverse FFT ocean.
//
// The pipeline follows the structure established by Tessendorf (Simulating
// Ocean Water) for the spectrum and dispersion, Horvath (Empirical Directional
// Wave Spectra for Computer Graphics) for the TMA/Hasselmann directional model,
// and Matusiak for packing two real-valued transforms into one complex FFT.
// The MIT-licensed GodotOceanWaves project by 2Retr0 demonstrates the same
// combination, and Somnium matches its parameterisation so the two renderers
// can be compared directly. This WGSL is an original implementation: the row
// transform is restructured for WebGPU's 256-invocation workgroup ceiling, the
// foam history lives in its own read-write texture rather than an aliased
// read-modify-write of the normal map, and the FFT scratch buffer is shared
// across cascades because they are updated one per frame.
//
// Stage order per cascade:
//   generate_spectrum  (only when authored parameters change)
//   modulate           spectrum -> 4 packed complex rows, buffer half 0
//   fft_row            half 0 -> half 1
//   transpose          half 1 -> half 0
//   fft_row            half 0 -> half 1
//   unpack             half 1 -> displacement map, normal map, foam state
//
// A second transpose is deliberately omitted. It rotates the resulting tile by
// a quarter turn, which is invisible on an isotropic wave field, and it would
// cost another full pass over four megabytes per cascade.

const PI: f32 = 3.141592653589793;
const G: f32 = 9.81;
const NUM_SPECTRA: u32 = 4u;

// The row transform stages a whole FFT row in workgroup memory. Two rows of
// 1024 complex values is 16 KiB, which is exactly WebGPU's guaranteed workgroup
// storage budget, so this is the largest map the shared-memory path can hold.
const MAX_MAP_SIZE: u32 = 1024u;
const FFT_THREADS: u32 = 256u;
const TRANSPOSE_TILE: u32 = 32u;

struct SpectrumParams {
    // The CPU side may request a smaller transform without a shader change;
    // the workgroup allocation is always the maximum.
    map_size: u32,
    stages: u32,
    _pad0: u32,
    _pad1: u32,
    tile_length: vec2<f32>,
    depth: f32,
    time: f32,
    alpha: f32,
    peak_frequency: f32,
    wind_speed: f32,
    wind_angle: f32,
    swell: f32,
    detail: f32,
    spread: f32,
    whitecap: f32,
    foam_grow_rate: f32,
    foam_decay_rate: f32,
    seed: vec2<i32>,
}

@group(0) @binding(0) var<storage, read_write> spectrum: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> fft_data: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read_write> butterfly: array<vec4<f32>>;
@group(0) @binding(3) var<uniform> params: SpectrumParams;
@group(0) @binding(4) var displacement_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(5) var normal_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(6) var foam_state: texture_storage_2d<r32float, read_write>;

fn complex_mul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}

fn complex_conj(a: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x, -a.y);
}

fn exp_complex(x: f32) -> vec2<f32> {
    return vec2<f32>(cos(x), sin(x));
}

// ── Initial spectrum ──────────────────────────────────────────────────────
//
// A single deterministic hash per texel, so the field is reproducible from the
// authored seed alone and never needs a CPU upload.

fn hash2(x: vec2<u32>) -> vec2<f32> {
    var h32 = x.y + 374761393u + x.x * 3266489917u;
    h32 = 2246822519u * (h32 ^ (h32 >> 15u));
    h32 = 3266489917u * (h32 ^ (h32 >> 13u));
    let n = h32 ^ (h32 >> 16u);
    let rz = vec2<u32>(n, n * 48271u);
    return vec2<f32>((rz >> vec2<u32>(1u)) & vec2<u32>(0x7FFFFFFFu))
        / f32(0x7FFFFFFF);
}

/// Box-Muller transform from a uniform pair to a bivariate normal sample.
fn gaussian(x: vec2<f32>) -> vec2<f32> {
    let r = sqrt(-2.0 * log(max(x.x, 1e-9)));
    let theta = 2.0 * PI * x.y;
    return vec2<f32>(r * cos(theta), r * sin(theta));
}

/// Finite-depth dispersion and its derivative with respect to wavenumber.
fn dispersion_relation(k: f32) -> vec2<f32> {
    let a = k * params.depth;
    let b = tanh(a);
    let omega = sqrt(G * k * b);
    let d_omega = 0.5 * G * (b + a * (1.0 - b * b)) / max(omega, 1e-6);
    return vec2<f32>(omega, d_omega);
}

/// JONSWAP with Kitaigorodskii finite-depth attenuation (the TMA spectrum).
fn tma_spectrum(w: f32, w_p: f32, alpha: f32) -> f32 {
    let beta = 1.25;
    let gamma = 3.3;
    let sigma = select(0.09, 0.07, w <= w_p);
    let r = exp(-(w - w_p) * (w - w_p) / (2.0 * sigma * sigma * w_p * w_p));
    let jonswap = (alpha * G * G) / pow(w, 5.0)
        * exp(-beta * pow(w_p / w, 4.0)) * pow(gamma, r);
    let w_h = min(w * sqrt(params.depth / G), 2.0);
    let attenuation = select(
        1.0 - 0.5 * (2.0 - w_h) * (2.0 - w_h),
        0.5 * w_h * w_h,
        w_h <= 1.0,
    );
    return jonswap * attenuation;
}

fn longuet_higgins_normalization(s: f32) -> f32 {
    let a = sqrt(s);
    if s < 0.4 {
        return (0.5 / PI) + s * (0.220636 + s * (-0.109 + s * 0.090));
    }
    return inverseSqrt(PI) * (a * 0.5 + (1.0 / a) * 0.0625);
}

fn longuet_higgins_function(s: f32, theta: f32) -> f32 {
    return longuet_higgins_normalization(s) * pow(abs(cos(theta * 0.5)), 2.0 * s);
}

fn hasselmann_directional_spread(w: f32, w_p: f32, wind_speed: f32, theta: f32) -> f32 {
    let p = w / w_p;
    var s: f32;
    if w <= w_p {
        s = 6.97 * pow(abs(p), 4.06);
    } else {
        s = 9.77 * pow(abs(p), -2.33 - 1.45 * (wind_speed * w_p / G - 1.17));
    }
    let s_swell = 16.0 * tanh(w_p / w) * params.swell * params.swell;
    return longuet_higgins_function(s + s_swell, theta - params.wind_angle);
}

fn spectrum_amplitude(id: vec2<i32>, map_size: i32) -> vec2<f32> {
    let dk = 2.0 * PI / params.tile_length;
    let k_vec = (vec2<f32>(id) - f32(map_size) * 0.5) * dk;
    let k = length(k_vec) + 1e-6;
    let theta = atan2(k_vec.x, k_vec.y);

    let dispersion = dispersion_relation(k);
    let w = dispersion.x;
    let w_norm = dispersion.y / k * dk.x * dk.y;
    let s = tma_spectrum(w, params.peak_frequency, params.alpha);
    let directional = hasselmann_directional_spread(
        w, params.peak_frequency, params.wind_speed, theta);
    // `spread` fades the directional distribution towards a flat one, and
    // `detail` rolls off the shortest waves that the map cannot resolve.
    let d = mix(0.5 / PI, directional, 1.0 - params.spread)
        * exp(-(1.0 - params.detail) * (1.0 - params.detail) * k * k);
    let noise = hash2(vec2<u32>(id + params.seed));
    return gaussian(noise) * sqrt(2.0 * s * d * w_norm);
}

@compute @workgroup_size(16, 16, 1)
fn generate_spectrum(@builtin(global_invocation_id) gid: vec3<u32>) {
    let map_size = params.map_size;
    if gid.x >= map_size || gid.y >= map_size {
        return;
    }
    let size = i32(map_size);
    let id0 = vec2<i32>(gid.xy);
    // The conjugate partner at -k, wrapped into the map, is stored alongside so
    // the per-frame modulation never has to gather from a second texel.
    let id1 = ((-id0 % size) + size) % size;
    let h0 = spectrum_amplitude(id0, size);
    let h0_negative = spectrum_amplitude(id1, size);
    spectrum[gid.y * map_size + gid.x] =
        vec4<f32>(h0, complex_conj(h0_negative));
}

// ── Time modulation and gradient packing ──────────────────────────────────

@compute @workgroup_size(16, 16, 1)
fn modulate(@builtin(global_invocation_id) gid: vec3<u32>) {
    let map_size = params.map_size;
    if gid.x >= map_size || gid.y >= map_size {
        return;
    }
    let k_vec = (vec2<f32>(gid.xy) - f32(map_size) * 0.5)
        * 2.0 * PI / params.tile_length;
    let k = length(k_vec) + 1e-6;
    let k_unit = k_vec / k;

    let h0 = spectrum[gid.y * map_size + gid.x];
    let omega = sqrt(G * k * tanh(k * params.depth));
    let modulation = exp_complex(omega * params.time);
    let h = complex_mul(h0.xy, modulation)
        + complex_mul(h0.zw, complex_conj(modulation));
    // Multiplying by i, used repeatedly below to take a spatial derivative.
    let h_inv = vec2<f32>(-h.y, h.x);

    let hx = h_inv * k_unit.y;
    let hy = h;
    let hz = h_inv * k_unit.x;

    let dhy_dx = h_inv * k_vec.y;
    let dhy_dz = h_inv * k_vec.x;
    let dhx_dx = -h * k_vec.y * k_unit.y;
    let dhz_dz = -h * k_vec.x * k_unit.x;
    let dhz_dx = -h * k_vec.y * k_unit.x;

    // Each of these eight fields transforms to a real signal, so they pair up
    // into four complex transforms instead of eight.
    let stride = map_size * map_size;
    let index = gid.y * map_size + gid.x;
    fft_data[index] =
        vec2<f32>(hx.x - hy.y, hx.y + hy.x);
    fft_data[stride + index] =
        vec2<f32>(hz.x - dhy_dx.y, hz.y + dhy_dx.x);
    fft_data[2u * stride + index] =
        vec2<f32>(dhy_dz.x - dhx_dx.y, dhy_dz.y + dhx_dx.x);
    fft_data[3u * stride + index] =
        vec2<f32>(dhz_dz.x - dhz_dx.y, dhz_dz.y + dhz_dx.x);
}

// ── Stockham FFT ──────────────────────────────────────────────────────────

@compute @workgroup_size(64, 1, 1)
fn butterfly_precompute(@builtin(global_invocation_id) gid: vec3<u32>) {
    let map_size = params.map_size;
    let col = gid.x;
    let stage = gid.y;
    let stride = 1u << stage;
    let mid = map_size >> (stage + 1u);
    let i = col >> stage;
    let j = col % stride;

    let twiddle = exp_complex(PI / f32(stride) * f32(j));
    let r0 = stride * i + j;
    let r1 = stride * (i + mid) + j;
    let w0 = stride * (2u * i) + j;
    let w1 = stride * (2u * i + 1u) + j;

    let reads = vec2<f32>(bitcast<f32>(r0), bitcast<f32>(r1));
    butterfly[stage * map_size + w0] = vec4<f32>(reads, twiddle);
    butterfly[stage * map_size + w1] = vec4<f32>(reads, -twiddle);
}

var<workgroup> row_shared: array<vec2<f32>, 2u * MAX_MAP_SIZE>;

/// One workgroup transforms one row of one packed spectrum entirely in
/// workgroup memory. WebGPU caps a workgroup at 256 invocations, so each
/// invocation owns `map_size / 256` columns rather than the single column a
/// desktop-Vulkan kernel would use.
@compute @workgroup_size(256, 1, 1)
fn fft_row(
    @builtin(workgroup_id) group_id: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>,
) {
    let map_size = params.map_size;
    let stages = firstLeadingBit(map_size);
    let per_thread = map_size / FFT_THREADS;
    let plane = map_size * map_size;
    let read_base = group_id.z * plane + group_id.y * map_size;
    let write_base = NUM_SPECTRA * plane + read_base;

    for (var e = 0u; e < per_thread; e = e + 1u) {
        let col = local_id.x + e * FFT_THREADS;
        row_shared[col] = fft_data[read_base + col];
    }

    for (var stage = 0u; stage < stages; stage = stage + 1u) {
        workgroupBarrier();
        let read_half = (stage % 2u) * MAX_MAP_SIZE;
        let write_half = ((stage + 1u) % 2u) * MAX_MAP_SIZE;
        for (var e = 0u; e < per_thread; e = e + 1u) {
            let col = local_id.x + e * FFT_THREADS;
            let factors = butterfly[stage * map_size + col];
            let upper = row_shared[read_half + bitcast<u32>(factors.x)];
            let lower = row_shared[read_half + bitcast<u32>(factors.y)];
            row_shared[write_half + col] = upper + complex_mul(lower, factors.zw);
        }
    }

    workgroupBarrier();
    let final_half = (stages % 2u) * MAX_MAP_SIZE;
    for (var e = 0u; e < per_thread; e = e + 1u) {
        let col = local_id.x + e * FFT_THREADS;
        fft_data[write_base + col] = row_shared[final_half + col];
    }
}

// Padded by one column so the column-major read phase does not collide on a
// single workgroup-memory bank.
var<workgroup> transpose_tile: array<vec2<f32>, TRANSPOSE_TILE * (TRANSPOSE_TILE + 1u)>;

@compute @workgroup_size(32, 8, 1)
fn transpose(
    @builtin(workgroup_id) group_id: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>,
) {
    let map_size = params.map_size;
    let plane = map_size * map_size;
    let read_base = NUM_SPECTRA * plane + group_id.z * plane;
    let write_base = group_id.z * plane;

    let x = group_id.x * TRANSPOSE_TILE + local_id.x;
    for (var r = 0u; r < TRANSPOSE_TILE; r = r + 8u) {
        let y = group_id.y * TRANSPOSE_TILE + local_id.y + r;
        transpose_tile[(local_id.y + r) * (TRANSPOSE_TILE + 1u) + local_id.x] =
            fft_data[read_base + y * map_size + x];
    }
    workgroupBarrier();
    let tx = group_id.y * TRANSPOSE_TILE + local_id.x;
    for (var r = 0u; r < TRANSPOSE_TILE; r = r + 8u) {
        let ty = group_id.x * TRANSPOSE_TILE + local_id.y + r;
        fft_data[write_base + ty * map_size + tx] =
            transpose_tile[local_id.x * (TRANSPOSE_TILE + 1u) + local_id.y + r];
    }
}

// ── Unpack ────────────────────────────────────────────────────────────────

@compute @workgroup_size(16, 16, 1)
fn unpack(@builtin(global_invocation_id) gid: vec3<u32>) {
    let map_size = params.map_size;
    if gid.x >= map_size || gid.y >= map_size {
        return;
    }
    let plane = map_size * map_size;
    let index = NUM_SPECTRA * plane + gid.y * map_size + gid.x;
    let packed_0 = fft_data[index];
    let packed_1 = fft_data[index + plane];
    let packed_2 = fft_data[index + 2u * plane];
    let packed_3 = fft_data[index + 3u * plane];

    // The spectrum was built around a centred wavevector origin, so the
    // transform lands shifted by half a period. Alternating the sign per texel
    // is the same correction as an ifftshift, without a second pass.
    let parity = (gid.x & 1u) ^ (gid.y & 1u);
    let sign_shift = 1.0 - 2.0 * f32(parity);

    let displacement = vec3<f32>(packed_0.x, packed_0.y, packed_1.x);
    textureStore(displacement_out, vec2<i32>(gid.xy),
        vec4<f32>(displacement * sign_shift, 0.0));

    let dhy_dx = packed_1.y * sign_shift;
    let dhy_dz = packed_2.x * sign_shift;
    let dhx_dx = packed_2.y * sign_shift;
    let dhz_dz = packed_3.x * sign_shift;
    let dhz_dx = packed_3.y * sign_shift;

    // Horizontal choppiness folds the surface over itself where the Jacobian
    // of the displacement map drops below the whitecap threshold.
    let jacobian = (1.0 + dhx_dx) * (1.0 + dhz_dz) - dhz_dx * dhz_dx;
    let fold = -min(0.0, jacobian - params.whitecap);

    let texel = vec2<i32>(gid.xy);
    var foam = textureLoad(foam_state, texel).r;
    foam = foam * exp(-params.foam_decay_rate);
    foam = foam + fold * params.foam_grow_rate;
    foam = clamp(foam, 0.0, 1.0);
    textureStore(foam_state, texel, vec4<f32>(foam, 0.0, 0.0, 0.0));

    // Dividing by the horizontal stretch keeps the slope correct where the
    // surface has been compressed by the choppy displacement.
    let gradient = vec2<f32>(dhy_dx, dhy_dz)
        / (1.0 + abs(vec2<f32>(dhx_dx, dhz_dz)));
    textureStore(normal_out, texel, vec4<f32>(gradient, dhx_dx, foam));
}
