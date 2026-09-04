// MORROWIND-C: composition is declared here rather than assembled by a
// `format!` of `include_str!` calls at this pass's construction site. The
// resolver (`somnium_shader`) emits each module once, in this order, and
// hoists every `enable` above everything.
//!include "global_pool.wgsl"
//!include "brdf.wgsl"
//!include "sampling.wgsl"
//!include "atmosphere.wgsl"
//!include "hextile.wgsl"
//!include "terrain_material.wgsl"
//!include "clipmap_shade.wgsl"

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
/// CONTROL-M: a world-XZ field of how much sun reaches the ground through the
/// cloud layer. One scalar per column, sampled by every surface the sun lights
/// — terrain, water and meshes alike — so a cloud's shadow crosses a beach onto
/// the sea without either surface knowing what a cloud is.
@group(1) @binding(17) var cloud_shadow_tex: texture_2d<f32>;
/// `[centre_x, centre_z, extent_metres, strength]`. Strength zero means the
/// pass is off and the texture must not be read; that is what keeps a disabled
/// cloud pass free rather than merely cheap.
@group(1) @binding(18) var<uniform> cloud_shadow_params: vec4<f32>;
/// CONTROL-N: `[wet_diffuse, wet_specular, puddles, unused]`.
///
/// All zero when no weather is driving, and `apply_wetness` returns
/// immediately in that case — so a scene with no Weather component shades
/// bit-identically to how it did before this binding existed.
@group(1) @binding(19) var<uniform> weather: vec4<f32>;

/// Phase CONTROL-O: one deferred decal. Mirrors `pass::decal::GpuDecal`.
struct Decal {
    inv_transform: mat4x4<f32>,
    position_ws: vec3<f32>,
    radius: f32,
    base_color: vec4<f32>,
    albedo_map: i32,
    normal_map: i32,
    orm_map: i32,
    priority: i32,
    angle_fade_cos: f32,
    normal_strength: f32,
    roughness: f32,
    _pad: f32,
}

@group(1) @binding(20) var<storage, read> decals: array<Decal>;
/// Per-froxel `(offset, count)` into `decal_indices`. The froxel *geometry* is
/// the light grid's, read from `cluster_params`, so a pixel that has already
/// worked out its froxel for lighting reuses the answer here.
@group(1) @binding(21) var<storage, read> decal_offsets: array<ClusterOffset>;
@group(1) @binding(22) var<storage, read> decal_indices: array<u32>;
/// `[count, 0, 0, 0]`. Zero skips the whole path, so a scene with no decals
/// pays one uniform read.
@group(1) @binding(23) var<uniform> decal_params: vec4<u32>;

// MORROWIND-Z: software-sparse shadow cache. The params buffer keeps this path
// disabled until allocation, page raster, and sampling are all live.
struct VirtualShadowParams {
    geometry: vec4<u32>, // pages/axis, page texels, atlas texels, clip levels
    budget: vec4<u32>, // physical pages, render budget, enabled, CSM fallback
}
@group(1) @binding(24) var virtual_shadow_atlas: texture_depth_2d;
@group(1) @binding(25) var virtual_shadow_sampler: sampler_comparison;
@group(1) @binding(26) var<storage, read> virtual_shadow_pages: array<u32>;
@group(1) @binding(27) var<uniform> virtual_shadow: VirtualShadowParams;
@group(1) @binding(28) var<uniform> grain_words: array<vec4<u32>, 1024>;

/// Sun visibility through the cloud layer at a world position.
///
/// Returns 1 — no shadow at all — when the cloud pass is off, when the point
/// falls outside the field, or when the strength is zero. Outside rather than
/// clamped: the field is centred on the camera and clamping would smear the
/// edge texel across the whole horizon, which reads as a permanent shadow
/// wall at the edge of the world.
fn cloud_shadow_at(world_pos: vec3<f32>) -> f32 {
    let strength = cloud_shadow_params.w;
    if strength <= 0.0 {
        return 1.0;
    }
    let extent = max(cloud_shadow_params.z, 1.0);
    let local = (world_pos.xz - cloud_shadow_params.xy) / extent;
    if any(abs(local) > vec2<f32>(1.0)) {
        return 1.0;
    }
    let uv = local * 0.5 + 0.5;
    return clamp(textureSampleLevel(cloud_shadow_tex, default_sampler, uv, 0.0).r, 0.0, 1.0);
}

/// CONTROL-N: what rain does to a surface, following Lagarde's *Water drop 3b*.
///
/// Three effects, one authored material channel (`porosity`) and two driven
/// scalars, applied in place on the shading surface:
///
/// 1. **Albedo darkens non-linearly.** `albedo^(1 + k)` rather than
///    `albedo * (1 - k)`. A multiply darkens white as hard as black; water does
///    not — a wet white wall stays nearly white while wet asphalt goes almost
///    to pitch. The exponent form has that behaviour for free and it cannot
///    push a channel out of `0..1`.
/// 2. **Specular rises and roughness falls.** A water film is a smooth
///    dielectric layer over the surface, so `f0` moves toward water's 0.02 and
///    the microfacet roughness collapses toward it. Driven by the *specular*
///    scalar, which recovers before the diffuse one.
/// 3. **Standing water flattens the normal.** The accumulated-water term
///    interpolates the shading normal toward straight up, with a mirror-flat
///    puddle as the limit case. No separate puddle material, no second texture
///    set — §6.3's rule, and the reason porosity is what scales all of it.
///
/// A sealed surface (`porosity = 0`) is untouched by every one of these, which
/// is why a car parked in the rain does not go matte with the pavement.
fn apply_wetness(surface: ptr<function, Surface>, porosity: f32) {
    let wet_diffuse = weather.x;
    let wet_specular = weather.y;
    if wet_diffuse <= 0.001 && wet_specular <= 0.001 {
        return;
    }
    let uptake = clamp(porosity, 0.0, 1.0);

    // 1. Non-linear darkening. Capped at a doubling of the exponent, which is
    //    roughly the darkest a real porous surface gets.
    let darkening = 1.0 + uptake * wet_diffuse;
    (*surface).albedo = pow(max((*surface).albedo, vec3<f32>(0.0)), vec3<f32>(darkening));

    // 2. Water's own Fresnel, and a smoother microfacet distribution. Both are
    //    scaled by uptake: a sealed surface already has its own film.
    let film = uptake * wet_specular;
    (*surface).f0 = mix((*surface).f0, max((*surface).f0, vec3<f32>(0.02)), film);
    (*surface).roughness = mix((*surface).roughness, (*surface).roughness * 0.25, film)
        + 0.0;
    (*surface).roughness = clamp((*surface).roughness, 0.015, 1.0);

    // 3. Standing water. Independent of porosity — a puddle sits on top of a
    //    sealed surface just as happily as on a porous one.
    let puddle = clamp(weather.z, 0.0, 1.0);
    if puddle > 0.001 {
        (*surface).normal = normalize(mix(
            (*surface).normal, vec3<f32>(0.0, 1.0, 0.0), puddle * wet_specular));
        (*surface).roughness = mix((*surface).roughness, 0.02, puddle * wet_specular);
    }
}

