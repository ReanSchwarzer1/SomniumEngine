// Phase 21: forward pass for alpha-blended materials.
//
// The visibility buffer stores exactly one triangle per pixel, so it cannot
// represent see-through surfaces. Blended geometry (glTF `alphaMode: BLEND`)
// is therefore drawn here instead: a normal forward pass, after opaque shading
// has already filled the HDR target, depth-tested against the opaque depth but
// NOT writing depth, sorted back-to-front on the CPU.
//
// Shading is lighter than `shading.wgsl` but no longer blind to the scene:
// the sun, the clustered local lights (unshadowed), an IBL reflection, the
// material's normal map, and for the sorted path the scene behind the
// surface, refracted. Glass indoors is lit by lamps, not by a sun, and its
// cracks and dirt read only when those lamps catch them.

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
    porosity: f32,
    normal_scale: f32,
    // Parallax-occlusion height map and relief depth (metres); see `pool.rs`.
    height_map: i32,
    height_depth: f32,
    _hpad0: f32,
    _hpad1: f32,
    // Foliage wind response (`pool.rs` `wind`): bend, flutter; see `wind.wgsl`.
    wind_bend: f32,
    wind_flutter: f32,
    _wind_pad0: f32,
    _wind_pad1: f32,
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
    sun_angular_radius:         f32,
    _pad2_z:         f32,
    moon_direction:  vec3<f32>,
    moon_intensity:  f32,
}

@group(0) @binding(0) var<storage, read> vertices:  array<Vertex>;
@group(0) @binding(1) var<storage, read> indices:   array<u32>;
@group(0) @binding(2) var<storage, read> instances: array<Instance>;
@group(0) @binding(3) var<storage, read> view:      View;
@group(0) @binding(4) var textures:                 binding_array<texture_2d<f32>>;
@group(0) @binding(5) var<storage, read> materials: array<Material>;
@group(0) @binding(6) var<storage, read> light:     DirectionalLight;

// The clustered local lights, as `global_pool.wgsl` declares them.
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
    shading_mode: u32,
    num_local_lights: u32,
}

@group(0) @binding(7) var<storage, read> local_lights: array<GpuLocalLight>;
@group(0) @binding(8) var<storage, read> light_index_list: array<u32>;
@group(0) @binding(9) var<storage, read> cluster_offsets: array<ClusterOffset>;
@group(0) @binding(10) var<storage, read> cluster_params: ClusterParams;

@group(1) @binding(0) var tex_sampler: sampler;
@group(1) @binding(1) var env_cube:    texture_cube<f32>;
@group(1) @binding(2) var env_sampler: sampler;
// The opaque scene as it stood before this pass, for refraction.
@group(1) @binding(3) var scene_color: texture_2d<f32>;

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

/// What one blended fragment adds and what it lets through.
struct Glass {
    /// Light leaving the surface toward the eye: lit dirt and cracks (already
    /// weighted by their coverage) plus every reflection. Premultiplied.
    surface: vec3<f32>,
    /// Fraction of the scene behind that still shows, per channel.
    through: vec3<f32>,
    /// Shading normal, for the refraction lookup.
    normal: vec3<f32>,
    /// How far the normal map bends it from the mesh's own, 0..1: flat glass
    /// does not visibly refract, a crack or a ripple does.
    bend: f32,
}

/// GGX normal distribution, with the lobe floored so a point light on
/// polished glass is a small highlight rather than a single aliased pixel.
fn glass_ggx(n_dot_h: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (3.14159265 * d * d);
}

/// The same exponential depth slicing the light grid was built with
/// (`shading.wgsl` `compute_depth_slice`).
fn glass_depth_slice(view_depth: f32) -> u32 {
    let near = cluster_params.near;
    let far = cluster_params.far;
    if view_depth <= near { return 0u; }
    if view_depth >= far { return cluster_params.num_slices - 1u; }
    let slice = u32(f32(cluster_params.num_slices) * log(view_depth / near) / log(far / near));
    return min(slice, cluster_params.num_slices - 1u);
}

