//! Deterministic rule-driven scattering, independent of terrain and editor UI.
//!
//! Sources evaluate surface positions into [0,1] weights. Composition, tags
//! and exclusions use the same value path. Terrain and image data are borrowed
//! at the evaluation seam, so authored graphs contain no renderer handles.

use glam::{Vec2, Vec3};
use std::collections::BTreeMap;

/// A sampled ground position and its classifications.
#[derive(Clone, Debug)]
pub struct SurfacePoint {
    pub position: Vec3,
    pub normal: Vec3,
    /// Surface names mapped to weights in [0,1].
    pub tags: BTreeMap<String, f32>,
}

/// Single-channel image mapped onto a world XZ rectangle.
#[derive(Clone, Debug)]
pub struct GradientImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<f32>,
    pub min: Vec2,
    pub max: Vec2,
}

impl GradientImage {
    fn sample(&self, point: Vec2) -> f32 {
        if self.width == 0
            || self.height == 0
            || self.width as usize * self.height as usize != self.pixels.len()
            || !self.min.is_finite()
            || !self.max.is_finite()
            || (self.max - self.min).min_element() <= 0.0
        {
            return 0.0;
        }
        let uv = (point - self.min) / (self.max - self.min);
        if uv.min_element() < 0.0 || uv.max_element() > 1.0 {
            return 0.0;
        }
        let p = uv * Vec2::new((self.width - 1) as f32, (self.height - 1) as f32);
        let x = p.x.floor() as u32;
        let y = p.y.floor() as u32;
        let at = |x: u32, y: u32| self.pixels[(y * self.width + x) as usize];
        let [a, b, c, d] = [
            at(x, y),
            at((x + 1).min(self.width - 1), y),
            at(x, (y + 1).min(self.height - 1)),
            at((x + 1).min(self.width - 1), (y + 1).min(self.height - 1)),
        ];
        unit(
            (a + (b - a) * p.x.fract()) * (1.0 - p.y.fract())
                + (c + (d - c) * p.x.fract()) * p.y.fract(),
        )
    }
}

/// Composable scalar sources. Every field is durable graph data.
#[derive(Clone, Debug, PartialEq)]
pub enum Gradient {
    Constant(f32),
    Noise {
        seed: u32,
        frequency: f32,
    },
    Image {
        asset: String,
    },
    /// Linear world-height remap.
    Altitude {
        min: f32,
        max: f32,
    },
    /// Linear slope-angle remap in degrees (0 is level ground).
    Slope {
        min: f32,
        max: f32,
    },
    Distance {
        center: Vec3,
        radius: f32,
    },
    Shape {
        min: Vec3,
        max: Vec3,
    },
    SurfaceTag {
        name: String,
        minimum: f32,
    },
    Multiply(Box<Gradient>, Box<Gradient>),
    /// Inclusive distribution filter; rejected values become zero.
    Filter {
        source: Box<Gradient>,
        min: f32,
        max: f32,
    },
    Exclude {
        source: Box<Gradient>,
        min: Vec3,
        max: Vec3,
    },
    Invert(Box<Gradient>),
}

impl Gradient {
    fn validate(&self, depth: usize) -> Result<(), String> {
        if depth > 128 {
            return Err("Scatter graph exceeds 128 nested sources".into());
        }
        let range = |a: f32, b: f32| a.is_finite() && b.is_finite() && b > a;
        let bounds =
            |a: Vec3, b: Vec3| a.is_finite() && b.is_finite() && (b - a).min_element() > 0.0;
        let valid = match self {
            Self::Constant(v) => v.is_finite() && (0.0..=1.0).contains(v),
            Self::Noise { frequency, .. } => frequency.is_finite() && *frequency > 0.0,
            Self::Image { asset } => !asset.is_empty(),
            Self::Altitude { min, max } | Self::Slope { min, max } => range(*min, *max),
            Self::Distance { center, radius } => {
                center.is_finite() && radius.is_finite() && *radius > 0.0
            }
            Self::Shape { min, max } => bounds(*min, *max),
            Self::SurfaceTag { name, minimum } => {
                !name.is_empty() && minimum.is_finite() && (0.0..=1.0).contains(minimum)
            }
            Self::Multiply(a, b) => {
                a.validate(depth + 1)?;
                b.validate(depth + 1)?;
                true
            }
            Self::Filter { source, min, max } => {
                source.validate(depth + 1)?;
                min.is_finite() && max.is_finite() && min <= max
            }
            Self::Exclude { source, min, max } => {
                source.validate(depth + 1)?;
                bounds(*min, *max)
            }
            Self::Invert(source) => {
                source.validate(depth + 1)?;
                true
            }
        };
        if valid {
            Ok(())
        } else {
            Err("Scatter source has invalid bounds, values or asset name".into())
        }
    }