/// Phase CONTROL-O: project this froxel's decals onto the surface.
///
/// A deferred decal is a box. A pixel is inside one when its position, taken
/// into the decal's own space, lies within the unit cube — which is one
/// matrix multiply and three comparisons, and is why the *inverse* transform is
/// what the buffer carries.
///
/// Two fades, both necessary:
///
/// - **Edge fade** toward the box's faces, so a decal does not end on a hard
///   line at the limit of its own volume.
/// - **Angle fade** against the decal's own -Y axis. Without it a projection
///   aimed at the floor smears down every wall inside its box, which is the
///   characteristic failure of naïve deferred decals and the reason
///   `angle_fade_cos` is authored rather than fixed.
///
/// Applied in ascending priority so the highest-priority decal is applied last
/// and wins; the CPU sorts, so the shader does not have to.
fn apply_decals(surface: ptr<function, Surface>, world_pos: vec3<f32>, froxel: u32) {
    let count = decal_params.x;
    if count == 0u {
        return;
    }
    let bucket = decal_offsets[froxel];
    if bucket.count == 0u {
        return;
    }
    let geometric_normal = (*surface).normal;

    for (var i = 0u; i < bucket.count; i = i + 1u) {
        let index = decal_indices[bucket.offset + i];
        if index >= count {
            continue;
        }
        let decal = decals[index];

        let local = (decal.inv_transform * vec4<f32>(world_pos, 1.0)).xyz;
        if any(abs(local) > vec3<f32>(0.5)) {
            continue;
        }

        // The decal projects along its own -Y, so its UVs are the other two
        // axes and its facing is +Y in decal space taken back to the world.
        let axis = normalize((decal.inv_transform * vec4<f32>(0.0, 1.0, 0.0, 0.0)).xyz);
        let facing = dot(geometric_normal, axis);
        if facing <= decal.angle_fade_cos {
            continue;
        }
        let angle_weight = smoothstep(
            decal.angle_fade_cos, min(decal.angle_fade_cos + 0.25, 1.0), facing);

        // Edge fade: strongest at the centre of the box, zero at its faces.
        let edge = (vec3<f32>(0.5) - abs(local)) * 2.0;
        let edge_weight = smoothstep(0.0, 0.35, min(edge.x, min(edge.y, edge.z)));

        let uv = local.xz + vec2<f32>(0.5);
        var colour = decal.base_color;
        if decal.albedo_map >= 0 {
            colour *= textureSampleLevel(
                textures[decal.albedo_map], default_sampler, uv, 0.0);
        }
        let alpha = clamp(colour.a * angle_weight * edge_weight, 0.0, 1.0);
        if alpha <= 0.001 {
            continue;
        }

        (*surface).albedo = mix((*surface).albedo, colour.rgb, alpha);
        (*surface).roughness = mix((*surface).roughness, decal.roughness, alpha);
        if decal.orm_map >= 0 {
            let orm = textureSampleLevel(
                textures[decal.orm_map], default_sampler, uv, 0.0);
            (*surface).roughness = mix((*surface).roughness, orm.g, alpha);
            (*surface).metallic = mix((*surface).metallic, orm.b, alpha);
        }
        if decal.normal_map >= 0 && decal.normal_strength > 0.0 {
            let tangent_normal =
                textureSampleLevel(textures[decal.normal_map], default_sampler, uv, 0.0).xyz
                * 2.0 - 1.0;
            // Built against the decal's own frame rather than the surface's:
            // a decal's normal map describes the decal, and re-deriving a
            // tangent basis from the surface would rotate it with whatever it
            // happened to land on.
            let n = axis;
            let t = normalize((decal.inv_transform * vec4<f32>(1.0, 0.0, 0.0, 0.0)).xyz);
            let b = cross(n, t);
            let mapped = normalize(t * tangent_normal.x + n * tangent_normal.z + b * tangent_normal.y);
            (*surface).normal = normalize(mix(
                (*surface).normal, mapped, alpha * decal.normal_strength));
        }
    }
}

/// Highest mip index of the environment map (must match `IblPass::MIP_COUNT - 1`).
const ENV_MAX_MIP: f32 = 5.0;

// Pipeline overrides. The compact PSO (Island, hex/POM off, ReSTIR sun)
// sets these false so DXC deletes PCSS/contact/clipmap/debug from the
// shader. Runtime `shading_mode` bits cannot do that.
override enable_pcss: bool = true;
override enable_contact: bool = true;
override enable_clipmap: bool = true;

/// Phase TSUSHIMA-F1: how hard micro-shadowing is allowed to bite.
///
/// Unity HDRP exposes the same control and ships it at zero, leaving artists
/// to dial it up; the technique is a look, not a law. Measured here at 1.0 it
/// removed 39% of terrain radiance at an 8-degree sun — most of which was the
/// wrong AO being fed in, but not all of it, because at a grazing sun `N.L` is
/// small over the whole landscape and this term is a function of `N.L`.
override micro_shadow_opacity: f32 = 1.0;

/// Phase TSUSHIMA-E: screen-space geometric specular antialiasing.
///
/// A pipeline override rather than a `shading_mode` bit because, unlike the F
/// terms, this one is two derivative instructions on a vector — cheap, but
/// derivatives are the one thing worth being able to compile out entirely.
override enable_specular_aa: bool = true;

/// Tokuyoshi & Kaplanyan's published constants: sigma^2 = 1/(2*pi), kappa = 0.18.
const SPEC_AA_SIGMA2: f32 = 0.15915494;
const SPEC_AA_KAPPA: f32 = 0.18;

/// Occlusion at the scale micro-shadowing is actually about: **the material's
/// own AO map, and nothing else.**
///
/// Not GTAO, and not TSUSHIMA-C's sky visibility. This took two goes to get
/// right and the second failure is the instructive one.
///
/// `micro_shadow` is a **hard cutoff** — `saturate(N.L + 2ao^2 - 1)`. Feeding a
/// hard cutoff a screen-space quantity means every wobble in GTAO's estimate
/// becomes a *visible edge in direct sunlight*, and feeding it an interpolated
/// vertex normal means the cutoff traces the mesh triangulation. Together they
/// produced exactly that: dark blotches following GTAO, and triangular facets
/// following the mesh, both worst on open sunlit hillsides where there is no
/// micro-relief to justify either.
///
/// The material AO is the right input because it is what the term was designed
/// against: a texture-scale record of relief below the pixel footprint, which
/// is the only thing "micro-shadowing" is meant to describe. GTAO belongs to
/// the ambient term, where it already is.
var<private> micro_occlusion: f32 = 1.0;
override enable_debug: bool = true;

// Phase DOOM-B ablation. 0 = normal; 1 sky, 2 mesh, 3 foliage, 4 terrain shade
// and everything else returns black. `SOMNIUM_SHADE_ABLATE` drives it and the
// codes match `pass::shading::ablate`.
//
// This deliberately produces a wrong image. It exists because DOOM-A proved the
// shading pass runs exactly one fragment per pixel, so its 25.8 ms on Coastal
// ground is entirely per-pixel cost — and the only way to attribute that cost to
// a class of pixel is to run one class at a time and time it. §17.7 measured 25D
// the same way, through a debug shader, for the same reason: there was no other
// instrument that could answer the question.
//
// It measures *execution* cost, not occupancy cost. A class that early-outs here
// still contributes its registers to the pipeline's high-water mark, so the sum
// of the ablated timings will come out *below* the un-ablated total. That gap is
// the occupancy tax, and it is the part DOOM-C's separate pipelines recover on
// top of the execution savings.
override shade_ablate: u32 = 0u;

const ABLATE_OFF:     u32 = 0u;
const ABLATE_SKY:     u32 = 1u;
const ABLATE_MESH:    u32 = 2u;
const ABLATE_FOLIAGE: u32 = 3u;
const ABLATE_TERRAIN: u32 = 4u;

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
    // Phase TSUSHIMA-F: the fit itself moved to `brdf.wgsl` as `env_brdf_ab`,
    // because `ab` is what the multiple-scattering terms need and this
    // function used to compute it one line before discarding it.
    let ab = env_brdf_ab(roughness, n_dot_v);
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
    // Probes sit at texel centres (gid + 0.5) / 4, so uvw 0.125 → index 0.
    let g = clamp(uvw * 4.0 - 0.5, vec3<f32>(0.0), vec3<f32>(3.0));
    let i0 = vec3<u32>(floor(g));
    let i1 = min(i0 + vec3<u32>(1u), vec3<u32>(3u));
    let f = g - vec3<f32>(i0);
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

