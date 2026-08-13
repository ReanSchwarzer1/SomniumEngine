// Somnium Engine — Stochastic hex-tiling (Phase 25F).
//
// The loudest remaining tell that terrain is rendered rather than photographed
// is *repetition*: one tiled albedo at a fixed rate produces a grid the eye
// locks onto immediately, and no amount of lighting work hides it.
//
// Heitz & Neyret's answer, by way of `example_repo/bgfx-master/examples/
// 49-hextile/fs_hextile.sc` (which is itself ported from Mikkelsen's
// hextile-demo): partition UV space into a triangular grid, give each grid
// vertex a hashed offset and rotation, sample the texture three times at those
// offsets, and blend by barycentric weight. The lattice is destroyed without a
// second texture and without a visible seam.
//
// Three details from the reference are load-bearing and easy to drop:
//
// 1. **`textureSampleGrad`, with the derivatives rotated per tap.** Each tap
//    reads a *different* place in the texture, so implicit derivatives would be
//    computed across a discontinuity and collapse the mip selection to noise.
// 2. **Weights raised to a high power** before normalising, so the blend region
//    is narrow. Linear barycentric blending shows all three samples at once
//    over most of the surface, which reads as a wash rather than as detail.
// 3. **Luminance modulation** (`Dw`), which biases the blend toward the darker
//    sample. Blending two versions of the same texture linearly raises the mean
//    and flattens contrast; this is the cheap stand-in for the histogram-
//    preserving blend, which needs a precomputed histogram texture.
//
// `ProduceHexWeights` from the reference is deliberately not ported — it exists
// only to colour the debug weights view.

const HEX_PI: f32 = 3.14159265358979;
/// Blend sharpness. The reference's `g_exp` is 7; 5 here.
///
/// A narrower blend hides the cross-fade but makes each tile boundary a harder
/// step, and with an already-seamless procedural layer there is no wash to
/// avoid — the cross-fade is the cheaper artefact of the two.
const HEX_WEIGHT_EXP: f32 = 5.0;
/// How far the luminance term is allowed to bias the blend. The reference uses
/// 0.6; 0.25 here.
///
/// The bias toward the darker tap is a stand-in for histogram-preserving
/// blending, and it assumes a texture with real tonal range. Against a
/// low-contrast procedural layer it mostly shifts each tile's *mean*, which
/// reads as soft dark patches following the lattice — visible in the first
/// render as exactly that.
const HEX_LUMA_FALLOFF: f32 = 0.25;
/// Contrast applied to the final weights by `hex_gain3`. Above 0.5 sharpens,
/// 0.5 is a no-op. The reference uses 0.7; 0.6 here, for the same reason the
/// exponent came down.
const HEX_GAIN: f32 = 0.6;
/// How much each tap is rotated.
///
/// **Zero, which is the reference's own default** (`hextile.cpp`,
/// `m_tileRotationStrength = 0.0f`) — its slider goes to 20 and starts at 0.
/// Rotation is the part of the technique that looks like it should help most
/// and does the most damage: it makes neighbouring taps disagree about
/// orientation as well as position, and with weights this sharp that
/// disagreement lands on a hard edge. Rendered at 1.0 the simplex lattice was
/// plainly visible as triangular seams — worse than the repetition it was
/// meant to hide.
///
/// The hashed *offset* is what actually breaks the lattice. Rotation is left
/// wired up, and `hex_sample_normal` still counter-rotates correctly, so raising
/// this for a strongly directional texture stays an option.
const HEX_ROT_STRENGTH: f32 = 0.0;

/// Rotate by the tap's angle. `rot` is `(cos, sin)`.
///
/// Matches the reference's row-vector `mul(v, M)` convention, so the UV offset
/// and its derivatives are transformed identically — which is the whole point
/// of carrying the rotation around rather than baking it into the offset.
fn hex_rotate(v: vec2<f32>, rot: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(v.x * rot.x + v.y * rot.y, -v.x * rot.y + v.y * rot.x);
}

/// The inverse of [`hex_rotate`], for bringing a sampled tangent-space normal
/// back into the surface's own frame. See `hex_sample_normal`.
fn hex_unrotate(v: vec2<f32>, rot: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(v.x * rot.x - v.y * rot.y, v.x * rot.y + v.y * rot.x);
}

/// Deterministic per-vertex rotation, as `(cos, sin)`.
fn hex_rot_for(idx: vec2<f32>) -> vec2<f32> {
    var angle = abs(idx.x * idx.y) + abs(idx.x + idx.y) + HEX_PI;
    if angle < 0.0 { angle += 2.0 * HEX_PI; }
    if angle > HEX_PI { angle -= 2.0 * HEX_PI; }
    angle *= HEX_ROT_STRENGTH;
    return vec2<f32>(cos(angle), sin(angle));
}

