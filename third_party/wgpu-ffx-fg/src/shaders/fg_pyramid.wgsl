// Inpainting pyramids, one dispatch per mip. AMD builds them with the single
// pass downsampler (SPD), whose last workgroup reads mips the others wrote
// through `globallycoherent` memory; WGSL has no such qualifier, so each mip is
// its own dispatch here. The reductions are AMD's SpdReduce4 and source loads
// from ffx_frameinterpolation_compute_game_vector_field_inpainting_pyramid.h
// (mode 0, motion vectors) and ffx_frameinterpolation_compute_inpainting_pyramid.h
// (mode 1, colour). Out-of-range texels weigh nothing.
//
// Entry points: `game_field_mip0` (source: the game motion vector field),
// `color_mip0` (source: the interpolated frame), `reduce` (source: the mip above).

struct PyramidParams {
    src_size: vec2<i32>,
    dst_size: vec2<i32>,
    mode: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> c: FgConstants;
@group(0) @binding(1) var<uniform> p: PyramidParams;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;

fn reduce4(v0: vec4<f32>, v1: vec4<f32>, v2: vec4<f32>, v3: vec4<f32>) -> vec4<f32> {
    if (p.mode == 0u) {
        // Motion vectors: average the entries that hold a vector.
        let w = vec4<f32>(f32(v0.z > 0.0), f32(v1.z > 0.0), f32(v2.z > 0.0), f32(v3.z > 0.0));
        let sum = w.x + w.y + w.z + w.w;
        let v = v0 * w.x + v1 * w.y + v2 * w.z + v3 * w.w;
        return v / select(1.0, sum, sum > FI_EPSILON);
    }
    // Colour: weighted by how much each texel is not inpainted.
    let w = vec4<f32>(v0.w, v1.w, v2.w, v3.w);
    let sum = w.x + w.y + w.z + w.w;
    if (sum == 0.0) {
        return vec4<f32>(0.0);
    }
    return (v0 * w.x + v1 * w.y + v2 * w.z + v3 * w.w) / sum;
}
