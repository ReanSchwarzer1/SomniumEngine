// Somnium Engine — Visibility Buffer Shading Pass
// Phase 12: Clustered Local Lights + Cel-Shading Mode

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
// Phases 24U/25I: froxel volumetrics. rgb = log in-scattering to this depth,
// a = surviving transmittance. See pass/volumetric.rs.
@group(1) @binding(9) var volumetrics: texture_3d<f32>;
@group(1) @binding(10) var volumetric_sampler: sampler;
// Metres the volume spans; 0 when volumetrics are switched off.
@group(1) @binding(11) var<uniform> volumetric_range: vec4<f32>;
// Phase 24L: traced indirect diffuse. `rgb` is incoming irradiance — the GI
// pass does not know this surface's albedo and must not, or it would be applied
// twice — and `a` is the "a traced result exists" flag, on the convention 24K
// established. Alpha 0 is what an unsupported device and a switched-off pass
// both produce, and it is what sends `evaluate_ibl` back to the cubemap.
@group(1) @binding(12) var restir_gi: texture_2d<f32>;
// Phase 24M/N/O/P/Q: world cache + scene specular / path tracer (2D) and the
// clipmapped volume (cache rgb, SDF in alpha). Dummy 1×1 targets when off.
@group(1) @binding(13) var lighting_aux: texture_2d<f32>;
@group(1) @binding(14) var world_volume: texture_3d<f32>;
@group(1) @binding(15) var<uniform> lighting_extra: vec4<f32>;
// x = flags (bit0 cache, 1 specular, 2 path tracer, 3 sdf, 4 probes)
// y = cache intensity, z = cell size metres, w = volume half-extent cells
@group(1) @binding(16) var<storage, read> sh_probes: array<vec4<f32>>;

/// Highest mip index of the environment map (must match `IblPass::MIP_COUNT - 1`).
const ENV_MAX_MIP: f32 = 5.0;

/// Scale applied to image-based ambient (Phase 25M-2).
///
/// Set to 1.0 (physically correct). Screen-space ambient occlusion (GTAO),
/// ReSTIR GI, and Lagarde specular occlusion now handle the shadowing of sky
/// light in creases and occluded regions, so the old 0.35 scale-back fudge
/// is no longer required.


/// Perspective-correct barycentric at an NDC sample (Phase 25N).
fn vis_barycentric(
    ndc0: vec2<f32>, ndc1: vec2<f32>, ndc2: vec2<f32>,
    c0w: f32, c1w: f32, c2w: f32,
    sample_ndc: vec2<f32>,
) -> vec3<f32> {
    let det = (ndc1.y - ndc2.y) * (ndc0.x - ndc2.x) + (ndc2.x - ndc1.x) * (ndc0.y - ndc2.y);
    let w0 = ((ndc1.y - ndc2.y) * (sample_ndc.x - ndc2.x) + (ndc2.x - ndc1.x) * (sample_ndc.y - ndc2.y)) / det;
    let w1 = ((ndc2.y - ndc0.y) * (sample_ndc.x - ndc2.x) + (ndc0.x - ndc2.x) * (sample_ndc.y - ndc2.y)) / det;
    let w2 = 1.0 - w0 - w1;
    var bary = vec3<f32>(w0 / c0w, w1 / c1w, w2 / c2w);
    return bary / (bary.x + bary.y + bary.z);
}

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

/// Light transmitted through a thin, two-sided surface (Phase 24S/25M-2).
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

    // UE's two-sided foliage model: ordinary reflection remains on the front,
    // while a wrapped and energy-bounded lobe carries light through the back.
    // There is intentionally no constant ambient term; that old term produced
    // albedo-tinted glow even when no light crossed the leaf.
    const WRAP: f32 = 0.5;
    let back_wrap = saturate((-dot(surface.normal, light_dir) + WRAP)
        / ((1.0 + WRAP) * (1.0 + WRAP)));
    let view_scatter = pow(saturate(dot(surface.view_dir, -light_dir)), 4.0);
    let scatter = mix(0.35, 1.0, view_scatter);

    return light_color * surface.albedo * transmission * back_wrap * scatter;
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

/// Lambert irradiance of a quad (Phase 24R). Linearly Transformed Cosines
/// reduce to this closed form for the diffuse lobe; specular still uses the
/// existing area BRDF with an equivalent angular radius.
fn ltc_quad_diffuse(
    pos: vec3<f32>,
    n: vec3<f32>,
    center: vec3<f32>,
    light_n: vec3<f32>,
    half_x: f32,
    half_y: f32,
) -> f32 {
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if abs(dot(light_n, up)) > 0.95 {
        up = vec3<f32>(1.0, 0.0, 0.0);
    }
    let t = normalize(cross(up, light_n));
    let b = cross(light_n, t);
    let v0 = center + t * half_x + b * half_y;
    let v1 = center - t * half_x + b * half_y;
    let v2 = center - t * half_x - b * half_y;
    let v3 = center + t * half_x - b * half_y;
    let p0 = normalize(v0 - pos);
    let p1 = normalize(v1 - pos);
    let p2 = normalize(v2 - pos);
    let p3 = normalize(v3 - pos);
    let pts = array<vec3<f32>, 4>(p0, p1, p2, p3);
    var sum = 0.0;
    for (var i = 0; i < 4; i++) {
        let a = pts[i];
        let c = pts[(i + 1) % 4];
        let h = acos(clamp(dot(a, c), -1.0, 1.0));
        let x = cross(a, c);
        let xl = length(x);
        if xl > 1e-6 {
            sum += h * dot(x / xl, n);
        }
    }
    return max(sum, 0.0) * 0.5 / 3.14159265;
}

