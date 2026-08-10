// Somnium Engine — motion blur (Phase 24Z, the half that waited on 24AD).
//
// A real camera's shutter is open for a finite slice of each frame, and
// anything that moves during that slice is smeared across the film. A renderer
// that samples one instant instead produces a sequence of perfectly sharp
// frames, which reads as strobing rather than as motion — the artefact is most
// obvious exactly where motion matters most, in a fast pan.
//
// # The filter
//
// Gather along the velocity vector, centred on the pixel, with a dithered start
// offset so the taps of neighbouring pixels interleave rather than banding. Two
// taps per iteration walking outward in opposite directions, each weighted by
// whether it *should* have reached this pixel.
//
// # Reference
//
// `WickedEngine-master/WickedEngine/shaders/motionblurCS.hlsl`, which is Jorge
// Jimenez's *Next Generation Post Processing in Call of Duty: Advanced Warfare*
// (SIGGRAPH 2014). Two pieces are ported and one is deliberately not:
//
// - **`DepthCmp`** classifies a tap as foreground or background relative to the
//   centre. A moving foreground object should bleed *over* a static background;
//   a static foreground must not be smeared by a background moving behind it.
//   Without it a fast pan drags the silhouette of everything static.
// - **`SpreadCmp`** asks whether the tap's own blur is long enough to reach the
//   centre pixel at all. This is what stops a still object from picking up
//   colour from a fast one merely because it is nearby.
// - **Tile-max / neighbourhood-max is not ported.** Wicked reduces velocity to
//   tiles and gathers along the tile's maximum, which is what lets a fast object
//   blur *outside* its own silhouette. That needs two more reduction passes and
//   only pays off with fast-moving *objects*; Somnium's velocity is camera-only
//   (see 24AD), and under camera motion the whole frame moves together, so the
//   centre velocity and the neighbourhood maximum agree almost everywhere. This
//   is Wicked's own `MOTIONBLUR_CHEAP` configuration, chosen for the same
//   reason it offers it.

struct MotionBlurParams {
    inv_resolution: vec2<f32>,
    /// Shutter fraction: how much of the frame interval the shutter is open.
    /// 0.5 is a 180° shutter, the film default.
    strength: f32,
    /// Taps per side. Cost is linear in this.
    samples: f32,
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var velocity_tex: texture_2d<f32>;
@group(0) @binding(2) var depth_tex: texture_depth_2d;
@group(0) @binding(3) var<uniform> mb: MotionBlurParams;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vi & 2u) * 2.0 - 1.0;
    out.clip_pos = vec4<f32>(x, -y, 0.0, 1.0);
    return out;
}

/// Interleaved gradient noise — the same dither the shadow and GTAO passes use.
///
/// A fixed start offset makes the taps of every pixel land on the same grid and
/// the blur bands; jittering the start per pixel turns those bands into noise
/// the eye integrates away.
fn mb_dither(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

/// Foreground / background classification, from Jimenez via Wicked's `DepthCmp`.
///
/// Returns `(is_sample_in_front, is_sample_behind)` as a soft pair rather than
/// a branch, so the two are blended near the depth of the centre instead of
/// switching hard along a silhouette.
fn depth_cmp(centre: f32, sample_depth: f32, scale: f32) -> vec2<f32> {
    return saturate(vec2<f32>(0.5) + vec2<f32>(scale, -scale) * (sample_depth - centre));
}

/// Does a blur of length `spread` reach `offset` pixels away?
fn spread_cmp(offset_len: f32, spread: vec2<f32>, scale: f32) -> vec2<f32> {
    return saturate(scale * spread - vec2<f32>(offset_len) + vec2<f32>(1.0));
}

fn sample_weight(
    centre_depth: f32,
    sample_depth: f32,
    offset_len: f32,
    centre_spread: f32,
    sample_spread: f32,
    px_scale: f32,
    depth_scale: f32,
) -> f32 {
    let d = depth_cmp(centre_depth, sample_depth, depth_scale);
    let s = spread_cmp(offset_len, vec2<f32>(centre_spread, sample_spread), px_scale);
    return dot(d, s);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(src));
    let coord = vec2<i32>(in.clip_pos.xy);
    let uv = (vec2<f32>(coord) + 0.5) * mb.inv_resolution;

    let centre = textureLoad(src, coord, 0);
    let centre_velocity = textureLoad(velocity_tex, coord, 0).xy * mb.strength;
    let centre_depth = textureLoad(depth_tex, coord, 0);

    // In pixels. Below half a pixel there is nothing to smear and the gather
    // would only cost a dozen taps to reproduce the centre.
    let px_len = length(centre_velocity / mb.inv_resolution);
    if px_len < 0.5 {
        return centre;
    }

    let range = max(mb.samples, 1.0);
    // The dither shifts where the walk starts, not how far it goes.
    let jitter = mb_dither(vec2<f32>(coord)) - 0.5;
    let step = centre_velocity / range;

    var sum = vec4<f32>(0.0);
    var taps = 0.0;
    var i = 1.0;
    loop {
        if i > range {
            break;
        }
        let t = i + jitter;
        // Two taps, opposite directions. Sampling symmetrically is what keeps
        // the blur centred on the object rather than trailing behind it.
        for (var s = 0; s < 2; s = s + 1) {
            let dir = select(-1.0, 1.0, s == 0);
            let uv2 = uv + step * t * dir;
            let c2 = clamp(
                vec2<i32>(uv2 / mb.inv_resolution),
                vec2<i32>(0, 0),
                dims - vec2<i32>(1, 1),
            );
            let col = textureLoad(src, c2, 0);
            let d2 = textureLoad(depth_tex, c2, 0);
            let v2 = textureLoad(velocity_tex, c2, 0).xy * mb.strength;
            let spread2 = length(v2 / mb.inv_resolution);
            let offset_len = abs(t) * px_len / range;
            // `depth_scale` is large because depth here is non-linear and the
            // interesting differences are tiny; 1000 is Wicked's own value.
            let w = sample_weight(centre_depth, d2, offset_len, px_len, spread2, 1.0, 1000.0);
            sum += vec4<f32>(col.rgb, 1.0) * w;
            taps += 1.0;
        }
        i = i + 1.0;
    }

    if taps <= 0.0 || sum.a <= 0.0 {
        return centre;
    }
    sum /= taps;
    // The weights do not sum to one — that is the point. Whatever coverage the
    // gather failed to account for is filled from the centre pixel, so a region
    // no tap could legitimately reach keeps its own colour instead of fading
    // toward black. Straight from Wicked's `sum.rgb + (1 - sum.w) * center`.
    return vec4<f32>(sum.rgb + (1.0 - sum.a) * centre.rgb, centre.a);
}
