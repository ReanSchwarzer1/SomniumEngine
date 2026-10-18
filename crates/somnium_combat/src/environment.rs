//! The ground as combat sees it: surface tags on a grid, and what spells do
//! to them (GAMEPLAY 5: water freezes, grass burns, wind carries fire, stone
//! walls block).
//!
//! The host paints the field from authored surface volumes (or, later, from
//! terrain layers) and reads [`SurfaceChange`]s back to swap decals, start and
//! stop fire emitters, and make frozen water walkable. Fire spreads on a fixed
//! quarter-second tick from the world's seeded generator, biased downwind, so
//! a burning field replays the same way every time.
//!
//! | Element | Grass | Water | Ice | Snow | Burning | Ash |
//! |---|---|---|---|---|---|---|
//! | Flame | burns (unless wet: dries) | steam | melts to water | melts to water | refuels | — |
//! | Frost | — | freezes to walkable ice | refreezes | — | smothered to ash | — |
//! | Water | soaks (will not catch) | — | — | — | put out to ash | — |
//! | Gale | — | — | — | — | throws fire downwind | — |
//! | Stone | — | — | — | — | smothered to ash | — |
//! | Mending | — | — | — | — | — | grows back to grass |
use crate::Rng;
use crate::spell::Element;
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// What a patch of ground is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Surface {
    /// Nothing that reacts: earth, paving.
    Bare,
    /// Grass, reeds, dry brush: it burns.
    Grass,
    /// Standing water: it freezes, it soaks whoever stands in it.
    Water,
    /// Frozen water: walkable and slippery; melts back.
    Ice,
    /// Snow: melts to water under flame.
    Snow,
    /// On fire.
    Burning,
    /// Burnt out.
    Ash,
    /// Rock: never burns, stops fire.
    Stone,
}

impl Surface {
    /// Every surface, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::Bare,
        Self::Grass,
        Self::Water,
        Self::Ice,
        Self::Snow,
        Self::Burning,
        Self::Ash,
        Self::Stone,
    ];

    /// Display names, in [`Self::ALL`] order.
    pub const NAMES: [&'static str; 8] = [
        "Bare", "Grass", "Water", "Ice", "Snow", "Burning", "Ash", "Stone",
    ];

    /// Friction multiplier for a body standing on it.
    #[must_use]
    pub fn traction(self) -> f32 {
        match self {
            Self::Ice => 0.25,
            Self::Water => 0.7,
            Self::Snow => 0.8,
            _ => 1.0,
        }
    }

    /// Whether a person can stand on it (water is a wade at best; the host
    /// decides depth, this says whether the surface itself bears weight).
    #[must_use]
    pub fn bears_weight(self) -> bool {
        !matches!(self, Self::Water)
    }
}

/// Seconds a grass cell burns.
pub const GRASS_FUEL: f32 = 5.0;
/// Seconds frozen water stays frozen.
pub const ICE_SECONDS: f32 = 30.0;
/// Seconds soaked grass refuses to catch.
pub const WET_SECONDS: f32 = 20.0;
/// Fire spread step.
pub const SPREAD_TICK: f32 = 0.25;
/// Chance per tick that fire jumps to one still-air orthogonal neighbour.
pub const SPREAD_CHANCE: f32 = 0.12;
const MAX_CELLS: usize = 4_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Cell {
    surface: Surface,
    /// Burning: fuel left. Ice: seconds until it melts.
    timer: f32,
    /// Seconds until a soaked cell dries.
    wet: f32,
}

/// One cell's change, for the host to present.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceChange {
    /// Cell column.
    pub x: u32,
    /// Cell row.
    pub z: u32,
    /// World centre of the cell on the ground plane (x, z).
    pub center: Vec2,
    /// Before.
    pub from: Surface,
    /// After.
    pub to: Surface,
}

/// Moving air: authored draughts, or a gale spell's wake.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindVolume {
    /// Centre.
    #[serde(with = "crate::serde_vec3")]
    pub center: Vec3,
    /// Radius of influence (full strength at the centre, none at the edge).
    pub radius: f32,
    /// Air velocity at the centre, metres per second.
    #[serde(with = "crate::serde_vec3")]
    pub velocity: Vec3,
    /// Seconds left; infinite for authored wind.
    pub time_left: f32,
}

