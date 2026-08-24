// MORROWIND-C: composition is declared here rather than assembled by a
// `format!` of `include_str!` calls at this pass's construction site. The
// resolver (`somnium_shader`) emits each module once, in this order, and
// hoists every `enable` above everything.
//!include "atmosphere.wgsl"

// Somnium Engine — Froxel volumetrics: aerial perspective + fog (24U, 25I).
//
// Concatenated after atmosphere.wgsl, whose density, extinction, phase and LUT
// helpers this reuses — the point being that the sky, the environment cubemap
// and this all integrate the *same* atmosphere, so they cannot disagree about
// what the air is made of.
//
// ## What this is
//
// A 3-D table indexed by (screen x, screen y, distance from camera). Each texel
// holds the light scattered *into* the view ray between the camera and that
// distance, plus how much of the original surface radiance survives the trip.
// Shading then needs one texture fetch per pixel to apply both:
//
//     colour = colour * transmittance + inscattering
//
// ## Why one volume for two features
//
// **25I aerial perspective** and **24U volumetric fog** are the same integral.
// They differ only in what is scattering — the atmosphere's Rayleigh and Mie
// terms, or a fog medium the artist places — and in whether the sun's
// contribution at each step is shadow-tested. Building them as one march means
// distant hills desaturate and a light shaft crosses the valley by the same
// code, and there is no second place for the medium to be defined.
//
// ## References
//
// - `bevy_pbr/src/atmosphere/aerial_view_lut.wgsl` — the 3-D LUT layout, the
//   log-space storage, and the analytic per-segment integration.
// - `bevy_pbr/src/volumetric_fog/volumetric_fog.wgsl` — shadow-sampled
//   in-scattering for shafts, and the Henyey-Greenstein asymmetry parameter.

// Mirror of `shadow/mod.rs::GpuDirectionalLight` (320 bytes), which
// `shading.wgsl` also declares. This is a separate pipeline with its own
// module, so it needs its own declaration — the layout is what must agree, and
// the Rust struct is the single source both mirror. Only the cascade matrices,
// splits and atlas size are read here.
struct DirectionalLight {
    direction: vec3<f32>,
    _pad0: f32,
    color: vec3<f32>,
    _pad1: f32,
    view_proj: array<mat4x4<f32>, 4>,
    cascade_splits: vec4<f32>,
    shadow_map_size: f32,
    ibl_intensity: f32,
    sun_angular_radius: f32,
    _pad2_z: f32,
    moon_direction: vec3<f32>,
    moon_intensity: f32,
}

struct VolumetricParams {
    inv_view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    camera_pos: vec3<f32>,
    /// Distance the volume spans. Beyond it, the last slice is held.
    max_distance: f32,
    sun_direction: vec3<f32>,
    /// Fog extinction per metre. 0 disables the fog medium entirely, leaving
    /// pure atmospheric aerial perspective.
    fog_density: f32,
    sun_illuminance: vec3<f32>,
    /// Henyey-Greenstein asymmetry. 0 is isotropic; positive scatters forward,
    /// which is what makes a shaft brighten as you look toward the sun.
    fog_asymmetry: f32,
    /// Height in metres over which fog density falls to 1/e. 0 = uniform.
    fog_height_falloff: f32,
    /// World height the fog density is measured at.
    fog_base_height: f32,
    /// Non-zero shadow-tests each step, which is what draws light shafts.
    shafts_enabled: u32,
    /// Visibility contrast of shadowed in-scatter. 0 disables the shadow
    /// modulation and 1 applies the full shadow-map visibility.
    shaft_intensity: f32,
    /// Phase 24U temporal reprojection.
    prev_view_proj: mat4x4<f32>,
    /// Zero on the first frame or after a resize, where the history describes
    /// a different volume.
    history_valid: f32,
    /// Per-frame offset of the sample position within each step.
    jitter: f32,
    _pad2: f32,
    _pad3: f32,
}

