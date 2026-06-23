// Phase 11.5B: Editor transform gizmo shader.
//
// Renders unlit, per-vertex-colored geometry for the translate / rotate / scale
// gizmos. The model matrix in GizmoParams places the gizmo at the selected
// entity's world position and scales it to stay a constant screen size.
// No depth test: gizmos always render on top of scene geometry.

struct GizmoView {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    view:          mat4x4<f32>,
    camera_pos:    vec3<f32>,
    _pad:          f32,
}

struct GizmoParams {
    model: mat4x4<f32>,
}

@group(0) @binding(0) var<storage, read> gv: GizmoView;
@group(0) @binding(1) var<storage, read> gp: GizmoParams;

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
    let world = gp.model * vec4(in.position, 1.0);
    return VOut(gv.view_proj * world, in.color);
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    return vec4(in.color, 1.0);
}
