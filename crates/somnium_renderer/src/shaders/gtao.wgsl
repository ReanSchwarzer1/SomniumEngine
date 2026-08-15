// Phase 24I: ground-truth ambient occlusion with bent normals.
//
// Phase 17I wired *baked* occlusion, so anything without an AO map — all
// terrain, every procedural mesh, every foliage instance — still received sky
// light unattenuated. That is why creases and contact points stayed flat and
// why shaded bark reads as sky-blue rather than dark.
//
// GTAO (Jimenez et al. 2016) rather than classic SSAO: instead of counting
// occluded sample points, it searches each screen-space slice for its *horizon*
// angles and integrates the visible arc analytically. That produces a real
// visibility integral rather than a heuristic darkening, so it matches a
// ray-traced reference instead of merely resembling one — which matters because
// this term will later feed the GI gather rather than just tinting the image.
//
// Normals come from depth rather than a G-buffer. The visibility buffer stores
// only IDs, and adding a normal target for this alone would cost bandwidth on
// every frame; reconstructing from depth is accurate enough for a visibility
// search and keeps the pass self-contained.

struct GtaoParams {
    /// Camera projection, for view-space reconstruction.
    proj: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    /// Reciprocal render size.
    inv_resolution: vec2<f32>,
    /// World-space radius the search covers.
    radius: f32,
    /// Raises AO to this power. >1 deepens contact darkening.
    power: f32,
    /// How strongly AO is applied, 0..1.
    intensity: f32,
    /// Frame counter, for temporal rotation of the sample pattern.
    frame: u32,
    /// Camera near plane, for linearising depth.
    near: f32,
    _pad: f32,
}

// Two entry points with two pipeline layouts. `main` uses 0-2, `denoise` uses
// 0, 3 and 4; naga tracks resources per entry point, so each pipeline's layout
// only has to cover what its own entry point touches. Binding the raw target
// as both a storage-write and a sampled texture in one layout would be a usage
// conflict, which is what forces the split.
@group(0) @binding(0) var depth_tex: texture_depth_2d;
@group(0) @binding(1) var gtao_out:  texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: GtaoParams;
@group(0) @binding(3) var gtao_in:   texture_2d<f32>;
@group(0) @binding(4) var gtao_denoised: texture_storage_2d<rgba8unorm, write>;

/// Slices around the view vector, and steps taken along each.
///
/// Two slices with a per-pixel rotation is the usual real-time budget: the
/// rotation turns the missing directions into noise, and noise is what the
/// spatial filter and TAA are for. More slices would be cleaner per frame and
/// is not worth the cost when something downstream is already accumulating.
const GTAO_SLICES: i32 = 2;
const GTAO_STEPS:  i32 = 8;

/// How far above its own tangent plane a sample must sit to count as an
/// occluder, as a sine of the angle. Small enough to keep real contact
/// darkening, large enough that a surface seen edge-on does not shadow itself.
const GTAO_PLANE_BIAS: f32 = 0.1;

/// View-space position for a pixel.
fn view_position(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let view = params.inv_proj * ndc;
    return view.xyz / view.w;
}

fn load_view_position(coord: vec2<i32>) -> vec3<f32> {
    let depth = textureLoad(depth_tex, coord, 0);
    let uv = (vec2<f32>(coord) + 0.5) * params.inv_resolution;
    return view_position(uv, depth);
}

/// View-space normal from the depth buffer.
///
/// Takes the *closer* of the two neighbours on each axis before
/// differencing. A naive central difference straddles depth discontinuities
/// and produces normals that face nowhere real, which shows up as bright or
/// black rims along every silhouette — the same class of error that plagues
/// screen-space effects generally.
fn reconstruct_normal(coord: vec2<i32>, centre: vec3<f32>) -> vec3<f32> {
    let left  = load_view_position(coord + vec2<i32>(-1, 0));
    let right = load_view_position(coord + vec2<i32>(1, 0));
    let down  = load_view_position(coord + vec2<i32>(0, -1));
    let up    = load_view_position(coord + vec2<i32>(0, 1));

    let dx = select(right - centre, centre - left,
        abs(left.z - centre.z) < abs(right.z - centre.z));
    let dy = select(up - centre, centre - down,
        abs(down.z - centre.z) < abs(up.z - centre.z));

    // `cross(dy, dx)`, not `cross(dx, dy)`.
    //
    // The two screen axes do not agree about handedness once they reach view
    // space. `coord.x + 1` is `+NDC x` is `+view x`, so `dx` runs `+x`; but
    // `view_position` flips y (`1.0 - uv.y * 2.0`), so `coord.y + 1` runs
    // **−view y** and `dy` runs `−y`. `cross(dx, dy)` is therefore
    // `cross(+x, −y) = −z`: a normal pointing away from the camera on every
    // visible surface.
    //
    // That is not a subtle error in the output. A back-facing normal puts
    // `n_angle` near ±π, the horizon clamps hand `integrate_arc` an arc it was
    // never meant to see, and the closed form returns a *negative* visibility
    // (−0.5 for an unoccluded pixel). `saturate` then takes it to exactly zero,
    // so `gtao.a` was 0 for every pixel with geometry in it — which multiplies
    // both terms of `evaluate_ibl` and removed all indirect light from the
    // entire scene while leaving direct sun untouched. Dbg 8 rendered pure
    // black on terrain, meshes and foliage alike.
    //
    // It read as a terrain or shadow problem for a long time because direct
    // lighting was unaffected: surfaces were lit, just never *ambient* lit.
    return normalize(cross(dy, dx));
}

