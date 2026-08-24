//! Paths, flattening and tessellation (Phase MORROWIND, MORROWIND-D).
//!
//! Seam 4b's CPU half. `phase_MORROWIND.md` §8 item 2 calls this out as the
//! single item that unblocks the most later work:
//!
//! > *"line, polyline, quadratic and cubic bezier, arc, with joins and caps.
//! > Tessellated on the CPU into the shaped stream. This single item unblocks
//! > the node graph's wires (MORROWIND-K), the timeline's curves (MORROWIND-L),
//! > the spline editor (MORROWIND-P) and every radial or rotated game widget."*
//!
//! # Why CPU tessellation, decided once
//!
//! `bevy-plugins/bevy_vello-main` was read specifically to decide **against** a
//! compute-based vector rasteriser, and §8 asks for the reason to be recorded so
//! the question is not reopened annually. It is:
//!
//! - Somnium's vector needs are **wires, curves, splines and rotated widgets** —
//!   thousands of short strokes, not glyph-density fills with winding rules.
//!   Tessellation is linear in segment count and the segment counts are small.
//! - A compute rasteriser is a second renderer inside the UI pass, with its own
//!   scheduling, its own intermediate targets and its own interaction with the
//!   frozen `draw_over` ordering. That is a large amount of machinery to gain
//!   correctness on cases Somnium does not have.
//! - The output of this module is **triangles in local space**, which the shaped
//!   pipeline consumes exactly like any other geometry. Nothing downstream has
//!   to know a path was involved.
//!
//! Revisit when the UI needs arbitrary filled glyph outlines at speed, or
//! conflation-artifact-free overlapping fills. Neither is on the roadmap.
//!
//! # Tolerance is in device pixels
//!
//! Flattening is keyed by `(path, tolerance)` and cached across frames — a node
//! graph's wires do not change shape while the user pans, and re-flattening them
//! every frame is the obvious performance mistake. **Tolerance is in *device*
//! pixels, so it must be recomputed when DPI changes.** Phase 27 already fixed a
//! DPI correctness bug; the plan is explicit that it must not be reintroduced,
//! and [`Path::flatten`] takes the tolerance as an argument rather than reading
//! a constant precisely so the caller cannot forget.

use glam::Vec2;

/// A flattening tolerance below which subdivision stops being worth it.
///
/// Half a device pixel: the shaped pipeline antialiases analytically, so a
/// chord that deviates by less than half a pixel is inside the antialiasing
/// ramp and subdividing further changes no pixel.
pub const DEFAULT_TOLERANCE: f32 = 0.5;

/// Hard ceiling on the segments one curve may flatten to.
///
/// A degenerate control polygon — a cubic with a cusp, or one built from
/// unvalidated user input — can drive the segment estimate arbitrarily high.
/// The bound is generous enough that no legitimate curve reaches it and low
/// enough that a bad one costs microseconds rather than a frame.
const MAX_SEGMENTS: usize = 256;

/// How a stroke turns a corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Join {
    /// Extend both edges to their intersection, falling back to [`Join::Bevel`]
    /// past the miter limit.
    #[default]
    Miter,
    /// A fan of triangles around the joint.
    Round,
    /// One triangle across the gap.
    Bevel,
}

/// How a stroke ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cap {
    /// Stop at the endpoint.
    #[default]
    Butt,
    /// A half-disc past the endpoint.
    Round,
    /// A half-width square past the endpoint.
    Square,
}

/// A dash pattern, as an on/off period.
///
/// One period rather than an arbitrary array: every dash in the editor and in
/// the plan's named consumers is a simple on/off, and an array turns a cheap
/// arc-length walk into a state machine. When something needs `[4, 2, 1, 2]`,
/// that is the moment to widen this — not before.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dash {
    /// Length of the drawn part.
    pub on: f32,
    /// Length of the gap.
    pub off: f32,
    /// Distance into the pattern at the path's start. Animating this is how a
    /// "marching ants" selection is drawn.
    pub phase: f32,
}

/// Everything a stroke needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    /// Total width in local units. The outline sits half on each side.
    pub width: f32,
    pub join: Join,
    pub cap: Cap,
    /// Past this ratio of miter length to half-width, a miter becomes a bevel.
    /// 4.0 is the SVG default and the reason a nearly-doubled-back polyline
    /// does not grow a spike several screens long.
    pub miter_limit: f32,
    pub dash: Option<Dash>,
}

impl Default for Stroke {
    fn default() -> Self {
        Self {
            width: 1.0,
            join: Join::Miter,
            cap: Cap::Butt,
            miter_limit: 4.0,
            dash: None,
        }
    }
}

impl Stroke {
    /// A plain stroke of `width`.
    #[must_use]
    pub fn new(width: f32) -> Self {
        Self {
            width,
            ..Default::default()
        }
    }

    /// Set the join.
    #[must_use]
    pub fn with_join(mut self, join: Join) -> Self {
        self.join = join;
        self
    }

    /// Set the cap.
    #[must_use]
    pub fn with_cap(mut self, cap: Cap) -> Self {
        self.cap = cap;
        self
    }

