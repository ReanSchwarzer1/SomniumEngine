// The interpolated frame: the previous and current back buffers fetched along
// the game's vectors and along optical flow, blended by disocclusion and by how
// well each pair agrees. Alpha carries how much the pixel is to be inpainted.
// Translation of ffx_frameinterpolation.h.

@group(0) @binding(0) var<uniform> c: FgConstants;
@group(0) @binding(1) var<storage, read> game_mv_x: array<u32>;
@group(0) @binding(2) var<storage, read> game_mv_y: array<u32>;
@group(0) @binding(3) var pyramid: texture_2d<f32>;
@group(0) @binding(4) var<storage, read> of_mv_x: array<u32>;
@group(0) @binding(5) var<storage, read> of_mv_y: array<u32>;
@group(0) @binding(6) var disocclusion_mask: texture_2d<f32>;
@group(0) @binding(7) var previous_source: texture_2d<f32>;
@group(0) @binding(8) var current_source: texture_2d<f32>;
@group(0) @binding(9) var linear_clamp: sampler;
@group(0) @binding(10) var<storage, read> counters: array<u32>;
@group(0) @binding(11) var interpolated: texture_storage_2d<rgba8unorm, write>;

struct SourceColor {
    raw: vec3<f32>,
    weight_sum: f32,
}

fn frame_index_since_last_reset() -> u32 {
    return counters[COUNTER_FRAME_INDEX_SINCE_LAST_RESET];
}

fn sample_bilinear(is_current: bool, uv: vec2<f32>, mv: vec2<f32>, tex_size: vec2<i32>) -> SourceColor {
    let b = bilinear(uv + mv, tex_size);
    var color = vec3<f32>(0.0);
    var weight_sum = 0.0;
    for (var s = 0; s < 4; s++) {
        let p = b.base + bilinear_offset(s);
        if (is_in_rect(p, c.rect_base, c.rect_size)) {
            let w = bilinear_weight(b, s);
            if (is_current) {
                color += textureLoad(current_source, p, 0).rgb * w;
            } else {
                color += textureLoad(previous_source, p, 0).rgb * w;
            }
            weight_sum += w;
        }
    }
    return SourceColor(select(vec3<f32>(0.0), color / weight_sum, weight_sum != 0.0), weight_sum);
}

fn sample_optical_flow_field(uv: vec2<f32>) -> VectorFieldEntry {
    let size = c.optical_flow_size;
    let b = bilinear(uv, size);
    var e = new_vector_field_entry();
    var weight_sum = 0.0;
    for (var s = 0; s < 4; s++) {
        let p = b.base + bilinear_offset(s);
        if (is_on_screen(p, size)) {
            let w = bilinear_weight(b, s);
            let i = pixel_index(p, size.x);
            let entry = unpack_vector_field_entries(vec2<u32>(of_mv_x[i], of_mv_y[i]));
            e.motion_vector += entry.motion_vector * w;
            e.high_priority += entry.high_priority * w;
            e.low_priority += entry.low_priority * w;
            weight_sum += w;
        }
    }
    if (weight_sum > 0.0) {
        e.motion_vector /= weight_sum;
        e.high_priority /= weight_sum;
        e.low_priority /= weight_sum;
    }
    e.neg_outside = !is_uv_inside(uv - e.motion_vector);
    e.pos_outside = !is_uv_inside(uv + e.motion_vector);
    e.velocity = length(e.motion_vector);
    return e;
}

struct Interpolated {
    color: vec3<f32>,
    inpainting_weight: f32,
}

