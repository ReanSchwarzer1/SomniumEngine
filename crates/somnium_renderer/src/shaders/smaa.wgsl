// Somnium Engine — SMAA 1x (MORROWIND-AC)
//
// Morphological anti-aliasing after Jimenez, Echevarria, Sousa and Gutierrez,
// *SMAA: Enhanced Subpixel Morphological Antialiasing* (Eurographics 2012).
// Three fullscreen passes over the tone-mapped LDR image, in the slot FXAA
// occupies: detect edges, reconstruct how much each edge covers its pixel, then
// blend.
//
// # What this is, precisely
//
// The three-pass structure, the luma edge test with local contrast adaptation,
// and the along-edge search are the paper's. **The precomputed `AreaTex` and
// `SearchTex` lookup tables are not used.** `smaa_coverage` below solves the
// same quantity analytically from the reconstructed silhouette geometry, which
// is what those tables bake. Two consequences, stated rather than discovered
// later:
//
//   - No table is vendored, so nothing here inherits a third-party asset
//     licence. Somnium's rule is to implement from the literature, and a
//     generated data table is exactly the kind of thing that rule is about.
//   - The **diagonal pattern pass** and the **sharp-corner rounding** of full
//     SMAA are not implemented. Both are separate refinements driven by their
//     own tables. Near-45-degree edges are therefore handled by the orthogonal
//     path alone and are slightly softer than reference SMAA would leave them.
//
// This is why the Details label says "SMAA 1x" and this comment says what that
// does and does not include. S2x and 4x are absent for a structural reason, not
// an effort one: both resolve MSAA subsamples, and a visibility buffer stores
// one triangle per pixel with `sample_count: 1` everywhere.

struct SmaaParams {
    /// 1 / target size, in texels.
    inv_size: vec2<f32>,
    /// Relative luma contrast that marks an edge (`SmaaPreset::threshold`).
    threshold: f32,
    /// How far along an edge to search (`SmaaPreset::max_search_steps`).
    max_search_steps: f32,
}

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> params: SmaaParams;
// Only bound for the blend pass; a 1x1 dummy otherwise.
@group(0) @binding(3) var aux_tex: texture_2d<f32>;