    /// Set a dash pattern.
    #[must_use]
    pub fn with_dash(mut self, on: f32, off: f32, phase: f32) -> Self {
        self.dash = Some(Dash { on, off, phase });
        self
    }
}

/// One command in a path.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Segment {
    MoveTo(Vec2),
    LineTo(Vec2),
    QuadTo(Vec2, Vec2),
    CubicTo(Vec2, Vec2, Vec2),
    /// Centre, radii, start angle, sweep, x-axis rotation.
    Arc {
        centre: Vec2,
        radii: Vec2,
        start: f32,
        sweep: f32,
    },
    Close,
}

/// A sequence of contours, in local coordinates.
///
/// `PartialEq`, `Eq` and `Hash` compare **bit patterns**, not float values.
/// That is deliberate and it is what makes a `Path` a sound cache key: two
/// paths whose control points are bit-identical flatten identically, and
/// `-0.0 == 0.0` while `NaN != NaN` are float rules that would make a cache
/// either collide or never hit. Bit equality has neither problem.
#[derive(Clone, Debug, Default)]
pub struct Path {
    segments: Vec<Segment>,
}

/// Bit-pattern view of a path, for hashing and equality.
fn path_bits(path: &Path) -> impl Iterator<Item = u32> + '_ {
    path.segments.iter().flat_map(|segment| match *segment {
        Segment::MoveTo(p) => vec![0, p.x.to_bits(), p.y.to_bits()],
        Segment::LineTo(p) => vec![1, p.x.to_bits(), p.y.to_bits()],
        Segment::QuadTo(c, p) => vec![2, c.x.to_bits(), c.y.to_bits(), p.x.to_bits(), p.y.to_bits()],
        Segment::CubicTo(c1, c2, p) => vec![
            3,
            c1.x.to_bits(),
            c1.y.to_bits(),
            c2.x.to_bits(),
            c2.y.to_bits(),
            p.x.to_bits(),
            p.y.to_bits(),
        ],
        Segment::Arc {
            centre,
            radii,
            start,
            sweep,
        } => vec![
            4,
            centre.x.to_bits(),
            centre.y.to_bits(),
            radii.x.to_bits(),
            radii.y.to_bits(),
            start.to_bits(),
            sweep.to_bits(),
        ],
        Segment::Close => vec![5],
    })
}

impl PartialEq for Path {
    fn eq(&self, other: &Self) -> bool {
        self.segments.len() == other.segments.len() && path_bits(self).eq(path_bits(other))
    }
}

impl Eq for Path {}

impl std::hash::Hash for Path {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for word in path_bits(self) {
            state.write_u32(word);
        }
    }
}

/// One flattened contour: a polyline, possibly closed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contour {
    /// Points in order. A closed contour does **not** repeat its first point.
    pub points: Vec<Vec2>,
    pub closed: bool,
}

impl Path {
    /// An empty path.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a new contour.
    pub fn move_to(&mut self, p: Vec2) -> &mut Self {
        self.segments.push(Segment::MoveTo(p));
        self
    }

    /// Straight line to `p`.
    pub fn line_to(&mut self, p: Vec2) -> &mut Self {
        self.segments.push(Segment::LineTo(p));
        self
    }

    /// Quadratic bezier through control `c` to `p`.
    pub fn quad_to(&mut self, c: Vec2, p: Vec2) -> &mut Self {
        self.segments.push(Segment::QuadTo(c, p));
        self
    }

    /// Cubic bezier through controls `c1`, `c2` to `p`.
    pub fn cubic_to(&mut self, c1: Vec2, c2: Vec2, p: Vec2) -> &mut Self {
        self.segments.push(Segment::CubicTo(c1, c2, p));
        self
    }

    /// An elliptical arc, by centre and sweep.
    ///
    /// Centre-parameterised rather than SVG's endpoint parameterisation,
    /// because every caller in this engine — a pie chart, a radial menu, a
    /// rounded corner, a circle — already knows the centre, and the endpoint
    /// form exists to serialise SVG rather than to be authored.
    pub fn arc(&mut self, centre: Vec2, radii: Vec2, start: f32, sweep: f32) -> &mut Self {
        self.segments.push(Segment::Arc {
            centre,
            radii,
            start,
            sweep,
        });
        self
    }

    /// Close the current contour.
    pub fn close(&mut self) -> &mut Self {
        self.segments.push(Segment::Close);
        self
    }

    /// Whether anything has been added.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// A full circle, as one closed contour.
    #[must_use]
    pub fn circle(centre: Vec2, radius: f32) -> Self {
        let mut path = Self::new();
        path.move_to(centre + Vec2::new(radius, 0.0));
        path.arc(centre, Vec2::splat(radius), 0.0, std::f32::consts::TAU);
        path.close();
        path
    }

    /// A polyline through `points`.
    #[must_use]
    pub fn polyline(points: &[Vec2]) -> Self {
        let mut path = Self::new();
        let Some((first, rest)) = points.split_first() else {
            return path;
        };
        path.move_to(*first);
        for p in rest {
            path.line_to(*p);
        }
        path
    }

