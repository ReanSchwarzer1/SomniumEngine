//! Phase CONTROL-K: authored curves and colour gradients as reflected values.
//!
//! # Why these are value types and not widgets
//!
//! A curve is the answer to "how does this number change as that number
//! moves". Every engine surveyed in phase CONTROL §6 grew one, and every one
//! of them made the same first mistake: the curve lived in the editor and the
//! runtime got a baked array. That splits the description in two, which is the
//! defect `reflect_registry.rs` exists to prevent.
//!
//! So a [`Curve`] is a [`ReflectValue`](crate::reflect::ReflectValue) like a
//! float is. It serialises through `scene_schema`, it is addressed by
//! `(StableId, FieldId)` like any other field, it undoes through the one
//! generic `SetFieldCmd`, and a component declares one by naming it in
//! `component_schema!`. The editor's curve editor is then just the property
//! editor registered for [`FieldType::Curve`](crate::reflect::FieldType::Curve),
//! exactly as the numeric field is the editor registered for `F64`.
//!
//! # Evaluation is CPU-side and sampling is the GPU story
//!
//! Nothing here touches the GPU. A consumer that needs a curve in a shader
//! calls [`Curve::sample_into`] and uploads the resulting table; that is what
//! makes an edit live with no refresh button (phase CONTROL §5.3 names Ultra
//! Dynamic Sky's "Refresh Settings" as the anti-pattern). Re-sampling 64 floats
//! per edited frame is free, and it means the shader never learns what a
//! keyframe is.

use std::fmt;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Interpolation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A named starting shape, and the constructor that builds it.
///
/// Spelled out as a type so [`Curve::PRESETS`] and [`Gradient`]'s eventual
/// equivalent read as a table rather than as a signature.
pub type NamedCurve = (&'static str, fn() -> Curve);

/// How the segment *leaving* a key reaches the next one.
///
/// Stored per key rather than per curve because the useful shapes are mixed:
/// a light that ramps up smoothly and then cuts out is one `Smooth` key
/// followed by one `Step` key, and a curve-wide mode cannot express it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Interpolation {
    /// Hold this key's value until the next key.
    Step,
    /// Straight line to the next key.
    #[default]
    Linear,
    /// Cubic Hermite using this key's `out_tangent` and the next key's
    /// `in_tangent`.
    Smooth,
}

impl Interpolation {
    /// Durable name, written to scene files.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Linear => "linear",
            Self::Smooth => "smooth",
        }
    }

    /// Parse a durable name. Unknown text reads as [`Self::Linear`], which is
    /// the shape a file from a newer build degrades to most safely.
    #[must_use]
    pub fn from_str_or_linear(text: &str) -> Self {
        match text {
            "step" => Self::Step,
            "smooth" => Self::Smooth,
            _ => Self::Linear,
        }
    }

    /// The next mode in the cycle the editor's right-click uses.
    #[must_use]
    pub const fn cycled(self) -> Self {
        match self {
            Self::Linear => Self::Smooth,
            Self::Smooth => Self::Step,
            Self::Step => Self::Linear,
        }
    }
}

impl fmt::Display for Interpolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Curve
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// One authored control point.
///
/// Tangents are in value-units per time-unit — the slope of the curve at the
/// key — which is the convention Unity, Unreal and Fyrox all use and the one
/// that makes `Smooth` a plain Hermite evaluation with no rescaling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveKey {
    /// Position along the curve's domain.
    pub t: f32,
    /// Value at `t`.
    pub v: f32,
    /// Slope arriving at this key, used by the preceding `Smooth` segment.
    pub in_tangent: f32,
    /// Slope leaving this key, used by this key's `Smooth` segment.
    pub out_tangent: f32,
    /// Shape of the segment leaving this key.
    pub interpolation: Interpolation,
}

impl CurveKey {
    /// A linear key with flat tangents.
    #[must_use]
    pub const fn new(t: f32, v: f32) -> Self {
        Self {
            t,
            v,
            in_tangent: 0.0,
            out_tangent: 0.0,
            interpolation: Interpolation::Linear,
        }
    }

