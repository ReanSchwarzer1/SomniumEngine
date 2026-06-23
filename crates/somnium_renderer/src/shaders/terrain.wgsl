// Somnium Engine — Heightmap Terrain Pass (Phase 14C)
//
// Concatenated after brdf.wgsl (Surface + evaluate_brdf).
//
// Splatmap-weighted blending of 4 PBR layers (texture arrays), height-based
// blend sharpening, triplanar cliff projection on steep slopes, and the same
// CSM shadows + clustered local lights as the visibility shading pass.
//
// References:
// - bevy_triplanar_splatting (example_repo/bevy-plugins/) — array-texture
//   splat sampling + triplanar weight blending.
// - shading.wgsl — shadow / cluster helpers (duplicated; passes are separate
//   pipelines and WGSL has no #include).

// ─── Shared structs (must match shading.wgsl layouts) ───────────────────────

struct View {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    view:          mat4x4<f32>,
    camera_pos:    vec3<f32>,
    _padding:      f32,
    time:          f32,
}

struct DirectionalLight {
    direction:       vec3<f32>,
    _pad0:           f32,
    color:           vec3<f32>,
    _pad1:           f32,
    view_proj:       array<mat4x4<f32>, 4>,
    cascade_splits:  vec4<f32>,
    shadow_map_size: f32,
    _pad2_x:         f32,
    _pad2_y:         f32,
    _pad2_z:         f32,
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

// TerrainParams — matches terrain::GpuTerrainParams (80 bytes).
struct TerrainParams {
    layer_tiling: vec4<f32>,     // UV repeats per metre, one per layer
    brush: vec4<f32>,            // xy = world XZ, z = radius, w = mode (0/1/2)
    terrain_origin: vec2<f32>,   // world XZ of terrain-local (0, 0)
    inv_world_size: vec2<f32>,
    cliff_layer: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

// ─── Bindings ────────────────────────────────────────────────────────────────

@group(0) @binding(0) var<storage, read> view: View;
@group(0) @binding(1) var<storage, read> light: DirectionalLight;
@group(0) @binding(2) var shadow_atlas: texture_depth_2d;
@group(0) @binding(3) var shadow_sampler: sampler_comparison;
@group(0) @binding(4) var<storage, read> local_lights: array<GpuLocalLight>;
@group(0) @binding(5) var<storage, read> light_index_list: array<u32>;
@group(0) @binding(6) var<storage, read> cluster_offsets: array<ClusterOffset>;
@group(0) @binding(7) var<storage, read> cluster_params: ClusterParams;

@group(1) @binding(0) var<uniform> params: TerrainParams;
@group(1) @binding(1) var<uniform> model: mat4x4<f32>;
@group(1) @binding(2) var splatmap: texture_2d<f32>;
@group(1) @binding(3) var layer_albedo: texture_2d_array<f32>;
@group(1) @binding(4) var layer_normal: texture_2d_array<f32>;
@group(1) @binding(5) var layer_roughness: texture_2d_array<f32>;
@group(1) @binding(6) var terrain_sampler: sampler;

// ─── Vertex shader ───────────────────────────────────────────────────────────

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,          // terrain-global [0,1] splat UV
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    let world = model * vec4<f32>(position, 1.0);
    out.world_pos = world.xyz;
    out.normal = normalize((model * vec4<f32>(normal, 0.0)).xyz);
    out.uv = uv;
    out.clip_pos = view.view_proj * world;
    return out;
}

// ─── Shadow helpers (same algorithm as shading.wgsl) ─────────────────────────

fn get_cascade_index(view_depth: f32) -> u32 {
    if view_depth < light.cascade_splits.x { return 0u; }
    if view_depth < light.cascade_splits.y { return 1u; }
    if view_depth < light.cascade_splits.z { return 2u; }
    return 3u;
}

fn atlas_uv(cascade: u32, uv: vec2<f32>) -> vec2<f32> {
    let offsets = array<vec2<f32>, 4>(
        vec2(0.0, 0.0), vec2(0.5, 0.0), vec2(0.0, 0.5), vec2(0.5, 0.5),
    );
    return uv * 0.5 + offsets[cascade];
}

fn sample_shadow(world_pos: vec3<f32>, view_depth: f32) -> f32 {
    let cascade = get_cascade_index(view_depth);
    let light_clip = light.view_proj[cascade] * vec4<f32>(world_pos, 1.0);
    let ndc = light_clip.xyz / light_clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 1.0 - (ndc.y * 0.5 + 0.5));
    let atlas_coord = atlas_uv(cascade, uv);
    let compare_depth = ndc.z;

