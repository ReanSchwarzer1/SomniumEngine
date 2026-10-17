// Optical flow vectors (one per 8x8 block of the back buffer) smoothed over
// their 3x3 neighbourhood and scattered half way, like the game's. Translation
// of ffx_frameinterpolation_optical_flow_vector_field.h. The confidence AMD loads
// there is never used, so it is not bound.

@group(0) @binding(0) var<uniform> c: FgConstants;
@group(0) @binding(1) var<storage, read> optical_flow: array<vec2<i32>>;
@group(0) @binding(2) var previous_source: texture_2d<f32>;
@group(0) @binding(3) var current_source: texture_2d<f32>;
@group(0) @binding(4) var linear_clamp: sampler;
@group(0) @binding(5) var<storage, read_write> of_mv_x: array<atomic<u32>>;
@group(0) @binding(6) var<storage, read_write> of_mv_y: array<atomic<u32>>;

fn load_optical_flow(pos: vec2<i32>) -> vec2<f32> {
    if (!is_on_screen(pos, c.optical_flow_size)) {
        return vec2<f32>(0.0);
    }
    return vec2<f32>(optical_flow[pixel_index(pos, c.optical_flow_size.x)]) * c.optical_flow_scale;
}

fn compute_optical_flow_field_mvs(id: vec2<i32>, of_vector: vec2<f32>) {
    let uv = (vec2<f32>(id) + 0.5) / vec2<f32>(c.optical_flow_size);
    let mv_half = of_vector * 0.5;

    let prev_col = textureSampleLevel(previous_source, linear_clamp, uv + of_vector, 0.0).rgb;
    let curr_col = textureSampleLevel(current_source, linear_clamp, uv, 0.0).rgb;
    let prev_luma = 0.001 + raw_to_luminance(prev_col);
    let curr_luma = 0.001 + raw_to_luminance(curr_col);

    let rect_size = vec2<f32>(c.rect_size);
    let velocity = length(of_vector * rect_size);
    let high = u32(velocity > 1.0) * u32(saturate(velocity / length(rect_size * 0.05)) * f32(PRIORITY_HIGH_MAX));

    if (high > 0u) {
        let low = u32(round(min_divided_by_max(prev_luma, curr_luma) * f32(PRIORITY_LOW_MAX)))
            * u32(is_uv_inside(uv + of_vector));
        let packed = pack_vector_field_entries(true, high, low, mv_half);
        let b = bilinear(uv + mv_half, c.optical_flow_size);
        for (var s = 0; s < 4; s++) {
            let p = b.base + bilinear_offset(s);
            if (is_on_screen(p, c.optical_flow_size)) {
                let i = pixel_index(p, c.optical_flow_size.x);
                atomicMax(&of_mv_x[i], packed.x);
                atomicMax(&of_mv_y[i], packed.y);
            }
        }
    }
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let pos = vec2<i32>(gid.xy);
    if (!is_on_screen(pos, c.optical_flow_size)) {
        return;
    }
    let rect_size = vec2<f32>(c.rect_size);

    var avg = vec2<f32>(0.0);
    var sw = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let vs = load_optical_flow(pos + vec2<i32>(x, y));
            let len = length(vs * rect_size);
            let w = max(0.0, 512.0 - len) * f32(len > 1.0);
            avg += vs * w;
            sw += w;
        }
    }
    avg = select(vec2<f32>(0.0), avg / sw, sw > 0.0);

    var of_vector = vec2<f32>(0.0);
    sw = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let vs = load_optical_flow(pos + vec2<i32>(x, y));
            let len = length(vs * rect_size);
            let len_factor = max(0.0, 512.0 - len) * f32(len > 1.0);
            let d = dot(avg, vs);
            // pow of a negative base is NaN in HLSL, which max() then drops to 0.
            let w = select(0.0, pow(d, 1.25), d > 0.0) * len_factor;
            of_vector += vs * w;
            sw += w;
        }
    }
    if (sw > FI_EPSILON) {
        of_vector /= sw;
    }
    compute_optical_flow_field_mvs(pos, of_vector);
}
