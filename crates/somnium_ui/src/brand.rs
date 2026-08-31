//! The engine's own mark, at sizes the operating system asks for.
//!
//! MORROWIND-J step 2. The editor has drawn its mark in its own title bar since
//! Phase 26; what it has never had is an icon the *system* can show, so the
//! taskbar button, the alt-tab card and the window list all fell back to the
//! blank default. Those are the places a person looks to find a window among
//! twenty others, and a blank one is the hardest to find.
//!
//! The source is `assets/brand/somnium-s-eclipse.svg`, the same drawing
//! [`crate::icons::IconId::EngineMark`] uses. One drawing rather than a
//! hand-made bitmap beside it, because a mark that drifts from the one in the
//! title bar is worse than no mark: it reads as two applications.

/// One rasterised mark: RGBA8, `size` square, ready for `winit::window::Icon`.
pub struct MarkImage {
    pub rgba: Vec<u8>,
    pub size: u32,
}

/// The size Windows asks for in the taskbar and alt-tab (`ICON_BIG`).
///
/// 64 rather than 256: the mark is two strokes on a 64-unit grid, so a larger
/// raster buys nothing but memory, and Windows downsamples cleanly.
pub const LARGE: u32 = 64;

/// The size a title bar and a window-list row ask for (`ICON_SMALL`).
pub const SMALL: u32 = 32;

/// Rasterise the engine mark at `size`, tinted and on a transparent field.
///
/// Returns `None` only if the vendored source fails to parse or the pixmap
/// cannot be allocated, which is a broken build rather than a runtime
/// condition — callers carry on without an icon rather than refusing to open a
/// window over a missing decoration.
#[must_use]
pub fn mark(size: u32, tint: [u8; 4]) -> Option<MarkImage> {
    let source = crate::icon_svg::source_for(crate::icons::IconId::EngineMark)?;
    // The shared rasteriser keeps coverage and discards colour, which is
    // exactly what is wanted: the mark strokes in `currentColor`, and the tint
    // belongs to whoever is drawing it.
    let coverage = crate::icon_svg::rasterize(source, size)?;
    let mut rgba = Vec::with_capacity(coverage.len() * 4);
    for alpha in coverage {
        // Straight alpha, not premultiplied: `Icon::from_rgba` documents 32bpp
        // RGBA and Windows composites it itself. Premultiplying here dims the
        // edge pixels of every stroke.
        rgba.extend_from_slice(&[tint[0], tint[1], tint[2], alpha]);
    }
    Some(MarkImage { rgba, size })
}

/// The mark in the editor's accent, which is what the title bar draws.
#[must_use]
pub fn accent_mark(size: u32) -> Option<MarkImage> {
    mark(size, crate::theme::ACCENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_rasterises_at_both_sizes_the_system_asks_for() {
        for size in [SMALL, LARGE] {
            let image = mark(size, [0x4C, 0x8D, 0xFF, 0xFF]).expect("vendored source parses");
            assert_eq!(image.size, size);
            assert_eq!(
                image.rgba.len(),
                (size * size * 4) as usize,
                "Icon::from_rgba rejects any other length"
            );
        }
    }

    #[test]
    fn the_mark_is_drawn_rather_than_a_transparent_square() {
        // The failure this catches is silent: a source that parsed but rendered
        // nothing gives a fully transparent icon, which looks exactly like
        // having no icon at all.
        let image = accent_mark(LARGE).expect("vendored source parses");
        let lit = image.rgba.chunks_exact(4).filter(|px| px[3] > 8).count();
        let total = (LARGE * LARGE) as usize;
        assert!(
            lit > total / 20,
            "only {lit} of {total} pixels have any coverage"
        );
        assert!(
            lit < total * 9 / 10,
            "{lit} of {total} pixels covered; the mark is strokes, not a fill"
        );
    }

    #[test]
    fn the_tint_reaches_every_covered_pixel() {
        let tint = [0x11, 0x22, 0x33, 0xFF];
        let image = mark(SMALL, tint).expect("vendored source parses");
        for px in image.rgba.chunks_exact(4).filter(|px| px[3] > 0) {
            assert_eq!([px[0], px[1], px[2]], [tint[0], tint[1], tint[2]]);
        }
    }
}