fn ltc_disc_diffuse(
    pos: vec3<f32>,
    n: vec3<f32>,
    center: vec3<f32>,
    light_n: vec3<f32>,
    radius: f32,
) -> f32 {
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if abs(dot(light_n, up)) > 0.95 {
        up = vec3<f32>(1.0, 0.0, 0.0);
    }
    let t = normalize(cross(up, light_n));
    let b = cross(light_n, t);
    var prev = vec3<f32>(0.0);
    var sum = 0.0;
    let sides = 8;
    for (var i = 0; i <= sides; i++) {
        let a = 6.2831853 * f32(i % sides) / f32(sides);
        let v = center + (t * cos(a) + b * sin(a)) * radius;
        let p = normalize(v - pos);
        if i > 0 {
            let h = acos(clamp(dot(prev, p), -1.0, 1.0));
            let x = cross(prev, p);
            let xl = length(x);
            if xl > 1e-6 {
                sum += h * dot(x / xl, n);
            }
        }
        prev = p;
    }
    return max(sum, 0.0) * 0.5 / 3.14159265;
}

fn closest_on_segment(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-8), 0.0, 1.0);
    return a + ab * t;
}

fn sh_irradiance(n: vec3<f32>, base: u32) -> vec3<f32> {
    let c1 = 0.429043;
    let c2 = 0.511664;
    let c3 = 0.743125;
    let c4 = 0.886227;
    let c5 = 0.247708;
    let l00 = sh_probes[base + 0u].rgb;
    let l1m1 = sh_probes[base + 1u].rgb;
    let l10 = sh_probes[base + 2u].rgb;
    let l11 = sh_probes[base + 3u].rgb;
    let l2m2 = sh_probes[base + 4u].rgb;
    let l2m1 = sh_probes[base + 5u].rgb;
    let l20 = sh_probes[base + 6u].rgb;
    let l21 = sh_probes[base + 7u].rgb;
    let l22 = sh_probes[base + 8u].rgb;
    let x = n.x;
    let y = n.y;
    let z = n.z;
    return c1 * l22 * (x * x - y * y)
        + c3 * l20 * z * z
        + c4 * l00
        - c5 * l20
        + 2.0 * c1 * (l2m2 * x * y + l21 * x * z + l2m1 * y * z)
        + 2.0 * c2 * (l11 * x + l1m1 * y + l10 * z);
}

fn world_volume_uvw(pos: vec3<f32>) -> vec3<f32> {
    let cell = max(lighting_extra.z, 0.25);
    let half_cells = max(lighting_extra.w, 1.0);
    let origin = floor(view.camera_pos / cell) * cell;
    return ((pos - origin) / cell + vec3<f32>(half_cells)) / (half_cells * 2.0);
}

fn sample_sh_probes(pos: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let uvw = clamp(world_volume_uvw(pos), vec3<f32>(0.0), vec3<f32>(1.0));
    let g = uvw * 3.0;
    let i0 = vec3<u32>(clamp(floor(g), vec3<f32>(0.0), vec3<f32>(3.0)));
    let i1 = min(i0 + vec3<u32>(1u), vec3<u32>(3u));
    let f = fract(g);
    let b000 = (i0.x + i0.y * 4u + i0.z * 16u) * 9u;
    let b100 = (i1.x + i0.y * 4u + i0.z * 16u) * 9u;
    let b010 = (i0.x + i1.y * 4u + i0.z * 16u) * 9u;
    let b110 = (i1.x + i1.y * 4u + i0.z * 16u) * 9u;
    let b001 = (i0.x + i0.y * 4u + i1.z * 16u) * 9u;
    let b101 = (i1.x + i0.y * 4u + i1.z * 16u) * 9u;
    let b011 = (i0.x + i1.y * 4u + i1.z * 16u) * 9u;
    let b111 = (i1.x + i1.y * 4u + i1.z * 16u) * 9u;
    let x00 = mix(sh_irradiance(n, b000), sh_irradiance(n, b100), f.x);
    let x10 = mix(sh_irradiance(n, b010), sh_irradiance(n, b110), f.x);
    let x01 = mix(sh_irradiance(n, b001), sh_irradiance(n, b101), f.x);
    let x11 = mix(sh_irradiance(n, b011), sh_irradiance(n, b111), f.x);
    let y0 = mix(x00, x10, f.y);
    let y1 = mix(x01, x11, f.y);
    return mix(y0, y1, f.z);
}

