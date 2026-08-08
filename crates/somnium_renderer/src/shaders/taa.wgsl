// Phase 24F: temporal anti-aliasing.
//
// The projection is jittered by a sub-pixel offset each frame, so successive
// frames sample the scene at slightly different positions. This pass folds them
// together: reproject where each pixel was last frame, fetch that history, and
// blend. Over a handful of frames the accumulated samples resolve edges no
// single-sample raster can.
//
// This matters far more here than a generic AA pass would suggest. The visibility
// buffer has no MSAA available at all, and thin foliage with a bright specular
// lobe was sparkling badly enough to be the most visible defect in the renderer.
// It is also a hard prerequisite for everything in 24H and 24I: those techniques
// are stochastic, and without temporal accumulation their noise is unreadable.
//
// Reprojection is depth-based rather than velocity-based. See `REPROJECTION`.

struct TaaParams {
    /// Current-frame inverse view-projection, for reconstructing world position.
    inv_view_proj: mat4x4<f32>,
    /// Previous frame's view-projection, for finding where that point was.
    prev_view_proj: mat4x4<f32>,
    /// Reciprocal render target size.
    inv_resolution: vec2<f32>,
    /// Fraction of history retained when nothing invalidates it.
    blend_factor: f32,
    /// Zero on the first frame after a reset, so history is ignored.
    history_valid: f32,
    /// Debug visualisation selector; 0 = off. See `SOMNIUM_TAA_DEBUG`.
    debug_mode: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var current_tex:  texture_2d<f32>;
@group(0) @binding(1) var history_tex:  texture_2d<f32>;
@group(0) @binding(2) var depth_tex:    texture_depth_2d;
@group(0) @binding(3) var linear_samp:  sampler;
@group(0) @binding(4) var<uniform> taa: TaaParams;
/// Auto-exposure result; `[0]` is the linear multiplier (Phase 24A-3).
@group(0) @binding(5) var<storage, read> metered: array<f32, 2>;

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

/// Largest value allowed into the blend.
///
/// The HDR target is Rgba16Float, whose finite range ends at 65 504.
const HDR_CEILING: f32 = 60000.0;

/// Strip non-finite values before they reach the blend.
///
/// This is what produced the black speckling across shiny surfaces. With a
/// 100 000 lux sun, the GGX highlight on a near-mirror surface — the helmet's
/// visor, wet bark — exceeds what Rgba16Float can hold, so the shading pass
/// stores `Inf`. Then `tonemap_for_blend(Inf)` is `Inf / (1 + Inf)`, which is
/// **NaN**, and NaN propagates through min, max and clamp unpredictably, lands
/// in the history buffer, and persists there because history feeds itself.
///
/// It appeared only with TAA on because without it nothing ever divides by
/// infinity — the tone mapper simply clamps a huge value to white.
///
/// `c == c` is false only for NaN, which detects it without needing `isnan`.
fn sanitize(c: vec3<f32>) -> vec3<f32> {
    let finite = select(vec3<f32>(0.0), c, c == c);
    return clamp(finite, vec3<f32>(0.0), vec3<f32>(HDR_CEILING));
}

/// Tone-map into a bounded range before blending, and back out after.
///
/// Averaging HDR values directly lets a single very bright sample dominate the
/// mean, so a specular glint flickers instead of resolving — exactly the
/// foliage sparkle this pass exists to remove. Blending in a compressed space
/// weights samples by how they will actually appear (Karis).
///
/// **Exposure is applied first, and this is not optional.** The curve assumes
/// roughly exposure-normalised input, but the HDR target holds *pre-exposure*
/// luminance in cd/m² — thousands outdoors. Feeding it raw, 5 000 maps to
/// 0.9998 and 6 000 to 0.99983, so an entire neighbourhood collapses into
/// ~1e-4 of range at the very top of the curve. The clip box becomes
/// degenerate, history is judged out of bounds on essentially every pixel, and
/// the inverse `c / (1 - c)` is so ill-conditioned there that a 1e-4 correction
/// changes the result by a factor of two. That is what produced the black
/// patches, and a debug view of the clip flags showed it firing on 100% of the
/// frame rather than only at silhouettes.
fn blend_exposure() -> f32 {
    return max(metered[0], 1e-8);
}

fn tonemap_for_blend(c_in: vec3<f32>) -> vec3<f32> {
    let c = c_in * blend_exposure();
    return c / (1.0 + max(max(c.r, c.g), c.b));
}

fn untonemap_for_blend(c: vec3<f32>) -> vec3<f32> {
    let expanded = c / max(1.0 - max(max(c.r, c.g), c.b), 1e-4);
    return expanded / blend_exposure();
}

/// Clip `history` to the colour range of the current 3x3 neighbourhood
/// (Playdead's temporal reprojection AA).
///
/// Clipping toward the neighbourhood centre rather than clamping per channel
/// keeps the hue of the history intact; clamping each channel independently
/// shifts colours as it corrects them. This is what removes ghosting: history
/// that no longer resembles its surroundings is pulled back to something that
/// does, instead of smearing across the frame.
fn clip_to_neighbourhood(history: vec3<f32>, minimum: vec3<f32>, maximum: vec3<f32>) -> vec3<f32> {
    let centre = 0.5 * (maximum + minimum);
    let extent = 0.5 * (maximum - minimum) + 1e-5;
    let offset = history - centre;
    let units  = abs(offset / extent);
    let largest = max(max(units.x, units.y), units.z);
    if largest > 1.0 {
        return centre + offset / largest;
    }
    return history;
}

/// Catmull-Rom history sampling, 9 taps (MJP's optimisation of the 16-tap form).
///
/// Bilinear resampling of the history every frame compounds: each blend blurs
/// it slightly, and after a hundred frames the image is visibly soft. A
/// higher-order filter keeps it sharp for the cost of a few extra taps.
fn sample_history_catmull_rom(uv: vec2<f32>, resolution: vec2<f32>) -> vec3<f32> {
    let sample_pos = uv * resolution;
    let tex_pos1 = floor(sample_pos - 0.5) + 0.5;
    let f = sample_pos - tex_pos1;

    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);

