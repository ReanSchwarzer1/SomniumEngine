//! Internal 3D resolution caps, independent of the swapchain / UI.
//!
//! The editor window (and fullscreen) stay at the display's pixel size so
//! chrome and gizmos stay sharp. Scene passes render into a smaller target
//! and bilinear-upscale. Index 0 is Native — no cap.

/// Labels for the viewport toolbar combo. Order matches [`scene_size_for_preset`].
pub const VIEWPORT_RESOLUTION_LABELS: [&str; 5] =
    ["Native", "2560×1440", "1920×1080", "1600×900", "1280×720"];

const CAPS: [(u32, u32); 5] = [(0, 0), (2560, 1440), (1920, 1080), (1600, 900), (1280, 720)];

/// Scene buffer size for a window and a named preset.
///
/// Fits inside both the window and the preset box, so a 2560×1440 fullscreen
/// view at 1920×1080 is exactly 1920×1080, and a 1080p window never supersamples.
pub fn scene_size_for_preset(window_w: u32, window_h: u32, preset: usize) -> (u32, u32) {
    let w = window_w.max(1);
    let h = window_h.max(1);
    let Some(&(cap_w, cap_h)) = CAPS.get(preset) else {
        return (w, h);
    };
    if cap_w == 0 || cap_h == 0 {
        return (w, h);
    }
    let scale = (cap_w as f32 / w as f32)
        .min(cap_h as f32 / h as f32)
        .min(1.0);
    (
        ((w as f32 * scale).round() as u32).max(1),
        ((h as f32 * scale).round() as u32).max(1),
    )
}

#[cfg(test)]
mod tests {
    use super::scene_size_for_preset;

    #[test]
    fn native_matches_the_window() {
        assert_eq!(scene_size_for_preset(2560, 1440, 0), (2560, 1440));
    }

    #[test]
    fn two_k_window_at_1080p_is_exactly_1080p() {
        assert_eq!(scene_size_for_preset(2560, 1440, 2), (1920, 1080));
    }

    #[test]
    fn never_exceeds_the_window() {
        assert_eq!(scene_size_for_preset(1280, 720, 2), (1280, 720));
    }
}
