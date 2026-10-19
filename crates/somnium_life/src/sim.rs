//! The life simulation: villagers, livestock and carts, stepped by LOD.

use crate::lod::{LodConfig, Tier};
use crate::place::{LaneGraph, Place, RoadPath, SpotKind};
use crate::routine::{Activity, Role, Routine};
use crate::{Clock, Pose, Rng};
use glam::{Vec2, Vec3};
use std::collections::HashMap;

mod person;
pub use person::{PersonAt, Schedule, ScheduleBlock, SpotRef};

/// What a villager's day follows.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Goal {
    /// Its role's routine.
    Routine,
    /// A person's schedule: this spot, doing this.
    At(usize, Activity),
    /// A person whose schedule has him elsewhere: indoors here, unseen.
    Away,
}

/// Walking pace, metres a second.
const WALK: f32 = 1.35;
/// A beast's amble.
const AMBLE: f32 = 0.55;
/// How close one cart keeps behind another going the same way.
const CART_GAP: f32 = 8.0;
/// Running from danger, metres a second.
const RUN: f32 = 3.6;

/// Seconds ahead of stepping in through a door that a near villager asks for
/// it, so the leaf is open when they reach it.
pub const DOOR_LEAD: f32 = 1.0;
/// Neighbours idling at one spot this close stop to talk.
const TALK_REACH: f32 = 4.0;
/// How far a dog wanders from its home's door, metres.
const DOG_ROAM: f32 = 8.0;
/// Seconds before a villager notices the player again.
const NOTICE_AGAIN: f64 = 60.0;
/// How far to the side of the road's line a cart drives (each way its own side).
const CART_SIDE: f32 = 0.9;

/// An agent's identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AgentId(pub u32);

/// The ground's height where agents walk (the host's terrain); `None` keeps
/// the height of the lanes and spots.
pub trait Ground {
    /// Height at a point on the ground plane.
    fn height(&self, _x: f32, _z: f32) -> Option<f32> {
        None
    }
}

/// No terrain: heights come from the spots and lanes.
pub struct Flat;

impl Ground for Flat {}

fn flat(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.z - b.z).length()
}

fn planar(v: Vec3) -> Option<Vec3> {
    Vec3::new(v.x, 0.0, v.z).try_normalize()
}

/// What a new villager is.
#[derive(Clone, Debug, PartialEq)]
pub struct VillagerDesc {
    /// Name.
    pub name: String,
    /// Trade.
    pub role: Role,
    /// Place index.
    pub place: usize,
    /// Home spot (index in the place).
    pub home: usize,
    /// Workplace spot, if the role has one here.
    pub work: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
enum Doing {
    /// At (or inside) a spot.
    At { spot: usize, shuffle_at: f64 },
    /// On the way.
    Walking {
        path: Vec<Vec3>,
        next: usize,
        to: usize,
    },
}

/// A villager.
#[derive(Clone, Debug, PartialEq)]
pub struct Villager {
    /// Identity.
    pub id: AgentId,
    /// Name.
    pub name: String,
    /// Trade.
    pub role: Role,
    /// Place index.
    pub place: usize,
    /// Home spot.
    pub home: usize,
    /// Workplace spot.
    pub work: Option<usize>,
    /// Feet.
    pub position: Vec3,
    /// Planar facing.
    pub facing: Vec3,
    /// For the animation.
    pub pose: Pose,
    /// Out in the open (not indoors).
    pub visible: bool,
    /// How much it matters now.
    pub tier: Tier,
    /// What it is about.
    pub activity: Activity,
    /// The spot it is at or bound for.
    pub target: Option<usize>,
    routine: Routine,
    doing: Doing,
    owed: f32,
    /// Sheltering from an alarm until then.
    flee_until: f64,
    /// Not to notice the player again before then.
    notice_after: f64,
    /// Hours it runs behind the routine, so a household does not leave as one.
    late: f32,
    /// It has asked for the door of the indoor spot it walks to.
    door_asked: bool,
    rng: Rng,
    /// A person's own body (a glb), drawn instead of one for the role.
    pub body: Option<String>,
    goal: Goal,
}

/// Kinds of livestock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BeastKind {
    /// Sheep.
    Sheep,
    /// Cattle.
    Cow,
    /// Goats.
    Goat,
    /// Pigs.
    Pig,
    /// Hens.
    Hen,
    /// A household's dog: it keeps about its home's door, not a pen.
    Dog,
}

