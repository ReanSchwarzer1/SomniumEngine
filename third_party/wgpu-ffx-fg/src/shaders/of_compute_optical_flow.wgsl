// Block matching at one pyramid level: each 8x8 block of this frame's luma is
// searched for in the previous frame within +-8 pixels of the coarser level's
// vector. Translation of ffx_opticalflow_compute_optical_flow_v5.h (non-MSAD,
// top-left bias fix, local search fallback). One workgroup of 64 threads covers
// 16x16 pixels (2x2 blocks). AMD's wave sums and minimums (with a groupshared
// fix-up for wave32) are workgroup tree reductions here; AMD reads the
// prediction from and writes the result to one texture, where here the
// prediction is its own buffer, written by the coarser level's scale pass.

const COMPARE_SIZE: i32 = 8;
const SEARCH_RADIUS: i32 = 8;
const THREAD_COUNT: i32 = 64;
const SEARCH_BUFFER_SIZE_X: i32 = 6; // (COMPARE_SIZE + SEARCH_RADIUS * 2) / 4
const SEARCH_BUFFER_SIZE_Y: i32 = 24; // COMPARE_SIZE + SEARCH_RADIUS * 2
const SAD_MAP_ROW: i32 = 5; // (SEARCH_RADIUS * 2) / 4 + bank breaker

@group(0) @binding(0) var<uniform> c: OfConstants;
@group(0) @binding(1) var<storage, read> luma: array<u32>;
@group(0) @binding(2) var<storage, read> luma_previous: array<u32>;
@group(0) @binding(3) var<storage, read> prediction: array<vec2<i32>>;
@group(0) @binding(4) var<storage, read_write> optical_flow: array<vec2<i32>>;
@group(0) @binding(5) var<storage, read> scene_change_detection: array<u32>;

var<workgroup> pixels: array<u32, 16>; // [8][2]
var<workgroup> search_buffer: array<u32, 144>; // SEARCH_BUFFER_SIZE_Y * SEARCH_BUFFER_SIZE_X
var<workgroup> sad_map: array<u32, 320>; // [4][16][5]
var<workgroup> reduction: array<u32, 64>;
var<workgroup> scene_changed_flag: u32;

fn flow_size() -> vec2<i32> {
    let s = level_size(c.level);
    return vec2<i32>((s.x + 7) / 8, (s.y + 7) / 8);
}

fn load_prediction(pos: vec2<i32>) -> vec2<i32> {
    let size = flow_size();
    if (any(pos < vec2<i32>(0)) || any(pos >= size)) {
        return vec2<i32>(0);
    }
    return prediction[u32(pos.y) * u32(size.x) + u32(pos.x)];
}

fn store_optical_flow(pos: vec2<i32>, v: vec2<i32>) {
    let size = flow_size();
    if (all(pos >= vec2<i32>(0)) && all(pos < size)) {
        optical_flow[u32(pos.y) * u32(size.x) + u32(pos.x)] = v;
    }
}

// Every thread passes its value; every thread gets the workgroup's sum or min.
fn reduce_sum(li: u32, v: u32) -> u32 {
    reduction[li] = v;
    workgroupBarrier();
    for (var s = 32u; s >= 1u; s = s >> 1u) {
        if (li < s) {
            reduction[li] = reduction[li] + reduction[li + s];
        }
        workgroupBarrier();
    }
    let r = reduction[0];
    workgroupBarrier();
    return r;
}

fn reduce_min(li: u32, v: u32) -> u32 {
    reduction[li] = v;
    workgroupBarrier();
    for (var s = 32u; s >= 1u; s = s >> 1u) {
        if (li < s) {
            reduction[li] = min(reduction[li], reduction[li + s]);
        }
        workgroupBarrier();
    }
    let r = reduction[0];
    workgroupBarrier();
    return r;
}

fn sad_map_index(k: i32, search_id: vec2<i32>) -> i32 {
    return (k * 16 + search_id.y) * SAD_MAP_ROW + search_id.x;
}

fn load_search_buffer(li: i32, shifted: vec2<i32>) {
    let base_x = shifted.x - SEARCH_RADIUS;
    let base_y = shifted.y - SEARCH_RADIUS;
    for (var id = li; id < SEARCH_BUFFER_SIZE_X * SEARCH_BUFFER_SIZE_Y; id += THREAD_COUNT) {
        let idx = id % SEARCH_BUFFER_SIZE_X;
        let idy = id / SEARCH_BUFFER_SIZE_X;
        search_buffer[id] = load_second_image_packed_luma(vec2<i32>(base_x + idx * 4, base_y + idy));
    }
    workgroupBarrier();
}

