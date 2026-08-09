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
    sun_angular_radius: f32,
    _pad2_z: f32,
}

struct CascadeUniform {
    index: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

// @group(0) — GlobalResourcePool (bindings 0..6)
struct Material {
    base_color: vec4<f32>,
    roughness: f32,
    metallic: f32,
    albedo_map: i32,
    normal_map: i32,
    metallic_roughness_map: i32,
    alpha_cutoff: f32,
    flags: u32,
    occlusion_map: i32,
    transmission: f32,
    // Three scalars, not a vec3.
    //
    // WGSL gives vec3<f32> a 16-byte alignment, so `emissive: vec3<f32>` here
    // sat at offset 64 and rounded the struct to 96 bytes, while Rust's
    // repr(C) packs [f32; 3] at offset 52 for a total of 80. Every material
    // past index 0 was therefore read from the wrong offset: `metallic` came
    // back as garbage, and a metallic reading of ~1 zeroes kD, so the sun's
    // diffuse term vanished on those materials and only IBL remained. That is
    // why primitives looked flat and showed no shadow (there was no sun term
    // left to darken), and why foliage rendered with wrong colours -- one bug,
    // scaling with material index.
    emissive_r: f32,
    emissive_g: f32,
    emissive_b: f32,
    emissive_map: i32,
    // Phase 25A-2: slot in the terrain-material array, or -1.
    terrain_index: i32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0) var<storage, read> vertices:  array<Vertex>;
@group(0) @binding(1) var<storage, read> indices:   array<u32>;
@group(0) @binding(2) var<storage, read> instances: array<Instance>;
// binding 3 (view) is present in the layout but unused here.
@group(0) @binding(4) var textures: binding_array<texture_2d<f32>>;
@group(0) @binding(5) var<storage, read> materials: array<Material>;
@group(0) @binding(6) var<storage, read> light: DirectionalLight;

// @group(1) — per-cascade uniform
@group(1) @binding(0) var<uniform> cascade: CascadeUniform;
// @group(2) — sampler for the alpha-cutout test (Phase 17E)
@group(2) @binding(0) var cutout_sampler: sampler;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) material_id: u32,
}

@vertex
fn vs_main(
    @builtin(vertex_index)   v_idx:    u32,
    @builtin(instance_index) inst_idx: u32,
) -> VOut {
    let instance  = instances[inst_idx];
    let index     = indices[instance.index_offset + v_idx];
    let vert      = vertices[instance.vertex_offset + index];
    let world_pos = instance.model * vec4<f32>(vert.pos_x, vert.pos_y, vert.pos_z, 1.0);
    let vp        = light.view_proj[cascade.index];

    var out: VOut;
    out.clip = vp * world_pos;
    out.uv = vec2<f32>(vert.u, vert.v);
    out.material_id = instance.material_id;
    return out;
}

// Phase 17E: alpha-tested geometry has to cut out its shadow too.
//
// The pass was depth-only with no fragment stage, so every alpha-tested card
// cast the shadow of its whole quad. A field of grass then buried itself under
// thousands of solid rectangles and came out nearly black.
//
// There are no colour targets — the fragment stage exists purely so `discard`
// can reach the depth buffer.
@fragment
fn fs_main(in: VOut) {
    let material = materials[in.material_id];
    let ddx_uv = dpdx(in.uv);
    let ddy_uv = dpdy(in.uv);
    if material.alpha_cutoff > 0.0 && material.albedo_map >= 0 {
        let alpha = textureSampleGrad(
            textures[material.albedo_map], cutout_sampler, in.uv, ddx_uv, ddy_uv,
        ).a;
        if alpha < material.alpha_cutoff {
            discard;
        }
    }
}
