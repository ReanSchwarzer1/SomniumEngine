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
    _pad0: f32,
    _pad1: f32,
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
    sun_angular_radius: f32,                  // offset 312  Phase 24E
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
// Phase 24I: `rgb` = bent normal in [0,1], `a` = screen-space visibility.
@group(1) @binding(6) var gtao_tex: texture_2d<f32>;
// Phase 24X: scene depth, for the contact-shadow march.
@group(1) @binding(7) var scene_depth: texture_depth_2d;
// Phase 24K: traced sun visibility. `.a` is 0 when ReSTIR did not run, which
// is how the shader knows to fall back to the shadow map.
@group(1) @binding(8) var restir_vis: texture_2d<f32>;

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

/// Light transmitted through a thin surface (Phase 24S).
///
/// Frostbite's approximation (Barré-Brisebois & Bouchard). A real subsurface
/// solve is out of scope, but the visual signature of translucency is
/// specific and cheap to reproduce: light leaving the *far* side of a thin
/// surface, spread by scattering, brightest when the viewer looks almost
/// straight into the source through the material.
///
/// This is what the foliage has been missing all along. Leaves lit only by
/// reflection stay flat and dark no matter how correct their albedo is —
/// which is exactly the symptom the grass has shown since Phase 17. A backlit
/// leaf glowing green is most of what makes vegetation read as alive.
fn transmitted_light(
    surface: Surface,
    light_dir: vec3<f32>,
    light_color: vec3<f32>,
    transmission: f32,
) -> vec3<f32> {
    if transmission <= 0.0 {
        return vec3<f32>(0.0);
    }

    /// Bends the transmitted direction back toward the surface normal, so the
    /// glow wraps around the silhouette instead of appearing only where the
    /// light is exactly behind.
    const DISTORTION: f32 = 0.25;
    /// Tightens the lobe. Higher means the glow appears only closer to
    /// straight-through, which is what thin, dense material looks like.
    const POWER: f32 = 4.0;
    /// Ambient share, so a leaf in shade is still translucent rather than
    /// switching off entirely when the sun is not behind it.
    const AMBIENT: f32 = 0.15;

    // The direction light takes leaving the far side.
    let transmit_dir = normalize(-light_dir + surface.normal * DISTORTION);
    let lobe = pow(saturate(dot(surface.view_dir, transmit_dir)), POWER);

    // Tinted by albedo: light passing through a leaf picks up its colour, which
    // is why backlit foliage reads more saturated than the same leaf lit from
    // the front.
    return light_color * (lobe + AMBIENT) * transmission * surface.albedo;
}

