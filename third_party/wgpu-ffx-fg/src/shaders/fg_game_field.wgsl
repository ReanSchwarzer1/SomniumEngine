// Reading the game motion vector field, with inpainting from the pyramid where
// it holds no vector. Translation of ComputeMvInpaintingLevel and
// LoadInpaintedGameFieldMv (ffx_frameinterpolation_common.h).
//
// The including pass declares `c` (FgConstants), `game_mv_x` and `game_mv_y`
// (array<u32>, render size) and `pyramid` (texture_2d<f32>, all mips).

fn load_game_field_mv(pos: vec2<i32>) -> vec2<u32> {
    if (!is_on_screen(pos, c.render_size)) {
        return vec2<u32>(0u);
    }
    let i = pixel_index(pos, c.render_size.x);
    return vec2<u32>(game_mv_x[i], game_mv_y[i]);
}

fn load_inpainting_pyramid(mip: i32, pos: vec2<i32>) -> vec4<f32> {
    return textureLoad(pyramid, pos, mip);
}

fn compute_mv_inpainting_level(uv: vec2<f32>, mip: i32, tex_size: vec2<i32>) -> vec4<f32> {
    let b = bilinear(uv, tex_size);
    var sum = 0.0;
    var color = vec4<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let pos = b.base + bilinear_offset(i);
        if (is_on_screen(pos, tex_size)) {
            let s = load_inpainting_pyramid(mip, pos);
            let priority = s.z;
            let valid = f32(s.z > 0.0);
            let w = bilinear_weight(b, i) * valid * priority;
            sum += w;
            color += s * w;
        }
    }
    return color / select(1.0, sum, sum > 0.0);
}

fn load_inpainted_game_field_mv(uv: vec2<f32>) -> VectorFieldEntry {
    let px = vec2<i32>(uv * vec2<f32>(c.render_size));
    var e = unpack_vector_field_entries(load_game_field_mv(px));
    if (!e.valid) {
        var tex_size = c.render_size;
        var inpainted = vec4<f32>(0.0);
        let mips = min(11, i32(textureNumLevels(pyramid)));
        for (var mip = 0; mip < mips && inpainted.w == 0.0; mip++) {
            tex_size = tex_size / 2;
            inpainted = compute_mv_inpainting_level(uv, mip, tex_size);
        }
        e.motion_vector = inpainted.xy;
        e.high_priority = inpainted.z;
        e.low_priority = inpainted.w;
        e.inpainted = true;
    }
    e.neg_outside = !is_uv_inside(uv - e.motion_vector);
    e.pos_outside = !is_uv_inside(uv + e.motion_vector);
    e.velocity = length(e.motion_vector);
    return e;
}
