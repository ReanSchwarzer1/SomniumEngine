// Pyramid mip 0 from the interpolated frame (display size): alpha becomes how
// much a texel is *not* to be inpainted, and nothing outside the rect counts.

@group(0) @binding(3) var interpolated: texture_2d<f32>;

fn load_source(pos: vec2<i32>) -> vec4<f32> {
    if (!is_on_screen(pos, p.src_size) || !is_in_rect(pos, c.rect_base, c.rect_size)) {
        return vec4<f32>(0.0);
    }
    var color = textureLoad(interpolated, pos, 0) * f32(c.display_size.x > 0);
    color.w = saturate(1.0 - color.w);
    return color;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let pos = vec2<i32>(gid.xy);
    if (!is_on_screen(pos, p.dst_size)) {
        return;
    }
    let s = pos * 2;
    textureStore(dst, pos, reduce4(load_source(s), load_source(s + vec2<i32>(1, 0)),
        load_source(s + vec2<i32>(0, 1)), load_source(s + vec2<i32>(1, 1))));
}