fn calculate_qsads(search_id: vec2<i32>) -> vec4<u32> {
    var s = vec4<u32>(0u);
    for (var dy = 0; dy < COMPARE_SIZE; dy++) {
        let row = (search_id.y + dy) * SEARCH_BUFFER_SIZE_X;
        let a0 = search_buffer[row + search_id.x];
        let a1 = search_buffer[row + search_id.x + 1];
        let a2 = search_buffer[row + search_id.x + 2];
        s += qsad(a0, a1, pixels[dy * 2 + 0]);
        s += qsad(a1, a2, pixels[dy * 2 + 1]);
    }
    return s;
}

fn encode_search_coord(coord: vec2<i32>) -> u32 {
    let abs_coord = vec2<u32>(abs(coord - vec2<i32>(8)));
    return (abs_coord.y << 12u) | (abs_coord.x << 8u) | (u32(coord.y) << 4u) | u32(coord.x);
}

fn decode_search_coord(bits: u32) -> vec2<i32> {
    return vec2<i32>(i32(bits & 0xfu) - SEARCH_RADIUS, i32((bits >> 4u) & 0xfu) - SEARCH_RADIUS);
}

fn prepare_sad_map(search_id: vec2<i32>, q: vec4<u32>) {
    sad_map[sad_map_index(0, search_id)] = (q.x << 16u) | encode_search_coord(vec2<i32>(search_id.x * 4 + 0, search_id.y));
    sad_map[sad_map_index(1, search_id)] = (q.y << 16u) | encode_search_coord(vec2<i32>(search_id.x * 4 + 1, search_id.y));
    sad_map[sad_map_index(2, search_id)] = (q.z << 16u) | encode_search_coord(vec2<i32>(search_id.x * 4 + 2, search_id.y));
    sad_map[sad_map_index(3, search_id)] = (q.w << 16u) | encode_search_coord(vec2<i32>(search_id.x * 4 + 3, search_id.y));
    workgroupBarrier();
}

fn bfe(src: u32, off: u32, bits: u32) -> u32 {
    return (src >> off) & ((1u << bits) - 1u);
}

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) local_index: u32) {
    let li = i32(local_index);
    let group_id = vec2<i32>(wg.xy);
    let search_id = vec2<i32>(i32(bfe(local_index, 0u, 2u)), i32(bfe(local_index, 2u, 4u)));
    let lane_to_block = i32(bfe(local_index, 1u, 1u) | (bfe(local_index, 5u, 1u) << 1u));
    let px_pos = (group_id << vec2<u32>(4u)) + search_id * vec2<i32>(4, 1);
    let current_of_pos = px_pos >> vec2<u32>(3u);

    if (local_index == 0u) {
        scene_changed_flag = u32(is_scene_changed());
    }
    if (workgroupUniformLoad(&scene_changed_flag) != 0u) {
        if ((search_id.y & 7) == 0 && (search_id.x & 1) == 0) {
            store_optical_flow(current_of_pos, vec2<i32>(0));
        }
        return;
    }

    let use_prediction = c.level != c.level_count - 1u;
    let packed_luma = load_first_image_packed_luma(px_pos);
    let prev_packed_luma = load_second_image_packed_luma(px_pos);
    let sad_4blocks = sad(packed_luma, prev_packed_luma);

    let of_group_offset = group_id << vec2<u32>(1u);
    let pixel_group_offset = group_id << vec2<u32>(4u);

    for (var by = 0; by < 2; by++) {
        for (var bx = 0; bx < 2; bx++) {
            let block_id = vec2<i32>(bx, by);
            var current_vector = load_prediction(of_group_offset + block_id);
            if (!use_prediction) {
                current_vector = vec2<i32>(0);
            }
            if (lane_to_block == by * 2 + bx) {
                pixels[(search_id.y & 7) * 2 + (search_id.x & 1)] = packed_luma;
            }
            load_search_buffer(li, pixel_group_offset + block_id * 8 + current_vector);
            let q = calculate_qsads(search_id);
            prepare_sad_map(search_id, q);

            let own_min = min(min(sad_map[sad_map_index(0, search_id)], sad_map[sad_map_index(1, search_id)]),
                min(sad_map[sad_map_index(2, search_id)], sad_map[sad_map_index(3, search_id)]));
            let min_sad = reduce_min(local_index, own_min);
            var new_vector = current_vector + decode_search_coord(min_sad);

            // Local search fallback: keep the block still if not moving matches as well.
            let block_sad_sum = reduce_sum(local_index, select(0u, sad_4blocks, lane_to_block == bx + by * 2));
            if (c.level == 0u && block_sad_sum <= (min_sad >> 16u)) {
                new_vector = vec2<i32>(0);
            }
            if (local_index == 0u) {
                store_optical_flow(of_group_offset + block_id, new_vector);
            }
        }
    }
}