    /// A smooth key with flat tangents — the "ease" shape.
    #[must_use]
    pub const fn smooth(t: f32, v: f32) -> Self {
        Self {
            interpolation: Interpolation::Smooth,
            ..Self::new(t, v)
        }
    }
}

/// A one-dimensional authored function of time.
///
/// Keys are kept sorted by `t`. Outside the key range the curve is *clamped*
/// to the end values rather than extrapolated: an authored 24-hour track that
/// is asked for hour 25 must return midnight's value, not a slope run off the
/// end of the world.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Curve {
    keys: Vec<CurveKey>,
}

impl Curve {
    /// An empty curve. Evaluates to `0.0` everywhere.
    #[must_use]
    pub const fn empty() -> Self {
        Self { keys: Vec::new() }
    }

    /// A flat curve holding `v` across the whole domain.
    #[must_use]
    pub fn constant(v: f32) -> Self {
        Self::from_keys(vec![CurveKey::new(0.0, v), CurveKey::new(1.0, v)])
    }

    /// A straight ramp from `(0, from)` to `(1, to)`.
    #[must_use]
    pub fn ramp(from: f32, to: f32) -> Self {
        Self::from_keys(vec![CurveKey::new(0.0, from), CurveKey::new(1.0, to)])
    }

    /// The named starting shapes the editor offers.
    ///
    /// Presets are constructors rather than authored assets on purpose: these
    /// five are the shapes every tool ships, they are two keys each, and a
    /// preset *library* is a content problem that would need a file format,
    /// a browser and a naming rule to be worth having.
    pub const PRESETS: [NamedCurve; 5] = [
        ("Linear", || Self::ramp(0.0, 1.0)),
        ("Ease In", || {
            Self::from_keys(vec![
                CurveKey {
                    interpolation: Interpolation::Smooth,
                    out_tangent: 0.0,
                    ..CurveKey::new(0.0, 0.0)
                },
                CurveKey {
                    in_tangent: 2.0,
                    ..CurveKey::new(1.0, 1.0)
                },
            ])
        }),
        ("Ease Out", || {
            Self::from_keys(vec![
                CurveKey {
                    interpolation: Interpolation::Smooth,
                    out_tangent: 2.0,
                    ..CurveKey::new(0.0, 0.0)
                },
                CurveKey::new(1.0, 1.0),
            ])
        }),
        ("Ease In-Out", || {
            Self::from_keys(vec![CurveKey::smooth(0.0, 0.0), CurveKey::smooth(1.0, 1.0)])
        }),
        ("Constant", || Self::constant(1.0)),
    ];

    /// Build from keys, sorting and sanitising them.
    #[must_use]
    pub fn from_keys(keys: Vec<CurveKey>) -> Self {
        let mut curve = Self { keys };
        curve.sanitize();
        curve
    }

    /// The authored keys, ordered by `t`.
    #[must_use]
    pub fn keys(&self) -> &[CurveKey] {
        &self.keys
    }

    /// Number of keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the curve has no keys at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Insert a key and return its index after sorting.
    ///
    /// The returned index is what a UI needs in order to keep a freshly added
    /// key selected: the insert may reorder, and re-finding the key by value
    /// afterwards is how selection drifts onto its neighbour.
    pub fn insert(&mut self, key: CurveKey) -> usize {
        self.keys.push(key);
        self.sanitize();
        // Exact comparison, deliberately: this is re-finding the value that
        // was just written, not comparing two computed floats. A tolerance
        // here would match a *neighbouring* key and silently move the
        // selection onto it.
        #[allow(clippy::float_cmp)]
        self.keys
            .iter()
            .position(|k| k.t == key.t && k.v == key.v)
            .unwrap_or(0)
    }

    /// Remove the key at `index`, if it exists.
    ///
    /// A curve is allowed to become empty; the editor refuses the last
    /// deletion at its own layer, because "no keys" is a legitimate authored
    /// state for a track that is deliberately absent.
    pub fn remove(&mut self, index: usize) -> Option<CurveKey> {
        (index < self.keys.len()).then(|| self.keys.remove(index))
    }