/// Shade one blended fragment.
///
/// MORROWIND-AC split this out of `fs_main` so the sorted path and the
/// weighted-blended path shade identically. Any difference between the two
/// images is then a difference in *compositing*, which is the thing being
/// compared — not a difference in lighting, which would make the A/B
/// meaningless.
///
/// Alpha is coverage: how much of the pixel is something opaque on the glass
/// (dirt, frost, the crushed face of a crack). That part is lit as a diffuse
/// surface. The rest is glass, which reflects by Fresnel and transmits the
/// remainder tinted by the base colour.
fn shade(in: VOut, front: bool) -> Glass {
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
    var geo_n = normalize(in.normal);
    if !front {
        geo_n = -geo_n;
    }
    var n = geo_n;
    var bend = 0.0;
    if material.normal_map >= 0 {
        // Tangent frame from screen derivatives (this is a forward pass, so
        // they are well defined). A degenerate UV map keeps the mesh normal.
        let dp1 = dpdx(in.world_pos);
        let dp2 = dpdy(in.world_pos);
        let duv1 = dpdx(in.uv);
        let duv2 = dpdy(in.uv);
        let det = duv1.x * duv2.y - duv2.x * duv1.y;
        let t_raw = (dp1 * duv2.y - dp2 * duv1.y) * sign(det);
        let t_ortho = t_raw - geo_n * dot(t_raw, geo_n);
        if abs(det) > 1.0e-12 && dot(t_ortho, t_ortho) > 1.0e-16 {
            let t = normalize(t_ortho);
            let b = cross(geo_n, t) * sign(det);
            let tn = textureSample(textures[material.normal_map], tex_sampler, in.uv).xyz * 2.0 - 1.0;
            n = normalize(t * tn.x * material.normal_scale + b * tn.y * material.normal_scale + geo_n * tn.z);
            bend = saturate(length(n - geo_n) * 2.0);
        }
    }
    let v = normalize(view.camera_pos - in.world_pos);
    let n_dot_v = max(dot(n, v), 1.0e-3);
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);
    // Fresnel: glass turns mirror-like at grazing angles, and its silhouette
    // becomes more opaque, which is what makes it read as a surface at all.
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - n_dot_v, 5.0);
    let alpha_ggx = max(roughness * roughness, 0.004);

    var diffuse = vec3<f32>(0.0);
    var specular = vec3<f32>(0.0);

    // Direct sun, no shadow lookup: the shadow atlas is bound to the shading
    // pass's group and glass rarely reads as shadowed anyway.
    let sun = normalize(light.direction);
    let sun_n_dot_l = max(dot(n, sun), 0.0);
    if sun_n_dot_l > 0.0 {
        let h = normalize(v + sun);
        diffuse += light.color * sun_n_dot_l / 3.14159265;
        specular += light.color * sun_n_dot_l
            * min(glass_ggx(max(dot(n, h), 0.0), max(alpha_ggx, 0.02)) * 0.25, 40.0);
    }

    // The lamps. Unshadowed: a pane is small and its occluders are rarely
    // between it and the fixture that lights the room it is in.
    if cluster_params.num_local_lights > 0u {
        let view_depth = max(-(view.view * vec4<f32>(in.world_pos, 1.0)).z, 0.0);
        let tile = vec2<u32>(in.clip.xy) / vec2(cluster_params.tile_size);
        let froxel = min(tile.x, cluster_params.grid_width - 1u)
            + min(tile.y, cluster_params.grid_height - 1u) * cluster_params.grid_width
            + glass_depth_slice(view_depth) * cluster_params.grid_width * cluster_params.grid_height;
        let cluster = cluster_offsets[froxel];
        for (var i = 0u; i < cluster.count; i++) {
            let ll = local_lights[light_index_list[cluster.offset + i]];
            var to_light = ll.position_ws - in.world_pos;
            // The emitter's size as seen from here widens the highlight.
            var size = max(ll.radius, 0.02);
            if ll.light_type == 4u {
                // A tube: the nearest point of its axis.
                let axis = normalize(ll.direction_ws);
                let along = clamp(dot(in.world_pos - ll.position_ws, axis), -ll._pad1, ll._pad1);
                to_light = ll.position_ws + axis * along - in.world_pos;
            } else if ll.light_type == 2u {
                size = sqrt(max(ll._pad1 * ll._pad2, 0.0025));
            }
            let dist = length(to_light);
            if dist > ll.range || dist < 1.0e-4 {
                continue;
            }
            let l = to_light / dist;
            let ratio = dist / ll.range;
            let window = saturate(1.0 - ratio * ratio * ratio * ratio);
            var atten = window * window / max(dist * dist, 0.0064);
            if ll.light_type == 1u {
                atten *= smoothstep(ll.spot_cos_outer, ll.spot_cos_inner, dot(-l, normalize(ll.direction_ws)));
            } else if ll.light_type == 2u || ll.light_type == 3u {
                // Panels and discs emit from one face.
                atten *= saturate(dot(-l, normalize(ll.direction_ws)) * 4.0);
            }
            let n_dot_l = dot(n, l);
            if n_dot_l <= 0.0 || atten <= 0.0 {
                continue;
            }
            let h = normalize(v + l);
            let lobe = clamp(alpha_ggx + size / (2.0 * dist), 0.0, 1.0);
            diffuse += ll.color * atten * n_dot_l / 3.14159265;
            // Energy kept when the lobe is widened for the emitter's size.
            let widen = (alpha_ggx * alpha_ggx) / (lobe * lobe);
            specular += ll.color * atten * n_dot_l
                * min(glass_ggx(max(dot(n, h), 0.0), lobe) * widen * 0.25, 40.0);
        }
    }

    // Environment: a mirror direction for the glass, the blurriest mip as the
    // ambient that lights whatever sits on it.
    let r = reflect(-v, n);
    let env = textureSampleLevel(env_cube, env_sampler, r, roughness * ENV_MAX_MIP).rgb * light.ibl_intensity;
    diffuse += textureSampleLevel(env_cube, env_sampler, n, ENV_MAX_MIP).rgb * light.ibl_intensity;

    var out: Glass;
    out.surface = albedo * (1.0 - metallic) * diffuse * alpha + (env + specular) * fresnel;
    // Clear glass transmits what it does not reflect, tinted; coverage blocks.
    let tint = mix(vec3<f32>(1.0), material.base_color.rgb, 0.5);
    out.through = (vec3<f32>(1.0) - fresnel) * (1.0 - alpha) * tint;
    out.normal = n;
    out.bend = bend;
    return out;
}

