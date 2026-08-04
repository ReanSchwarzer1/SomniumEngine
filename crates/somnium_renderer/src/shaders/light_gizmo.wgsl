// Phase 13E: Light gizmo shader.
//
// Draws unlit, per-vertex-colored LINE geometry for point / spot / directional
// light visualizations. Unlike gizmo.wgsl there is no model matrix — the CPU
// emits world-space line segments directly, because every light has its own
// transform and they are all batched into a single draw call.
//
// Rendered to the swapchain after tone mapping, with no depth test, so light
// bounds stay visible through scene geometry (standard editor behaviour).

struct GizmoView {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    view:          mat4x4<f32>,
    camera_pos:    vec3<f32>,
    _pad:          f32,
}

@group(0) @binding(0) var<storage, read> gv: GizmoView;

struct VIn {
    @location(0) position: vec3<f32>,
    @location(1) color:    vec3<f32>,
}

struct VOut {
    @builtin(position) clip:  vec4<f32>,
    @location(0)       color: vec3<f32>,
}

@vertex
fn vs_main(in: VIn) -> VOut {
    return VOut(gv.view_proj * vec4(in.position, 1.0), in.color);
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    return vec4(in.color, 1.0);
}
