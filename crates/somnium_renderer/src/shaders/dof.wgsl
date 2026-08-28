// MORROWIND-C: composition is declared here rather than assembled by a
// `format!` of `include_str!` calls at this pass's construction site. The
// resolver (`somnium_shader`) emits each module once, in this order, and
// hoists every `enable` above everything.
//!include "sampling.wgsl"

// Phase 24Z: depth of field.
//
// Nearly free once the camera is physical. Phase 24A gave it an aperture in
// f-stops for exposure, and that same aperture is exactly what sets the circle
// of confusion — the two are the same number doing two jobs, which is why a
// real photograph cannot change one without the other. Opening up to f/1.4 both
// brightens the frame and throws the background out of focus, and a renderer
// that lets you do one without the other is telling a small lie in every shot.

struct DofParams {
    /// Reciprocal render size.
    inv_resolution: vec2<f32>,
    /// Distance in metres the lens is focused at.
    focus_distance: f32,
    /// Aperture diameter in metres — focal length divided by the f-number.
    aperture: f32,
    /// Lens focal length in metres.
    focal_length: f32,
    /// Largest circle of confusion allowed, in pixels. Bounds the sample
    /// radius so a badly-set focus cannot cost the whole frame time.
    max_coc: f32,
    /// Near and far planes, for linearising depth.
    near: f32,
    far: f32,
}

@group(0) @binding(0) var color_tex: texture_2d<f32>;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var samp:      sampler;
@group(0) @binding(3) var<uniform> params: DofParams;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0)       uv:   vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VOut {
    var xs = array<f32, 3>(-1.0,  3.0, -1.0);
    var ys = array<f32, 3>(-1.0, -1.0,  3.0);
    let p  = vec2<f32>(xs[vid], ys[vid]);
    return VOut(vec4<f32>(p, 0.0, 1.0), vec2<f32>((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5));
}

/// Depth buffer value to metres in front of the camera.
fn linear_depth(d: f32) -> f32 {
    let n = params.near;
    let f = params.far;
    return (n * f) / max(f - d * (f - n), 1e-6);
}

/// Circle of confusion in metres on the sensor, from the thin-lens equation.
///
/// Signed: negative in front of the focal plane, positive behind. The sign is
/// what a full implementation uses to keep near blur from bleeding over sharp
/// background, and it is preserved here even though this gather does not yet
/// separate the two fields.
fn circle_of_confusion(depth_m: f32) -> f32 {
    let s = max(params.focus_distance, 1e-3);
    let d = max(depth_m, 1e-3);
    let f = params.focal_length;
    // |A · f · (d − s)| / (d · (s − f))
    return params.aperture * f * (d - s) / (d * max(s - f, 1e-4));
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let centre = textureSampleLevel(color_tex, samp, in.uv, 0.0).rgb;
    let depth = textureSampleLevel(depth_tex, samp, in.uv, 0);

    // Sky stays sharp: a point at the far plane is not a real distance, and
    // blurring it produces a halo along every silhouette against it.
    if depth >= 1.0 {
        return vec4<f32>(centre, 1.0);
    }

    // Sensor-relative CoC scaled to pixels. A 36 mm full-frame sensor is the
    // reference the f-stop numbers assume.
    const SENSOR_WIDTH_M: f32 = 0.036;
    let coc_m = circle_of_confusion(linear_depth(depth));
    let coc_px = abs(coc_m) / SENSOR_WIDTH_M / params.inv_resolution.x;
    let radius = min(coc_px, params.max_coc);

    // Under a pixel of blur is no blur. Skipping here also keeps the common
    // in-focus case at one texture fetch.
    if radius < 1.0 {
        return vec4<f32>(centre, 1.0);
    }

    // Gather on a Vogel disk, rotated per pixel: a fixed pattern at this sample
    // count reads as a ring of ghosts around every highlight, and rotating it
    // turns those rings into noise the temporal filter can absorb.
    const DOF_SAMPLES: i32 = 24;
    let rotation = interleaved_gradient_noise(in.uv / params.inv_resolution, 0u) * 6.28318530;

    var accum = centre;
    var weight = 1.0;
    for (var i = 0; i < DOF_SAMPLES; i = i + 1) {
        let offset = vogel_disk_sample(u32(i), u32(DOF_SAMPLES), rotation)
            * radius * params.inv_resolution;
        let uv = in.uv + offset;
        let sample_depth = textureSampleLevel(depth_tex, samp, uv, 0);
        let sample_coc = abs(circle_of_confusion(linear_depth(sample_depth)))
            / SENSOR_WIDTH_M / params.inv_resolution.x;

        // Only accept a neighbour if it is itself blurred enough to spread this
        // far. Without that test a sharp foreground bleeds into the blurred
        // background behind it, which reads as a halo and is the classic
        // giveaway of a gather-based depth of field.
        if sample_coc >= length(offset / params.inv_resolution) {
            accum += textureSampleLevel(color_tex, samp, uv, 0.0).rgb;
            weight += 1.0;
        }
    }

    return vec4<f32>(accum / weight, 1.0);
}