impl BeastKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::Sheep,
        Self::Cow,
        Self::Goat,
        Self::Pig,
        Self::Hen,
        Self::Dog,
    ];
    /// Display names, in [`Self::ALL`] order.
    pub const NAMES: [&'static str; 6] = ["Sheep", "Cow", "Goat", "Pig", "Hen", "Dog"];

    fn pace(self) -> f32 {
        match self {
            Self::Cow => AMBLE * 0.8,
            Self::Hen => AMBLE * 0.7,
            Self::Dog => AMBLE * 1.8,
            _ => AMBLE,
        }
    }
}

/// A beast in a pen or field.
#[derive(Clone, Debug, PartialEq)]
pub struct Beast {
    /// Identity.
    pub id: AgentId,
    /// Kind.
    pub kind: BeastKind,
    /// Place index.
    pub place: usize,
    /// The pen (or field) spot it keeps to.
    pub pen: usize,
    /// Feet.
    pub position: Vec3,
    /// Planar facing.
    pub facing: Vec3,
    /// Grazing or walking.
    pub pose: Pose,
    /// How much it matters now.
    pub tier: Tier,
    to: Option<Vec3>,
    until: f64,
    owed: f32,
    rng: Rng,
    /// Bolting from an alarm until then.
    bolt_until: f64,
}

/// A cart on a road.
#[derive(Clone, Debug, PartialEq)]
pub struct Cart {
    /// Identity.
    pub id: AgentId,
    /// Place index.
    pub place: usize,
    /// Road index in the place.
    pub road: usize,
    /// Where it is.
    pub position: Vec3,
    /// Planar facing.
    pub facing: Vec3,
    /// Driving, or standing.
    pub pose: Pose,
    /// How much it matters now.
    pub tier: Tier,
    /// Metres a second.
    pub speed: f32,
    s: f32,
    dir: f32,
    wait_until: f64,
    owed: f32,
    rng: Rng,
}

/// What the last step did, for budgets and readouts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Agents in each tier.
    pub near: usize,
    /// Mid-range agents.
    pub mid: usize,
    /// Far agents.
    pub far: usize,
    /// Agents actually stepped.
    pub stepped: usize,
    /// Agents due but held over by the budget.
    pub deferred: usize,
    /// Agents asleep (beyond `LodConfig::asleep`), not stepped.
    pub asleep: usize,
}

/// A place's derived data: lanes as a graph, roads by distance.
#[derive(Clone, Debug)]
struct Placed {
    place: Place,
    lanes: LaneGraph,
    roads: Vec<RoadPath>,
}

/// Every place and everyone in them.
#[derive(Clone, Debug)]
pub struct LifeWorld {
    /// How often each tier steps.
    pub lod: LodConfig,
    /// New routes worked out a tick at most (a lane search each). When a
    /// routine changes the hour for half a town at once, the rest set off a
    /// tick or two later instead of the frame hitching.
    pub paths_per_tick: usize,
    /// How fast villagers walk and run, metres a second: match them to the
    /// game's walk and run clips (their root speed) so feet do not slide.
    pub walk_speed: f32,
    /// See `walk_speed`; running from an alarm.
    pub run_speed: f32,
    places: Vec<Placed>,
    villagers: Vec<Villager>,
    beasts: Vec<Beast>,
    carts: Vec<Cart>,
    tick: u64,
    time: f64,
    next_id: u32,
    stats: Stats,
    /// Places adopted by a stable key (see [`LifeWorld::adopt_place`]).
    keyed: HashMap<u64, usize>,
    /// Doors near villagers passed through in the last step.
    door_passes: Vec<Vec3>,
    /// People with days of their own ([`LifeWorld::adopt_person`]).
    persons: Vec<person::Person>,
    /// Hours of the world's day counted since the first step, and the hour last stepped at (persons' clock).
    person_hours: f64,
    last_hour: Option<f32>,
    /// Game hours on the road between two places (keys, the smaller first).
    travel: HashMap<(u64, u64), f32>,
}

impl LifeWorld {
    /// An empty world.
    #[must_use]
    pub fn new(lod: LodConfig) -> Self {
        Self {
            lod,
            paths_per_tick: 24,
            walk_speed: WALK,
            run_speed: RUN,
            places: Vec::new(),
            villagers: Vec::new(),
            beasts: Vec::new(),
            carts: Vec::new(),
            tick: 0,
            time: 0.0,
            next_id: 1,
            stats: Stats::default(),
            keyed: HashMap::new(),
            door_passes: Vec::new(),
            persons: Vec::new(),
            person_hours: 0.0,
            last_hour: None,
            travel: HashMap::new(),
        }
    }