@group(0) @binding(0) var<uniform> vol: VolumetricParams;
@group(0) @binding(1) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(2) var multiscatter_lut: texture_2d<f32>;
@group(0) @binding(3) var lut_sampler: sampler;
@group(0) @binding(4) var volume_out: texture_storage_3d<rgba16float, write>;
@group(0) @binding(5) var<storage, read> vol_light: DirectionalLight;
@group(0) @binding(6) var vol_shadow_atlas: texture_depth_2d;
/// Phase 24U: the previous frame's volume, for temporal reprojection.
@group(0) @binding(7) var vol_history: texture_3d<f32>;
@group(0) @binding(8) var vol_history_sampler: sampler;

/// Steps taken per slice. Each slice integrates its own segment, so total step
/// count is this times the slice count.
const VOL_STEPS_PER_SLICE: u32 = 2u;
/// Guards the analytic integration's division when a medium barely absorbs.
const VOL_MIN_EXTINCTION: f32 = 1e-7;

/// Henyey-Greenstein phase function — how much light scatters toward `cos_theta`.
///
/// The atmosphere's Mie term already uses this shape; a fog medium wants it too,
/// with an artist-facing asymmetry so a shaft can be tuned to bloom toward the
/// sun without changing the density.
fn hg_phase(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    let denom = 1.0 + g2 - 2.0 * g * cos_theta;
    return (1.0 - g2) / (4.0 * 3.14159265 * max(denom * sqrt(max(denom, 1e-6)), 1e-6));
}

/// Hard, single-tap sun visibility for one point in the volume.
///
/// Deliberately **not** the PCSS path `shading.wgsl` uses. That one exists to
/// make a surface's shadow edge look right — blocker search, Vogel filtering,
/// cascade blending — and costs 40 taps. A froxel needs a yes/no answer at a
/// tenth of the screen's resolution, and the volume's own filtering smooths the
/// result. Copying the surface path here would be both slower and wrong.
fn volume_sun_visibility(world_pos: vec3<f32>, view_depth: f32) -> f32 {
    var cascade = 3u;
    if view_depth < vol_light.cascade_splits.x { cascade = 0u; }
    else if view_depth < vol_light.cascade_splits.y { cascade = 1u; }
    else if view_depth < vol_light.cascade_splits.z { cascade = 2u; }

    let clip = vol_light.view_proj[cascade] * vec4<f32>(world_pos, 1.0);
    if clip.w <= 0.0 {
        return 1.0;
    }
    let ndc = clip.xyz / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 1.0 - (ndc.y * 0.5 + 0.5));
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))
        || ndc.z < 0.0 || ndc.z > 1.0
    {
        return 1.0;
    }

    let offsets = array<vec2<f32>, 4>(
        vec2(0.0, 0.0), vec2(0.5, 0.0), vec2(0.0, 0.5), vec2(0.5, 0.5),
    );
    let atlas = uv * 0.5 + offsets[cascade];
    let texel = vec2<i32>(atlas * vol_light.shadow_map_size);
    let stored = textureLoad(vol_shadow_atlas, texel, 0);
    // Air has no surface to bias against, so a plain epsilon is enough.
    return select(1.0, 0.0, stored < ndc.z - 0.0015);
}