fn compute_interpolated_color(pos: vec2<i32>) -> Interpolated {
    let uv_rect = (vec2<f32>(pos - c.rect_base) + 0.5) / vec2<f32>(c.rect_size);
    let uv_screen = (vec2<f32>(pos) + 0.5) / vec2<f32>(c.display_size);
    let lr_uv_rect = uv_rect * (vec2<f32>(c.render_size) / vec2<f32>(c.max_render_size));
    let letterbox = vec2<f32>(c.rect_size) / vec2<f32>(c.display_size);

    // Game vectors are top-left aligned; this scales them to render-size UV.
    let game_mv = load_inpainted_game_field_mv(uv_rect);
    // Optical flow ran on the back buffers, letterbox and all.
    let of_mv = sample_optical_flow_field(uv_screen);

    // Binarised disocclusion.
    var disocclusion = vec2<f32>(saturate(textureSampleLevel(disocclusion_mask, linear_clamp, lr_uv_rect, 0.0).xy)
        == vec2<f32>(1.0, 1.0));

    let prev_game = sample_bilinear(false, uv_screen, game_mv.motion_vector * letterbox, c.display_size);
    let curr_game = sample_bilinear(true, uv_screen, -game_mv.motion_vector * letterbox, c.display_size);
    let prev_of = sample_bilinear(false, uv_screen, of_mv.motion_vector * letterbox, c.display_size);
    let curr_of = sample_bilinear(true, uv_screen, -of_mv.motion_vector * letterbox, c.display_size);

    var inpainting = 0.0;
    var color: vec3<f32>;
    var disoccluded = 0.0;
    {
        // x of 1: the interpolated pixel lies inside the previous frame; y: the current one.
        disocclusion.x *= f32(!game_mv.pos_outside);
        disocclusion.y *= f32(!game_mv.neg_outside);

        // Inpaint where both directions are disoccluded.
        inpainting = saturate(max(inpainting, f32(length(disocclusion) <= FI_EPSILON)));

        var t = 0.5;
        t += 0.5 * (1.0 - disocclusion.x);
        t -= 0.5 * (1.0 - disocclusion.y);
        color = mix(prev_game.raw, curr_game.raw, saturate(t));
        disoccluded = saturate(1.0 - min(disocclusion.x, disocclusion.y));

        if (prev_game.weight_sum == 0.0) {
            color = curr_game.raw;
        } else if (curr_game.weight_sum == 0.0) {
            color = prev_game.raw;
        }
        if (prev_game.weight_sum == 0.0 && curr_game.weight_sum == 0.0) {
            inpainting = 1.0;
        }
    }
    {
        var of_t = 0.5;
        if (prev_of.weight_sum > 0.0 && curr_of.weight_sum > 0.0) {
            of_t = 0.5;
        } else if (prev_of.weight_sum > 0.0) {
            of_t = 0.0;
        } else {
            of_t = 1.0;
        }
        let of_color = mix(prev_of.raw, curr_of.raw, of_t);

        let of_sim = normalized_dot3(prev_of.raw, curr_of.raw);
        var game_sim = normalized_dot3(prev_game.raw, curr_game.raw);
        game_sim = mix(max(FI_EPSILON, game_sim), 1.0, saturate(disoccluded));
        var game_mv_bias = saturate(game_sim / max(FI_EPSILON, of_sim));

        let frame_index_factor = f32(frame_index_since_last_reset() < 10u);
        game_mv_bias = mix(game_mv_bias, 1.0, frame_index_factor);

        color = mix(of_color, color, saturate(game_mv_bias));
    }
    return Interpolated(color, inpainting);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let pos = vec2<i32>(gid.xy);
    if (!is_on_screen(pos, c.display_size)) {
        return;
    }
    var color = vec3<f32>(0.0);
    var inpainting = 0.0;
    if (!is_in_rect(pos, c.rect_base, c.rect_size) || frame_index_since_last_reset() == 0u) {
        // Just reset, or outside the rect: the current back buffer, not interpolated.
        color = textureLoad(current_source, pos, 0).rgb;
    } else {
        let r = compute_interpolated_color(pos);
        color = r.color;
        inpainting = r.inpainting_weight;
    }
    textureStore(interpolated, pos, vec4<f32>(color, inpainting));
}