/// Image-based ambient: diffuse irradiance + split-sum specular.
/// Phase 24L: `traced_diffuse` replaces the diffuse half when the GI pass has a
/// result for this pixel. Only the diffuse half — the specular lobe still comes
/// from the environment cubemap, because ReSTIR GI resolves *diffuse* indirect
/// and a mirror needs a sharp reflection this pass cannot give it.
fn evaluate_ibl(surface: Surface, traced_diffuse: vec4<f32>) -> vec3<f32> {
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
    var diffuse = irradiance * surface.albedo * kd;
    if traced_diffuse.a > 0.5 {
        // The traced term is irradiance, so the albedo and the dielectric
        // fraction still belong here. Applying them in the shading pass rather
        // than in the GI pass keeps one definition of what a surface's colour
        // is — the GI pass only ever sees the *bounce* surface's albedo, never
        // this one's, and applying both would square it.
        diffuse = traced_diffuse.rgb * surface.albedo * kd;
    }

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
/// Shadow texels to push the sample along the surface normal (Phase 25L).
///
/// Tunable in one place because it is the whole acne/peter-panning trade: too
/// small and a heightfield stipples itself black, too large and shadows detach
/// from what casts them. `SOMNIUM_SHADOW_OFFSET` overrides it at runtime for
/// finding the knee.
const SHADOW_NORMAL_OFFSET_TEXELS: f32 = 1.5;
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
    // **Phase 25L: the recovery was taking the wrong vector.** A depth epsilon
    // alone is a constant in NDC, and a cascade's NDC-to-world scale varies by
    // orders of magnitude across the four — so an epsilon tuned to stop acne in
    // cascade 3 detaches shadows in cascade 0, and one tuned for cascade 0 does
    // nothing further out. On the CDLOD dataset, whose relief is metre-scale
    // against a shadow texel covering several metres, that showed as the surface
    // stippled black wherever a texel's stored depth belonged to a neighbouring
    // ridge rather than to this fragment.
    //
    // The fix the earlier attempts wanted is a **world-space** offset of one
    // shadow texel, and it needs the cascade's world-per-NDC scale. `ndc.x` is
    // `dot(row0, world)`, so the gradient of NDC with respect to world position
    // is **row** 0 of the matrix, and world units per NDC unit is the reciprocal
    // of its length. The previous attempts took *column* 0, which for
    // `proj * view` mixes the x, y and depth scales and is not that gradient —
    // which is exactly why the offset came out far enough to walk the sample
    // out of the shadow.
    //
    // glam is column-major, so row 0 is the `.x` of each of the first three
    // columns.
    let m = light.view_proj[cascade];
    let row0 = vec3<f32>(m[0].x, m[1].x, m[2].x);
    let world_per_ndc = 1.0 / max(length(row0), 1e-9);
    // Each cascade occupies one quadrant of the atlas, so its own resolution is
    // half the atlas dimension, spanning 2 NDC units.
    let texel_world = 2.0 * world_per_ndc / (light.shadow_map_size * 0.5);

    let n_dot_l = saturate(dot(normal, normalize(light.direction)));
    // Offset along the surface normal, widened at grazing angles where one
    // texel spans more depth. Offsetting in the plane of the surface rather
    // than along depth is what avoids the acne/peter-panning trade entirely.
    //
    // Phase 25M-2B: quadratic ramp — the old linear `(1 + 2*(1-NdotL))` was
    // too mild for the large texel sizes of outer cascades at grazing angles.
    let grazing = 1.0 - n_dot_l;
    let offset_pos = world_pos
        + normal * texel_world * max(1.0, 4.0 * grazing * grazing) * SHADOW_NORMAL_OFFSET_TEXELS;
    let light_clip = light.view_proj[cascade] * vec4<f32>(offset_pos, 1.0);
    let ndc = light_clip.xyz / light_clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 1.0 - (ndc.y * 0.5 + 0.5));
    let atlas_coord = atlas_uv(cascade, uv);
    // Phase 25M-2: the grazing-angle term belongs to the world-space
    // geometric-normal offset above, not to a mixed-unit depth expression.
    // Keep only a small residual depth epsilon. `texel_world` is measured in
    // metres and cannot be multiplied directly into an NDC depth bias; doing
    // so made the offset explode at grazing angles. The world-space normal
    // offset above is the dimensionally correct grazing-angle treatment.
    let compare_depth = ndc.z - 0.0002;

    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || compare_depth > 1.0 {
        return 1.0;
    }

    let texel_size = 1.0 / light.shadow_map_size;
    let rotation = interleaved_gradient_noise(pixel, u32(light.shadow_map_size) % 64u)
        * 6.28318530;

    // Bit 1 of shading_mode: PCSS. Off is a single comparison so shadows still
    // exist without the 16+24 tap filter.
    if (cluster_params.shading_mode & 2u) == 0u {
        return textureSampleCompare(
            shadow_atlas, shadow_sampler, atlas_coord, compare_depth);
    }

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

