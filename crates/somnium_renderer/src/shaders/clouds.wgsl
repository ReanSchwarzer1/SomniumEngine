// Somnium Engine — Phase CONTROL-M: the volumetric cloud march.
//
// Concatenated after `atmosphere.wgsl`, whose LUT helpers and constants this
// reuses. That reuse is the point rather than a convenience: the clouds, the
// sky and the froxel volume must agree about the colour of the sun, and the
// only way to guarantee that is for all three to read the same transmittance
// LUT rather than each carrying its own sun tint.
//
// ## References
//
// - Schneider & Vos, SIGGRAPH 2015 — the shape model, the height gradients per
//   cloud type, Beer's law with the powder term, and the cone-sampled shadow
//   taps toward the sun.
// - Toft & Bowles, arXiv:1609.05344 — the adaptive step (large until density,
//   small inside), the early-out on transmittance, and the measured warning
//   that a per-pixel jittered ray start can cost more than the steps it saves
//   through texture-cache incoherence. `jitter_enabled` exists so that is a
//   measurement in this engine and not a belief.
// - Epic's Volumetric Cloud docs — Beer Shadow Maps as the ground-level cloud
//   shadow model, which is what `cloud_shadow` below writes.

struct CloudParams {
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    /// Metres from the ground to the bottom of the cloud layer.
    layer_bottom: f32,
    sun_direction: vec3<f32>,
    /// Layer thickness in metres.
    layer_thickness: f32,
    sun_illuminance: vec3<f32>,
    /// Density multiplier. Zero is a clear sky and skips the march entirely.
    density: f32,
    /// Wind offset in metres, accumulated on the CPU so the clouds keep moving
    /// across a pause in the same direction they were going.
    wind_offset: vec2<f32>,
    /// Metres of world per repeat of the weather map.
    weather_scale: f32,
    /// Metres of world per repeat of the base shape volume.
    shape_scale: f32,
    /// Strength of the high-frequency erosion, `0..1`.
    detail_strength: f32,
    /// Henyey–Greenstein forward lobe.
    phase_forward: f32,
    /// Henyey–Greenstein backward lobe. Negative.
    phase_backward: f32,
    /// Blend between the two lobes.
    phase_blend: f32,
    /// Ambient contribution from the sky, scaling the multiscatter LUT.
    ambient: f32,
    /// Extra absorption applied to precipitating columns.
    precipitation: f32,
    /// Non-zero applies the blue-noise ray-start offset.
    jitter_enabled: f32,
    /// Frame counter, for the temporal component of the jitter.
    frame: f32,
    /// Primary march steps at full quality.
    max_steps: f32,
    /// Light-march steps toward the sun.
    light_steps: f32,
    /// Half-extent, in metres, of the world-XZ cloud shadow map.
    shadow_extent: f32,
    /// `0..1`. Zero leaves the ground unshadowed by cloud.
    shadow_strength: f32,
    /// Distance in metres at which the march gives up. Clouds beyond the
    /// horizon are not worth the steps.
    max_distance: f32,
    /// Range the froxel volume spans, so aerial perspective can be applied to
    /// the cloud layer. Zero disables the lookup.
    volumetric_range: f32,
    _pad0: f32,
    _pad1: f32,
}

@group(0) @binding(0) var<uniform> cp: CloudParams;
@group(0) @binding(1) var base_noise_tex:   texture_3d<f32>;
@group(0) @binding(2) var detail_noise_tex: texture_3d<f32>;
@group(0) @binding(3) var weather_tex:      texture_2d<f32>;
@group(0) @binding(4) var cloud_sampler:    sampler;
@group(0) @binding(5) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(6) var multiscatter_lut:  texture_2d<f32>;
@group(0) @binding(7) var lut_sampler:       sampler;
@group(0) @binding(8) var scatter_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(9) var shadow_out:  texture_storage_2d<r16float, write>;
@group(0) @binding(10) var scene_depth: texture_depth_2d;
@group(0) @binding(11) var cloud_volumetrics: texture_3d<f32>;

const CLOUD_PI: f32 = 3.14159265;
/// Transmittance below which the march stops. Anything darker is opaque.
const CLOUD_MIN_TRANSMITTANCE: f32 = 0.01;
/// Consecutive empty samples before the march goes back to long steps.
const CLOUD_EMPTY_BEFORE_ESCAPE: i32 = 8;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Shape
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn cloud_remap(v: f32, old_lo: f32, old_hi: f32, new_lo: f32, new_hi: f32) -> f32 {
    return new_lo + (v - old_lo) / max(old_hi - old_lo, 1e-6) * (new_hi - new_lo);
}

