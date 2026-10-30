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

/// One ghost or the halo: the blurred chain read at `at`, its colour split a little along `toward` the way a lens's
/// glass splits it, faded toward the frame's edge.
fn flare_tap(at: vec2<f32>, toward: vec2<f32>, edge_power: f32) -> vec3<f32> {
    let fade = pow(saturate(1.0 - length(vec2<f32>(0.5) - at) / 0.7071), edge_power);
    let split = toward * 0.012;
    let rgb = vec3<f32>(
        textureSampleLevel(src_tex, src_samp, at + split, 0.0).r,
        textureSampleLevel(src_tex, src_samp, at, 0.0).g,
        textureSampleLevel(src_tex, src_samp, at - split, 0.0).b);
    return rgb * fade;
}

/// The lens's flare (17 Oct 2026; engine features record E3): ghosts of every bright thing mirrored through the frame's
/// centre and a halo ring round it, as light bounces between a real lens's elements. Like the bloom, no threshold:
/// the ghosts carry a small share of all light, so only a lamp, the moon or a torch shone at the eye shows one.
/// Added onto the chain's top level, so `filter_radius` carries the flare's strength already divided by the bloom's
/// intensity (the post pass multiplies the chain by that intensity again).
@fragment
fn fs_flare(in: VOut) -> @location(0) vec4<f32> {
    let uv = vec2<f32>(1.0) - in.uv;
    let toward = (vec2<f32>(0.5) - uv) * 0.34;
    var result = vec3<f32>(0.0);
    // Ghosts at falling sizes and strengths, each tinted as a coated element would tint it.
    let tints = array<vec3<f32>, 4>(
        vec3<f32>(1.0, 0.82, 0.62), vec3<f32>(0.55, 0.8, 1.0), vec3<f32>(0.9, 0.6, 1.0), vec3<f32>(0.7, 1.0, 0.75));
    for (var i = 0u; i < 4u; i++) {
        let at = uv + toward * f32(i + 1u);
        if all(at >= vec2<f32>(0.0)) && all(at <= vec2<f32>(1.0)) {
            result += flare_tap(at, toward, 8.0) * tints[i] * (0.8 / f32(i + 1u));
        }
    }
    // The halo: a ring at a fixed distance from the centre.
    let aspect = params.src_texel.y / max(params.src_texel.x, 1.0e-6);
    var dir = vec2<f32>(0.5) - uv;
    dir.x *= aspect;
    let len = max(length(dir), 1.0e-4);
    var halo_vec = dir / len * 0.42;
    halo_vec.x /= aspect;
    let halo_at = uv + halo_vec;
    // Read over a disc round that point, not at it: one tap drew a lamp's halo as a crisp thin ring a third of the
    // screen across, which read as overexposure (TSF, 23 Oct 2026). The disc spreads the same light into a broad,
    // soft band.
    var halo = vec3<f32>(0.0);
    for (var k = 0u; k < 8u; k++) {
        let a = f32(k) * 0.7853982;
        halo += flare_tap(halo_at + vec2<f32>(cos(a) / aspect, sin(a)) * 0.035, toward, 4.0);
    }
    result += halo * 0.125 * vec3<f32>(0.85, 0.9, 1.0) * 0.5;
    let finite = select(vec3<f32>(0.0), result, result == result);
    return vec4<f32>(min(finite * params.filter_radius, vec3<f32>(60000.0)), 1.0);
}