    /// The doors near villagers passed through in the last step: stepping out
    /// of an indoor spot, or about [`DOOR_LEAD`] seconds from stepping in, each
    /// once. A host opens them (a game's doors, by the door's position).
    #[must_use]
    pub fn door_passes(&self) -> &[Vec3] {
        &self.door_passes
    }

    /// Add a place that comes and goes (with a streamed cell), keyed by
    /// something stable such as [`Place::key`]: the first time, it is added
    /// and, with a seed, populated; when it streams back, the place it already
    /// is comes back with its people where life left them. Returns its index
    /// and whether it was new.
    pub fn adopt_place(&mut self, key: u64, place: Place, seed: Option<u64>) -> (usize, bool) {
        if let Some(&index) = self.keyed.get(&key) {
            return (index, false);
        }
        let index = self.add_place(place);
        if let Some(seed) = seed {
            self.populate(index, seed);
        }
        self.keyed.insert(key, index);
        (index, true)
    }

    /// Danger at `at` (a fight, a fire, a beast loose): villagers within
    /// `radius` run home and stay in for `seconds`, beasts there bolt to the
    /// far side of their pen, carts there stop. Returns how many took fright.
    pub fn alarm(&mut self, at: Vec3, radius: f32, seconds: f32) -> usize {
        let until = self.time + f64::from(seconds.max(0.0));
        let near = |p: Vec3| flat(p, at) <= radius;
        let mut frightened = 0;
        for v in self.villagers.iter_mut().filter(|v| near(v.position)) {
            v.flee_until = v.flee_until.max(until);
            frightened += 1;
        }
        let places = &self.places;
        for b in self.beasts.iter_mut().filter(|b| near(b.position)) {
            b.bolt_until = b.bolt_until.max(until.min(self.time + 6.0));
            if let Some(pen) = places.get(b.place).and_then(|p| p.place.spots.get(b.pen)) {
                // The pen's side away from the danger.
                let away = planar(pen.position - at).unwrap_or(Vec3::X);
                b.to = Some(pen.position + away * pen.radius.max(0.1) * 0.8);
                b.until = until;
            }
            frightened += 1;
        }
        for c in self.carts.iter_mut().filter(|c| near(c.position)) {
            c.wait_until = c.wait_until.max(until);
            c.pose = Pose::Idle;
            frightened += 1;
        }
        frightened
    }

    /// The player passes: villagers out of doors within `reach` (not running
    /// from danger) notice them, those standing turn to face them, and each is
    /// returned at most once a minute, for the game's greeting barks.
    pub fn notice(&mut self, player: Vec3, reach: f32) -> Vec<AgentId> {
        let now = self.time;
        let mut noticed = Vec::new();
        for v in &mut self.villagers {
            if v.tier != Tier::Near
                || !v.visible
                || now < v.flee_until
                || now < v.notice_after
                || flat(v.position, player) > reach
            {
                continue;
            }
            v.notice_after = now + NOTICE_AGAIN;
            if matches!(v.doing, Doing::At { .. }) {
                v.facing = planar(player - v.position).unwrap_or(v.facing);
            }
            noticed.push(v.id);
        }
        noticed
    }

    /// Keep people and beasts out of a disc on the ground (the player, a
    /// fire): those near enough to see who stand in it step out to its edge,
    /// and those walking through it brush round it. Call after
    /// [`LifeWorld::step`]; returns how many moved.
    pub fn keep_clear(&mut self, at: Vec3, radius: f32) -> usize {
        let out = |position: &mut Vec3, facing: Vec3| {
            let off = Vec3::new(position.x - at.x, 0.0, position.z - at.z);
            let d = off.length();
            if d >= radius {
                return false;
            }
            // Dead centre: step aside, to the right of where it faces.
            let away = if d > 1e-3 {
                off / d
            } else {
                Vec3::new(-facing.z, 0.0, facing.x)
            };
            *position = Vec3::new(at.x, position.y, at.z) + away * radius;
            true
        };
        let mut moved = 0;
        for v in &mut self.villagers {
            if v.visible && v.tier == Tier::Near && out(&mut v.position, v.facing) {
                moved += 1;
            }
        }
        for b in &mut self.beasts {
            if b.tier == Tier::Near && out(&mut b.position, b.facing) {
                moved += 1;
            }
        }
        moved
    }

