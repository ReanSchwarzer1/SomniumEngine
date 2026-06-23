enable primitive_index;

// Visibility Buffer - Rasterization Pass

struct Vertex {
    pos_x: f32,
    pos_y: f32,
    pos_z: f32,
    norm_x: f32,
    norm_y: f32,
    norm_z: f32,
    u: f32,
    v: f32,
}

struct Instance {
    model: mat4x4<f32>,
    material_id: u32,
    vertex_offset: u32,
    index_offset: u32,
    _padding: u32,
}

struct View {
    view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    _padding: f32,
}

@group(0) @binding(0) var<storage, read> vertices: array<Vertex>;
@group(0) @binding(1) var<storage, read> indices: array<u32>;
@group(0) @binding(2) var<storage, read> instances: array<Instance>;
@group(0) @binding(3) var<storage, read> view: View;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) @interpolate(flat) instance_id: u32,
}

@vertex
fn vs_main(
    @builtin(vertex_index) v_idx: u32,
    @builtin(instance_index) inst_idx: u32
) -> VertexOutput {
    var out: VertexOutput;
    
    let instance = instances[inst_idx];
    let index = indices[instance.index_offset + v_idx];
    let vertex = vertices[instance.vertex_offset + index];
    
    let pos = vec3<f32>(vertex.pos_x, vertex.pos_y, vertex.pos_z);
    out.clip_pos = view.view_proj * instance.model * vec4<f32>(pos, 1.0);
    out.instance_id = inst_idx;
    
    return out;
}

@fragment
fn fs_main(
    @builtin(primitive_index) prim_idx: u32,
    @location(0) @interpolate(flat) instance_id: u32
) -> @location(0) u32 {
    // Add 1 so 0 is reserved as the sky/background sentinel (clear value is 0).
    // Supports up to 1022 simultaneous instances (inst 0..1022 → stored as 1..1023).
    let packed_id = ((instance_id + 1u) << 22u) | (prim_idx & 0x3FFFFFu);
    return packed_id;
}
