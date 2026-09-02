// Somnium Engine - PBR BRDF Library
// Ported/Inspired by SpartanEngine (Panos Karabelas)

const PI: f32 = 3.14159265359;
const INV_PI: f32 = 0.31830988618;
const FLT_MIN: f32 = 1.175494351e-38;

struct Surface {
    albedo: vec3<f32>,
    roughness: f32,
    metallic: f32,
    normal: vec3<f32>,
    view_dir: vec3<f32>,
    f0: vec3<f32>,
    /// Baked ambient occlusion (glTF/ARM red channel). 1.0 = fully open sky.
    occlusion: f32,
    /// Average unoccluded direction, world space (Phase 24I). Falls back to the
    /// surface normal where nothing occludes.
    bent_normal: vec3<f32>,
};

struct AngularInfo {
    n_dot_l: f32,
    n_dot_v: f32,
    n_dot_h: f32,
    v_dot_h: f32,
    l_dot_h: f32,
    /// Phase TSUSHIMA-F: Hammon's diffuse needs it, and it is one dot product
    /// on two vectors already in registers.
    l_dot_v: f32,
};

fn get_angular_info(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>) -> AngularInfo {
    let h = normalize(v + l);
    var info: AngularInfo;
    info.n_dot_l = saturate(dot(n, l));
    info.n_dot_v = saturate(dot(n, v));
    info.n_dot_h = saturate(dot(n, h));
    info.v_dot_h = saturate(dot(v, h));
    info.l_dot_h = saturate(dot(l, h));
    info.l_dot_v = dot(l, v);
    return info;
}

// ── Phase TSUSHIMA-F ─────────────────────────────────────────────────────────
//
// Four terms, and the reason they are here rather than in `shading.wgsl` is
// that three of them belong to the BRDF and the fourth needs the split-sum
// pair the BRDF's environment term already computes.
//
// The prompt that started TSUSHIMA asked whether the BRDF could be improved.
// It can, and this is that work — but it is the fifth-largest thing wrong with
// the terrain, not the first, which is why B, C and D came before it.

/// Whether the TSUSHIMA-F energy terms are live this frame.
///
/// A private flag rather than a read of `cluster_params`, because this module
/// is composed into five roots — shading, the two ReSTIR passes, the ray-hit
/// shader and the clipmap — and not all of them bind the cluster grid. A
/// module that reaches for a binding its host may not have is a module that
/// compiles in one root and fails in another, which is the mistake
/// `TERRAIN_PI` was added to avoid in TSUSHIMA-B.
///
/// Defaults to **off**, so a root that never sets it keeps exactly the
/// response it had. `shading.wgsl` turns them on per frame.
///
/// Three flags rather than one, because the three terms pull in different
/// directions and a single switch cannot say which did what. Measured
/// together they moved terrain radiance 39%; measured apart, two of them are
/// small and one is not, and that is a fact worth being able to recover.
var<private> brdf_multiscatter: bool = false;
var<private> brdf_rough_diffuse: bool = false;
var<private> brdf_micro_shadow: bool = false;

/// The split-sum scale/bias pair — Karis' mobile approximation, via Lazarov.
///
/// Split out from `env_brdf_approx` (which now calls it) because `ab` *itself*
/// is what every multiple-scattering term needs, and the old signature
/// computed it one line before throwing it away.
fn env_brdf_ab(roughness: f32, n_dot_v: f32) -> vec2<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    return vec2<f32>(-1.04, 1.04) * a004 + r.zw;
}

/// Restore the energy single-scatter GGX loses between microfacets.
///
/// Filament's form. At r = 0.2 it is a couple of percent; at the r ≈ 0.85 that
/// is all of this phase's subject it is large, and because the loss is
/// roughness-*dependent* it does not merely darken the ground — it flattens
/// the difference between a rough patch and a smooth one, which is a real part
/// of why terrain reads as clay.
fn energy_compensation(f0: vec3<f32>, ab: vec2<f32>) -> vec3<f32> {
    return vec3<f32>(1.0) + f0 * (1.0 / max(ab.x + ab.y, 1e-4) - 1.0);
}

