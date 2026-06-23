// Phase 11.5J: GPU Particle System.
//
// Billboard instanced rendering (reference: bevy_enoki CPU+GPU particle pipeline).
//
// Vertex shader: generates 6 vertices per particle (two CCW triangles = one quad),
// expanding billboard corners in view space using camera_right / camera_up vectors
// extracted from the view matrix.  No vertex buffer — all geometry comes from
// vertex_index and the per-particle storage buffer.
//
// Fragment shader: multiplies the interpolated particle color (which fades from
// color_start to color_end as lifetime_frac goes 0→1) with a smooth radial alpha
// for a soft billboard look.

struct ParticleView {
    view_proj:    mat4x4<f32>,   //   0 bytes, 64 bytes
    camera_right: vec3<f32>,     //  64 bytes, 12 bytes
    _pad0:        f32,           //  76 bytes,  4 bytes
    camera_up:    vec3<f32>,     //  80 bytes, 12 bytes
    _pad1:        f32,           //  92 bytes,  4 bytes
}                                // = 96 bytes

struct GpuParticle {
    position: vec3<f32>,   // world-space centre
    size:     f32,         // billboard half-width (in metres)
    color:    vec4<f32>,   // linear RGBA
}

@group(0) @binding(0) var<uniform>       pview:     ParticleView;
@group(0) @binding(1) var<storage, read> particles: array<GpuParticle>;

// 2 CCW triangles forming a unit quad (indexed by vertex_index in [0,6))
const QUAD_UV: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
);
// Corresponding billboard offsets in [-1,1]
const QUAD_OFF: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0,  1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(1.0,  1.0), vec2<f32>(-1.0, 1.0),
);

struct VertOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv:    vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(
    @builtin(vertex_index)   vid: u32,
    @builtin(instance_index) iid: u32,
) -> VertOut {
    let p      = particles[iid];
    let offset = QUAD_OFF[vid] * p.size * 0.5;
    let world  = p.position
                 + pview.camera_right * offset.x
                 + pview.camera_up    * offset.y;

    var out: VertOut;
    out.clip_pos = pview.view_proj * vec4<f32>(world, 1.0);
    out.uv       = QUAD_UV[vid];
    out.color    = p.color;
    return out;
}

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    // Soft radial falloff: 1 at centre, 0 at edges
    let d     = distance(in.uv, vec2<f32>(0.5));
    let alpha = smoothstep(0.5, 0.2, d);
    return vec4<f32>(in.color.rgb, in.color.a * alpha);
}