    /// Near villagers idling at the same outdoor spot pair off and talk,
    /// facing each other; the rest go back to what they were doing.
    fn converse(&mut self) {
        let mut at_spot: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
        for (i, v) in self.villagers.iter_mut().enumerate() {
            if v.pose == Pose::Talk {
                v.pose = v.activity.pose();
            }
            let idling = matches!(
                v.activity,
                Activity::Chores | Activity::Wander | Activity::Market | Activity::Drink
            );
            if let Doing::At { spot, .. } = v.doing
                && idling
                && v.visible
                && v.tier == Tier::Near
            {
                at_spot.entry((v.place, spot)).or_default().push(i);
            }
        }
        for group in at_spot.values() {
            for pair in group.chunks(2) {
                let [a, b] = *pair else { continue };
                let (pa, pb) = (self.villagers[a].position, self.villagers[b].position);
                if flat(pa, pb) > TALK_REACH {
                    continue;
                }
                for (me, other) in [(a, pb), (b, pa)] {
                    let v = &mut self.villagers[me];
                    v.pose = Pose::Talk;
                    v.facing = planar(other - v.position).unwrap_or(v.facing);
                }
            }
        }
    }

    /// How many places there are.
    #[must_use]
    pub fn place_count(&self) -> usize {
        self.places.len()
    }

    /// Add a place; returns its index.
    pub fn add_place(&mut self, place: Place) -> usize {
        self.places.push(Placed {
            lanes: LaneGraph::new(&place.lanes),
            roads: place.roads.iter().map(|r| RoadPath::new(r)).collect(),
            place,
        });
        self.places.len() - 1
    }

    /// A place.
    #[must_use]
    pub fn place(&self, index: usize) -> Option<&Place> {
        self.places.get(index).map(|p| &p.place)
    }

    /// Every villager.
    #[must_use]
    pub fn villagers(&self) -> &[Villager] {
        &self.villagers
    }

    /// Every beast.
    #[must_use]
    pub fn beasts(&self) -> &[Beast] {
        &self.beasts
    }

    /// Every cart.
    #[must_use]
    pub fn carts(&self) -> &[Cart] {
        &self.carts
    }

    /// The last step's counts.
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Seconds simulated.
    #[must_use]
    pub fn time(&self) -> f64 {
        self.time
    }

    fn next(&mut self) -> AgentId {
        let id = AgentId(self.next_id);
        self.next_id += 1;
        id
    }

    /// A villager, at home and indoors until its routine sends it out.
    pub fn spawn_villager(&mut self, desc: VillagerDesc) -> Option<AgentId> {
        let home = self
            .places
            .get(desc.place)?
            .place
            .spots
            .get(desc.home)?
            .position;
        let id = self.next();
        let mut rng = Rng::new(u64::from(id.0) ^ 0x51F1);
        let late = rng.range(-0.35, 0.35);
        self.villagers.push(Villager {
            id,
            name: desc.name,
            role: desc.role,
            place: desc.place,
            home: desc.home,
            work: desc.work,
            position: home,
            facing: Vec3::Z,
            pose: Pose::Inside,
            visible: false,
            tier: Tier::Near,
            activity: Activity::Sleep,
            target: None,
            routine: desc.role.routine(),
            doing: Doing::At {
                spot: desc.home,
                shuffle_at: 0.0,
            },
            owed: 0.0,
            flee_until: 0.0,
            notice_after: 0.0,
            late,
            door_asked: false,
            rng,
            body: None,
            goal: Goal::Routine,
        });
        Some(id)
    }

    /// A beast grazing in `pen` (a spot of the place).
    pub fn spawn_beast(&mut self, kind: BeastKind, place: usize, pen: usize) -> Option<AgentId> {
        let spot = self.places.get(place)?.place.spots.get(pen)?.clone();
        let id = self.next();
        let mut rng = Rng::new(u64::from(id.0) ^ 0xBEA5);
        let position = inside(&spot.position, spot.radius, &mut rng);
        let until = f64::from(rng.range(0.0, 8.0));
        self.beasts.push(Beast {
            id,
            kind,
            place,
            pen,
            position,
            facing: Vec3::new(rng.range(-1.0, 1.0), 0.0, rng.range(-1.0, 1.0))
                .normalize_or(Vec3::Z),
            pose: Pose::Graze,
            tier: Tier::Near,
            to: None,
            until,
            owed: 0.0,
            rng,
            bolt_until: 0.0,
        });
        Some(id)
    }

    /// A cart on road `road` of `place`, starting `start` (0 to 1) along it.
    pub fn spawn_cart(&mut self, place: usize, road: usize, start: f32) -> Option<AgentId> {
        let path = self.places.get(place)?.roads.get(road)?;
        let s = path.length() * start.clamp(0.0, 1.0);
        let (position, tangent) = path.at(s);
        let id = self.next();
        let mut rng = Rng::new(u64::from(id.0) ^ 0xCA27);
        let speed = rng.range(1.9, 2.5);
        self.carts.push(Cart {
            id,
            place,
            road,
            position,
            facing: tangent,
            pose: Pose::Drive,
            tier: Tier::Near,
            speed,
            s,
            dir: 1.0,
            wait_until: 0.0,
            owed: 0.0,
            rng,
        });
        Some(id)
    }

