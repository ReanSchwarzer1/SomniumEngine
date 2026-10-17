// One pyramid mip from the mip above it.

@group(0) @binding(3) var src: texture_2d<f32>;

fn load_source(pos: vec2<i32>) -> vec4<f32> {
    if (!is_on_screen(pos, p.src_size)) {
        return vec4<f32>(0.0);
    }
    return textureLoad(src, pos, 0);
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