    if any(atlas_coord < vec2(0.0)) || any(atlas_coord > vec2(1.0)) || compare_depth > 1.0 {
        return 1.0;
    }

    let texel_size = 1.0 / light.shadow_map_size;
    var shadow = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
            shadow += textureSampleCompare(
                shadow_atlas, shadow_sampler, atlas_coord + offset, compare_depth,
            );
        }
    }
    return shadow / 9.0;
}

// ─── Clustered lighting helpers (same algorithm as shading.wgsl) ─────────────

fn smooth_distance_attenuation(dist: f32, range: f32) -> f32 {
    let ratio = dist / range;
    let ratio2 = ratio * ratio;
    let ratio4 = ratio2 * ratio2;
    let factor = saturate(1.0 - ratio4);
    return (factor * factor) / max(dist * dist, 0.0001);
}

fn compute_depth_slice(view_depth: f32) -> u32 {
    let near = cluster_params.near;
    let far = cluster_params.far;
    if view_depth <= near { return 0u; }
    if view_depth >= far { return cluster_params.num_slices - 1u; }
    let log_ratio = log(far / near);
    let slice = u32(f32(cluster_params.num_slices) * log(view_depth / near) / log_ratio);
    return min(slice, cluster_params.num_slices - 1u);
}

// ─── Layer sampling ──────────────────────────────────────────────────────────

struct LayerSample {
    albedo: vec3<f32>,
    height: f32,        // albedo alpha = procedural height (for height blend)
    normal_ts: vec3<f32>,
    roughness: f32,
}

fn sample_layer(layer: u32, uv: vec2<f32>) -> LayerSample {
    var s: LayerSample;
    let a = textureSample(layer_albedo, terrain_sampler, uv, layer);
    s.albedo = a.rgb;
    s.height = a.a;
    s.normal_ts = textureSample(layer_normal, terrain_sampler, uv, layer).rgb * 2.0 - 1.0;
    s.roughness = textureSample(layer_roughness, terrain_sampler, uv, layer).r;
    return s;
}

// Height-based blend sharpening (Phase 14C-3): materials with greater
// procedural height "poke through" at transitions instead of soft-mixing.
fn height_blend(weights: vec4<f32>, heights: vec4<f32>) -> vec4<f32> {
    let blend_sharpness = 0.25;
    let adjusted = heights * 0.4 + weights;
    let max_h = max(max(adjusted.x, adjusted.y), max(adjusted.z, adjusted.w));
    let threshold = max_h - blend_sharpness;
    let cut = max(adjusted - vec4(threshold), vec4(0.0)) * step(vec4(0.001), weights);
    return cut / max(cut.x + cut.y + cut.z + cut.w, 0.0001);
}

// Triplanar sample of one layer (bevy_triplanar_splatting pattern):
// project along the three axes and blend by a sharpened normal weight.
fn triplanar_albedo(layer: u32, world_pos: vec3<f32>, n: vec3<f32>, tiling: f32) -> vec3<f32> {
    var w = pow(abs(n), vec3(4.0));
    w = w / (w.x + w.y + w.z);
    let cx = textureSample(layer_albedo, terrain_sampler, world_pos.zy * tiling, layer).rgb;
    let cy = textureSample(layer_albedo, terrain_sampler, world_pos.xz * tiling, layer).rgb;
    let cz = textureSample(layer_albedo, terrain_sampler, world_pos.xy * tiling, layer).rgb;
    return cx * w.x + cy * w.y + cz * w.z;
}