/// Occlusion applied to *direct* light.
///
/// Ref: "The Technical Art of Uncharted 4", Brinck & Maximov, GDC 2016,
/// transcribed from Unity HDRP's `ComputeMicroShadowing`, which cites it.
///
/// `evaluate_ibl`'s comment is right that AO belongs to indirect light — the
/// sun has shadow maps and multiplying it by AO as well double-darkens. But
/// the consequence of leaving direct light entirely un-occluded is that a
/// crevice lit by the sun reads perfectly flat, because nothing below the
/// pixel's own footprint can shadow it. `aperture` is how wide a cone the
/// surface still sees the light through; subtracting one turns it into a
/// cutoff that vanishes on a surface facing the light and bites hardest at
/// grazing incidence, which is exactly where sub-pixel relief self-shadows.
fn micro_shadow(n_dot_l: f32, ao: f32, opacity: f32) -> f32 {
    let aperture = 2.0 * ao * ao;
    let shadowed = saturate(n_dot_l + aperture - 1.0);
    return mix(1.0, shadowed, opacity);
}

/// Hammon's GGX-consistent diffuse.
///
/// Ref: Earl Hammon, Jr., "PBR Diffuse Lighting for GGX+Smith Microsurfaces",
/// GDC 2017, slide 113. The 1.05 is not a fudge — slide 108 derives it as the
/// exact normalisation 21/(20·π) for a Fresnel-symmetric diffuse lobe, and
/// notes it is "just 5% larger than the pure Lambertian BRDF". 0.1159 is the
/// fitted multiple-scattering coefficient.
///
/// What it buys over Burley on this content is **retroreflection**: a rough
/// mineral surface bounces light back toward the source, which is why a dirt
/// track brightens when you stand with the sun behind you and why Burley's
/// ground never does.
///
/// The slide's last line extracts from the PDF as `albedo·single + albedo·multi`,
/// but superscripts do not survive text extraction and the second factor is
/// `albedo²`: a multiple-scattering lobe carries one extra albedo factor per
/// bounce, and EON's independent derivation of the same quantity (JCGT 14(1)
/// Eq. 18) has exactly the same ρ² for exactly that reason.
fn diffuse_hammon(
    albedo: vec3<f32>,
    roughness: f32,
    n_dot_l: f32,
    n_dot_v: f32,
    n_dot_h: f32,
    l_dot_v: f32,
) -> vec3<f32> {
    let alpha = roughness * roughness;
    let facing = 0.5 + 0.5 * l_dot_v;
    let rough = facing * (0.9 - 0.4 * facing) * ((0.5 + n_dot_h) / max(n_dot_h, 1e-4));
    let smooth_t = 1.05
        * (1.0 - pow(1.0 - n_dot_l, 5.0))
        * (1.0 - pow(1.0 - n_dot_v, 5.0));
    let single = mix(smooth_t, rough, alpha) * INV_PI;
    let multi = 0.1159 * alpha;
    return albedo * (vec3<f32>(single) + albedo * multi);
}

// FRESNEL - Schlick approximation
fn F_Schlick(f0: vec3<f32>, v_dot_h: f32) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(saturate(1.0 - v_dot_h), 5.0);
}

// DISTRIBUTION - Trowbridge-Reitz GGX
fn D_GGX(n_dot_h: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d + FLT_MIN);
}

// VISIBILITY - Smith Joint GGX
fn V_SmithGGX(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let ggxv = n_dot_l * sqrt(n_dot_v * n_dot_v * (1.0 - a2) + a2);
    let ggxl = n_dot_v * sqrt(n_dot_l * n_dot_l * (1.0 - a2) + a2);
    return 0.5 / max(ggxv + ggxl, FLT_MIN);
}