fn sh_probe_volume_weight(pos: vec3<f32>) -> f32 {
    let uvw = world_volume_uvw(pos);
    let g = uvw * 4.0 - 0.5;
    // Coverage is horizontal. Terrain height crossing a narrow Y extent must
    // not draw contour rings; Y still selects/clamps the vertical probes.
    let edge = min(min(g.x, g.z), min(3.0 - g.x, 3.0 - g.z));
    // Fade through the outer probe cell instead of clamping its lighting over
    // the whole world. Clamped edge probes produced visible horizontal bands
    // on Island terrain outside the 4x4x4 camera-relative volume.
    return smoothstep(-0.25, 0.5, edge);
}

/// Image-based ambient: diffuse irradiance + split-sum specular.
/// Phase 24L: `traced_diffuse` replaces the diffuse half when the GI pass has a
/// result for this pixel. Only the diffuse half — the specular lobe still comes
/// from the environment cubemap, because ReSTIR GI resolves *diffuse* indirect
/// and a mirror needs a sharp reflection this pass cannot give it.
fn evaluate_ibl_diffuse(surface: Surface, traced_diffuse: vec4<f32>) -> vec3<f32> {
    let n = surface.normal;

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

    return diffuse * surface.occlusion;
}

/// Bound a specular reflection against the sky it is reflecting.
///
/// The environment cube is the atmosphere's own radiance, clamped at 60,000 —
/// and near the sun it reaches that clamp. A surface whose reflection vector
/// lands on that spike returns 60,000 from a single pixel, while its
/// neighbours a fraction of a degree away return a few thousand. That is a
/// firefly, and it is exactly what the shipped maps show: terrain mean 1421,
/// p99.9 4277, and **eighteen pixels sitting on 60,000 exactly**.
///
/// The sun is *also* added analytically by `evaluate_brdf_area`, with the
/// area-light treatment that gives it a real angular size. So its appearance
/// in the reflection cube is a double count as well as an aliasing source, and
/// bounding it here loses nothing that is not already accounted for.
///
/// The bound is relative to the cube's own roughest mip — the average sky
/// radiance — so it needs no absolute constant and stays correct at dusk, at
/// night and under cloud, when 60,000 would be meaningless. Headroom opens up
/// as roughness falls, because a near-mirror legitimately *does* return far
/// more than the average, and closes on rough ground, which cannot.
///
/// Luminance-scaled rather than per-channel clamped: clamping channels
/// independently shifts the highlight's hue toward whichever one did not clip.
fn clamp_specular_radiance(radiance: vec3<f32>, sky_mean: vec3<f32>, roughness: f32) -> vec3<f32> {
    let lum = dot(radiance, vec3<f32>(0.2126, 0.7152, 0.0722));
    let mean = max(dot(sky_mean, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-4);
    // 8x the sky's mean on rough ground, 256x for a mirror.
    let headroom = mix(8.0, 256.0, smoothstep(0.30, 0.0, roughness));
    let ceiling = mean * headroom;
    if lum <= ceiling {
        return radiance;
    }
    return radiance * (ceiling / max(lum, 1e-4));
}

fn evaluate_ibl_specular(surface: Surface) -> vec3<f32> {
    let n = surface.normal;
    let v = surface.view_dir;
    let n_dot_v = max(dot(n, v), 1e-4);
    let r = reflect(-v, n);
    let mip = surface.roughness * ENV_MAX_MIP;
    let sky_mean = textureSampleLevel(env_cube, env_sampler, n, ENV_MAX_MIP).rgb;
    let prefiltered = clamp_specular_radiance(
        textureSampleLevel(env_cube, env_sampler, r, mip).rgb, sky_mean, surface.roughness);
    let specular = prefiltered * env_brdf_approx(surface.f0, surface.roughness, n_dot_v);
    let spec_ao  = specular_occlusion(n_dot_v, surface.occlusion, surface.roughness);

    return specular * spec_ao;
}

/// Multiple-scattering IBL, Fdez-Agüera (JCGT 8(1), 2019).
///
/// No new LUT and no new parameter: built entirely from the `ab` pair
/// `env_brdf_ab` already returns. `Ems` is the energy the single-scatter term
/// failed to account for, `FmsEms` is the geometric series that puts it back,
/// and `kD` is what is *left* for diffuse once both specular terms have taken
/// their share.
///
/// That coupling is the reason this replaces `evaluate_ibl`'s body rather than
/// adding a term to it. The old split computed a diffuse lobe and a specular
/// lobe independently and lost the energy that should have passed between
/// them; you cannot recover that by adding something to the outside.
fn evaluate_ibl_ms(surface: Surface, traced_diffuse: vec4<f32>) -> vec3<f32> {
    let n = surface.normal;
    let v = surface.view_dir;
    let n_dot_v = max(dot(n, v), 1e-4);
    let ab = env_brdf_ab(surface.roughness, n_dot_v);

    // Roughness-dependent Fresnel: a rough surface's grazing response never
    // reaches the mirror value, and plain Schlick here is what gives rough
    // dielectrics a bright rim they should not have.
    let fr = max(vec3<f32>(1.0 - surface.roughness), surface.f0) - surface.f0;
    let k_s = surface.f0 + fr * pow(1.0 - n_dot_v, 5.0);

    let r = reflect(-v, n);
    let sky_mean = textureSampleLevel(env_cube, env_sampler, n, ENV_MAX_MIP).rgb;
    let radiance = clamp_specular_radiance(
        textureSampleLevel(env_cube, env_sampler, r, surface.roughness * ENV_MAX_MIP).rgb,
        sky_mean,
        surface.roughness,
    );

    // The same bent-normal gather and the same traced-diffuse override the
    // single-scatter path used, so TSUSHIMA-C's landscape-scale bent normal
    // and Phase 24L's ReSTIR result both still reach this.
    let gather_n = normalize(mix(n, surface.bent_normal, 0.75));
    var irradiance = textureSampleLevel(env_cube, env_sampler, gather_n, ENV_MAX_MIP).rgb;
    if traced_diffuse.a > 0.5 {
        irradiance = traced_diffuse.rgb;
    }

    let fss_ess = k_s * ab.x + ab.y;
    let ems = 1.0 - (ab.x + ab.y);
    let f_avg = surface.f0 + (vec3<f32>(1.0) - surface.f0) / 21.0;
    let fms_ems = ems * fss_ess * f_avg / (vec3<f32>(1.0) - f_avg * ems);
    let k_d = surface.albedo * (vec3<f32>(1.0) - fss_ess - fms_ems)
        * (1.0 - surface.metallic);

    let spec_ao = specular_occlusion(n_dot_v, surface.occlusion, surface.roughness);
    return fss_ess * radiance * spec_ao
        + (fms_ems + k_d) * irradiance * surface.occlusion;
}

fn evaluate_ibl(surface: Surface, traced_diffuse: vec4<f32>) -> vec3<f32> {
    if brdf_multiscatter {
        return evaluate_ibl_ms(surface, traced_diffuse) * light.ibl_intensity;
    }
    // Occlusion applies to indirect light only. The sun already has shadow
    // maps, and multiplying it by AO as well double-darkens lit surfaces.
    return (evaluate_ibl_diffuse(surface, traced_diffuse)
        + evaluate_ibl_specular(surface)) * light.ibl_intensity;
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
    out.clip_pos = vec4<f32>(x, y, tile_params.split_depth, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, 1.0 - (y * 0.5 + 0.5));
    return out;
}

// ─── Tile vertex shader (Phase DOOM-C) ───────────────────────────────────────
//
// One instanced quad per classified tile, replacing the fullscreen triangle
// when binned shading is on. The fragment shader below is unchanged and unaware
// — it reads `clip_pos.xy` for its pixel and `uv` for its ray, and both come out
// the same as they did from the fullscreen path, which is what makes the parity
// gate meaningful rather than a comparison of two rewrites.

struct TileParams {
    tiles_x: u32,
    tile_size: u32,
    /// First tile of this bin's slice of `tile_list`.
    bin_offset: u32,
    /// Viewport width in pixels, for the NDC conversion.
    width: u32,
    height: u32,
    /// Phase DOOM-E: clip-space depth of the aerial split, which `vs_main`
    /// emits so the depth test can decide which half of the screen this
    /// pipeline covers. Zero on the un-split path, which has no depth
    /// attachment and ignores it.
    split_depth: f32,
    _pad1: u32,
    _pad2: u32,
}

@group(3) @binding(0) var<storage, read> tile_list: array<u32>;
@group(3) @binding(1) var<uniform> tile_params: TileParams;

@vertex
fn vs_tile(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    let tile = tile_list[tile_params.bin_offset + instance_index];
    let tx = tile % tile_params.tiles_x;
    let ty = tile / tile_params.tiles_x;

    // Two triangles covering the tile. Written out rather than derived from bit
    // tricks on the vertex index: the fullscreen triangle above can afford to
    // be clever because it has three vertices and one job, and a wrong corner
    // here would show up as a shifted tile, which is a slow thing to debug.
    var corners = array<vec2<u32>, 6>(
        vec2<u32>(0u, 0u),
        vec2<u32>(1u, 0u),
        vec2<u32>(0u, 1u),
        vec2<u32>(0u, 1u),
        vec2<u32>(1u, 0u),
        vec2<u32>(1u, 1u),
    );
    let corner = corners[vertex_index];

    let px = vec2<f32>(
        f32((tx + corner.x) * tile_params.tile_size),
        f32((ty + corner.y) * tile_params.tile_size),
    );
    // Not clamped to the viewport: a tile on the right or bottom edge is
    // partly outside it, and the rasterizer discards those pixels for free.
    // Clamping instead would shrink the quad and leave the edge unshaded.
    let uv = px / vec2<f32>(f32(tile_params.width), f32(tile_params.height));

    var out: VertexOutput;
    out.clip_pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
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
    // exist without the 16+24 tap filter. `enable_pcss` is the compile-time
    // kill: a runtime bit still leaves the loops in the shader.
    if !enable_pcss || (cluster_params.shading_mode & 2u) == 0u {
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

/// Sparse-page lookup for the main opaque/terrain path.
///
/// Returns -1 when the page is absent. The caller then samples the resident
/// CSM atlas, making a page-budget miss a coarse shadow rather than a flash of
/// unshadowed geometry.
fn sample_virtual_shadow(
    world_pos: vec3<f32>,
    normal: vec3<f32>,
    view_depth: f32,
) -> f32 {
    if virtual_shadow.budget.z == 0u {
        return -1.0;
    }
    let side = virtual_shadow.geometry.x;
    let page_texels = virtual_shadow.geometry.y;
    let atlas_texels = virtual_shadow.geometry.z;
    let levels = virtual_shadow.geometry.w;
    if side == 0u || page_texels == 0u || atlas_texels == 0u || levels == 0u {
        return -1.0;
    }

    let level = min(get_cascade_index(view_depth), levels - 1u);
    // The physical-page raster uses the same normal-offset convention as CSM.
    let receiver = world_pos + normal * 0.01;
    let clip = light.view_proj[level] * vec4<f32>(receiver, 1.0);
    if abs(clip.w) <= 1e-6 {
        return -1.0;
    }
    let ndc = clip.xyz / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || ndc.z > 1.0 {
        return -1.0;
    }

    let virtual_texel = min(vec2<u32>(uv * f32(side)), vec2<u32>(side - 1u));
    let table_index = level * side * side + virtual_texel.y * side + virtual_texel.x;
    let physical = virtual_shadow_pages[table_index];
    if physical == 0xffffffffu {
        return select(1.0, -1.0, virtual_shadow.budget.w != 0u);
    }

    let physical_side = atlas_texels / page_texels;
    let tile = vec2<u32>(physical % physical_side, physical / physical_side);
    let local = fract(uv * f32(side));
    // Keep hardware bilinear filtering half a texel inside this tile so it
    // cannot read an unrelated neighbour from the physical pool.
    let half_texel = 0.5 / f32(page_texels);
    let page_uv = clamp(local, vec2<f32>(half_texel), vec2<f32>(1.0 - half_texel));
    let atlas_uv = (vec2<f32>(tile) + page_uv) / f32(physical_side);
    return textureSampleCompare(
        virtual_shadow_atlas,
        virtual_shadow_sampler,
        atlas_uv,
        ndc.z - 0.0002,
    );
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
    let sparse = sample_virtual_shadow(world_pos, normal, view_depth);
    if sparse >= 0.0 {
        var result = sparse;
        if enable_contact && (cluster_params.shading_mode & 4u) != 0u {
            result = min(result, contact_shadow(world_pos, normal, normalize(light.direction), pixel));
        }
        return result;
    }
    let cascade = get_cascade_index(view_depth);
    let near = select(light.cascade_splits[cascade - 1u], 0.0, cascade == 0u);
    let far = light.cascade_splits[cascade];

    var shadow = sample_shadow_cascade(world_pos, normal, cascade, pixel);

    // Contact shadows only ever darken. The shadow map is authoritative for
    // everything at its own scale; this fills in below that scale.
    // Bit 2 of shading_mode: contact march. Default on.
    if enable_contact && (cluster_params.shading_mode & 4u) != 0u {
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
    // Published for the terrain material's stochastic filtering, which needs a
    // screen-space dither index. See `terrain_screen_pixel`.
    terrain_screen_pixel = vec2<u32>(in.clip_pos.xy);
    micro_occlusion = 1.0;
    // Phase TSUSHIMA-F. Set once per pixel, read by `brdf.wgsl`, which cannot
    // reach `cluster_params` itself because it is composed into roots that do
    // not bind the cluster grid.
    brdf_multiscatter = (cluster_params.shading_mode & 64u) != 0u;
    brdf_rough_diffuse = (cluster_params.shading_mode & 128u) != 0u;
    brdf_micro_shadow = (cluster_params.shading_mode & 256u) != 0u;

    // Phase DOOM-C: the screen UV is *derived* from the fragment coordinate,
    // not taken from the interpolator.
    //
    // Both vertex shaders produce the same UV analytically, but not to the last
    // bit: interpolating across a triangle that spans the whole screen and
    // interpolating across an 8-pixel quad give answers a ULP or two apart. That
    // is invisible almost everywhere and decisive at a threshold — a mip level,
    // a hex-tile cell edge, a parallax step count — so the binned path differed
    // from the fullscreen one on 12 684 terrain pixels until this line existed.
    // `clip_pos.xy` is the pixel centre and is exact in both, so this is the
    // only formulation the two paths can agree on.
    let screen_uv = in.clip_pos.xy / vec2<f32>(textureDimensions(vis_buffer));
    let vis_texel    = textureLoad(vis_buffer, pixel_coords, 0).rg;
    let vis_data     = vis_texel.x;

    // ── Sky / background ────────────────────────────────────────────────────
    if vis_data == 0u {
        if shade_ablate != ABLATE_OFF && shade_ablate != ABLATE_SKY {
            return vec4<f32>(0.0, 0.0, 0.0, 1.0);
        }
        let ndc = (screen_uv * 2.0 - 1.0) * vec2<f32>(1.0, -1.0);
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

    // Phase DOOM-B ablation. Placed after the material fetch — which is the
    // earliest point the class is known — and before the triangle setup, so an
    // excluded pixel pays two buffer loads and nothing else. The same three
    // tests `census.wgsl` uses, deliberately: a census and an ablation that
    // disagreed about what "terrain" means would produce a cost table nobody
    // could use.
    if shade_ablate != ABLATE_OFF {
        var pixel_class = ABLATE_MESH;
        if material.terrain_index >= 0 {
            pixel_class = ABLATE_TERRAIN;
        } else if material.alpha_cutoff > 0.0 {
            pixel_class = ABLATE_FOLIAGE;
        }
        if pixel_class != shade_ablate {
            return vec4<f32>(0.0, 0.0, 0.0, 1.0);
        }
    }

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

    let target_ndc = (screen_uv * 2.0 - 1.0) * vec2<f32>(1.0, -1.0);
    var bary = vis_barycentric(ndc0, ndc1, ndc2, c0.w, c1.w, c2.w, target_ndc);

    let uv0 = vec2<f32>(v0.u, v0.v);
    let uv1 = vec2<f32>(v1.u, v1.v);
    let uv2 = vec2<f32>(v2.u, v2.v);
    let uv = uv0 * bary.x + uv1 * bary.y + uv2 * bary.z;

    // Phase 25N: analytic UV gradients. Implicit dpdx across a vis-buffer
    // 2×2 quad straddles unrelated triangles, so foliage mips jump per pixel.
    // Evaluate this triangle's barycentric at the neighbouring pixels instead.
    //
    // The neighbour reconstructs used to run even when Analytic Mips was off
    // (the result was then zeroed). That is two extra vis-buffer barycentrics
    // on every terrain pixel for a feature terrain does not use.
    var uv_ddx = vec2<f32>(0.0);
    var uv_ddy = vec2<f32>(0.0);
    let analytic_grad = (cluster_params.shading_mode & 8u) != 0u;
    if analytic_grad {
        let pixel = 1.0 / vec2<f32>(textureDimensions(vis_buffer));
        let bary_x = vis_barycentric(ndc0, ndc1, ndc2, c0.w, c1.w, c2.w, target_ndc + vec2<f32>(2.0 * pixel.x, 0.0));
        let bary_y = vis_barycentric(ndc0, ndc1, ndc2, c0.w, c1.w, c2.w, target_ndc + vec2<f32>(0.0, -2.0 * pixel.y));
        uv_ddx = (uv0 * bary_x.x + uv1 * bary_x.y + uv2 * bary_x.z) - uv;
        uv_ddy = (uv0 * bary_y.x + uv1 * bary_y.y + uv2 * bary_y.z) - uv;
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
        // Phase TSUSHIMA-F1: the *material-scale* half, kept separately.
        micro_occlusion = surface.occlusion;
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

    // ── Foliage: curved card normals (Phase 17E, re-gated in TSUSHIMA-J) ─────
    //
    // A leaf or blade drawn as a flat card shares one normal across its whole
    // width, so it lights as a flat plate — the tell that vegetation is
    // cardboard rather than a plant. Real leaves are curved, and a curved
    // surface fans its normals across its width.
    //
    // Ported from `SpartanEngine-master/data/shaders/g_buffer.hlsl`, its
    // "foliage curved normals": rotate the normal about the axis running along
    // the card by an angle taken from how far across the card's *width* the
    // pixel sits. Spartan carries a `width_percent` vertex attribute for this.
    // Somnium substitutes `uv.x`, and that substitution is only valid when the
    // material really is painted on cards whose UV runs 0..1 across one blade.
    //
    // # Why the gate is `FOLIAGE_CARD` and not `FOLIAGE`
    //
    // It was `FOLIAGE` for six phases, and every scanned plant in the palette
    // sets `FOLIAGE`. None of them are cards. A Poly Haven grass tuft is
    // seventeen modelled clusters sharing one *atlas*, so `uv.x` is the blade's
    // address in the texture — not a position across it — and neighbouring
    // blades whose art sits at opposite ends of the sheet were being bent up to
    // 120 degrees apart from one another. The result was a ground plane of
    // scattered normals: blotchy under a low sun, a sheet of white specular
    // sparkle under a moon, and wrong at every distance in between, which is
    // why no lighting change ever made it better.
    //
    // The flag is authored rather than detected because geometry cannot answer
    // it. A crossed-quad billboard *is* a card and has the same normal spread
    // as a modelled tuft that is not one.
    if (material.flags & 4u) != 0u {
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
        let tm_idx = u32(material.terrain_index);
        var terrain: TerrainSurface;
        // `enable_live_terrain` is checked first and is a pipeline override, so
        // when the clipmap owns every queued terrain the live branch is
        // statically dead and `evaluate_terrain_material` is dropped from the
        // module. The `clipmap_enabled` test below is a storage read the
        // compiler cannot prove uniform: it branches correctly at runtime, but
        // it cannot delete either body, and occupancy is the union of both.
        if !enable_live_terrain {
            terrain = evaluate_clipmap_material(
                terrain_materials[tm_idx], hit_point, geo_normal, uv, world_ddx, world_ddy);
        } else if enable_clipmap && terrain_materials[tm_idx].clipmap_enabled != 0u {
            terrain = evaluate_clipmap_material(
                terrain_materials[tm_idx], hit_point, geo_normal, uv, world_ddx, world_ddy);
        } else {
            terrain = evaluate_terrain_material(
                tm_idx, hit_point, geo_normal, uv, world_ddx, world_ddy);
        }
        surface.albedo = terrain.albedo;
        surface.roughness = terrain.roughness;
        surface.metallic = 0.0;
        surface.normal = terrain.normal;
        // Phase 25K: the material's own occlusion, folded into the screen-space
        // term the same way a glTF occlusion map is — the two know different
        // things, and GTAO cannot see detail below a pixel.
        surface.occlusion = surface.occlusion * terrain.occlusion;
        // Phase TSUSHIMA-F1: the terrain material's own AO, on its own. See
        // `micro_occlusion` for why GTAO must not be in here.
        micro_occlusion = terrain.occlusion;
        terrain_taps = terrain.taps;
        terrain_discarded = terrain.discarded;
        terrain_selected_rgb = terrain.selected_rgb;
        terrain_weight_rgb = terrain.weight_rgb;
        terrain_parallax_shadow_factor = terrain.parallax_shadow;

        // ── The relief normal that survives distance (TSUSHIMA-E) ──────────
        //
        // Coarse LODs skip vertices, so past the near field the shading normal
        // is a *point sample* of the heightfield's normal field every eighth
        // cell — aliased, not filtered. This replaces it with the properly
        // averaged normal at the pixel's own footprint, and puts the variance
        // that averaging discarded back as roughness.
        //
        // Cross-faded in over the same range TSUSHIMA-B hands over at, because
        // it is the same boundary for the same reason: inside it the mesh is
        // fine enough that its own normals are already the right answer.
        {
            let tm_e = terrain_materials[tm_idx];
            if tm_e.relief_map >= 0 {
                let splat_ddx = world_ddx * tm_e.inv_world_size;
                let splat_ddy = world_ddy * tm_e.inv_world_size;
                let relief = terrain_relief_normal(tm_e, uv, splat_ddx, splat_ddy);
                let d = distance(hit_point, view.camera_pos);
                let w = smoothstep(tm_e.relief_takeover * 0.5, tm_e.relief_takeover, d);
                if w > 0.0 {
                    // Blend the *shading* normal only. `resolve_surfgrad` has
                    // already resolved the layer normal maps against
                    // `geo_normal`, and the geometric normal itself still
                    // drives shadow bias and the surface-gradient frame — this
                    // must not disturb either.
                    surface.normal = normalize(mix(surface.normal, relief.xyz, w));
                    surface.roughness = mix(
                        surface.roughness,
                        widen_roughness_toksvig(surface.roughness, relief.w),
                        w,
                    );
                }
            }
        }

        // ── Baked terrain visibility (Phases TSUSHIMA-B, TSUSHIMA-C) ────────
        //
        // Applied here rather than inside either material function, because
        // both quantities are properties of the heightfield and not of the
        // material. Putting them here means the live path, the clipmap path
        // and the virtual-texture path all get them from one call site
        // instead of three that would drift.
        let tm = terrain_materials[tm_idx];

        // B. Folded into the same channel the relief self-shadow uses, so
        // there is one definition of "how much direct sun reaches here" and
        // `shadow_factor` below picks up both without knowing about either.
        //
        // Cross-faded against the cascades over the last cascade's range
        // rather than replacing them. Inside 100 m the cascades are strictly
        // better — they see meshes, and the horizon map only ever sees
        // terrain, so a rock casting onto the ground exists in one and not
        // the other. Past it they see nothing at all.
        let horizon = terrain_horizon_shadow(
            tm, uv, normalize(light.direction), light.sun_angular_radius);
        let horizon_takeover = smoothstep(70.0, 100.0, distance(hit_point, view.camera_pos));
        terrain_parallax_shadow_factor =
            terrain_parallax_shadow_factor * mix(1.0, horizon, horizon_takeover);

        // C. A **product** against the occlusion already accumulated, not a
        // `min`.
        //
        // The plan called this one either way and said to measure it, so it
        // was measured. `min` was tried first, on the argument that two
        // occlusion terms describing the same hemisphere should not both
        // apply. It is nearly invisible: it can only bite where baked sky
        // visibility is *lower* than GTAO's answer, which on this terrain is
        // the floor of a deep valley and 1.4% of visible pixels — a mean
        // absolute change of 0.17 against a terrain radiance of 1440.
        //
        // The product is right because the two terms are very nearly
        // independent. GTAO searches a few metres and cannot see a ridge line;
        // the bake sees the ridge line and has no idea a boulder is sitting
        // next to you. They occlude different parts of the sky, and the
        // fraction of sky surviving two independent occluders is the product
        // of the two fractions. `min` is the correct composition only when one
        // term's occluders are a superset of the other's, which is exactly
        // what these two are not.
        let sky_vis = terrain_sky_visibility(tm, uv);

        // ── Ground that knows where the water went (TSUSHIMA-H2) ───────────
        //
        // The macro octaves in `terrain_material.wgsl` are noise: variance at
        // the right scales, meaning nothing. This is the term that makes the
        // difference between noise and a landscape — one band of the tint
        // driven by something the ground actually is. Sheltered ground holds
        // water and the organic matter that comes with it and reads damp and
        // green; open ground drains, bleaches and reads dry.
        //
        // Driven from C's bake rather than from slope, because slope cannot
        // tell a valley floor from a plain and sky visibility can: they have
        // the same normal and very different drainage.
        //
        // It lives here, not in `evaluate_terrain_material`, for the reason
        // the block below states about B and C — `sky_vis` is already in hand
        // at this one call site, and putting the tint in the material would
        // mean a *second* fetch of the same texture in the live path and a
        // third copy of the rule in the clipmap and virtual-texture paths.
        // The cost of that placement is that the tint is a multiply on linear
        // albedo rather than on the perceptual value the octaves use, which is
        // a reparametrisation of the strength and nothing more.
        //
        // The remap is not `1 - sky_vis`, and its lower edge is not the map's
        // *minimum* either. TSUSHIMA-C measured 0.47 to 1.00 with a mean of
        // 0.93, and the first cut read that as "the useful band is 0.99 down to
        // 0.78". That was reading the range instead of the distribution: 0.47
        // is one valley floor, almost every visible pixel sits within a few
        // hundredths of 1.0, and a remap stretched to the minimum leaves the
        // term multiplying by two percent over the whole frame. Measured, it
        // moved 1,604 of 705,355 terrain pixels.
        //
        // The upper edge is exactly 1.0 so genuinely open ground is the exact
        // identity, and the lower edge sits just below the mean, which is where
        // the pixels that differ from open ground actually are.
        let sheltered = smoothstep(1.0, 0.88, sky_vis) * tm.macro_octave_strength.w;
        // Luminance ~0.94, so this is mostly a hue shift and only slightly a
        // darkening. C already darkens sheltered ground through `occlusion`
        // below, and that is a transport term; this one is what the ground is
        // made of. Both are true, but stacking two full-strength darkenings on
        // the same pixels would read as a painted-on shadow.
        surface.albedo = surface.albedo * mix(vec3(1.0), vec3(0.88, 0.97, 0.78), sheltered);

        surface.occlusion = surface.occlusion * sky_vis;
        // The landscape-scale bent normal, **composed with** the contact-scale
        // one GTAO already wrote rather than replacing it. The two see
        // different occluders — GTAO sees the boulder a metre away and cannot
        // see the valley wall, the bake sees the valley wall and cannot see
        // the boulder — so overwriting one with the other throws away half the
        // answer. Summing two unit directions and renormalising is the cheap
        // composition of their two visibility cones, and it degrades correctly:
        // where one is unoccluded it is vertical and contributes nothing but
        // height.
        //
        // `evaluate_ibl_diffuse` already gathers along `surface.bent_normal`
        // at a 0.75 mix and needs no change at all, which is why this term
        // costs so little to add. Pulled a quarter of the way back toward the
        // geometric normal so a deep valley still shades as ground rather than
        // as a wall.
        if terrain_sky_bent.y > 0.0 {
            let composed = normalize(surface.bent_normal + terrain_sky_bent);
            surface.bent_normal = normalize(mix(composed, geo_normal, 0.25));
        }
    }


    surface.view_dir = normalize(view.camera_pos - hit_point);
    // CONTROL-O. Before `f0` is derived, because a decal changes base colour
    // and metallic and `f0` is a function of both — deriving it first and
    // then painting over the albedo would leave a decal with the surface's
    // Fresnel response instead of its own.
    if decal_params.x > 0u {
        let decal_view_pos = view.view * vec4<f32>(hit_point, 1.0);
        let decal_tile = vec2<u32>(in.clip_pos.xy) / vec2(cluster_params.tile_size);
        let decal_slice = compute_depth_slice(max(-decal_view_pos.z, 0.0));
        let decal_froxel = decal_tile.x
            + decal_tile.y * cluster_params.grid_width
            + decal_slice * cluster_params.grid_width * cluster_params.grid_height;
        apply_decals(&surface, hit_point, decal_froxel);
    }

    // ── Geometric specular antialiasing (TSUSHIMA-E) ─────────────────────────
    //
    // Tokuyoshi & Kaplanyan, I3D 2019. The filter kernel comes from the
    // screen-space derivatives of the shading normal, and turns normal
    // variance the pixel cannot resolve into roughness it can — which is the
    // only correct answer to a specular lobe narrower than a pixel.
    //
    // KAPPA clamps how far it may go. Without the clamp a silhouette edge —
    // where the normal derivative is enormous and meaningless — turns the
    // surface fully rough in a one-pixel band.
    //
    // **Position is the whole correctness argument, and the first version got
    // it wrong.** It sat above the terrain branch, which then overwrote
    // `surface.normal` and `surface.roughness` outright — so every terrain
    // pixel computed the filter and threw it away, and the relief normal and
    // decals overwrote it again after that. The comment there claimed it ran
    // "after every other write", which was the intent and not the code.
    //
    // It now runs after terrain, relief, wetness and decals: the last point
    // where anything writes the normal or the roughness, and before `f0` is
    // derived from them. `dpdx` is also well defined here, which it would not
    // be inside the terrain branch — that is a storage read the compiler
    // cannot prove uniform, and a derivative taken in non-uniform control flow
    // is undefined.
    //
    // `specular_aa_runs_after_every_normal_and_roughness_writer` pins the
    // ordering so this cannot silently regress again.
    if enable_specular_aa {
        let dndx = dpdx(surface.normal);
        let dndy = dpdy(surface.normal);
        let variance = SPEC_AA_SIGMA2 * (dot(dndx, dndx) + dot(dndy, dndy));
        let kernel = min(2.0 * variance, SPEC_AA_KAPPA);
        let alpha = surface.roughness * surface.roughness;
        surface.roughness = sqrt(sqrt(saturate(alpha * alpha + kernel)));
    }

    surface.f0       = mix(vec3<f32>(0.04), surface.albedo, surface.metallic);
    if material.terrain_index >= 0 {
        surface.f0 = surface.f0 + vec3<f32>(terrain_wet_f0);
    } else {
        // CONTROL-N. Meshes only: terrain already has XV-H's own wetness
        // path, driven by the same weather state one level up, and applying
        // both would darken the ground twice.
        apply_wetness(&surface, material.porosity);
    }

    // ── Shadow factor ────────────────────────────────────────────────────────
    // View-space depth: positive Z distance from camera.
    let view_pos   = view.view * vec4<f32>(hit_point, 1.0);
    let view_depth = -view_pos.z; // right-handed: Z is negative in front of camera

    // Phase 24K: prefer the traced result where it exists. It has no cascades,
    // no depth bias and no peter-panning, and its penumbra comes from the sun's
    // actual angular size rather than from a filter chosen to look about right.
    //
    // Bit 4 is a *uniform* "ReSTIR wrote this frame". Branching on
    // `textureLoad(restir_vis).a` instead compiled both paths into every
    // wavefront: DXC flattens varying texture-alpha tests, so PCSS's 16
    // blocker loads + 24 compares (and the 12-step contact march inside
    // `sample_shadow`) still ran on every terrain pixel. That is why turning
    // hex/POM off did not move Shading ms — those were not the 40 ms.
    let traced = textureLoad(restir_vis, pixel_coords, 0);
    var shadow_factor: f32;
    if (cluster_params.shading_mode & 16u) != 0u {
        shadow_factor = traced.r * terrain_parallax_shadow_factor;
    } else {
        shadow_factor = sample_shadow(hit_point, shadow_normal, view_depth, in.clip_pos.xy)
            * terrain_parallax_shadow_factor;
    }
    // Phase TSUSHIMA-F1: micro-shadowing. Folded in here for the same reason
    // the relief self-shadow is — one definition of how much direct light
    // reaches this point, which every direct term below then reads without
    // having to know what went into it.
    //
    // Uses `surface.occlusion`, which by this point carries GTAO, the
    // material's own AO and (on terrain) TSUSHIMA-C's sky visibility. That
    // last one is arguably wrong: sky visibility is a landscape-scale
    // quantity and micro-shadowing is about relief below the pixel footprint,
    // so a valley floor now gets a little direct-light occlusion for a reason
    // that has nothing to do with micro-relief. It is small, it is in the
    // right direction, and separating the two means carrying a second
    // occlusion channel through the whole pass — noted rather than done.
    // Deliberately **not** on foliage.
    //
    // Micro-shadowing describes relief below the pixel footprint, and it reads
    // the material's AO map to find it. On a solid surface that map is exactly
    // that. On foliage it is not: a grass tuft's occlusion map encodes the
    // shade of the tuft's own *interior* at card scale — the shader says so
    // where it samples it — and it is already doing its job on the ambient
    // term. Feeding it to a hard cutoff on direct light darkens foliage a
    // second time for a reason that has nothing to do with micro-relief, and
    // at a grazing sun, where `N.L` is small over everything, that is most of
    // the foliage's direct lighting gone. What is left is the translucency and
    // ambient terms, which is why it reads as a flat uniform wash rather than
    // as lit grass.
    //
    // This is the third correction to what this term is fed — sky visibility,
    // then GTAO, now foliage AO. The pattern is the same every time: a hard
    // cutoff is only as well behaved as the field it thresholds, and every
    // occlusion channel in this renderer measures something different.
    if brdf_micro_shadow && (material.flags & 2u) == 0u {
        let ms_ndl = saturate(dot(surface.normal, normalize(light.direction)));
        shadow_factor = shadow_factor
            * micro_shadow(ms_ndl, micro_occlusion, micro_shadow_opacity);
    }

    // CONTROL-M. Folded into `shadow_factor` rather than applied separately,
    // so every consumer of the sun's visibility — direct, transmitted and the
    // cel path — picks it up from one place and none of them can be missed.
    shadow_factor *= cloud_shadow_at(hit_point);

    // 9 = albedo, 10 = shading normal, 11 = terrain_index as a flag.
    // Material-path probes: a surface that renders black is either unlit or
    // untextured, and only looking at the channels separately tells which.
    let dbg = select(0.0, light._pad2_z, enable_debug);
    if dbg > 8.5 && dbg < 9.5 {
        return vec4<f32>(surface.albedo, 1.0);
    }
    if dbg > 9.5 && dbg < 10.5 {
        return vec4<f32>(surface.normal * 0.5 + 0.5, 1.0);
    }
    if dbg > 10.5 && dbg < 11.5 {
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
    if dbg > 7.5 && dbg < 8.5 {
        return vec4<f32>(vec3<f32>(surface.occlusion), 1.0);
    }

    // 12 = terrain layer taps as a fraction of the 36-tap worst case
    // (Phase XV-D). Written straight to the HDR target before exposure, so the
    // capture harness's mean terrain luminance times TERRAIN_MAX_TAPS *is* the
    // mean taps per pixel — which is how the detail budget gets a number
    // instead of a claim.
    if dbg > 11.5 && dbg < 12.5 {
        return vec4<f32>(vec3<f32>(f32(terrain_taps) / TERRAIN_MAX_TAPS), 1.0);
    }
    // 32 = clipmap albedo (Phase DF). Same as mode 9 on terrain when the
    // cache is on; black on non-terrain so a missed bind is obvious.
    if dbg > 31.5 && dbg < 32.5 {
        return vec4<f32>(surface.albedo, 1.0);
    }
    // 33 = clipmap ring index, 0 = finest.
    if dbg > 32.5 && dbg < 33.5 {
        return vec4<f32>(vec3<f32>(terrain_clipmap_ring), 1.0);
    }
    // 34 = which stage of the clipmap stack produced the pixel. Flat colours,
    // not a ramp: the question is categorical, and a ramp is what made mode 33
    // unable to separate the outermost ring from no ring at all.
    if dbg > 33.5 && dbg < 34.5 {
        let src = terrain_clipmap_source;
        if src < -0.5 { return vec4<f32>(0.05, 0.05, 0.05, 1.0); }  // not clipmap
        if src < 0.5 { return vec4<f32>(0.10, 0.80, 0.25, 1.0); }   // detail ring
        if src < 1.5 { return vec4<f32>(0.20, 0.45, 1.00, 1.0); }   // macro ring
        if src < 2.5 { return vec4<f32>(1.00, 0.15, 0.10, 1.0); }   // macro-map fallback
        return vec4<f32>(1.00, 0.95, 0.10, 1.0);                    // constant colour
    }

    // 13 = terrain chunk LOD. Rust places lod+1 in the instance padding only
    // while this debug view is active; zero means a non-terrain instance.
    if dbg > 12.5 && dbg < 13.5 {
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
    if dbg > 13.5 && dbg < 14.5 {
        let edge = 1.0 - smoothstep(0.005, 0.025, min(bary.x, min(bary.y, bary.z)));
        return vec4<f32>(vec3<f32>(edge * 4.0), 1.0);
    }
    // 15/16 = interpolated geometric normal / actual receiver-bias normal.
    if dbg > 14.5 && dbg < 15.5 {
        return vec4<f32>(geo_normal * 0.5 + 0.5, 1.0);
    }
    if dbg > 15.5 && dbg < 16.5 {
        return vec4<f32>(shadow_normal * 0.5 + 0.5, 1.0);
    }
    // 17 = screen-space contact-shadow factor before cascade composition.
    if dbg > 16.5 && dbg < 17.5 {
        let contact = contact_shadow(hit_point, shadow_normal, normalize(light.direction), in.clip_pos.xy);
        return vec4<f32>(vec3<f32>(contact), 1.0);
    }
    // 18 = splat weight discarded by strongest-four (XV-D).
    if dbg > 17.5 && dbg < 18.5 {
        return vec4<f32>(vec3<f32>(terrain_discarded * 4.0), 1.0);
    }
    // 19 = first three selected layer indices, 0..1 over layers 0–15.
    if dbg > 18.5 && dbg < 19.5 {
        return vec4<f32>(terrain_selected_rgb, 1.0);
    }
    // 20 = raw strongest-four weights of the first three selected layers.
    if dbg > 19.5 && dbg < 20.5 {
        return vec4<f32>(terrain_weight_rgb, 1.0);
    }
    // 21 = dominant selected-layer albedo (solo).
    if dbg > 20.5 && dbg < 21.5 {
        return vec4<f32>(terrain_dominant_albedo, 1.0);
    }
    // 22 = cliff projection blend.
    if dbg > 21.5 && dbg < 22.5 {
        return vec4<f32>(vec3<f32>(terrain_cliff_blend_dbg), 1.0);
    }
    // 23 = wetness factor (moisture affinity × global wetness).
    if dbg > 22.5 && dbg < 23.5 {
        return vec4<f32>(vec3<f32>(terrain_wetness_factor), 1.0);
    }

    // Lighting debug (SOMNIUM_SHADOW_DEBUG): 1 = shadow factor.
    if dbg > 0.5 && dbg < 1.5 {
        return vec4<f32>(vec3<f32>(shadow_factor), 1.0);
    }
    // 6 = final shadow_factor in hue, immune to exposure.
    //   green = shadowed (< 0.5), red = lit (>= 0.5)
    if dbg > 5.5 && dbg < 6.5 {
        if shadow_factor < 0.5 { return vec4<f32>(0.0, 4.0, 0.0, 1.0); }
        return vec4<f32>(4.0, 0.0, 0.0, 1.0);
    }
    // 5 = blocker_search verdict at this fragment, in hue.
    //   red   = search found no blocker (PCSS early-returns lit)
    //   green = search found one (a shadow should appear here)
    if dbg > 4.5 && dbg < 5.5 {
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
    if dbg > 3.5 && dbg < 4.5 {
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
        // Phase TSUSHIMA-H: the sun's lobe is bounded before it is scaled by
        // the sun's illuminance, so the ceiling is a fraction of the light
        // actually arriving rather than an absolute number that would mean
        // something different at noon and at dusk.
        var direct_light = clamp_specular_lobe(
            evaluate_brdf_area(surface, light_dir, light.sun_angular_radius),
            surface.roughness,
        ) * light_color * shadow_factor + moonlight;

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
            // Probe misses carry no environment energy, so this is only the
            // bounced diffuse term. Base diffuse/specular IBL remains intact.
            let probe_diffuse = max(sample_sh_probes(hit_point, gather_n), vec3<f32>(0.0))
                * surface.albedo * kd * surface.occlusion;
            ambient += probe_diffuse * light.ibl_intensity
                * sh_probe_volume_weight(hit_point);
        } else if (extra_flags & 1u) != 0u {
            let kd = (vec3<f32>(1.0) - surface.f0) * (1.0 - surface.metallic);
            ambient += vol_sample.rgb * surface.albedo * kd * lighting_extra.y * surface.occlusion;
        }
        // SDF owns volume alpha; the world cache writes occupancy there, so
        // the two cannot run as one field. Cache-on skips the cone-trace.
        if (extra_flags & 8u) != 0u && (extra_flags & 1u) == 0u {
            // March in cell units: a 0.15 m first step saturates against 2 m
            // voxels and the cone never occludes. Start inside the first cell
            // so a cube on the ground actually darkens the terrain around it.
            let cell = max(lighting_extra.z, 0.25);
            var sdf_ao = 1.0;
            var march = cell * 0.4;
            for (var s = 0u; s < 6u; s++) {
                let p = hit_point + geo_normal * march;
                let d = textureSampleLevel(world_volume, volumetric_sampler, world_volume_uvw(p), 0.0).a;
                sdf_ao = min(sdf_ao, saturate(d / max(march * 0.5, 1e-3)));
                march *= 1.55;
            }
            ambient *= sdf_ao;
            direct_light *= mix(1.0, sdf_ao, 0.45);
        }
        if (extra_flags & 2u) != 0u {
            let aux_uv = (vec2<f32>(pixel_coords) + 0.5) / vec2<f32>(textureDimensions(vis_buffer));
            let spec_gi = textureSampleLevel(lighting_aux, default_sampler, aux_uv, 0.0);
            let spec_w = spec_gi.a * saturate(1.0 - surface.roughness);
            let n_dot_v = max(dot(surface.normal, surface.view_dir), 1e-4);
            let traced_spec = spec_gi.rgb
                * env_brdf_approx(surface.f0, surface.roughness, n_dot_v)
                * specular_occlusion(n_dot_v, surface.occlusion, surface.roughness);
            let baseline_spec = evaluate_ibl_specular(surface) * light.ibl_intensity;
            // Replace only the environment specular lobe. The old whole-
            // ambient mix discarded diffuse illumination and treated raw hit
            // radiance as an already-evaluated BRDF, bleaching boats/terrain.
            ambient += (traced_spec - baseline_spec) * spec_w;
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
                local_light_contrib += clamp_specular_lobe(
                    evaluate_brdf_area(surface, L, angular), surface.roughness,
                ) * ll.color * atten_val;
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
        if dbg > 6.5 && dbg < 7.5 {
            let ld = dot(direct_light, vec3<f32>(0.2126, 0.7152, 0.0722));
            let la = dot(ambient, vec3<f32>(0.2126, 0.7152, 0.0722));
            if surface.metallic > 0.5 { return vec4<f32>(0.0, 0.0, 4.0, 1.0); }
            if ld > la { return vec4<f32>(0.0, 4.0, 0.0, 1.0); }
            return vec4<f32>(4.0, 0.0, 0.0, 1.0);
        }

        // 2 = sun only, 3 = ambient only. Isolates which term a surface's
        // brightness actually comes from.
        if dbg > 1.5 && dbg < 2.5 {
            result = direct_light;
        } else if dbg > 2.5 && dbg < 3.5 {
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
            volumetrics, volumetric_sampler, vec3<f32>(screen_uv, w), 0.0);

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
    if dbg > 23.5 && dbg < 31.5 {
        let luma = dot(result, vec3<f32>(0.2126, 0.7152, 0.0722));
        if dbg < 24.5 {
            let t = saturate(log2(luma + 1.0) / 10.0);
            return vec4<f32>(t, 0.2 * (1.0 - t), 1.0 - t, 1.0) * 4.0;
        }
        if dbg < 25.5 {
            let gi = textureLoad(restir_gi, pixel_coords, 0);
            return vec4<f32>(gi.rgb * 4.0, 1.0);
        }
        if dbg < 26.5 {
            let tile = vec2<u32>(in.clip_pos.xy) / vec2(cluster_params.tile_size);
            let slice = compute_depth_slice(view_depth);
            let idx = tile.x + tile.y * cluster_params.grid_width
                + slice * cluster_params.grid_width * cluster_params.grid_height;
            let n = f32(cluster_offsets[idx].count) / 8.0;
            return vec4<f32>(n, 0.15, 1.0 - saturate(n), 1.0) * 4.0;
        }
        if dbg < 27.5 {
            let vol_dbg = textureSampleLevel(
                world_volume, volumetric_sampler, world_volume_uvw(hit_point), 0.0);
            return vec4<f32>(vol_dbg.rgb * 4.0, 1.0);
        }
        if dbg < 28.5 {
            return vec4<f32>(textureSampleLevel(lighting_aux, default_sampler, screen_uv, 0.0).rgb * 4.0, 1.0);
        }
        if dbg < 29.5 {
            let vol_dbg = textureSampleLevel(
                world_volume, volumetric_sampler, world_volume_uvw(hit_point), 0.0);
            return vec4<f32>(vec3<f32>(vol_dbg.a * 0.1), 1.0);
        }
        if dbg < 30.5 {
            let mip = length(uv_ddx) * 64.0;
            return vec4<f32>(mip, mip * 0.4, 0.1, 1.0) * 4.0;
        }
        return vec4<f32>(textureSampleLevel(lighting_aux, default_sampler, screen_uv, 0.0).rgb, 1.0);
    }

    if (bitcast<u32>(lighting_extra.x) & 4u) != 0u {
        let traced = textureSampleLevel(lighting_aux, default_sampler, screen_uv, 0.0);
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