// ─── Fragment shader ─────────────────────────────────────────────────────────

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let geo_normal = normalize(in.normal);

    // 1. Splat weights for this fragment.
    var weights = textureSample(splatmap, terrain_sampler, in.uv);
    weights = weights / max(weights.x + weights.y + weights.z + weights.w, 0.0001);

    // 2-3. Sample all four layers at their tiled UVs; gather heights, then
    // sharpen the weights with the height blend and accumulate PBR inputs.
    // (Texture-array sampling must be uniform, so all layers are sampled.)
    let local_xz = in.world_pos.xz - params.terrain_origin;
    let s0 = sample_layer(0u, local_xz * params.layer_tiling.x);
    let s1 = sample_layer(1u, local_xz * params.layer_tiling.y);
    let s2 = sample_layer(2u, local_xz * params.layer_tiling.z);
    let s3 = sample_layer(3u, local_xz * params.layer_tiling.w);

    let w = height_blend(weights, vec4(s0.height, s1.height, s2.height, s3.height));

    var albedo = s0.albedo * w.x + s1.albedo * w.y + s2.albedo * w.z + s3.albedo * w.w;
    let normal_ts = normalize(
        s0.normal_ts * w.x + s1.normal_ts * w.y + s2.normal_ts * w.z + s3.normal_ts * w.w,
    );
    var roughness = s0.roughness * w.x + s1.roughness * w.y + s2.roughness * w.z + s3.roughness * w.w;

    // 4. Triplanar cliff projection on steep slopes (Phase 14C step 4).
    // Sampled unconditionally — textureSample needs uniform control flow.
    let steepness = 1.0 - abs(geo_normal.y);
    let cliff_blend = smoothstep(0.45, 0.7, steepness);
    let cliff = triplanar_albedo(
        params.cliff_layer, in.world_pos - vec3(params.terrain_origin.x, 0.0, params.terrain_origin.y),
        geo_normal, params.layer_tiling[params.cliff_layer],
    );
    albedo = mix(albedo, cliff, cliff_blend);
    roughness = mix(roughness, 0.8, cliff_blend);

    // Tangent basis for an up-facing heightfield: tangent along +X projected
    // onto the surface (terrain UVs are axis-aligned by construction).
    let tangent = normalize(vec3<f32>(1.0, 0.0, 0.0) - geo_normal * geo_normal.x);
    let bitangent = cross(geo_normal, tangent);
    let tbn = mat3x3<f32>(tangent, bitangent, geo_normal);
    // Generated normal maps use Z-up tangent space; remap to TBN (x→T, y→B).
    let shading_normal = normalize(tbn * normalize(vec3(normal_ts.x, normal_ts.y, max(normal_ts.z, 0.2))));

    // 5. PBR lighting — same BRDF + shadow + cluster path as shading.wgsl.
    var surface: Surface;
    surface.albedo = albedo;
    surface.roughness = max(roughness, 0.05);
    surface.metallic = 0.0;
    surface.normal = shading_normal;
    surface.view_dir = normalize(view.camera_pos - in.world_pos);
    surface.f0 = vec3<f32>(0.04);

    let view_pos = view.view * vec4<f32>(in.world_pos, 1.0);
    let view_depth = -view_pos.z;
    let shadow_factor = sample_shadow(in.world_pos, view_depth);

    let light_dir = normalize(light.direction);
    var result = evaluate_brdf(surface, light_dir) * light.color * shadow_factor;

    if cluster_params.num_local_lights > 0u {
        let frag_coord = vec2<u32>(in.clip_pos.xy);
        let tile = frag_coord / vec2(cluster_params.tile_size);
        let depth_slice = compute_depth_slice(view_depth);
        let froxel_idx = tile.x + tile.y * cluster_params.grid_width
            + depth_slice * cluster_params.grid_width * cluster_params.grid_height;

        let cluster_data = cluster_offsets[froxel_idx];
        for (var i = 0u; i < cluster_data.count; i++) {
            let ll = local_lights[light_index_list[cluster_data.offset + i]];
            let light_vec = ll.position_ws - in.world_pos;
            let dist = length(light_vec);
            if dist > ll.range { continue; }
            let l = light_vec / dist;
            var atten = smooth_distance_attenuation(dist, ll.range);
            if ll.light_type == 1u {
                let cos_angle = dot(-l, normalize(ll.direction_ws));
                atten *= smoothstep(ll.spot_cos_outer, ll.spot_cos_inner, cos_angle);
            }
            result += evaluate_brdf(surface, l) * ll.color * atten;
        }
    }

    result += 0.03 * surface.albedo; // ambient

    // 6. Brush cursor ring (Phase 14D-3): drawn in-shader so it follows the
    // terrain contour exactly. brush.w: 0 = off, 1 = sculpt (green), 2 = paint (blue).
    if params.brush.w > 0.5 {
        let d = distance(in.world_pos.xz, params.brush.xy);
        let ring_width = max(params.brush.z * 0.04, 0.15);
        let ring = 1.0 - smoothstep(0.0, ring_width, abs(d - params.brush.z));
        let fill = (1.0 - smoothstep(params.brush.z * 0.85, params.brush.z, d)) * 0.08;
        var cursor_color = vec3<f32>(0.2, 1.0, 0.3);
        if params.brush.w > 1.5 {
            cursor_color = vec3<f32>(0.25, 0.55, 1.0);
        }
        result = mix(result, cursor_color * 2.0, clamp(ring * 0.8 + fill, 0.0, 1.0));
    }

    return vec4<f32>(result, 1.0);
}
