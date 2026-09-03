// Somnium Engine — the global resource pool, shared by every pass that needs
// the scene (Phase 24L).
//
// These declarations used to live at the top of `shading.wgsl`, which was fine
// while the shading pass was the only thing that resolved geometry. ReSTIR GI
// resolves a *ray hit* through the same `instances` array — deliberately, so a
// traced surface and a rasterised one can never disagree about what the scene
// is — and duplicating the block into a second file would have made that
// agreement a coincidence maintained by hand.
//
// Concatenated ahead of any module that binds `@group(0)`.

// ─── Shared structs ─────────────────────────────────────────────────────────

// wgpu 30 requires `binding_array<...>` to be behind an explicit enable
// directive; wgpu 29 accepted it without one. Found by MORROWIND-C, because
// MORROWIND-A2 bumped wgpu to 30 and left this crate's `naga` dev-dependency
// on 29 — so the validation test was checking these files with the *old*
// front end and passed. The resolver hoists and de-duplicates `enable`
// lines, so a module that includes this one inherits it.
enable wgpu_binding_array;

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
    // Phase CONTROL-N: water uptake, 0..1. See `pool.rs`.
    porosity: f32,
    _pad2: f32,
}

// Phase 11D: view matrix added at offset 128 (Option A — buffer expanded to 208 bytes).
// visibility.wgsl's shorter View struct still reads only view_proj at offset 0 — no change needed there.
struct View {
    view_proj:     mat4x4<f32>,   // offset   0  (64 bytes)
    inv_view_proj: mat4x4<f32>,   // offset  64  (64 bytes)
    view:          mat4x4<f32>,   // offset 128  (64 bytes)  ← Phase 11D
    camera_pos:    vec3<f32>,     // offset 192  (12 bytes)
    _padding:      f32,           // offset 204  ( 4 bytes)
    // debug_flags at offset 208 would need buffer expansion; instead we repurpose _padding:
    // bit 0 of _padding (reinterpreted as u32) = cascade debug overlay enable.
    // We use a separate f32 field below for clarity.
}

// GpuDirectionalLight (336 bytes) — matches shadow/mod.rs::GpuDirectionalLight.
struct DirectionalLight {
    direction:       vec3<f32>,               // offset   0
    _pad0:           f32,                     // offset  12
    color:           vec3<f32>,               // offset  16  pre-multiplied by intensity
    _pad1:           f32,                     // offset  28
    view_proj:       array<mat4x4<f32>, 4>,   // offset  32  (256 bytes)
    cascade_splits:  vec4<f32>,               // offset 288  view-space far Z per cascade
    shadow_map_size: f32,                     // offset 304  total atlas texels (4096)
    ibl_intensity:   f32,                     // offset 308  Phase 22C: editable indirect strength
    sun_angular_radius: f32,                  // offset 312  Phase 24E
    _pad2_z:         f32,                     // offset 316
    moon_direction:  vec3<f32>,               // offset 320  Phase 25M-2: physical lunar orbit
    moon_intensity:  f32,                     // offset 332  Phase 25M-2: moonlight illuminance in lux
}

struct GpuLocalLight {
    position_ws: vec3<f32>,
    range: f32,
    color: vec3<f32>,
    light_type: u32,
    direction_ws: vec3<f32>,
    spot_cos_outer: f32,
    spot_cos_inner: f32,
    radius: f32,
    _pad1: f32,
    _pad2: f32,
}

struct ClusterOffset {
    offset: u32,
    count: u32,
}

struct ClusterParams {
    grid_width: u32,
    grid_height: u32,
    num_slices: u32,
    tile_size: u32,
    near: f32,
    far: f32,
    // Bit 0 = cel, bit 1 = PCSS, bit 2 = contact shadows, bit 3 = analytic UV grads,
    // bit 4 = ReSTIR DI wrote sun visibility; bit 5 = DREAMS-B terrain STF.
    shading_mode: u32,
    num_local_lights: u32,
}

// ─── Bindings ────────────────────────────────────────────────────────────────

@group(0) @binding(0) var<storage, read> vertices:  array<Vertex>;
@group(0) @binding(1) var<storage, read> indices:   array<u32>;
@group(0) @binding(2) var<storage, read> instances: array<Instance>;
@group(0) @binding(3) var<storage, read> view:      View;
@group(0) @binding(4) var textures:                 binding_array<texture_2d<f32>>;
@group(0) @binding(5) var<storage, read> materials: array<Material>;
@group(0) @binding(6) var<storage, read> light:     DirectionalLight;
@group(0) @binding(7) var<storage, read> local_lights: array<GpuLocalLight>;
@group(0) @binding(8) var<storage, read> light_index_list: array<u32>;
@group(0) @binding(9) var<storage, read> cluster_offsets: array<ClusterOffset>;
@group(0) @binding(10) var<storage, read> cluster_params: ClusterParams;