    // Fold the two middle taps into one bilinear fetch.
    let w12 = w1 + w2;
    let offset12 = w2 / max(w12, vec2<f32>(1e-5));

    let tex_pos0 = (tex_pos1 - 1.0) / resolution;
    let tex_pos3 = (tex_pos1 + 2.0) / resolution;
    let tex_pos12 = (tex_pos1 + offset12) / resolution;

    var result = vec3<f32>(0.0);
    result += textureSampleLevel(history_tex, linear_samp,
        vec2<f32>(tex_pos0.x, tex_pos0.y), 0.0).rgb * w0.x * w0.y;
    result += textureSampleLevel(history_tex, linear_samp,
        vec2<f32>(tex_pos12.x, tex_pos0.y), 0.0).rgb * w12.x * w0.y;
    result += textureSampleLevel(history_tex, linear_samp,
        vec2<f32>(tex_pos3.x, tex_pos0.y), 0.0).rgb * w3.x * w0.y;

    result += textureSampleLevel(history_tex, linear_samp,
        vec2<f32>(tex_pos0.x, tex_pos12.y), 0.0).rgb * w0.x * w12.y;
    result += textureSampleLevel(history_tex, linear_samp,
        vec2<f32>(tex_pos12.x, tex_pos12.y), 0.0).rgb * w12.x * w12.y;
    result += textureSampleLevel(history_tex, linear_samp,
        vec2<f32>(tex_pos3.x, tex_pos12.y), 0.0).rgb * w3.x * w12.y;

    result += textureSampleLevel(history_tex, linear_samp,
        vec2<f32>(tex_pos0.x, tex_pos3.y), 0.0).rgb * w0.x * w3.y;
    result += textureSampleLevel(history_tex, linear_samp,
        vec2<f32>(tex_pos12.x, tex_pos3.y), 0.0).rgb * w12.x * w3.y;
    result += textureSampleLevel(history_tex, linear_samp,
        vec2<f32>(tex_pos3.x, tex_pos3.y), 0.0).rgb * w3.x * w3.y;

