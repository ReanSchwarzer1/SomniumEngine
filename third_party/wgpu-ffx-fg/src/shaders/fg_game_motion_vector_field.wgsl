// The game's motion vectors scattered to where each pixel sits half way between
// the two frames, prioritised by depth (near wins) and by how well the colours
// match. Translation of ffx_frameinterpolation_game_motion_vector_field.h.

@group(0) @binding(0) var<uniform> c: FgConstants;
@group(0) @binding(1) var<storage, read> dilated_depth: array<f32>;
@group(0) @binding(2) var<storage, read> dilated_motion: array<vec2<f32>>;
@group(0) @binding(3) var previous_source: texture_2d<f32>;
@group(0) @binding(4) var current_source: texture_2d<f32>;
@group(0) @binding(5) var linear_clamp: sampler;
@group(0) @binding(6) var<storage, read_write> game_mv_x: array<atomic<u32>>;
@group(0) @binding(7) var<storage, read_write> game_mv_y: array<atomic<u32>>;

fn priority_from_view_space_depth(depth_in_meters: f32) -> u32 {
    let d = pow(max(depth_in_meters, 0.0), 0.33);
    let factor = u32((1.0 - d * (1.0 / (1.0 + d))) * f32(PRIORITY_HIGH_MAX));
    return max(1u, factor);
}

fn update_field(pos: vec2<i32>, packed: vec2<u32>) {
    let i = pixel_index(pos, c.render_size.x);
    atomicMax(&game_mv_x[i], packed.x);
    atomicMax(&game_mv_y[i], packed.y);
}

fn update_field_ex(pos: vec2<i32>, packed: vec2<u32>) -> u32 {
    let i = pixel_index(pos, c.render_size.x);
    let previous_x = atomicMax(&game_mv_x[i], packed.x);
    let previous_y = atomicMax(&game_mv_y[i], packed.y);
    return max(previous_x, previous_y);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let pos = vec2<i32>(gid.xy);
    if (!is_on_screen(pos, c.render_size)) {
        return;
    }
    let uv_screen = (vec2<f32>(pos) + 0.5) / vec2<f32>(c.render_size);
    let rect_start = vec2<f32>(c.rect_base) / vec2<f32>(c.display_size);
    let letterbox = vec2<f32>(c.rect_size) / vec2<f32>(c.display_size);
    let uv_rect = rect_start + uv_screen * letterbox;

    let i = pixel_index(pos, c.render_size.x);
    let depth = dilated_depth[i];
    let game_mv = dilated_motion[i];
    let mv_half = game_mv * 0.5;
    let interpolated_uv = uv_screen + mv_half;

    let view_depth = device_depth_to_view_space(depth, c.device_to_view);
    let high_primary = priority_from_view_space_depth(view_depth);

    // Current pixel's colour in the previous frame and in the current one.
    let prev_col = textureSampleLevel(previous_source, linear_clamp, uv_rect + game_mv * letterbox, 0.0).rgb;
    let curr_col = textureSampleLevel(current_source, linear_clamp, uv_rect, 0.0).rgb;
    let prev_luma = 0.001 + raw_to_luminance(prev_col);
    let curr_luma = 0.001 + raw_to_luminance(curr_col);

    var low = u32(round(min_divided_by_max(prev_luma, curr_luma) * f32(PRIORITY_LOW_MAX)))
        * u32(is_uv_inside(uv_rect + game_mv * letterbox));

    // Primary vectors.
    {
        let packed = pack_vector_field_entries(true, high_primary, low, mv_half);
        let b = bilinear(interpolated_uv, c.render_size);
        for (var s = 0; s < 4; s++) {
            let p = b.base + bilinear_offset(s);
            if (is_on_screen(p, c.render_size)) {
                update_field(p, packed);
            }
        }
    }

    // Secondary vectors, mainly to improve the inpainted ones.
    if (length(mv_half * vec2<f32>(c.render_size)) > FI_EPSILON) {
        var write_secondary = true;
        var primary_hits = 0u;
        let step = length(1.0 / vec2<f32>(c.render_size));
        let step_mv = normalize(game_mv);
        let break_dist = min(length(mv_half), length(vec2<f32>(0.5, 0.5)));
        // Reverse depth priority for secondary vectors.
        let high_secondary = max(1u, PRIORITY_HIGH_MAX - high_primary);

        for (var scale = step; scale <= break_dist && write_secondary; scale += step) {
            let secondary_uv = interpolated_uv - step_mv * scale;
            let b = bilinear(secondary_uv, c.render_size);
            let to_center_raw = vec2<f32>(0.5, 0.5) - secondary_uv;
            let to_center = select(vec2<f32>(0.0), normalize(to_center_raw), length(to_center_raw) > 0.0);
            low = u32(max(0.0, dot(to_center, step_mv)) * f32(PRIORITY_LOW_MAX));
            let packed = pack_vector_field_entries(false, high_secondary, low, mv_half);
            // Only the first bilinear location.
            let p = b.base;
            write_secondary = write_secondary && is_on_screen(p, c.render_size);
            if (write_secondary) {
                let existing = update_field_ex(p, packed);
                primary_hits += u32(packed_entry_is_primary(existing));
                write_secondary = write_secondary && primary_hits <= 3u;
            }
        }
    }
}
