// Bilinear blit from the internal 3D target onto the swapchain.
//
// Scene passes can run at 1080p while the window (and UI) stay at the
// display's size. A filtered sample is the whole pass — CAS already ran on
// the internal image and is not an upscaler.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var out: VertexOutput;
    var xs = array<f32, 3>(-1.0, 3.0, -1.0);
    var ys = array<f32, 3>(-1.0, -1.0, 3.0);
    let p = vec2<f32>(xs[vi], ys[vi]);
    out.clip_pos = vec4<f32>(p, 0.0, 1.0);
    // UV origin is top-left, same as post-process and FXAA.
    out.uv = vec2<f32>((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(src, src_sampler, in.uv);
}