    /// A cubic wire from `from` to `to` with horizontal tangents.
    ///
    /// The node-graph connection shape, provided here rather than reinvented in
    /// MORROWIND-K: the handle length scales with horizontal distance so a
    /// short link stays taut and a long one bows, and it has a floor so two
    /// stacked ports do not produce a straight line that hides which way the
    /// edge runs.
    #[must_use]
    pub fn wire(from: Vec2, to: Vec2) -> Self {
        let handle = ((to.x - from.x).abs() * 0.5).max(24.0);
        let mut path = Self::new();
        path.move_to(from);
        path.cubic_to(
            from + Vec2::new(handle, 0.0),
            to - Vec2::new(handle, 0.0),
            to,
        );
        path
    }

    /// Flatten every curve to line segments within `tolerance`.
    ///
    /// `tolerance` is the maximum distance in **device pixels** between the
    /// true curve and the polyline. See the module docs on DPI.
    #[must_use]
    pub fn flatten(&self, tolerance: f32) -> Vec<Contour> {
        let tolerance = tolerance.max(1e-3);
        let mut out: Vec<Contour> = Vec::new();
        let mut current: Option<Contour> = None;
        let mut cursor = Vec2::ZERO;

        // A contour with fewer than two points draws nothing and would make
        // every downstream loop guard against an empty slice.
        let flush = |current: &mut Option<Contour>, out: &mut Vec<Contour>| {
            if let Some(contour) = current.take()
                && contour.points.len() >= 2
            {
                out.push(contour);
            }
        };

        for segment in &self.segments {
            match *segment {
                Segment::MoveTo(p) => {
                    flush(&mut current, &mut out);
                    cursor = p;
                    current = Some(Contour {
                        points: vec![p],
                        closed: false,
                    });
                }
                Segment::LineTo(p) => {
                    let contour = current.get_or_insert_with(|| Contour {
                        points: vec![cursor],
                        closed: false,
                    });
                    push_unique(&mut contour.points, p);
                    cursor = p;
                }
                Segment::QuadTo(c, p) => {
                    let contour = current.get_or_insert_with(|| Contour {
                        points: vec![cursor],
                        closed: false,
                    });
                    flatten_quad(cursor, c, p, tolerance, &mut contour.points);
                    cursor = p;
                }
                Segment::CubicTo(c1, c2, p) => {
                    let contour = current.get_or_insert_with(|| Contour {
                        points: vec![cursor],
                        closed: false,
                    });
                    flatten_cubic(cursor, c1, c2, p, tolerance, &mut contour.points);
                    cursor = p;
                }
                Segment::Arc {
                    centre,
                    radii,
                    start,
                    sweep,
                } => {
                    let contour = current.get_or_insert_with(|| Contour {
                        points: vec![cursor],
                        closed: false,
                    });
                    flatten_arc(centre, radii, start, sweep, tolerance, &mut contour.points);
                    if let Some(last) = contour.points.last() {
                        cursor = *last;
                    }
                }
                Segment::Close => {
                    if let Some(contour) = current.as_mut() {
                        contour.closed = true;
                        // A closed contour does not repeat its first point; the
                        // wrap is implied. Repeating it would make every join
                        // loop emit one degenerate joint.
                        if contour.points.len() >= 2
                            && contour.points.last().is_some_and(|last| {
                                last.distance_squared(contour.points[0]) < 1e-8
                            })
                        {
                            contour.points.pop();
                        }
                        cursor = contour.points.first().copied().unwrap_or(cursor);
                    }
                    flush(&mut current, &mut out);
                }
            }
        }
        flush(&mut current, &mut out);
        out
    }
}

/// Append `p` unless it is already the last point.
///
/// Duplicate points produce zero-length segments, and a zero-length segment has
/// no normal — which is a division by zero in the stroker and a NaN in the
/// vertex buffer, and NaN geometry is the kind of bug that renders as "the
/// whole panel vanished".
fn push_unique(points: &mut Vec<Vec2>, p: Vec2) {
    if points.last().is_some_and(|last| last.distance_squared(p) < 1e-8) {
        return;
    }
    points.push(p);
}

/// Segments needed to flatten a quadratic within `tolerance`.
///
/// `B''(t) = 2(p0 - 2c + p1)` is constant, and the error of an `n`-segment
/// uniform approximation is bounded by `|B''| / (8n²)`. Solving for `n` gives
/// the closed form below — no recursion, no repeated distance tests.
fn quad_segments(p0: Vec2, c: Vec2, p1: Vec2, tolerance: f32) -> usize {
    let deviation = (p0 - 2.0 * c + p1).length();
    if deviation <= 1e-6 {
        return 1;
    }
    let n = (deviation / (4.0 * tolerance)).sqrt().ceil();
    (n as usize).clamp(1, MAX_SEGMENTS)
}

fn flatten_quad(p0: Vec2, c: Vec2, p1: Vec2, tolerance: f32, out: &mut Vec<Vec2>) {
    let n = quad_segments(p0, c, p1, tolerance);
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let mt = 1.0 - t;
        push_unique(out, mt * mt * p0 + 2.0 * mt * t * c + t * t * p1);
    }
}

