// Phase 12B-1: Native UI render pass shader.
// Vertex: pos(f32x2) | uv(f32x2) | color(unorm8x4) — 20 bytes.
// Group 0 binding 0: ortho projection uniform (64 bytes).
// Group 1 binding 0/1: texture2d + sampler — white 1x1 for solid rects,
//   font/icon atlas for mask quads. Vertex tint is authored sRGB with straight
//   alpha. It is decoded once for an sRGB target, then straight-alpha blended.

// UiPass replaces this declaration with `false` only for a non-sRGB surface.
const OUTPUT_IS_SRGB: bool = true;

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
    var tint_rgb = in.color.rgb;
    if OUTPUT_IS_SRGB {
        let low = tint_rgb / vec3<f32>(12.92);
        let high = pow((tint_rgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
        tint_rgb = select(high, low, tint_rgb <= vec3<f32>(0.04045));
    }

    // Font and icon textures are linear white RGB + coverage alpha masks.
    // Future coloured textures must use an sRGB texture view so sampling
    // decodes them before this multiplication.
    return vec4<f32>(tint_rgb * tex_sample.rgb, in.color.a * tex_sample.a);
}
