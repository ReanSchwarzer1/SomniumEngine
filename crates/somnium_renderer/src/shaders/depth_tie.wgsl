// Depth ties (29 Oct 2026): which of two faces in one plane a pixel keeps, the same every frame under FSR's and TAA's
// jitter. The visibility pass draws each opaque draw its tie's steps nearer (renderer.rs `depth_tie`: smaller things
// 8 to 15 over larger 0 to 7); blended surfaces (transparent.wgsl) are drawn `DEPTH_TIE_GLASS` steps nearer, one above
// them all, so glass stays over the picture behind it; the occlusion tests (cull.wgsl, culling.rs) allow for the most
// a tie can move anything.
//
// One step is as much depth as two copies of a plane can round apart. Seen at a slant that is mostly the rasterizer
// snapping their corners to its sub-pixel grid: the view's `depth_tie_slope` (pass/visibility.rs `depth_tie_slope`,
// the near plane times a pixel's angle times 1/64) times how far the plane is turned from facing the eye, over its
// distance from the eye, so a wall seen edge-on gets far more than a floor seen from standing height. At least the
// depth's own rounding: one float step near 1 for a plane faced (copies of it never fought), about four for one turned
// edge-on (`DEPTH_TIE_STEP_TURNED`). At most `DEPTH_TIE_STEP_MAX`. Positions are taken from the eye (`depth_tie_clip`),
// or far from the world's origin the rounding is many times all of that. Faded out from 12 to 24 m: farther off a tie's
// steps would move whole layers (a rug, a painting) as often as they settle a fight, so faces still in one plane there
// are the scene's to separate. tests/depth_tie_jitter.rs holds ties one step apart, with steps half the size too.

const DEPTH_TIE_STEP_MIN: f32 = 6e-8;
/// What a plane turned from the eye adds to the least step: the depth it rounds to across the screen, whatever the
/// pixels, which a step must beat by two of clip z's own steps (at 45 degrees and 8 m, at the game's pixels, one step
/// of clip z still fought: tests/depth_tie_jitter.rs).
const DEPTH_TIE_STEP_TURNED: f32 = 2.4e-7;
const DEPTH_TIE_STEP_MAX: f32 = 2e-6;
const DEPTH_TIE_FADE_START: f32 = 12.0;
const DEPTH_TIE_FADE_END: f32 = 24.0;
/// Blended surfaces' tie: one above the largest opaque tie (15).
const DEPTH_TIE_GLASS: f32 = 16.0;
/// The most any tie draws anything nearer, near the eye (pass/visibility.rs `DEPTH_TIE_REACH`): `DEPTH_TIE_GLASS`
/// steps at their largest.
const DEPTH_TIE_REACH: f32 = 3.2e-5;

/// A world position's clip position, taken from the eye: the view-projection of the offset from the eye plus that of
/// the eye, which is the same for every vertex. `view_proj * world` sums products of the world's coordinates and takes
/// back almost all of them, so its rounding grows with the distance from the world's origin: at the Town's 50 m it put
/// two copies of one plane 16 float steps of depth apart, more than any tie (F's check, 29 Oct). From the eye, the
/// products are only as large as the distance from the eye.
fn depth_tie_clip(view_proj: mat4x4<f32>, world: vec3<f32>, camera_pos: vec3<f32>) -> vec4<f32> {
    return view_proj * vec4<f32>(world - camera_pos, 0.0) + view_proj * vec4<f32>(camera_pos, 1.0);
}

/// How much of a tie is left at view depth `w`: all of it to 12 m, none past 24 m.
fn depth_tie_fade(w: f32) -> f32 {
    return saturate((DEPTH_TIE_FADE_END - w) / (DEPTH_TIE_FADE_END - DEPTH_TIE_FADE_START));
}

/// What to take off clip-space z (it is already times w) to draw a vertex `tie` steps nearer: `normal` and `world`
/// are the vertex's (the normal need not be unit length), `clip_w` its clip-space w.
fn depth_tie_offset(tie: f32, normal: vec3<f32>, world: vec3<f32>, clip_w: f32, view_matrix: mat4x4<f32>,
                    camera_pos: vec3<f32>, slope: f32) -> f32 {
    let length_n = length(normal);
    let h = select(1000.0, abs(dot(normal, camera_pos - world)) / length_n, length_n > 1e-6);
    let turned = select(1.0, length((view_matrix * vec4<f32>(normal, 0.0)).xy) / length_n, length_n > 1e-6);
    let least = DEPTH_TIE_STEP_MIN + DEPTH_TIE_STEP_TURNED * turned;
    let step = clamp(slope * turned / max(h, 0.01), least, DEPTH_TIE_STEP_MAX);
    return tie * step * depth_tie_fade(clip_w) * clip_w;
}