/// Segments needed to flatten a cubic within `tolerance`.
///
/// `|B''(t)| <= 6 max(|p0-2c1+c2|, |c1-2c2+p1|)`, so the `n`-segment error is
/// bounded by `3M / (4n²)`.
fn cubic_segments(p0: Vec2, c1: Vec2, c2: Vec2, p1: Vec2, tolerance: f32) -> usize {
    let a = (p0 - 2.0 * c1 + c2).length();
    let b = (c1 - 2.0 * c2 + p1).length();
    let m = a.max(b);
    if m <= 1e-6 {
        return 1;
    }
    let n = ((3.0 * m) / (4.0 * tolerance)).sqrt().ceil();
    (n as usize).clamp(1, MAX_SEGMENTS)
}

fn flatten_cubic(p0: Vec2, c1: Vec2, c2: Vec2, p1: Vec2, tolerance: f32, out: &mut Vec<Vec2>) {
    let n = cubic_segments(p0, c1, c2, p1, tolerance);
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let mt = 1.0 - t;
        let point = mt * mt * mt * p0
            + 3.0 * mt * mt * t * c1
            + 3.0 * mt * t * t * c2
            + t * t * t * p1;
        push_unique(out, point);
    }
}

fn flatten_arc(
    centre: Vec2,
    radii: Vec2,
    start: f32,
    sweep: f32,
    tolerance: f32,
    out: &mut Vec<Vec2>,
) {
    let radius = radii.x.abs().max(radii.y.abs());
    if radius <= 1e-6 || sweep.abs() <= 1e-6 {
        return;
    }
    // The sagitta of a chord subtending angle `theta` on a circle of radius `r`
    // is `r(1 - cos(theta/2))`. Solving for `theta` at the tolerance gives the
    // largest step that stays inside it.
    let ratio = (1.0 - tolerance / radius).clamp(-1.0, 1.0);
    let step = 2.0 * ratio.acos();
    let n = if step <= 1e-4 {
        MAX_SEGMENTS
    } else {
        ((sweep.abs() / step).ceil() as usize).clamp(1, MAX_SEGMENTS)
    };
    for i in 1..=n {
        let angle = start + sweep * (i as f32 / n as f32);
        push_unique(
            out,
            centre + Vec2::new(radii.x * angle.cos(), radii.y * angle.sin()),
        );
    }
}

/// Triangles produced by tessellation: a flat list, three vertices per triangle.
pub type Triangles = Vec<Vec2>;

/// Expand a flattened contour into stroke triangles.
///
/// Emits, in order: one quad per segment, one join per interior joint, and caps
/// on an open contour. Nothing here is indexed — the shaped stream uploads a
/// plain vertex list, and an index buffer would save memory that a UI frame does
/// not spend.
#[must_use]
pub fn stroke_contour(contour: &Contour, stroke: &Stroke) -> Triangles {
    let half = (stroke.width * 0.5).max(1e-4);
    let mut out = Triangles::new();

    if let Some(dash) = stroke.dash {
        for piece in dash_contour(contour, dash) {
            stroke_polyline(&piece, half, stroke, &mut out, false);
        }
        return out;
    }
    stroke_polyline(contour, half, stroke, &mut out, contour.closed);
    out
}

fn stroke_polyline(
    contour: &Contour,
    half: f32,
    stroke: &Stroke,
    out: &mut Triangles,
    closed: bool,
) {
    let points = &contour.points;
    if points.len() < 2 {
        // A single point with a round or square cap is still a visible dot, and
        // dashing a path routinely produces one. Silently dropping it makes a
        // dotted line disappear at exactly the phase where every dash lands on
        // a vertex.
        if points.len() == 1 && stroke.cap != Cap::Butt {
            emit_cap(points[0], Vec2::X, half, stroke.cap, out);
            emit_cap(points[0], -Vec2::X, half, stroke.cap, out);
        }
        return;
    }

    let count = points.len();
    let last_segment = if closed { count } else { count - 1 };

    for i in 0..last_segment {
        let a = points[i];
        let b = points[(i + 1) % count];
        let Some(dir) = (b - a).try_normalize() else {
            continue;
        };
        let n = Vec2::new(-dir.y, dir.x) * half;
        quad(a - n, a + n, b + n, b - n, out);
    }

    let joints = if closed { count } else { count - 1 };
    for i in 1..joints + usize::from(closed) {
        let prev = points[(i + count - 1) % count];
        let at = points[i % count];
        let next = points[(i + 1) % count];
        emit_join(prev, at, next, half, stroke, out);
    }

    if !closed {
        if let Some(dir) = (points[1] - points[0]).try_normalize() {
            emit_cap(points[0], -dir, half, stroke.cap, out);
        }
        if let Some(dir) = (points[count - 1] - points[count - 2]).try_normalize() {
            emit_cap(points[count - 1], dir, half, stroke.cap, out);
        }
    }
}

