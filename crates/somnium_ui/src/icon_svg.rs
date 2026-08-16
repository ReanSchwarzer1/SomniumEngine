//! Phase 26-Zeta-E — the vendored SVG icon sources and their rasterizer.
//!
//! Before Zeta-E every editor glyph was drawn with procedural line and arc
//! calls in [`crate::icons`]. That was the right call when the phase needed
//! original, licence-clean art in a hurry, but it cannot produce an optically
//! consistent family: each glyph carried its own hand-tuned stroke weight and
//! terminal, which is exactly the 2/5 iconography score in `phase_26_Zeta.md`
//! §2.3.
//!
//! The family is now [Tabler](https://github.com/tabler/tabler-icons) (MIT),
//! drawn on a 24 × 24 grid with 2 px round-capped strokes, plus the sixteen
//! original Somnium icons from the approved design package, which are drawn on
//! the same grid so the two sets are optically indistinguishable. Sources are
//! vendored under `assets/icons/` and compiled in with `include_str!`, so the
//! editor has no runtime file dependency and no way to start with a half-built
//! atlas.
//!
//! **Raster size.** Cells are [`crate::icons::ICON_CELL`] = 32 px and glyphs
//! draw at 16 / 20 / 24 logical px. 32 → 16 is an exact 2:1 box filter under
//! linear sampling, and 32 → 20/24 is a mild downscale, so a single cut serves
//! every size at 100 % without mipmaps. At 200 % the 24 px action icons upscale
//! by 1.5× and go slightly soft; a DPI-aware regeneration is tracked as
//! remaining Zeta-E work rather than pretended away here.
//!
//! Only the alpha channel survives. Tabler sources stroke in `currentColor`,
//! which usvg resolves to opaque black; the atlas keeps RGB at white and uses
//! the rendered alpha as coverage, so the existing UI shader tints each glyph
//! with a semantic colour at draw time.

use crate::icons::IconId;