/// Visible arc for one slice, given its two horizon angles.
///
/// The closed form from the GTAO paper: the cosine-weighted integral over the
/// arc between the horizons, projected onto the surface normal. This is the
/// step that makes the result an actual visibility fraction rather than an
/// occlusion heuristic.
fn integrate_arc(h1: f32, h2: f32, n: f32) -> f32 {
    let a = -cos(2.0 * h1 - n) + cos(n) + 2.0 * h1 * sin(n);
    let b = -cos(2.0 * h2 - n) + cos(n) + 2.0 * h2 * sin(n);
    return 0.25 * (a + b);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(depth_tex);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let coord = vec2<i32>(gid.xy);
    let depth = textureLoad(depth_tex, coord, 0);

    // Nothing drawn here: sky is unoccluded by definition, and a
    // reconstructed position on the far plane is meaningless anyway.
    if depth >= 1.0 {
        textureStore(gtao_out, coord, vec4<f32>(0.5, 0.5, 1.0, 1.0));
        return;
    }

    let centre = load_view_position(coord);
    let normal = reconstruct_normal(coord, centre);
    let view_dir = normalize(-centre);

    // Screen-space radius for the world-space search radius at this depth, so
    // occlusion stays the same physical size rather than shrinking with
    // distance the way a fixed pixel radius would.
    let radius_px = params.radius * params.proj[0][0]
        / max(-centre.z, 1e-3) / params.inv_resolution.x * 0.5;
    let step_px = max(radius_px / f32(GTAO_STEPS), 1.0);

    let noise = interleaved_gradient_noise(vec2<f32>(coord), params.frame);
    var occlusion = 0.0;
    var bent = vec3<f32>(0.0);

    for (var s = 0; s < GTAO_SLICES; s = s + 1) {
        // Rotate the slice set per pixel and per frame. Without this the same
        // few directions are sampled everywhere and the result bands visibly.
        let angle = (f32(s) + noise) * 3.14159265 / f32(GTAO_SLICES);
        let dir = vec2<f32>(cos(angle), sin(angle));

        // Project the slice plane's normal out of the surface normal to get
        // the reference direction the horizons are measured against.
        let slice_dir = vec3<f32>(dir.x, dir.y, 0.0);
        let plane_normal = normalize(cross(slice_dir, view_dir));
        let projected = normal - plane_normal * dot(normal, plane_normal);
        let projected_len = length(projected);
        if projected_len < 1e-4 {
            continue;
        }
        let projected_n = projected / projected_len;

        // Signed angle of the projected normal within the slice.
        let sign_n = sign(dot(cross(projected_n, view_dir), plane_normal));
        let n_angle = sign_n * acos(clamp(dot(projected_n, view_dir), -1.0, 1.0));

        // Walk outward in both directions, tracking the highest horizon found.
        var cos_h1 = -1.0;
        var cos_h2 = -1.0;
        for (var step = 1; step <= GTAO_STEPS; step = step + 1) {
            let offset = dir * (f32(step) + noise) * step_px;

            let s1 = load_view_position(coord + vec2<i32>(offset));
            let d1 = s1 - centre;
            let len1 = length(d1);
            // A sample lying in the surface's own tangent plane *is* the
            // surface, not something in front of it. Seen at a grazing angle a
            // heightfield's neighbours run almost parallel to the view ray, so
            // without this test every one of them registers as a horizon and
            // the ground occludes itself to near-black — which is exactly what
            // terrain did the first time it consumed GTAO (0.029 visibility on
            // open ground). Requiring a sample to sit measurably *above* the
            // tangent plane is the standard remedy and costs one dot product.
            if len1 > 1e-4 && len1 < params.radius
                && dot(d1 / len1, normal) > GTAO_PLANE_BIAS {
                // Falloff keeps distant geometry from carving occlusion into
                // surfaces it is nowhere near.
                let falloff = saturate(1.0 - len1 / params.radius);
                cos_h1 = max(cos_h1, dot(d1 / len1, view_dir) * falloff);
            }

            let s2 = load_view_position(coord - vec2<i32>(offset));
            let d2 = s2 - centre;
            let len2 = length(d2);
            if len2 > 1e-4 && len2 < params.radius
                && dot(d2 / len2, normal) > GTAO_PLANE_BIAS {
                let falloff = saturate(1.0 - len2 / params.radius);
                cos_h2 = max(cos_h2, dot(d2 / len2, view_dir) * falloff);
            }
        }

        let h1 = n_angle + max(-acos(clamp(cos_h1, -1.0, 1.0)) - n_angle, -1.5707963);
        let h2 = n_angle + min(acos(clamp(cos_h2, -1.0, 1.0)) - n_angle, 1.5707963);

        // Floored at zero. The closed form can go negative when it is handed an
        // arc outside its domain — which is what a wrongly-signed normal does —
        // and a negative visibility is meaningless. Without the floor the error
        // propagates into `saturate(...)` as exactly 0, i.e. *fully occluded*,
        // which is the most destructive possible reading of a broken input and
        // silently removes all indirect light. Clamping makes the failure
        // direction "no occlusion" instead: the same image a disabled pass
        // gives, which is obvious to nobody but harmless to everybody.
        occlusion += projected_len * max(integrate_arc(h1, h2, n_angle), 0.0);

        // Bent normal: the average unoccluded direction. Cheaper to derive from
        // the horizon midpoint than to accumulate separately, and it is what
        // lets the indirect specular cone point away from the occluder instead
        // of straight out along the surface normal.
        let bent_angle = (h1 + h2) * 0.5;
        bent += view_dir * cos(bent_angle) + slice_dir * sin(bent_angle);
    }

    var ao = saturate(occlusion / f32(GTAO_SLICES));
    ao = pow(ao, params.power);
    ao = mix(1.0, ao, params.intensity);

    // Bent normal packed to [0,1]; falls back to the surface normal when the
    // slices disagree, which happens on flat, fully open surfaces where the
    // bent normal is the surface normal anyway.
    var bent_n = normal;
    if length(bent) > 1e-4 {
        bent_n = normalize(bent);
    }
    textureStore(gtao_out, coord, vec4<f32>(bent_n * 0.5 + 0.5, ao));
}