fn emit_join(prev: Vec2, at: Vec2, next: Vec2, half: f32, stroke: &Stroke, out: &mut Triangles) {
    let (Some(d0), Some(d1)) = ((at - prev).try_normalize(), (next - at).try_normalize()) else {
        return;
    };
    let cross = d0.x * d1.y - d0.y * d1.x;
    if cross.abs() < 1e-6 {
        return; // collinear: the two segment quads already meet flush.
    }
    // Outer side of the turn. The inner side overlaps and needs nothing, which
    // is why this is a fan or a triangle rather than a full ring.
    let sign = if cross > 0.0 { -1.0 } else { 1.0 };
    let n0 = Vec2::new(-d0.y, d0.x) * half * sign;
    let n1 = Vec2::new(-d1.y, d1.x) * half * sign;

    match stroke.join {
        Join::Bevel => triangle(at, at + n0, at + n1, out),
        Join::Round => {
            let start = n0.to_angle();
            let mut sweep = n1.to_angle() - start;
            // Take the short way round; the long way is the inner side, which is
            // already covered by the two segment quads.
            while sweep > std::f32::consts::PI {
                sweep -= std::f32::consts::TAU;
            }
            while sweep < -std::f32::consts::PI {
                sweep += std::f32::consts::TAU;
            }
            let steps = ((sweep.abs() / 0.4).ceil() as usize).clamp(1, 32);
            let mut previous = at + n0;
            for i in 1..=steps {
                let angle = start + sweep * (i as f32 / steps as f32);
                let point = at + Vec2::new(angle.cos(), angle.sin()) * half;
                triangle(at, previous, point, out);
                previous = point;
            }
        }
        Join::Miter => {
            let bisector = (n0 + n1).try_normalize();
            let Some(bisector) = bisector else {
                triangle(at, at + n0, at + n1, out);
                return;
            };
            let cos_half = bisector.dot(n0.normalize());
            // `1/cos` blows up as the turn approaches a full reversal, which is
            // exactly the spike the miter limit exists to prevent.
            let length = if cos_half.abs() < 1e-4 {
                f32::INFINITY
            } else {
                half / cos_half
            };
            if length / half > stroke.miter_limit {
                triangle(at, at + n0, at + n1, out);
                return;
            }
            let tip = at + bisector * length;
            triangle(at, at + n0, tip, out);
            triangle(at, tip, at + n1, out);
        }
    }
}

fn emit_cap(at: Vec2, outward: Vec2, half: f32, cap: Cap, out: &mut Triangles) {
    let n = Vec2::new(-outward.y, outward.x) * half;
    match cap {
        Cap::Butt => {}
        Cap::Square => {
            let tip = outward * half;
            quad(at - n, at + n, at + n + tip, at - n + tip, out);
        }
        Cap::Round => {
            let start = (-n).to_angle();
            let steps = 12;
            let mut previous = at - n;
            for i in 1..=steps {
                let angle = start + std::f32::consts::PI * (i as f32 / steps as f32);
                let point = at + Vec2::new(angle.cos(), angle.sin()) * half;
                triangle(at, previous, point, out);
                previous = point;
            }
        }
    }
}

/// Split a contour into the drawn pieces of a dash pattern.
fn dash_contour(contour: &Contour, dash: Dash) -> Vec<Contour> {
    let on = dash.on.max(1e-3);
    let off = dash.off.max(0.0);
    let period = on + off;
    if off <= 1e-4 {
        return vec![contour.clone()];
    }

    let mut pieces = Vec::new();
    let mut current: Vec<Vec2> = Vec::new();
    // Position within the pattern; `< on` means the pen is down.
    let mut phase = dash.phase.rem_euclid(period);
    let mut down = phase < on;
    if down {
        current.push(contour.points[0]);
    }

    let count = contour.points.len();
    let segments = if contour.closed { count } else { count - 1 };
    for i in 0..segments {
        let a = contour.points[i];
        let b = contour.points[(i + 1) % count];
        let length = a.distance(b);
        if length <= 1e-6 {
            continue;
        }
        let dir = (b - a) / length;
        let mut travelled = 0.0;
        while travelled < length {
            let remaining_in_state = if down { on - phase } else { period - phase };
            let step = remaining_in_state.min(length - travelled);
            travelled += step;
            phase += step;
            let point = a + dir * travelled;
            if down {
                current.push(point);
            }
            if phase >= period - 1e-6 {
                phase = 0.0;
                down = true;
                current.clear();
                current.push(point);
            } else if down && phase >= on - 1e-6 {
                down = false;
                if current.len() >= 2 {
                    pieces.push(Contour {
                        points: std::mem::take(&mut current),
                        closed: false,
                    });
                } else {
                    current.clear();
                }
            }
        }
    }
    if current.len() >= 2 {
        pieces.push(Contour {
            points: current,
            closed: false,
        });
    }
    pieces
}

