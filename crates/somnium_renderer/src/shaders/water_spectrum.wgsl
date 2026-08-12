// Somnium Engine Phase IV-F: deterministic two-cascade inverse FFT water.
// The implementation is original WGSL. Initial amplitudes use a finite-depth
// JONSWAP/TMA spectrum with Hasselmann directional spreading, following the
// equations documented by Horvath and the MIT GodotOceanWaves reference. The
// inverse transform remains Somnium's radix-2 ping-pong implementation.

struct SpectrumParams {
    dimension: u32,
    stage_size: u32,
    axis: u32,
    input_is_a: u32,
    time: f32,
    delta_time: f32,
    patch_length: f32,
    speed: f32,
    wind_dir: vec2<f32>,
    choppy: f32,
    foam_decay: f32,
    foam_threshold: f32,
    water_depth: f32,
    _pad1: vec2<f32>,
}

@group(0) @binding(0) var<storage, read> initial_spectrum: array<vec2<f32>>;
@group(0) @binding(1) var<storage, read_write> spectrum_a: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read_write> spectrum_b: array<vec2<f32>>;
@group(0) @binding(3) var<uniform> params: SpectrumParams;
@group(0) @binding(4) var displacement_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(5) var gradient_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(6) var previous_gradient: texture_2d<f32>;

const PI: f32 = 3.141592653589793;
const TAU: f32 = 6.283185307179586;

fn complex_mul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}

fn load_complex(index: u32) -> vec2<f32> {
    if params.input_is_a != 0u { return spectrum_a[index]; }
    return spectrum_b[index];
}

fn store_complex(index: u32, value: vec2<f32>) {
    if params.input_is_a != 0u { spectrum_b[index] = value; }
    else { spectrum_a[index] = value; }
}

fn reverse_bits(value: u32, bit_count: u32) -> u32 {
    return reverseBits(value) >> (32u - bit_count);
}

@compute @workgroup_size(8, 8, 1)
fn update_spectrum(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = params.dimension;
    if id.x >= n || id.y >= n { return; }
    let index = id.y * n + id.x;
    let negative = ((n - id.y) % n) * n + ((n - id.x) % n);
    let h0 = initial_spectrum[index];
    let h0_negative_conjugate = vec2<f32>(initial_spectrum[negative].x,
        -initial_spectrum[negative].y);
    let signed_x = select(f32(id.x), f32(id.x) - f32(n), id.x > n / 2u);
    let signed_y = select(f32(id.y), f32(id.y) - f32(n), id.y > n / 2u);
    let k = vec2<f32>(signed_x, signed_y) * TAU / params.patch_length;
    let k_length = length(k);
    let omega = sqrt(9.81 * k_length * tanh(k_length * params.water_depth))
        * params.speed;
    let phase = omega * params.time;
    let positive = vec2<f32>(cos(phase), sin(phase));
    let negative_phase = vec2<f32>(positive.x, -positive.y);
    let height = complex_mul(h0, positive)
        + complex_mul(h0_negative_conjugate, negative_phase);
    let direction = select(vec2<f32>(0.0), k / k_length, k_length > 1e-6);
    // Multiplication by i*c maps (real,imaginary) to (-c*imaginary,c*real).
    let dx = vec2<f32>(direction.x * height.y, -direction.x * height.x)
        * params.choppy;
    let dz = vec2<f32>(direction.y * height.y, -direction.y * height.x)
        * params.choppy;
    let field_stride = n * n;
    spectrum_a[index] = dx;
    spectrum_a[field_stride + index] = dz;
    spectrum_a[field_stride * 2u + index] = height;
}

@compute @workgroup_size(8, 8, 1)
fn bit_reverse(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = params.dimension;
    if id.x >= n || id.y >= n || id.z >= 3u { return; }
    let bits = u32(log2(f32(n)));
    let source = id.z * n * n + id.y * n + id.x;
    let destination = id.z * n * n
        + reverse_bits(id.y, bits) * n + reverse_bits(id.x, bits);
    spectrum_b[destination] = spectrum_a[source];
}