/// Separable-ish 4x4 box denoise over the AO channel.
///
/// Two slices per pixel is deliberately under-sampled, so the raw output is
/// noisy by construction. Depth-weighting the blur keeps it from bleeding
/// occlusion across silhouettes, which would put a dark halo around every
/// object — the exact artefact this kind of pass is notorious for.
@compute @workgroup_size(8, 8, 1)
fn denoise(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(depth_tex);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let coord = vec2<i32>(gid.xy);
    let centre_depth = textureLoad(depth_tex, coord, 0);

    var total = 0.0;
    var weight_sum = 0.0;
    var bent = vec3<f32>(0.0);
    for (var y = -2; y <= 1; y = y + 1) {
        for (var x = -2; x <= 1; x = x + 1) {
            let c = clamp(coord + vec2<i32>(x, y), vec2<i32>(0), vec2<i32>(dims) - 1);
            let d = textureLoad(depth_tex, c, 0);
            // Reject neighbours on the far side of a depth discontinuity.
            let w = select(0.0, 1.0, abs(d - centre_depth) < 0.001);
            let sample = textureLoad(gtao_in, c, 0);
            total += sample.a * w;
            bent += (sample.rgb * 2.0 - 1.0) * w;
            weight_sum += w;
        }
    }
    let ao = select(1.0, total / weight_sum, weight_sum > 0.0);
    var bent_n = vec3<f32>(0.0, 0.0, 1.0);
    if length(bent) > 1e-4 {
        bent_n = normalize(bent);
    }
    textureStore(gtao_denoised, coord, vec4<f32>(bent_n * 0.5 + 0.5, ao));
}