/// The reactive ground of one combat area.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvironmentField {
    origin: Vec2,
    cell: f32,
    width: u32,
    depth: u32,
    cells: Vec<Cell>,
    active: BTreeSet<u32>,
    spread_clock: f32,
    /// Wind in this area.
    pub winds: Vec<WindVolume>,
    changes: Vec<SurfaceChange>,
}

impl EnvironmentField {
    /// A bare field covering `min` to `max` on the ground plane in cells of
    /// `cell` metres. `None` for a degenerate or oversized field.
    #[must_use]
    pub fn new(min: Vec2, max: Vec2, cell: f32) -> Option<Self> {
        if !min.is_finite() || !max.is_finite() || !cell.is_finite() || cell <= 0.01 {
            return None;
        }
        let size = max - min;
        if size.x <= 0.0 || size.y <= 0.0 {
            return None;
        }
        let width = (size.x / cell).ceil() as u64;
        let depth = (size.y / cell).ceil() as u64;
        if width * depth > MAX_CELLS as u64 {
            return None;
        }
        Some(Self {
            origin: min,
            cell,
            width: width as u32,
            depth: depth as u32,
            cells: vec![
                Cell {
                    surface: Surface::Bare,
                    timer: 0.0,
                    wet: 0.0,
                };
                (width * depth) as usize
            ],
            active: BTreeSet::new(),
            spread_clock: 0.0,
            winds: Vec::new(),
            changes: Vec::new(),
        })
    }

    /// Cell size in metres.
    #[must_use]
    pub fn cell_size(&self) -> f32 {
        self.cell
    }