/// The sorted path: the scene behind the surface is read from the copy taken
/// before this pass, so glass whose normal map bends its normal refracts
/// what is behind it. The pipeline's alpha blend then only has to keep the
/// unrefracted share of the destination.
@fragment
fn fs_main(in: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let g = shade(in, front);
    let through = dot(g.through, vec3<f32>(0.3333));
    if g.bend <= 0.0 {
        // Flat glass: let the blend show the destination itself, which keeps
        // panes behind this one (drawn earlier in the sort) visible.
        let a = 1.0 - through;
        return vec4<f32>(g.surface / max(a, 1.0e-3), a);
    }
    // Thin-slab refraction: the ray bends toward the normal on entry and
    // meets the scene a little to the side of where it was heading.
    let v = normalize(view.camera_pos - in.world_pos);
    let bent = refract(-v, g.normal, 1.0 / 1.5);
    let exit = view.view_proj * vec4<f32>(in.world_pos + (bent + v) * 0.12, 1.0);
    let straight = view.view_proj * vec4<f32>(in.world_pos, 1.0);
    let dims = vec2<f32>(textureDimensions(scene_color));
    let shift = (exit.xy / exit.w - straight.xy / straight.w) * vec2<f32>(0.5, -0.5);
    let uv = clamp(in.clip.xy / dims + shift, vec2<f32>(0.001), vec2<f32>(0.999));
    let behind = textureSampleLevel(scene_color, tex_sampler, uv, 0.0).rgb;
    // Refracted where the normal bends, plain blending where it does not.
    let refracted = g.surface + behind * g.through;
    let a = mix(1.0 - through, 1.0, g.bend);
    let rgb = mix(g.surface, refracted, g.bend);
    return vec4<f32>(rgb / max(a, 1.0e-3), a);
}

// ─────────────────────────────────────────────────────────────────────────────
// Weighted-blended OIT (MORROWIND-AC)
//
// McGuire and Bavoil, *Weighted Blended Order-Independent Transparency*
// (JCGT 2013). Two targets instead of one blended pass:
//
//   accum  += (rgb * a, a) * w(z, a)      additive
//   reveal *= (1 - a)                     multiplicative
//
// and the composite resolves `accum.rgb / accum.a` against `reveal`. Nothing
// depends on draw order, so intersecting surfaces stop depending on which
// object's origin happened to be nearer — which is the failure the sorted path
// cannot fix, because a per-object key cannot order a per-pixel question.
// ─────────────────────────────────────────────────────────────────────────────

struct OitOut {
    @location(0) accum: vec4<f32>,
    @location(1) reveal: f32,
}

/// The paper's weight function, equation 9.
///
/// Two jobs, and they pull against each other: near-camera fragments must
/// dominate far ones, and the whole thing must stay inside `f16` range or the
/// accumulation buffer saturates to `inf` and the pixel resolves to NaN. The
/// `1e-5 .. 3e3` clamp is what keeps a distant, nearly-transparent fragment
/// from underflowing to zero and a close, nearly-opaque one from overflowing.
fn oit_weight(z: f32, a: f32) -> f32 {
    let d = 1.0 - z;
    return clamp(pow(a + 0.01, 4.0) + max(0.01, 3.0e3 * d * d * d), 1.0e-5, 3.0e3);
}

@fragment
fn fs_oit(in: VOut, @builtin(front_facing) front: bool) -> OitOut {
    let g = shade(in, front);
    // No refraction here: order independence has no "scene behind" to read.
    let a = 1.0 - dot(g.through, vec3<f32>(0.3333));
    // `in.clip.z` is the post-projection depth wgpu hands the fragment, already
    // in 0..1, so no view-space reconstruction is needed here.
    let w = oit_weight(in.clip.z, a);
    var out: OitOut;
    out.accum = vec4<f32>(g.surface, a) * w;
    out.reveal = a;
    return out;
}
