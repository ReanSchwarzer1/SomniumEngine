// FidelityFX frame interpolation: shared definitions.
//
// WGSL translation of ffx_frameinterpolation_common.h and the accessors of
// ffx_frameinterpolation_callbacks_hlsl.h (AMD FidelityFX SDK 1.1.4, MIT; see LICENSE).
// Prepended to every frame interpolation pass. Declares no bindings: each pass
// declares its own, and the chunks below only name the ones they need.
//
// Departures from the HLSL, all for WGSL rather than for behaviour:
// - Per-pixel data AMD keeps in UAV textures (dilated depth and motion, the
//   reconstructed depths, the two motion vector fields) lives in storage
//   buffers indexed `y * width + x`, so atomics and read-write need no format
//   features. Out-of-range reads return zero, as D3D's do.
// - Reconstructed depths are stored as `1 - depth` and merged with atomicMax, so
//   a cleared (zero) buffer means the far plane. Somnium's depth is standard
//   (near 0, far 1), the option AMD merges with InterlockedMin.
// - The back buffer is LDR sRGB (transfer function 0), so the HDR transfer
//   functions are not translated. No distortion field, no HUD-less present.

struct FgConstants {
    render_size: vec2<i32>,
    display_size: vec2<i32>,
    max_render_size: vec2<i32>,
    optical_flow_size: vec2<i32>,
    device_to_view: vec4<f32>,
    optical_flow_scale: vec2<f32>,
    jitter: vec2<f32>,
    motion_vector_scale: vec2<f32>,
    reset: u32,
    dispatch_flags: u32,
    rect_base: vec2<i32>,
    rect_size: vec2<i32>,
}

const FI_EPSILON: f32 = 1e-3;
const RECONSTRUCTED_DEPTH_BILINEAR_WEIGHT_THRESHOLD: f32 = 1e-3;

const DRAW_DEBUG_TEAR_LINES: u32 = 1u;
const DRAW_DEBUG_RESET_INDICATORS: u32 = 2u;

const COUNTER_FRAME_INDEX_SINCE_LAST_RESET: u32 = 1u;

// Motion vector field entry layout (32 bits): 16 vector coefficient (f16),
// 5 low priority, 10 high priority, 1 primary indication.
const PRIORITY_LOW_MAX: u32 = 31u;
const PRIORITY_HIGH_MAX: u32 = 1023u;
const PRIORITY_LOW_OFFSET: u32 = 16u;
const PRIORITY_HIGH_OFFSET: u32 = 21u;
const PRIMARY_VECTOR_BIT: u32 = 0x80000000u;

fn rgb_to_luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn linear_from_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * (1.0 / 12.92);
    let hi = pow(max(c, vec3<f32>(0.0)) * (1.0 / 1.055) + vec3<f32>(0.055 / 1.055), vec3<f32>(2.4));
    return select(hi, lo, c < vec3<f32>(0.04045));
}

fn raw_to_linear(raw: vec3<f32>) -> vec3<f32> {
    return linear_from_srgb(raw);
}

fn raw_to_luminance(raw: vec3<f32>) -> f32 {
    return rgb_to_luma(raw_to_linear(raw));
}

struct Bilinear {
    base: vec2<i32>,
    frac: vec2<f32>,
}

fn bilinear(uv: vec2<f32>, size: vec2<i32>) -> Bilinear {
    let px = uv * vec2<f32>(size) - vec2<f32>(0.5);
    let base = vec2<i32>(floor(px));
    return Bilinear(base, px - floor(px));
}

// Sample order (0,0), (1,0), (0,1), (1,1), as AMD's BilinearSamplingData.
fn bilinear_offset(i: i32) -> vec2<i32> {
    return vec2<i32>(i & 1, i >> 1u);
}

fn bilinear_weight(b: Bilinear, i: i32) -> f32 {
    let fx = select(1.0 - b.frac.x, b.frac.x, (i & 1) == 1);
    let fy = select(1.0 - b.frac.y, b.frac.y, (i >> 1u) == 1);
    return fx * fy;
}

fn is_on_screen(pos: vec2<i32>, size: vec2<i32>) -> bool {
    return all(bitcast<vec2<u32>>(pos) < bitcast<vec2<u32>>(size));
}

fn is_uv_inside(uv: vec2<f32>) -> bool {
    return uv.x > 0.0 && uv.x < 1.0 && uv.y > 0.0 && uv.y < 1.0;
}