/// Specular occlusion from baked AO (Lagarde & de Rousiers).
///
/// Ambient occlusion describes hemispherical visibility, so applying it
/// unchanged to a mirror-like reflection is wrong. This narrows it by view
/// angle and roughness: grazing, rough surfaces in a crevice lose most of
/// their reflection, while a smooth surface facing you keeps its highlight.
///
/// This is what was making foliage read blue-grey. Grass albedo is a dark
/// olive, so the 4% Fresnel sheen of an unoccluded sky reflection was a large
/// fraction of the blade's final colour, and the sky is blue.
fn specular_occlusion(n_dot_v: f32, ao: f32, roughness: f32) -> f32 {
    return saturate(pow(n_dot_v + ao, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao);
}

/// Image-based ambient: diffuse irradiance + split-sum specular.
fn evaluate_ibl(surface: Surface) -> vec3<f32> {
    let n = surface.normal;
    let v = surface.view_dir;
    let n_dot_v = max(dot(n, v), 1e-4);

    // Diffuse: the roughest mip approximates a cosine-convolved irradiance
    // map. Not a true convolution, but close enough visually and it saves a
    // whole extra prefilter chain.
    // Bent normal rather than the surface normal: it points along the average
    // *unoccluded* direction, so a surface in a crevice gathers light from the
    // opening instead of from the wall beside it. This is the part of GTAO that
    // changes the colour of indirect light rather than only its amount.
    let gather_n = normalize(mix(n, surface.bent_normal, 0.75));
    let irradiance = textureSampleLevel(env_cube, env_sampler, gather_n, ENV_MAX_MIP).rgb;
    let kd = (vec3<f32>(1.0) - surface.f0) * (1.0 - surface.metallic);
    let diffuse = irradiance * surface.albedo * kd;

    // Specular: prefiltered radiance along the reflection vector, weighted by
    // the analytic BRDF term.
    let r = reflect(-v, n);
    let mip = surface.roughness * ENV_MAX_MIP;
    let prefiltered = textureSampleLevel(env_cube, env_sampler, r, mip).rgb;
    let specular = prefiltered * env_brdf_approx(surface.f0, surface.roughness, n_dot_v);
    let spec_ao  = specular_occlusion(n_dot_v, surface.occlusion, surface.roughness);

    // Occlusion applies to indirect light only. The sun already has shadow
    // maps, and multiplying it by AO as well double-darkens lit surfaces.
    return (diffuse * surface.occlusion + specular * spec_ao) * light.ibl_intensity;
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

// ── Shadows (Phase 24H) ─────────────────────────────────────────────────────
//
// Replaces a fixed 3x3 PCF kernel. Three problems with that: the penumbra was
// always the same width regardless of how far the caster was from the receiver,
// which is the single clearest tell that a shadow is not real; cascade
// boundaries showed as hard lines where filter width changed; and constant
// depth bias traded acne for peter-panning with no setting that avoided both.

/// Sample count for the blocker search and for the filter itself.
///
/// The search can afford to be coarser: it only needs an average depth, while
/// the filter's samples are visible as noise if too few.
const SHADOW_BLOCKER_SAMPLES: i32 = 16;
const SHADOW_FILTER_SAMPLES: i32 = 24;

/// Average depth of occluders above this point, or -1 if there are none.
///
/// The first half of PCSS: how far away the blocker is determines how wide its
/// penumbra should be, which is why a contact point is sharp and the same
/// object's shadow is soft metres away.
fn blocker_search(
    atlas_coord: vec2<f32>,
    cascade: u32,
    compare_depth: f32,
    search_radius: f32,
    rotation: f32,
) -> f32 {
    var blocker_sum = 0.0;
    var blocker_count = 0.0;

    for (var i = 0; i < SHADOW_BLOCKER_SAMPLES; i = i + 1) {
        let offset = vogel_disk_sample(u32(i), u32(SHADOW_BLOCKER_SAMPLES), rotation)
            * search_radius;
        let uv = atlas_coord + offset;
        // Clamp into this cascade's quadrant: straying outside samples a
        // neighbouring cascade's depths, which are unrelated.
        let quadrant_min = atlas_uv(cascade, vec2<f32>(0.0));
        let quadrant_max = atlas_uv(cascade, vec2<f32>(1.0));
        let clamped = clamp(uv, quadrant_min, quadrant_max);

        // textureLoad rather than a sampled fetch: the blocker search wants the
        // raw stored depth, and the only sampler bound to the atlas is a
        // comparison sampler, which returns a pass/fail result instead.
        let texel = vec2<i32>(clamped * light.shadow_map_size);
        let depth = textureLoad(shadow_atlas, texel, 0);
        if depth < compare_depth {
            blocker_sum += depth;
            blocker_count += 1.0;
        }
    }

    if blocker_count < 0.5 {
        return -1.0;
    }
    return blocker_sum / blocker_count;
}

/// Percentage-closer soft shadows for one cascade.
fn sample_shadow_cascade(
    world_pos: vec3<f32>,
    normal: vec3<f32>,
    cascade: u32,
    pixel: vec2<f32>,
) -> f32 {
    // Normal-offset bias: push the sample along the surface normal rather than
    // along depth. Depth bias has to grow with slope to stop acne, and by the
    // time it is large enough on a grazing surface it has detached the shadow
    // from its caster. Offsetting in the plane of the surface sidesteps both.
    // A depth-space bias, not a world-space normal offset.
    //
    // Two previous attempts offset the sample position along the normal by a
    // "world texel size" recovered from the cascade matrix. That recovery is
    // wrong: column 0 of `proj * view` mixes the x, y and depth scales, so its
    // length is not the cascade's world width, and the resulting offset was far
    // enough to walk the sample out of the shadow entirely. Measured directly,
    // the shadow map holds the caster and reports occlusion correctly at the
    // un-offset position — it was only ever the offset that lost it.
    //
    // Comparing at the fragment's own position with a small depth epsilon,
    // widened at grazing angles where a texel spans more depth, is both simpler
    // and what the probe that found this actually did.
    let offset_pos = world_pos;
    let light_clip = light.view_proj[cascade] * vec4<f32>(offset_pos, 1.0);
    let ndc = light_clip.xyz / light_clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 1.0 - (ndc.y * 0.5 + 0.5));
    let atlas_coord = atlas_uv(cascade, uv);
    let n_dot_l = saturate(dot(normal, normalize(light.direction)));
    let compare_depth = ndc.z - (0.0005 + 0.0025 * (1.0 - n_dot_l));

    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || compare_depth > 1.0 {
        return 1.0;
    }

    let texel_size = 1.0 / light.shadow_map_size;
    let rotation = interleaved_gradient_noise(pixel, u32(light.shadow_map_size) % 64u)
        * 6.28318530;

    // Search radius scales with the sun's angular size: a larger source casts
    // wider penumbrae, so it has to look further for blockers.
    let search_radius = max(light.sun_angular_radius * 40.0, 2.0) * texel_size;
    let blocker_depth = blocker_search(
        atlas_coord, cascade, compare_depth, search_radius, rotation);

    // No blocker found: fully lit, and no filtering needed.
    if blocker_depth < 0.0 {
        return 1.0;
    }

    // Penumbra width from the similar-triangles relation: the further the
    // blocker is above the receiver, the wider its shadow's soft edge.
    let penumbra = (compare_depth - blocker_depth) / max(blocker_depth, 1e-4);
    let filter_radius = clamp(
        penumbra * light.sun_angular_radius * 400.0,
        1.0,
        16.0,
    ) * texel_size;

    var shadow = 0.0;
    let quadrant_min = atlas_uv(cascade, vec2<f32>(0.0));
    let quadrant_max = atlas_uv(cascade, vec2<f32>(1.0));
    for (var i = 0; i < SHADOW_FILTER_SAMPLES; i = i + 1) {
        let offset = vogel_disk_sample(u32(i), u32(SHADOW_FILTER_SAMPLES), rotation)
            * filter_radius;
        let clamped = clamp(atlas_coord + offset, quadrant_min, quadrant_max);
        shadow += textureSampleCompare(
            shadow_atlas, shadow_sampler, clamped, compare_depth);
    }
    return shadow / f32(SHADOW_FILTER_SAMPLES);
}

// ── Contact shadows (Phase 24X) ─────────────────────────────────────────────
//
// A shadow map cannot resolve contact. Its texels cover centimetres at best,
// and the normal-offset bias from 24H deliberately pushes samples off the
// surface, which erases exactly the darkening where two surfaces meet — a
// trunk against soil, a leaf on the leaf beneath it. Those small dark contacts
// are a large part of why objects read as resting on a surface rather than
// hovering above it.
//
// This marches a short ray through the depth buffer toward the light and looks
// for anything crossing it. Parameters follow Bend Studio's screen-space
// shadows; the wavefront scheduling that makes their version fast is not ported
// here, only the sampling behaviour.

/// Steps along the ray. Short by design — this fills the gap the shadow map
/// leaves at contact range, and anything longer is the shadow map's job.
const CONTACT_STEPS: i32 = 12;
/// World-space length of the march.
const CONTACT_LENGTH: f32 = 0.35;
/// How far behind a depth sample still counts as the same surface.
///
/// Without a thickness limit every thin object casts an infinitely deep
/// shadow volume behind itself, because the march cannot tell a thin leaf from
/// a wall extending away from the camera.
const CONTACT_THICKNESS: f32 = 0.05;

fn contact_shadow(world_pos: vec3<f32>, light_dir: vec3<f32>, pixel: vec2<f32>) -> f32 {
    let step_world = CONTACT_LENGTH / f32(CONTACT_STEPS);

    // Jitter the start so the march's step pattern becomes noise rather than
    // visible banding; TAA then resolves it.
    let jitter = interleaved_gradient_noise(pixel, u32(light.shadow_map_size) % 64u);

    var occluded = 0.0;
    for (var i = 1; i <= CONTACT_STEPS; i = i + 1) {
        let t = (f32(i) + jitter) * step_world;
        let sample_pos = world_pos + light_dir * t;

        let clip = view.view_proj * vec4<f32>(sample_pos, 1.0);
        if clip.w <= 0.0 {
            break;
        }
        let ndc = clip.xyz / clip.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
            break;
        }

        // Compare the ray's depth against what the depth buffer actually holds
        // at that pixel. Closer stored depth means geometry sits between this
        // point and the light.
        let texel = vec2<i32>(uv * vec2<f32>(textureDimensions(scene_depth)));
        let scene_z = textureLoad(scene_depth, texel, 0);
        let diff = ndc.z - scene_z;

        // The upper bound is what stops a surface far behind the ray from
        // registering as an occluder.
        if diff > 0.0 && diff < CONTACT_THICKNESS {
            // Fade with distance so the shadow ends softly instead of stopping
            // at a hard ring where the march runs out.
            occluded = max(occluded, 1.0 - f32(i) / f32(CONTACT_STEPS));
        }
    }

    // Contrast, following Bend's parameter of the same name: contacts are
    // small and a linear falloff reads as haze rather than as shadow.
    return saturate(1.0 - occluded * 4.0);
}