    fn evaluate(&self, point: &SurfacePoint, images: &BTreeMap<String, GradientImage>) -> f32 {
        let inside = |min: Vec3, max: Vec3| {
            point.position.cmpge(min).all() && point.position.cmple(max).all()
        };
        let p = point.position;
        unit(match self {
            Self::Constant(value) => *value,
            Self::Noise { seed, frequency } => {
                let x = p.x * frequency;
                let z = p.z * frequency;
                let ix = x.floor() as i32;
                let iz = z.floor() as i32;
                let sx = x.fract().rem_euclid(1.0);
                let sz = z.fract().rem_euclid(1.0);
                let sx = sx * sx * (3.0 - 2.0 * sx);
                let sz = sz * sz * (3.0 - 2.0 * sz);
                let a = random(*seed, ix, iz, 0);
                let b = random(*seed, ix.wrapping_add(1), iz, 0);
                let c = random(*seed, ix, iz.wrapping_add(1), 0);
                let d = random(*seed, ix.wrapping_add(1), iz.wrapping_add(1), 0);
                (a + (b - a) * sx) * (1.0 - sz) + (c + (d - c) * sx) * sz
            }
            Self::Image { asset } => images
                .get(asset)
                .map_or(0.0, |image| image.sample(Vec2::new(p.x, p.z))),
            Self::Altitude { min, max } => (p.y - min) / (max - min),
            Self::Slope { min, max } => {
                let degrees = point
                    .normal
                    .normalize_or_zero()
                    .dot(Vec3::Y)
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees();
                (degrees - min) / (max - min)
            }
            Self::Distance { center, radius } => 1.0 - p.distance(*center) / radius,
            Self::Shape { min, max } => f32::from(inside(*min, *max)),
            Self::SurfaceTag { name, minimum } => f32::from(
                point
                    .tags
                    .get(name)
                    .is_some_and(|v| v.is_finite() && *v >= *minimum),
            ),
            Self::Multiply(a, b) => a.evaluate(point, images) * b.evaluate(point, images),
            Self::Filter { source, min, max } => {
                let v = source.evaluate(point, images);
                f32::from(v >= *min && v <= *max)
            }
            Self::Exclude { source, min, max } => {
                if inside(*min, *max) {
                    0.0
                } else {
                    source.evaluate(point, images)
                }
            }
            Self::Invert(source) => 1.0 - source.evaluate(point, images),
        })
    }
}

fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn random(seed: u32, x: i32, z: i32, salt: u32) -> f32 {
    let mut value =
        seed ^ (x as u32).wrapping_mul(0x9e37_79b9) ^ (z as u32).wrapping_mul(0x85eb_ca6b) ^ salt;
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    (value >> 8) as f32 / 16_777_216.0
}

/// Runtime description compiled from the shared graph editor.
#[derive(Clone, Debug, PartialEq)]
pub struct ScatterRule {
    pub gradient: Gradient,
    pub seed: u32,
    /// Maximum candidates per square metre. Density is independently editable
    /// and changes acceptance without moving existing candidate positions.
    pub spacing: f32,
    pub density: f32,
    /// 0 is a regular grid, 1 uses a seeded position inside each grid cell.
    pub jitter: f32,
    pub scale_min: f32,
    pub scale_max: f32,
    pub max_instances: u32,
}

