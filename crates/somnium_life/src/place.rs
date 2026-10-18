//! Places, their spots, and the lanes people walk between them.

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// glam here has no serde feature: positions travel as `[x, y, z]`.
mod v3 {
    use glam::Vec3;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &Vec3, s: S) -> Result<S::Ok, S::Error> {
        v.to_array().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec3, D::Error> {
        <[f32; 3]>::deserialize(d).map(Vec3::from_array)
    }
}

/// Polylines as lists of `[x, y, z]`.
mod lines {
    use glam::Vec3;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &[Vec<Vec3>], s: S) -> Result<S::Ok, S::Error> {
        v.iter()
            .map(|l| l.iter().map(|p| p.to_array()).collect::<Vec<_>>())
            .collect::<Vec<_>>()
            .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Vec<Vec3>>, D::Error> {
        Vec::<Vec<[f32; 3]>>::deserialize(d).map(|v| {
            v.into_iter()
                .map(|l| l.into_iter().map(Vec3::from_array).collect())
                .collect()
        })
    }
}

/// What a spot is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SpotKind {
    /// A household's door.
    Home,
    /// A workplace's door: smithy, storehouse, barn, mill.
    Work,
    /// Stalls and trade.
    Market,
    /// The inn or the hall: drink and company.
    Inn,
    /// The church or shrine.
    Chapel,
    /// Tilled ground.
    Field,
    /// Where livestock are kept (a byre's yard, a fold).
    Pen,
    /// The green, the yard, the square: where people stand about.
    Green,
    /// The well.
    Well,
}

impl SpotKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 9] = [
        Self::Home,
        Self::Work,
        Self::Market,
        Self::Inn,
        Self::Chapel,
        Self::Field,
        Self::Pen,
        Self::Green,
        Self::Well,
    ];

    /// Display names, in [`Self::ALL`] order.
    pub const NAMES: [&'static str; 9] = [
        "Home", "Work", "Market", "Inn", "Chapel", "Field", "Pen", "Green", "Well",
    ];

    /// Whether arriving means going inside (out of sight).
    #[must_use]
    pub fn indoor(self) -> bool {
        matches!(self, Self::Home | Self::Work | Self::Inn | Self::Chapel)
    }

    /// The spot a building of this type gives, by the names settlement
    /// generators use (`cottage`, `longhouse`, `smithy`, `church`, `byre`...).
    #[must_use]
    pub fn for_building(kind: &str) -> Option<Self> {
        let k = kind.to_ascii_lowercase();
        Some(match k.as_str() {
            "cottage" | "longhouse" | "farmhouse" | "house" | "townhouse" | "hut" | "hovel"
            | "croft" => Self::Home,
            "smithy" | "forge" | "storehouse" | "barn" | "granary" | "mill" | "workshop"
            | "tannery" | "bakery" | "brewery" | "warehouse" => Self::Work,
            "church" | "chapel" | "shrine" | "temple" | "kirk" => Self::Chapel,
            "inn" | "tavern" | "alehouse" | "hall" | "meadhall" => Self::Inn,
            "byre" | "stable" | "fold" | "pen" | "sty" => Self::Pen,
            "market" | "stall" | "market_hall" => Self::Market,
            "well" => Self::Well,
            _ => return None,
        })
    }

    /// The spot a named open area gives ("the green", "the yard", "market
    /// square", "north field").
    #[must_use]
    pub fn for_area(name: &str) -> Option<Self> {
        let n = name.to_ascii_lowercase();
        let has = |w: &[&str]| w.iter().any(|w| n.contains(w));
        if has(&["market", "square"]) {
            Some(Self::Market)
        } else if has(&["field", "meadow", "croft", "acre", "strip"]) {
            Some(Self::Field)
        } else if has(&["pen", "fold", "paddock", "pasture"]) {
            Some(Self::Pen)
        } else if has(&["well"]) {
            Some(Self::Well)
        } else if has(&["green", "yard", "common", "close"]) {
            Some(Self::Green)
        } else {
            None
        }
    }
}