    /// Move a key, re-sorting, and return its index afterwards.
    pub fn move_key(&mut self, index: usize, t: f32, v: f32) -> usize {
        if index >= self.keys.len() {
            return index;
        }
        let mut key = self.keys[index];
        key.t = t;
        key.v = v;
        self.keys[index] = key;
        self.sanitize();
        #[allow(clippy::float_cmp)]
        self.keys
            .iter()
            .position(|k| *k == key)
            .unwrap_or(index.min(self.keys.len().saturating_sub(1)))
    }

    /// Mutable access to a key, for tangent and interpolation edits that
    /// cannot reorder.
    pub fn key_mut(&mut self, index: usize) -> Option<&mut CurveKey> {
        self.keys.get_mut(index)
    }

    /// Sort by time and drop non-finite keys.
    ///
    /// Called after every mutation rather than trusting callers. A NaN key
    /// makes the sort itself ill-defined, so it is removed before ordering,
    /// not after.
    pub fn sanitize(&mut self) {
        self.keys.retain(|k| {
            k.t.is_finite()
                && k.v.is_finite()
                && k.in_tangent.is_finite()
                && k.out_tangent.is_finite()
        });
        self.keys
            .sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    }

    /// The `[first, last]` key times, or `None` when empty.
    #[must_use]
    pub fn domain(&self) -> Option<(f32, f32)> {
        Some((self.keys.first()?.t, self.keys.last()?.t))
    }

    /// Evaluate at `t`, clamped to the authored domain.
    #[must_use]
    pub fn evaluate(&self, t: f32) -> f32 {
        match self.keys.len() {
            0 => 0.0,
            1 => self.keys[0].v,
            _ => {
                let first = &self.keys[0];
                let last = &self.keys[self.keys.len() - 1];
                if t <= first.t {
                    return first.v;
                }
                if t >= last.t {
                    return last.v;
                }
                // Binary search for the segment. `partition_point` is the
                // index of the first key strictly after `t`, so `i - 1` is
                // the segment's left key and the range check above guarantees
                // `i` is in `1..len`.
                let index = self.keys.partition_point(|key| key.t <= t);
                let left = &self.keys[index - 1];
                let right = &self.keys[index];
                let span = right.t - left.t;
                if span <= f32::EPSILON {
                    return right.v;
                }
                let along = (t - left.t) / span;
                match left.interpolation {
                    Interpolation::Step => left.v,
                    Interpolation::Linear => left.v + (right.v - left.v) * along,
                    Interpolation::Smooth => hermite(
                        left.v,
                        right.v,
                        left.out_tangent,
                        right.in_tangent,
                        span,
                        along,
                    ),
                }
            }
        }
    }

