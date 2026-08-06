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

struct Material {
    base_color: vec4<f32>,
    roughness: f32,
    metallic: f32,
    albedo_map: i32,
    normal_map: i32,
    metallic_roughness_map: i32,
    alpha_cutoff: f32,
    flags: u32,
    _padding: u32,
}

@group(0) @binding(0) var<storage, read> vertices: array<Vertex>;
@group(0) @binding(1) var<storage, read> indices: array<u32>;
@group(0) @binding(2) var<storage, read> instances: array<Instance>;
@group(0) @binding(3) var<storage, read> view: View;
// Phase 17D: alpha cutout needs the albedo texture here, not just in shading.
// The visibility buffer decides what exists at each pixel, so a leaf's cut-away
// corners have to be discarded now — resolving them later would be too late,
// the depth buffer would already show a solid quad.
@group(0) @binding(4) var textures: binding_array<texture_2d<f32>>;
@group(0) @binding(5) var<storage, read> materials: array<Material>;

@group(1) @binding(0) var cutout_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) @interpolate(flat) instance_id: u32,
    // Triangle index relative to the MESH, not to the draw.
    //
    // This used to come from `@builtin(primitive_index)`, which restarts at 0
    // for every draw call. That was fine while a draw was a whole mesh, but
    // Phase 15F splits a mesh across one draw per cluster, and the shading pass
    // uses this id to fetch the triangle out of the geometry pool — a
    // per-draw id would send it to the wrong triangle in every cluster after
    // the first.
    //
    // `vertex_index` includes `first_vertex`, which for a cluster draw is that
    // cluster's index offset within the mesh, so dividing by three recovers the
    // mesh-relative triangle. All three vertices of triangle k have indices
    // 3k, 3k+1 and 3k+2, so integer division gives k for each of them and the
    // flat interpolation is well defined.
    @location(1) @interpolate(flat) prim_id: u32,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) material_id: u32,
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
    out.uv = vec2<f32>(vertex.u, vertex.v);
    out.material_id = instance.material_id;
    out.clip_pos = view.view_proj * instance.model * vec4<f32>(pos, 1.0);
    out.instance_id = inst_idx;
    out.prim_id = v_idx / 3u;
    
    return out;
}

@fragment
fn fs_main(
    @location(0) @interpolate(flat) instance_id: u32,
    @location(1) @interpolate(flat) prim_idx: u32,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) material_id: u32
) -> @location(0) u32 {
    // Derivatives are taken at top level, where control flow is uniform, and
    // fed to textureSampleGrad below. Sampling inside the branch directly would
    // break WGSL's uniformity rule, and dropping to LOD 0 instead would make
    // distant foliage crawl with aliasing.
    let ddx_uv = dpdx(uv);
    let ddy_uv = dpdy(uv);

    let material = materials[material_id];
    // Cutoff is 0 for OPAQUE and BLEND, so this costs one compare for
    // everything that is not alpha-tested.
    if material.alpha_cutoff > 0.0 && material.albedo_map >= 0 {
        let alpha = textureSampleGrad(
            textures[material.albedo_map], cutout_sampler, uv, ddx_uv, ddy_uv,
        ).a;
        if alpha < material.alpha_cutoff {
            discard;
        }
    }

    // Phase 15C: 16/16 split — 65 535 instances x 65 536 triangles per draw.
    // Was 10/22, which capped the whole scene at 1022 draws; triangle counts
    // that large belong in separate draws (and will become meshlets in 15D).
    //
    // Add 1 so 0 stays reserved as the sky/background sentinel (the vis buffer
    // clears to 0). Instance 0 therefore encodes as 0x00010000, never 0.
    let packed_id = ((instance_id + 1u) << 16u) | (prim_idx & 0xFFFFu);
    return packed_id;
}