fn is_in_rect(pos: vec2<i32>, corner: vec2<i32>, size: vec2<i32>) -> bool {
    return pos.x >= corner.x && pos.x < size.x + corner.x && pos.y >= corner.y && pos.y < size.y + corner.y;
}

fn min_divided_by_max(v0: f32, v1: f32) -> f32 {
    let m = max(v0, v1);
    return select(0.0, min(v0, v1) / m, m != 0.0);
}

fn normalized_dot3(v0: vec3<f32>, v1: vec3<f32>) -> f32 {
    let max_length = max(length(v0), length(v1));
    return select(1.0, dot(v0 / max_length, v1 / max_length), max_length > 0.0);
}

fn device_depth_to_view_space(device_depth: f32, d2v: vec4<f32>) -> f32 {
    return d2v.y / (device_depth - d2v.x);
}

fn compute_ndc(px: vec2<f32>, size: vec2<i32>) -> vec2<f32> {
    return px / vec2<f32>(size) * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
}

fn view_space_position(viewport_pos: vec2<i32>, viewport_size: vec2<i32>, device_depth: f32, d2v: vec4<f32>) -> vec3<f32> {
    let z = device_depth_to_view_space(device_depth, d2v);
    let ndc = compute_ndc(vec2<f32>(viewport_pos), viewport_size);
    return vec3<f32>(d2v.z * ndc.x * z, d2v.w * ndc.y * z, z);
}

fn pixel_index(pos: vec2<i32>, width: i32) -> u32 {
    return u32(pos.y) * u32(width) + u32(pos.x);
}

// `1 - depth` keeps the far plane at the cleared zero; see the header.
fn depth_to_store(depth: f32) -> u32 {
    return bitcast<u32>(1.0 - depth);
}

fn depth_from_store(bits: u32) -> f32 {
    return 1.0 - bitcast<f32>(bits);
}

// ── Motion vector field ─────────────────────────────────────────────────────

struct VectorFieldEntry {
    motion_vector: vec2<f32>,
    high_priority: f32,
    low_priority: f32,
    valid: bool,
    primary: bool,
    secondary: bool,
    inpainted: bool,
    velocity: f32,
    neg_outside: bool,
    pos_outside: bool,
}

fn new_vector_field_entry() -> VectorFieldEntry {
    return VectorFieldEntry(vec2<f32>(0.0), 0.0, 0.0, false, false, false, false, 0.0, false, false);
}

fn packed_entry_is_primary(packed: u32) -> bool {
    return (packed & PRIMARY_VECTOR_BIT) != 0u;
}

fn f32_to_f16_bits(v: f32) -> u32 {
    return pack2x16float(vec2<f32>(v, 0.0)) & 0xffffu;
}

fn f16_bits_to_f32(bits: u32) -> f32 {
    return unpack2x16float(bits & 0xffffu).x;
}

fn pack_vector_field_entries(is_primary: bool, high: u32, low: u32, mv: vec2<f32>) -> vec2<u32> {
    let priority = select(0u, PRIMARY_VECTOR_BIT, is_primary)
        | ((high & PRIORITY_HIGH_MAX) << PRIORITY_HIGH_OFFSET)
        | ((low & PRIORITY_LOW_MAX) << PRIORITY_LOW_OFFSET);
    return vec2<u32>(priority | f32_to_f16_bits(mv.x), priority | f32_to_f16_bits(mv.y));
}

fn unpack_vector_field_entries(packed: vec2<u32>) -> VectorFieldEntry {
    var e = new_vector_field_entry();
    e.high_priority = f32((packed.x >> PRIORITY_HIGH_OFFSET) & PRIORITY_HIGH_MAX) / f32(PRIORITY_HIGH_MAX);
    e.low_priority = f32((packed.x >> PRIORITY_LOW_OFFSET) & PRIORITY_LOW_MAX) / f32(PRIORITY_LOW_MAX);
    e.primary = packed_entry_is_primary(packed.x);
    e.valid = e.high_priority > 0.0;
    e.secondary = e.valid && !e.primary;
    // Reverse priority factor for secondary vectors.
    if (e.secondary) {
        e.high_priority = 1.0 - e.high_priority;
    }
    e.motion_vector = vec2<f32>(f16_bits_to_f32(packed.x), f16_bits_to_f32(packed.y));
    e.inpainted = false;
    return e;
}

fn tonemap(rgb: vec3<f32>) -> vec3<f32> {
    return rgb / (max(max(0.0, rgb.r), max(rgb.g, rgb.b)) + 1.0);
}
