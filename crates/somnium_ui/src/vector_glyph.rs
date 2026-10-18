//! A mark drawn from an SVG at any size and angle, in one colour: objective markers, pointers, a game's own sigils.
//!
//! The SVG (from VectorCraft, the Craft tools) is read once into the UI's own paths, in the drawing's coordinates;
//! drawing places, turns and scales them through the transform stack, so the flatten cache holds one entry per
//! glyph however often it moves. Fills and strokes keep their opacity; their colours give way to the tint, which is
//! what keeps a HUD to one restrained colour whatever the drawing was made in.

use crate::draw::DrawingContext;
use crate::path::{Cap, Join, Path, Stroke};
use crate::shaped::ShapedInstance;
use glam::Vec2;
use resvg::tiny_skia::PathSegment;
use resvg::usvg;

#[derive(Clone, Debug)]
struct Part {
    path: Path,
    /// Fill opacity, when filled.
    fill: Option<f32>,
    /// Stroke width (drawing units) and opacity, when stroked.
    stroke: Option<(Stroke, f32)>,
}

/// One parsed mark.
#[derive(Clone, Debug)]
pub struct VectorGlyph {
    size: Vec2,
    parts: Vec<Part>,
}

fn walk(group: &usvg::Group, parts: &mut Vec<Part>) {
    for node in group.children() {
        match node {
            usvg::Node::Group(inner) => walk(inner, parts),
            usvg::Node::Path(shape) if shape.is_visible() => {
                let t = shape.abs_transform();
                let at = |x: f32, y: f32| {
                    Vec2::new(t.sx * x + t.kx * y + t.tx, t.ky * x + t.sy * y + t.ty)
                };
                let mut path = Path::new();
                for segment in shape.data().segments() {
                    match segment {
                        PathSegment::MoveTo(p) => {
                            path.move_to(at(p.x, p.y));
                        }
                        PathSegment::LineTo(p) => {
                            path.line_to(at(p.x, p.y));
                        }
                        PathSegment::QuadTo(c, p) => {
                            path.quad_to(at(c.x, c.y), at(p.x, p.y));
                        }
                        PathSegment::CubicTo(a, b, p) => {
                            path.cubic_to(at(a.x, a.y), at(b.x, b.y), at(p.x, p.y));
                        }
                        PathSegment::Close => {
                            path.close();
                        }
                    }
                }
                let scale = (t.sx * t.sy - t.kx * t.ky).abs().sqrt();
                let stroke = shape.stroke().map(|s| {
                    let join = match s.linejoin() {
                        usvg::LineJoin::Round => Join::Round,
                        usvg::LineJoin::Bevel => Join::Bevel,
                        _ => Join::Miter,
                    };
                    let cap = match s.linecap() {
                        usvg::LineCap::Round => Cap::Round,
                        usvg::LineCap::Square => Cap::Square,
                        usvg::LineCap::Butt => Cap::Butt,
                    };
                    (
                        Stroke::new(s.width().get() * scale)
                            .with_join(join)
                            .with_cap(cap),
                        s.opacity().get(),
                    )
                });
                parts.push(Part {
                    path,
                    fill: shape.fill().map(|f| f.opacity().get()),
                    stroke,
                });
            }
            _ => {}
        }
    }
}

impl VectorGlyph {
    /// Read an SVG's paths. Text, images and gradients are not marks and are left out.
    pub fn parse(svg: &str) -> Result<Self, String> {
        let tree =
            usvg::Tree::from_str(svg, &usvg::Options::default()).map_err(|e| e.to_string())?;
        let mut parts = Vec::new();
        walk(tree.root(), &mut parts);
        if parts.is_empty() {
            return Err("the SVG has no paths".into());
        }
        let size = tree.size();
        Ok(Self {
            size: Vec2::new(size.width(), size.height()),
            parts,
        })
    }

    /// The drawing's own size (its view box).
    #[must_use]
    pub fn size(&self) -> Vec2 {
        self.size
    }

    /// Draw so that the drawing's point `anchor` (in its own units) lands on `at`, the drawing `px` tall, turned by
    /// `radians` about that point, in `color` (its alpha times each part's opacity).
    pub fn draw(
        &self,
        ctx: &mut DrawingContext,
        anchor: Vec2,
        at: Vec2,
        px: f32,
        radians: f32,
        color: [u8; 4],
    ) {
        let s = px / self.size.y.max(1e-3);
        let (sin, cos) = radians.sin_cos();
        let (a, b, c, d) = (s * cos, s * sin, -s * sin, s * cos);
        let place = ShapedInstance {
            xform: [
                a,
                b,
                c,
                d,
                at.x - (a * anchor.x + c * anchor.y),
                at.y - (b * anchor.x + d * anchor.y),
            ],
            ..ShapedInstance::identity([255, 255, 255, 255])
        };
        let tint = |opacity: f32| {
            let mut tinted = color;
            tinted[3] = (f32::from(color[3]) * opacity.clamp(0.0, 1.0)).round() as u8;
            ShapedInstance::identity(tinted)
        };
        ctx.push_transformed(place);
        for part in &self.parts {
            if let Some(opacity) = part.fill {
                ctx.push_path(&part.path, tint(opacity));
            }
            if let Some((stroke, opacity)) = &part.stroke {
                ctx.push_stroke(&part.path, stroke, tint(*opacity));
            }
        }
        ctx.pop_transform();
    }
}

/// The objective mark, drawn in VectorCraft (`assets/story/source`): a lamp-flame lozenge with a core and a stem
/// whose tip, at `(12, 21.4)`, points at the place.
pub const OBJECTIVE_MARK_SVG: &str = include_str!("../assets/story/objective_mark.svg");
/// The edge-of-screen pointer, facing +x, its chevron's point at `(16.5, 12)`.
pub const OBJECTIVE_POINTER_SVG: &str = include_str!("../assets/story/objective_pointer.svg");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_marks_parse_into_fills_and_round_strokes() {
        let mark = VectorGlyph::parse(OBJECTIVE_MARK_SVG).unwrap();
        assert_eq!(mark.size(), Vec2::new(24.0, 24.0));
        assert_eq!(mark.parts.len(), 3);
        assert_eq!(
            mark.parts.iter().filter(|p| p.fill.is_some()).count(),
            1,
            "the core is the one fill"
        );
        assert!(
            mark.parts
                .iter()
                .filter_map(|p| p.stroke.as_ref())
                .all(|(s, _)| (s.width - 1.35).abs() < 1e-4)
        );
        let pointer = VectorGlyph::parse(OBJECTIVE_POINTER_SVG).unwrap();
        assert!(
            matches!(&pointer.parts[0].stroke, Some((s, _)) if s.cap == Cap::Round && s.join == Join::Round)
        );
        assert!(
            VectorGlyph::parse("<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'/>")
                .is_err()
        );
        assert!(VectorGlyph::parse("not svg").is_err());
    }
}