/// Fog density at a world height, with an exponential falloff so it pools in
/// valleys rather than filling the sky.
fn fog_density_at(world_y: f32) -> f32 {
    if vol.fog_density <= 0.0 {
        return 0.0;
    }
    if vol.fog_height_falloff <= 0.0 {
        return vol.fog_density;
    }
    let h = max(world_y - vol.fog_base_height, 0.0);
    return vol.fog_density * exp(-h / vol.fog_height_falloff);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(volume_out);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }

    // Ray for this froxel column, from the camera through the pixel centre.
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(dims.xy);
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let near = vol.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    let far = vol.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let ray_dir = normalize(far.xyz / far.w - near.xyz / near.w);

    let sun_dir = normalize(vol.sun_direction);
    let cos_theta = dot(ray_dir, sun_dir);
    let rayleigh = rayleigh_phase(cos_theta);
    let mie = mie_phase(cos_theta);
    let fog_phase = hg_phase(cos_theta, vol.fog_asymmetry);

    // The atmosphere functions work in planet-centred coordinates; the scene is
    // metres about the origin, so the camera sits at the planet's surface.
    let world_origin = vec3<f32>(0.0, GROUND_RADIUS + vol.camera_pos.y * 0.001, 0.0);

    let slices = dims.z;
    var prev_t = 0.0;
    var inscatter = vec3<f32>(0.0);
    var throughput = vec3<f32>(1.0);

    for (var slice = 0u; slice < slices; slice = slice + 1u) {
        for (var step = 0u; step < VOL_STEPS_PER_SLICE; step = step + 1u) {
            // Position within the slice, offset by a per-frame jitter (24U).
            //
            // A fixed midpoint samples the same points every frame, so a thin
            // medium is either always hit or always missed and the error is a
            // *stationary* pattern — banding that sits still while the camera
            // moves, which is the most visible kind. Moving the sample each
            // frame turns that bias into noise, and the temporal blend below
            // averages it away. This is the half of the technique that lets the
            // step count come down; reprojection alone would only smear.
            // Jitter the two ordered strata, never wrap them with fract(). The
            // wrapped form can put step 1 before step 0, making dt negative;
            // negative extinction amplifies light and erases shafts.
            let frac = (f32(step) + clamp(vol.jitter, 0.001, 0.999))
                / f32(VOL_STEPS_PER_SLICE);
            let t = vol.max_distance * (f32(slice) + frac) / f32(slices);
            let dt = t - prev_t;
            prev_t = t;

            let world_pos = vol.camera_pos + ray_dir * t;

            // ── Atmosphere (25I) ────────────────────────────────────────────
            // Altitude in the atmosphere model's units (km above the ground).
            let sample_r = GROUND_RADIUS + max(world_pos.y, 0.0) * 0.001;
            let altitude = sample_r - GROUND_RADIUS;
            let density = atmosphere_density(altitude);
            let extinction_air = atmosphere_extinction(altitude) * 0.001;
            // **Units.** The atmosphere model is in kilometres — its
            // extinction is km⁻¹ and its scale heights are 8 km and 1.2 km —
            // while the scene marches in metres. Converting the air terms to
            // per-metre here keeps one unit for the whole integral; leaving it
            // out makes the air a thousand times denser, which reads as fog
            // going opaque within a metre of the camera.
            const KM_PER_M: f32 = 0.001;
            let scatter_rayleigh = RAYLEIGH_SCATTERING * density.x * KM_PER_M;
            let scatter_mie = vec3<f32>(MIE_SCATTERING * density.y * KM_PER_M);

            let mu_sun = sun_dir.y;
            // Phase 25M. `atmosphere.wgsl` and `atmosphere_lut.wgsl` both guard
            // their sun transmittance with this; the froxel volume did not, and
            // it is the one place the omission was visible. Below the horizon
            // the LUT lookup clamps to its last valid row — the reddest one,
            // because at grazing angles Rayleigh has taken everything but red
            // out — so every froxel went on being lit by the reddest possible
            // sunlight instead of by none. That is the red frame.
            var sun_transmittance = vec3<f32>(0.0);
            var multiscatter = vec3<f32>(0.0);
            if !ray_hits_ground(sample_r, mu_sun) {
                sun_transmittance = sample_transmittance(
                    transmittance_lut, lut_sampler, sample_r, mu_sun);
                multiscatter = sample_multiscatter(
                    multiscatter_lut, lut_sampler, sample_r, mu_sun);
            }

            var sun_vis = 1.0;
            if vol.shafts_enabled != 0u {
                // Cascade splits are camera/view depth, not radial ray length.
                // They only match at screen centre; using t selected a wrong
                // cascade toward the edges and made shafts appear absent.
                let view_depth = max(-(vol.view * vec4<f32>(world_pos, 1.0)).z, 0.0);
                let shadow_vis = volume_sun_visibility(world_pos, view_depth);
                // "Shaft amount" is contrast, not a global light multiplier.
                // Multiplying all lit froxels made the enabled image globally
                // brighter; auto exposure then normalized that shift and made
                // the actual occlusion pattern appear inert. Keep unoccluded
                // single scattering unchanged and only remove direct light in
                // shadowed fog. Values above one mean full contrast so old
                // scenes using the former 1.5 default remain sensible.
                sun_vis = mix(1.0, shadow_vis, saturate(vol.shaft_intensity));
            }

            var step_scatter = (scatter_rayleigh * rayleigh + scatter_mie * mie)
                * sun_transmittance * sun_vis
                + (scatter_rayleigh + scatter_mie) * multiscatter;

            // ── Fog medium and shafts (24U) ─────────────────────────────────
            let fog = fog_density_at(world_pos.y);
            var extinction = extinction_air;
            if fog > 0.0 {
                // Fog scatters greyly: a water-droplet medium is not
                // wavelength-selective the way Rayleigh is.
                step_scatter += vec3<f32>(fog * fog_phase) * sun_vis * sun_transmittance;
                extinction += vec3<f32>(fog);
            }

            // Analytic integration of the segment rather than a Riemann sum:
            // the closed form is exact for a constant medium over the step and
            // stops thin media from being under-counted at low step counts.
            let step_transmittance = exp(-extinction * dt);
            let integrated = (step_scatter - step_scatter * step_transmittance)
                / max(extinction, vec3<f32>(VOL_MIN_EXTINCTION));
            inscatter += throughput * integrated * vol.sun_illuminance;
            throughput *= step_transmittance;
        }

        // Log space, so the hardware's linear filtering between slices
        // interpolates an exponential quantity correctly rather than cutting
        // the corner off every curve (bevy_solari's aerial LUT does the same).
        let mean_transmittance = dot(throughput, vec3<f32>(1.0 / 3.0));
        var value = vec4<f32>(log(max(inscatter, vec3<f32>(1e-6))), mean_transmittance);

        // ── Temporal reprojection (Phase 24U) ────────────────────────────────
        // Where this froxel's centre was in the previous frame's volume. The
        // froxel grid is attached to the camera, so a froxel does not keep its
        // identity across a move — reprojecting through world space is what
        // makes the history mean the same piece of air.
        if vol.history_valid > 0.5 {
            let slice_t = vol.max_distance * (f32(slice) + 0.5) / f32(slices);
            let centre_ws = vol.camera_pos + ray_dir * slice_t;
            let prev_clip = vol.prev_view_proj * vec4<f32>(centre_ws, 1.0);
            if prev_clip.w > 0.0 {
                let prev_ndc = prev_clip.xy / prev_clip.w;
                let prev_uv = vec2<f32>(prev_ndc.x * 0.5 + 0.5, 0.5 - prev_ndc.y * 0.5);
                let prev_w = slice_t / vol.max_distance;
                if all(prev_uv >= vec2<f32>(0.0)) && all(prev_uv <= vec2<f32>(1.0))
                    && prev_w <= 1.0 {
                    let history = textureSampleLevel(
                        vol_history, vol_history_sampler,
                        vec3<f32>(prev_uv, prev_w), 0.0);
                    // A small new-frame weight: the volume is cheap to be wrong
                    // about for a frame and expensive to have crawl. 0.05 is
                    // ~20 frames of accumulation, which at 60 Hz is a third of
                    // a second — below the threshold where a fog change reads
                    // as lag.
                    value = mix(history, value, 0.05);
                }
            }
        }

        textureStore(
            volume_out,
            vec3<i32>(i32(gid.x), i32(gid.y), i32(slice)),
            value,
        );
    }
}