    /// Fill `out` with `out.len()` uniform samples across `[t0, t1]`.
    ///
    /// This is the only route a curve takes to a shader. The caller owns the
    /// table's size, because the right size is a property of the consumer —
    /// a day track wants 64, a colour response wants 256.
    pub fn sample_into(&self, t0: f32, t1: f32, out: &mut [f32]) {
        let n = out.len();
        if n == 0 {
            return;
        }
        if n == 1 {
            out[0] = self.evaluate(t0);
            return;
        }
        #[allow(clippy::cast_precision_loss)]
        let denom = (n - 1) as f32;
        for (i, slot) in out.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let u = i as f32 / denom;
            *slot = self.evaluate(t0 + (t1 - t0) * u);
        }
    }

    /// Convenience wrapper returning a freshly allocated table.
    #[must_use]
    pub fn sample(&self, t0: f32, t1: f32, count: usize) -> Vec<f32> {
        let mut out = vec![0.0; count];
        self.sample_into(t0, t1, &mut out);
        out
    }

    /// Flatten to `[t, v, in_tangent, out_tangent, interpolation]` quintuples.
    ///
    /// The serializer's wire shape. A flat float array rather than a nested
    /// object because a curve with forty keys should not cost forty JSON
    /// objects in a scene file, and because the reader can validate the length
    /// in one comparison.
    #[must_use]
    pub fn to_flat(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.keys.len() * 5);
        for key in &self.keys {
            out.push(key.t);
            out.push(key.v);
            out.push(key.in_tangent);
            out.push(key.out_tangent);
            out.push(match key.interpolation {
                Interpolation::Step => 0.0,
                Interpolation::Linear => 1.0,
                Interpolation::Smooth => 2.0,
            });
        }
        out
    }

    /// Rebuild from [`Self::to_flat`]. A trailing partial quintuple is dropped.
    #[must_use]
    pub fn from_flat(flat: &[f32]) -> Self {
        let mut keys = Vec::with_capacity(flat.len() / 5);
        for chunk in flat.chunks_exact(5) {
            keys.push(CurveKey {
                t: chunk[0],
                v: chunk[1],
                in_tangent: chunk[2],
                out_tangent: chunk[3],
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                interpolation: match chunk[4].round() as i32 {
                    0 => Interpolation::Step,
                    2 => Interpolation::Smooth,
                    _ => Interpolation::Linear,
                },
            });
        }
        Self::from_keys(keys)
    }

    /// Whether every stored float is finite.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.keys.iter().all(|k| {
            k.t.is_finite()
                && k.v.is_finite()
                && k.in_tangent.is_finite()
                && k.out_tangent.is_finite()
        })
    }
}

/// Cubic Hermite over a segment of width `span`, with slopes in value-per-time.
fn hermite(v0: f32, v1: f32, m0: f32, m1: f32, span: f32, u: f32) -> f32 {
    let u2 = u * u;
    let u3 = u2 * u;
    let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
    let h10 = u3 - 2.0 * u2 + u;
    let h01 = -2.0 * u3 + 3.0 * u2;
    let h11 = u3 - u2;
    h00 * v0 + h10 * span * m0 + h01 * v1 + h11 * span * m1
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Gradient
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// One colour stop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    /// Position in `0..=1`.
    pub t: f32,
    /// Linear RGBA. Linear, not sRGB: everything downstream of the editor is
    /// linear, and a gradient that stored display values would be the one
    /// place the convention broke.
    pub color: [f32; 4],
}

impl GradientStop {
    /// A stop at `t` with linear RGBA `color`.
    #[must_use]
    pub const fn new(t: f32, color: [f32; 4]) -> Self {
        Self { t, color }
    }
}

/// A colour ramp over `0..=1`, interpolated linearly in linear RGB.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Gradient {
    stops: Vec<GradientStop>,
}

impl Gradient {
    /// An empty gradient. Evaluates to opaque black.
    #[must_use]
    pub const fn empty() -> Self {
        Self { stops: Vec::new() }
    }

    /// A two-stop ramp.
    #[must_use]
    pub fn ramp(from: [f32; 4], to: [f32; 4]) -> Self {
        Self::from_stops(vec![
            GradientStop::new(0.0, from),
            GradientStop::new(1.0, to),
        ])
    }

    /// Build from stops, sorting and sanitising them.
    #[must_use]
    pub fn from_stops(stops: Vec<GradientStop>) -> Self {
        let mut gradient = Self { stops };
        gradient.sanitize();
        gradient
    }

    /// The authored stops, ordered by `t`.
    #[must_use]
    pub fn stops(&self) -> &[GradientStop] {
        &self.stops
    }

    /// Number of stops.
    #[must_use]
    pub fn len(&self) -> usize {
        self.stops.len()
    }

