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
}

@group(0) @binding(0) var current_tex:  texture_2d<f32>;
@group(0) @binding(1) var history_tex:  texture_2d<f32>;
@group(0) @binding(2) var depth_tex:    texture_depth_2d;
@group(0) @binding(3) var linear_samp:  sampler;
@group(0) @binding(4) var<uniform> taa: TaaParams;

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

/// Tone-map into a bounded range before blending, and back out after.
///
/// Averaging HDR values directly lets a single very bright sample dominate the
/// mean, so a specular glint flickers instead of resolving — exactly the
/// foliage sparkle this pass exists to remove. Blending in a compressed space
/// weights samples by how they will actually appear (Karis).
fn tonemap_for_blend(c: vec3<f32>) -> vec3<f32> {
    return c / (1.0 + max(max(c.r, c.g), c.b));
}

fn untonemap_for_blend(c: vec3<f32>) -> vec3<f32> {
    return c / max(1.0 - max(max(c.r, c.g), c.b), 1e-4);
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

    return max(result, vec3<f32>(0.0));
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let resolution = 1.0 / taa.inv_resolution;
    let coord = vec2<i32>(in.uv * resolution);
    let current = textureLoad(current_tex, coord, 0).rgb;

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
                textureLoad(current_tex, coord + vec2<i32>(x, y), 0).rgb);
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

    let history_raw = sample_history_catmull_rom(prev_uv, resolution);
    let history = clip_to_neighbourhood(
        tonemap_for_blend(history_raw), minimum, maximum);

    let blended = mix(tonemap_for_blend(current), history, taa.blend_factor);
    return vec4<f32>(untonemap_for_blend(blended), 1.0);
}