fn contact_shadow(
    world_pos: vec3<f32>,
    normal: vec3<f32>,
    light_dir: vec3<f32>,
    pixel: vec2<f32>,
) -> f32 {
    // CPU atmospheric transmittance is authoritative for whether direct
    // sunlight still exists; an elevation threshold creates a twilight step.
    if max(light.color.r, max(light.color.g, light.color.b)) <= 1.0e-6 {
        return 1.0;
    }

    let step_world = CONTACT_LENGTH / f32(CONTACT_STEPS);

    // Phase 25L: start the march off the surface, along the normal.
    //
    // The march compares its own depth against the depth buffer, and its first
    // steps are still *on* the surface it started from. On ground seen at a
    // grazing angle — which terrain always is — those steps land within
    // `CONTACT_THICKNESS` of the stored depth, so the surface shadows itself as
    // a fine stipple over everything. Offsetting by one step's worth of world
    // space is below the scale this term exists to resolve and costs nothing.
    let start = world_pos + normal * step_world;

    // Jitter the start so the march's step pattern becomes noise rather than
    // visible banding; TAA then resolves it.
    let jitter = interleaved_gradient_noise(pixel, u32(light.shadow_map_size) % 64u);

    var occluded = 0.0;
    for (var i = 1; i <= CONTACT_STEPS; i = i + 1) {
        let t = (f32(i) + jitter) * step_world;
        let sample_pos = start + light_dir * t;

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

        // `CONTACT_THICKNESS` is metres. Comparing it to `ndc.z` made the
        // accepted slab grow dramatically with distance because perspective
        // depth is nonlinear; on landscape terrain, unrelated neighbouring
        // triangles then counted as 5 cm blockers and stamped large polygonal
        // shadows across the ground. Reconstruct the sampled surface and do
        // the thickness test in view-space metres instead.
        let scene_ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, scene_z, 1.0);
        let scene_world_h = view.inv_view_proj * scene_ndc;
        let scene_world = scene_world_h.xyz / scene_world_h.w;
        let ray_view_depth = -(view.view * vec4<f32>(sample_pos, 1.0)).z;
        let scene_view_depth = -(view.view * vec4<f32>(scene_world, 1.0)).z;
        let diff = ray_view_depth - scene_view_depth;

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
    // Bit 2 of shading_mode: contact march. Default on.
    if (cluster_params.shading_mode & 4u) != 0u {
        shadow = min(shadow, contact_shadow(world_pos, normal, normalize(light.direction), pixel));
    }

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
    let vis_texel    = textureLoad(vis_buffer, pixel_coords, 0).rg;
    let vis_data     = vis_texel.x;

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
        // Phase 25M-2D: physical moon direction from the simplified lunar
        // orbital model in sun.rs, not the old `-sun_dir` full-moon hack.
        let moon_dir = normalize(light.moon_direction);
        let moon_strength = saturate(1.0 - sun_illuminance / 10.0);
        let detail = sky_detail(ray_dir, sun_dir, sun_illuminance, moon_dir, moon_strength);

        return vec4<f32>(sky + detail, 1.0);
    }

    // ── PBR surface ─────────────────────────────────────────────────────────
    // Phase 15C: 16/16 split (see visibility.wgsl for the packing).
    // Separate channels, no packing. See visibility.wgsl.
    let instance_id = vis_texel.x - 1u;
    let prim_id     = vis_texel.y;

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
    var bary = vis_barycentric(ndc0, ndc1, ndc2, c0.w, c1.w, c2.w, target_ndc);

    let uv0 = vec2<f32>(v0.u, v0.v);
    let uv1 = vec2<f32>(v1.u, v1.v);
    let uv2 = vec2<f32>(v2.u, v2.v);
    let uv = uv0 * bary.x + uv1 * bary.y + uv2 * bary.z;

    // Phase 25N: analytic UV gradients. Implicit dpdx across a vis-buffer
    // 2×2 quad straddles unrelated triangles, so foliage mips jump per pixel.
    // Evaluate this triangle's barycentric at the neighbouring pixels instead.
    let pixel = 1.0 / vec2<f32>(textureDimensions(vis_buffer));
    let bary_x = vis_barycentric(ndc0, ndc1, ndc2, c0.w, c1.w, c2.w, target_ndc + vec2<f32>(2.0 * pixel.x, 0.0));
    let bary_y = vis_barycentric(ndc0, ndc1, ndc2, c0.w, c1.w, c2.w, target_ndc + vec2<f32>(0.0, -2.0 * pixel.y));
    var uv_ddx = (uv0 * bary_x.x + uv1 * bary_x.y + uv2 * bary_x.z) - uv;
    var uv_ddy = (uv0 * bary_y.x + uv1 * bary_y.y + uv2 * bary_y.z) - uv;
    let analytic_grad = (cluster_params.shading_mode & 8u) != 0u;
    if !analytic_grad {
        uv_ddx = vec2<f32>(0.0);
        uv_ddy = vec2<f32>(0.0);
    }

    let normal_interp = normalize(
        vec3<f32>(v0.norm_x, v0.norm_y, v0.norm_z) * bary.x +
        vec3<f32>(v1.norm_x, v1.norm_y, v1.norm_z) * bary.y +
        vec3<f32>(v2.norm_x, v2.norm_y, v2.norm_z) * bary.z
    );
    var geo_normal = normalize((instance.model * vec4<f32>(normal_interp, 0.0)).xyz);

    // Hard-surface meshes bias along their real triangle plane. Terrain is the
    // important exception: its central-difference vertex normals describe one
    // continuous height field, while the per-face normal jumps at every fan
    // triangle. Using that discontinuous normal for receiver bias made the
    // shadow term reproduce the topology as large dark triangles. Terrain
    // therefore uses the smooth geometric normal; normal maps remain excluded.
    let face_cross = cross(p1 - p0, p2 - p0);
    var shadow_normal = geo_normal;
    if material.terrain_index < 0 && dot(face_cross, face_cross) > 1.0e-16 {
        shadow_normal = normalize(face_cross);
        if dot(shadow_normal, geo_normal) < 0.0 {
            shadow_normal = -shadow_normal;
        }
    }

    let hit_point = p0 * bary.x + p1 * bary.y + p2 * bary.z;

    // Phase 17D: a double-sided surface can be seen from behind, where its
    // authored normal points away and every lighting term comes out dark. Flip
    // it toward the viewer. Only for materials flagged double-sided — doing it
    // unconditionally would light the inside of closed geometry.
    // Phase 25M-2: face the material frame toward the viewer and handle light
    // arriving from the back with the explicit two-sided transmission lobe.
    // Keep the material frame tied to the visible face. Back lighting is an
    // explicit transmission lobe; it must not flip normals as the sun sets.
    let view_dir_early = normalize(view.camera_pos - hit_point);
    if (material.flags & 1u) != 0u && dot(geo_normal, view_dir_early) < 0.0 {
        geo_normal = -geo_normal;
        shadow_normal = -shadow_normal;
    }

    // TBN matrix (derived from edge vectors + UV deltas, no vertex tangents)
    let edge0 = p1 - p0;
    let edge1 = p2 - p0;
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
    // Handedness, from the sign of the UV determinant.
    //
    // A mirrored UV island has a negative determinant, and mirroring is
    // routine on bark and other trunk-like meshes because it halves the
    // texture needed for a symmetric surface. Without this sign the bitangent
    // points the wrong way there, which inverts the normal map's green channel
    // and tilts the shading normal away from the light. It shows as hard-edged
    // patches that follow UV seams rather than anything in the geometry —
    // dark navy where the wrongly-tilted normal picks up sky instead of sun.
    let handedness = select(-1.0, 1.0, tbn_det >= 0.0);
    let bitangent = cross(geo_normal, tangent) * handedness;
    let tbn       = mat3x3<f32>(tangent, bitangent, geo_normal);

    // PBR surface setup
    var surface: Surface;
    surface.albedo    = material.base_color.rgb;
    surface.normal    = geo_normal;
    if material.albedo_map >= 0 {
        if analytic_grad {
            surface.albedo *= textureSampleGrad(
                textures[material.albedo_map], default_sampler, uv, uv_ddx, uv_ddy).rgb;
        } else {
            surface.albedo *= textureSample(textures[material.albedo_map], default_sampler, uv).rgb;
        }
    }

    surface.occlusion = 1.0;
    surface.roughness = max(material.roughness, 0.05);
    surface.metallic  = material.metallic;
    if material.metallic_roughness_map >= 0 {
        let mr = select(
            textureSample(textures[material.metallic_roughness_map], default_sampler, uv),
            textureSampleGrad(textures[material.metallic_roughness_map], default_sampler, uv, uv_ddx, uv_ddy),
            analytic_grad,
        );
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
    surface.bent_normal = select(geo_normal, bent_world, length(bent_view) > 0.1);

    // Occlusion comes from its own texture, never from the metallic-roughness
    // map: glTF leaves that map's red channel undefined, and models that store
    // AO separately (the damaged helmet among them) leave it at zero, which
    // read as occlusion renders pitch black.
    //
    // Foliage leans on this heavily — a grass tuft's interior sits in its own
    // shade, and without it every blade receives full open sky.
    if material.occlusion_map >= 0 {
        if analytic_grad {
            surface.occlusion = textureSampleGrad(
                textures[material.occlusion_map], default_sampler, uv, uv_ddx, uv_ddy).r;
        } else {
            surface.occlusion = textureSample(
                textures[material.occlusion_map], default_sampler, uv).r;
        }
    }

    var normal_variance = 0.0;
    if material.normal_map >= 0 && tbn_valid {
        let nm_sample = select(
            textureSample(textures[material.normal_map], default_sampler, uv).rgb,
            textureSampleGrad(textures[material.normal_map], default_sampler, uv, uv_ddx, uv_ddy).rgb,
            analytic_grad,
        );
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

    // ── Foliage: curved card normals (Phase 17E) ─────────────────────────────
    //
    // A leaf or blade is modelled as a flat card, so every pixel across it
    // shares one normal and the whole card lights as a flat plate — the tell
    // that vegetation is cardboard rather than a plant. Real leaves are curved,
    // and a curved surface fans its normals across its width.
    //
    // Ported from `SpartanEngine-master/data/shaders/g_buffer.hlsl`, its
    // "foliage curved normals": rotate the normal about the axis running along
    // the card by an angle taken from how far across the card's *width* the
    // pixel sits. Spartan carries a `width_percent` vertex attribute for this;
    // Somnium has no such attribute and does not need one — on a foliage card
    // `uv.x` **is** the distance across the blade, which is what makes this
    // free here.
    //
    // Gated on `MATERIAL_FLAG_FOLIAGE` rather than on `transmission`, because
    // glass is transmissive too and must not be bent into a leaf.
    if (material.flags & 2u) != 0u {
        // ±60° across the card, matching the reference's 120° span.
        let curve = clamp((uv.x - 0.5) * 2.0943951, -1.5707963, 1.5707963);
        // The axis along the card's length: perpendicular to both the face
        // normal and the direction the UV's x runs in.
        let axis_raw = cross(surface.normal, tangent);
        if dot(axis_raw, axis_raw) > 1.0e-12 {
            let axis = normalize(axis_raw);
            let c = cos(curve);
            let s = sin(curve);
            // Rodrigues' rotation of the normal about that axis.
            surface.normal = normalize(
                surface.normal * c
                + cross(axis, surface.normal) * s
                + axis * dot(axis, surface.normal) * (1.0 - c)
            );
        }
    }

    // Phase 25M-2: foliage roughness floor (night wet/metallic fix).
    //
    // Leaves are not mirrors — their waxy cuticle is rough, and at grazing
    // angles the micro-roughness of surface irregularities dominates.
    // Without this floor the environment specular (which is all that remains
    // at night when diffuse goes to zero) makes grass read as wet metal.
    if (material.flags & 2u) != 0u {
        let foliage_ndv = abs(dot(surface.normal, view_dir_early));
        let foliage_roughness_floor = mix(0.6, 0.35, foliage_ndv);
        surface.roughness = max(surface.roughness, foliage_roughness_floor);
    }

    // ── Terrain (Phase 25A-2) ────────────────────────────────────────────────
    //
    // The only material branch terrain needs. Everything above it — decoding
    // the visibility buffer, fetching the triangle, interpolating position,
    // normal and UV — is the shared path, because chunks are in the global
    // vertex pool and carry an ordinary `vertex_offset`. Everything below it is
    // shared too, which is what finally gives terrain GTAO, contact shadows,
    // traced sun visibility, IBL and correct cascade blending, and what stops
    // the next lighting change having to be written twice.
    if material.terrain_index >= 0 {
        // Derivatives of world position, which the hex-tiled layer sampling
        // needs explicitly. Taken here rather than inside the material because
        // that is where the terrain UVs are derived from.
        let world_ddx = dpdx(hit_point.xz);
        let world_ddy = dpdy(hit_point.xz);
        let terrain = evaluate_terrain_material(
            u32(material.terrain_index), hit_point, geo_normal, uv,
            world_ddx, world_ddy);
        surface.albedo = terrain.albedo;
        surface.roughness = terrain.roughness;
        surface.metallic = 0.0;
        surface.normal = terrain.normal;
        // Phase 25K: the material's own occlusion, folded into the screen-space
        // term the same way a glTF occlusion map is — the two know different
        // things, and GTAO cannot see detail below a pixel.
        surface.occlusion = surface.occlusion * terrain.occlusion;
        terrain_taps = terrain.taps;
        terrain_discarded = terrain.discarded;
        terrain_selected_rgb = terrain.selected_rgb;
        terrain_weight_rgb = terrain.weight_rgb;
        terrain_parallax_shadow_factor = terrain.parallax_shadow;
    }


    surface.view_dir = normalize(view.camera_pos - hit_point);
    surface.f0       = mix(vec3<f32>(0.04), surface.albedo, surface.metallic);
    if material.terrain_index >= 0 {
        surface.f0 = surface.f0 + vec3<f32>(terrain_wet_f0);
    }

    // ── Shadow factor ────────────────────────────────────────────────────────
    // View-space depth: positive Z distance from camera.
    let view_pos   = view.view * vec4<f32>(hit_point, 1.0);
    let view_depth = -view_pos.z; // right-handed: Z is negative in front of camera

    // Phase 24K: prefer the traced result where it exists. It has no cascades,
    // no depth bias and no peter-panning, and its penumbra comes from the sun's
    // actual angular size rather than from a filter chosen to look about right.
    let traced = textureLoad(restir_vis, pixel_coords, 0);
    // Bias follows the actual triangle plane, not interpolated vertex data, a
    // normal map, or a foliage card's synthetic curvature.
    //
    // When ReSTIR DI wrote a result, that *is* the sun visibility — PCSS was
    // previously still evaluated and then discarded, which also threw away
    // POM self-shadow. Skip the filter and keep the relief term.
    var shadow_factor: f32;
    if traced.a > 0.5 {
        shadow_factor = traced.r * terrain_parallax_shadow_factor;
    } else {
        shadow_factor = sample_shadow(hit_point, shadow_normal, view_depth, in.clip_pos.xy)
            * terrain_parallax_shadow_factor;
    }

    // 9 = albedo, 10 = shading normal, 11 = terrain_index as a flag.
    // Material-path probes: a surface that renders black is either unlit or
    // untextured, and only looking at the channels separately tells which.
    if light._pad2_z > 8.5 && light._pad2_z < 9.5 {
        return vec4<f32>(surface.albedo, 1.0);
    }
    if light._pad2_z > 9.5 && light._pad2_z < 10.5 {
        return vec4<f32>(surface.normal * 0.5 + 0.5, 1.0);
    }
    if light._pad2_z > 10.5 && light._pad2_z < 11.5 {
        if material.terrain_index >= 0 {
            return vec4<f32>(0.0, 1.0, 0.0, 1.0);
        }
        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
    }

    // 8 = the occlusion actually reaching the surface, greyscale.
    //
    // Added while chasing why toggling GTAO changed nothing on terrain: the
    // ambient-only view read exactly zero there, which is what an occlusion of
    // 0 produces, and only a direct look at the term could say whether that was
    // GTAO's answer or a texture nobody had written.
    if light._pad2_z > 7.5 && light._pad2_z < 8.5 {
        return vec4<f32>(vec3<f32>(surface.occlusion), 1.0);
    }

    // 12 = terrain layer taps as a fraction of the 36-tap worst case
    // (Phase XV-D). Written straight to the HDR target before exposure, so the
    // capture harness's mean terrain luminance times TERRAIN_MAX_TAPS *is* the
    // mean taps per pixel — which is how the detail budget gets a number
    // instead of a claim.
    if light._pad2_z > 11.5 && light._pad2_z < 12.5 {
        return vec4<f32>(vec3<f32>(f32(terrain_taps) / TERRAIN_MAX_TAPS), 1.0);
    }

    // 13 = terrain chunk LOD. Rust places lod+1 in the instance padding only
    // while this debug view is active; zero means a non-terrain instance.
    if light._pad2_z > 12.5 && light._pad2_z < 13.5 {
        let lod = instance._padding;
        if lod == 0u { return vec4<f32>(0.08, 0.08, 0.08, 1.0); }
        let palette = array<vec3<f32>, 5>(
            vec3<f32>(0.10, 0.85, 0.25), vec3<f32>(0.20, 0.55, 1.00),
            vec3<f32>(0.85, 0.75, 0.10), vec3<f32>(1.00, 0.35, 0.08),
            vec3<f32>(0.75, 0.12, 0.85)
        );
        return vec4<f32>(palette[min(lod - 1u, 4u)] * 4.0, 1.0);
    }
    // 14 = analytic triangle edges reconstructed from visibility barycentrics.
    if light._pad2_z > 13.5 && light._pad2_z < 14.5 {
        let edge = 1.0 - smoothstep(0.005, 0.025, min(bary.x, min(bary.y, bary.z)));
        return vec4<f32>(vec3<f32>(edge * 4.0), 1.0);
    }
    // 15/16 = interpolated geometric normal / actual receiver-bias normal.
    if light._pad2_z > 14.5 && light._pad2_z < 15.5 {
        return vec4<f32>(geo_normal * 0.5 + 0.5, 1.0);
    }
    if light._pad2_z > 15.5 && light._pad2_z < 16.5 {
        return vec4<f32>(shadow_normal * 0.5 + 0.5, 1.0);
    }
    // 17 = screen-space contact-shadow factor before cascade composition.
    if light._pad2_z > 16.5 && light._pad2_z < 17.5 {
        let contact = contact_shadow(hit_point, shadow_normal, normalize(light.direction), in.clip_pos.xy);
        return vec4<f32>(vec3<f32>(contact), 1.0);
    }
    // 18 = splat weight discarded by strongest-four (XV-D).
    if light._pad2_z > 17.5 && light._pad2_z < 18.5 {
        return vec4<f32>(vec3<f32>(terrain_discarded * 4.0), 1.0);
    }
    // 19 = first three selected layer indices, 0..1 over layers 0–15.
    if light._pad2_z > 18.5 && light._pad2_z < 19.5 {
        return vec4<f32>(terrain_selected_rgb, 1.0);
    }
    // 20 = raw strongest-four weights of the first three selected layers.
    if light._pad2_z > 19.5 && light._pad2_z < 20.5 {
        return vec4<f32>(terrain_weight_rgb, 1.0);
    }
    // 21 = dominant selected-layer albedo (solo).
    if light._pad2_z > 20.5 && light._pad2_z < 21.5 {
        return vec4<f32>(terrain_dominant_albedo, 1.0);
    }
    // 22 = cliff projection blend.
    if light._pad2_z > 21.5 && light._pad2_z < 22.5 {
        return vec4<f32>(vec3<f32>(terrain_cliff_blend_dbg), 1.0);
    }
    // 23 = wetness factor (moisture affinity × global wetness).
    if light._pad2_z > 22.5 && light._pad2_z < 23.5 {
        return vec4<f32>(vec3<f32>(terrain_wetness_factor), 1.0);
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

    if (cluster_params.shading_mode & 1u) == 1u {
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

        // Phase 25M-2 (UE5 pattern): Directional Moonlight.
        // In UE5, when the sun drops below the horizon, the moon acts as a secondary
        // directional light (AtmosphereLightIndex 1), illuminating terrain and foliage from
        // `moon_direction` with a cool blue tint (~0.15 lux).
        var moonlight = vec3<f32>(0.0);
        let sun_illuminance = dot(light.color, vec3<f32>(0.2126, 0.7152, 0.0722));
        let moon_strength = saturate(1.0 - sun_illuminance / 10.0);
        let is_foliage = (material.flags & 2u) != 0u;

        if moon_strength > 0.0 && light.moon_intensity > 0.0 {
            let moon_dir = normalize(light.moon_direction);
            let moon_color = vec3<f32>(0.55, 0.72, 1.0) * light.moon_intensity * moon_strength;

            // `evaluate_brdf` already contains N.L. The previous extra
            // multiply squared the front-face response, while its attempted
            // double-sided factor could not revive a back face after the BRDF
            // had already returned zero.
            moonlight = evaluate_brdf(surface, moon_dir) * moon_color;

            // Transmitted moonlight for foliage leaves
            if is_foliage && material.transmission > 0.0 {
                moonlight += transmitted_light(surface, moon_dir, moon_color, material.transmission);
            }
        }

        // Direct sunlight + directional moonlight
        let direct_light = evaluate_brdf_area(surface, light_dir, light.sun_angular_radius)
            * light_color * shadow_factor + moonlight;

        // Transmitted sunlight follows the same atmospheric fade as every
        // other direct term, with no independent elevation threshold.
        var transmitted = vec3<f32>(0.0);
        if sun_illuminance > 1.0e-6 {
            transmitted = transmitted_light(
                surface, light_dir, light_color, material.transmission);
        }

        let gi_texel = textureLoad(restir_gi, vec2<i32>(in.clip_pos.xy), 0);
        var ambient = evaluate_ibl(surface, gi_texel);
        let extra_flags = bitcast<u32>(lighting_extra.x);
        let vol_uvw = world_volume_uvw(hit_point);
        let vol_sample = textureSampleLevel(world_volume, volumetric_sampler, vol_uvw, 0.0);
        if (extra_flags & 16u) != 0u {
            let kd = (vec3<f32>(1.0) - surface.f0) * (1.0 - surface.metallic);
            let gather_n = normalize(mix(surface.normal, surface.bent_normal, 0.75));
            ambient += sample_sh_probes(hit_point, gather_n) * surface.albedo * kd * surface.occlusion;
        } else if (extra_flags & 1u) != 0u {
            let kd = (vec3<f32>(1.0) - surface.f0) * (1.0 - surface.metallic);
            ambient += vol_sample.rgb * surface.albedo * kd * lighting_extra.y * surface.occlusion;
        }
        // SDF owns volume alpha; the world cache writes occupancy there, so
        // the two cannot run as one field. Cache-on skips the cone-trace.
        if (extra_flags & 8u) != 0u && (extra_flags & 1u) == 0u {
            var sdf_ao = 1.0;
            var march = 0.15;
            for (var s = 0u; s < 6u; s++) {
                let p = hit_point + geo_normal * march;
                let d = textureSampleLevel(world_volume, volumetric_sampler, world_volume_uvw(p), 0.0).a;
                sdf_ao = min(sdf_ao, saturate(d / max(march, 1e-3)));
                march *= 1.7;
            }
            ambient *= sdf_ao;
        }
        if (extra_flags & 2u) != 0u {
            let aux_uv = (vec2<f32>(pixel_coords) + 0.5) / vec2<f32>(textureDimensions(vis_buffer));
            let spec_gi = textureSampleLevel(lighting_aux, default_sampler, aux_uv, 0.0);
            let spec_w = spec_gi.a * saturate(1.0 - surface.roughness);
            ambient = mix(ambient, spec_gi.rgb, spec_w);
        }

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

                if ll.light_type == 4u {
                    let axis = normalize(ll.direction_ws);
                    let half_len = max(ll._pad1, 0.05);
                    let a = ll.position_ws - axis * half_len;
                    let b = ll.position_ws + axis * half_len;
                    let q = closest_on_segment(hit_point, a, b);
                    let to_l = q - hit_point;
                    let dist_t = length(to_l);
                    if dist_t > ll.range { continue; }
                    let Lt = to_l / max(dist_t, 1e-4);
                    let atten_t = smooth_distance_attenuation(dist_t, ll.range);
                    let r = max(ll.radius, 0.01);
                    let angular = atan(r / max(dist_t, 1e-3));
                    let facing = max(length(cross(Lt, axis)), 0.15);
                    local_light_contrib += evaluate_brdf_area(surface, Lt, angular)
                        * ll.color * atten_t * facing;
                    continue;
                }

                let light_vec = ll.position_ws - hit_point;
                let dist = length(light_vec);
                if dist > ll.range { continue; }

                let L = light_vec / dist;
                var atten_val = smooth_distance_attenuation(dist, ll.range);
                if ll.light_type == 2u {
                    let half_x = max(ll._pad1, 0.05);
                    let half_y = max(ll._pad2, 0.05);
                    let ln = normalize(ll.direction_ws);
                    let irr = ltc_quad_diffuse(
                        hit_point, surface.normal, ll.position_ws, ln, half_x, half_y);
                    let eq_r = sqrt(half_x * half_y / 3.14159265);
                    let angular = atan(eq_r / max(dist, 1e-3));
                    local_light_contrib += evaluate_brdf_area(surface, L, angular)
                        * ll.color * atten_val * irr * 3.14159265;
                    continue;
                }
                if ll.light_type == 3u {
                    let r = max(ll.radius, 0.05);
                    let ln = normalize(ll.direction_ws);
                    let irr = ltc_disc_diffuse(
                        hit_point, surface.normal, ll.position_ws, ln, r);
                    let angular = atan(r / max(dist, 1e-3));
                    local_light_contrib += evaluate_brdf_area(surface, L, angular)
                        * ll.color * atten_val * irr * 3.14159265;
                    continue;
                }
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
            if analytic_grad {
                emissive *= textureSampleGrad(
                    textures[material.emissive_map], default_sampler, uv, uv_ddx, uv_ddy).rgb;
            } else {
                emissive *= textureSample(
                    textures[material.emissive_map], default_sampler, uv).rgb;
            }
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
        } else if light._pad2_z > 2.5 && light._pad2_z < 3.5 {
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

    // The terrain brush ring, after lighting because it is an editor overlay
    // rather than a material property (Phase 25A-2).
    if material.terrain_index >= 0 {
        result = terrain_brush_overlay(u32(material.terrain_index), hit_point, result);
    }

    // ── Aerial perspective and fog (Phases 25I, 24U) ─────────────────────────
    //
    // One fetch applies both: distant surfaces lose contrast into the sky's own
    // colour, and any fog medium adds its in-scattering along the way. Applied
    // here, at the end of the shared shading path, it reaches **terrain, meshes
    // and foliage alike** — which is the payoff of 25A-2 putting terrain in this
    // pass rather than its own.
    //
    // The sky is not touched: it returned far above, and its radiance already
    // came from a full march through the same atmosphere. Applying this to it
    // as well would count the air twice.
    if volumetric_range.x > 0.0 {
        let dist = length(hit_point - view.camera_pos);
        let slices = f32(textureDimensions(volumetrics).z);

        // Each texel is the integral over its *whole* slice, so sampling at a
        // slice boundary needs the previous slice's centre — hence the half
        // slice offset (bevy's aerial_view_lut sampling does the same).
        let w = saturate(dist / volumetric_range.x - 0.5 / slices);
        let sample = textureSampleLevel(
            volumetrics, volumetric_sampler, vec3<f32>(in.uv, w), 0.0);

        // Anything nearer than the first slice centre clamps to that slice, so
        // without this fade the full first-slice scattering would be applied
        // right at the camera — fog appearing on the lens.
        let fade = saturate(dist / (volumetric_range.x / slices));
        let inscatter = exp(sample.rgb) * fade;
        let transmittance = mix(1.0, sample.a, fade);

        result = result * transmittance + inscatter;
    }

    // Phase 24AB: lighting debug views (24–31). 0–23 remain the existing
    // material / terrain / shadow probes.
    if light._pad2_z > 23.5 && light._pad2_z < 31.5 {
        let luma = dot(result, vec3<f32>(0.2126, 0.7152, 0.0722));
        if light._pad2_z < 24.5 {
            let t = saturate(log2(luma + 1.0) / 10.0);
            return vec4<f32>(t, 0.2 * (1.0 - t), 1.0 - t, 1.0) * 4.0;
        }
        if light._pad2_z < 25.5 {
            let gi = textureLoad(restir_gi, pixel_coords, 0);
            return vec4<f32>(gi.rgb * 4.0, 1.0);
        }
        if light._pad2_z < 26.5 {
            let tile = vec2<u32>(in.clip_pos.xy) / vec2(cluster_params.tile_size);
            let slice = compute_depth_slice(view_depth);
            let idx = tile.x + tile.y * cluster_params.grid_width
                + slice * cluster_params.grid_width * cluster_params.grid_height;
            let n = f32(cluster_offsets[idx].count) / 8.0;
            return vec4<f32>(n, 0.15, 1.0 - saturate(n), 1.0) * 4.0;
        }
        if light._pad2_z < 27.5 {
            let vol_dbg = textureSampleLevel(
                world_volume, volumetric_sampler, world_volume_uvw(hit_point), 0.0);
            return vec4<f32>(vol_dbg.rgb * 4.0, 1.0);
        }
        if light._pad2_z < 28.5 {
            return vec4<f32>(textureSampleLevel(lighting_aux, default_sampler, in.uv, 0.0).rgb * 4.0, 1.0);
        }
        if light._pad2_z < 29.5 {
            let vol_dbg = textureSampleLevel(
                world_volume, volumetric_sampler, world_volume_uvw(hit_point), 0.0);
            return vec4<f32>(vec3<f32>(vol_dbg.a * 0.1), 1.0);
        }
        if light._pad2_z < 30.5 {
            let mip = length(uv_ddx) * 64.0;
            return vec4<f32>(mip, mip * 0.4, 0.1, 1.0) * 4.0;
        }
        return vec4<f32>(textureSampleLevel(lighting_aux, default_sampler, in.uv, 0.0).rgb, 1.0);
    }

    if (bitcast<u32>(lighting_extra.x) & 4u) != 0u {
        let traced = textureSampleLevel(lighting_aux, default_sampler, in.uv, 0.0);
        if traced.a > 0.01 {
            result = traced.rgb;
        }
    }

    // Clamp below Rgba16Float's finite limit of 65 504. A GGX highlight on a
    // near-mirror surface under a 100 000 lux sun overshoots it, and the
    // resulting Inf poisons anything downstream that divides — TAA's tone-map
    // step turns it into NaN. Prevented here as well as guarded there.
    return vec4<f32>(min(result, vec3<f32>(60000.0)), 1.0);
}