/// Centre of the tile owned by a grid vertex, in unskewed UV space.
fn hex_centre(vertex: vec2<f32>) -> vec2<f32> {
    // Inverse of the skew below: rows (1, 0.5) and (0, 1/1.15470054).
    let unskewed = vec2<f32>(vertex.x + 0.5 * vertex.y, vertex.y / 1.15470054);
    return unskewed / (2.0 * sqrt(3.0));
}

fn hex_hash(p: vec2<f32>) -> vec2<f32> {
    let r = vec2<f32>(
        dot(p, vec2<f32>(127.1, 311.7)),
        dot(p, vec2<f32>(269.5, 183.3)),
    );
    return fract(sin(r) * 43758.5453);
}

/// Contrast on the barycentric weights: sharpens above 0.5, softens below.
fn hex_gain3(x: vec3<f32>, r: f32) -> vec3<f32> {
    let k = log(1.0 - r) / log(0.5);
    let s = 2.0 * step(vec3<f32>(0.5), x);
    let m = 2.0 * (1.0 - s);
    let res = 0.5 * s + 0.25 * m * pow(max(vec3<f32>(0.0), s + x * m), vec3<f32>(k));
    return res / (res.x + res.y + res.z);
}

/// The three grid vertices covering `uv` and their barycentric weights.
struct HexGrid {
    w: vec3<f32>,
    v1: vec2<f32>,
    v2: vec2<f32>,
    v3: vec2<f32>,
}

fn hex_grid(uv_in: vec2<f32>) -> HexGrid {
    // Scales the input against the tile size.
    let uv = uv_in * (2.0 * sqrt(3.0));

    // Skew into a simplex triangle grid: rows (1, -0.57735027) and
    // (0, 1.15470054).
    let skewed = vec2<f32>(uv.x - 0.57735027 * uv.y, 1.15470054 * uv.y);

    let base_id = floor(skewed);
    let f = fract(skewed);
    let third = 1.0 - f.x - f.y;

    // Which of the two triangles in the skewed cell this point falls in.
    let s = step(0.0, -third);
    let s2 = 2.0 * s - 1.0;

    var g: HexGrid;
    g.w = vec3<f32>(-third * s2, s - f.y * s2, s - f.x * s2);
    g.v1 = base_id + vec2<f32>(s, s);
    g.v2 = base_id + vec2<f32>(s, 1.0 - s);
    g.v3 = base_id + vec2<f32>(1.0 - s, s);
    return g;
}

/// One tap's UV, rotation and derivatives.
struct HexTap {
    uv: vec2<f32>,
    ddx: vec2<f32>,
    ddy: vec2<f32>,
    rot: vec2<f32>,
}

fn hex_tap(vertex: vec2<f32>, uv: vec2<f32>, ddx: vec2<f32>, ddy: vec2<f32>) -> HexTap {
    let rot = hex_rot_for(vertex);
    let cen = hex_centre(vertex);
    var t: HexTap;
    t.rot = rot;
    t.uv = hex_rotate(uv - cen, rot) + cen + hex_hash(vertex);
    t.ddx = hex_rotate(ddx, rot);
    t.ddy = hex_rotate(ddy, rot);
    return t;
}

/// Blend weights for three taps, given each tap's luminance.
fn hex_weights(w: vec3<f32>, luma: vec3<f32>) -> vec3<f32> {
    let dw = mix(vec3<f32>(1.0), luma, HEX_LUMA_FALLOFF);
    var weights = dw * pow(w, vec3<f32>(HEX_WEIGHT_EXP));
    weights = weights / max(weights.x + weights.y + weights.z, 1e-6);
    return hex_gain3(weights, HEX_GAIN);
}

const HEX_LUMA: vec3<f32> = vec3<f32>(0.299, 0.587, 0.114);

/// Hex-tiled colour sample. Drop-in for `textureSampleGrad` on a tiled texture.
///
/// Takes a **bindless index** rather than a `texture_2d<f32>`. Pulling a texture
/// out of the binding array into a local and passing it across a function
/// boundary is legal WGSL and segfaults naga's SPIR-V backend outright — the
/// process dies during pipeline creation with no diagnostic at all. Indexing
/// the array at the point of use is also what every other sampling site in this
/// engine does, so this stays consistent with them.
fn hex_sample(
    map: i32,
    uv: vec2<f32>,
    ddx: vec2<f32>,
    ddy: vec2<f32>,
) -> vec4<f32> {
    let g = hex_grid(uv);
    let t1 = hex_tap(g.v1, uv, ddx, ddy);
    let t2 = hex_tap(g.v2, uv, ddx, ddy);
    let t3 = hex_tap(g.v3, uv, ddx, ddy);

    let c1 = textureSampleGrad(textures[map], default_sampler, t1.uv, t1.ddx, t1.ddy);
    let c2 = textureSampleGrad(textures[map], default_sampler, t2.uv, t2.ddx, t2.ddy);
    let c3 = textureSampleGrad(textures[map], default_sampler, t3.uv, t3.ddx, t3.ddy);

    let w = hex_weights(
        g.w,
        vec3<f32>(dot(c1.rgb, HEX_LUMA), dot(c2.rgb, HEX_LUMA), dot(c3.rgb, HEX_LUMA)),
    );
    return w.x * c1 + w.y * c2 + w.z * c3;
}

