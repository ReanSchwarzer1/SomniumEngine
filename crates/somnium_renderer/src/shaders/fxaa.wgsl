// Phase 15A2: FXAA (Fast Approximate Anti-Aliasing).
//
// Ported from Timothy Lottes' FXAA 3.11 "console" quality preset (NVIDIA).
// Single fullscreen pass over the tone-mapped LDR image: find high-contrast
// pixels, estimate the edge direction from luma differences across the 4
// diagonal neighbours, then blur along that edge.
//
// Runs BEFORE the gizmo / outline / UI passes so editor overlays and text stay
// pixel-sharp — FXAA on UI text looks smeared.
//
// Every tap uses `textureSampleLevel`, not `textureSample`, on purpose: the
// contrast early-out below is a data-dependent branch, which makes the control
// flow after it non-uniform. Implicit-derivative sampling is illegal there in
// WGSL; explicit-LOD sampling is not. The source has no mips anyway.

struct FxaaParams {
    /// Reciprocal of the render target size, in UV units per pixel.
    inv_size: vec2<f32>,
    /// Relative contrast needed to treat a pixel as an edge.
    edge_threshold: f32,
    /// Absolute darkness floor — below this, skip (avoids chewing on noise).
    edge_threshold_min: f32,
}

@group(0) @binding(0) var src_tex:  texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;
@group(0) @binding(2) var<uniform> fx: FxaaParams;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0)       uv:   vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VOut {
    var xs = array<f32, 3>(-1.0,  3.0, -1.0);
    var ys = array<f32, 3>(-1.0, -1.0,  3.0);
    let p  = vec2(xs[vid], ys[vid]);
    let uv = vec2((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    return VOut(vec4(p, 0.0, 1.0), uv);
}

/// Perceptual luminance weights (Rec. 601), as used by the original FXAA.
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.299, 0.587, 0.114));
}

fn tap(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(src_tex, src_samp, uv, 0.0).rgb;
}

// Direction-search tuning, from the FXAA 3.11 reference.
const SPAN_MAX: f32 = 8.0;
const REDUCE_MUL: f32 = 1.0 / 8.0;
const REDUCE_MIN: f32 = 1.0 / 128.0;

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let inv = fx.inv_size;

    let rgb_m = tap(in.uv);
    let l_m  = luma(rgb_m);
    let l_nw = luma(tap(in.uv + vec2(-1.0, -1.0) * inv));
    let l_ne = luma(tap(in.uv + vec2( 1.0, -1.0) * inv));
    let l_sw = luma(tap(in.uv + vec2(-1.0,  1.0) * inv));
    let l_se = luma(tap(in.uv + vec2( 1.0,  1.0) * inv));

    let l_min = min(l_m, min(min(l_nw, l_ne), min(l_sw, l_se)));
    let l_max = max(l_m, max(max(l_nw, l_ne), max(l_sw, l_se)));

    // Flat neighbourhood — leave it alone. This keeps FXAA from softening
    // smooth gradients, and skips most of the screen.
    if (l_max - l_min) < max(fx.edge_threshold_min, l_max * fx.edge_threshold) {
        return vec4(rgb_m, 1.0);
    }

    // Edge direction: perpendicular to the luma gradient across the diagonals.
    var dir = vec2(
        -((l_nw + l_ne) - (l_sw + l_se)),
         ((l_nw + l_sw) - (l_ne + l_se)),
    );

    // Bias the step length by local contrast so faint edges move less.
    let dir_reduce = max((l_nw + l_ne + l_sw + l_se) * 0.25 * REDUCE_MUL, REDUCE_MIN);
    let rcp_dir_min = 1.0 / (min(abs(dir.x), abs(dir.y)) + dir_reduce);
    dir = clamp(dir * rcp_dir_min, vec2(-SPAN_MAX), vec2(SPAN_MAX)) * inv;

    // Two-tap average (inner) and four-tap average (outer, wider reach).
    let rgb_a = 0.5 * (
        tap(in.uv + dir * (1.0 / 3.0 - 0.5)) +
        tap(in.uv + dir * (2.0 / 3.0 - 0.5))
    );
    let rgb_b = rgb_a * 0.5 + 0.25 * (
        tap(in.uv + dir * -0.5) +
        tap(in.uv + dir *  0.5)
    );

    // If the wider blur pushed luma outside the neighbourhood range it has
    // strayed onto an unrelated surface — fall back to the tighter average.
    let l_b = luma(rgb_b);
    if l_b < l_min || l_b > l_max {
        return vec4(rgb_a, 1.0);
    }
    return vec4(rgb_b, 1.0);
}