/// The vertical density profile for a cloud type in `0..1`.
///
/// Three named shapes blended by the weather map's type channel: stratus is a
/// flat sheet low in the layer, cumulus is a rounded mass in the middle, and
/// cumulonimbus fills the whole layer. Returned as a multiplier on the shape
/// noise, which is what stops a cumulus from having a flat top.
fn height_gradient(height: f32, cloud_type: f32) -> f32 {
    let stratus = smoothstep(0.0, 0.08, height) * (1.0 - smoothstep(0.18, 0.32, height));
    let cumulus = smoothstep(0.02, 0.22, height) * (1.0 - smoothstep(0.60, 0.95, height));
    let cumulonimbus = smoothstep(0.0, 0.10, height) * (1.0 - smoothstep(0.85, 1.0, height));

    let low = mix(stratus, cumulus, clamp(cloud_type * 2.0, 0.0, 1.0));
    return mix(low, cumulonimbus, clamp((cloud_type - 0.5) * 2.0, 0.0, 1.0));
}

fn sample_weather(world_xz: vec2<f32>) -> vec4<f32> {
    let uv = (world_xz + cp.wind_offset) / max(cp.weather_scale, 1.0);
    return textureSampleLevel(weather_tex, cloud_sampler, uv, 0.0);
}

/// Density at a world point. `cheap` skips the detail erosion, which is what
/// makes the long-step search affordable.
fn cloud_density(world_pos: vec3<f32>, cheap: bool) -> f32 {
    let height = (world_pos.y - cp.layer_bottom) / max(cp.layer_thickness, 1.0);
    if height < 0.0 || height > 1.0 {
        return 0.0;
    }

    let weather = sample_weather(world_pos.xz);
    let coverage = weather.r;
    if coverage <= 0.001 {
        return 0.0;
    }

    // The wind carries the *shape* as well as the weather field, and the two
    // move at slightly different rates so the sky does not read as one rigid
    // sheet sliding past.
    let shape_uvw = (world_pos + vec3<f32>(cp.wind_offset.x, 0.0, cp.wind_offset.y) * 1.35)
        / max(cp.shape_scale, 1.0);
    let base = textureSampleLevel(base_noise_tex, cloud_sampler, shape_uvw, 0.0);

    // Erode the Perlin–Worley base by its own higher-frequency Worley
    // channels, then by the height gradient, then by coverage. Order matters:
    // coverage last is what makes the coverage slider behave like a dissolve
    // rather than like a density multiply.
    let worley_fbm = base.g * 0.625 + base.b * 0.25 + base.a * 0.125;
    var shape = clamp(cloud_remap(base.r, worley_fbm - 1.0, 1.0, 0.0, 1.0), 0.0, 1.0);
    shape *= height_gradient(height, weather.g);
    var density = clamp(cloud_remap(shape, 1.0 - coverage, 1.0, 0.0, 1.0), 0.0, 1.0) * coverage;

    if density <= 0.0 {
        return 0.0;
    }
    if cheap || cp.detail_strength <= 0.0 {
        return density * cp.density;
    }

    // Detail erosion, applied only near the edges. Schneider's trick: at the
    // core the high-frequency term is inverted so the interior gains billows
    // instead of being eaten away.
    let detail_uvw = (world_pos + vec3<f32>(cp.wind_offset.x, 0.0, cp.wind_offset.y) * 2.1)
        / max(cp.shape_scale * 0.11, 1.0);
    let detail = textureSampleLevel(detail_noise_tex, cloud_sampler, detail_uvw, 0.0);
    let detail_fbm = detail.r * 0.625 + detail.g * 0.25 + detail.b * 0.125;
    let modifier = mix(detail_fbm, 1.0 - detail_fbm, clamp(height * 5.0, 0.0, 1.0));
    density = cloud_remap(
        density,
        modifier * cp.detail_strength * 0.35,
        1.0,
        0.0,
        1.0,
    );

    return clamp(density, 0.0, 1.0) * cp.density;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Lighting
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn hg(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    let denom = 1.0 + g2 - 2.0 * g * cos_theta;
    return (1.0 - g2) / (4.0 * CLOUD_PI * max(denom * sqrt(max(denom, 1e-6)), 1e-6));
}

/// Dual-lobe phase: a strong forward lobe for the silver lining and a weak
/// backward one so a cloud away from the sun is not flat black.
fn dual_lobe(cos_theta: f32) -> f32 {
    return mix(
        hg(cos_theta, cp.phase_forward),
        hg(cos_theta, cp.phase_backward),
        clamp(cp.phase_blend, 0.0, 1.0),
    );
}

/// Beer's law with Schneider's powder term.
///
/// Beer alone makes the sun-facing side of a cloud *darker* the denser it is,
/// which is backwards for what the eye expects at the edge of a cumulus. The
/// powder term restores the dark-edge/bright-core reading.
fn beer_powder(optical_depth: f32) -> f32 {
    let beer = exp(-optical_depth);
    let powder = 1.0 - exp(-optical_depth * 2.0);
    return beer * mix(1.0, powder * 2.0, 0.5);
}

/// Six cone-spread taps toward the sun, accumulating optical depth.
///
/// Cone-spread rather than a straight line because a straight march
/// under-samples the neighbouring cloud that is actually casting the shadow,
/// and the error shows up as a cloud lit as though it were alone in the sky.
fn light_march(origin: vec3<f32>, sun: vec3<f32>) -> f32 {
    let steps = max(i32(cp.light_steps), 1);
    // Spread across the layer rather than across the ray, for the same reason
    // the primary march does: a light ray at a shallow sun angle must not
    // sample the cloud more coarsely than one at noon.
    let step_len = max(cp.layer_thickness / f32(steps) * 1.5, 1.0);
    var optical_depth = 0.0;
    var t = 0.0;
    for (var i = 0; i < steps; i = i + 1) {
        // The cone widens with distance; the offsets are a fixed low-discrepancy
        // set rather than random so the result does not shimmer.
        let spread = f32(i) / f32(steps) * step_len * 0.6;
        let jitter = vec3<f32>(
            sin(f32(i) * 2.399) * spread,
            cos(f32(i) * 1.117) * spread * 0.3,
            cos(f32(i) * 2.399) * spread,
        );
        t += step_len;
        let p = origin + sun * t + jitter;
        optical_depth += cloud_density(p, true) * step_len * 0.01;
    }
    // One long tap far along the ray catches a distant bank that the short
    // cone misses entirely.
    let far = origin + sun * step_len * f32(steps) * 4.0;
    optical_depth += cloud_density(far, true) * step_len * 0.04;
    return optical_depth;
}

/// Where the ray enters and leaves the cloud slab, in metres along the ray.
///
/// A flat slab rather than two concentric spheres. The curvature of a 1 km
/// layer over a scene a kilometre across is well under a pixel, and the slab
/// form has no case where a grazing ray produces an enormous interval.
fn slab_interval(origin: vec3<f32>, dir: vec3<f32>) -> vec2<f32> {
    let bottom = cp.layer_bottom;
    let top = cp.layer_bottom + cp.layer_thickness;
    if abs(dir.y) < 1e-4 {
        // Parallel to the slab: either inside it for ever or never in it.
        if origin.y >= bottom && origin.y <= top {
            return vec2<f32>(0.0, cp.max_distance);
        }
        return vec2<f32>(-1.0, -1.0);
    }
    let t0 = (bottom - origin.y) / dir.y;
    let t1 = (top - origin.y) / dir.y;
    let enter = max(min(t0, t1), 0.0);
    let exit = max(t0, t1);
    if exit <= 0.0 {
        return vec2<f32>(-1.0, -1.0);
    }
    return vec2<f32>(enter, min(exit, cp.max_distance));
}

/// Interleaved-gradient noise — the cheapest blue-noise-like offset there is,
/// and the one whose cost Toft & Bowles measured.
fn ign(pixel: vec2<f32>, frame: f32) -> f32 {
    let p = pixel + 5.588238 * (frame % 64.0);
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

/// Aerial perspective for the cloud layer, sampled at its own depth.
///
/// Applied to the cloud layer **separately** from the scene layer and only
/// then composited, because applying it once after compositing is wrong: the
/// scene behind a cloud is further away than the cloud is, and one fetch
/// cannot be right for both.
fn cloud_aerial(view_uv: vec2<f32>, distance: f32) -> vec4<f32> {
    if cp.volumetric_range <= 0.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let w = clamp(distance / cp.volumetric_range, 0.0, 1.0);
    let sample = textureSampleLevel(
        cloud_volumetrics, lut_sampler, vec3<f32>(view_uv, w), 0.0);
    // Log-space inscatter, matching `volumetric.wgsl`'s storage.
    return vec4<f32>(exp(sample.rgb), clamp(sample.a, 0.0, 1.0));
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// The march
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

@compute @workgroup_size(8, 8, 1)
fn march(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(scatter_out);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(dims);

    // A clear sky costs one store per quarter-res pixel and nothing else.
    if cp.density <= 0.0 {
        textureStore(scatter_out, vec2<i32>(gid.xy), vec4<f32>(0.0, 0.0, 0.0, 1.0));
        return;
    }

    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let near = cp.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    let far = cp.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let near_ws = near.xyz / near.w;
    let far_ws = far.xyz / far.w;
    let ray_dir = normalize(far_ws - near_ws);

    var interval = slab_interval(cp.camera_pos, ray_dir);
    if interval.y <= interval.x {
        textureStore(scatter_out, vec2<i32>(gid.xy), vec4<f32>(0.0, 0.0, 0.0, 1.0));
        return;
    }

    // Occlusion by geometry. The depth buffer is full resolution and this pass
    // is quarter; a point sample of the matching texel is the conservative
    // choice, and the alternative — a min over the 2×2 — makes clouds vanish
    // behind a single foreground pixel.
    let depth_dims = textureDimensions(scene_depth);
    let depth_texel = vec2<i32>(uv * vec2<f32>(depth_dims));
    let depth = textureLoad(scene_depth, depth_texel, 0);
    if depth > 0.0 && depth < 1.0 {
        // Reconstruct the world position of the occluder and clamp the march.
        let occ_clip = cp.inv_view_proj * vec4<f32>(ndc, depth, 1.0);
        let occ_ws = occ_clip.xyz / occ_clip.w;
        let occ_distance = length(occ_ws - cp.camera_pos);
        interval.y = min(interval.y, occ_distance);
        if interval.y <= interval.x {
            textureStore(scatter_out, vec2<i32>(gid.xy), vec4<f32>(0.0, 0.0, 0.0, 1.0));
            return;
        }
    }

    let span = interval.y - interval.x;
    let steps = max(i32(cp.max_steps), 8);

    // **The step is a distance in metres, derived from the layer's own
    // thickness — not `span / steps`.**
    //
    // The first cut divided the whole ray, and the whole ray is enormous: a
    // shallow ray through a slab 2 km thick can span 60 km, so `span / 48` was
    // a coarse step over a kilometre long and a cloud got sampled three or
    // four times through its entire depth. That is what the blocky,
    // stair-stepped cloud edges were — not the quarter-resolution buffer, which
    // was the obvious suspect and the wrong one.
    //
    // Sampling density is now constant in metres however the ray is angled, so
    // a cloud on the horizon is shaded like a cloud overhead. The cost is
    // bounded by the iteration cap and the transmittance early-out rather than
    // by the step count, which is the right way round: a ray that leaves the
    // slab immediately does almost no work whatever `max_steps` says.
    let long_step = max(cp.layer_thickness / f32(steps) * 3.0, 1.0);
    let short_step = long_step * 0.35;
    // Enough iterations to cross the layer several times at the fine step,
    // capped so a grazing ray cannot run away.
    let iteration_cap = steps * 6;

    // Toft & Bowles' jitter. Their measurement is that this can cost
    // 2.3 → 7.5 ms through texture-cache incoherence, so whether it pays is a
    // number this engine takes rather than a belief it inherits.
    //
    // `cp.frame` is zero unless *temporal* jitter is on, and it defaults off.
    // With no cloud-history filter — the clouds ride in the TAA buffer, and TAA
    // has no motion vector for a sky pixel to reproject — a pattern that moved
    // every frame was not dithering, it was visible shimmer. Frame-stable
    // spatial jitter still breaks up the banding it was added for.
    var t = interval.x;
    if cp.jitter_enabled > 0.5 {
        t += ign(vec2<f32>(gid.xy), cp.frame) * long_step;
    }

    let sun = normalize(cp.sun_direction);
    let cos_theta = dot(ray_dir, sun);
    let phase = dual_lobe(cos_theta);

    // Ambient comes from the same multiscatter LUT the sky uses, so a cloud at
    // dusk is lit by the dusk sky rather than by a constant.
    let r = GROUND_RADIUS + max(cp.camera_pos.y, 0.0) * 0.001;
    var sun_transmittance = vec3<f32>(0.0);
    var ambient_sky = vec3<f32>(0.0);
    if !ray_hits_ground(r, sun.y) {
        sun_transmittance = sample_transmittance(transmittance_lut, lut_sampler, r, sun.y);
        ambient_sky = sample_multiscatter(multiscatter_lut, lut_sampler, r, sun.y);
    }

    var scatter = vec3<f32>(0.0);
    var transmittance = 1.0;
    var empty_runs = 0;
    var inside = false;
    // Transmittance-weighted mean depth: a cloud has no single depth, and this
    // is what the aerial-perspective fetch and CONTROL-N's motion vectors both
    // need in place of one.
    var depth_accum = 0.0;
    var depth_weight = 0.0;

    for (var i = 0; i < iteration_cap; i = i + 1) {
        if t >= interval.y || transmittance < CLOUD_MIN_TRANSMITTANCE {
            break;
        }
        let step_len = select(long_step, short_step, inside);
        let p = cp.camera_pos + ray_dir * (t + step_len * 0.5);

        if !inside {
            // Searching: cheap density only, long steps.
            if cloud_density(p, true) > 0.0 {
                // Step back one long step and switch to fine sampling, or the
                // cloud's leading edge is cut off flat.
                inside = true;
                empty_runs = 0;
                t = max(t - long_step, interval.x);
                continue;
            }
            t += step_len;
            continue;
        }

        let density = cloud_density(p, false);
        if density <= 0.0 {
            empty_runs += 1;
            if empty_runs >= CLOUD_EMPTY_BEFORE_ESCAPE {
                inside = false;
            }
            t += step_len;
            continue;
        }
        empty_runs = 0;

        // Precipitating columns absorb more, which is what makes the base of a
        // rain cloud read as dark rather than merely thick.
        let weather = sample_weather(p.xz);
        let extinction = density * (1.0 + weather.b * cp.precipitation * 2.0) * 0.05;
        let optical_depth = light_march(p, sun) * 4.0;
        let sun_energy = beer_powder(optical_depth);

        let luminance = sun_transmittance * cp.sun_illuminance * sun_energy * phase
            + ambient_sky * cp.ambient * (0.35 + 0.65 * density);

        let step_transmittance = exp(-extinction * step_len);
        // Analytic segment integration, as the froxel volume does: the closed
        // form stops a thin medium being under-counted at low step counts.
        let integrated = (luminance - luminance * step_transmittance)
            / max(extinction, 1e-6);
        scatter += transmittance * integrated;

        let weight = transmittance * (1.0 - step_transmittance);
        depth_accum += t * weight;
        depth_weight += weight;

        transmittance *= step_transmittance;
        t += step_len;
    }

    // Aerial perspective, at the cloud's own transmittance-weighted depth.
    let cloud_distance = select(interval.x, depth_accum / max(depth_weight, 1e-6),
        depth_weight > 1e-6);
    let aerial = cloud_aerial(uv, cloud_distance);
    scatter = scatter * aerial.a + aerial.rgb * (1.0 - transmittance);

    textureStore(
        scatter_out,
        vec2<i32>(gid.xy),
        vec4<f32>(scatter, clamp(transmittance, 0.0, 1.0)),
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Cloud shadows
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A world-XZ transmittance field, centred on the camera.
///
/// The Beer Shadow Map shape Epic recommends for ground-level viewing, reduced
/// to its useful core: one scalar per column saying how much sun reaches the
/// ground there. Terrain and water read it in `shading.wgsl`, so a cloud's
/// shadow crosses a beach onto the sea without either surface knowing what a
/// cloud is.
@compute @workgroup_size(8, 8, 1)
fn cloud_shadow(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(shadow_out);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    if cp.density <= 0.0 || cp.shadow_strength <= 0.0 {
        textureStore(shadow_out, vec2<i32>(gid.xy), vec4<f32>(1.0));
        return;
    }

    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(dims);
    let ground = vec2<f32>(
        cp.camera_pos.x + (uv.x * 2.0 - 1.0) * cp.shadow_extent,
        cp.camera_pos.z + (uv.y * 2.0 - 1.0) * cp.shadow_extent,
    );

    let sun = normalize(cp.sun_direction);
    if sun.y <= 0.02 {
        // A sun on or below the horizon casts no cloud shadow worth the name,
        // and the slab intersection degenerates.
        textureStore(shadow_out, vec2<i32>(gid.xy), vec4<f32>(1.0));
        return;
    }

    let origin = vec3<f32>(ground.x, 0.0, ground.y);
    let enter = (cp.layer_bottom - origin.y) / sun.y;
    let exit = (cp.layer_bottom + cp.layer_thickness - origin.y) / sun.y;
    let steps = 8;
    let step_len = (exit - enter) / f32(steps);
    var optical_depth = 0.0;
    for (var i = 0; i < steps; i = i + 1) {
        let t = enter + (f32(i) + 0.5) * step_len;
        optical_depth += cloud_density(origin + sun * t, true) * step_len * 0.05;
    }
    let transmittance = mix(1.0, exp(-optical_depth), clamp(cp.shadow_strength, 0.0, 1.0));
    textureStore(shadow_out, vec2<i32>(gid.xy), vec4<f32>(transmittance));
}