    /// Columns and rows.
    #[must_use]
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.depth)
    }

    fn index(&self, x: u32, z: u32) -> usize {
        (z * self.width + x) as usize
    }

    fn coords(&self, p: Vec2) -> Option<(u32, u32)> {
        let local = (p - self.origin) / self.cell;
        if !local.is_finite() || local.x < 0.0 || local.y < 0.0 {
            return None;
        }
        let (x, z) = (local.x as u32, local.y as u32);
        (x < self.width && z < self.depth).then_some((x, z))
    }

    /// World centre of a cell.
    #[must_use]
    pub fn center(&self, x: u32, z: u32) -> Vec2 {
        self.origin + (Vec2::new(x as f32, z as f32) + 0.5) * self.cell
    }

    /// The surface under a world point (Bare outside the field).
    #[must_use]
    pub fn surface_at(&self, p: Vec3) -> Surface {
        self.coords(Vec2::new(p.x, p.z))
            .map_or(Surface::Bare, |(x, z)| self.cells[self.index(x, z)].surface)
    }

    /// Whether the cell under a point is soaked.
    #[must_use]
    pub fn wet_at(&self, p: Vec3) -> bool {
        self.coords(Vec2::new(p.x, p.z))
            .is_some_and(|(x, z)| self.cells[self.index(x, z)].wet > 0.0)
    }

    /// How many cells are of each surface, in [`Surface::ALL`] order.
    #[must_use]
    pub fn census(&self) -> [u32; 8] {
        let mut out = [0; 8];
        for c in &self.cells {
            out[c.surface as usize] += 1;
        }
        out
    }

    fn set(&mut self, x: u32, z: u32, surface: Surface, timer: f32) {
        let i = self.index(x, z);
        let from = self.cells[i].surface;
        self.cells[i].surface = surface;
        self.cells[i].timer = timer;
        if timer > 0.0 || self.cells[i].wet > 0.0 {
            self.active.insert(i as u32);
        }
        if from != surface {
            self.changes.push(SurfaceChange {
                x,
                z,
                center: self.center(x, z),
                from,
                to: surface,
            });
        }
    }

    fn cells_in(&self, min: Vec2, max: Vec2) -> impl Iterator<Item = (u32, u32)> + use<> {
        let lo = ((min - self.origin) / self.cell).floor().max(Vec2::ZERO);
        let hi = ((max - self.origin) / self.cell)
            .ceil()
            .min(Vec2::new(self.width as f32, self.depth as f32));
        let (x0, z0, x1, z1) = (lo.x as u32, lo.y as u32, hi.x as u32, hi.y as u32);
        (z0..z1).flat_map(move |z| (x0..x1).map(move |x| (x, z)))
    }

    /// Paint an axis-aligned rectangle (authoring; reports no changes).
    pub fn paint_rect(&mut self, min: Vec2, max: Vec2, surface: Surface) {
        let cells: Vec<_> = self.cells_in(min, max).collect();
        for (x, z) in cells {
            let c = self.center(x, z);
            if c.x >= min.x && c.x <= max.x && c.y >= min.y && c.y <= max.y {
                let i = self.index(x, z);
                self.cells[i] = Cell {
                    surface,
                    timer: if surface == Surface::Burning {
                        GRASS_FUEL
                    } else {
                        0.0
                    },
                    wet: 0.0,
                };
                if surface == Surface::Burning {
                    self.active.insert(i as u32);
                }
            }
        }
    }

    /// Paint a disc (authoring; reports no changes).
    pub fn paint_circle(&mut self, center: Vec2, radius: f32, surface: Surface) {
        let cells: Vec<_> = self
            .cells_in(center - radius, center + radius)
            .filter(|&(x, z)| self.center(x, z).distance(center) <= radius)
            .collect();
        for (x, z) in cells {
            let i = self.index(x, z);
            self.cells[i] = Cell {
                surface,
                timer: 0.0,
                wet: 0.0,
            };
        }
    }

    fn react_cell(&mut self, x: u32, z: u32, element: Element) {
        let i = self.index(x, z);
        let cell = self.cells[i];
        match (element, cell.surface) {
            (Element::Flame, Surface::Grass) if cell.wet > 0.0 => {
                self.cells[i].wet = 0.0;
            }
            (Element::Flame, Surface::Grass) => self.set(x, z, Surface::Burning, GRASS_FUEL),
            (Element::Flame, Surface::Burning) => {
                self.cells[i].timer = GRASS_FUEL;
            }
            (Element::Flame, Surface::Ice | Surface::Snow) => self.set(x, z, Surface::Water, 0.0),
            (Element::Frost, Surface::Water) => self.set(x, z, Surface::Ice, ICE_SECONDS),
            (Element::Frost, Surface::Ice) => {
                self.cells[i].timer = ICE_SECONDS;
            }
            (Element::Frost | Element::Stone, Surface::Burning) => {
                self.set(x, z, Surface::Ash, 0.0);
            }
            (Element::Water, Surface::Burning) => {
                self.set(x, z, Surface::Ash, 0.0);
                self.cells[i].wet = WET_SECONDS;
                self.active.insert(i as u32);
            }
            (Element::Water, Surface::Grass | Surface::Bare | Surface::Ash) => {
                self.cells[i].wet = WET_SECONDS;
                self.active.insert(i as u32);
            }
            (Element::Mending, Surface::Ash) => self.set(x, z, Surface::Grass, 0.0),
            _ => {}
        }
    }

    /// Apply an element over a disc. Gale throws fire along `direction`.
    pub fn react(
        &mut self,
        element: Element,
        center: Vec3,
        radius: f32,
        direction: Vec3,
        rng: &mut Rng,
    ) {
        if !center.is_finite() || !radius.is_finite() || radius < 0.0 {
            return;
        }
        let c = Vec2::new(center.x, center.z);
        let r = radius.max(self.cell * 0.5);
        let cells: Vec<_> = self
            .cells_in(c - r, c + r)
            .filter(|&(x, z)| self.center(x, z).distance(c) <= r)
            .collect();
        if element == Element::Gale {
            let dir = Vec2::new(direction.x, direction.z)
                .try_normalize()
                .unwrap_or(Vec2::X);
            let burning: Vec<_> = cells
                .iter()
                .copied()
                .filter(|&(x, z)| self.cells[self.index(x, z)].surface == Surface::Burning)
                .collect();
            for (x, z) in burning {
                // Embers carried one to three cells downwind.
                for step in 1..=3 {
                    if !rng.chance(0.8 / step as f32) {
                        continue;
                    }
                    let to = self.center(x, z) + dir * self.cell * step as f32;
                    if let Some((tx, tz)) = self.coords(to) {
                        self.react_cell(tx, tz, Element::Flame);
                    }
                }
            }
            return;
        }
        for (x, z) in cells {
            self.react_cell(x, z, element);
        }
    }

    /// Apply an element along a line `half_width` either side (a frost step's
    /// trail, a wall's foot).
    pub fn react_line(&mut self, element: Element, a: Vec3, b: Vec3, half_width: f32) {
        let (a2, b2) = (Vec2::new(a.x, a.z), Vec2::new(b.x, b.z));
        let w = half_width.max(self.cell * 0.5);
        let cells: Vec<_> = self
            .cells_in(a2.min(b2) - w, a2.max(b2) + w)
            .filter(|&(x, z)| {
                let p = self.center(x, z);
                let ab = b2 - a2;
                let t = if ab.length_squared() > 1e-8 {
                    ((p - a2).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (a2 + ab * t).distance(p) <= w
            })
            .collect();
        for (x, z) in cells {
            self.react_cell(x, z, element);
        }
    }

    /// Air velocity at a point.
    #[must_use]
    pub fn wind_at(&self, p: Vec3) -> Vec3 {
        self.winds
            .iter()
            .filter(|w| w.radius > 0.0)
            .map(|w| {
                let d = Vec2::new(p.x - w.center.x, p.z - w.center.z).length();
                w.velocity * (1.0 - d / w.radius).max(0.0)
            })
            .sum()
    }

    /// Burn fuel, melt ice, dry soaked ground, age wind, and spread fire.
    pub fn tick(&mut self, dt: f32, rng: &mut Rng) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        for w in &mut self.winds {
            w.time_left -= dt;
        }
        self.winds.retain(|w| w.time_left > 0.0);
        let active: Vec<u32> = self.active.iter().copied().collect();
        for i in active {
            let (x, z) = (i % self.width, i / self.width);
            let idx = i as usize;
            self.cells[idx].wet = (self.cells[idx].wet - dt).max(0.0);
            let surface = self.cells[idx].surface;
            if matches!(surface, Surface::Burning | Surface::Ice) {
                self.cells[idx].timer -= dt;
                if self.cells[idx].timer <= 0.0 {
                    let to = if surface == Surface::Burning {
                        Surface::Ash
                    } else {
                        Surface::Water
                    };
                    self.set(x, z, to, 0.0);
                }
            }
            let cell = self.cells[idx];
            let lasting =
                matches!(cell.surface, Surface::Burning | Surface::Ice) && cell.timer > 0.0;
            if !lasting && cell.wet <= 0.0 {
                self.active.remove(&i);
            }
        }
        self.spread_clock += dt;
        while self.spread_clock >= SPREAD_TICK {
            self.spread_clock -= SPREAD_TICK;
            self.spread(rng);
        }
    }

    fn spread(&mut self, rng: &mut Rng) {
        let burning: Vec<u32> = self
            .active
            .iter()
            .copied()
            .filter(|&i| self.cells[i as usize].surface == Surface::Burning)
            .collect();
        let mut ignite = BTreeSet::new();
        for i in burning {
            let (x, z) = (i % self.width, i / self.width);
            let here = self.center(x, z);
            let wind = self.wind_at(Vec3::new(here.x, 0.0, here.y));
            let wind = Vec2::new(wind.x, wind.z);
            for (dx, dz) in [
                (-1i32, -1i32),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ] {
                let (nx, nz) = (x as i32 + dx, z as i32 + dz);
                if nx < 0 || nz < 0 || nx >= self.width as i32 || nz >= self.depth as i32 {
                    continue;
                }
                let n = self.index(nx as u32, nz as u32);
                let cell = self.cells[n];
                if cell.surface != Surface::Grass || cell.wet > 0.0 {
                    continue;
                }
                let dir = Vec2::new(dx as f32, dz as f32).normalize();
                let along = wind.dot(dir);
                let factor = (1.0 + along.max(0.0) * 0.35 - (-along).max(0.0) * 0.08)
                    .clamp(0.05, 4.0)
                    * if dx != 0 && dz != 0 { 0.7 } else { 1.0 };
                if rng.chance(SPREAD_CHANCE * factor) {
                    ignite.insert(n as u32);
                }
            }
        }
        for n in ignite {
            let (x, z) = (n % self.width, n / self.width);
            self.set(x, z, Surface::Burning, GRASS_FUEL);
        }
    }

    /// Changes since the last drain.
    pub fn drain_changes(&mut self) -> Vec<SurfaceChange> {
        std::mem::take(&mut self.changes)
    }

    /// Cells currently on fire, for presentation.
    pub fn burning(&self) -> impl Iterator<Item = Vec2> + '_ {
        self.active
            .iter()
            .filter(|&&i| self.cells[i as usize].surface == Surface::Burning)
            .map(|&i| self.center(i % self.width, i / self.width))
    }

    /// Every cell that is not bare, for presentation: centre and surface.
    pub fn painted(&self) -> impl Iterator<Item = (Vec2, Surface)> + '_ {
        self.cells
            .iter()
            .enumerate()
            .filter(|(_, c)| c.surface != Surface::Bare)
            .map(|(i, c)| {
                let i = i as u32;
                (self.center(i % self.width, i / self.width), c.surface)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field() -> EnvironmentField {
        EnvironmentField::new(Vec2::splat(-10.0), Vec2::splat(10.0), 0.5).unwrap()
    }

    #[test]
    fn frost_freezes_water_into_walkable_ice_that_melts_back() {
        let mut f = field();
        f.paint_circle(Vec2::ZERO, 3.0, Surface::Water);
        let mut rng = Rng::new(1);
        assert!(!f.surface_at(Vec3::ZERO).bears_weight());
        f.react(Element::Frost, Vec3::ZERO, 2.0, Vec3::Z, &mut rng);
        assert_eq!(f.surface_at(Vec3::ZERO), Surface::Ice);
        assert!(f.surface_at(Vec3::ZERO).bears_weight());
        assert_eq!(f.surface_at(Vec3::new(2.7, 0.0, 0.0)), Surface::Water);
        let changes = f.drain_changes();
        assert!(!changes.is_empty() && changes.iter().all(|c| c.to == Surface::Ice));
        for _ in 0..(ICE_SECONDS as usize + 1) {
            f.tick(1.0, &mut rng);
        }
        assert_eq!(f.surface_at(Vec3::ZERO), Surface::Water, "it melts");
        // Fire melts ice at once.
        f.react(Element::Frost, Vec3::ZERO, 1.0, Vec3::Z, &mut rng);
        f.react(Element::Flame, Vec3::ZERO, 1.0, Vec3::Z, &mut rng);
        assert_eq!(f.surface_at(Vec3::ZERO), Surface::Water);
    }

    #[test]
    fn grass_burns_spreads_and_leaves_ash() {
        let mut f = field();
        f.paint_rect(Vec2::splat(-10.0), Vec2::splat(10.0), Surface::Grass);
        let mut rng = Rng::new(3);
        f.react(Element::Flame, Vec3::ZERO, 0.5, Vec3::Z, &mut rng);
        let lit = f.census()[Surface::Burning as usize];
        assert!(lit >= 1);
        for _ in 0..40 {
            f.tick(0.25, &mut rng);
        }
        let census = f.census();
        assert!(
            census[Surface::Burning as usize] + census[Surface::Ash as usize] > lit * 10,
            "the fire spread: {census:?}"
        );
        assert!(
            census[Surface::Ash as usize] > 0,
            "and burnt out behind itself"
        );
    }

    #[test]
    fn wind_carries_fire_downwind() {
        let run = |wind: Vec3| {
            let mut f = field();
            f.paint_rect(Vec2::splat(-10.0), Vec2::splat(10.0), Surface::Grass);
            f.winds.push(WindVolume {
                center: Vec3::ZERO,
                radius: 30.0,
                velocity: wind,
                time_left: f32::INFINITY,
            });
            let mut rng = Rng::new(11);
            f.react(Element::Flame, Vec3::ZERO, 0.5, Vec3::Z, &mut rng);
            for _ in 0..24 {
                f.tick(0.25, &mut rng);
            }
            let (mut east, mut west) = (0, 0);
            for (p, s) in f.painted() {
                if matches!(s, Surface::Burning | Surface::Ash) {
                    if p.x > 1.0 {
                        east += 1;
                    } else if p.x < -1.0 {
                        west += 1;
                    }
                }
            }
            (east, west)
        };
        let (east, west) = run(Vec3::new(8.0, 0.0, 0.0));
        assert!(
            east > west * 2,
            "an east wind's fire runs east: {east} vs {west}"
        );
    }

    #[test]
    fn water_soaks_grass_so_it_will_not_catch_and_puts_fire_out() {
        let mut f = field();
        f.paint_rect(Vec2::splat(-10.0), Vec2::splat(10.0), Surface::Grass);
        let mut rng = Rng::new(5);
        f.react(Element::Water, Vec3::ZERO, 2.0, Vec3::Z, &mut rng);
        f.react(Element::Flame, Vec3::ZERO, 1.0, Vec3::Z, &mut rng);
        assert_eq!(
            f.surface_at(Vec3::ZERO),
            Surface::Grass,
            "wet grass dries instead"
        );
        f.react(
            Element::Flame,
            Vec3::new(5.0, 0.0, 5.0),
            1.0,
            Vec3::Z,
            &mut rng,
        );
        assert_eq!(f.surface_at(Vec3::new(5.0, 0.0, 5.0)), Surface::Burning);
        f.react(
            Element::Water,
            Vec3::new(5.0, 0.0, 5.0),
            1.5,
            Vec3::Z,
            &mut rng,
        );
        assert_eq!(f.surface_at(Vec3::new(5.0, 0.0, 5.0)), Surface::Ash);
        f.react(
            Element::Mending,
            Vec3::new(5.0, 0.0, 5.0),
            1.0,
            Vec3::Z,
            &mut rng,
        );
        assert_eq!(
            f.surface_at(Vec3::new(5.0, 0.0, 5.0)),
            Surface::Grass,
            "mending regrows"
        );
    }

    #[test]
    fn stone_stops_fire_and_gale_throws_it() {
        let banded = || {
            let mut f = field();
            f.paint_rect(
                Vec2::new(-10.0, -10.0),
                Vec2::new(10.0, 10.0),
                Surface::Grass,
            );
            f.paint_rect(Vec2::new(1.0, -10.0), Vec2::new(2.0, 10.0), Surface::Stone);
            f
        };
        let beyond = |f: &EnvironmentField| {
            f.painted()
                .filter(|(p, s)| p.x > 2.0 && matches!(s, Surface::Burning | Surface::Ash))
                .count()
        };
        let mut rng = Rng::new(9);
        let mut still = banded();
        still.react(
            Element::Flame,
            Vec3::new(0.75, 0.0, 0.0),
            0.5,
            Vec3::Z,
            &mut rng,
        );
        for _ in 0..40 {
            still.tick(0.25, &mut rng);
        }
        assert_eq!(
            beyond(&still),
            0,
            "fire does not cross a stone band in still air"
        );
        assert!(still.census()[Surface::Ash as usize] > 0);

        let mut windy = banded();
        windy.react(
            Element::Flame,
            Vec3::new(0.75, 0.0, 0.0),
            0.5,
            Vec3::Z,
            &mut rng,
        );
        for _ in 0..20 {
            windy.react(
                Element::Gale,
                Vec3::new(0.75, 0.0, 0.0),
                1.0,
                Vec3::X,
                &mut rng,
            );
            windy.tick(0.25, &mut rng);
        }
        assert!(beyond(&windy) > 0, "a gale carries fire over the stone");
    }

    #[test]
    fn the_same_seed_burns_the_same_field() {
        let burn = || {
            let mut f = field();
            f.paint_rect(Vec2::splat(-10.0), Vec2::splat(10.0), Surface::Grass);
            let mut rng = Rng::new(42);
            f.react(Element::Flame, Vec3::ZERO, 0.5, Vec3::Z, &mut rng);
            for _ in 0..30 {
                f.tick(0.1, &mut rng);
            }
            f.census()
        };
        assert_eq!(burn(), burn());
    }

    #[test]
    fn degenerate_fields_are_refused() {
        assert!(EnvironmentField::new(Vec2::ZERO, Vec2::ZERO, 1.0).is_none());
        assert!(EnvironmentField::new(Vec2::ZERO, Vec2::ONE, 0.0).is_none());
        assert!(
            EnvironmentField::new(Vec2::ZERO, Vec2::splat(1.0e5), 0.01).is_none(),
            "too many cells"
        );
    }
}
