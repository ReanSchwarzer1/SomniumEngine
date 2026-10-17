// Scene change detection, part 2: each histogram smoothed and compared with last
// frame's by symmetric KL divergence; the last of the 27 workgroups folds the
// three shifted comparisons into the scene change value and history bits.
// Translation of ffx_opticalflow_compute_scd_divergence.h, with one change: AMD's
// y==1 groups overwrite the previous histogram and zero the current one while
// other groups may still be reading them. Here they write `next_previous`, and
// the host copies it over `previous_histogram` and clears `histogram` after the
// dispatch. Barriers stand in for wave lockstep in the final reduction steps.

const FACTOR: f32 = 1000000.0;
const WHERE_TO_STOP: u32 = 26u; // 3 * 9 - 1
const HISTOGRAM_COUNT: f32 = 9.0;

@group(0) @binding(0) var<uniform> c: OfConstants;
@group(0) @binding(1) var<storage, read> histogram: array<u32>;
@group(0) @binding(2) var<storage, read> previous_histogram: array<f32>;
@group(0) @binding(3) var<storage, read_write> next_previous: array<f32>;
@group(0) @binding(4) var<storage, read_write> scd_temp: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> scene_change_detection: array<atomic<u32>>;

var<workgroup> source_histogram: array<f32, 256>;
var<workgroup> filtered_histogram: array<f32, 256>;
var<workgroup> temp_buffer: array<f32, 256>;
var<workgroup> temp_buffer2: array<vec2<f32>, 256>;

fn kernel(i: i32) -> f32 {
    switch i {
        case 0: { return 0.0088122291; }
        case 1: { return 0.027143577; }
        case 2: { return 0.065114059; }
        case 3: { return 0.12164907; }
        case 4: { return 0.17699835; }
        default: { return 0.20056541; }
    }
}

fn source_at(i: i32) -> f32 {
    return source_histogram[clamp(i, 0, 255)];
}

@compute @workgroup_size(256, 1, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) local_index: u32) {
    let li = i32(local_index);
    source_histogram[local_index] = f32(histogram[gid.x]);
    workgroupBarrier();

    let read = li - 5;
    var val = 0.0;
    val += kernel(0) * source_at(read + 0);
    val += kernel(1) * source_at(read + 1);
    val += kernel(2) * source_at(read + 2);
    val += kernel(3) * source_at(read + 3);
    val += kernel(4) * source_at(read + 4);
    val += kernel(5) * source_at(read + 5);
    val += kernel(4) * source_at(read + 6);
    val += kernel(3) * source_at(read + 7);
    val += kernel(2) * source_at(read + 8);
    val += kernel(1) * source_at(read + 9);
    val += kernel(0) * source_at(read + 10);
    val += 1.0;

    if (gid.y == 0u) {
        if (li == 0) {
            filtered_histogram[255] = 1.0;
        } else {
            filtered_histogram[li - 1] = val;
        }
    } else if (gid.y == 1u) {
        filtered_histogram[li] = val;
    } else if (gid.y == 2u) {
        if (li == 255) {
            filtered_histogram[0] = 1.0;
        } else {
            filtered_histogram[li + 1] = val;
        }
    }
    workgroupBarrier();

    temp_buffer[li] = filtered_histogram[li];
    workgroupBarrier();
    for (var s = 128; s >= 1; s = s >> 1u) {
        if (li < s) {
            temp_buffer[li] += temp_buffer[li + s];
        }
        workgroupBarrier();
    }

    filtered_histogram[li] /= temp_buffer[0];
    let current = filtered_histogram[li];
    let previous = previous_histogram[gid.x];
    temp_buffer2[li] = vec2<f32>(current * log(current / previous), previous * log(previous / current));
    workgroupBarrier();
    for (var s = 128; s >= 1; s = s >> 1u) {
        if (li < s) {
            temp_buffer2[li] += temp_buffer2[li + s];
        }
        workgroupBarrier();
    }

    if (li == 0) {
        let sum = temp_buffer2[0];
        let res_float = 1.0 - exp(-(abs(sum.x) + abs(sum.y)));
        let res_uint = u32((res_float / HISTOGRAM_COUNT) * FACTOR);
        atomicAdd(&scd_temp[gid.y], res_uint);
        let old_finished = atomicAdd(&scene_change_detection[SCD_OUTPUT_COMPLETED_WORKGROUPS_SLOT], 1u);
        if (old_finished == WHERE_TO_STOP) {
            let res0 = atomicLoad(&scd_temp[0]);
            let res1 = atomicLoad(&scd_temp[1]);
            let res2 = atomicLoad(&scd_temp[2]);
            let scene_change_value = f32(min(res0, min(res1, res2))) / FACTOR;
            var history = atomicLoad(&scene_change_detection[SCD_OUTPUT_HISTORY_BITS_SLOT]) << 1u;
            if (scene_change_value > 0.45) {
                history = history | 1u;
            }
            atomicStore(&scene_change_detection[SCD_OUTPUT_SCENE_CHANGE_SLOT], bitcast<u32>(scene_change_value));
            atomicStore(&scene_change_detection[SCD_OUTPUT_HISTORY_BITS_SLOT], history);
            atomicStore(&scene_change_detection[SCD_OUTPUT_COMPLETED_WORKGROUPS_SLOT], 0u);
            atomicStore(&scd_temp[0], 0u);
            atomicStore(&scd_temp[1], 0u);
            atomicStore(&scd_temp[2], 0u);
        }
    }
    if (gid.y == 1u) {
        next_previous[gid.x] = current;
    }
}