    /// Whether the gradient has no stops at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stops.is_empty()
    }

    /// Insert a stop and return its index after sorting.
    pub fn insert(&mut self, stop: GradientStop) -> usize {
        self.stops.push(stop);
        self.sanitize();
        // Exact, for the same reason as `Curve::insert`.
        #[allow(clippy::float_cmp)]
        self.stops
            .iter()
            .position(|s| s.t == stop.t && s.color == stop.color)
            .unwrap_or(0)
    }

    /// Remove the stop at `index`, if it exists.
    pub fn remove(&mut self, index: usize) -> Option<GradientStop> {
        (index < self.stops.len()).then(|| self.stops.remove(index))
    }

    /// Mutable access to a stop.
    pub fn stop_mut(&mut self, index: usize) -> Option<&mut GradientStop> {
        self.stops.get_mut(index)
    }

    /// Move a stop along the ramp and return its index afterwards.
    pub fn move_stop(&mut self, index: usize, t: f32) -> usize {
        if index >= self.stops.len() {
            return index;
        }
        let stop = GradientStop {
            t,
            color: self.stops[index].color,
        };
        self.stops[index] = stop;
        self.sanitize();
        #[allow(clippy::float_cmp)]
        self.stops
            .iter()
            .position(|s| *s == stop)
            .unwrap_or(index.min(self.stops.len().saturating_sub(1)))
    }

    /// Sort by position and drop non-finite stops.
    pub fn sanitize(&mut self) {
        self.stops
            .retain(|s| s.t.is_finite() && s.color.iter().all(|c| c.is_finite()));
        for stop in &mut self.stops {
            stop.t = stop.t.clamp(0.0, 1.0);
        }
        self.stops
            .sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    }

    /// Evaluate at `t`, clamped to the end stops.
    #[must_use]
    pub fn evaluate(&self, t: f32) -> [f32; 4] {
        match self.stops.len() {
            0 => [0.0, 0.0, 0.0, 1.0],
            1 => self.stops[0].color,
            _ => {
                let first = &self.stops[0];
                let last = &self.stops[self.stops.len() - 1];
                if t <= first.t {
                    return first.color;
                }
                if t >= last.t {
                    return last.color;
                }
                let index = self.stops.partition_point(|stop| stop.t <= t);
                let left = &self.stops[index - 1];
                let right = &self.stops[index];
                let span = right.t - left.t;
                if span <= f32::EPSILON {
                    return right.color;
                }
                let along = (t - left.t) / span;
                let mut out = [0.0_f32; 4];
                for (slot, (lo, hi)) in out
                    .iter_mut()
                    .zip(left.color.iter().zip(right.color.iter()))
                {
                    *slot = lo + (hi - lo) * along;
                }
                out
            }
        }
    }

    /// Fill `out` with `out.len()` uniform RGBA samples across `0..=1`.
    pub fn sample_into(&self, out: &mut [[f32; 4]]) {
        let n = out.len();
        if n == 0 {
            return;
        }
        if n == 1 {
            out[0] = self.evaluate(0.0);
            return;
        }
        #[allow(clippy::cast_precision_loss)]
        let denom = (n - 1) as f32;
        for (i, slot) in out.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let u = i as f32 / denom;
            *slot = self.evaluate(u);
        }
    }

    /// Flatten to `[t, r, g, b, a]` quintuples.
    #[must_use]
    pub fn to_flat(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.stops.len() * 5);
        for stop in &self.stops {
            out.push(stop.t);
            out.extend_from_slice(&stop.color);
        }
        out
    }

    /// Rebuild from [`Self::to_flat`].
    #[must_use]
    pub fn from_flat(flat: &[f32]) -> Self {
        let mut stops = Vec::with_capacity(flat.len() / 5);
        for chunk in flat.chunks_exact(5) {
            stops.push(GradientStop {
                t: chunk[0],
                color: [chunk[1], chunk[2], chunk[3], chunk[4]],
            });
        }
        Self::from_stops(stops)
    }

    /// Whether every stored float is finite.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.stops
            .iter()
            .all(|s| s.t.is_finite() && s.color.iter().all(|c| c.is_finite()))
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Slider response
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// How a slider's travel maps to its value.
///
/// NeoAxis calls this a *convenient distribution* and it lands here rather
/// than in the widget because it is a property of the quantity, not of the
/// control: light intensity in lux and fog density per metre are both
/// hopeless on a linear track, and they are hopeless in every panel that
/// shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SliderCurve {
    /// Value is proportional to travel.
    #[default]
    Linear,
    /// Value is proportional to travel squared — fine control near the low end.
    Squared,
    /// Value is geometric in travel; requires a strictly positive range.
    Exponential,
}

