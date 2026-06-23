// Phase 11.5I: Selection outline pass.
//
// Two-subpass stencil approach (reference: bevy_mod_outline):
//   Pass 1 (vs_stencil / fs_stencil): writes stencil=1 for every pixel covered
//     by the selected entity's geometry, without writing any color.
//   Pass 2 (vs_outline / fs_outline): renders a clip-space-extruded copy of the
//     geometry; stencil NOTEQUAL=1 ensures only the "halo" pixels survive.
//
// Vertex pulling from storage buffers avoids the need for a separate vertex
// buffer — the same GeometryPool buffers used by the visibility pass are reused.
//
// Normal extrusion formula (inspired by bevy_mod_outline clip-space expansion):
//   1. Compute clip-space position and clip-space world normal direction.
//   2. Normalize the XY component of the projected normal.
//   3. Multiply by outline_width * clip.w so the width is perspective-correct
//      (constant in screen space regardless of entity depth).

struct GpuVertex {
    pos_x: f32, pos_y: f32, pos_z: f32,
    nx:    f32, ny:    f32, nz:    f32,
    u:     f32, v:     f32,
}

struct OutlineUniforms {
    view_proj:     mat4x4<f32>,   // 64 bytes  offset   0
    model:         mat4x4<f32>,   // 64 bytes  offset  64
    outline_color: vec4<f32>,     // 16 bytes  offset 128
    outline_width: f32,           //  4 bytes  offset 144
    vertex_offset: u32,           //  4 bytes  offset 148
    index_offset:  u32,           //  4 bytes  offset 152
    _pad:          u32,           //  4 bytes  offset 156
}                                 // total = 160 bytes

@group(0) @binding(0) var<uniform>         u:       OutlineUniforms;
@group(0) @binding(1) var<storage, read>   vertices: array<GpuVertex>;
@group(0) @binding(2) var<storage, read>   indices:  array<u32>;

// ── Shared vertex-pull helper ─────────────────────────────────────────────────

fn pull_clip_pos(vid: u32) -> vec4<f32> {
    let idx = indices[u.index_offset + vid] + u.vertex_offset;
    let v   = vertices[idx];
    let pos = vec3<f32>(v.pos_x, v.pos_y, v.pos_z);
    return u.view_proj * u.model * vec4<f32>(pos, 1.0);
}

fn pull_world_normal(vid: u32) -> vec3<f32> {
    let idx = indices[u.index_offset + vid] + u.vertex_offset;
    let v   = vertices[idx];
    // Model * normal (ignore non-uniform scale; editor meshes are uniform-scale)
    return (u.model * vec4<f32>(v.nx, v.ny, v.nz, 0.0)).xyz;
}

// ── Sub-pass 1: stencil write ─────────────────────────────────────────────────

@vertex
fn vs_stencil(@builtin(vertex_index) vid: u32) -> @builtin(position) vec4<f32> {
    return pull_clip_pos(vid);
}

@fragment
fn fs_stencil() -> @location(0) vec4<f32> {
    // Color output discarded (write_mask = NONE in the pipeline).
    // Only stencil=1 is written via the depth-stencil state.
    return vec4<f32>(0.0);
}

// ── Sub-pass 2: extruded outline ──────────────────────────────────────────────

@vertex
fn vs_outline(@builtin(vertex_index) vid: u32) -> @builtin(position) vec4<f32> {
    let clip        = pull_clip_pos(vid);
    let world_norm  = pull_world_normal(vid);

    // Project world-space normal into clip space (direction only, no translate).
    let clip_norm_dir = (u.view_proj * vec4<f32>(world_norm, 0.0)).xy;

    // Safe normalize: if the normal projects to zero (e.g. fully front-facing),
    // default to a small upward bias to avoid NaN.
    let len  = length(clip_norm_dir);
    let safe = select(vec2<f32>(0.0, 1.0), clip_norm_dir / len, len > 0.0001);

    // Perspective-correct extrusion: multiply by clip.w so the screen-space
    // offset stays constant regardless of depth.
    let offset = safe * u.outline_width * clip.w;

    return vec4<f32>(clip.x + offset.x, clip.y + offset.y, clip.z, clip.w);
}

@fragment
fn fs_outline() -> @location(0) vec4<f32> {
    return u.outline_color;
}
