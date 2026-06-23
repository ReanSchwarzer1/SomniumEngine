// Phase 12B-1: Native UI render pass shader.
// Vertex: pos(f32x2) | uv(f32x2) | color(unorm8x4) — 20 bytes.
// Group 0 binding 0: ortho projection uniform (64 bytes).
// Group 1 binding 0/1: texture2d + sampler — white 1x1 for solid rects,
//   font atlas for glyph quads. Fragment multiplies vertex color by sample.

struct Ortho {
    proj: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> ortho: Ortho;

@group(1) @binding(0) var t_tex: texture_2d<f32>;
@group(1) @binding(1) var s_tex: sampler;

struct VertexInput {
    @location(0) pos:   vec2<f32>,
    @location(1) uv:    vec2<f32>,
    @location(2) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       uv:       vec2<f32>,
    @location(1)       color:    vec4<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_pos = ortho.proj * vec4<f32>(in.pos, 0.0, 1.0);
    out.uv       = in.uv;
    out.color    = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_sample = textureSample(t_tex, s_tex, in.uv);
    return in.color * tex_sample;
}