/// Deterministic placement ready for an ordinary mesh/prefab instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScatterInstance {
    pub position: Vec3,
    pub normal: Vec3,
    pub yaw: f32,
    pub scale: f32,
}

impl ScatterRule {
    /// Load each referenced source image once, mapped across the bake region.
    /// Missing or malformed masks are authoring errors, never silent zero
    /// density. The interactive runtime may instead supply cached images.
    pub fn load_images(
        &self,
        min: Vec2,
        max: Vec2,
    ) -> Result<BTreeMap<String, GradientImage>, String> {
        self.validate()?;
        let mut pending = vec![&self.gradient];
        let mut images = BTreeMap::new();
        while let Some(source) = pending.pop() {
            match source {
                Gradient::Image { asset } if !images.contains_key(asset) => {
                    let decoded = image::open(asset)
                        .map_err(|error| format!("Scatter mask {asset}: {error}"))?
                        .to_luma32f();
                    let (width, height) = decoded.dimensions();
                    images.insert(
                        asset.clone(),
                        GradientImage {
                            width,
                            height,
                            pixels: decoded.into_raw(),
                            min,
                            max,
                        },
                    );
                }
                Gradient::Multiply(a, b) => {
                    pending.push(a);
                    pending.push(b);
                }
                Gradient::Filter { source, .. }
                | Gradient::Exclude { source, .. }
                | Gradient::Invert(source) => pending.push(source),
                _ => {}
            }
        }
        Ok(images)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.gradient.validate(0)?;
        if !self.spacing.is_finite()
            || self.spacing < 0.01
            || !self.density.is_finite()
            || self.density < 0.0
            || !self.jitter.is_finite()
            || !(0.0..=1.0).contains(&self.jitter)
            || !self.scale_min.is_finite()
            || !self.scale_max.is_finite()
            || self.scale_min <= 0.0
            || self.scale_max < self.scale_min
            || self.max_instances > 250_000
        {
            return Err(
                "Invalid scatter spacing, density, jitter, scale or instance budget".into(),
            );
        }
        Ok(())
    }

