// Where the interpolated frame sees something hidden in the previous frame (x)
// or in the current one (y). Translation of ffx_frameinterpolation_disocclusion_mask.h.

@group(0) @binding(0) var<uniform> c: FgConstants;
@group(0) @binding(1) var<storage, read> reconstructed_interpolated_depth: array<u32>;
@group(0) @binding(2) var<storage, read> reconstructed_previous_depth: array<u32>;
@group(0) @binding(3) var<storage, read> dilated_depth: array<f32>;
@group(0) @binding(4) var<storage, read> game_mv_x: array<u32>;
@group(0) @binding(5) var<storage, read> game_mv_y: array<u32>;
@group(0) @binding(6) var pyramid: texture_2d<f32>;
@group(0) @binding(7) var disocclusion_mask: texture_storage_2d<rgba8unorm, write>;

fn load_estimated_depth(which: u32, pos: vec2<i32>) -> f32 {
    let i = pixel_index(pos, c.render_size.x);
    if (which == 0u) {
        return depth_from_store(reconstructed_previous_depth[i]);
    }
    return dilated_depth[i];
}

fn compute_depth_clip(which: u32, uv_sample: vec2<f32>, current_depth: f32) -> f32 {
    let d2v = c.device_to_view;
    let current_view = device_depth_to_view_space(current_depth, d2v);
    let b = bilinear(uv_sample, c.render_size);
    var depth_sum = 0.0;
    var weight_sum = 0.0;
    for (var s = 0; s < 4; s++) {
        let p = b.base + bilinear_offset(s);
        if (is_on_screen(p, c.render_size)) {
            let w = bilinear_weight(b, s);
            if (w > RECONSTRUCTED_DEPTH_BILINEAR_WEIGHT_THRESHOLD) {
                let prev_depth = load_estimated_depth(which, p);
                let prev_view = device_depth_to_view_space(prev_depth, d2v);
                let depth_diff = current_view - prev_view;
                if (depth_diff > 0.0) {
                    // Standard depth: the plane is the farther of the two.
                    let plane_depth = max(prev_depth, current_depth);
                    let center = view_space_position(c.render_size / 2, c.render_size, plane_depth, d2v);
                    let corner = view_space_position(vec2<i32>(0, 0), c.render_size, plane_depth, d2v);
                    let half_viewport_width = length(vec2<f32>(c.render_size));
                    let depth_threshold = min(current_view, prev_view);
                    let k_sep = 1.37e-05;
                    let k_fov = length(corner) / length(center);
                    let required_separation = k_sep * k_fov * half_viewport_width * depth_threshold;
                    depth_sum += f32(required_separation / depth_diff >= 1.0) * w;
                    weight_sum += w;
                }
            }
        }
    }
    return select(0.0, saturate(1.0 - depth_sum / weight_sum), weight_sum > 0.0);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let pos = vec2<i32>(gid.xy);
    if (!is_on_screen(pos, c.render_size)) {
        return;
    }
    let dilated = depth_from_store(reconstructed_interpolated_depth[pixel_index(pos, c.render_size.x)]);
    let depth_uv = (vec2<f32>(pos) + 0.5) / vec2<f32>(c.render_size);

    let game_mv = load_inpainted_game_field_mv(depth_uv);

    let clip_to_previous = 1.0 - compute_depth_clip(0u, depth_uv + game_mv.motion_vector, dilated);
    let clip_to_current = 1.0 - compute_depth_clip(1u, depth_uv - game_mv.motion_vector, dilated);
    var mask = vec2<f32>(f32(clip_to_previous >= FI_EPSILON), f32(clip_to_current >= FI_EPSILON));

    // No false disocclusion where the primary vector points off screen.
    let src_mv = game_mv.motion_vector * 2.0;
    let previous_pos = vec2<i32>((depth_uv + src_mv) * vec2<f32>(c.render_size));
    mask.x = saturate(mask.x + f32(!is_on_screen(previous_pos, c.render_size)));
    let current_pos = vec2<i32>((depth_uv - src_mv) * vec2<f32>(c.render_size));
    mask.y = saturate(mask.y + f32(!is_on_screen(current_pos, c.render_size)));

    textureStore(disocclusion_mask, pos, vec4<f32>(mask, 0.0, 0.0));
}
