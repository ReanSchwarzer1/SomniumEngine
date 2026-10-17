// Scene change detection, part 1: a 256-bucket luma histogram for each of 3x3
// screen regions. Translation of ffx_opticalflow_generate_scd_histogram.h.
// Workgroup 32x8 (256 threads, one bucket each); 16 groups down, so rows step by
// 128; z picks the region.

const LBASE: u32 = 10u;
const HISTOGRAMS_PER_DIM: u32 = 3u;

@group(0) @binding(0) var<uniform> c: OfConstants;
@group(0) @binding(1) var<storage, read> luma: array<u32>;
@group(0) @binding(2) var<storage, read_write> histogram: array<atomic<u32>>;

var<workgroup> scd_buffer: array<atomic<u32>, 2560>;

fn load_luma(pos: vec2<i32>) -> u32 {
    let p = clamp(pos, vec2<i32>(0), c.luma_size - 1);
    return luma[u32(p.y) * u32(c.luma_size.x) + u32(p.x)];
}

@compute @workgroup_size(32, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) local_index: u32) {
    let div_x = u32(c.luma_size.x) / HISTOGRAMS_PER_DIM;
    let div_y = u32(c.luma_size.y) / HISTOGRAMS_PER_DIM;
    let nx = gid.z % HISTOGRAMS_PER_DIM;
    let ny = gid.z / HISTOGRAMS_PER_DIM;
    let start_x = div_x * nx;
    let start_y = div_y * ny;
    let stop_x = start_x + div_x;
    let stop_y = start_y + div_y;

    let buffer_offset = local_index * LBASE;
    for (var i = 0u; i < LBASE; i++) {
        atomicStore(&scd_buffer[buffer_offset + i], 0u);
    }
    workgroupBarrier();

    var coord = vec2<u32>(start_x + 4u * gid.x, start_y + gid.y);
    if (coord.x < stop_x) {
        let scrambling = local_index % LBASE;
        for (; coord.y < stop_y; coord.y += 128u) {
            let p = vec2<i32>(coord);
            let color = vec4<u32>(load_luma(p), load_luma(p + vec2<i32>(1, 0)), load_luma(p + vec2<i32>(2, 0)),
                load_luma(p + vec2<i32>(3, 0))) * LBASE;
            atomicAdd(&scd_buffer[color.x + scrambling], 1u);
            atomicAdd(&scd_buffer[color.y + scrambling], 1u);
            atomicAdd(&scd_buffer[color.z + scrambling], 1u);
            atomicAdd(&scd_buffer[color.w + scrambling], 1u);
        }
    }
    workgroupBarrier();

    var value = 0u;
    for (var i = 0u; i < LBASE; i++) {
        value += atomicLoad(&scd_buffer[buffer_offset + i]);
    }
    let histogram_start = 256u * gid.z;
    atomicAdd(&histogram[histogram_start + local_index], value);
}
