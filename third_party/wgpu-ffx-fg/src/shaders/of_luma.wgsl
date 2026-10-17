// Packed luma reads (four pixels per word) from the current level of both
// frames. The including pass declares `c` (OfConstants), `luma` (this frame)
// and `luma_previous` (the frame before), both array<u32> at level `c.level`.
// LoadFirstImagePackedLuma / LoadSecondImagePackedLuma in AMD's callbacks.

fn packed_luma_index(pos: vec2<i32>) -> vec2<u32> {
    let size = level_size(c.level);
    let adjusted = vec2<i32>(clamp(pos.x, 0, max(size.x - 4, 0)), clamp(pos.y, 0, size.y - 1));
    return vec2<u32>(u32(adjusted.y) * u32(size.x) + u32(adjusted.x), u32(size.x));
}

fn load_first_image_packed_luma(pos: vec2<i32>) -> u32 {
    let i = packed_luma_index(pos);
    return get_packed_luma(i32(i.y), pos.x, luma[i.x], luma[i.x + 1u], luma[i.x + 2u], luma[i.x + 3u]);
}

fn load_second_image_packed_luma(pos: vec2<i32>) -> u32 {
    let i = packed_luma_index(pos);
    return get_packed_luma(i32(i.y), pos.x, luma_previous[i.x], luma_previous[i.x + 1u], luma_previous[i.x + 2u],
        luma_previous[i.x + 3u]);
}

fn is_scene_changed() -> bool {
    if (c.frame_index <= 5u) {
        return true;
    }
    return (scene_change_detection[SCD_OUTPUT_HISTORY_BITS_SLOT] & 0xfu) != 0u;
}