    /// Scatter into a half-open world XZ rectangle, sampling the supplied
    /// terrain adapter. No global RNG, allocations per candidate or GPU state.
    /// Oversized regions fail before work starts, rather than freezing editing.
    pub fn scatter(
        &self,
        min: Vec2,
        max: Vec2,
        images: &BTreeMap<String, GradientImage>,
        mut surface: impl FnMut(Vec2) -> Option<SurfacePoint>,
    ) -> Result<Vec<ScatterInstance>, String> {
        self.validate()?;
        if !min.is_finite()
            || !max.is_finite()
            || (max - min).min_element() <= 0.0
            || min.abs().max_element() > 1_000_000.0
            || max.abs().max_element() > 1_000_000.0
        {
            return Err("Invalid scatter region".into());
        }
        let first = (min / self.spacing).floor().as_ivec2();
        let last = (max / self.spacing).ceil().as_ivec2();
        let count = i64::from(last.x - first.x) * i64::from(last.y - first.y);
        if count > 250_000 {
            return Err("Scatter region exceeds 250000 candidates; increase spacing".into());
        }
        let mut output = Vec::new();
        if self.max_instances == 0 || self.density == 0.0 {
            return Ok(output);
        }
        for z in first.y..last.y {
            for x in first.x..last.x {
                let offset = Vec2::new(random(self.seed, x, z, 1), random(self.seed, x, z, 2));
                let position = (Vec2::new(x as f32, z as f32)
                    + Vec2::splat(0.5).lerp(offset, self.jitter))
                    * self.spacing;
                if position.cmplt(min).any() || position.cmpge(max).any() {
                    continue;
                }
                let Some(point) = surface(position) else {
                    continue;
                };
                if !point.position.is_finite()
                    || !point.normal.is_finite()
                    || point.normal.length_squared() < f32::EPSILON
                {
                    continue;
                }
                let probability = unit(
                    self.gradient.evaluate(&point, images)
                        * self.density
                        * self.spacing
                        * self.spacing,
                );
                if random(self.seed, x, z, 3) >= probability {
                    continue;
                }
                output.push(ScatterInstance {
                    position: point.position,
                    normal: point.normal.normalize(),
                    yaw: random(self.seed, x, z, 4) * std::f32::consts::TAU,
                    scale: self.scale_min
                        + (self.scale_max - self.scale_min) * random(self.seed, x, z, 5),
                });
                if output.len() >= self.max_instances as usize {
                    return Ok(output);
                }
            }
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn point(p: Vec2) -> Option<SurfacePoint> {
        Some(SurfacePoint {
            position: Vec3::new(p.x, 2.0, p.y),
            normal: Vec3::Y,
            tags: BTreeMap::from([("soil".into(), 1.0)]),
        })
    }
    fn rule() -> ScatterRule {
        ScatterRule {
            gradient: Gradient::Constant(1.0),
            seed: 42,
            spacing: 1.0,
            density: 1.0,
            jitter: 1.0,
            scale_min: 0.5,
            scale_max: 2.0,
            max_instances: 1000,
        }
    }

    #[test]
    fn deterministic_and_adjacent_regions_have_identical_candidates() {
        let rule = rule();
        let images = BTreeMap::new();
        let all = rule
            .scatter(Vec2::ZERO, Vec2::splat(10.0), &images, point)
            .unwrap();
        assert_eq!(
            all,
            rule.scatter(Vec2::ZERO, Vec2::splat(10.0), &images, point)
                .unwrap()
        );
        let mut split = rule
            .scatter(Vec2::ZERO, Vec2::new(5.0, 10.0), &images, point)
            .unwrap();
        split.extend(
            rule.scatter(Vec2::new(5.0, 0.0), Vec2::splat(10.0), &images, point)
                .unwrap(),
        );
        assert_eq!(all.len(), split.len());
        assert!(all.iter().all(|p| split.contains(p)));
        let sparse = ScatterRule {
            density: 0.4,
            ..rule
        }
        .scatter(Vec2::ZERO, Vec2::splat(10.0), &images, point)
        .unwrap();
        assert!(!sparse.is_empty() && sparse.len() < all.len());
        assert!(sparse.iter().all(|p| all.contains(p)));
    }

    #[test]
    fn sources_tags_and_exclusions_compose() {
        let p = point(Vec2::ONE).unwrap();
        let images = BTreeMap::new();
        for source in [
            Gradient::Altitude { min: 0.0, max: 4.0 },
            Gradient::Distance {
                center: p.position,
                radius: 2.0,
            },
            Gradient::Shape {
                min: Vec3::ZERO,
                max: Vec3::splat(4.0),
            },
            Gradient::SurfaceTag {
                name: "soil".into(),
                minimum: 0.5,
            },
        ] {
            source.validate(0).unwrap();
            assert!(source.evaluate(&p, &images) > 0.0);
        }
        assert_eq!(
            Gradient::Slope {
                min: 0.0,
                max: 90.0
            }
            .evaluate(&p, &images),
            0.0
        );
        let source = Gradient::Exclude {
            source: Box::new(Gradient::Constant(1.0)),
            min: Vec3::ZERO,
            max: Vec3::splat(3.0),
        };
        assert_eq!(source.evaluate(&p, &images), 0.0);
        assert_eq!(
            Gradient::SurfaceTag {
                name: "rock".into(),
                minimum: 0.0
            }
            .evaluate(&p, &images),
            0.0
        );
    }

    #[test]
    fn image_bilinear_and_invalid_budgets() {
        let image = GradientImage {
            width: 2,
            height: 2,
            pixels: vec![0.0, 1.0, 0.0, 1.0],
            min: Vec2::ZERO,
            max: Vec2::ONE,
        };
        assert_eq!(image.sample(Vec2::splat(0.5)), 0.5);
        assert_eq!(image.sample(Vec2::splat(2.0)), 0.0);
        assert!(rule()
            .scatter(Vec2::ZERO, Vec2::splat(10000.0), &BTreeMap::new(), point)
            .is_err());
        assert!(ScatterRule {
            density: f32::NAN,
            ..rule()
        }
        .validate()
        .is_err());
    }
}
