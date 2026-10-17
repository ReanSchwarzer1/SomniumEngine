//!include "dream_lens.wgsl"

struct HighlightParams {
    color: vec4<f32>,
    inverse_output: vec2<f32>,
    width_pixels: f32,
    show_mask: f32,
    dream: vec4<f32>,
    frame: vec2<f32>,
    time: f32,
    _pad: f32,
    source_uv_offset: vec2<f32>,
    _pad2: vec2<f32>,
}

@group(0) @binding(0) var highlight_mask: texture_2d<f32>;
@group(0) @binding(1) var mask_sampler: sampler;
@group(0) @binding(2) var<uniform> highlight: HighlightParams;

struct FullscreenVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> FullscreenVertex {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return FullscreenVertex(vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0), vec2<f32>(uv.x, 1.0 - uv.y));
}

fn selected_at(uv: vec2<f32>) -> f32 {
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return 0.0;
    }
    let mapped = dream_coordinates(uv, highlight.dream, highlight.time, highlight.frame).uv;
    return textureSampleLevel(highlight_mask, mask_sampler, mapped + highlight.source_uv_offset, 0.0).r;
}

@fragment
fn fs_main(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let centre = selected_at(in.uv);
    if centre <= 0.001 {
        discard;
    }
    var inside = 1.0;
    let radius = highlight.inverse_output * highlight.width_pixels;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            if x != 0 || y != 0 {
                inside = min(inside, selected_at(in.uv + vec2<f32>(f32(x), f32(y)) * radius));
            }
        }
    }
    // An inner border cannot spill onto a foreground occluder. All parts share one mask.
    let edge = select(centre * (1.0 - inside), centre, highlight.show_mask > 0.5);
    return vec4<f32>(highlight.color.rgb, highlight.color.a * edge);
}
