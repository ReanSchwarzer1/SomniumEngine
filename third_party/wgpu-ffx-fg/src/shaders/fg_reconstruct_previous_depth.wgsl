// Depth of the interpolated frame: each pixel's dilated depth pushed half way
// along its motion. Translation of ffx_frameinterpolation_reconstruct_previous_depth.h.

@group(0) @binding(0) var<uniform> c: FgConstants;
@group(0) @binding(1) var<storage, read> dilated_depth: array<f32>;
@group(0) @binding(2) var<storage, read> dilated_motion: array<vec2<f32>>;
@group(0) @binding(3) var<storage, read_write> reconstructed_interpolated_depth: array<atomic<u32>>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let pos = vec2<i32>(gid.xy);
    if (!is_on_screen(pos, c.render_size)) {
        return;
    }
    let i = pixel_index(pos, c.render_size.x);
    let mv = dilated_motion[i] * 0.5;
    let depth = dilated_depth[i];
    let uv = (vec2<f32>(pos) + 0.5) / vec2<f32>(c.render_size);
    let b = bilinear(uv + mv, c.render_size);
    for (var s = 0; s < 4; s++) {
        let w = bilinear_weight(b, s);
        if (w > RECONSTRUCTED_DEPTH_BILINEAR_WEIGHT_THRESHOLD) {
            let p = b.base + bilinear_offset(s);
            if (is_on_screen(p, c.render_size)) {
                atomicMax(&reconstructed_interpolated_depth[pixel_index(p, c.render_size.x)], depth_to_store(depth));
            }
        }
    }
}