/// One vendored source per glyph. Icons absent from this table keep their
/// procedural fallback, so adding a variant to `IconId` cannot blank the UI.
pub const SVG_SOURCES: &[(IconId, &str)] = &[
    // ── Somnium extension — original, from the approved design package ──────
    (
        IconId::SculptRaise,
        include_str!("../assets/icons/somnium/sculpt-raise.svg"),
    ),
    (
        IconId::SculptLower,
        include_str!("../assets/icons/somnium/sculpt-lower.svg"),
    ),
    (
        IconId::SculptSmooth,
        include_str!("../assets/icons/somnium/sculpt-smooth.svg"),
    ),
    (
        IconId::SculptFlatten,
        include_str!("../assets/icons/somnium/sculpt-flatten.svg"),
    ),
    (
        IconId::SculptNoise,
        include_str!("../assets/icons/somnium/sculpt-noise.svg"),
    ),
    (
        IconId::PaintLayer,
        include_str!("../assets/icons/somnium/paint-layer.svg"),
    ),
    (
        IconId::FoliagePaint,
        include_str!("../assets/icons/somnium/foliage-paint.svg"),
    ),
    (
        IconId::FoliageErase,
        include_str!("../assets/icons/somnium/foliage-erase.svg"),
    ),
    (
        IconId::FoliageSingle,
        include_str!("../assets/icons/somnium/foliage-single.svg"),
    ),
    (
        IconId::Water,
        include_str!("../assets/icons/somnium/water-body.svg"),
    ),
    (
        IconId::Vessel,
        include_str!("../assets/icons/somnium/vessel.svg"),
    ),
    (
        IconId::VoxelTerrain,
        include_str!("../assets/icons/somnium/voxel-terrain.svg"),
    ),
    (
        IconId::PostFx,
        include_str!("../assets/icons/somnium/post-fx.svg"),
    ),
    (
        IconId::LightProbe,
        include_str!("../assets/icons/somnium/light-probe.svg"),
    ),
    (
        IconId::MaterialGraph,
        include_str!("../assets/icons/somnium/material-graph.svg"),
    ),
    (
        IconId::RayTrace,
        include_str!("../assets/icons/somnium/ray-trace.svg"),
    ),
    // ── Brand ───────────────────────────────────────────────────────────────
    // Route A "Eclipse": two counter-rotating crescent blades whose negative
    // space reads as the S. The 16 px micro cut is a separate drawing with
    // simplified counters; the mark draws at 24 px in the application scope, so
    // the full construction is the right source here.
    (
        IconId::EngineMark,
        include_str!("../assets/brand/somnium-s-eclipse.svg"),
    ),
    // ── Tabler (MIT) ────────────────────────────────────────────────────────
    (
        IconId::File,
        include_str!("../assets/icons/tabler/file.svg"),
    ),
    (
        IconId::Edit,
        include_str!("../assets/icons/tabler/edit.svg"),
    ),
    (
        IconId::View,
        include_str!("../assets/icons/tabler/view.svg"),
    ),
    (
        IconId::Window,
        include_str!("../assets/icons/tabler/window.svg"),
    ),
    (
        IconId::Help,
        include_str!("../assets/icons/tabler/help.svg"),
    ),
    (
        IconId::HelpCircle,
        include_str!("../assets/icons/tabler/help-circle.svg"),
    ),
    (
        IconId::Save,
        include_str!("../assets/icons/tabler/save.svg"),
    ),
    (
        IconId::Undo,
        include_str!("../assets/icons/tabler/undo.svg"),
    ),
    (
        IconId::Redo,
        include_str!("../assets/icons/tabler/redo.svg"),
    ),
    (
        IconId::Play,
        include_str!("../assets/icons/tabler/play.svg"),
    ),
    (
        IconId::Pause,
        include_str!("../assets/icons/tabler/pause.svg"),
    ),
    (
        IconId::Stop,
        include_str!("../assets/icons/tabler/stop.svg"),
    ),
    (
        IconId::Translate,
        include_str!("../assets/icons/tabler/translate.svg"),
    ),
    (
        IconId::Rotate,
        include_str!("../assets/icons/tabler/rotate.svg"),
    ),
    (
        IconId::Scale,
        include_str!("../assets/icons/tabler/scale.svg"),
    ),
    (
        IconId::Select,
        include_str!("../assets/icons/tabler/select.svg"),
    ),
    (
        IconId::Landscape,
        include_str!("../assets/icons/tabler/landscape.svg"),
    ),
    (
        IconId::Foliage,
        include_str!("../assets/icons/tabler/foliage.svg"),
    ),
    (
        IconId::Search,
        include_str!("../assets/icons/tabler/search.svg"),
    ),
    (
        IconId::Filter,
        include_str!("../assets/icons/tabler/filter.svg"),
    ),
    (
        IconId::Settings,
        include_str!("../assets/icons/tabler/settings.svg"),
    ),
    (
        IconId::Dock,
        include_str!("../assets/icons/tabler/dock.svg"),
    ),
    (
        IconId::Close,
        include_str!("../assets/icons/tabler/close.svg"),
    ),
    (
        IconId::Folder,
        include_str!("../assets/icons/tabler/folder.svg"),
    ),
    (
        IconId::FolderOpen,
        include_str!("../assets/icons/tabler/folder-open.svg"),
    ),
    (
        IconId::Chevron,
        include_str!("../assets/icons/tabler/chevron.svg"),
    ),
    (
        IconId::ChevronDown,
        include_str!("../assets/icons/tabler/chevron-down.svg"),
    ),
    (
        IconId::Visibility,
        include_str!("../assets/icons/tabler/visibility.svg"),
    ),
    (IconId::Add, include_str!("../assets/icons/tabler/add.svg")),
    (
        IconId::Delete,
        include_str!("../assets/icons/tabler/delete.svg"),
    ),
    (
        IconId::Duplicate,
        include_str!("../assets/icons/tabler/duplicate.svg"),
    ),
    (
        IconId::Import,
        include_str!("../assets/icons/tabler/import.svg"),
    ),
    (
        IconId::Profiler,
        include_str!("../assets/icons/tabler/profiler.svg"),
    ),
    (
        IconId::OutputLog,
        include_str!("../assets/icons/tabler/output-log.svg"),
    ),
    (
        IconId::ContentDrawer,
        include_str!("../assets/icons/tabler/content-drawer.svg"),
    ),
    (
        IconId::Cube,
        include_str!("../assets/icons/tabler/cube.svg"),
    ),
    (
        IconId::Sphere,
        include_str!("../assets/icons/tabler/sphere.svg"),
    ),
    (
        IconId::Plane,
        include_str!("../assets/icons/tabler/plane.svg"),
    ),
    (
        IconId::Cylinder,
        include_str!("../assets/icons/tabler/cylinder.svg"),
    ),
    (
        IconId::DirectionalLight,
        include_str!("../assets/icons/tabler/directional-light.svg"),
    ),
    (
        IconId::PointLight,
        include_str!("../assets/icons/tabler/point-light.svg"),
    ),
    (
        IconId::SpotLight,
        include_str!("../assets/icons/tabler/spot-light.svg"),
    ),
    (
        IconId::Particle,
        include_str!("../assets/icons/tabler/particle.svg"),
    ),
    (
        IconId::Terrain,
        include_str!("../assets/icons/tabler/terrain.svg"),
    ),
    (
        IconId::EmptyEntity,
        include_str!("../assets/icons/tabler/empty-entity.svg"),
    ),
    (
        IconId::Camera,
        include_str!("../assets/icons/tabler/camera.svg"),
    ),
    (
        IconId::Mesh,
        include_str!("../assets/icons/tabler/mesh.svg"),
    ),
    (
        IconId::Texture,
        include_str!("../assets/icons/tabler/texture.svg"),
    ),
    (
        IconId::Material,
        include_str!("../assets/icons/tabler/material.svg"),
    ),
    (
        IconId::Scene,
        include_str!("../assets/icons/tabler/scene.svg"),
    ),
    (
        IconId::Audio,
        include_str!("../assets/icons/tabler/audio.svg"),
    ),
    (
        IconId::Shader,
        include_str!("../assets/icons/tabler/shader.svg"),
    ),
    (
        IconId::Font,
        include_str!("../assets/icons/tabler/font.svg"),
    ),
    (
        IconId::Script,
        include_str!("../assets/icons/tabler/script.svg"),
    ),
    (
        IconId::Json,
        include_str!("../assets/icons/tabler/json.svg"),
    ),
    (
        IconId::License,
        include_str!("../assets/icons/tabler/license.svg"),
    ),
    (
        IconId::Unknown,
        include_str!("../assets/icons/tabler/unknown.svg"),
    ),
    (
        IconId::Derived,
        include_str!("../assets/icons/tabler/derived.svg"),
    ),
    (
        IconId::Transform,
        include_str!("../assets/icons/tabler/transform.svg"),
    ),
    (
        IconId::Light,
        include_str!("../assets/icons/tabler/light.svg"),
    ),
    (IconId::Ok, include_str!("../assets/icons/tabler/ok.svg")),
    (
        IconId::Warn,
        include_str!("../assets/icons/tabler/warn.svg"),
    ),
    (
        IconId::Error,
        include_str!("../assets/icons/tabler/error.svg"),
    ),
    (
        IconId::Check,
        include_str!("../assets/icons/tabler/check.svg"),
    ),
    (
        IconId::Minimize,
        include_str!("../assets/icons/tabler/minimize.svg"),
    ),
    (
        IconId::Maximize,
        include_str!("../assets/icons/tabler/maximize.svg"),
    ),
    (
        IconId::ImmersivePlay,
        include_str!("../assets/icons/tabler/immersive-play.svg"),
    ),
];

