// Back buffer to 8-bit luma at full size. Translation of
// ffx_opticalflow_prepare_luma.h for an LDR back buffer (transfer function 0:
// luminance of the stored values, not linearised, as AMD does).

@group(0) @binding(0) var<uniform> c: OfConstants;
@group(0) @binding(1) var input_color: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> luma_out: array<u32>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = c.luma_size;
    for (var y = 0; y < 2; y++) {
        for (var x = 0; x < 2; x++) {
            let pos = vec2<i32>(gid.xy) * 2 + vec2<i32>(x, y);
            if (pos.x < size.x && pos.y < size.y) {
                let rgb = textureLoad(input_color, pos, 0).rgb;
                let luminance = 0.2126 * rgb.x + 0.7152 * rgb.y + 0.0722 * rgb.z;
                luma_out[u32(pos.y) * u32(size.x) + u32(pos.x)] = min(u32(max(luminance, 0.0) * 255.0), 255u);
            }
        }
    }
}
