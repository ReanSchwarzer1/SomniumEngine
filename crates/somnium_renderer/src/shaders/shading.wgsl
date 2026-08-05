// Somnium Engine — Visibility Buffer Shading Pass
// Phase 12: Clustered Local Lights + Cel-Shading Mode

// ─── Shared structs ─────────────────────────────────────────────────────────

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
    _pad0: i32,
    _pad1: i32,
    _pad2: i32,
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

// GpuDirectionalLight (320 bytes) — matches shadow/mod.rs::GpuDirectionalLight.
struct DirectionalLight {
    direction:       vec3<f32>,               // offset   0
    _pad0:           f32,                     // offset  12
    color:           vec3<f32>,               // offset  16  pre-multiplied by intensity
    _pad1:           f32,                     // offset  28
    view_proj:       array<mat4x4<f32>, 4>,   // offset  32  (256 bytes)
    cascade_splits:  vec4<f32>,               // offset 288  view-space far Z per cascade
    shadow_map_size: f32,                     // offset 304  total atlas texels (4096)
    ibl_intensity:   f32,                     // offset 308  Phase 22C: editable indirect strength
    _pad2_y:         f32,                     // offset 312
    _pad2_z:         f32,                     // offset 316
}

struct GpuLocalLight {
    position_ws: vec3<f32>,
    range: f32,
    color: vec3<f32>,
    light_type: u32,
    direction_ws: vec3<f32>,
    spot_cos_outer: f32,
    spot_cos_inner: f32,
    _pad0: f32,
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

@group(1) @binding(0) var vis_buffer:      texture_2d<u32>;
@group(1) @binding(1) var default_sampler: sampler;
@group(1) @binding(2) var shadow_atlas:    texture_depth_2d;
@group(1) @binding(3) var shadow_sampler:  sampler_comparison;
// Phase 19: prefiltered environment cubemap. Mip i holds radiance convolved
// for roughness i / ENV_MAX_MIP.
@group(1) @binding(4) var env_cube:    texture_cube<f32>;
@group(1) @binding(5) var env_sampler: sampler;

/// Highest mip index of the environment map (must match `IblPass::MIP_COUNT - 1`).
const ENV_MAX_MIP: f32 = 5.0;

/// Scale applied to image-based ambient.
///
/// Physically this should be 1.0, but the engine has no ambient occlusion yet,
/// so sky light reaches every surface unattenuated — including the insides of
/// creases and anything sitting in the sun's shadow. At full strength that
/// washes shadows out badly. Until SSAO (or a glTF occlusion map) lands, the
/// indirect term is scaled back so shadow contrast survives.


/// Analytic fit to the split-sum BRDF integration term (Karis' mobile
/// approximation, via Lazarov). Avoids shipping and binding a 2-D LUT for what
/// is a smooth two-parameter function.
fn env_brdf_approx(f0: vec3<f32>, roughness: f32, n_dot_v: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

/// Image-based ambient: diffuse irradiance + split-sum specular.
fn evaluate_ibl(surface: Surface) -> vec3<f32> {
    let n = surface.normal;
    let v = surface.view_dir;
    let n_dot_v = max(dot(n, v), 1e-4);

    // Diffuse: the roughest mip approximates a cosine-convolved irradiance
    // map. Not a true convolution, but close enough visually and it saves a
    // whole extra prefilter chain.
    let irradiance = textureSampleLevel(env_cube, env_sampler, n, ENV_MAX_MIP).rgb;
    let kd = (vec3<f32>(1.0) - surface.f0) * (1.0 - surface.metallic);
    let diffuse = irradiance * surface.albedo * kd;

    // Specular: prefiltered radiance along the reflection vector, weighted by
    // the analytic BRDF term.
    let r = reflect(-v, n);
    let mip = surface.roughness * ENV_MAX_MIP;
    let prefiltered = textureSampleLevel(env_cube, env_sampler, r, mip).rgb;
    let specular = prefiltered * env_brdf_approx(surface.f0, surface.roughness, n_dot_v);

    return (diffuse + specular) * light.ibl_intensity;
}

// ─── Vertex shader ───────────────────────────────────────────────────────────

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((in_vertex_index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(in_vertex_index & 2u) * 2.0 - 1.0;
    out.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, 1.0 - (y * 0.5 + 0.5));
    return out;
}

// ─── Shadow helpers ──────────────────────────────────────────────────────────

// Returns the cascade index (0..3) for a given positive view-space depth.
fn get_cascade_index(view_depth: f32) -> u32 {
    if view_depth < light.cascade_splits.x { return 0u; }
    if view_depth < light.cascade_splits.y { return 1u; }
    if view_depth < light.cascade_splits.z { return 2u; }
    return 3u;
}

// Maps a per-cascade UV in [0,1] into the corresponding atlas quadrant UV.
fn atlas_uv(cascade: u32, uv: vec2<f32>) -> vec2<f32> {
    let offsets = array<vec2<f32>, 4>(
        vec2(0.0, 0.0),
        vec2(0.5, 0.0),
        vec2(0.0, 0.5),
        vec2(0.5, 0.5),
    );
    return uv * 0.5 + offsets[cascade];
}

// 3×3 PCF shadow factor in [0,1]; 1.0 = fully lit, 0.0 = fully in shadow.
fn sample_shadow(world_pos: vec3<f32>, view_depth: f32) -> f32 {
    // Select cascade from view-space depth (positive = in front of camera).
    let cascade      = get_cascade_index(view_depth);
    let light_clip   = light.view_proj[cascade] * vec4<f32>(world_pos, 1.0);
    let ndc          = light_clip.xyz / light_clip.w;

    // NDC → UV. Flip Y because wgpu's texture V-axis is top-down.
    let uv           = vec2<f32>(ndc.x * 0.5 + 0.5, 1.0 - (ndc.y * 0.5 + 0.5));
    let atlas_coord  = atlas_uv(cascade, uv);
    let compare_depth = ndc.z; // orthographic: w=1, so ndc.z is already in [0,1]

    // Early-out: position outside the shadow frustum → fully lit.
    if any(atlas_coord < vec2(0.0)) || any(atlas_coord > vec2(1.0)) || compare_depth > 1.0 {
        return 1.0;
    }

    // 3×3 PCF kernel. texel_size in atlas UV space = 1/4096 (one shadow texel).
    let texel_size = 1.0 / light.shadow_map_size;
    var shadow = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
            shadow += textureSampleCompare(
                shadow_atlas, shadow_sampler,
                atlas_coord + offset, compare_depth,
            );
        }
    }
    return shadow / 9.0;
}

