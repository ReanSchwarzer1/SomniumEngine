// Phase 24T: bloom as a lens response.
//
// Deliberately **not** threshold-based. A threshold asks "which pixels count as
// bright?", which is a question with no physical answer and one that changes
// meaning the moment exposure changes — a scene metered for night would bloom
// everything, a scene metered for noon nothing. Real bloom is light scattering
// inside the lens and sensor, which happens to *all* light in proportion to how
// much there is.
//
// So: no threshold. A progressive downsample builds a mip chain, and a
// progressive upsample sums it back with a small weight. Bright regions
// dominate the result naturally because they contribute more energy, which is
// the same reason a real lens flares around a light and not around a wall.
// Follows the Call of Duty presentation (Jimenez, SIGGRAPH 2014).

struct BloomParams {
    /// Texel size of the *source* mip being read.
    src_texel: vec2<f32>,
    /// Radius of the upsample tent filter, in source texels.
    filter_radius: f32,
    /// How much of the blurred chain reaches the final image.
    intensity: f32,
}

@group(0) @binding(0) var src_tex:  texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;
@group(0) @binding(2) var<uniform> params: BloomParams;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0)       uv:   vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VOut {
    var xs = array<f32, 3>(-1.0,  3.0, -1.0);
    var ys = array<f32, 3>(-1.0, -1.0,  3.0);
    let p  = vec2<f32>(xs[vid], ys[vid]);
    return VOut(vec4<f32>(p, 0.0, 1.0), vec2<f32>((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5));
}

/// 13-tap downsample.
///
/// A plain bilinear halving aliases badly on the small, very bright features
/// bloom exists to spread — a specular glint occupying one texel would flicker
/// in and out as the camera moves, and the blur would broadcast that flicker
/// across the screen. The overlapping tap pattern is what keeps it stable.
@fragment
fn fs_downsample(in: VOut) -> @location(0) vec4<f32> {
    let t = params.src_texel;

    let a = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>(-2.0 * t.x,  2.0 * t.y), 0.0).rgb;
    let b = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>( 0.0,        2.0 * t.y), 0.0).rgb;
    let c = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>( 2.0 * t.x,  2.0 * t.y), 0.0).rgb;

    let d = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>(-2.0 * t.x,  0.0), 0.0).rgb;
    let e = textureSampleLevel(src_tex, src_samp, in.uv,                                0.0).rgb;
    let f = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>( 2.0 * t.x,  0.0), 0.0).rgb;

    let g = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>(-2.0 * t.x, -2.0 * t.y), 0.0).rgb;
    let h = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>( 0.0,       -2.0 * t.y), 0.0).rgb;
    let i = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>( 2.0 * t.x, -2.0 * t.y), 0.0).rgb;

    let j = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>(-t.x,  t.y), 0.0).rgb;
    let k = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>( t.x,  t.y), 0.0).rgb;
    let l = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>(-t.x, -t.y), 0.0).rgb;
    let m = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>( t.x, -t.y), 0.0).rgb;

    // Weights from the Call of Duty chain: the inner quad carries half the
    // total, the surrounding groups an eighth each.
    var result = e * 0.125;
    result += (a + c + g + i) * 0.03125;
    result += (b + d + f + h) * 0.0625;
    result += (j + k + l + m) * 0.125;

    // Guard the chain against non-finite input for the same reason TAA needs
    // it: one Inf in a blur spreads across everything it touches.
    let finite = select(vec3<f32>(0.0), result, result == result);
    return vec4<f32>(min(finite, vec3<f32>(60000.0)), 1.0);
}

/// 9-tap tent upsample, added onto the destination mip.
///
/// Blend is additive at the pipeline level, so each level accumulates onto the
/// one above it. Summing progressively rather than blurring once at full
/// resolution is what makes the falloff wide and smooth without a kernel the
/// size of the screen.
@fragment
fn fs_upsample(in: VOut) -> @location(0) vec4<f32> {
    let r = params.src_texel * params.filter_radius;

    let a = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>(-r.x,  r.y), 0.0).rgb;
    let b = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>( 0.0,  r.y), 0.0).rgb;
    let c = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>( r.x,  r.y), 0.0).rgb;
    let d = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>(-r.x,  0.0), 0.0).rgb;
    let e = textureSampleLevel(src_tex, src_samp, in.uv,                        0.0).rgb;
    let f = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>( r.x,  0.0), 0.0).rgb;
    let g = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>(-r.x, -r.y), 0.0).rgb;
    let h = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>( 0.0, -r.y), 0.0).rgb;
    let i = textureSampleLevel(src_tex, src_samp, in.uv + vec2<f32>( r.x, -r.y), 0.0).rgb;

    var result = e * 4.0;
    result += (b + d + f + h) * 2.0;
    result += (a + c + g + i);
    result *= 1.0 / 16.0;

    return vec4<f32>(result, 1.0);
}
