// Phase 21: forward pass for alpha-blended materials.
//
// The visibility buffer stores exactly one triangle per pixel, so it cannot
// represent see-through surfaces. Blended geometry (glTF `alphaMode: BLEND`)
// is therefore drawn here instead: a normal forward pass, after opaque shading
// has already filled the HDR target, depth-tested against the opaque depth but
// NOT writing depth, sorted back-to-front on the CPU.
//
// Shading is deliberately lighter than `shading.wgsl`: the sun with its shadow
// plus an IBL reflection. Glass reads mostly as a reflection and a tint, and
// skipping the clustered-light loop keeps this pass cheap for what is usually
// a small amount of screen area.

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
}

struct View {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    view:          mat4x4<f32>,
    camera_pos:    vec3<f32>,
    _padding:      f32,
}

struct DirectionalLight {
    direction:       vec3<f32>,
    _pad0:           f32,
    color:           vec3<f32>,
    _pad1:           f32,
    view_proj:       array<mat4x4<f32>, 4>,
    cascade_splits:  vec4<f32>,
    shadow_map_size: f32,
    ibl_intensity:   f32,
    _pad2_y:         f32,
    _pad2_z:         f32,
}

@group(0) @binding(0) var<storage, read> vertices:  array<Vertex>;
@group(0) @binding(1) var<storage, read> indices:   array<u32>;
@group(0) @binding(2) var<storage, read> instances: array<Instance>;
@group(0) @binding(3) var<storage, read> view:      View;
@group(0) @binding(4) var textures:                 binding_array<texture_2d<f32>>;
@group(0) @binding(5) var<storage, read> materials: array<Material>;
@group(0) @binding(6) var<storage, read> light:     DirectionalLight;

@group(1) @binding(0) var tex_sampler: sampler;
@group(1) @binding(1) var env_cube:    texture_cube<f32>;
@group(1) @binding(2) var env_sampler: sampler;

const ENV_MAX_MIP: f32 = 5.0;


struct VOut {
    @builtin(position) clip:      vec4<f32>,
    @location(0)       world_pos: vec3<f32>,
    @location(1)       normal:    vec3<f32>,
    @location(2)       uv:        vec2<f32>,
    @location(3) @interpolate(flat) material_id: u32,
}

@vertex
fn vs_main(
    @builtin(vertex_index)   v_idx:    u32,
    @builtin(instance_index) inst_idx: u32,
) -> VOut {
    // Same programmable vertex pulling as the visibility pass — no vertex
    // buffer is bound; geometry comes from the global pool.
    let instance = instances[inst_idx];
    let index    = indices[instance.index_offset + v_idx];
    let vertex   = vertices[instance.vertex_offset + index];

    let local  = vec3<f32>(vertex.pos_x, vertex.pos_y, vertex.pos_z);
    let world  = instance.model * vec4<f32>(local, 1.0);
    let normal = normalize((instance.model * vec4<f32>(vertex.norm_x, vertex.norm_y, vertex.norm_z, 0.0)).xyz);

    var out: VOut;
    out.clip        = view.view_proj * world;
    out.world_pos   = world.xyz;
    out.normal      = normal;
    out.uv          = vec2<f32>(vertex.u, vertex.v);
    out.material_id = instance.material_id;
    return out;
}

@fragment
fn fs_main(in: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let material = materials[in.material_id];

    var albedo = material.base_color.rgb;
    var alpha  = material.base_color.a;
    if material.albedo_map >= 0 {
        let s = textureSample(textures[material.albedo_map], tex_sampler, in.uv);
        albedo *= s.rgb;
        alpha  *= s.a;
    }

    var roughness = max(material.roughness, 0.05);
    var metallic  = material.metallic;
    if material.metallic_roughness_map >= 0 {
        let mr = textureSample(textures[material.metallic_roughness_map], tex_sampler, in.uv);
        roughness = max(mr.g, 0.05);
        metallic  = mr.b;
    }

    // Blended materials are usually double-sided and thin (window glass), so
    // flip the normal on back faces or the far side lights inside-out.
    var n = normalize(in.normal);
    if !front {
        n = -n;
    }
    let v = normalize(view.camera_pos - in.world_pos);
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);

    // Direct sun, no shadow lookup: the shadow atlas is bound to the shading
    // pass's group and glass rarely reads as shadowed anyway.
    let l = normalize(light.direction);
    let n_dot_l = max(dot(n, l), 0.0);
    let h = normalize(v + l);
    let spec = pow(max(dot(n, h), 0.0), 64.0);
    let direct = (albedo * n_dot_l + vec3<f32>(spec) * 0.6) * light.color;

    // Environment reflection — this is most of what sells glass.
    let r = reflect(-v, n);
    let env = textureSampleLevel(env_cube, env_sampler, r, roughness * ENV_MAX_MIP).rgb;

    // Fresnel: glass turns mirror-like at grazing angles, and its silhouette
    // becomes more opaque, which is what makes it read as a surface at all.
    let n_dot_v = max(dot(n, v), 0.0);
    let fresnel = 0.04 + 0.96 * pow(1.0 - n_dot_v, 5.0);

    let color = direct + (env * light.ibl_intensity) * (f0 + vec3<f32>(fresnel));
    let out_alpha = clamp(alpha + fresnel * (1.0 - alpha), 0.0, 1.0);
    return vec4<f32>(color, out_alpha);
}