// ─── Clustered lighting helpers ──────────────────────────────────────────────

// UE4/5 physically-based inverse-square attenuation with smooth cutoff
fn smooth_distance_attenuation(dist: f32, range: f32) -> f32 {
    let ratio = dist / range;
    let ratio2 = ratio * ratio;
    let ratio4 = ratio2 * ratio2;
    let factor = saturate(1.0 - ratio4);
    return (factor * factor) / max(dist * dist, 0.0001);
}

// Exponential depth slice (matches CPU side)
fn compute_depth_slice(view_depth: f32) -> u32 {
    let near = cluster_params.near;
    let far = cluster_params.far;
    if view_depth <= near { return 0u; }
    if view_depth >= far { return cluster_params.num_slices - 1u; }
    let log_ratio = log(far / near);
    let slice = u32(f32(cluster_params.num_slices) * log(view_depth / near) / log_ratio);
    return min(slice, cluster_params.num_slices - 1u);
}

// ─── Fragment shader ─────────────────────────────────────────────────────────

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let pixel_coords = vec2<i32>(in.clip_pos.xy);
    let vis_data     = textureLoad(vis_buffer, pixel_coords, 0).r;

    // ── Sky / background ────────────────────────────────────────────────────
    if vis_data == 0u {
        let ndc = (in.uv * 2.0 - 1.0) * vec2<f32>(1.0, -1.0);
        let near_plane = view.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
        let far_plane  = view.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
        let ray_dir    = normalize(far_plane.xyz / far_plane.w - near_plane.xyz / near_plane.w);

        let up            = ray_dir.y;
        let horizon_color = vec3<f32>(0.5, 0.7, 0.9);
        let zenith_color  = vec3<f32>(0.05, 0.1, 0.3);
        let ground_color  = vec3<f32>(0.05, 0.04, 0.03);

        var sky = mix(horizon_color, zenith_color, pow(max(up, 0.0), 0.5));
        sky = mix(sky, ground_color, exp(-max(-up, 0.0) * 10.0));

        let sun_dir = normalize(light.direction);
        let sun_dot = max(dot(ray_dir, sun_dir), 0.0);
        let sun     = pow(sun_dot, 1024.0) * vec3<f32>(10.0, 8.0, 5.0);
        let glow    = pow(sun_dot, 64.0)   * vec3<f32>(0.5, 0.4, 0.2);

        return vec4<f32>(sky + sun + glow, 1.0);
    }

    // ── PBR surface ─────────────────────────────────────────────────────────
    // Phase 15C: 16/16 split (see visibility.wgsl for the packing).
    let instance_id = (vis_data >> 16u) - 1u;
    let prim_id     = vis_data & 0xFFFFu;

    let instance = instances[instance_id];
    let material = materials[instance.material_id];

    let i0 = indices[instance.index_offset + prim_id * 3u + 0u];
    let i1 = indices[instance.index_offset + prim_id * 3u + 1u];
    let i2 = indices[instance.index_offset + prim_id * 3u + 2u];

    let v0 = vertices[instance.vertex_offset + i0];
    let v1 = vertices[instance.vertex_offset + i1];
    let v2 = vertices[instance.vertex_offset + i2];

    let p0 = (instance.model * vec4<f32>(v0.pos_x, v0.pos_y, v0.pos_z, 1.0)).xyz;
    let p1 = (instance.model * vec4<f32>(v1.pos_x, v1.pos_y, v1.pos_z, 1.0)).xyz;
    let p2 = (instance.model * vec4<f32>(v2.pos_x, v2.pos_y, v2.pos_z, 1.0)).xyz;

    let c0 = view.view_proj * instance.model * vec4<f32>(v0.pos_x, v0.pos_y, v0.pos_z, 1.0);
    let c1 = view.view_proj * instance.model * vec4<f32>(v1.pos_x, v1.pos_y, v1.pos_z, 1.0);
    let c2 = view.view_proj * instance.model * vec4<f32>(v2.pos_x, v2.pos_y, v2.pos_z, 1.0);

    let ndc0 = c0.xy / c0.w;
    let ndc1 = c1.xy / c1.w;
    let ndc2 = c2.xy / c2.w;

    let target_ndc = (in.uv * 2.0 - 1.0) * vec2<f32>(1.0, -1.0);
    let det        = (ndc1.y - ndc2.y) * (ndc0.x - ndc2.x) + (ndc2.x - ndc1.x) * (ndc0.y - ndc2.y);
    let w0 = ((ndc1.y - ndc2.y) * (target_ndc.x - ndc2.x) + (ndc2.x - ndc1.x) * (target_ndc.y - ndc2.y)) / det;
    let w1 = ((ndc2.y - ndc0.y) * (target_ndc.x - ndc2.x) + (ndc0.x - ndc2.x) * (target_ndc.y - ndc2.y)) / det;
    let w2 = 1.0 - w0 - w1;

    var bary = vec3<f32>(w0 / c0.w, w1 / c1.w, w2 / c2.w);
    bary = bary / (bary.x + bary.y + bary.z);

    let uv = vec2<f32>(v0.u, v0.v) * bary.x
           + vec2<f32>(v1.u, v1.v) * bary.y
           + vec2<f32>(v2.u, v2.v) * bary.z;

    let normal_interp = normalize(
        vec3<f32>(v0.norm_x, v0.norm_y, v0.norm_z) * bary.x +
        vec3<f32>(v1.norm_x, v1.norm_y, v1.norm_z) * bary.y +
        vec3<f32>(v2.norm_x, v2.norm_y, v2.norm_z) * bary.z
    );
    let geo_normal = normalize((instance.model * vec4<f32>(normal_interp, 0.0)).xyz);

    let hit_point = p0 * bary.x + p1 * bary.y + p2 * bary.z;

    // TBN matrix (derived from edge vectors + UV deltas, no vertex tangents)
    let edge0 = p1 - p0;
    let edge1 = p2 - p0;
    let uv0   = vec2<f32>(v0.u, v0.v);
    let uv1   = vec2<f32>(v1.u, v1.v);
    let uv2   = vec2<f32>(v2.u, v2.v);
    let duv0  = uv1 - uv0;
    let duv1  = uv2 - uv0;

    let tbn_det  = duv0.x * duv1.y - duv1.x * duv0.y;
    let inv_det  = 1.0 / (tbn_det + 1e-7);
    var tangent  = normalize((edge0 * duv1.y - edge1 * duv0.y) * inv_det);
    tangent      = normalize(tangent - dot(tangent, geo_normal) * geo_normal);
    let bitangent = cross(geo_normal, tangent);
    let tbn       = mat3x3<f32>(tangent, bitangent, geo_normal);

    // PBR surface setup
    var surface: Surface;
    surface.albedo    = material.base_color.rgb;
    if material.albedo_map >= 0 {
        surface.albedo *= textureSample(textures[material.albedo_map], default_sampler, uv).rgb;
    }

    surface.roughness = max(material.roughness, 0.05);
    surface.metallic  = material.metallic;
    if material.metallic_roughness_map >= 0 {
        let mr = textureSample(textures[material.metallic_roughness_map], default_sampler, uv);
        surface.roughness = max(mr.g, 0.05);
        surface.metallic  = mr.b;
    }

    surface.normal = geo_normal;
    if material.normal_map >= 0 {
        let nm_sample  = textureSample(textures[material.normal_map], default_sampler, uv).rgb;
        let tangent_n  = nm_sample * 2.0 - vec3<f32>(1.0);
        surface.normal = normalize(tbn * tangent_n);
    }

    surface.view_dir = normalize(view.camera_pos - hit_point);
    surface.f0       = mix(vec3<f32>(0.04), surface.albedo, surface.metallic);

    // ── Shadow factor ────────────────────────────────────────────────────────
    // View-space depth: positive Z distance from camera.
    let view_pos   = view.view * vec4<f32>(hit_point, 1.0);
    let view_depth = -view_pos.z; // right-handed: Z is negative in front of camera

    let shadow_factor = sample_shadow(hit_point, view_depth);

    // ── Shading ───────────────────────────────────────────────────────────────
    var result: vec3<f32>;

    if cluster_params.shading_mode == 1u {
        // ── Cel-shading path ─────────────────────────────────────────────────
        let NdotL = max(dot(surface.normal, normalize(light.direction)), 0.0);

        // Quantize into 3 discrete bands
        var cel_factor: f32;
        if NdotL > 0.7 { cel_factor = 1.0; }
        else if NdotL > 0.3 { cel_factor = 0.55; }
        else { cel_factor = 0.2; }

        // Apply shadow
        cel_factor *= shadow_factor;

        // Rim highlight (silhouette edge glow)
        let NdotV = max(dot(surface.normal, surface.view_dir), 0.0);
        let rim = 1.0 - NdotV;
        let rim_factor = rim * rim * rim * rim;
        let rim_color = surface.albedo * 0.4;

        // Directional light contribution
        result = surface.albedo * light.color * cel_factor + rim_color * rim_factor;

        // Local lights with cel treatment
        if cluster_params.num_local_lights > 0u {
            let frag_coord_cel = vec2<u32>(in.clip_pos.xy);
            let tile_cel = frag_coord_cel / vec2(cluster_params.tile_size);
            let depth_slice_cel = compute_depth_slice(view_depth);
            let grid_w_cel = cluster_params.grid_width;
            let grid_h_cel = cluster_params.grid_height;
            let froxel_idx_cel = tile_cel.x + tile_cel.y * grid_w_cel + depth_slice_cel * grid_w_cel * grid_h_cel;

            let cluster_cel = cluster_offsets[froxel_idx_cel];
            for (var j = 0u; j < cluster_cel.count; j++) {
                let ll_idx = light_index_list[cluster_cel.offset + j];
                let ll = local_lights[ll_idx];

                let lv = ll.position_ws - hit_point;
                let d = length(lv);
                if d > ll.range { continue; }

                let Ll = lv / d;
                var atten = smooth_distance_attenuation(d, ll.range);
                if ll.light_type == 1u {
                    let ca = dot(-Ll, normalize(ll.direction_ws));
                    atten *= smoothstep(ll.spot_cos_outer, ll.spot_cos_inner, ca);
                }

                let local_NdotL = max(dot(surface.normal, Ll), 0.0);
                var local_cel: f32;
                if local_NdotL > 0.7 { local_cel = 1.0; }
                else if local_NdotL > 0.3 { local_cel = 0.55; }
                else { local_cel = 0.2; }

                result += surface.albedo * ll.color * local_cel * atten;
            }
        }

        // Cel shading keeps a flat ambient on purpose: environment reflections
        // would fight the deliberately flat, banded look.
        result += 0.03 * surface.albedo;
    } else {
        // ── PBR path (existing) ──────────────────────────────────────────────
        let light_dir   = normalize(light.direction);
        let light_color = light.color;
        let direct_light = evaluate_brdf(surface, light_dir) * light_color * shadow_factor;
        // Phase 19: real environment lighting instead of a flat 3% fudge —
        // this is what lets metals reflect the sky.
        let ambient = evaluate_ibl(surface);

        // Local lights (clustered)
        var local_light_contrib = vec3<f32>(0.0);
        if cluster_params.num_local_lights > 0u {
            let frag_coord = vec2<u32>(in.clip_pos.xy);
            let tile = frag_coord / vec2(cluster_params.tile_size);
            let depth_slice_pbr = compute_depth_slice(view_depth);
            let grid_w = cluster_params.grid_width;
            let grid_h = cluster_params.grid_height;
            let froxel_idx = tile.x + tile.y * grid_w + depth_slice_pbr * grid_w * grid_h;

            let cluster_data = cluster_offsets[froxel_idx];
            for (var i = 0u; i < cluster_data.count; i++) {
                let light_idx = light_index_list[cluster_data.offset + i];
                let ll = local_lights[light_idx];

                let light_vec = ll.position_ws - hit_point;
                let dist = length(light_vec);
                if dist > ll.range { continue; }

                let L = light_vec / dist;
                var atten_val = smooth_distance_attenuation(dist, ll.range);
                if ll.light_type == 1u {
                    let cos_angle = dot(-L, normalize(ll.direction_ws));
                    atten_val *= smoothstep(ll.spot_cos_outer, ll.spot_cos_inner, cos_angle);
                }

                local_light_contrib += evaluate_brdf(surface, L) * ll.color * atten_val;
            }
        }

        result = direct_light + local_light_contrib + ambient;
    }

    // ── Cascade debug overlay (controlled by _padding repurposed as a flag) ──
    // When view._padding == 1.0 (set from Rust via set_cascade_debug), tint by cascade.
    if view._padding > 0.5 {
        let cascade = get_cascade_index(view_depth);
        let tints = array<vec3<f32>, 4>(
            vec3(1.0, 0.3, 0.3), // cascade 0 → red
            vec3(0.3, 1.0, 0.3), // cascade 1 → green
            vec3(0.3, 0.3, 1.0), // cascade 2 → blue
            vec3(1.0, 1.0, 0.3), // cascade 3 → yellow
        );
        result = mix(result, tints[cascade], 0.5);
    }

    return vec4<f32>(result, 1.0);
}