/// Triangulate a closed contour by ear clipping.
///
/// O(n²), which is the right complexity here: the contours this fills are
/// rounded rectangles, pie slices, arrowheads and node bodies — tens of points,
/// not thousands. A monotone-partition triangulator is asymptotically better and
/// is several hundred lines that would run on inputs where the difference is
/// unmeasurable.
///
/// Returns an empty list for a self-intersecting contour rather than producing
/// overlapping triangles: a fill that silently renders wrong is harder to
/// diagnose than one that renders nothing.
#[must_use]
pub fn fill_contour(contour: &Contour) -> Triangles {
    let mut points = contour.points.clone();
    if points.len() < 3 {
        return Triangles::new();
    }
    // Ear clipping does not detect self-intersection on its own: given a bow
    // tie it finds ears and emits overlapping triangles. The check is O(n²)
    // over contours of tens of points, which is unmeasurable here and is the
    // only way the contract above ("draws nothing rather than something wrong")
    // is actually kept.
    if self_intersects(&points) {
        return Triangles::new();
    }
    // Ear clipping assumes counter-clockwise winding.
    if signed_area(&points) < 0.0 {
        points.reverse();
    }

    let mut out = Triangles::new();
    let mut indices: Vec<usize> = (0..points.len()).collect();
    // Each successful clip removes one vertex; the guard bounds the failure
    // case, where no ear is found because the contour self-intersects.
    let mut guard = points.len() * points.len();

    while indices.len() > 3 {
        guard -= 1;
        if guard == 0 {
            return Triangles::new();
        }
        let mut clipped = false;
        for i in 0..indices.len() {
            let prev = indices[(i + indices.len() - 1) % indices.len()];
            let at = indices[i];
            let next = indices[(i + 1) % indices.len()];
            let (a, b, c) = (points[prev], points[at], points[next]);
            if cross(b - a, c - b) <= 0.0 {
                continue; // reflex
            }
            if indices
                .iter()
                .any(|&j| j != prev && j != at && j != next && point_in_triangle(points[j], a, b, c))
            {
                continue;
            }
            triangle(a, b, c, &mut out);
            indices.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            return Triangles::new();
        }
    }
    triangle(points[indices[0]], points[indices[1]], points[indices[2]], &mut out);
    out
}

/// Whether any two non-adjacent edges of the closed contour cross.
fn self_intersects(points: &[Vec2]) -> bool {
    let n = points.len();
    for i in 0..n {
        let (a0, a1) = (points[i], points[(i + 1) % n]);
        // Skip `j == i` and the two edges sharing a vertex with `i`: they touch
        // by construction, and treating a shared endpoint as a crossing would
        // reject every polygon.
        for j in (i + 2)..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            let (b0, b1) = (points[j], points[(j + 1) % n]);
            if segments_cross(a0, a1, b0, b1) {
                return true;
            }
        }
    }
    false
}