@compute @workgroup_size(8, 8, 1)
fn fft_stage(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = params.dimension;
    if id.x >= n || id.y >= n || id.z >= 3u { return; }
    let coordinate = select(id.x, id.y, params.axis != 0u);
    let m = params.stage_size;
    let half = m / 2u;
    let offset = coordinate % m;
    let pair = offset % half;
    let base = coordinate - offset;
    let c0 = base + pair;
    let c1 = c0 + half;
    let index0 = id.z * n * n + select(id.y * n + c0, c0 * n + id.x, params.axis != 0u);
    let index1 = id.z * n * n + select(id.y * n + c1, c1 * n + id.x, params.axis != 0u);
    let output_index = id.z * n * n + id.y * n + id.x;
    let u = load_complex(index0);
    let v = load_complex(index1);
    let angle = TAU * f32(pair) / f32(m);
    let rotated = complex_mul(v, vec2<f32>(cos(angle), sin(angle)));
    store_complex(output_index, select(u + rotated, u - rotated, offset >= half));
}

fn displaced(index: u32, field: u32, scale: f32) -> f32 {
    return spectrum_b[field * params.dimension * params.dimension + index].x * scale;
}

@compute @workgroup_size(8, 8, 1)
fn compose(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = params.dimension;
    if id.x >= n || id.y >= n { return; }
    let x0 = (id.x + n - 1u) % n;
    let x1 = (id.x + 1u) % n;
    let y0 = (id.y + n - 1u) % n;
    let y1 = (id.y + 1u) % n;
    let center = id.y * n + id.x;
    let left = id.y * n + x0;
    let right = id.y * n + x1;
    let back = y0 * n + id.x;
    let front = y1 * n + id.x;
    let normalization = 1.0 / f32(n * n);
    let dx = displaced(center, 0u, normalization);
    let dz = displaced(center, 1u, normalization);
    let height = displaced(center, 2u, normalization);
    let cell = params.patch_length / f32(n);
    let dhdx = (displaced(right, 2u, normalization)
        - displaced(left, 2u, normalization)) / (2.0 * cell);
    let dhdz = (displaced(front, 2u, normalization)
        - displaced(back, 2u, normalization)) / (2.0 * cell);
    let dxdx = (displaced(right, 0u, normalization)
        - displaced(left, 0u, normalization)) / (2.0 * cell);
    let dxdz = (displaced(front, 0u, normalization)
        - displaced(back, 0u, normalization)) / (2.0 * cell);
    let dzdx = (displaced(right, 1u, normalization)
        - displaced(left, 1u, normalization)) / (2.0 * cell);
    let dzdz = (displaced(front, 1u, normalization)
        - displaced(back, 1u, normalization)) / (2.0 * cell);
    let jacobian = (1.0 + dxdx) * (1.0 + dzdz) - dxdz * dzdx;
    let fold = max(1.0 - jacobian - params.foam_threshold, 0.0);
    // Rare's Sea of Thieves ocean progressively blurs foam feedback so white
    // water disperses instead of remaining a sharp simulation texel. A small
    // periodic cross filter supplies that transport without another pass.
    let previous_center = textureLoad(previous_gradient, vec2<i32>(id.xy), 0).a;
    let previous = previous_center * 0.50
        + textureLoad(previous_gradient, vec2<i32>(vec2<u32>(x0, id.y)), 0).a * 0.125
        + textureLoad(previous_gradient, vec2<i32>(vec2<u32>(x1, id.y)), 0).a * 0.125
        + textureLoad(previous_gradient, vec2<i32>(vec2<u32>(id.x, y0)), 0).a * 0.125
        + textureLoad(previous_gradient, vec2<i32>(vec2<u32>(id.x, y1)), 0).a * 0.125;
    let decay = exp(-params.delta_time / max(params.foam_decay, 0.05));
    let grow = fold * (2.4 + params.delta_time * 9.0);
    let foam = clamp(max(grow, previous * decay), 0.0, 1.0);
    textureStore(displacement_out, vec2<i32>(id.xy), vec4<f32>(dx, dz, height, jacobian));
    textureStore(gradient_out, vec2<i32>(id.xy), vec4<f32>(dhdx, dhdz,
        max(1.0 - jacobian, 0.0), foam));
}