/// Hex-tiled tangent-space normal, returned already decoded to `[-1, 1]`.
///
/// **Each tap's `xy` is counter-rotated before blending.** The reference tiles
/// colour only, so this case does not arise there — but a normal map stores its
/// vector in the texture's own UV frame, and each tap read that texture through
/// a different rotation. Blending the raw samples would average three normals
/// that disagree about which way "along U" points, which shows up as lighting
/// that swims as the camera moves and as a visible discontinuity at every tile
/// boundary — the exact artefact hex-tiling exists to remove, reintroduced one
/// level down.
fn hex_sample_normal(
    map: i32,
    uv: vec2<f32>,
    ddx: vec2<f32>,
    ddy: vec2<f32>,
) -> vec3<f32> {
    let g = hex_grid(uv);
    let t1 = hex_tap(g.v1, uv, ddx, ddy);
    let t2 = hex_tap(g.v2, uv, ddx, ddy);
    let t3 = hex_tap(g.v3, uv, ddx, ddy);

    let s1 = textureSampleGrad(textures[map], default_sampler, t1.uv, t1.ddx, t1.ddy).rgb * 2.0 - 1.0;
    let s2 = textureSampleGrad(textures[map], default_sampler, t2.uv, t2.ddx, t2.ddy).rgb * 2.0 - 1.0;
    let s3 = textureSampleGrad(textures[map], default_sampler, t3.uv, t3.ddx, t3.ddy).rgb * 2.0 - 1.0;

    let n1 = vec3<f32>(hex_unrotate(s1.xy, t1.rot), s1.z);
    let n2 = vec3<f32>(hex_unrotate(s2.xy, t2.rot), s2.z);
    let n3 = vec3<f32>(hex_unrotate(s3.xy, t3.rot), s3.z);

    // Weighted by the same rule as colour, using each tap's flatness as the
    // stand-in for luminance so a tap full of detail does not get washed out by
    // two flat ones.
    let w = hex_weights(g.w, vec3<f32>(n1.z, n2.z, n3.z));
    return w.x * n1 + w.y * n2 + w.z * n3;
}

/// Hex-tiled packed surface map (normal XY, roughness, AO).
///
/// The colour hex path cannot be reused here: RG is a tangent-space normal
/// that must be counter-rotated per tap, B is roughness, A is AO. Treating
/// the pack as RGB colour (or as an RGB XYZ normal) shears lighting at every
/// tile boundary.
struct HexPackedSurface {
    normal_ts: vec3<f32>,
    roughness: f32,
    occlusion: f32,
}

fn hex_sample_packed_surface(
    map: i32,
    uv: vec2<f32>,
    ddx: vec2<f32>,
    ddy: vec2<f32>,
) -> HexPackedSurface {
    let g = hex_grid(uv);
    let t1 = hex_tap(g.v1, uv, ddx, ddy);
    let t2 = hex_tap(g.v2, uv, ddx, ddy);
    let t3 = hex_tap(g.v3, uv, ddx, ddy);
    let s1 = textureSampleGrad(textures[map], default_sampler, t1.uv, t1.ddx, t1.ddy);
    let s2 = textureSampleGrad(textures[map], default_sampler, t2.uv, t2.ddx, t2.ddy);
    let s3 = textureSampleGrad(textures[map], default_sampler, t3.uv, t3.ddx, t3.ddy);
    let n1xy = hex_unrotate(s1.rg * 2.0 - 1.0, t1.rot);
    let n2xy = hex_unrotate(s2.rg * 2.0 - 1.0, t2.rot);
    let n3xy = hex_unrotate(s3.rg * 2.0 - 1.0, t3.rot);
    let n1 = vec3<f32>(n1xy, sqrt(max(1.0 - dot(n1xy, n1xy), 0.0)));
    let n2 = vec3<f32>(n2xy, sqrt(max(1.0 - dot(n2xy, n2xy), 0.0)));
    let n3 = vec3<f32>(n3xy, sqrt(max(1.0 - dot(n3xy, n3xy), 0.0)));
    let w = hex_weights(g.w, vec3<f32>(n1.z, n2.z, n3.z));
    var out: HexPackedSurface;
    out.normal_ts = normalize(w.x * n1 + w.y * n2 + w.z * n3);
    out.roughness = w.x * s1.b + w.y * s2.b + w.z * s3.b;
    out.occlusion = w.x * s1.a + w.y * s2.a + w.z * s3.a;
    return out;
}
