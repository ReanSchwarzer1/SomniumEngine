// Fill what the interpolated frame could not see from the colour pyramid.
// Translation of ffx_frameinterpolation_inpainting.h. The interpolated frame is
// read from one texture and the result written to another (AMD writes in place);
// the HUD-less branch is not taken, as Somnium interpolates the picture before
// its UI and draws the UI over every presented frame.

@group(0) @binding(0) var<uniform> c: FgConstants;
@group(0) @binding(1) var interpolated: texture_2d<f32>;
@group(0) @binding(2) var pyramid: texture_2d<f32>;
@group(0) @binding(3) var<storage, read> scene_change_detection: array<u32>;
@group(0) @binding(4) var output: texture_storage_2d<rgba8unorm, write>;

fn compute_inpainting_level(uv: vec2<f32>, mip: i32, tex_size: vec2<i32>) -> vec4<f32> {
    let b = bilinear(uv, tex_size);
    var color = vec4<f32>(0.0);
    for (var s = 0; s < 4; s++) {
        let p = b.base + bilinear_offset(s);
        if (is_on_screen(p, tex_size)) {
            let sample_color = textureLoad(pyramid, p, mip);
            let w = bilinear_weight(b, s) * f32(sample_color.w > 0.0);
            color += vec4<f32>(sample_color.rgb * w, w);
        }
    }
    return color;
}

fn compute_inpainting(pos: vec2<i32>, fallback: vec3<f32>) -> vec3<f32> {
    let uv = (vec2<f32>(pos) + 0.5) / vec2<f32>(c.display_size);
    var color = vec4<f32>(0.0);
    var tex_size = c.display_size;
    let mips = min(10, i32(textureNumLevels(pyramid)));
    for (var mip = 0; mip < mips; mip++) {
        tex_size = tex_size / 2;
        let mip_color = compute_inpainting_level(uv, mip, tex_size);
        if (mip_color.w > 0.0) {
            let normalized = mip_color.rgb / mip_color.w;
            let mip_weight = pow(1.0 - f32(mip) / 10.0, 3.0) * mip_color.w;
            color += vec4<f32>(normalized, 1.0) * mip_weight;
        }
    }
    // AMD divides unguarded; an empty pyramid keeps the interpolated colour.
    return select(fallback, color.rgb / color.w, color.w > 0.0);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let pos = vec2<i32>(gid.xy);
    if (!is_on_screen(pos, c.display_size)) {
        return;
    }
    var color = textureLoad(interpolated, pos, 0);
    let inpainting = color.w;
    if (inpainting > FI_EPSILON) {
        color = vec4<f32>(mix(color.rgb, compute_inpainting(pos, color.rgb) * f32(c.display_size.x > 0), inpainting), color.w);
    }
    if ((c.dispatch_flags & DRAW_DEBUG_TEAR_LINES) != 0u) {
        if (pos.x < 16) {
            color.g = 1.0;
        } else if (pos.x > c.display_size.x - 16) {
            color = vec4<f32>(color.rgb + vec3<f32>(0.0, 0.5, 1.0), color.w);
        }
    }
    if ((c.dispatch_flags & DRAW_DEBUG_RESET_INDICATORS) != 0u) {
        if (pos.y < 32 && c.reset != 0u) {
            color.r = 1.0;
        } else if (pos.y > 32 && pos.y < 64 && (scene_change_detection[1] & 0xfu) != 0u) {
            color.b = 1.0;
        }
    }
    textureStore(output, pos, vec4<f32>(color.rgb, 1.0));
}
