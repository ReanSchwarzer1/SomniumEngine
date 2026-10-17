// Somnium Engine — screen-space velocity (Phase 24AD).
//!include "global_pool.wgsl"
//!include "wind.wgsl"
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
// under a **moving camera**. A point that moved on its own (C2, 16 October 2026:
// held tools, a body walking, a skinned hand) comes from `motion`: per instance,
// its previous model matrix (the renderer keys last frame's transforms by mesh
// and order, since the draw queue is sorted afresh) and, for a skinned mesh,
// where last frame's deformed vertices are kept. The pixel's point is found on
// its own triangle (the visibility buffer says which), and the same point of
// that triangle last frame is projected with the previous camera.
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

// @group(0) is the global pool (instances, materials, the view buffer's wind).
@group(1) @binding(0) var depth_tex: texture_depth_2d;
@group(1) @binding(1) var<uniform> vp: VelocityParams;
// Which instance covers the pixel, for foliage that moves on its own.
@group(1) @binding(2) var vis_tex: texture_2d<u32>;

struct InstanceMotion {
    prev_model: mat4x4<f32>,
    /// Where last frame's pose of a skinned mesh starts in `previous_pose`, or 0xffffffff.
    prev_vertex_base: u32,
    /// 0: nothing of its own (the camera and wind cover it).
    moving: u32,
    _pad0: u32,
    _pad1: u32,
}
@group(1) @binding(3) var<storage, read> motion: array<InstanceMotion>;
@group(1) @binding(4) var<storage, read> previous_pose: array<Vertex>;

fn vertex_position(v: Vertex) -> vec3<f32> {
    return vec3<f32>(v.pos_x, v.pos_y, v.pos_z);
}

/// Where `world` was last frame, for a pixel of instance `id`'s triangle `prim` that moved on its own.
fn previous_world(id: u32, prim: u32, world: vec3<f32>) -> vec3<f32> {
    let inst = instances[id];
    let mo = motion[id];
    let base = inst.index_offset + prim * 3u;
    let i0 = indices[base];
    let i1 = indices[base + 1u];
    let i2 = indices[base + 2u];
    let a = (inst.model * vec4<f32>(vertex_position(vertices[inst.vertex_offset + i0]), 1.0)).xyz;
    let b = (inst.model * vec4<f32>(vertex_position(vertices[inst.vertex_offset + i1]), 1.0)).xyz;
    let c = (inst.model * vec4<f32>(vertex_position(vertices[inst.vertex_offset + i2]), 1.0)).xyz;
    // The point's place on its triangle (barycentric), then the same place on last frame's triangle.
    let e0 = b - a;
    let e1 = c - a;
    let e2 = world - a;
    let d00 = dot(e0, e0);
    let d01 = dot(e0, e1);
    let d11 = dot(e1, e1);
    let d20 = dot(e2, e0);
    let d21 = dot(e2, e1);
    let denom = d00 * d11 - d01 * d01;
    var w = vec3<f32>(1.0 / 3.0);
    if abs(denom) > 1e-12 {
        let v = (d11 * d20 - d01 * d21) / denom;
        let u = (d00 * d21 - d01 * d20) / denom;
        w = vec3<f32>(1.0 - v - u, v, u);
    }
    var pa = vertex_position(vertices[inst.vertex_offset + i0]);
    var pb = vertex_position(vertices[inst.vertex_offset + i1]);
    var pc = vertex_position(vertices[inst.vertex_offset + i2]);
    if mo.prev_vertex_base != 0xffffffffu {
        pa = vertex_position(previous_pose[mo.prev_vertex_base + i0]);
        pb = vertex_position(previous_pose[mo.prev_vertex_base + i1]);
        pc = vertex_position(previous_pose[mo.prev_vertex_base + i2]);
    }
    let local = pa * w.x + pb * w.y + pc * w.z;
    return (mo.prev_model * vec4<f32>(local, 1.0)).xyz;
}

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

    // A point that moved on its own: where the same point of its triangle was last frame.
    var moved = false;
    if depth < 1.0 {
        let vis = textureLoad(vis_tex, coord, 0).xy;
        if vis.x != 0u && vis.x - 1u < arrayLength(&motion) && motion[vis.x - 1u].moving != 0u {
            world = previous_world(vis.x - 1u, vis.y, world);
            moved = true;
        }
    }

    // Foliage wind moves a point without the camera moving: take this frame's
    // swayed position back to its rest pose (two fixed-point steps; the sway
    // barely depends on where along it the point is) and forward to where the
    // wind had it last frame. Same `wind_offset` as every raster pass.
    if depth < 1.0 && !moved {
        let vis = textureLoad(vis_tex, coord, 0).xy;
        if vis.x != 0u {
            let inst = instances[vis.x - 1u];
            let m = materials[inst.material_id];
            if m.terrain_index < 0 && (m.wind_bend > 0.0 || m.wind_flutter > 0.0) {
                let pivot = inst.model[3].xyz;
                var rest = world - wind_offset(world, pivot, m.wind_bend, m.wind_flutter,
                                               view.wind_time.x, view.wind, view.camera_pos);
                rest = world - wind_offset(rest, pivot, m.wind_bend, m.wind_flutter,
                                           view.wind_time.x, view.wind, view.camera_pos);
                world = rest + wind_offset(rest, pivot, m.wind_bend, m.wind_flutter,
                                           view.wind_time.y, view.wind, view.camera_pos);
            }
        }
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
