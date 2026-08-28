// MORROWIND-C: composition is declared here rather than assembled by a
// `format!` of `include_str!` calls at this pass's construction site. The
// resolver (`somnium_shader`) emits each module once, in this order, and
// hoists every `enable` above everything.
//!include "global_pool.wgsl"
//!include "pixel_class.wgsl"

// Somnium Engine — the pixel census (Phase DOOM-B).
//
// Counts how many pixels the shading pass will run each *class* of its code on.
// DOOM-A established that `Shading` is 25.8 of a 38.4 ms Coastal ground frame
// and that its fragment-invocation count is exactly the pixel count — so the
// cost is per-pixel, and the only remaining question is *which* pixels. That is
// this shader.
//
// The classification itself lives in `pixel_class.wgsl`, shared with the tile
// classifier that routes DOOM-C's bins. The two apply different thresholds —
// three distance buckets here, one aerial split there — but they may never
// disagree about what a pixel *is*.
//
// Concatenated after `global_pool.wgsl` and `pixel_class.wgsl`.

const BIN_SKY:          u32 = 0u;
const BIN_MESH:         u32 = 1u;
const BIN_FOLIAGE:      u32 = 2u;
const BIN_TERRAIN_NEAR: u32 = 3u;
const BIN_TERRAIN_MID:  u32 = 4u;
const BIN_TERRAIN_FAR:  u32 = 5u;
const BIN_TOTAL:        u32 = 6u;
const BIN_COUNT:        u32 = 7u;

struct CensusParams {
    width: u32,
    height: u32,
    // Camera distance in metres below which terrain counts as near, and above
    // which it counts as far. Uniforms rather than constants because DOOM-E has
    // to sweep them to choose its split, and a recompile per candidate would
    // make that a slow afternoon.
    near_split: f32,
    far_split: f32,
}

@group(1) @binding(0) var vis_buffer: texture_2d<u32>;
@group(1) @binding(1) var class_depth: texture_depth_2d;
@group(1) @binding(2) var<storage, read_write> counters: array<atomic<u32>, 7>;
@group(1) @binding(3) var<uniform> params: CensusParams;

// One set of counters per workgroup, summed into the global set once.
//
// A direct global `atomicAdd` per pixel would be 3.5 million atomics landing on
// seven addresses, which turns a census into a serialisation point and changes
// the frame it is measuring. 256 threads reduce into group-shared memory first,
// so the global traffic is one add per bin per workgroup — about 97 000 for a
// maximized Native frame instead of 3.5 million.
var<workgroup> local_counts: array<atomic<u32>, 7>;

fn census_bin(coord: vec2<i32>) -> u32 {
    let pc = pc_classify(coord, vec2<u32>(params.width, params.height));
    switch pc.kind {
        case PC_SKY: {
            return BIN_SKY;
        }
        case PC_FOLIAGE: {
            return BIN_FOLIAGE;
        }
        case PC_TERRAIN: {
            if pc.distance < params.near_split {
                return BIN_TERRAIN_NEAR;
            }
            if pc.distance < params.far_split {
                return BIN_TERRAIN_MID;
            }
            return BIN_TERRAIN_FAR;
        }
        default: {
            return BIN_MESH;
        }
    }
}

@compute @workgroup_size(16, 16, 1)
fn cs_main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    if lane < BIN_COUNT {
        atomicStore(&local_counts[lane], 0u);
    }
    workgroupBarrier();

    // The dispatch is rounded up to whole workgroups, so the last row and
    // column contain threads with no pixel. They must still reach both
    // barriers — an early `return` here would leave the group's reduction
    // waiting on invocations that never arrive.
    let inside = gid.x < params.width && gid.y < params.height;
    if inside {
        let bin = census_bin(vec2<i32>(i32(gid.x), i32(gid.y)));
        atomicAdd(&local_counts[bin], 1u);
        atomicAdd(&local_counts[BIN_TOTAL], 1u);
    }

    workgroupBarrier();
    if lane < BIN_COUNT {
        let v = atomicLoad(&local_counts[lane]);
        if v > 0u {
            atomicAdd(&counters[lane], v);
        }
    }
}
