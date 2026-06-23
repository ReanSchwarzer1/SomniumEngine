// Phase 11.5K: ACES tone-mapping + vignette post-process pass.
//
// Reads an Rgba16Float HDR texture and writes tone-mapped LDR to the swapchain.
// Applied after the shading pass and grid overlay, before the UI overlay.

struct PostParams {
    exposure:         f32,
    vignette_strength: f32,
    _pad0:            f32,
    _pad1:            f32,
}

@group(0) @binding(0) var hdr_tex:  texture_2d<f32>;
@group(0) @binding(1) var hdr_samp: sampler;
@group(0) @binding(2) var<uniform> pp: PostParams;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0)       uv:   vec2<f32>,
}

// Full-screen triangle: NDC (-1,-1), (3,-1), (-1,3).
// UV origin is top-left → UV.y = (1 - NDC.y) / 2.
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VOut {
    var xs = array<f32, 3>(-1.0,  3.0, -1.0);
    var ys = array<f32, 3>(-1.0, -1.0,  3.0);
    let p  = vec2(xs[vid], ys[vid]);
    let uv = vec2((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    return VOut(vec4(p, 0.0, 1.0), uv);
}

// ACES filmic tone-mapping approximation (Narkowicz 2015).
fn aces_film(x: vec3<f32>) -> vec3<f32> {
    return clamp(
        (x * (2.51 * x + vec3(0.03))) / (x * (2.43 * x + vec3(0.59)) + vec3(0.14)),
        vec3(0.0), vec3(1.0)
    );
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    var hdr = textureSample(hdr_tex, hdr_samp, in.uv).rgb;

    // Exposure adjustment.
    hdr *= pp.exposure;

    // ACES tone mapping → LDR in [0, 1].
    let ldr = aces_film(hdr);

    // Radial vignette: darkens screen edges.
    let uv_c = in.uv - vec2(0.5);
    let vign  = 1.0 - smoothstep(0.35, 0.75, length(uv_c) * 1.4 * pp.vignette_strength);

    return vec4(ldr * vign, 1.0);
}
