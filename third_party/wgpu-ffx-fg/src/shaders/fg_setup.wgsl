// Frame counter since the last reset or scene change. Translation of
// ffx_frameinterpolation_setup.h; the field clears it also did are clear_buffer
// calls on the host, since every field here is a buffer.

@group(0) @binding(0) var<uniform> c: FgConstants;
@group(0) @binding(1) var<storage, read_write> counters: array<u32>;
@group(0) @binding(2) var<storage, read> scene_change_detection: array<u32>;

fn has_scene_changed() -> bool {
    // A change in any of the 4 previous frames (4 history bits).
    return (scene_change_detection[1] & 0xfu) != 0u;
}

@compute @workgroup_size(1, 1, 1)
fn main() {
    if (c.reset != 0u || has_scene_changed()) {
        counters[COUNTER_FRAME_INDEX_SINCE_LAST_RESET] = 0u;
    } else {
        counters[COUNTER_FRAME_INDEX_SINCE_LAST_RESET] = counters[COUNTER_FRAME_INDEX_SINCE_LAST_RESET] + 1u;
    }
}