impl SliderCurve {
    /// Map normalised travel `u` in `0..=1` to a value in `min..=max`.
    #[must_use]
    pub fn to_value(self, u: f32, min: f32, max: f32) -> f32 {
        let u = u.clamp(0.0, 1.0);
        match self {
            Self::Linear => min + (max - min) * u,
            Self::Squared => min + (max - min) * u * u,
            Self::Exponential if min > 0.0 && max > min => min * (max / min).powf(u),
            // A non-positive bound has no logarithm, so an exponential slider
            // over such a range degrades to linear rather than producing NaN.
            Self::Exponential => min + (max - min) * u,
        }
    }

    /// Inverse of [`Self::to_value`].
    #[must_use]
    pub fn to_travel(self, value: f32, min: f32, max: f32) -> f32 {
        if (max - min).abs() <= f32::EPSILON {
            return 0.0;
        }
        match self {
            Self::Linear => ((value - min) / (max - min)).clamp(0.0, 1.0),
            Self::Squared => ((value - min) / (max - min)).clamp(0.0, 1.0).sqrt(),
            Self::Exponential if min > 0.0 && max > min && value > 0.0 => {
                ((value / min).ln() / (max / min).ln()).clamp(0.0, 1.0)
            }
            Self::Exponential => ((value - min) / (max - min)).clamp(0.0, 1.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_curve_is_zero_and_does_not_panic() {
        let curve = Curve::empty();
        assert_eq!(curve.evaluate(0.0), 0.0);
        assert_eq!(curve.evaluate(-5.0), 0.0);
        assert_eq!(curve.evaluate(f32::INFINITY), 0.0);
    }

    #[test]
    fn evaluation_clamps_outside_the_authored_domain() {
        // The reason it clamps: hour 25 of a 24-hour track must be midnight's
        // value, not an extrapolated slope.
        let curve = Curve::ramp(2.0, 8.0);
        assert_eq!(curve.evaluate(-3.0), 2.0);
        assert_eq!(curve.evaluate(9.0), 8.0);
    }

    #[test]
    fn a_linear_segment_is_a_straight_line() {
        let curve = Curve::ramp(0.0, 10.0);
        assert!((curve.evaluate(0.25) - 2.5).abs() < 1e-5);
        assert!((curve.evaluate(0.5) - 5.0).abs() < 1e-5);
    }

    #[test]
    fn a_step_segment_holds_its_left_value() {
        let curve = Curve::from_keys(vec![
            CurveKey {
                interpolation: Interpolation::Step,
                ..CurveKey::new(0.0, 1.0)
            },
            CurveKey::new(1.0, 9.0),
        ]);
        assert_eq!(curve.evaluate(0.99), 1.0);
        assert_eq!(curve.evaluate(1.0), 9.0);
    }

    #[test]
    fn a_smooth_segment_is_flat_at_both_ends() {
        // Flat tangents are the "ease in, ease out" default, and the property
        // that makes them useful is exactly this: the derivative vanishes at
        // the keys, so two adjoining eased segments do not kink.
        let curve = Curve::from_keys(vec![CurveKey::smooth(0.0, 0.0), CurveKey::smooth(1.0, 1.0)]);
        let d0 = curve.evaluate(0.01) - curve.evaluate(0.0);
        let d_mid = curve.evaluate(0.51) - curve.evaluate(0.50);
        assert!(d0 < d_mid * 0.2, "start should be flatter than the middle");
        assert!((curve.evaluate(0.5) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn keys_are_sorted_whatever_order_they_arrive_in() {
        let curve = Curve::from_keys(vec![CurveKey::new(1.0, 5.0), CurveKey::new(0.0, 1.0)]);
        assert_eq!(curve.keys()[0].t, 0.0);
        assert_eq!(curve.evaluate(0.0), 1.0);
    }

    #[test]
    fn non_finite_keys_are_dropped_rather_than_poisoning_the_sort() {
        let curve = Curve::from_keys(vec![
            CurveKey::new(0.0, 1.0),
            CurveKey::new(f32::NAN, 2.0),
            CurveKey::new(1.0, 3.0),
        ]);
        assert_eq!(curve.len(), 2);
        assert!(curve.is_finite());
    }

    #[test]
    fn a_curve_round_trips_through_its_flat_form() {
        let curve = Curve::from_keys(vec![
            CurveKey {
                in_tangent: -1.5,
                out_tangent: 2.5,
                interpolation: Interpolation::Smooth,
                ..CurveKey::new(0.0, 1.0)
            },
            CurveKey {
                interpolation: Interpolation::Step,
                ..CurveKey::new(0.5, 4.0)
            },
            CurveKey::new(1.0, 0.0),
        ]);
        assert_eq!(Curve::from_flat(&curve.to_flat()), curve);
    }

    #[test]
    fn sampling_hits_both_endpoints() {
        let curve = Curve::ramp(3.0, 7.0);
        let table = curve.sample(0.0, 1.0, 5);
        assert_eq!(table.len(), 5);
        assert!((table[0] - 3.0).abs() < 1e-5);
        assert!((table[4] - 7.0).abs() < 1e-5);
        assert!((table[2] - 5.0).abs() < 1e-5);
    }

    #[test]
    fn a_gradient_interpolates_each_channel() {
        let gradient = Gradient::ramp([0.0, 0.0, 0.0, 1.0], [1.0, 0.5, 0.0, 0.0]);
        let mid = gradient.evaluate(0.5);
        assert!((mid[0] - 0.5).abs() < 1e-5);
        assert!((mid[1] - 0.25).abs() < 1e-5);
        assert!((mid[3] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn a_gradient_round_trips_through_its_flat_form() {
        let gradient = Gradient::from_stops(vec![
            GradientStop::new(0.0, [1.0, 0.0, 0.0, 1.0]),
            GradientStop::new(0.4, [0.0, 1.0, 0.0, 1.0]),
            GradientStop::new(1.0, [0.0, 0.0, 1.0, 0.0]),
        ]);
        assert_eq!(Gradient::from_flat(&gradient.to_flat()), gradient);
    }

    #[test]
    fn gradient_stops_are_clamped_into_the_unit_domain() {
        let gradient = Gradient::from_stops(vec![
            GradientStop::new(-2.0, [1.0; 4]),
            GradientStop::new(5.0, [0.0; 4]),
        ]);
        assert_eq!(gradient.stops()[0].t, 0.0);
        assert_eq!(gradient.stops()[1].t, 1.0);
    }

    #[test]
    fn every_preset_is_a_usable_curve_from_zero_to_one() {
        for (name, build) in Curve::PRESETS {
            let curve = build();
            assert!(curve.len() >= 2, "{name} needs at least two keys");
            assert!(curve.is_finite(), "{name} produced a non-finite key");
            let (t0, t1) = curve.domain().expect("preset has a domain");
            assert!((t0 - 0.0).abs() < 1e-6 && (t1 - 1.0).abs() < 1e-6, "{name}");
        }
    }

    #[test]
    fn an_exponential_slider_round_trips_and_is_finer_at_the_bottom() {
        let curve = SliderCurve::Exponential;
        let (min, max) = (0.01_f32, 100.0_f32);
        let value = curve.to_value(0.5, min, max);
        assert!((curve.to_travel(value, min, max) - 0.5).abs() < 1e-4);
        // Half the travel reaches the geometric mean, not the arithmetic one:
        // that is the whole point.
        assert!((value - 1.0).abs() < 1e-3, "midpoint was {value}");
    }

    #[test]
    fn an_exponential_slider_over_a_non_positive_range_degrades_to_linear() {
        let curve = SliderCurve::Exponential;
        let value = curve.to_value(0.5, -1.0, 1.0);
        assert!(value.is_finite());
        assert!((value - 0.0).abs() < 1e-6);
    }
}
