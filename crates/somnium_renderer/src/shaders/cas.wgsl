// Somnium Engine — Contrast Adaptive Sharpening (Phase 24AC).
//
// TAA resolves a jittered history into a stable image, and the price is
// softness: every frame is a weighted average of several sub-pixel offsets, so
// the highest frequencies the renderer produced are exactly the ones it loses.
// A plain unsharp mask gives them back and gives back the noise with them,
// haloing every high-contrast edge in the process.
//
// CAS is the answer AMD published for that: sharpen by an amount **derived
// per pixel from the local contrast**. Where the neighbourhood already spans
// most of the available range there is nothing safe to add, so it adds nothing;
// where the neighbourhood is flat there is headroom, so it sharpens hard. The
// result is detail returned without ringing, which a fixed-strength filter
// cannot do at any setting.
//
// # The filter
//
//   0 A 0        `A` is negative — a ring of negative lobes around a centre of
//   A 1 A        1.0, divided by the sum of the weights. `A` per pixel is
//   0 A 0        `sqrt(headroom) * peak`, and `peak` runs from -1/8 (gentle) to
//                -1/5 (maximum ringing) across the sharpness knob.
//
// # Reference
//
// `SpartanEngine-master/data/shaders/amd_fidelity_fx/` — `cas.hlsl` and the
// `CasFilter` no-scaling path in `ffx_cas.h` (AMD FidelityFX CAS, MIT).
// Spartan compiles it with neither `CAS_BETTER_DIAGONALS` nor `CAS_SLOW`, so
// this follows the same configuration: a cross-shaped soft min/max and the
// green channel's weight applied to all three, which is AMD's own default and
// what the header's dead-code comment is written around.
//
// Deliberately **not** ported: `APrxLoRcpF1` / `APrxLoSqrtF1`, AMD's bit-trick
// approximations of reciprocal and square root. They exist to save cycles on
// hardware where those are slow; this is one full-screen pass at the end of the
// frame, and the exact forms are what `CAS_GO_SLOWER` selects anyway.

struct CasParams {
    /// 0 = gentle (least ringing), 1 = maximum.
    sharpness: f32,
    /// Blend against the unsharpened image. 1 is full CAS.
    strength: f32,
    _pad0: f32,
    _pad1: f32,
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> cas: CasParams;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
};

/// Fullscreen triangle, same as the other post passes.
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vi & 2u) * 2.0 - 1.0;
    out.clip_pos = vec4<f32>(x, -y, 0.0, 1.0);
    return out;
}

/// Clamped fetch. The 3×3 neighbourhood runs off the edge of the image on the
/// border pixels, and a wrapped read there would sharpen against the opposite
/// side of the screen.
fn cas_load(coord: vec2<i32>, dims: vec2<i32>) -> vec3<f32> {
    let c = clamp(coord, vec2<i32>(0, 0), dims - vec2<i32>(1, 1));
    return textureLoad(src, c, 0).rgb;
}

fn min3v(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>) -> vec3<f32> {
    return min(a, min(b, c));
}

fn max3v(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>) -> vec3<f32> {
    return max(a, max(b, c));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(src));
    let sp = vec2<i32>(in.clip_pos.xy);

    //  a b c
    //  d e f
    //  g h i
    let a = cas_load(sp + vec2<i32>(-1, -1), dims);
    let b = cas_load(sp + vec2<i32>( 0, -1), dims);
    let c = cas_load(sp + vec2<i32>( 1, -1), dims);
    let d = cas_load(sp + vec2<i32>(-1,  0), dims);
    let e = cas_load(sp, dims);
    let f = cas_load(sp + vec2<i32>( 1,  0), dims);
    let g = cas_load(sp + vec2<i32>(-1,  1), dims);
    let h = cas_load(sp + vec2<i32>( 0,  1), dims);
    let i = cas_load(sp + vec2<i32>( 1,  1), dims);

    // Soft min and max over the cross, not the full 3×3. The diagonals are
    // loaded because `CAS_BETTER_DIAGONALS` wants them and because leaving them
    // out would make this a different filter to compare against; AMD's default
    // path measures contrast on the cross alone.
    let mn = min3v(min3v(d, e, f), b, h);
    let mx = max3v(max3v(d, e, f), b, h);

    // How much room is left before sharpening would clip. `mn` is the distance
    // to black, `1 - mx` the distance to white; the nearer limit is the one that
    // binds. Divided by `mx` so it is a *relative* headroom — a dark region and
    // a bright one with the same contrast ratio get the same treatment, which
    // is what stops CAS from over-sharpening shadows.
    //
    // `max(mx, ...)` guards a fully black neighbourhood: AMD's approximate
    // reciprocal returns a huge finite number there, but an exact `1/0` is
    // infinity and `0 * inf` is NaN — one NaN pixel then spreads through every
    // later filter that touches it.
    let headroom = saturate(min(mn, vec3<f32>(1.0) - mx) / max(mx, vec3<f32>(1.0e-5)));

    // Shaped by a square root: the raw ratio falls away too fast, leaving
    // mid-contrast detail — which is most of an image — barely sharpened.
    let amp = sqrt(headroom);

    // Peak lobe weight, negative. -1/8 at sharpness 0, -1/5 at 1.
    let peak = -1.0 / mix(8.0, 5.0, saturate(cas.sharpness));
    // Green's weight for all three channels. AMD's default: the eye's contrast
    // response is dominated by green, and per-channel weights let the ring
    // shift the hue of an edge rather than only its contrast.
    let w = amp.g * peak;

    let rcp_weight = 1.0 / (1.0 + 4.0 * w);
    let sharpened = saturate((b * w + d * w + f * w + h * w + e) * rcp_weight);

    // `strength` is Somnium's, not AMD's: it fades the whole effect for A/B and
    // for a taste dial that does not change the ringing characteristics the way
    // `sharpness` does.
    return vec4<f32>(mix(e, sharpened, saturate(cas.strength)), 1.0);
}
