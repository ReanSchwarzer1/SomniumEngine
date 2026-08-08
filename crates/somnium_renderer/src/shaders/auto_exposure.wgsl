// Phase 24A-3: auto-exposure by luminance histogram.
//
// Two compute passes over the HDR target:
//
//   1. `build_histogram` bins every pixel by log2 luminance into 256 buckets.
//   2. `resolve_exposure` reduces those buckets to an average and adapts the
//      stored exposure toward it over time.
//
// A histogram rather than a simple average because a scene's luminance range is
// enormous once lights are in physical units — a sun disc at 10⁵ cd/m² sitting
// in a few pixels would drag a plain mean far off what the frame actually looks
// like. Binning in log space and discarding the darkest and brightest tails
// meters the way a camera's centre-weighted average does: on the subject rather
// than on the extremes.

struct ExposureParams {
    /// Reciprocal of the log-luminance range, for mapping luminance → bin.
    inv_log_range:   f32,
    /// Lowest log2 luminance the histogram covers.
    min_log_lum:     f32,
    /// Seconds since the last frame, for adaptation.
    delta_time:      f32,
    /// Adaptation rate when the scene gets brighter (eye closes down: fast).
    speed_down:      f32,
    /// Adaptation rate when the scene gets darker (eye opens up: slow).
    speed_up:        f32,
    /// Stops added to the metered result.
    exposure_compensation: f32,
    /// Clamp on the metered EV100, low end.
    min_ev100:       f32,
    /// Clamp on the metered EV100, high end.
    max_ev100:       f32,
}

@group(0) @binding(0) var          hdr_tex:   texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> histogram: array<atomic<u32>, 256>;
@group(0) @binding(2) var<storage, read_write> exposure:  array<f32, 2>;
@group(0) @binding(3) var<uniform>             params:    ExposureParams;

/// Rec. 709 luminance.
fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

/// Map a luminance to a histogram bin.
///
/// Bin 0 is reserved for "effectively black" so that large empty regions of sky
/// or shadow do not drag the average down; everything else spreads across the
/// remaining 255 bins in log space.
fn luminance_to_bin(lum: f32) -> u32 {
    if lum < 1e-5 {
        return 0u;
    }
    let log_lum = clamp((log2(lum) - params.min_log_lum) * params.inv_log_range, 0.0, 1.0);
    return u32(log_lum * 254.0 + 1.0);
}

var<workgroup> local_bins: array<atomic<u32>, 256>;

@compute @workgroup_size(16, 16, 1)
fn build_histogram(
    @builtin(global_invocation_id)   gid: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    // Zero this workgroup's private copy first. Accumulating locally and
    // merging once per workgroup keeps 65 536 pixels from contending on 256
    // global atomics.
    atomicStore(&local_bins[lid], 0u);
    workgroupBarrier();

    let dims = textureDimensions(hdr_tex);
    if gid.x < dims.x && gid.y < dims.y {
        let color = textureLoad(hdr_tex, vec2<i32>(gid.xy), 0).rgb;
        atomicAdd(&local_bins[luminance_to_bin(luminance(color))], 1u);
    }

    workgroupBarrier();
    let count = atomicLoad(&local_bins[lid]);
    if count > 0u {
        atomicAdd(&histogram[lid], count);
    }
}

var<workgroup> bin_totals: array<u32, 256>;

@compute @workgroup_size(256, 1, 1)
fn resolve_exposure(@builtin(local_invocation_index) lid: u32) {
    let count = atomicLoad(&histogram[lid]);

    // Weight each bin by its own index, so the sum reduces to a weighted mean
    // of log luminance rather than of luminance itself.
    bin_totals[lid] = count * lid;
    workgroupBarrier();

    // Parallel reduction over the 256 bins.
    for (var cutoff = 128u; cutoff > 0u; cutoff >>= 1u) {
        if lid < cutoff {
            bin_totals[lid] += bin_totals[lid + cutoff];
        }
        workgroupBarrier();
    }

    if lid != 0u {
        // Clear for the next frame while thread 0 finishes.
        atomicStore(&histogram[lid], 0u);
        return;
    }

    // Bin 0 held the near-black pixels and is deliberately excluded, so the
    // denominator is the number of pixels that carried usable signal.
    let black_pixels = f32(count);
    let weighted_sum = f32(bin_totals[0]);
    let dims = textureDimensions(hdr_tex);
    let lit_pixels = max(f32(dims.x * dims.y) - black_pixels, 1.0);

    // Undo the bin mapping to recover average log2 luminance.
    let mean_bin = weighted_sum / lit_pixels;
    let avg_log_lum = (mean_bin - 1.0) / 254.0 / params.inv_log_range + params.min_log_lum;
    let avg_luminance = exp2(avg_log_lum);

    // Saturation-based metering (Lagarde & de Rousiers): the EV that places
    // this average luminance at middle grey.
    var target_ev = log2(max(avg_luminance, 1e-5) * 100.0 / 12.5);
    target_ev = clamp(target_ev - params.exposure_compensation,
                      params.min_ev100, params.max_ev100);

    // Exponential adaptation, framed so the rate is per second rather than per
    // frame — otherwise the eye adjusts at a speed that depends on frame rate.
    let previous = exposure[1];
    var adapted = target_ev;
    if previous > -100.0 {
        let speed = select(params.speed_up, params.speed_down, target_ev > previous);
        adapted = previous + (target_ev - previous) * (1.0 - exp(-params.delta_time * speed));
    }

    exposure[1] = adapted;
    // The multiplier the post-process pass actually reads: 1 / (1.2 · 2^EV).
    exposure[0] = 1.0 / (1.2 * exp2(adapted));

    atomicStore(&histogram[0], 0u);
}