// DIFFUSE - Burley (Disney)
fn Diffuse_Burley(albedo: vec3<f32>, roughness: f32, n_dot_v: f32, n_dot_l: f32, v_dot_h: f32) -> vec3<f32> {
    let f90 = 0.5 + 2.0 * v_dot_h * v_dot_h * roughness;
    let light_scatter = 1.0 + (f90 - 1.0) * pow(1.0 - n_dot_l, 5.0);
    let view_scatter = 1.0 + (f90 - 1.0) * pow(1.0 - n_dot_v, 5.0);
    return albedo * (light_scatter * view_scatter * INV_PI);
}

// Final BRDF Evaluation (Simplified for Isotropic)
/// BRDF for an area light of angular radius `angular_radius` (Phase 24E).
///
/// The sun subtends 0.53°, so its highlight has area. Widening the specular
/// lobe's roughness by the source's angular size spreads the highlight to match
/// (Karis' sphere-light approximation), and the energy factor keeps the lobe's
/// total reflected light constant so spreading it does not also brighten it.
///
/// The correction applies to the **specular term only**. Diffuse reflection
/// does not care how large the source is, only how much light arrives, so
/// scaling it here would darken every lit surface as a side effect.
fn evaluate_brdf_area(surface: Surface, l: vec3<f32>, angular_radius: f32) -> vec3<f32> {
    let angular = get_angular_info(surface.normal, surface.view_dir, l);

    let alpha = surface.roughness * surface.roughness;
    let widened = clamp(alpha + angular_radius * 0.5, alpha, 1.0);
    let energy = alpha / max(widened, 1e-4);
    let spec_roughness = sqrt(widened);

    let D = D_GGX(angular.n_dot_h, spec_roughness);
    let V = V_SmithGGX(angular.n_dot_v, angular.n_dot_l, spec_roughness);
    let F = F_Schlick(surface.f0, angular.v_dot_h);
    var Fr = D * V * F * energy;

    let kS = F;
    let kD = (vec3<f32>(1.0) - kS) * (1.0 - surface.metallic);
    var Fd = Diffuse_Burley(
        surface.albedo, surface.roughness,
        angular.n_dot_v, angular.n_dot_l, angular.v_dot_h,
    );

    // Phase TSUSHIMA-F. One test, two terms, applied to the sun and every
    // local light alike — the sun is not a special case for how a surface
    // reflects, only for how it is shadowed.
    if brdf_multiscatter {
        Fr = Fr * energy_compensation(
            surface.f0, env_brdf_ab(spec_roughness, angular.n_dot_v));
    }
    if brdf_rough_diffuse {
        Fd = diffuse_hammon(
            surface.albedo, surface.roughness,
            angular.n_dot_l, angular.n_dot_v, angular.n_dot_h, angular.l_dot_v,
        );
    }

    return (kD * Fd + Fr) * angular.n_dot_l;
}

fn evaluate_brdf(surface: Surface, l: vec3<f32>) -> vec3<f32> {
    let angular = get_angular_info(surface.normal, surface.view_dir, l);

    // Specular BRDF
    let D = D_GGX(angular.n_dot_h, surface.roughness);
    let V = V_SmithGGX(angular.n_dot_v, angular.n_dot_l, surface.roughness);
    let F = F_Schlick(surface.f0, angular.v_dot_h);
    var Fr = D * V * F;

    // Diffuse BRDF (Energy Conserving)
    let kS = F;
    let kD = (vec3<f32>(1.0) - kS) * (1.0 - surface.metallic);
    var Fd = Diffuse_Burley(surface.albedo, surface.roughness, angular.n_dot_v, angular.n_dot_l, angular.v_dot_h);

    // Phase TSUSHIMA-F, kept in step with `evaluate_brdf_area` above. The
    // moonlight path is this function's only caller and it would otherwise be
    // the one surface response in the renderer still losing energy.
    if brdf_multiscatter {
        Fr = Fr * energy_compensation(
            surface.f0, env_brdf_ab(surface.roughness, angular.n_dot_v));
    }
    if brdf_rough_diffuse {
        Fd = diffuse_hammon(
            surface.albedo, surface.roughness,
            angular.n_dot_l, angular.n_dot_v, angular.n_dot_h, angular.l_dot_v,
        );
    }

    return (kD * Fd + Fr) * angular.n_dot_l;
}