/// Rasterize one SVG source into a `size × size` alpha mask.
///
/// Returns `None` if the source fails to parse or the pixmap cannot be
/// allocated; callers fall back to the procedural glyph rather than leaving a
/// hole in the atlas.
pub fn rasterize(source: &str, size: u32) -> Option<Vec<u8>> {
    let options = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_str(source, &options).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;

    // Every source declares a 24×24 viewBox, but scale from the parsed size so
    // a future source with a different box is not silently cropped.
    let sz = tree.size();
    let scale = (size as f32 / sz.width().max(1.0)).min(size as f32 / sz.height().max(1.0));
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // Keep coverage only. The sources stroke in `currentColor`, which resolves
    // to opaque black; RGB is discarded so the UI shader can tint the glyph
    // with whatever semantic colour the widget's state resolves to.
    Some(pixmap.pixels().iter().map(|p| p.alpha()).collect())
}

/// Look up the vendored source for a glyph, if it has one.
pub fn source_for(id: IconId) -> Option<&'static str> {
    SVG_SOURCES
        .iter()
        .find(|(candidate, _)| *candidate == id)
        .map(|(_, svg)| *svg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_vendored_source_parses_and_marks_pixels() {
        // A source that parses but renders nothing is worse than a missing one:
        // it silently replaces a working procedural glyph with a blank cell.
        for (id, svg) in SVG_SOURCES {
            let mask = rasterize(svg, crate::icons::ICON_CELL)
                .unwrap_or_else(|| panic!("{id:?} failed to rasterize"));
            let covered = mask.iter().filter(|&&a| a > 8).count();
            assert!(
                covered > 12,
                "{id:?} rendered only {covered} covered pixels — check the viewBox and stroke"
            );
        }
    }

    #[test]
    fn sources_are_unique_per_icon() {
        let mut seen = std::collections::HashSet::new();
        for (id, _) in SVG_SOURCES {
            assert!(seen.insert(*id), "{id:?} is listed twice");
        }
    }

    #[test]
    fn the_engine_mark_is_the_original_eclipse_route() {
        // Guards against the brand slot quietly picking up a utility glyph.
        let mark = source_for(IconId::EngineMark).expect("brand mark must be vendored");
        assert!(mark.contains("<svg"));
        assert!(!mark.contains("tabler"));
    }
}
