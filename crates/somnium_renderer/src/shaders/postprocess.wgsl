// Phase 11.5K: tone-mapping + vignette post-process pass.
// Phase 24A/24B: physical exposure and a selectable tone-mapping curve.
//
// Reads an Rgba16Float HDR texture and writes tone-mapped LDR to the swapchain.
// Applied after the shading pass and grid overlay, before the UI overlay.

struct PostParams {
    /// Linear multiplier from scene luminance (cd/m²) to display range.
    /// Derived from EV100, not an arbitrary gain — see `light_units.rs`.
    exposure:          f32,
    vignette_strength: f32,
    /// Chromatic-aberration offset in UV units at the screen edge. 0 = off.
    ca_strength:       f32,
    /// 0 = AgX, 1 = ACES, 2 = Reinhard. Matches `Tonemapper::as_index`.
    tonemapper:        u32,
    /// Non-zero when `metered` should be used in place of `exposure`.
    auto_exposure:     u32,
    /// How much of the bloom chain reaches the image.
    bloom_intensity:   f32,
    _pad1:             u32,
    _pad2:             u32,
}

@group(0) @binding(0) var hdr_tex:  texture_2d<f32>;
@group(0) @binding(1) var hdr_samp: sampler;
@group(0) @binding(2) var<uniform> pp: PostParams;
/// `[0]` = metered exposure multiplier, `[1]` = adapted EV100 (Phase 24A-3).
@group(0) @binding(3) var<storage, read> metered: array<f32, 2>;
/// Blurred bloom chain (Phase 24T).
@group(0) @binding(4) var bloom_tex: texture_2d<f32>;

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

// ── AgX (Phase 24B) ─────────────────────────────────────────────────────────
//
// Troy Sobotka's AgX, as the analytic form rather than the 3-D LUT the
// reference implementation ships. A LUT would mean shipping and binding a KTX2
// asset for what is a closed-form curve; the polynomial below matches it to
// well within 8-bit output precision.
//
// Why replace ACES: ACES pushes very bright saturated colours toward the
// primaries, so an intense warm light skews orange and then clips to white
// rather than desaturating the way film does. That was tolerable when the sun
// was an arbitrary 3.0, and stops being tolerable at 100 000 lux with a sun
// disc in frame. AgX desaturates smoothly into the highlight instead, which is
// what keeps a sunrise looking photographed.

// Rec.709 → AgX working space. Columns, matching WGSL's mat3x3 constructor.
const AGX_INSET = mat3x3<f32>(
    vec3<f32>(0.8566271533, 0.1373189729, 0.1118982130),
    vec3<f32>(0.0951212405, 0.7612419906, 0.0767994186),
    vec3<f32>(0.0482516061, 0.1014390365, 0.8113023684),
);

// Inverse of the above, back to Rec.709.
const AGX_OUTSET = mat3x3<f32>(
    vec3<f32>( 1.1271005818, -0.1413297635, -0.1413297635),
    vec3<f32>(-0.1106066431,  1.1578237022, -0.1106066431),
    vec3<f32>(-0.0164939387, -0.0164939387,  1.2519364066),
);

/// Dynamic range AgX maps, in stops around middle grey.
const AGX_MIN_EV: f32 = -12.47393;
const AGX_MAX_EV: f32 = 4.026069;

/// Sixth-order fit to AgX's default contrast sigmoid.
fn agx_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2
         - 40.14 * x4 * x
         + 31.96 * x4
         - 6.868 * x2 * x
         + 0.4298 * x2
         + 0.1191 * x
         - 0.00232;
}

fn agx(color_in: vec3<f32>) -> vec3<f32> {
    var color = AGX_INSET * max(color_in, vec3(0.0));

    // Log-encode across AgX's range. The floor keeps log2 finite on black.
    color = clamp(log2(max(color, vec3(1e-10))), vec3(AGX_MIN_EV), vec3(AGX_MAX_EV));
    color = (color - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV);

    color = agx_contrast(color);
    color = AGX_OUTSET * color;

    // AgX emits display-encoded values, but this pass writes to an sRGB target
    // that applies its own encode. Undo AgX's so the two do not compound.
    return clamp(pow(max(color, vec3(0.0)), vec3(2.2)), vec3(0.0), vec3(1.0));
}

/// Plain Reinhard, kept as a neutral reference point for comparisons.
fn reinhard(x: vec3<f32>) -> vec3<f32> {
    return clamp(x / (vec3(1.0) + x), vec3(0.0), vec3(1.0));
}

fn tonemap(hdr: vec3<f32>) -> vec3<f32> {
    switch pp.tonemapper {
        case 1u: { return aces_film(hdr); }
        case 2u: { return reinhard(hdr); }
        // 3: passthrough, for debug views. A curve would grade the very values
        // being inspected, and exposure would crush a 0/1 flag image to black.
        case 3u: { return clamp(hdr, vec3(0.0), vec3(1.0)); }
        default: { return agx(hdr); }
    }
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    // Chromatic aberration: split the RGB channels along the vector from the
    // screen centre, so fringing grows toward the edges like a real lens.
    // Written branch-free — at ca_strength = 0 all three taps land on the same
    // texel, which is exactly the un-aberrated result. (A branch around
    // textureSample would risk WGSL's uniform-control-flow rules.)
    let ca_dir = in.uv - vec2(0.5);
    let ca_off = ca_dir * pp.ca_strength;
    var hdr = vec3(
        textureSample(hdr_tex, hdr_samp, in.uv - ca_off).r,
        textureSample(hdr_tex, hdr_samp, in.uv).g,
        textureSample(hdr_tex, hdr_samp, in.uv + ca_off).b,
    );

    // Phase 24T: bloom added *before* exposure and tone mapping, because it is
    // scattering inside the lens — it happens to the light on its way to the
    // sensor, not to the picture afterwards.
    let bloom = textureSample(bloom_tex, hdr_samp, in.uv).rgb;
    hdr += bloom * pp.bloom_intensity;

    // Exposure. Physical now, not an arbitrary gain: either the value the
    // manual camera settings imply, or what the histogram metered this frame.
    let exposure = select(pp.exposure, metered[0], pp.auto_exposure != 0u);
    hdr *= exposure;

    // Tone map → LDR in [0, 1].
    let ldr = tonemap(hdr);

    // Radial vignette: darkens screen edges.
    let uv_c = in.uv - vec2(0.5);
    let vign  = 1.0 - smoothstep(0.35, 0.75, length(uv_c) * 1.4 * pp.vignette_strength);

    return vec4(ldr * vign, 1.0);
}
