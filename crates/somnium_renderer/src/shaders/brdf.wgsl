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
};

fn get_angular_info(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>) -> AngularInfo {
    let h = normalize(v + l);
    var info: AngularInfo;
    info.n_dot_l = saturate(dot(n, l));
    info.n_dot_v = saturate(dot(n, v));
    info.n_dot_h = saturate(dot(n, h));
    info.v_dot_h = saturate(dot(v, h));
    info.l_dot_h = saturate(dot(l, h));
    return info;
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
    let Fr = D * V * F * energy;

    let kS = F;
    let kD = (vec3<f32>(1.0) - kS) * (1.0 - surface.metallic);
    let Fd = Diffuse_Burley(
        surface.albedo, surface.roughness,
        angular.n_dot_v, angular.n_dot_l, angular.v_dot_h,
    );

    return (kD * Fd + Fr) * angular.n_dot_l;
}

fn evaluate_brdf(surface: Surface, l: vec3<f32>) -> vec3<f32> {
    let angular = get_angular_info(surface.normal, surface.view_dir, l);
    
    // Specular BRDF
    let D = D_GGX(angular.n_dot_h, surface.roughness);
    let V = V_SmithGGX(angular.n_dot_v, angular.n_dot_l, surface.roughness);
    let F = F_Schlick(surface.f0, angular.v_dot_h);
    let Fr = D * V * F;
    
    // Diffuse BRDF (Energy Conserving)
    let kS = F;
    let kD = (vec3<f32>(1.0) - kS) * (1.0 - surface.metallic);
    let Fd = Diffuse_Burley(surface.albedo, surface.roughness, angular.n_dot_v, angular.n_dot_l, angular.v_dot_h);
    
    return (kD * Fd + Fr) * angular.n_dot_l;
}
