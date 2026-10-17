// Up a level: each finer block takes whichever of its 4 nearest coarse vectors
// (doubled) matches best on the coarse luma. Translation of
// ffx_opticalflow_scale_optical_flow_advanced_v5.h; workgroup 8x8x4, z picking the
// candidate. `c.level` is the coarse level; the output is the next level's
// prediction.

@group(0) @binding(0) var<uniform> c: OfConstants;
@group(0) @binding(1) var<storage, read> luma: array<u32>;
@group(0) @binding(2) var<storage, read> luma_previous: array<u32>;
@group(0) @binding(3) var<storage, read> optical_flow: array<vec2<i32>>;
@group(0) @binding(4) var<storage, read_write> optical_flow_next_level: array<vec2<i32>>;
@group(0) @binding(5) var<storage, read> scene_change_detection: array<u32>;

var<workgroup> nearest_vectors: array<vec2<i32>, 256>; // [4][8][8]
var<workgroup> local_region: array<u32, 256>;
var<workgroup> sads: array<u32, 256>;
var<workgroup> scene_changed_flag: u32;

fn flow_size(level: u32) -> vec2<i32> {
    let s = level_size(level);
    return vec2<i32>((s.x + 7) / 8, (s.y + 7) / 8);
}

fn shared_index(z: u32, lid: vec3<u32>) -> u32 {
    return (z * 8u + lid.y) * 8u + lid.x;
}

@compute @workgroup_size(8, 8, 4)
fn main(@builtin(global_invocation_id) gid_u: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>,
        @builtin(local_invocation_index) local_index: u32) {
    let gid = vec3<i32>(gid_u);
    let next_size = flow_size(c.level - 1u);
    let in_range = gid.x < next_size.x && gid.y < next_size.y;
    let out_index = u32(gid.y) * u32(next_size.x) + u32(gid.x);

    if (local_index == 0u) {
        scene_changed_flag = u32(is_scene_changed());
    }
    if (workgroupUniformLoad(&scene_changed_flag) != 0u) {
        if (in_range && lid.z == 0u) {
            optical_flow_next_level[out_index] = vec2<i32>(0);
        }
        return;
    }

    let x_offset = i32(lid.z % 2u) - 1 + gid.x % 2;
    let y_offset = i32(lid.z / 2u) - 1 + gid.y % 2;
    let src_pos = vec2<i32>(gid.x / 2 + x_offset, gid.y / 2 + y_offset);
    let src_size = flow_size(c.level);
    var nearest = vec2<i32>(0);
    if (all(src_pos >= vec2<i32>(0)) && all(src_pos < src_size)) {
        nearest = optical_flow[u32(src_pos.y) * u32(src_size.x) + u32(src_pos.x)];
    }
    nearest_vectors[shared_index(lid.z, lid)] = nearest * 2;

    // Row n = z of the block's 4x4 luma (one packed word each).
    let n = i32(lid.z);
    local_region[shared_index(lid.z, lid)] = load_first_image_packed_luma(vec2<i32>(gid.x * 4, gid.y * 4 + n));
    workgroupBarrier();

    var block_sad = 0u;
    for (var row = 0u; row < 4u; row++) {
        let luma_pos = vec2<i32>(gid.x * 4, gid.y * 4 + i32(row)) + nearest;
        block_sad += sad(local_region[shared_index(row, lid)], load_second_image_packed_luma(luma_pos));
    }
    sads[shared_index(lid.z, lid)] = block_sad;
    workgroupBarrier();

    if (lid.z == 0u && in_range) {
        var best_sad = 0xffffffffu;
        var best_id = 0u;
        for (var k = 0u; k < 4u; k++) {
            let s = sads[shared_index(k, lid)];
            if (s < best_sad) {
                best_sad = s;
                best_id = k;
            }
        }
        optical_flow_next_level[out_index] = nearest_vectors[shared_index(best_id, lid)];
    }
}