struct VOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VOut {
    // Fullscreen triangle. Same construction as every other resolve pass here.
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var out: VOut;
    out.uv = vec2<f32>(x, y);
    out.clip_pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

fn luma(c: vec3<f32>) -> f32 {
    // Rec.709 on an already tone-mapped image, matching `fxaa.wgsl` so the two
    // modes call the same pixels edges.
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn luma_at(uv: vec2<f32>) -> f32 {
    return luma(textureSampleLevel(src_tex, src_sampler, uv, 0.0).rgb);
}

// ─────────────────────────────────────────────────────────────────────────────
// Pass 1 — edge detection
//
// `r` marks a vertical edge on this pixel's **left** side, `g` a horizontal
// edge on its **top** side. Each boundary is therefore owned by exactly one
// pixel, which is what keeps the search in pass 2 from double-counting.
// ─────────────────────────────────────────────────────────────────────────────

@fragment
fn fs_edges(in: VOut) -> @location(0) vec4<f32> {
    let d = params.inv_size;
    let l_c = luma_at(in.uv);
    let l_left = luma_at(in.uv + vec2<f32>(-d.x, 0.0));
    let l_top = luma_at(in.uv + vec2<f32>(0.0, -d.y));

    var delta = vec2<f32>(abs(l_c - l_left), abs(l_c - l_top));
    var edges = step(vec2<f32>(params.threshold), delta);
    if edges.x + edges.y == 0.0 {
        return vec4<f32>(0.0);
    }

    // Local contrast adaptation (paper §3.1). An edge that is much weaker than
    // a neighbouring one is texture detail inside a gradient, not a silhouette;
    // filtering it is what makes FXAA soften surfaces. The factor of two is the
    // paper's.
    let l_right = luma_at(in.uv + vec2<f32>(d.x, 0.0));
    let l_bottom = luma_at(in.uv + vec2<f32>(0.0, d.y));
    let l_left2 = luma_at(in.uv + vec2<f32>(-2.0 * d.x, 0.0));
    let l_top2 = luma_at(in.uv + vec2<f32>(0.0, -2.0 * d.y));

    let max_delta = max(
        max(delta, vec2<f32>(abs(l_c - l_right), abs(l_c - l_bottom))),
        vec2<f32>(abs(l_left - l_left2), abs(l_top - l_top2)),
    );
    let strongest = max(max_delta.x, max_delta.y);
    edges *= step(vec2<f32>(strongest * 0.5), delta);

    return vec4<f32>(edges, 0.0, 1.0);
}

// ─────────────────────────────────────────────────────────────────────────────
// Pass 2 — blend weights
//
// Output convention, chosen so the blend pass needs no gather:
//   x = fraction taken from the LEFT neighbour
//   y = fraction taken from the TOP neighbour
//   z = fraction taken from the RIGHT neighbour
//   w = fraction taken from the BOTTOM neighbour
//
// z and w are the mirrored results of the boundaries owned by the pixels at
// +1x and +1y, recomputed here rather than gathered in pass 3. That costs a
// second search and buys a blend pass that is one texture fetch and some
// arithmetic.
// ─────────────────────────────────────────────────────────────────────────────

fn edges_at(uv: vec2<f32>) -> vec2<f32> {
    return textureSampleLevel(src_tex, src_sampler, uv, 0.0).rg;
}

/// Coverage of the pixel on the positive side of an edge boundary.
///
/// `d1`/`d2` are the distances in pixels from this pixel to the two ends of the
/// edge run. `s1`/`s2` are which way the silhouette turns at each end: `+1` if
/// the crossing edge lies on the positive side, `-1` on the negative side, `0`
/// if the run was not terminated by a crossing within the search budget.
///
/// The reconstructed silhouette is the straight line whose offset from the
/// boundary is `±0.5` at a turning end and `0` where nothing turns. Its offset
/// at this pixel's centre is the fraction of this pixel that actually belongs
/// to the neighbour across the boundary — which is exactly the quantity
/// reference SMAA reads out of `AreaTex`.
///
/// Both ends turning the same way gives a constant `0.5`: a clean step, half
/// blended along its whole length. Opposite ways gives a linear ramp through
/// zero: a diagonal. One end turning gives a ramp to zero. Those three shapes
/// are the orthogonal pattern set.
fn smaa_coverage(d1: f32, d2: f32, s1: f32, s2: f32) -> f32 {
    let len = d1 + d2 + 1.0;
    let t = (d1 + 0.5) / len;
    let offset = mix(s1, s2, t) * 0.5;
    return clamp(offset, 0.0, 0.5);
}

/// Walk along a vertical edge from `uv`, stepping by `step_uv`, while the run
/// continues. Returns `(distance, turn)` where `turn` is the silhouette
/// direction at the end — see [`smaa_coverage`].
fn search_vertical(uv: vec2<f32>, step_uv: vec2<f32>) -> vec2<f32> {
    let d = params.inv_size;
    var p = uv;
    var dist = 0.0;
    loop {
        if dist >= params.max_search_steps {
            break;
        }
        p += step_uv;
        dist += 1.0;
        if edges_at(p).x < 0.5 {
            // The run ended one step back. The crossing that ended it is a
            // horizontal edge at the last pixel of the run, on one side or the
            // other of the boundary.
            let end = p - step_uv;
            let cross_pos = edges_at(end + vec2<f32>(0.0, 0.0)).y;
            let cross_neg = edges_at(end + vec2<f32>(-d.x, 0.0)).y;
            var turn = 0.0;
            if cross_pos > 0.5 {
                turn = 1.0;
            } else if cross_neg > 0.5 {
                turn = -1.0;
            }
            return vec2<f32>(dist - 1.0, turn);
        }
    }
    return vec2<f32>(dist, 0.0);
}

/// As [`search_vertical`], for a horizontal edge. `turn` is positive when the
/// crossing lies below the boundary.
fn search_horizontal(uv: vec2<f32>, step_uv: vec2<f32>) -> vec2<f32> {
    let d = params.inv_size;
    var p = uv;
    var dist = 0.0;
    loop {
        if dist >= params.max_search_steps {
            break;
        }
        p += step_uv;
        dist += 1.0;
        if edges_at(p).y < 0.5 {
            let end = p - step_uv;
            let cross_pos = edges_at(end + vec2<f32>(0.0, 0.0)).x;
            let cross_neg = edges_at(end + vec2<f32>(0.0, -d.y)).x;
            var turn = 0.0;
            if cross_pos > 0.5 {
                turn = 1.0;
            } else if cross_neg > 0.5 {
                turn = -1.0;
            }
            return vec2<f32>(dist - 1.0, turn);
        }
    }
    return vec2<f32>(dist, 0.0);
}

/// Weight for the vertical boundary owned by the pixel at `uv`.
fn vertical_weight(uv: vec2<f32>) -> f32 {
    if edges_at(uv).x < 0.5 {
        return 0.0;
    }
    let d = params.inv_size;
    let up = search_vertical(uv, vec2<f32>(0.0, -d.y));
    let down = search_vertical(uv, vec2<f32>(0.0, d.y));
    return smaa_coverage(up.x, down.x, up.y, down.y);
}

/// Weight for the horizontal boundary owned by the pixel at `uv`.
fn horizontal_weight(uv: vec2<f32>) -> f32 {
    if edges_at(uv).y < 0.5 {
        return 0.0;
    }
    let d = params.inv_size;
    let left = search_horizontal(uv, vec2<f32>(-d.x, 0.0));
    let right = search_horizontal(uv, vec2<f32>(d.x, 0.0));
    return smaa_coverage(left.x, right.x, left.y, right.y);
}

@fragment
fn fs_weights(in: VOut) -> @location(0) vec4<f32> {
    let d = params.inv_size;
    // This pixel owns its left and top boundaries; its right and bottom
    // boundaries are owned by the neighbours at +1x and +1y. Mirroring is the
    // sign flip inside `smaa_coverage`, so the neighbour's coverage is taken
    // from the other side of the same line.
    let from_left = vertical_weight(in.uv);
    let from_top = horizontal_weight(in.uv);
    let right_owner = in.uv + vec2<f32>(d.x, 0.0);
    let bottom_owner = in.uv + vec2<f32>(0.0, d.y);

    var from_right = 0.0;
    if edges_at(right_owner).x >= 0.5 {
        let up = search_vertical(right_owner, vec2<f32>(0.0, -d.y));
        let down = search_vertical(right_owner, vec2<f32>(0.0, d.y));
        // Negated turns: this pixel sits on the negative side of that boundary.
        from_right = smaa_coverage(up.x, down.x, -up.y, -down.y);
    }
    var from_bottom = 0.0;
    if edges_at(bottom_owner).y >= 0.5 {
        let left = search_horizontal(bottom_owner, vec2<f32>(-d.x, 0.0));
        let right = search_horizontal(bottom_owner, vec2<f32>(d.x, 0.0));
        from_bottom = smaa_coverage(left.x, right.x, -left.y, -right.y);
    }

    return vec4<f32>(from_left, from_top, from_right, from_bottom);
}

// ─────────────────────────────────────────────────────────────────────────────
// Pass 3 — neighbourhood blending
// ─────────────────────────────────────────────────────────────────────────────

@fragment
fn fs_blend(in: VOut) -> @location(0) vec4<f32> {
    // `aux_tex` is the weights target; `src_tex` is the colour image.
    let w = textureSampleLevel(aux_tex, src_sampler, in.uv, 0.0);
    let total = w.x + w.y + w.z + w.w;
    let centre = textureSampleLevel(src_tex, src_sampler, in.uv, 0.0);
    if total < 1.0e-5 {
        return centre;
    }
    let d = params.inv_size;
    let c_left = textureSampleLevel(src_tex, src_sampler, in.uv + vec2<f32>(-d.x, 0.0), 0.0);
    let c_top = textureSampleLevel(src_tex, src_sampler, in.uv + vec2<f32>(0.0, -d.y), 0.0);
    let c_right = textureSampleLevel(src_tex, src_sampler, in.uv + vec2<f32>(d.x, 0.0), 0.0);
    let c_bottom = textureSampleLevel(src_tex, src_sampler, in.uv + vec2<f32>(0.0, d.y), 0.0);

    // Normalised so the result stays in the convex hull of its taps: a pixel
    // with edges on all four sides must not brighten.
    let centre_w = max(1.0 - total, 0.0);
    let sum = centre_w + total;
    let blended =
        centre * centre_w + c_left * w.x + c_top * w.y + c_right * w.z + c_bottom * w.w;
    return blended / max(sum, 1.0e-5);
}