    /// House a household in every home of `place`, a trade for each by the
    /// workplaces there (a smith for each workshop, a keeper for the inn, a
    /// priest for the chapel, herders for the pens, the rest farmers or
    /// labourers, with elders and children), livestock in every pen, a dog for
    /// about one household in three, and carts
    /// on the longer roads. Returns everyone spawned.
    pub fn populate(&mut self, place: usize, seed: u64) -> Vec<AgentId> {
        let Some(p) = self.places.get(place).map(|p| p.place.clone()) else {
            return Vec::new();
        };
        let mut rng = Rng::new(seed ^ 0x9E3779B97F4A7C15);
        let of = |k: SpotKind| p.spots_of(k).collect::<Vec<_>>();
        let (homes, works, fields, pens, inns, chapels) = (
            of(SpotKind::Home),
            of(SpotKind::Work),
            of(SpotKind::Field),
            of(SpotKind::Pen),
            of(SpotKind::Inn),
            of(SpotKind::Chapel),
        );
        let nearest = |from: usize, among: &[usize]| {
            among.iter().copied().min_by(|a, b| {
                flat(p.spots[*a].position, p.spots[from].position)
                    .total_cmp(&flat(p.spots[*b].position, p.spots[from].position))
            })
        };
        // Trades still wanted, each with its workplace.
        let mut posts: Vec<(Role, usize)> = Vec::new();
        posts.extend(works.iter().map(|w| (Role::Smith, *w)));
        posts.extend(inns.iter().map(|w| (Role::Keeper, *w)));
        posts.extend(chapels.iter().map(|w| (Role::Priest, *w)));
        posts.extend(pens.iter().map(|w| (Role::Herder, *w)));
        let mut out = Vec::new();
        for (n, &home) in homes.iter().enumerate() {
            let size = 1 + (rng.unit() * 3.99) as usize;
            for k in 0..size {
                let (role, work) = if k == 0 {
                    match posts.pop() {
                        Some(post) => (post.0, Some(post.1)),
                        None if !fields.is_empty() => (Role::Farmer, nearest(home, &fields)),
                        None => (Role::Labourer, nearest(home, &works)),
                    }
                } else {
                    let r = rng.unit();
                    if r < 0.3 {
                        (Role::Child, None)
                    } else if r < 0.5 {
                        (Role::Elder, None)
                    } else if !fields.is_empty() {
                        (Role::Farmer, nearest(home, &fields))
                    } else {
                        (Role::Labourer, nearest(home, &works))
                    }
                };
                let name = format!(
                    "{} {}-{} ({})",
                    p.name,
                    n + 1,
                    k + 1,
                    Role::NAMES[role as usize]
                );
                if let Some(id) = self.spawn_villager(VillagerDesc {
                    name,
                    role,
                    place,
                    home,
                    work,
                }) {
                    out.push(id);
                }
            }
        }
        for &pen in &pens {
            let kind = [
                BeastKind::Sheep,
                BeastKind::Cow,
                BeastKind::Goat,
                BeastKind::Pig,
            ][(rng.unit() * 3.99) as usize];
            let count = 4 + (rng.unit() * 4.99) as usize;
            for _ in 0..count {
                if let Some(id) = self.spawn_beast(kind, place, pen) {
                    out.push(id);
                }
            }
        }
        // About one household in three keeps a dog.
        for &home in &homes {
            if rng.unit() < 0.35
                && let Some(id) = self.spawn_beast(BeastKind::Dog, place, home)
            {
                out.push(id);
            }
        }
        let lengths: Vec<f32> = self.places[place]
            .roads
            .iter()
            .map(RoadPath::length)
            .collect();
        for (road, length) in lengths.into_iter().enumerate() {
            let carts = if length > 200.0 {
                2
            } else {
                usize::from(length > 40.0)
            };
            for c in 0..carts {
                if let Some(id) = self.spawn_cart(place, road, c as f32 * 0.5 + 0.1) {
                    out.push(id);
                }
            }
        }
        out
    }