/// A place where something happens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spot {
    /// What for.
    pub kind: SpotKind,
    /// The door of a building, or the middle of an open spot.
    #[serde(with = "v3")]
    pub position: Vec3,
    /// How far from the middle people (or beasts) spread out.
    #[serde(default = "default_radius")]
    pub radius: f32,
    /// A name for readouts ("the smithy", "Bryn longhouse").
    #[serde(default)]
    pub label: String,
}

fn default_radius() -> f32 {
    2.0
}

impl Spot {
    /// A spot of `kind` at `position`.
    #[must_use]
    pub fn new(kind: SpotKind, position: Vec3) -> Self {
        Self {
            kind,
            position,
            radius: if kind.indoor() { 0.5 } else { 3.0 },
            label: String::new(),
        }
    }
}

/// A building as a settlement generator lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildingRef {
    /// Its type (`cottage`, `smithy`, ...).
    pub kind: String,
    /// Its door, or its middle.
    pub door: Vec3,
    /// A name for readouts.
    pub label: String,
}

/// A named open area as a settlement generator lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct AreaRef {
    /// Its name ("the green", "the yard").
    pub name: String,
    /// Its middle.
    pub centre: Vec3,
    /// About how far it reaches.
    pub radius: f32,
}

/// A settlement, or a quarter of one: its spots, lanes and roads.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Place {
    /// Name.
    pub name: String,
    /// Where things happen.
    pub spots: Vec<Spot>,
    /// Paths people walk, as polylines (they join where points meet).
    #[serde(default, with = "lines")]
    pub lanes: Vec<Vec<Vec3>>,
    /// Roads carts drive, as polylines.
    #[serde(default, with = "lines")]
    pub roads: Vec<Vec<Vec3>>,
}

impl Place {
    /// A place from a settlement generator's buildings, named areas and
    /// lanes. Buildings and areas whose type means nothing to life are left
    /// out; every lane is also a road.
    #[must_use]
    pub fn from_layout(
        name: &str,
        buildings: &[BuildingRef],
        areas: &[AreaRef],
        lanes: Vec<Vec<Vec3>>,
    ) -> Self {
        let mut spots: Vec<Spot> = buildings
            .iter()
            .filter_map(|b| {
                let kind = SpotKind::for_building(&b.kind)?;
                Some(Spot {
                    label: b.label.clone(),
                    ..Spot::new(kind, b.door)
                })
            })
            .collect();
        spots.extend(areas.iter().filter_map(|a| {
            let kind = SpotKind::for_area(&a.name)?;
            Some(Spot {
                kind,
                position: a.centre,
                radius: a.radius.max(1.0),
                label: a.name.clone(),
            })
        }));
        Self {
            name: name.into(),
            spots,
            roads: lanes.clone(),
            lanes,
        }
    }

    /// Indices of the spots of `kind`.
    pub fn spots_of(&self, kind: SpotKind) -> impl Iterator<Item = usize> + '_ {
        self.spots
            .iter()
            .enumerate()
            .filter(move |(_, s)| s.kind == kind)
            .map(|(i, _)| i)
    }
}

/// The lanes as a graph, for walking between spots along them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LaneGraph {
    nodes: Vec<Vec3>,
    edges: Vec<Vec<(usize, f32)>>,
}

/// Points closer than this are one junction.
const JOIN: f32 = 0.75;
/// Spots closer than this walk straight, not by the lanes.
const STRAIGHT: f32 = 12.0;

fn flat(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.z - b.z).length()
}

impl LaneGraph {
    /// The graph of a place's lanes.
    #[must_use]
    pub fn new(lanes: &[Vec<Vec3>]) -> Self {
        let mut g = Self::default();
        for lane in lanes {
            let mut previous: Option<usize> = None;
            for &p in lane {
                let node = g.node(p);
                if let Some(a) = previous
                    && a != node
                {
                    let d = flat(g.nodes[a], g.nodes[node]);
                    g.edges[a].push((node, d));
                    g.edges[node].push((a, d));
                }
                previous = Some(node);
            }
        }
        g
    }

