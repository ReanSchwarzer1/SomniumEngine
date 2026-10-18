// Editor selection and hover glow over the finished picture (Phase ALYX-J).
//
// Two coverage masks, made by the game highlight's mask pass from what is actually visible: what is selected and
// what the pointer is over. Selection gets a bright rim inside the shape, a faint fill, and a soft glow outside it
// that reads on a dark scene; hover gets a thin line only, so the two never look alike.

struct GlowParams {
    selection: vec4<f32>,
    hover: vec4<f32>,
    inverse_output: vec2<f32>,
    line_px: f32,
    glow_px: f32,
    source_uv_offset: vec2<f32>,
    fill: f32,
    has_hover: f32,
}

@group(0) @binding(0) var selection_mask: texture_2d<f32>;
@group(0) @binding(1) var hover_mask: texture_2d<f32>;
@group(0) @binding(2) var glow_sampler: sampler;
@group(0) @binding(3) var<uniform> glow: GlowParams;

struct FullscreenVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> FullscreenVertex {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return FullscreenVertex(vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0), vec2<f32>(uv.x, 1.0 - uv.y));
}

fn coverage(mask: texture_2d<f32>, uv: vec2<f32>) -> f32 {
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return 0.0;
    }
    return textureSampleLevel(mask, glow_sampler, uv + glow.source_uv_offset, 0.0).r;
}

const TAPS: i32 = 12;
const TAU: f32 = 6.2831853;

// The most and the least coverage on a ring `radius` pixels round `uv`.
fn ring(mask: texture_2d<f32>, uv: vec2<f32>, radius: f32) -> vec2<f32> {
    var most = 0.0;
    var least = 1.0;
    for (var i = 0; i < TAPS; i++) {
        let angle = f32(i) * (TAU / f32(TAPS));
        let c = coverage(mask, uv + vec2<f32>(cos(angle), sin(angle)) * glow.inverse_output * radius);
        most = max(most, c);
        least = min(least, c);
    }
    return vec2<f32>(most, least);
}

@fragment
fn fs_main(in: FullscreenVertex) -> @location(0) vec4<f32> {
    var colour = vec4<f32>(0.0);
    if glow.selection.a > 0.0 {
        let centre = coverage(selection_mask, in.uv);
        if centre > 0.5 {
            // Inside: a rim where the shape ends, over a faint wash of the colour.
            let rim = 1.0 - ring(selection_mask, in.uv, glow.line_px).y;
            colour = vec4<f32>(glow.selection.rgb, max(glow.fill, rim * 0.95) * glow.selection.a);
        } else {
            // Outside: the line hugging the shape, then a glow that fades over `glow_px`.
            let near = ring(selection_mask, in.uv, glow.line_px).x;
            let mid = ring(selection_mask, in.uv, glow.glow_px * 0.55).x;
            let far = ring(selection_mask, in.uv, glow.glow_px).x;
            let alpha = max(near * 0.85, max(mid * 0.4, far * 0.18));
            colour = vec4<f32>(glow.selection.rgb, alpha * glow.selection.a);
        }
    }
    if colour.a < 0.01 && glow.has_hover > 0.5 {
        let centre = coverage(hover_mask, in.uv);
        if centre > 0.5 {
            colour = vec4<f32>(glow.hover.rgb, 0.06 * glow.hover.a);
        } else {
            let near = ring(hover_mask, in.uv, glow.line_px * 0.75).x;
            colour = vec4<f32>(glow.hover.rgb, near * 0.8 * glow.hover.a);
        }
    }
    if colour.a < 0.003 {
        discard;
    }
    return colour;
}
