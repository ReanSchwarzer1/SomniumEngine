// Pyramid mip 0 from the game motion vector field (render size).

@group(0) @binding(3) var<storage, read> game_mv_x: array<u32>;
@group(0) @binding(4) var<storage, read> game_mv_y: array<u32>;

fn load_source(pos: vec2<i32>) -> vec4<f32> {
    if (!is_on_screen(pos, p.src_size)) {
        return vec4<f32>(0.0);
    }
    let i = pixel_index(pos, p.src_size.x);
    let e = unpack_vector_field_entries(vec2<u32>(game_mv_x[i], game_mv_y[i]));
    return vec4<f32>(e.motion_vector, e.high_priority, e.low_priority) * f32(c.display_size.x > 0);
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