/// Proper crossing only: shared endpoints and collinear touching do not count.
fn segments_cross(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> bool {
    let d1 = cross(a1 - a0, b0 - a0);
    let d2 = cross(a1 - a0, b1 - a0);
    let d3 = cross(b1 - b0, a0 - b0);
    let d4 = cross(b1 - b0, a1 - b0);
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

fn signed_area(points: &[Vec2]) -> f32 {
    let mut area = 0.0;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        area += a.x * b.y - b.x * a.y;
    }
    area * 0.5
}

fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

fn point_in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let d1 = cross(b - a, p - a);
    let d2 = cross(c - b, p - b);
    let d3 = cross(a - c, p - c);
    (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
}

fn triangle(a: Vec2, b: Vec2, c: Vec2, out: &mut Triangles) {
    out.push(a);
    out.push(b);
    out.push(c);
}

fn quad(a: Vec2, b: Vec2, c: Vec2, d: Vec2, out: &mut Triangles) {
    triangle(a, b, c, out);
    triangle(a, c, d, out);
}

/// The axis-aligned bounds of a triangle list, for UV mapping and culling.
#[must_use]
pub fn bounds(triangles: &[Vec2]) -> crate::types::Rect {
    let Some(first) = triangles.first() else {
        return crate::types::Rect::new(0.0, 0.0, 0.0, 0.0);
    };
    let (mut min, mut max) = (*first, *first);
    for p in triangles {
        min = min.min(*p);
        max = max.max(*p);
    }
    crate::types::Rect::new(min.x, min.y, max.x - min.x, max.y - min.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(triangles: &[Vec2]) -> f32 {
        triangles
            .chunks_exact(3)
            .map(|t| cross(t[1] - t[0], t[2] - t[0]).abs() * 0.5)
            .sum()
    }

    #[test]
    fn a_straight_line_flattens_to_two_points() {
        let path = Path::polyline(&[Vec2::ZERO, Vec2::new(10.0, 0.0)]);
        let contours = path.flatten(DEFAULT_TOLERANCE);
        assert_eq!(contours.len(), 1);
        assert_eq!(contours[0].points.len(), 2);
        assert!(!contours[0].closed);
    }

    /// A curve flattens finely enough that no chord misses it by more than the
    /// tolerance — checked against the true curve, not against a segment count.
    #[test]
    fn a_cubic_stays_within_its_tolerance() {
        let (p0, c1, c2, p1) = (
            Vec2::ZERO,
            Vec2::new(0.0, 100.0),
            Vec2::new(100.0, 100.0),
            Vec2::new(100.0, 0.0),
        );
        let mut path = Path::new();
        path.move_to(p0).cubic_to(c1, c2, p1);
        let points = &path.flatten(0.25).remove(0).points;

        for i in 0..64 {
            let t = i as f32 / 63.0;
            let mt = 1.0 - t;
            let truth = mt * mt * mt * p0
                + 3.0 * mt * mt * t * c1
                + 3.0 * mt * t * t * c2
                + t * t * t * p1;
            let nearest = points
                .windows(2)
                .map(|w| distance_to_segment(truth, w[0], w[1]))
                .fold(f32::INFINITY, f32::min);
            assert!(nearest <= 0.26, "t={t}: off by {nearest}");
        }
    }

    fn distance_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
        let ab = b - a;
        let len_sq = ab.length_squared();
        if len_sq < 1e-9 {
            return p.distance(a);
        }
        let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
        p.distance(a + ab * t)
    }

    /// A tighter tolerance produces more segments; a looser one, fewer.
    ///
    /// The failure this catches is a tolerance argument that is accepted and
    /// then ignored, which looks correct at the default and wastes an order of
    /// magnitude of vertices at high DPI.
    #[test]
    fn tolerance_actually_changes_the_segment_count() {
        let mut path = Path::new();
        path.move_to(Vec2::ZERO)
            .cubic_to(Vec2::new(0.0, 100.0), Vec2::new(100.0, 100.0), Vec2::new(100.0, 0.0));
        let coarse = path.flatten(4.0).remove(0).points.len();
        let fine = path.flatten(0.1).remove(0).points.len();
        assert!(fine > coarse * 4, "coarse {coarse}, fine {fine}");
    }

    #[test]
    fn a_degenerate_curve_does_not_explode() {
        // Every control point identical: zero deviation, one segment, and no
        // division by a zero norm anywhere.
        let mut path = Path::new();
        path.move_to(Vec2::ZERO)
            .cubic_to(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO);
        let contours = path.flatten(DEFAULT_TOLERANCE);
        assert!(contours.is_empty(), "a zero-length path draws nothing");
    }

    #[test]
    fn a_closed_contour_does_not_repeat_its_first_point() {
        let mut path = Path::new();
        path.move_to(Vec2::ZERO)
            .line_to(Vec2::new(10.0, 0.0))
            .line_to(Vec2::new(10.0, 10.0))
            .line_to(Vec2::ZERO)
            .close();
        let contour = path.flatten(DEFAULT_TOLERANCE).remove(0);
        assert!(contour.closed);
        assert_eq!(
            contour.points.len(),
            3,
            "the wrap is implied; repeating the first point emits a degenerate joint"
        );
    }

    #[test]
    fn a_circle_closes_and_has_about_the_right_area() {
        let path = Path::circle(Vec2::new(50.0, 50.0), 20.0);
        let contour = path.flatten(0.1).remove(0);
        assert!(contour.closed);
        let filled = fill_contour(&contour);
        let expected = std::f32::consts::PI * 400.0;
        let got = area(&filled);
        assert!(
            (got - expected).abs() / expected < 0.01,
            "circle area {got}, expected about {expected}"
        );
    }

    #[test]
    fn a_stroked_segment_covers_length_times_width() {
        let contour = Contour {
            points: vec![Vec2::ZERO, Vec2::new(100.0, 0.0)],
            closed: false,
        };
        let triangles = stroke_contour(&contour, &Stroke::new(4.0));
        assert_eq!(area(&triangles), 400.0);
    }

    /// The miter limit turns a spike into a bevel.
    ///
    /// Without it, a nearly-doubled-back polyline grows a miter tip whose length
    /// goes to infinity as the turn approaches 180 degrees — which renders as a
    /// line shooting off the screen from a wire that merely folded back.
    #[test]
    fn the_miter_limit_stops_the_spike() {
        let contour = Contour {
            points: vec![
                Vec2::ZERO,
                Vec2::new(100.0, 0.0),
                Vec2::new(0.0, 0.5), // an almost complete reversal
            ],
            closed: false,
        };
        let mitred = stroke_contour(&contour, &Stroke::new(4.0));
        let far = mitred
            .iter()
            .map(|p| p.length())
            .fold(0.0f32, f32::max);
        assert!(
            far < 400.0,
            "miter tip reached {far}, which is the spike the limit exists to prevent"
        );
    }

    #[test]
    fn joins_and_caps_each_add_geometry() {
        let corner = Contour {
            points: vec![Vec2::ZERO, Vec2::new(50.0, 0.0), Vec2::new(50.0, 50.0)],
            closed: false,
        };
        let bevel = stroke_contour(&corner, &Stroke::new(6.0).with_join(Join::Bevel));
        let round = stroke_contour(&corner, &Stroke::new(6.0).with_join(Join::Round));
        assert!(round.len() > bevel.len(), "a round join is a fan, not a triangle");

        let butt = stroke_contour(&corner, &Stroke::new(6.0));
        let square = stroke_contour(&corner, &Stroke::new(6.0).with_cap(Cap::Square));
        assert!(area(&square) > area(&butt), "a square cap extends past the end");
    }

    #[test]
    fn a_closed_stroke_has_no_caps_and_every_joint_joined() {
        let square = Contour {
            points: vec![
                Vec2::ZERO,
                Vec2::new(20.0, 0.0),
                Vec2::new(20.0, 20.0),
                Vec2::new(0.0, 20.0),
            ],
            closed: true,
        };
        let closed = stroke_contour(&square, &Stroke::new(2.0).with_join(Join::Bevel));
        let mut open_points = square.points.clone();
        open_points.push(Vec2::ZERO);
        let open = stroke_contour(
            &Contour {
                points: open_points,
                closed: false,
            },
            &Stroke::new(2.0).with_join(Join::Bevel),
        );
        // The closed form is legitimately *larger*: four segments and four
        // joins, where the open form walking the same corners has four
        // segments and only three joins, because its ends are ends rather than
        // a corner. Butt caps add nothing. Asserting the counts rather than an
        // inequality is what makes that difference visible instead of looking
        // like a missing join.
        assert_eq!(closed.len() / 3, 4 * 2 + 4, "4 segment quads + 4 bevels");
        assert_eq!(open.len() / 3, 4 * 2 + 3, "same segments, one join fewer, no caps");
    }

    #[test]
    fn dashes_draw_less_than_a_solid_line() {
        let contour = Contour {
            points: vec![Vec2::ZERO, Vec2::new(100.0, 0.0)],
            closed: false,
        };
        let solid = stroke_contour(&contour, &Stroke::new(2.0));
        let dashed = stroke_contour(&contour, &Stroke::new(2.0).with_dash(4.0, 4.0, 0.0));
        let ratio = area(&dashed) / area(&solid);
        assert!(
            (0.4..0.6).contains(&ratio),
            "a 4-on 4-off dash should cover about half; covered {ratio}"
        );
    }

    /// Animating the phase moves the dashes without changing how much is drawn.
    #[test]
    fn the_dash_phase_shifts_the_pattern() {
        let contour = Contour {
            points: vec![Vec2::ZERO, Vec2::new(100.0, 0.0)],
            closed: false,
        };
        let a = stroke_contour(&contour, &Stroke::new(2.0).with_dash(4.0, 4.0, 0.0));
        let b = stroke_contour(&contour, &Stroke::new(2.0).with_dash(4.0, 4.0, 4.0));
        assert_ne!(a, b, "the phase must move the pattern");
        assert!((area(&a) - area(&b)).abs() < 12.0, "and not change the ink much");
    }

    #[test]
    fn a_self_intersecting_fill_draws_nothing_rather_than_something_wrong() {
        // A bow tie: ear clipping cannot triangulate it, and overlapping
        // triangles would be harder to diagnose than an empty fill.
        let contour = Contour {
            points: vec![
                Vec2::ZERO,
                Vec2::new(10.0, 10.0),
                Vec2::new(10.0, 0.0),
                Vec2::new(0.0, 10.0),
            ],
            closed: true,
        };
        assert!(fill_contour(&contour).is_empty());
    }

    #[test]
    fn fill_handles_both_windings() {
        let ccw = Contour {
            points: vec![Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)],
            closed: true,
        };
        let cw = Contour {
            points: ccw.points.iter().rev().copied().collect(),
            closed: true,
        };
        assert_eq!(area(&fill_contour(&ccw)), 50.0);
        assert_eq!(area(&fill_contour(&cw)), 50.0);
    }

    /// A concave polygon is filled by its own area, not its convex hull's.
    #[test]
    fn a_concave_fill_is_not_its_convex_hull() {
        // An L, area 300 out of a 20x20 hull.
        let contour = Contour {
            points: vec![
                Vec2::ZERO,
                Vec2::new(20.0, 0.0),
                Vec2::new(20.0, 10.0),
                Vec2::new(10.0, 10.0),
                Vec2::new(10.0, 20.0),
                Vec2::new(0.0, 20.0),
            ],
            closed: true,
        };
        assert!((area(&fill_contour(&contour)) - 300.0).abs() < 0.01);
    }

    #[test]
    fn a_wire_bows_between_two_ports() {
        let path = Path::wire(Vec2::ZERO, Vec2::new(200.0, 100.0));
        let points = &path.flatten(0.5).remove(0).points;
        assert!(points.len() > 8, "a wire is a curve, not a line");
        assert_eq!(points[0], Vec2::ZERO);
        assert!(points.last().unwrap().abs_diff_eq(Vec2::new(200.0, 100.0), 1e-3));
    }

    /// Two stacked ports still get a visible curve rather than a straight line.
    #[test]
    fn a_wire_between_stacked_ports_still_bows() {
        let path = Path::wire(Vec2::ZERO, Vec2::new(0.0, 40.0));
        let points = path.flatten(0.5).remove(0).points;
        let widest = points.iter().map(|p| p.x.abs()).fold(0.0f32, f32::max);
        // With a 24 px handle floor the S peaks at about 6.75 px of bow either
        // side — slight, deliberate, and the same look Blueprint and Godot give
        // two stacked ports. The number to defend is "not zero", not a round
        // figure: a straight line here would hide which way the edge runs.
        assert!(
            widest > 5.0,
            "a vertical wire must not collapse to a line; bowed only {widest}"
        );
    }

    #[test]
    fn bounds_covers_every_vertex() {
        let triangles = vec![
            Vec2::new(-5.0, 2.0),
            Vec2::new(10.0, -3.0),
            Vec2::new(4.0, 8.0),
        ];
        let r = bounds(&triangles);
        assert_eq!((r.x, r.y, r.w, r.h), (-5.0, -3.0, 15.0, 11.0));
    }
}
