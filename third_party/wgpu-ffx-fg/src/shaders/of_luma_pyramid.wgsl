// One level of the luma pyramid from the level above (2x2 average). AMD's SPD
// pass builds levels 1-6 in one dispatch from float intermediates and truncates
// only on store; per-level dispatches round instead, so the bias stays under
// half a step rather than growing a step per level.

@group(0) @binding(0) var<uniform> c: OfConstants;
@group(0) @binding(1) var<storage, read> luma_src: array<u32>;
@group(0) @binding(2) var<storage, read_write> luma_dst: array<u32>;

fn load(pos: vec2<i32>, size: vec2<i32>) -> f32 {
    let p = clamp(pos, vec2<i32>(0), size - 1);
    return f32(luma_src[u32(p.y) * u32(size.x) + u32(p.x)]);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dst_size = level_size(c.level);
    let src_size = level_size(c.level - 1u);
    let pos = vec2<i32>(gid.xy);
    if (pos.x >= dst_size.x || pos.y >= dst_size.y) {
        return;
    }
    let s = pos * 2;
    let avg = (load(s, src_size) + load(s + vec2<i32>(1, 0), src_size) + load(s + vec2<i32>(0, 1), src_size)
        + load(s + vec2<i32>(1, 1), src_size)) * 0.25;
    luma_dst[u32(pos.y) * u32(dst_size.x) + u32(pos.x)] = u32(avg + 0.5);
}
