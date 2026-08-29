// Weighted-blended OIT resolve (MORROWIND-AC).
//
// Reads the two targets `transparent.wgsl::fs_oit` accumulated into and blends
// the result over the HDR image with ordinary `SrcAlpha / OneMinusSrcAlpha`.
// See that file for the accumulation rules and the reference.

@group(0) @binding(0) var accum_tex: texture_2d<f32>;
@group(0) @binding(1) var reveal_tex: texture_2d<f32>;

struct VOut {
    @builtin(position) clip_pos: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VOut {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var out: VOut;
    out.clip_pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(in.clip_pos.xy);
    let reveal = textureLoad(reveal_tex, texel, 0).r;

    // `reveal` is the product of `(1 - a)` over every fragment that touched
    // this pixel, so 1.0 means nothing did. Returning a fully transparent
    // fragment — rather than skipping with `discard` — keeps the blend state
    // uniform and costs the same.
    if reveal > 0.9999 {
        return vec4<f32>(0.0);
    }

    let accum = textureLoad(accum_tex, texel, 0);

    // The weights cancel: `accum.rgb` is the weighted sum of premultiplied
    // colour and `accum.a` the weighted sum of alpha, so the ratio is the
    // weighted *average* colour and the weight function drops out. That is the
    // whole trick — the weights decide who dominates, not what the colour is.
    //
    // A saturated `accum.a` would divide by infinity and produce NaN, which
    // reaches the screen as a black or white pixel that survives tone mapping.
    // The paper's guard is a clamp, and it is cheaper than a NaN sweep later.
    let denom = max(accum.a, 1.0e-5);
    var color = accum.rgb / denom;
    if any(color != color) {
        // NaN still reachable if `accum.rgb` itself overflowed. Falling back to
        // black is wrong, but it is visibly wrong in one pixel rather than
        // poisoning the bloom pyramid, which averages it across the frame.
        color = vec3<f32>(0.0);
    }

    // `1 - reveal` is the total coverage, which is exactly the alpha this
    // resolve should composite with.
    return vec4<f32>(color, 1.0 - reveal);
}