/// Shadow factor in [0,1]; 1.0 = fully lit.
///
/// Blends across the cascade boundary rather than switching at it. An abrupt
/// switch shows as a line across the ground where filter width and resolution
/// change together, and it is far more obvious in motion than in a still.
fn sample_shadow(world_pos: vec3<f32>, normal: vec3<f32>, view_depth: f32, pixel: vec2<f32>) -> f32 {
    let cascade = get_cascade_index(view_depth);
    let near = select(light.cascade_splits[cascade - 1u], 0.0, cascade == 0u);
    let far = light.cascade_splits[cascade];

    var shadow = sample_shadow_cascade(world_pos, normal, cascade, pixel);

    // Contact shadows only ever darken. The shadow map is authoritative for
    // everything at its own scale; this fills in below that scale.
    shadow = min(shadow, contact_shadow(world_pos, normalize(light.direction), pixel));

    // Blend over the last 10% of the cascade's range.
    //
    // The `max(band, 1e-4)` here was hiding a degenerate case rather than
    // handling it. When `far - near` collapses, dividing by 1e-4 sends
    // `into_band` to a huge positive number, `saturate` pins it to 1, and the
    // mix returns the *next* cascade outright — which does not cover this
    // fragment, so its lookup falls outside and early-returns "lit". Every
    // shadow in the visibility-buffer path was being blended away to nothing
    // this way, while terrain and water kept theirs because neither blends
    // cascades at all. Guard the band instead of dividing by an epsilon.
    let band = (far - near) * 0.1;
    if band > 1e-3 && cascade < 3u {
        let into_band = (view_depth - (far - band)) / band;
        if into_band > 0.0 {
            let next = sample_shadow_cascade(world_pos, normal, cascade + 1u, pixel);
            return mix(shadow, next, saturate(into_band));
        }
    }
    return shadow;
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

        // Phase 24C: sample the environment cubemap rather than evaluating a
        // second sky here. That cubemap is now generated by a real atmospheric
        // ray-march (`ibl_gen.wgsl`), and sharing it means the background, the
        // ambient light and the reflections cannot disagree — before this there
        // were three separate gradients maintained by hand.
        let sky = textureSampleLevel(env_cube, env_sampler, ray_dir, 0.0).rgb;

        // Sharp detail at screen resolution, not cubemap resolution.
        let sun_dir = normalize(light.direction);
        let sun_illuminance = dot(light.color, vec3<f32>(0.2126, 0.7152, 0.0722));
        // See ibl_gen.wgsl: night is keyed to illuminance, not elevation.
        let moon_dir = -sun_dir;
        let moon_strength = saturate(1.0 - sun_illuminance / 10.0);
        let detail = sky_detail(ray_dir, sun_dir, sun_illuminance, moon_dir, moon_strength);

        return vec4<f32>(sky + detail, 1.0);
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
    var geo_normal = normalize((instance.model * vec4<f32>(normal_interp, 0.0)).xyz);

    let hit_point = p0 * bary.x + p1 * bary.y + p2 * bary.z;

    // Phase 17D: a double-sided surface can be seen from behind, where its
    // authored normal points away and every lighting term comes out dark. Flip
    // it toward the viewer. Only for materials flagged double-sided — doing it
    // unconditionally would light the inside of closed geometry.
    // Phase 17E: face the normal toward the *sun*, not the viewer.
    //
    // Facing the viewer looks right until you stand between the sun and the
    // surface: the flipped normal then points away from the light, N.L goes
    // negative, and back-lit foliage renders black. Real leaves are thin and
    // translucent, so both faces receive light — flipping toward the sun is the
    // cheap stand-in for that, and it is what made a field of grass stop
    // looking like a field of ash.
    if (material.flags & 1u) != 0u && dot(geo_normal, normalize(light.direction)) < 0.0 {
        geo_normal = -geo_normal;
    }

    // TBN matrix (derived from edge vectors + UV deltas, no vertex tangents)
    let edge0 = p1 - p0;
    let edge1 = p2 - p0;
    let uv0   = vec2<f32>(v0.u, v0.v);
    let uv1   = vec2<f32>(v1.u, v1.v);
    let uv2   = vec2<f32>(v2.u, v2.v);
    let duv0  = uv1 - uv0;
    let duv1  = uv2 - uv0;

    // Degenerate UVs have to be detected, not nudged past.
    //
    // This read `1.0 / (tbn_det + 1e-7)`, which does not rescue a degenerate
    // triangle — it manufactures a huge `inv_det`, and when the numerator is
    // also near zero `normalize` returns NaN. A NaN normal reflects whatever
    // the environment map holds, so on foliage it came out as flat facets of
    // sky blue scattered through the canopy, across bark, and streaked down
    // grass blades. Packed foliage atlases produce exactly this: collinear or
    // duplicated UVs on cards, and mirrored islands that flip the determinant.
    //
    // Adding an epsilon is also wrong for a *negative* determinant near
    // -1e-7, where it pushes the denominator toward zero rather than away.
    let tbn_det = duv0.x * duv1.y - duv1.x * duv0.y;
    var tangent = vec3<f32>(0.0);
    var tbn_valid = abs(tbn_det) > 1.0e-12;
    if tbn_valid {
        let raw_tangent = (edge0 * duv1.y - edge1 * duv0.y) / tbn_det;
        if dot(raw_tangent, raw_tangent) > 1.0e-16 {
            tangent = normalize(raw_tangent);
            // Gram-Schmidt can also collapse, when the tangent is parallel to
            // the normal.
            let ortho = tangent - dot(tangent, geo_normal) * geo_normal;
            if dot(ortho, ortho) > 1.0e-12 {
                tangent = normalize(ortho);
            } else {
                tbn_valid = false;
            }
        } else {
            tbn_valid = false;
        }
    }
    if !tbn_valid {
        // A stable arbitrary frame. The normal map is skipped below when the
        // frame is arbitrary, since applying one to a meaningless tangent is
        // how the garbage got in.
        let up = select(
            vec3<f32>(0.0, 1.0, 0.0),
            vec3<f32>(1.0, 0.0, 0.0),
            abs(geo_normal.y) > 0.99,
        );
        tangent = normalize(cross(up, geo_normal));
    }
    let bitangent = cross(geo_normal, tangent);
    let tbn       = mat3x3<f32>(tangent, bitangent, geo_normal);

    // PBR surface setup
    var surface: Surface;
    surface.albedo    = material.base_color.rgb;
    if material.albedo_map >= 0 {
        surface.albedo *= textureSample(textures[material.albedo_map], default_sampler, uv).rgb;
    }

    surface.occlusion = 1.0;
    surface.roughness = max(material.roughness, 0.05);
    surface.metallic  = material.metallic;
    if material.metallic_roughness_map >= 0 {
        let mr = textureSample(textures[material.metallic_roughness_map], default_sampler, uv);
        surface.roughness = max(mr.g, 0.05);
        surface.metallic  = mr.b;
    }
    // Phase 24I: fold screen-space occlusion into the baked term.
    //
    // Multiplied rather than replacing: the two measure different things. A
    // baked map knows about detail too small or too enclosed to appear on
    // screen, while GTAO knows about geometry the map's author never saw —
    // a trunk meeting terrain, one object resting against another. Taking the
    // minimum would discard whichever is more informative at any given pixel.
    let gtao = textureLoad(gtao_tex, pixel_coords, 0);
    surface.occlusion = surface.occlusion * gtao.a;

    // GTAO works in view space; the gather happens in world space.
    let bent_view = gtao.rgb * 2.0 - 1.0;
    let bent_world = normalize(
        (transpose(view.view) * vec4<f32>(bent_view, 0.0)).xyz);
    surface.bent_normal = select(surface.normal, bent_world, length(bent_view) > 0.1);

    // Occlusion comes from its own texture, never from the metallic-roughness
    // map: glTF leaves that map's red channel undefined, and models that store
    // AO separately (the damaged helmet among them) leave it at zero, which
    // read as occlusion renders pitch black.
    //
    // Foliage leans on this heavily — a grass tuft's interior sits in its own
    // shade, and without it every blade receives full open sky.
    if material.occlusion_map >= 0 {
        surface.occlusion = textureSample(
            textures[material.occlusion_map], default_sampler, uv).r;
    }

    surface.normal = geo_normal;
    var normal_variance = 0.0;
    if material.normal_map >= 0 && tbn_valid {
        let nm_sample  = textureSample(textures[material.normal_map], default_sampler, uv).rgb;
        let tangent_n  = nm_sample * 2.0 - vec3<f32>(1.0);
        surface.normal = normalize(tbn * tangent_n);

        // Phase 24F: specular anti-aliasing. A normal-map texel that averages
        // many differently-oriented normals has a *shorter* vector than a unit
        // one — the shortening measures how much detail the mip threw away.
        // Toksvig's insight is that this recovers the lost variance, which is
        // then folded into roughness so the lobe widens instead of sparkling.
        //
        // Without it, thin or distant detail flickers on every camera move, and
        // TAA fights the flicker rather than resolving it.
        let len = length(tbn * tangent_n);
        normal_variance = saturate(1.0 - len * len);
    }

    // Widen roughness by the variance the normal map lost to mipping.
    // Squared because roughness is perceptual and alpha is what the BRDF uses.
    if normal_variance > 0.0 {
        let alpha = surface.roughness * surface.roughness;
        surface.roughness = sqrt(sqrt(saturate(alpha * alpha + normal_variance)));
    }


    surface.view_dir = normalize(view.camera_pos - hit_point);
    surface.f0       = mix(vec3<f32>(0.04), surface.albedo, surface.metallic);

    // ── Shadow factor ────────────────────────────────────────────────────────
    // View-space depth: positive Z distance from camera.
    let view_pos   = view.view * vec4<f32>(hit_point, 1.0);
    let view_depth = -view_pos.z; // right-handed: Z is negative in front of camera

    // Phase 24K: prefer the traced result where it exists. It has no cascades,
    // no depth bias and no peter-panning, and its penumbra comes from the sun's
    // actual angular size rather than from a filter chosen to look about right.
    let traced = textureLoad(restir_vis, pixel_coords, 0);
    var shadow_factor = sample_shadow(hit_point, surface.normal, view_depth, in.clip_pos.xy);
    if traced.a > 0.5 {
        shadow_factor = traced.r;
    }

    // Lighting debug (SOMNIUM_SHADOW_DEBUG): 1 = shadow factor.
    if light._pad2_z > 0.5 && light._pad2_z < 1.5 {
        return vec4<f32>(vec3<f32>(shadow_factor), 1.0);
    }
    // 6 = final shadow_factor in hue, immune to exposure.
    //   green = shadowed (< 0.5), red = lit (>= 0.5)
    if light._pad2_z > 5.5 && light._pad2_z < 6.5 {
        if shadow_factor < 0.5 { return vec4<f32>(0.0, 4.0, 0.0, 1.0); }
        return vec4<f32>(4.0, 0.0, 0.0, 1.0);
    }
    // 5 = blocker_search verdict at this fragment, in hue.
    //   red   = search found no blocker (PCSS early-returns lit)
    //   green = search found one (a shadow should appear here)
    if light._pad2_z > 4.5 && light._pad2_z < 5.5 {
        let c = get_cascade_index(view_depth);
        let lc = light.view_proj[c] * vec4<f32>(hit_point, 1.0);
        let nd = lc.xyz / lc.w;
        let cuv = vec2<f32>(nd.x * 0.5 + 0.5, 1.0 - (nd.y * 0.5 + 0.5));
        if any(cuv < vec2<f32>(0.0)) || any(cuv > vec2<f32>(1.0)) {
            return vec4<f32>(0.0, 0.0, 4.0, 1.0);
        }
        let ac = atlas_uv(c, cuv);
        let ts = 1.0 / light.shadow_map_size;
        let sr = max(light.sun_angular_radius * 40.0, 2.0) * ts;
        let bd = blocker_search(ac, c, nd.z - 0.0005, sr, 0.0);
        if bd < 0.0 { return vec4<f32>(4.0, 0.0, 0.0, 1.0); }
        return vec4<f32>(0.0, 4.0, 0.0, 1.0);
    }
    // 4 = shadow-map plumbing, in hue so it survives exposure and tonemapping.
    //   red   = cascade uv outside [0,1] or compare_depth > 1 (early-out to lit)
    //   green = in range, and the atlas holds a nearer depth (should be shadow)
    //   blue  = in range, nothing nearer (correctly lit)
    if light._pad2_z > 3.5 && light._pad2_z < 4.5 {
        let c = get_cascade_index(view_depth);
        let lc = light.view_proj[c] * vec4<f32>(hit_point, 1.0);
        let nd = lc.xyz / lc.w;
        let cuv = vec2<f32>(nd.x * 0.5 + 0.5, 1.0 - (nd.y * 0.5 + 0.5));
        if any(cuv < vec2<f32>(0.0)) || any(cuv > vec2<f32>(1.0)) || nd.z > 1.0 {
            return vec4<f32>(4.0, 0.0, 0.0, 1.0);
        }
        let ac = atlas_uv(c, cuv);
        let d = textureLoad(shadow_atlas, vec2<i32>(ac * light.shadow_map_size), 0);
        if d < nd.z - 0.0005 {
            return vec4<f32>(0.0, 4.0, 0.0, 1.0);
        }
        return vec4<f32>(0.0, 0.0, 4.0, 1.0);
    }

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

        // Phase 24E: the sun is a disc, not a point. A point source gives a
        // highlight one pixel wide on anything smooth, which is among the
        // clearest tells that an image is rendered rather than photographed.
        let direct_light = evaluate_brdf_area(surface, light_dir, light.sun_angular_radius)
            * light_color * shadow_factor;

        // Phase 24S. Deliberately *not* multiplied by the shadow factor: the
        // whole point is light arriving through the surface from the side the
        // shadow map says is dark. Attenuating it by the shadow would remove
        // exactly the case the term exists for.
        let transmitted = transmitted_light(
            surface, light_dir, light_color, material.transmission);
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

                // Phase 24V: the light's angular radius as seen from here.
                // A 5 cm bulb one metre away subtends far more than a point,
                // and that is what stops its highlight being a single pixel on
                // anything polished.
                let angular = atan(max(ll.radius, 0.0) / max(dist, 1e-3));
                local_light_contrib +=
                    evaluate_brdf_area(surface, L, angular) * ll.color * atten_val;
            }
        }

        // Phase 24T: self-emitted light. Independent of every light in the
        // scene by definition — a screen is just as bright in a dark room.
        var emissive = vec3<f32>(material.emissive_r, material.emissive_g, material.emissive_b);
        if material.emissive_map >= 0 {
            emissive *= textureSample(
                textures[material.emissive_map], default_sampler, uv).rgb;
        }

        result = direct_light + transmitted + local_light_contrib + ambient + emissive;

        // 7 = which term actually lights this fragment, in hue.
        //   green = sun dominates, red = ambient dominates
        //   blue  = the surface reads as metallic (kD would be ~0)
        if light._pad2_z > 6.5 && light._pad2_z < 7.5 {
            let ld = dot(direct_light, vec3<f32>(0.2126, 0.7152, 0.0722));
            let la = dot(ambient, vec3<f32>(0.2126, 0.7152, 0.0722));
            if surface.metallic > 0.5 { return vec4<f32>(0.0, 0.0, 4.0, 1.0); }
            if ld > la { return vec4<f32>(0.0, 4.0, 0.0, 1.0); }
            return vec4<f32>(4.0, 0.0, 0.0, 1.0);
        }

        // 2 = sun only, 3 = ambient only. Isolates which term a surface's
        // brightness actually comes from.
        if light._pad2_z > 1.5 && light._pad2_z < 2.5 {
            result = direct_light;
        } else if light._pad2_z > 2.5 {
            result = ambient;
        }
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

    // Clamp below Rgba16Float's finite limit of 65 504. A GGX highlight on a
    // near-mirror surface under a 100 000 lux sun overshoots it, and the
    // resulting Inf poisons anything downstream that divides — TAA's tone-map
    // step turns it into NaN. Prevented here as well as guarded there.
    return vec4<f32>(min(result, vec3<f32>(60000.0)), 1.0);
}