    // Clamping at zero bounds the undershoot but does not remove it — a tap
    // set that sums below zero still lands at black rather than at the colour
    // it should have. The caller clamps to the current neighbourhood, which is
    // what actually contains it.
    return max(result, vec3<f32>(0.0));
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let resolution = 1.0 / taa.inv_resolution;
    let coord = vec2<i32>(in.uv * resolution);
    let current = sanitize(textureLoad(current_tex, coord, 0).rgb);

    // ── REPROJECTION ────────────────────────────────────────────────────────
    // Depth-based: reconstruct this pixel's world position, then project it
    // with the previous frame's matrices to find where it used to be.
    //
    // This handles camera motion exactly, which is what the editor viewport
    // spends its time doing. It does *not* handle objects that moved while the
    // camera stood still — that needs a velocity buffer written from previous
    // per-instance transforms, which the visibility pass does not yet produce.
    // Moving geometry will ghost until that lands; the neighbourhood clip below
    // limits how badly.
    // Closest-depth dilation. Reprojecting a pixel using its *own* depth is
    // wrong at a silhouette: an edge pixel often carries the background's
    // depth, so it reprojects to where the background was and fetches history
    // that belongs to something else. That shows up as a dark rim tracing every
    // object — which is exactly what it did here, most visibly on tree trunks
    // against bright ground.
    //
    // Taking the nearest depth in a 3x3 neighbourhood makes edge pixels follow
    // the foreground instead. Spartan does the same thing in
    // `get_closest_pixel_velocity_3x3`, for the same reason.
    var depth = 1.0;
    var closest = coord;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let c = coord + vec2<i32>(x, y);
            let d = textureLoad(depth_tex, c, 0);
            // Smaller is nearer: this depth buffer has 0 at the near plane.
            if d < depth {
                depth = d;
                closest = c;
            }
        }
    }

    // Reconstruct from the pixel the depth actually came from, not from this
    // one — using one pixel's depth with another's screen position is what
    // produced the offset in the first place.
    let closest_uv = (vec2<f32>(closest) + 0.5) * taa.inv_resolution;
    let ndc = vec4<f32>(closest_uv.x * 2.0 - 1.0, 1.0 - closest_uv.y * 2.0, depth, 1.0);
    let world = taa.inv_view_proj * ndc;
    let world_pos = world.xyz / world.w;

    let prev_clip = taa.prev_view_proj * vec4<f32>(world_pos, 1.0);
    if prev_clip.w <= 0.0 {
        return vec4<f32>(current, 1.0);
    }
    let prev_ndc = prev_clip.xy / prev_clip.w;
    let prev_uv = vec2<f32>(prev_ndc.x * 0.5 + 0.5, 0.5 - prev_ndc.y * 0.5);

    // Off screen last frame: there is no history to reuse.
    if any(prev_uv < vec2<f32>(0.0)) || any(prev_uv > vec2<f32>(1.0))
        || taa.history_valid < 0.5 {
        return vec4<f32>(current, 1.0);
    }

    // ── Neighbourhood bounds ────────────────────────────────────────────────
    // Built in the compressed space the blend happens in, so the clip and the
    // blend agree about what "close" means.
    var minimum = vec3<f32>(1e9);
    var maximum = vec3<f32>(-1e9);
    var moment1 = vec3<f32>(0.0);
    var moment2 = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let c = tonemap_for_blend(
                sanitize(textureLoad(current_tex, coord + vec2<i32>(x, y), 0).rgb));
            minimum = min(minimum, c);
            maximum = max(maximum, c);
            moment1 += c;
            moment2 += c * c;
        }
    }

    // Variance clipping (Salvi): a box around the mean sized by the actual
    // spread is tighter than the raw min/max on noisy input, so it rejects
    // stale history that a plain bounding box would let through.
    let inv_samples = 1.0 / 9.0;
    let mean = moment1 * inv_samples;
    let sigma = sqrt(max(moment2 * inv_samples - mean * mean, vec3<f32>(0.0)));
    minimum = max(minimum, mean - sigma * 1.25);
    maximum = min(maximum, mean + sigma * 1.25);

    let history_raw = sanitize(sample_history_catmull_rom(prev_uv, resolution));
    // Clip first to preserve hue, then hard-clamp into the same box.
    //
    // The clip alone is not enough, and this is what produced a black outline
    // around every silhouette. `clip_to_neighbourhood` only moves history that
    // falls *outside* the box — but at a silhouette the box spans from the dark
    // object to the bright background, so it is wide, and a near-black value
    // sits comfortably inside it and passes through at 90% weight.
    //
    // Catmull-Rom is what manufactures those values: its outer taps carry
    // negative weights, so it undershoots at high-contrast edges. Clamping the
    // filter output at zero turned that undershoot into black instead of
    // preventing it. The clamp below bounds history by what the current frame
    // actually contains, so no filter can invent a colour that is not there.
    let history = clamp(
        clip_to_neighbourhood(tonemap_for_blend(history_raw), minimum, maximum),
        minimum,
        maximum,
    );

    let blended = mix(tonemap_for_blend(current), history, taa.blend_factor);
    let resolved = untonemap_for_blend(blended);

    // ── Instrumentation ─────────────────────────────────────────────────────
    // Three attempts at this bug were reasoned from plausible mechanisms and
    // all three were wrong. These modes show the intermediate values directly,
    // so the failing pixels can be read rather than guessed at.
    //
    // Values are written raw, and the caller disables tone mapping for these
    // modes, so what reaches the screen is the number rather than a graded
    // version of it.
    switch taa.debug_mode {
        // 1: history straight out of the Catmull-Rom filter, before any
        // clipping. If the black is already here, the fault is upstream in
        // reprojection or in what the history buffer holds.
        case 1u: { return vec4<f32>(history_raw, 1.0); }
        // 2: history after clip and clamp, back in linear space. Black here but
        // not in mode 1 means the clip is what darkens it.
        case 2u: { return vec4<f32>(untonemap_for_blend(history), 1.0); }
        // 3: this frame's own colour, for reference.
        case 3u: { return vec4<f32>(current, 1.0); }
        // 4/5: the neighbourhood bounds the clip works against. If `minimum` is
        // black on the failing pixels, the box legitimately permits black and
        // the clamp was never going to help.
        case 4u: { return vec4<f32>(untonemap_for_blend(minimum), 1.0); }
        case 5u: { return vec4<f32>(untonemap_for_blend(maximum), 1.0); }
        // 8: reprojection error, |prev_uv - uv| in pixels.
        //    With a still camera this MUST be zero everywhere. Anything else
        //    means history is fetched from a moving location every frame.
        //    green = under 0.02 px, red = above.
        case 8u: {
            let d = length((prev_uv - in.uv) * resolution);
            if d < 0.02 { return vec4<f32>(0.0, 4.0, 0.0, 1.0); }
            return vec4<f32>(4.0, 0.0, 0.0, 1.0);
        }
        // 6: what actually happened, as a flag image.
        //    red   = the clip moved history (it fell outside the box)
        //    green = the clamp moved it further
        //    blue  = history passed through untouched
        case 6u: {
            let clipped = clip_to_neighbourhood(
                tonemap_for_blend(history_raw), minimum, maximum);
            let moved_by_clip = distance(clipped, tonemap_for_blend(history_raw)) > 1e-4;
            let moved_by_clamp = distance(history, clipped) > 1e-4;
            return vec4<f32>(
                select(0.0, 1.0, moved_by_clip),
                select(0.0, 1.0, moved_by_clamp),
                select(1.0, 0.0, moved_by_clip || moved_by_clamp),
                1.0,
            );
        }
        // 7: how far history sits from this frame, amplified. Bright means the
        // two disagree strongly, which is where a bad blend shows up.
        case 7u: {
            return vec4<f32>(abs(untonemap_for_blend(history) - current) * 4.0, 1.0);
        }
        default: {}
    }

    return vec4<f32>(resolved, 1.0);
}
