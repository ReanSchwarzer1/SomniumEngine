// Somnium Engine — screen-space velocity (Phase 24AD).
//
// Where each pixel was on the previous frame, in UV space. Written once and
// consumed by anything that needs to walk backwards through time: motion blur
// (24Z) needs it to know which direction to smear, and it is the surface a
// future skinning or rigid-body system plugs its object motion into.
//
// # What this does and does not cover
//
// Reconstructing the world position from *this* frame's depth and projecting it
// with the previous frame's matrix gives the exact motion of a **static** point
// under a **moving camera**. It does not give the motion of a point that moved
// on its own — for that the previous frame's model matrix has to travel with
// the instance, which Somnium has nowhere to put yet: the draw queue is sorted
// afresh every frame, so instance `i` is not the same object it was last frame,
// and there is no stable per-object id to key a history on. Nothing in the
// engine currently moves independently of the camera (there is no skinning, no
// wind, and rigid bodies do not yet write transforms), so the covered case is
// presently the only case. Phase 27 is when that changes, and the note is here
// so the gap is found by reading rather than by seeing a smear stay still.
//
// # Reference
//
// `WickedEngine-master/WickedEngine/shaders/visibility_velocityCS.hlsl` — the
// structure of a velocity pass over a visibility buffer, the `(0.5, -0.5)`
// clip-to-UV scale, the clamp to [-1, 1], and the trick for the background:
// a pixel with no geometry is treated as a point on the far plane along its own
// ray, so camera *rotation* still produces velocity for the sky. Without that
// the sky is the one part of the frame that never blurs, which reads as a hole
// punched through the motion.
//
// **Jitter.** Wicked subtracts the TAA jitter from both ends. Somnium's
// matrices are un-jittered here instead, which is the same correction applied
// one level up: `TaaPass::record` already established that both ends of a
// reprojection must be un-jittered, having measured 51 000 of 51 000 pixels
// reprojecting wrongly with a still camera when they were not.

struct VelocityParams {
    /// Un-jittered, current frame. Reconstructs world position from depth.
    inv_view_proj: mat4x4<f32>,
    /// Un-jittered, previous frame.
    prev_view_proj: mat4x4<f32>,
    inv_resolution: vec2<f32>,
    /// Zero on the first frame and after a camera cut, where "previous" means
    /// nothing and a velocity would be a fabrication.
    valid: f32,
    _pad: f32,
}

@group(0) @binding(0) var depth_tex: texture_depth_2d;
@group(0) @binding(1) var<uniform> vp: VelocityParams;

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

/// Un-project an NDC point with the current inverse view-projection.
fn world_from_ndc(ndc: vec3<f32>) -> vec3<f32> {
    let p = vp.inv_view_proj * vec4<f32>(ndc, 1.0);
    return p.xyz / p.w;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec2<f32> {
    if vp.valid < 0.5 {
        return vec2<f32>(0.0);
    }

    let coord = vec2<i32>(in.clip_pos.xy);
    let uv = (vec2<f32>(coord) + 0.5) * vp.inv_resolution;
    let ndc_xy = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let depth = textureLoad(depth_tex, coord, 0);

    var world: vec3<f32>;
    if depth >= 1.0 {
        // No geometry. Take a point far along this pixel's ray rather than
        // returning zero: the sky does move on screen when the camera turns,
        // and a zero there would leave it the only unblurred part of a whip pan.
        // Just inside the far plane, because exactly 1.0 un-projects to a point
        // at infinity whose previous-frame projection is a division by ~0.
        world = world_from_ndc(vec3<f32>(ndc_xy, 0.9999));
    } else {
        world = world_from_ndc(vec3<f32>(ndc_xy, depth));
    }

    let prev_clip = vp.prev_view_proj * vec4<f32>(world, 1.0);
    if prev_clip.w <= 0.0 {
        // Behind the previous camera: there is no history for this pixel and a
        // projected position would be mirrored through the origin.
        return vec2<f32>(0.0);
    }
    let prev_ndc = prev_clip.xy / prev_clip.w;

    // Clip → UV. The y flip is the same one the NDC construction above applies.
    let velocity = (prev_ndc - ndc_xy) * vec2<f32>(0.5, -0.5);

    // Clamped as Wicked clamps it. A pixel that reprojects off the far side of
    // the screen would otherwise hand motion blur a gather direction hundreds
    // of screens long, and the loop would read the same clamped edge texel for
    // every tap.
    return clamp(velocity, vec2<f32>(-1.0), vec2<f32>(1.0));
}
