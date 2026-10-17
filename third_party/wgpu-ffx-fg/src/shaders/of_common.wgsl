// FidelityFX optical flow: shared definitions.
//
// WGSL translation of ffx_opticalflow_common.h and the accessors of
// ffx_opticalflow_callbacks_hlsl.h (AMD FidelityFX SDK 1.1.4, MIT; see LICENSE).
// The non-MSAD path (AMD's choice on wave32 hardware). Luma levels and flow
// vectors live in storage buffers indexed `y * width + x`; wave intrinsics are
// replaced by workgroup reductions in the passes that used them.

struct OfConstants {
    luma_size: vec2<i32>,
    level: u32,
    level_count: u32,
    frame_index: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

const SCD_OUTPUT_SCENE_CHANGE_SLOT: u32 = 0u;
const SCD_OUTPUT_HISTORY_BITS_SLOT: u32 = 1u;
const SCD_OUTPUT_COMPLETED_WORKGROUPS_SLOT: u32 = 2u;

fn level_size(level: u32) -> vec2<i32> {
    return vec2<i32>(c.luma_size.x >> level, c.luma_size.y >> level);
}

fn get_packed_luma(width: i32, x: i32, luma0: u32, luma1: u32, luma2: u32, luma3: u32) -> u32 {
    var packed = luma0 | (luma1 << 8u) | (luma2 << 16u) | (luma3 << 24u);
    if (x < 0) {
        let filler = packed & 0xffu;
        if (x <= -1) { packed = (packed << 8u) | filler; }
        if (x <= -2) { packed = (packed << 8u) | filler; }
        if (x <= -3) { packed = (packed << 8u) | filler; }
    } else if (x > width - 4) {
        let filler = packed & 0xff000000u;
        if (x >= width - 3) { packed = (packed >> 8u) | filler; }
        if (x >= width - 2) { packed = (packed >> 8u) | filler; }
        if (x >= width - 1) { packed = (packed >> 8u) | filler; }
    }
    return packed;
}

fn sad(a: u32, b: u32) -> u32 {
    return u32(abs(i32((a >> 0u) & 0xffu) - i32((b >> 0u) & 0xffu))
        + abs(i32((a >> 8u) & 0xffu) - i32((b >> 8u) & 0xffu))
        + abs(i32((a >> 16u) & 0xffu) - i32((b >> 16u) & 0xffu))
        + abs(i32((a >> 24u) & 0xffu) - i32((b >> 24u) & 0xffu)));
}

fn qsad(a0_in: u32, a1_in: u32, b: u32) -> vec4<u32> {
    var a0 = a0_in;
    var a1 = a1_in;
    var r: vec4<u32>;
    r.x = sad(a0, b);
    a0 = (a0 >> 8u) | ((a1 & 0xffu) << 24u);
    a1 = a1 >> 8u;
    r.y = sad(a0, b);
    a0 = (a0 >> 8u) | ((a1 & 0xffu) << 24u);
    a1 = a1 >> 8u;
    r.z = sad(a0, b);
    a0 = (a0 >> 8u) | ((a1 & 0xffu) << 24u);
    r.w = sad(a0, b);
    return r;
}