    fn node(&mut self, p: Vec3) -> usize {
        // ponytail: linear search for a junction; a grid when lanes pass a few thousand points.
        if let Some(i) = self.nodes.iter().position(|n| flat(*n, p) < JOIN) {
            return i;
        }
        self.nodes.push(p);
        self.edges.push(Vec::new());
        self.nodes.len() - 1
    }

    fn nearest(&self, p: Vec3) -> Option<usize> {
        (0..self.nodes.len())
            .min_by(|a, b| flat(self.nodes[*a], p).total_cmp(&flat(self.nodes[*b], p)))
    }

    /// Junctions in the graph.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether there are no lanes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The way from `from` to `to`: onto the nearest lane, along the lanes,
    /// and off to `to` (which is always the last point). Near spots, or a
    /// place without lanes, go straight.
    #[must_use]
    pub fn path(&self, from: Vec3, to: Vec3) -> Vec<Vec3> {
        if self.nodes.is_empty() || flat(from, to) < STRAIGHT {
            return vec![to];
        }
        let (Some(a), Some(b)) = (self.nearest(from), self.nearest(to)) else {
            return vec![to];
        };
        if a == b {
            return vec![to];
        }
        let Some(chain) = self.dijkstra(a, b) else {
            return vec![to];
        };
        let mut path: Vec<Vec3> = chain.into_iter().map(|i| self.nodes[i]).collect();
        path.push(to);
        path
    }

    fn dijkstra(&self, a: usize, b: usize) -> Option<Vec<usize>> {
        #[derive(PartialEq)]
        struct Item(f32, usize);
        impl Eq for Item {}
        impl Ord for Item {
            fn cmp(&self, other: &Self) -> Ordering {
                other.0.total_cmp(&self.0).then(self.1.cmp(&other.1))
            }
        }
        impl PartialOrd for Item {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }
        let mut dist = vec![f32::INFINITY; self.nodes.len()];
        let mut prev = vec![usize::MAX; self.nodes.len()];
        let mut heap = BinaryHeap::new();
        dist[a] = 0.0;
        heap.push(Item(0.0, a));
        while let Some(Item(d, n)) = heap.pop() {
            if n == b {
                break;
            }
            if d > dist[n] {
                continue;
            }
            for &(m, w) in &self.edges[n] {
                let nd = d + w;
                if nd < dist[m] {
                    dist[m] = nd;
                    prev[m] = n;
                    heap.push(Item(nd, m));
                }
            }
        }
        if !dist[b].is_finite() {
            return None;
        }
        let mut chain = vec![b];
        let mut n = b;
        while n != a {
            n = prev[n];
            chain.push(n);
        }
        chain.reverse();
        Some(chain)
    }
}

/// A road as distances along it, for carts.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoadPath {
    pub points: Vec<Vec3>,
    pub cum: Vec<f32>,
}

impl RoadPath {
    pub fn new(points: &[Vec3]) -> Self {
        let mut cum = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                total += p.distance(points[i - 1]);
            }
            cum.push(total);
        }
        Self {
            points: points.to_vec(),
            cum,
        }
    }

    pub fn length(&self) -> f32 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// Point and unit tangent at `s` metres along.
    pub fn at(&self, s: f32) -> (Vec3, Vec3) {
        match self.points.len() {
            0 => return (Vec3::ZERO, Vec3::Z),
            1 => return (self.points[0], Vec3::Z),
            _ => {}
        }
        let s = s.clamp(0.0, self.length());
        let i = self
            .cum
            .partition_point(|c| *c <= s)
            .clamp(1, self.points.len() - 1);
        let (a, b) = (self.points[i - 1], self.points[i]);
        let span = (self.cum[i] - self.cum[i - 1]).max(1e-4);
        let t = ((s - self.cum[i - 1]) / span).clamp(0.0, 1.0);
        (a.lerp(b, t), (b - a).normalize_or(Vec3::Z))
    }
}