    /// Advance everyone by `dt` seconds of real time at `clock`. Agents near a
    /// point in `focus` step now; further out, on their stride with the time
    /// they are owed (see [`LodConfig`]). When more are due than the budget
    /// allows, those owed the most go first.
    pub fn step(&mut self, dt: f32, clock: Clock, focus: &[Vec3], ground: &impl Ground) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.tick += 1;
        self.time += f64::from(dt);
        self.follow_schedules(clock);
        let (tick, now, lod) = (self.tick, self.time, self.lod);
        let mut stats = Stats::default();

        // Who steps: tiers, owed time, strides, then the budget by most owed.
        #[derive(Clone, Copy)]
        enum Who {
            Villager(usize),
            Beast(usize),
            Cart(usize),
        }
        let mut near: Vec<Who> = Vec::new();
        let mut waiting: Vec<(f32, Who)> = Vec::new();
        let mut sort = |tier: Tier, id: AgentId, owed: &mut f32, who: Who, stats: &mut Stats| {
            match tier {
                Tier::Near => stats.near += 1,
                Tier::Mid => stats.mid += 1,
                Tier::Far => stats.far += 1,
                Tier::Asleep => stats.asleep += 1,
            }
            *owed += dt;
            if !lod.due(tier, tick, id.0) {
                return;
            }
            if tier == Tier::Near {
                near.push(who);
            } else {
                waiting.push((*owed, who));
            }
        };
        for (i, v) in self.villagers.iter_mut().enumerate() {
            v.tier = lod.tier(v.position, focus);
            // A person (the off-screen protagonist) is never asleep: his day goes on.
            if v.goal != Goal::Routine && v.tier == Tier::Asleep {
                v.tier = Tier::Far;
            }
            sort(v.tier, v.id, &mut v.owed, Who::Villager(i), &mut stats);
        }
        for (i, b) in self.beasts.iter_mut().enumerate() {
            b.tier = lod.tier(b.position, focus);
            sort(b.tier, b.id, &mut b.owed, Who::Beast(i), &mut stats);
        }
        for (i, c) in self.carts.iter_mut().enumerate() {
            c.tier = lod.tier(c.position, focus);
            sort(c.tier, c.id, &mut c.owed, Who::Cart(i), &mut stats);
        }
        if waiting.len() > lod.budget {
            waiting.sort_by(|a, b| b.0.total_cmp(&a.0));
            stats.deferred = waiting.len() - lod.budget;
            waiting.truncate(lod.budget);
        }
        let chosen: Vec<Who> = near
            .into_iter()
            .chain(waiting.into_iter().map(|w| w.1))
            .collect();
        stats.stepped = chosen.len();

        // Carts keep their distance from one ahead going their way, and turn
        // short of one standing at the end they are bound for. Only the carts
        // stepping now look, and only along their own road.
        let places = &self.places;
        let mut lanes: HashMap<usize, (bool, f32)> = HashMap::new();
        let stepping: Vec<usize> = chosen
            .iter()
            .filter_map(|w| match w {
                Who::Cart(i) => Some(*i),
                _ => None,
            })
            .collect();
        if !stepping.is_empty() {
            let mut by_road: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
            for (j, o) in self.carts.iter().enumerate() {
                by_road.entry((o.place, o.road)).or_default().push(j);
            }
            for i in stepping {
                let c = &self.carts[i];
                let others = || {
                    by_road[&(c.place, c.road)]
                        .iter()
                        .map(|j| &self.carts[*j])
                        .filter(|o| o.id != c.id)
                };
                let blocked = others().any(|o| {
                    let gap = (o.s - c.s) * c.dir;
                    o.dir == c.dir && gap > 0.0 && gap < CART_GAP
                });
                let length = places
                    .get(c.place)
                    .and_then(|p| p.roads.get(c.road))
                    .map_or(0.0, RoadPath::length);
                let end = if c.dir > 0.0 { length } else { 0.0 };
                let taken = others().any(|o| (o.s - end).abs() < 1.0 && o.wait_until > now);
                lanes.insert(
                    i,
                    (blocked, if taken { end - c.dir * CART_GAP } else { end }),
                );
            }
        }
        let mut paths = self.paths_per_tick;
        let pace = (self.walk_speed, self.run_speed);
        let mut doors = std::mem::take(&mut self.door_passes);
        doors.clear();
        for who in chosen {
            match who {
                Who::Villager(i) => {
                    let v = &mut self.villagers[i];
                    let owed = std::mem::take(&mut v.owed);
                    if let Some(p) = places.get(v.place) {
                        v.advance(p, clock, ground, owed, now, &mut paths, pace, &mut doors);
                    }
                }
                Who::Beast(i) => {
                    let b = &mut self.beasts[i];
                    let owed = std::mem::take(&mut b.owed);
                    if let Some(spot) = places.get(b.place).and_then(|p| p.place.spots.get(b.pen)) {
                        // A dog's "pen" is its home's door: it roams round it.
                        let radius = if b.kind == BeastKind::Dog {
                            spot.radius.max(DOG_ROAM)
                        } else {
                            spot.radius
                        };
                        b.advance(spot.position, radius, ground, owed, now);
                    }
                }
                Who::Cart(i) => {
                    let c = &mut self.carts[i];
                    let owed = std::mem::take(&mut c.owed);
                    if let Some(road) = places.get(c.place).and_then(|p| p.roads.get(c.road)) {
                        c.advance(road, clock, lanes[&i], ground, owed, now);
                    }
                }
            }
        }
        self.door_passes = doors;
        self.converse();
        self.stats = stats;
    }
}

