// Somnium Engine — Shadow Map Pass
// Depth-only vertex shader for cascade shadow map generation.
//
// Uses programmable vertex pulling (same pattern as visibility.wgsl):
// no vertex buffer bindings; reads directly from storage arrays.
//
// @group(0) mirrors the GlobalResourcePool layout.
// @group(1) holds a per-cascade uniform (index 0..3) that selects which
// light.view_proj[cascade.index] to apply.

struct Vertex {
    pos_x: f32, pos_y: f32, pos_z: f32,
    norm_x: f32, norm_y: f32, norm_z: f32,
    u: f32, v: f32,
}

struct Instance {
    model: mat4x4<f32>,
    material_id: u32,
    vertex_offset: u32,
    index_offset: u32,
    _padding: u32,
}

struct DirectionalLight {
    direction: vec3<f32>,
    _pad0: f32,
    color: vec3<f32>,
    _pad1: f32,
    view_proj: array<mat4x4<f32>, 4>,
    cascade_splits: vec4<f32>,
    shadow_map_size: f32,
    _pad2_x: f32,
    _pad2_y: f32,
    _pad2_z: f32,
}

struct CascadeUniform {
    index: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

// @group(0) — GlobalResourcePool (bindings 0..6)
@group(0) @binding(0) var<storage, read> vertices:  array<Vertex>;
@group(0) @binding(1) var<storage, read> indices:   array<u32>;
@group(0) @binding(2) var<storage, read> instances: array<Instance>;
// bindings 3 (view), 4 (textures), 5 (materials) are present in the layout but unused here.
@group(0) @binding(6) var<storage, read> light: DirectionalLight;

// @group(1) — per-cascade uniform
@group(1) @binding(0) var<uniform> cascade: CascadeUniform;

@vertex
fn vs_main(
    @builtin(vertex_index)   v_idx:    u32,
    @builtin(instance_index) inst_idx: u32,
) -> @builtin(position) vec4<f32> {
    let instance  = instances[inst_idx];
    let index     = indices[instance.index_offset + v_idx];
    let vert      = vertices[instance.vertex_offset + index];
    let world_pos = instance.model * vec4<f32>(vert.pos_x, vert.pos_y, vert.pos_z, 1.0);
    let vp        = light.view_proj[cascade.index];
    return vp * world_pos;
}
