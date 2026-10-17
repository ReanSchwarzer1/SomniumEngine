// Reconstruct and dilate: the game's depth and motion at render size become the
// dilated depth and motion the later passes read, and the depth is pushed to
// where it lay in the previous frame. Translation of
// ffx_frameinterpolation_reconstruct_dilated_velocity_and_previous_depth.h with
// low-resolution motion vectors (Somnium's velocity is render size, unjittered).

@group(0) @binding(0) var<uniform> c: FgConstants;
@group(0) @binding(1) var depth_in: texture_depth_2d;
@group(0) @binding(2) var motion_in: texture_2d<f32>;
@group(0) @binding(3) var<storage, read_write> dilated_depth: array<f32>;
@group(0) @binding(4) var<storage, read_write> dilated_motion: array<vec2<f32>>;
@group(0) @binding(5) var<storage, read_write> reconstructed_previous_depth: array<atomic<u32>>;

fn load_input_depth(pos: vec2<i32>) -> f32 {
    return textureLoad(depth_in, clamp(pos, vec2<i32>(0), c.render_size - 1), 0);
}

fn load_input_motion_vector(pos: vec2<i32>) -> vec2<f32> {
    return textureLoad(motion_in, clamp(pos, vec2<i32>(0), c.render_size - 1), 0).xy * c.motion_vector_scale;
}

fn reconstruct_prev_depth(pos: vec2<i32>, depth: f32, mv_in: vec2<f32>, size: vec2<i32>) {
    let mv = mv_in * f32(length(mv_in * vec2<f32>(c.display_size)) > 0.1);
    let uv = (vec2<f32>(pos) + 0.5) / vec2<f32>(size);
    let b = bilinear(uv + mv, c.render_size);
    for (var i = 0; i < 4; i++) {
        let w = bilinear_weight(b, i);
        if (w > RECONSTRUCTED_DEPTH_BILINEAR_WEIGHT_THRESHOLD) {
            let store_pos = b.base + bilinear_offset(i);
            if (is_on_screen(store_pos, size)) {
                atomicMax(&reconstructed_previous_depth[pixel_index(store_pos, size.x)], depth_to_store(depth));
            }
        }
    }
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let pos = vec2<i32>(gid.xy);
    if (!is_on_screen(pos, c.render_size)) {
        return;
    }
    // Nearest depth in the 3x3 neighbourhood (smallest: standard depth).
    var offsets = array<vec2<i32>, 9>(
        vec2<i32>(0, 0), vec2<i32>(1, 0), vec2<i32>(0, 1), vec2<i32>(0, -1), vec2<i32>(-1, 0),
        vec2<i32>(-1, 1), vec2<i32>(1, 1), vec2<i32>(-1, -1), vec2<i32>(1, -1),
    );
    var nearest_coord = pos;
    var nearest_depth = load_input_depth(pos);
    for (var i = 1; i < 9; i++) {
        let p = pos + offsets[i];
        if (is_on_screen(p, c.render_size)) {
            let d = load_input_depth(p);
            if (d < nearest_depth) {
                nearest_coord = p;
                nearest_depth = d;
            }
        }
    }
    let mv = load_input_motion_vector(nearest_coord);
    let i = pixel_index(pos, c.render_size.x);
    dilated_depth[i] = nearest_depth;
    dilated_motion[i] = mv;
    reconstruct_prev_depth(pos, nearest_depth, mv, c.render_size);
}