/// A point inside a disc, for pens and outdoor spots.
fn inside(centre: &Vec3, radius: f32, rng: &mut Rng) -> Vec3 {
    let a = rng.unit() * std::f32::consts::TAU;
    let r = radius.max(0.1) * 0.85 * rng.unit().sqrt();
    *centre + Vec3::new(a.cos() * r, 0.0, a.sin() * r)
}

fn settle(position: &mut Vec3, ground: &impl Ground) {
    if let Some(y) = ground.height(position.x, position.z) {
        position.y = y;
    }
}

/// Move along `path` from `next` by up to `distance`; returns the distance
/// left over once at the end (or `None` while still on the way).
fn walk_path(
    position: &mut Vec3,
    facing: &mut Vec3,
    path: &[Vec3],
    next: &mut usize,
    mut distance: f32,
) -> Option<f32> {
    while *next < path.len() {
        let to = path[*next];
        let gap = position.distance(to);
        if let Some(dir) = planar(to - *position) {
            *facing = dir;
        }
        if gap <= distance {
            *position = to;
            distance -= gap;
            *next += 1;
        } else {
            *position += (to - *position) / gap.max(1e-6) * distance;
            return None;
        }
    }
    Some(distance)
}

impl Villager {
    fn choose(&mut self, place: &Place, activity: Activity) -> Option<usize> {
        if let Goal::At(spot, _) = self.goal {
            return Some(spot);
        }
        let wants = activity.spots();
        if let Some(w) = self.work
            && let Some(spot) = place.spots.get(w)
            && (activity == Activity::Work || wants.contains(&spot.kind))
        {
            return Some(w);
        }
        for kind in wants {
            if *kind == SpotKind::Home {
                return Some(self.home);
            }
            let mut near: Vec<usize> = place.spots_of(*kind).collect();
            if near.is_empty() {
                continue;
            }
            let from = self.position;
            near.sort_by(|a, b| {
                flat(place.spots[*a].position, from)
                    .total_cmp(&flat(place.spots[*b].position, from))
            });
            near.truncate(3);
            let pick = (self.rng.unit() * near.len() as f32) as usize;
            return near.get(pick.min(near.len() - 1)).copied();
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn advance(
        &mut self,
        placed: &Placed,
        clock: Clock,
        ground: &impl Ground,
        dt: f32,
        now: f64,
        paths: &mut usize,
        (walk, run): (f32, f32),
        doors: &mut Vec<Vec3>,
    ) {
        // Nobody sees a door open for anyone but the near.
        let mut ask = |at: Vec3, near: bool| {
            if near && !doors.contains(&at) {
                doors.push(at);
            }
        };
        let near = self.tier == Tier::Near;
        let place = &placed.place;
        if self.goal == Goal::Away {
            self.visible = false;
            self.pose = Pose::Inside;
            return;
        }
        let fleeing = now < self.flee_until;
        let wanted = match self.goal {
            _ if fleeing => Activity::Shelter,
            Goal::At(_, activity) => activity,
            _ if clock.lid_fallen() => Activity::Sleep,
            _ => self.routine.at(clock.hour - self.late),
        };
        let moved = matches!(self.goal, Goal::At(spot, _) if self.target != Some(spot));
        // Out of routes this tick: carry on as before and look again next tick.
        if (wanted != self.activity || self.target.is_none() || moved) && *paths > 0 {
            self.activity = wanted;
            let target = self.choose(place, wanted);
            if target != self.target
                && let Some(to) = target
            {
                *paths -= 1;
                // Out of the door it was behind.
                if !self.visible {
                    ask(self.position, near);
                }
                self.go(placed, to);
                self.door_asked = false;
            }
            self.target = target;
        }
        let pace = if fleeing { run } else { walk };
        let mut walked = dt * pace;
        loop {
            match &mut self.doing {
                Doing::Walking { path, next, to } => {
                    let to = *to;
                    let left = walk_path(&mut self.position, &mut self.facing, path, next, walked);
                    settle(&mut self.position, ground);
                    self.pose = if fleeing { Pose::Run } else { Pose::Walk };
                    self.visible = true;
                    // In through a door: asked for a little before it is reached
                    // (or on arriving, after a long step).
                    if !self.door_asked
                        && let Some(s) = place.spots.get(to).filter(|s| s.kind.indoor())
                        && (left.is_some() || flat(self.position, s.position) <= pace * DOOR_LEAD)
                    {
                        ask(s.position, near);
                        self.door_asked = true;
                    }
                    let Some(left) = left else {
                        break;
                    };
                    walked = left;
                    self.arrive(place, to, now);
                }
                Doing::At { spot, shuffle_at } => {
                    let spot = *spot;
                    let outdoors = place.spots.get(spot).is_some_and(|s| !s.kind.indoor());
                    if outdoors && now >= *shuffle_at && walked > 0.0 {
                        // A step or two about the spot, now and then.
                        let s = &place.spots[spot];
                        let slot = inside(&s.position, s.radius, &mut self.rng);
                        self.doing = Doing::Walking {
                            path: vec![slot],
                            next: 0,
                            to: spot,
                        };
                        continue;
                    }
                    break;
                }
            }
        }
    }

    /// Set off for spot `to`: out of the door if indoors, then by the lanes.
    fn go(&mut self, placed: &Placed, to: usize) {
        let Some(spot) = placed.place.spots.get(to) else {
            return;
        };
        let end = if spot.kind.indoor() {
            spot.position
        } else {
            inside(&spot.position, spot.radius, &mut self.rng)
        };
        if !self.visible {
            // Out of the door it was behind.
            self.visible = true;
        }
        self.doing = Doing::Walking {
            path: placed.lanes.path(self.position, end),
            next: 0,
            to,
        };
    }

    fn arrive(&mut self, place: &Place, spot: usize, now: f64) {
        let indoor = place.spots.get(spot).is_some_and(|s| s.kind.indoor());
        if indoor {
            self.visible = false;
            self.pose = Pose::Inside;
        } else {
            self.visible = true;
            self.pose = self.activity.pose();
            if let Some(s) = place.spots.get(spot)
                && let Some(dir) = planar(s.position - self.position)
            {
                self.facing = dir;
            }
        }
        self.doing = Doing::At {
            spot,
            shuffle_at: now + f64::from(self.rng.range(15.0, 60.0)),
        };
    }
}

impl Beast {
    fn advance(&mut self, centre: Vec3, radius: f32, ground: &impl Ground, dt: f32, now: f64) {
        let bolting = now < self.bolt_until;
        match self.to {
            Some(to) => {
                let mut next = 0;
                let pace = self.kind.pace() * if bolting { 4.0 } else { 1.0 };
                let left = walk_path(
                    &mut self.position,
                    &mut self.facing,
                    &[to],
                    &mut next,
                    dt * pace,
                );
                self.pose = if bolting { Pose::Run } else { Pose::Walk };
                if left.is_some() {
                    self.to = None;
                    self.pose = Pose::Graze;
                    self.until = now + f64::from(self.rng.range(4.0, 14.0));
                }
            }
            None if now >= self.until => {
                self.to = Some(inside(&centre, radius, &mut self.rng));
            }
            None => self.pose = Pose::Graze,
        }
        settle(&mut self.position, ground);
    }
}

impl Cart {
    fn advance(
        &mut self,
        road: &RoadPath,
        clock: Clock,
        (blocked, end): (bool, f32),
        ground: &impl Ground,
        dt: f32,
        now: f64,
    ) {
        let hour = clock.hour.rem_euclid(24.0);
        let working = !clock.lid_fallen() && (6.0..19.5).contains(&hour);
        if !working || now < self.wait_until || blocked {
            self.pose = Pose::Idle;
            return;
        }
        self.pose = Pose::Drive;
        self.s += self.dir * self.speed * dt;
        if (self.s - end) * self.dir >= 0.0 {
            self.s = end.clamp(0.0, road.length());
            self.dir = -self.dir;
            self.wait_until = now + f64::from(self.rng.range(15.0, 40.0));
        }
        let (point, tangent) = road.at(self.s);
        let facing = tangent * self.dir;
        let side = Vec3::new(facing.z, 0.0, -facing.x).normalize_or_zero();
        self.position = point + side * CART_SIDE;
        self.facing = planar(facing).unwrap_or(self.facing);
        settle(&mut self.position, ground);
    }
}
