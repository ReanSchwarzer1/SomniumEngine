//! Combat brains on the engine's behaviour trees, perception and navigation.
//!
//! A brain is a `somnium_ai::behavior::BehaviorInstance` whose native tasks
//! are named `combat.*` (so a designer can author a tree in the shared graph
//! editor with **Native Task** nodes and the same names), a
//! `somnium_ai::perception` observer and stimulus memory, and optionally a
//! corridor through `somnium_ai::navigation::NavWorld` when the host supplies a navmesh.
//!
//! Three layers, as in most action games:
//!
//! 1. **Perception** at ten hertz: sight in a cone blocked by the world and by
//!    spell walls (Night Fang's darkness shortens it), hearing of swings,
//!    shots and verses, and being struck. The strongest hostile becomes the
//!    target.
//! 2. **Decision**: the tree. It is reset when the situation changes (a
//!    target is found or lost, fear sets in), since the engine's composites
//!    resume their running child rather than re-checking priorities.
//! 3. **Reflex**, every step: a skilled fighter who sees a blow winding up at
//!    them raises a guard on the beat (a parry) or dodges; a missile in flight
//!    can be side-stepped.
//!
//! Coordination is a [`Tactics`] board per target: a few attack tokens, so
//! three raiders do not all swing at once, and slots around the target, so
//! the ones waiting circle and the flankers go behind.
//!
//! | Task | Succeeds when | Notes |
//! |---|---|---|
//! | `combat.has_target` | a hostile is known | |
//! | `combat.has_lead` | something was heard or last seen | |
//! | `combat.should_flee` | health and morale have broken | |
//! | `combat.too_close` | the target is inside the near edge of the band | ranged and casters |
//! | `combat.alert_allies` | always | shouts once per target |
//! | `combat.engage` | inside the preferred band | runs while closing |
//! | `combat.flank` | at a slot behind the target | |
//! | `combat.attack` | a chain (or a shot) is finished | waits for a token; circles without one |
//! | `combat.shoot` | a shot is away | the attack task for bows and rifles |
//! | `combat.keep_distance` | back outside the near edge | |
//! | `combat.cast` | a spell is released | fails when nothing can be cast |
//! | `combat.heal_ally` | a hurt ally was healed | fails when nobody needs it |
//! | `combat.flee` | far from every threat | |
//! | `combat.investigate` | reached the lead | |
//! | `combat.follow_leader` | near the leader | companions |
//! | `combat.hold` | at the held point | companions under orders |
//! | `combat.idle` | always | returns home |
use crate::geometry::{flat, planar_dir};
use crate::spell::{Address, Bearing, SpellRequest};
use crate::weapon::WeaponKind;
use crate::world::{Action, CombatEvent, CombatWorld, Command, Phase, Press};
use crate::{CombatQuery, CombatantId, Rng, TeamId};
use glam::Vec3;
use somnium_ai::behavior::{
    BehaviorInstance, BehaviorTree, Blackboard, Node, Status, Task, TaskHost,
};
use somnium_ai::navigation::NavWorld;
use somnium_ai::perception::{Observer, Sense, Stimulus, StimulusMemory};
use std::collections::BTreeMap;

/// Seconds between perception updates.
pub const THINK_INTERVAL: f32 = 0.1;
/// Seconds a stimulus is remembered.
pub const MEMORY_SECONDS: f64 = 10.0;

/// Fighting styles.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum Archetype {
    /// Sword or axe in close: engages and swings.
    Duelist,
    /// Keeps you on the point at reach.
    Spearman,
    /// Heavy hitter that walks through blows.
    Brute,
    /// Knife: circles and strikes from behind.
    Skirmisher,
    /// Keeps its distance and looses arrows.
    Archer,
    /// Keeps its distance, fires, reloads.
    Rifleman,
    /// Keeps its distance and casts.
    Caster,
    /// Follows a leader, fights the leader's enemies, takes orders.
    Companion,
}

impl Archetype {
    /// Every archetype, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::Duelist,
        Self::Spearman,
        Self::Brute,
        Self::Skirmisher,
        Self::Archer,
        Self::Rifleman,
        Self::Caster,
        Self::Companion,
    ];

    /// Display names, in [`Self::ALL`] order.
    pub const NAMES: [&'static str; 8] = [
        "Duelist",
        "Spearman",
        "Brute",
        "Skirmisher",
        "Archer",
        "Rifleman",
        "Caster",
        "Companion",
    ];

    /// The default profile.
    #[must_use]
    pub fn profile(self) -> Profile {
        let base = Profile {
            band: [1.2, 2.2],
            aggression: 0.6,
            skill: 0.4,
            flee_below: 0.25,
            morale: 0.8,
            alert_radius: 18.0,
            sight: 22.0,
            fov: 140.0,
            hearing: 18.0,
            flanker: false,
            combo: 2,
            spells: Vec::new(),
            leash: 0.0,
        };
        match self {
            Self::Duelist => Profile {
                skill: 0.5,
                combo: 3,
                ..base
            },
            Self::Spearman => Profile {
                band: [2.0, 3.0],
                skill: 0.45,
                ..base
            },
            Self::Brute => Profile {
                aggression: 0.85,
                skill: 0.15,
                flee_below: 0.1,
                combo: 2,
                ..base
            },
            Self::Skirmisher => Profile {
                band: [0.9, 1.6],
                aggression: 0.75,
                skill: 0.55,
                flee_below: 0.35,
                flanker: true,
                combo: 3,
                ..base
            },
            Self::Archer => Profile {
                band: [8.0, 24.0],
                skill: 0.35,
                flee_below: 0.3,
                sight: 32.0,
                ..base
            },
            Self::Rifleman => Profile {
                band: [12.0, 32.0],
                skill: 0.3,
                flee_below: 0.3,
                sight: 40.0,
                ..base
            },
            Self::Caster => Profile {
                band: [7.0, 18.0],
                skill: 0.4,
                flee_below: 0.3,
                spells: vec![
                    "ember_bolt".into(),
                    "fireburst".into(),
                    "close_wound".into(),
                ],
                ..base
            },
            Self::Companion => Profile {
                skill: 0.6,
                flee_below: 0.0,
                morale: 1.0,
                combo: 3,
                leash: 9.0,
                ..base
            },
        }
    }

    /// The default tree.
    #[must_use]
    pub fn tree(self) -> BehaviorTree {
        use Tree::{Do, Sel, Seq};
        let flee = || {
            Seq(vec![
                Do("combat.should_flee"),
                Do("combat.alert_allies"),
                Do("combat.flee"),
            ])
        };
        let search = || Seq(vec![Do("combat.has_lead"), Do("combat.investigate")]);
        let fight = match self {
            Self::Duelist | Self::Spearman | Self::Brute => Seq(vec![
                Do("combat.has_target"),
                Do("combat.alert_allies"),
                Do("combat.engage"),
                Do("combat.attack"),
            ]),
            Self::Skirmisher => Seq(vec![
                Do("combat.has_target"),
                Do("combat.alert_allies"),
                Do("combat.flank"),
                Do("combat.attack"),
            ]),
            Self::Archer | Self::Rifleman => Seq(vec![
                Do("combat.has_target"),
                Do("combat.alert_allies"),
                Sel(vec![
                    Seq(vec![Do("combat.too_close"), Do("combat.keep_distance")]),
                    Do("combat.shoot"),
                    Do("combat.engage"),
                ]),
            ]),
            Self::Caster => Seq(vec![
                Do("combat.has_target"),
                Do("combat.alert_allies"),
                Sel(vec![
                    Seq(vec![Do("combat.too_close"), Do("combat.keep_distance")]),
                    Do("combat.heal_ally"),
                    Do("combat.cast"),
                    Do("combat.engage"),
                ]),
            ]),
            Self::Companion => {
                return build(Sel(vec![
                    Do("combat.hold"),
                    Seq(vec![
                        Do("combat.has_target"),
                        Sel(vec![
                            Do("combat.heal_ally"),
                            Seq(vec![Do("combat.engage"), Do("combat.attack")]),
                        ]),
                    ]),
                    Do("combat.follow_leader"),
                ]));
            }
        };
        build(Sel(vec![flee(), fight, search(), Do("combat.idle")]))
    }
}

/// A nested description of a tree, flattened by [`build`].
#[derive(Clone, Debug)]
pub enum Tree {
    /// All in order.
    Seq(Vec<Tree>),
    /// First that succeeds.
    Sel(Vec<Tree>),
    /// A native task.
    Do(&'static str),
}

/// Flatten a [`Tree`] into a validated behaviour tree (root first).
#[must_use]
pub fn build(tree: Tree) -> BehaviorTree {
    fn push(t: &Tree, nodes: &mut Vec<Node>) -> usize {
        let index = nodes.len();
        nodes.push(Node::Task(Task::Wait { seconds: 0.0 }));
        let node = match t {
            Tree::Do(name) => Node::Task(Task::Native((*name).to_string())),
            Tree::Seq(children) | Tree::Sel(children) => {
                let ids = children.iter().map(|c| push(c, nodes)).collect();
                if matches!(t, Tree::Seq(_)) {
                    Node::Sequence(ids)
                } else {
                    Node::Selector(ids)
                }
            }
        };
        nodes[index] = node;
        index
    }
    let mut nodes = Vec::new();
    push(&tree, &mut nodes);
    BehaviorTree::new(0, nodes).expect("built trees are valid")
}

/// Tunables of a style.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Profile {
    /// Preferred distance to the target, near and far edge.
    pub band: [f32; 2],
    /// 0 to 1: how readily it commits.
    pub aggression: f32,
    /// 0 to 1: how often it parries or dodges a blow it sees coming.
    pub skill: f32,
    /// Health fraction below which fear can take it.
    pub flee_below: f32,
    /// Starting morale, 0 to 1.
    pub morale: f32,
    /// How far its shout carries.
    pub alert_radius: f32,
    /// Sight range.
    pub sight: f32,
    /// Full sight cone, degrees.
    pub fov: f32,
    /// Hearing range.
    pub hearing: f32,
    /// Goes for the back.
    pub flanker: bool,
    /// Longest chain it swings.
    pub combo: u8,
    /// Spells it knows to cast (asset ids).
    pub spells: Vec<String>,
    /// Companions: how far it strays from the leader.
    pub leash: f32,
}

/// A companion's standing order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Order {
    /// Follow and fight the leader's enemies.
    Follow,
    /// Stay here.
    Hold(Vec3),
    /// Go for this one.
    Attack(CombatantId),
    /// Break off and come back.
    FallBack,
}

/// What a combatant's AI is made of.
#[derive(Clone, Debug)]
pub struct BrainDesc {
    /// Style.
    pub archetype: Archetype,
    /// Override the style's profile.
    pub profile: Option<Profile>,
    /// Override the style's tree (an authored graph).
    pub tree: Option<BehaviorTree>,
    /// Leader, for companions.
    pub leader: Option<CombatantId>,
    /// Where it returns to when idle.
    pub home: Option<Vec3>,
}

impl BrainDesc {
    /// A brain of a style with its defaults.
    #[must_use]
    pub fn new(archetype: Archetype) -> Self {
        Self {
            archetype,
            profile: None,
            tree: None,
            leader: None,
            home: None,
        }
    }
}

/// Everything a brain knows besides its tree.
#[derive(Clone, Debug)]
pub struct Mind {
    /// Style.
    pub archetype: Archetype,
    /// Tunables.
    pub profile: Profile,
    /// Current target.
    pub target: Option<CombatantId>,
    /// Where something was last noticed.
    pub lead: Option<Vec3>,
    /// 0 to 1.
    pub morale: f32,
    /// Has broken and run.
    pub fleeing: bool,
    /// Leader (companions).
    pub leader: Option<CombatantId>,
    /// Order (companions).
    pub order: Order,
    /// Home.
    pub home: Option<Vec3>,
    /// What it is doing, for debugging and Details.
    pub doing: &'static str,
    memory: StimulusMemory,
    /// Corners, each with whether it is reached over an off-mesh link.
    route: Vec<(Vec3, bool)>,
    route_goal: Option<Vec3>,
    route_age: f32,
    think: f32,
    situation: u8,
    shouted_for: Option<CombatantId>,
    swings_left: u8,
    attacking: bool,
    casting: bool,
    guard_until: f64,
    reacted: Vec<(CombatantId, u32)>,
    fled_reported: bool,
    ranged: bool,
    reach: f32,
    charges: bool,
}

/// A combatant's AI.
#[derive(Clone, Debug)]
pub struct Brain {
    instance: BehaviorInstance,
    /// What it knows.
    pub mind: Mind,
}

impl Brain {
    /// A brain from its description, for the combatant `id`.
    #[must_use]
    pub fn new(desc: BrainDesc, id: CombatantId) -> Self {
        let _ = id;
        let profile = desc.profile.unwrap_or_else(|| desc.archetype.profile());
        let tree = desc.tree.unwrap_or_else(|| desc.archetype.tree());
        Self {
            instance: BehaviorInstance::new(tree),
            mind: Mind {
                archetype: desc.archetype,
                morale: profile.morale,
                profile,
                target: None,
                lead: None,
                fleeing: false,
                leader: desc.leader,
                order: Order::Follow,
                home: desc.home,
                doing: "Idle",
                memory: StimulusMemory::new(16, MEMORY_SECONDS).expect("valid policy"),
                route: Vec::new(),
                route_goal: None,
                route_age: 0.0,
                think: 0.0,
                situation: u8::MAX,
                shouted_for: None,
                swings_left: 0,
                attacking: false,
                casting: false,
                guard_until: f64::NEG_INFINITY,
                reacted: Vec::new(),
                fled_reported: false,
                ranged: false,
                reach: 2.0,
                charges: false,
            },
        }
    }

    /// The blackboard, for inspection.
    #[must_use]
    pub fn blackboard(&self) -> &Blackboard {
        &self.instance.blackboard
    }
}

/// Attack tokens and positions around each target.
#[derive(Clone, Debug, Default)]
pub struct Tactics {
    tokens: BTreeMap<CombatantId, Vec<(CombatantId, f64)>>,
    slots: BTreeMap<CombatantId, Vec<(CombatantId, u8)>>,
}

/// Slots around a target.
pub const SLOTS: u8 = 8;

impl Tactics {
    /// Ask to swing at `target`. Granted while fewer than `max` hold one.
    pub fn request(
        &mut self,
        target: CombatantId,
        attacker: CombatantId,
        max: usize,
        now: f64,
    ) -> bool {
        let holders = self.tokens.entry(target).or_default();
        if holders.iter().any(|(a, _)| *a == attacker) {
            return true;
        }
        if holders.len() < max.max(1) {
            holders.push((attacker, now));
            true
        } else {
            false
        }
    }

    /// Give a token back.
    pub fn release(&mut self, target: CombatantId, attacker: CombatantId) {
        if let Some(h) = self.tokens.get_mut(&target) {
            h.retain(|(a, _)| *a != attacker);
        }
    }

    /// How many hold a token on `target`.
    #[must_use]
    pub fn holders(&self, target: CombatantId) -> usize {
        self.tokens.get(&target).map_or(0, Vec::len)
    }

    /// The point `attacker` should stand at around `target`: the free slot
    /// nearest it, or for a flanker the one most behind the target.
    #[allow(clippy::too_many_arguments)]
    pub fn slot(
        &mut self,
        target: CombatantId,
        attacker: CombatantId,
        target_position: Vec3,
        target_facing: Vec3,
        from: Vec3,
        radius: f32,
        flank: bool,
    ) -> Vec3 {
        let taken = self.slots.entry(target).or_default();
        let point = |k: u8| {
            let a = f32::from(k) * std::f32::consts::TAU / f32::from(SLOTS);
            target_position + Vec3::new(a.cos(), 0.0, a.sin()) * radius
        };
        let mine = taken.iter().find(|(a, _)| *a == attacker).map(|(_, k)| *k);
        let score = |k: u8| {
            let p = point(k);
            let d = flat(p - from).length();
            if flank {
                let dir = planar_dir(p - target_position, Vec3::Z);
                target_facing.dot(dir) * 4.0 + d * 0.1
            } else {
                d
            }
        };
        let best = (0..SLOTS)
            .filter(|k| !taken.iter().any(|(a, s)| s == k && *a != attacker))
            .min_by(|a, b| score(*a).total_cmp(&score(*b)));
        let chosen = match (mine, best) {
            (Some(m), Some(b)) if score(b) + 0.5 < score(m) => b,
            (Some(m), _) => m,
            (None, Some(b)) => b,
            (None, None) => 0,
        };
        taken.retain(|(a, _)| *a != attacker);
        taken.push((attacker, chosen));
        point(chosen)
    }

    /// Forget everything about `id` (dead, gone).
    pub fn forget(&mut self, id: CombatantId) {
        self.tokens.remove(&id);
        self.slots.remove(&id);
        for h in self.tokens.values_mut() {
            h.retain(|(a, _)| *a != id);
        }
        for s in self.slots.values_mut() {
            s.retain(|(a, _)| *a != id);
        }
    }

    /// Expire stale tokens (a holder that never swung) and the dead.
    pub fn update(&mut self, now: f64, _ids: &[CombatantId], alive: impl Fn(CombatantId) -> bool) {
        for h in self.tokens.values_mut() {
            h.retain(|(a, at)| alive(*a) && now - *at < 3.0);
        }
        self.tokens.retain(|t, h| alive(*t) && !h.is_empty());
        self.slots.retain(|t, _| alive(*t));
    }
}

/// What a brain can see of a body, copied out so the tree can run while the
/// world is borrowed for writing commands.
#[derive(Clone, Copy, Debug)]
struct Seen {
    id: CombatantId,
    team: TeamId,
    position: Vec3,
    chest: Vec3,
    eye: Vec3,
    facing: Vec3,
    velocity: Vec3,
    radius: f32,
    health: f32,
    concealed: bool,
    guarding: bool,
    /// Winding up: seconds left, planar direction, reach, a serial for "this swing".
    windup: Option<(f32, Vec3, f32, u32)>,
    busy: bool,
    mounted: bool,
}

fn reach_of(def: &crate::weapon::MoveDef) -> f32 {
    use crate::weapon::HitShape;
    match def.shape {
        HitShape::Arc { radius, .. } => radius,
        HitShape::Thrust { length, .. } => length,
        HitShape::Sphere { forward, radius } => forward + radius,
        HitShape::Blade { .. } => 2.0,
    }
}

fn snapshot(w: &CombatWorld) -> Vec<Seen> {
    let now = w.time();
    w.combatants()
        .filter(|c| c.alive())
        .map(|c| Seen {
            id: c.id,
            team: c.team,
            position: c.position,
            chest: c.chest(),
            eye: c.eye(),
            facing: c.facing,
            velocity: c.velocity,
            radius: c.body.radius,
            health: c.vitals.health.fraction(),
            concealed: c.concealed_until > now,
            guarding: c.guard_time().is_some(),
            windup: match &c.action {
                Action::Attack(a) if a.phase == Phase::Windup => Some((
                    (a.def.windup - a.t).max(0.0),
                    a.dir,
                    reach_of(&a.def) + 0.6,
                    c.swings,
                )),
                _ => None,
            },
            busy: !matches!(c.action, Action::Idle | Action::Guard { .. }),
            mounted: c.mount.is_some(),
        })
        .collect()
}

struct Shout {
    by: CombatantId,
    team: TeamId,
    at: Vec3,
    target: CombatantId,
    radius: f32,
}

/// Called by the world when `id` is struck by `source`: being hurt is the
/// loudest stimulus there is.
pub(crate) fn hurt_by(w: &mut CombatWorld, id: CombatantId, source: CombatantId) {
    let now = w.time();
    let Some(position) = w.combatant(source).map(|s| s.position) else {
        return;
    };
    if let Some(c) = w.combatant_mut(id)
        && let Some(brain) = &mut c.brain
    {
        brain.mind.memory.remember(Stimulus {
            source: u64::from(source.0),
            position,
            strength: 1.5,
            sensed_at: now,
            sense: Sense::Hearing,
        });
        brain.mind.morale = brain.mind.morale.min(c.vitals.health.fraction() + 0.35);
    }
}

/// Tell a brain about `target` at `at`, as if it had heard it there: a shout
/// from outside the fight, a bell, a crime reported (the game's Hue and Cry).
pub fn alert(w: &mut CombatWorld, id: CombatantId, target: CombatantId, at: Vec3) {
    let now = w.time();
    if let Some(c) = w.combatant_mut(id)
        && let Some(brain) = &mut c.brain
    {
        brain.mind.memory.remember(Stimulus {
            source: u64::from(target.0),
            position: at,
            strength: 1.0,
            sensed_at: now,
            sense: Sense::Hearing,
        });
        brain.mind.think = 0.0;
    }
}

/// Give a companion an order.
pub fn order(w: &mut CombatWorld, id: CombatantId, order: Order) {
    if let Some(c) = w.combatant_mut(id)
        && let Some(brain) = &mut c.brain
    {
        brain.mind.order = order;
        if let Order::Attack(t) = order {
            brain.mind.target = Some(t);
        }
        brain.mind.situation = u8::MAX;
    }
}

/// A path's corners, each marked when the leg to it is one of the path's
/// off-mesh links (its ends sit within the link's radius of the leg's).
fn corners(nav: &NavWorld, path: &somnium_ai::navigation::NavPath) -> Vec<(Vec3, bool)> {
    let links: Vec<_> = path.links.iter().filter_map(|id| nav.link(*id)).collect();
    let near = |a: Vec3, b: Vec3, r: f32| a.distance(b) <= r + 1e-3;
    path.points
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            let link = i > 0
                && links.iter().any(|l| {
                    let from = path.points[i - 1];
                    (near(from, l.start, l.radius) && near(p, l.end, l.radius))
                        || (l.bidirectional
                            && near(from, l.end, l.radius)
                            && near(p, l.start, l.radius))
                });
            (p, link)
        })
        .collect()
}

struct Host<'a, Q: CombatQuery> {
    me: Seen,
    view: &'a [Seen],
    mind: &'a mut Mind,
    cmd: &'a mut Command,
    action: &'a Action,
    hostile: &'a dyn Fn(CombatantId) -> bool,
    allied: &'a dyn Fn(CombatantId) -> bool,
    tactics: &'a mut Tactics,
    shouts: &'a mut Vec<Shout>,
    casts: &'a mut Vec<SpellRequest>,
    events: &'a mut Vec<CombatEvent>,
    nav: Option<&'a NavWorld>,
    query: &'a Q,
    now: f64,
    dt: f32,
    max_attackers: usize,
    rng: &'a mut Rng,
    spells: &'a dyn Fn(&str) -> Option<crate::spell::SpellAsset>,
    caster: &'a crate::world::Combatant,
}

impl<Q: CombatQuery> Host<'_, Q> {
    fn find(&self, id: CombatantId) -> Option<&Seen> {
        self.view.iter().find(|s| s.id == id)
    }

    fn target(&self) -> Option<Seen> {
        self.mind.target.and_then(|t| self.find(t)).copied()
    }

    /// Gap between the bodies. To a melee fighter a height past a metre is
    /// far out of reach (someone on a ledge), so it keeps closing, up the link.
    fn distance(&self, other: &Seen) -> f32 {
        let gap = flat(other.position - self.me.position).length() - other.radius - self.me.radius;
        let rise = ((other.position.y - self.me.position.y).abs() - 1.0).max(0.0);
        if self.mind.ranged {
            gap
        } else {
            gap + rise * 10.0
        }
    }

    /// Steer toward `dest`, along the navmesh when there is one.
    fn go(&mut self, dest: Vec3, sprint: bool) -> bool {
        let me = self.me.position;
        // Passed: near on the ground plane, and not a ledge's height away.
        let at = |p: Vec3, within: f32| flat(p - me).length() < within && (p.y - me.y).abs() < 1.0;
        if at(dest, 0.4) {
            self.cmd.move_dir = Vec3::ZERO;
            return true;
        }
        let to = flat(dest - me);
        let mut dir = to.normalize_or_zero();
        if let Some(nav) = self.nav {
            // Walk the corridor's corners. `NavAgent` advances a corner only on
            // exact arrival, which assumes it moved the body; here collide and
            // slide moves it, so corners are passed within an arrival radius.
            // A leg over an off-mesh link (a vault, a climb) keeps its plan
            // until its end: one made mid-climb sends the body back to the foot.
            let on_link = self.mind.route.first().is_some_and(|c| c.1);
            let stale = if on_link {
                self.mind.route_age > 3.0
            } else {
                self.mind.route_goal.is_none_or(|g| g.distance(dest) > 1.0)
                    || self.mind.route_age > 1.0
            };
            if stale {
                // A point off the mesh (a slot in the air beside a ledge) is
                // planned to the nearest ground instead.
                let snap = (self.me.radius * 2.0).max(0.5);
                let planned = nav
                    .path(me, dest, snap)
                    .or_else(|| {
                        let near = nav.nearest(dest, 3.0)?;
                        nav.path(me, near, snap)
                    })
                    .map(|p| corners(nav, &p));
                // Falling or mid-climb the body is off the mesh: keep its plan.
                if planned.is_some() || nav.nearest(me, snap).is_some() {
                    self.mind.route = planned.unwrap_or_default();
                }
                self.mind.route_goal = Some(dest);
                self.mind.route_age = 0.0;
            }
            self.mind.route_age += self.dt;
            while self.mind.route.len() > 1 && at(self.mind.route[0].0, 0.35) {
                self.mind.route.remove(0);
            }
            if let [(end, _)] = self.mind.route[..]
                && at(end, 0.35)
            {
                // The end of a route planned to the nearest ground short of `dest`.
                self.cmd.move_dir = Vec3::ZERO;
                return true;
            }
            if let Some(&(next, link)) = self.mind.route.first() {
                dir = flat(next - me).try_normalize().unwrap_or(dir);
                // Run into the wall; the host's controller vaults or climbs it.
                self.cmd.climb = link;
            }
        }
        // Keep clear of friends.
        for s in self.view {
            if s.id == self.me.id || !(self.allied)(s.id) {
                continue;
            }
            let away = flat(self.me.position - s.position);
            let d = away.length();
            if d < 1.6 && d > 1e-3 {
                dir += away / d * (1.6 - d) * 0.8;
            }
        }
        dir = dir.normalize_or_zero();
        self.cmd.move_dir = dir;
        self.cmd.sprint = sprint;
        false
    }

    fn face(&mut self, point: Vec3) {
        self.cmd.aim = Some(point);
    }

    fn should_flee(&self) -> bool {
        let p = &self.mind.profile;
        self.mind.fleeing
            || (p.flee_below > 0.0 && self.me.health < p.flee_below && self.mind.morale < 0.6)
            || self.mind.morale <= 0.1 && p.flee_below > 0.0
    }

    fn band(&self) -> [f32; 2] {
        self.mind.profile.band
    }

    fn engage(&mut self) -> Status {
        let Some(t) = self.target() else {
            return Status::Failure;
        };
        let d = self.distance(&t);
        let band = self.band();
        self.face(t.chest);
        if d <= band[1] && (!self.mind.ranged || d >= band[0] * 0.8) {
            self.cmd.move_dir = Vec3::ZERO;
            return Status::Success;
        }
        if self.mind.ranged {
            // Close to the far edge of the band, not to the body.
            let toward = planar_dir(t.position - self.me.position, self.me.facing);
            let dest = t.position - toward * (band[0] + band[1]) * 0.5;
            self.go(dest, d > band[1] + 6.0);
            return Status::Running;
        }
        let radius = (band[0] + band[1]) * 0.5 + t.radius + self.me.radius;
        let dest = self.tactics.slot(
            t.id,
            self.me.id,
            t.position,
            t.facing,
            self.me.position,
            radius,
            self.mind.profile.flanker,
        );
        self.go(dest, d > 6.0);
        Status::Running
    }

    fn attack(&mut self) -> Status {
        let Some(t) = self.target() else {
            self.mind.attacking = false;
            return Status::Failure;
        };
        if self.mind.ranged {
            return self.shoot(t);
        }
        let d = self.distance(&t);
        self.face(t.chest);
        if self.mind.attacking {
            // Swinging: chain while the count lasts.
            match self.action {
                Action::Attack(a) => {
                    if a.phase == Phase::Recovery
                        && self.mind.swings_left > 0
                        && d <= self.mind.reach + 0.5
                    {
                        self.cmd.attack = Press::tap();
                        self.mind.swings_left -= 1;
                    }
                    return Status::Running;
                }
                Action::Idle | Action::Guard { .. } => {
                    self.mind.attacking = false;
                    self.tactics.release(t.id, self.me.id);
                    return Status::Success;
                }
                _ => return Status::Running,
            }
        }
        if d > self.mind.reach + 0.3 {
            return Status::Failure;
        }
        if !self
            .tactics
            .request(t.id, self.me.id, self.max_attackers, self.now)
        {
            // No opening: circle at the edge and wait for one.
            let around = Vec3::new(
                -(t.position.z - self.me.position.z),
                0.0,
                t.position.x - self.me.position.x,
            );
            let side = if self.me.id.0 % 2 == 0 { 1.0 } else { -1.0 };
            self.cmd.move_dir = planar_dir(around * side, self.me.facing) * 0.6;
            return Status::Success;
        }
        if self.me.busy {
            return Status::Running;
        }
        self.mind.attacking = true;
        let chain = self.mind.profile.combo.max(1);
        let roll = self.rng.unit();
        self.mind.swings_left =
            ((f32::from(chain) * (0.4 + roll * 0.6)).round() as u8).saturating_sub(1);
        // A guard invites the heavy; distance invites the charge.
        if t.guarding && self.rng.chance(self.mind.profile.aggression) {
            self.cmd.attack = Press::tap();
            self.mind.swings_left = 0;
            self.mind.doing = "Heavy";
        } else {
            self.cmd.attack = Press::tap();
            self.mind.doing = "Attacking";
        }
        Status::Running
    }

    fn shoot(&mut self, t: Seen) -> Status {
        // Lead the target by its flight time.
        let speed = self
            .caster
            .weapon
            .ranged
            .as_ref()
            .map_or(40.0, |r| r.projectile.speed);
        let flight = flat(t.position - self.me.position).length() / speed.max(1.0);
        let aim = t.position + t.velocity * flight;
        self.face(aim + Vec3::Y * 1.1);
        match self.action {
            Action::Aim { t: drawn } => {
                let draw = self.caster.weapon.ranged.as_ref().map_or(0.5, |r| r.draw);
                let release = self
                    .caster
                    .weapon
                    .ranged
                    .as_ref()
                    .is_some_and(|r| r.release_to_fire);
                if release && *drawn >= draw {
                    self.cmd.attack = Press::release();
                } else {
                    self.cmd.attack = Press::hold();
                }
                Status::Running
            }
            Action::Recover { .. } | Action::Reload { .. } => {
                self.mind.attacking = false;
                Status::Success
            }
            Action::Idle | Action::Guard { .. } => {
                if self.query.blocked(self.me.eye, t.chest) {
                    return Status::Failure;
                }
                self.cmd.attack = Press::tap();
                self.mind.attacking = true;
                self.mind.doing = "Shooting";
                Status::Running
            }
            _ => Status::Running,
        }
    }

    fn keep_distance(&mut self) -> Status {
        let Some(t) = self.target() else {
            return Status::Failure;
        };
        let d = self.distance(&t);
        let band = self.band();
        self.face(t.chest);
        if d >= band[0] + 1.0 {
            return Status::Success;
        }
        let away = planar_dir(self.me.position - t.position, -self.me.facing);
        self.go(
            self.me.position + away * (band[0] + 2.0 - d).max(1.0),
            false,
        );
        Status::Running
    }

    fn cast(&mut self, heal: bool) -> Status {
        if let Action::Cast(_) = self.action {
            return Status::Running;
        }
        if self.mind.casting {
            // The verse is out (or was refused): let the tree look again.
            self.mind.casting = false;
            return Status::Success;
        }
        if self.me.busy {
            return Status::Running;
        }
        let me = self.caster;
        let candidates = self.mind.profile.spells.clone();
        for id in candidates {
            let Some(asset) = (self.spells)(&id) else {
                continue;
            };
            let heals = asset.element == crate::spell::Element::Mending;
            if heals != heal {
                continue;
            }
            if me.cooldowns.get(&id).is_some_and(|r| *r > self.now) {
                continue;
            }
            let bearing = if heal {
                // The most hurt ally in reach, below half.
                let hurt = self
                    .view
                    .iter()
                    .filter(|s| (self.allied)(s.id) && s.health < 0.5)
                    .filter(|s| {
                        flat(s.position - self.me.position).length() <= asset.default_measure.range
                    })
                    .min_by(|a, b| a.health.total_cmp(&b.health));
                match hurt {
                    Some(s) => Bearing::Target(s.id),
                    None => return Status::Failure,
                }
            } else {
                let Some(t) = self.target() else {
                    return Status::Failure;
                };
                if flat(t.position - self.me.position).length() > asset.default_measure.range {
                    continue;
                }
                if asset.default_shape == crate::spell::Shape::Bloom {
                    Bearing::Cursor(t.position)
                } else {
                    Bearing::Target(t.id)
                }
            };
            let release = crate::spell::CastSpeed::ALL
                .into_iter()
                .rev()
                .find(|s| *s != crate::spell::CastSpeed::Silent && me.magic.knows_speed(*s))
                .unwrap_or(crate::spell::CastSpeed::Incantation);
            let request = SpellRequest {
                spell: asset.id.clone(),
                address: Address::Well,
                element: asset.element,
                shape: asset.default_shape,
                measure: asset.fit(asset.default_measure, &me.magic, 0),
                bearing,
                release,
            };
            if asset.validate(&request, &me.magic).is_err()
                || me.vitals.mana.current < asset.cost(&request).mana
            {
                continue;
            }
            self.casts.push(request);
            self.mind.casting = true;
            self.mind.doing = if heal { "Healing" } else { "Casting" };
            return Status::Running;
        }
        Status::Failure
    }

    fn flee(&mut self) -> Status {
        self.mind.fleeing = true;
        self.mind.doing = "Fleeing";
        if !self.mind.fled_reported {
            self.mind.fled_reported = true;
            self.events.push(CombatEvent::Fled { who: self.me.id });
        }
        let threats: Vec<Vec3> = self
            .view
            .iter()
            .filter(|s| (self.hostile)(s.id) && s.position.distance(self.me.position) < 30.0)
            .map(|s| s.position)
            .collect();
        if threats.is_empty() {
            self.cmd.move_dir = Vec3::ZERO;
            return Status::Success;
        }
        let centre = threats.iter().copied().sum::<Vec3>() / threats.len() as f32;
        let away = planar_dir(self.me.position - centre, -self.me.facing);
        self.go(self.me.position + away * 8.0, true);
        Status::Running
    }

    fn follow(&mut self) -> Status {
        let Some(leader) = self.mind.leader.and_then(|l| self.find(l)).copied() else {
            return self.idle();
        };
        let d = flat(leader.position - self.me.position).length();
        self.mind.doing = "Following";
        if d < 3.5 {
            self.cmd.move_dir = Vec3::ZERO;
            return Status::Success;
        }
        let behind = leader.position - leader.facing * 2.0
            + Vec3::new(leader.facing.z, 0.0, -leader.facing.x)
                * if self.me.id.0 % 2 == 0 { 1.5 } else { -1.5 };
        self.go(behind, d > 8.0);
        Status::Running
    }

    fn idle(&mut self) -> Status {
        self.mind.doing = "Idle";
        match self.mind.home {
            Some(h) if flat(h - self.me.position).length() > 1.0 => {
                self.go(h, false);
                Status::Running
            }
            _ => {
                self.cmd.move_dir = Vec3::ZERO;
                Status::Success
            }
        }
    }
}

impl<Q: CombatQuery> TaskHost for Host<'_, Q> {
    fn tick(&mut self, script: bool, name: &str, board: &mut Blackboard, _delta: f32) -> Status {
        if script {
            return Status::Failure;
        }
        let status = match name {
            "combat.has_target" => {
                if self.target().is_some() {
                    Status::Success
                } else {
                    Status::Failure
                }
            }
            "combat.has_lead" => {
                if self.mind.lead.is_some() {
                    Status::Success
                } else {
                    Status::Failure
                }
            }
            "combat.should_flee" => {
                if self.should_flee() {
                    Status::Success
                } else {
                    Status::Failure
                }
            }
            "combat.too_close" => match self.target() {
                Some(t) if self.distance(&t) < self.band()[0] => Status::Success,
                _ => Status::Failure,
            },
            "combat.alert_allies" => {
                if let Some(t) = self.target()
                    && self.mind.shouted_for != Some(t.id)
                {
                    self.mind.shouted_for = Some(t.id);
                    self.shouts.push(Shout {
                        by: self.me.id,
                        team: self.me.team,
                        at: t.position,
                        target: t.id,
                        radius: self.mind.profile.alert_radius,
                    });
                }
                Status::Success
            }
            "combat.engage" => {
                self.mind.doing = "Engaging";
                self.engage()
            }
            "combat.flank" => {
                let flanker = self.mind.profile.flanker;
                self.mind.profile.flanker = true;
                self.mind.doing = "Flanking";
                let s = self.engage();
                self.mind.profile.flanker = flanker;
                match (s, self.target()) {
                    (Status::Success, Some(t)) => {
                        let behind = t
                            .facing
                            .dot(planar_dir(self.me.position - t.position, t.facing))
                            < 0.2;
                        if behind || self.rng.chance(0.05) {
                            Status::Success
                        } else {
                            // In reach but in front: work round.
                            let radius = self.band()[1] + t.radius + self.me.radius;
                            let dest = self.tactics.slot(
                                t.id,
                                self.me.id,
                                t.position,
                                t.facing,
                                self.me.position,
                                radius,
                                true,
                            );
                            self.go(dest, false);
                            Status::Running
                        }
                    }
                    (s, _) => s,
                }
            }
            "combat.attack" | "combat.shoot" => self.attack(),
            "combat.keep_distance" => {
                self.mind.doing = "Backing off";
                self.keep_distance()
            }
            "combat.cast" => self.cast(false),
            "combat.heal_ally" => self.cast(true),
            "combat.flee" => self.flee(),
            "combat.investigate" => {
                self.mind.doing = "Searching";
                match self.mind.lead {
                    Some(lead) => {
                        self.face(lead);
                        if self.go(lead, false) {
                            self.mind.lead = None;
                            Status::Success
                        } else {
                            Status::Running
                        }
                    }
                    None => Status::Failure,
                }
            }
            "combat.follow_leader" => self.follow(),
            "combat.hold" => match self.mind.order {
                Order::Hold(p) => {
                    self.mind.doing = "Holding";
                    if let Some(t) = self.target() {
                        self.face(t.chest);
                    }
                    self.go(p, false);
                    Status::Running
                }
                Order::FallBack => {
                    self.mind.target = None;
                    self.follow()
                }
                _ => Status::Failure,
            },
            "combat.idle" => self.idle(),
            other => {
                board.insert(
                    "task_error".into(),
                    somnium_ai::behavior::Value::Text(format!("Unknown combat task {other}")),
                );
                Status::Failure
            }
        };
        board.insert(
            "doing".into(),
            somnium_ai::behavior::Value::Text(self.mind.doing.to_string()),
        );
        status
    }

    fn cancel(&mut self, _script: bool, name: &str, _board: &mut Blackboard) {
        if matches!(name, "combat.attack" | "combat.shoot")
            && let Some(t) = self.mind.target
        {
            self.tactics.release(t, self.me.id);
            self.mind.attacking = false;
        }
        self.cmd.move_dir = Vec3::ZERO;
    }
}

/// Perceive: update memory from sight and sound, then pick the target.
fn perceive(w: &CombatWorld, me: &Seen, mind: &mut Mind, view: &[Seen], query: &impl CombatQuery) {
    let now = w.time();
    let p = &mind.profile;
    let walls: Vec<_> = w
        .walls()
        .iter()
        .filter(|wall| wall.blocks_movement())
        .map(|wall| wall.obb)
        .collect();
    for other in view {
        if other.id == me.id || !w.hostile(me.id, other.id) {
            continue;
        }
        let observer = Observer {
            position: me.eye,
            forward: me.facing,
            sight_range: p.sight * if other.concealed { 0.4 } else { 1.0 },
            half_angle_degrees: (p.fov * 0.5).clamp(1.0, 180.0),
            hearing_range: p.hearing,
        };
        let seen = observer.see(u64::from(other.id.0), other.chest, now, |a, b| {
            let dir = b - a;
            let len = dir.length();
            query.blocked(a, b)
                || dir
                    .try_normalize()
                    .is_some_and(|d| walls.iter().any(|o| o.ray(a, d, len, 0.0).is_some()))
        });
        if let Some(s) = seen {
            mind.memory.remember(s);
        } else if flat(other.position - me.position).length() < 1.6 {
            // A body against your back is felt.
            mind.memory.remember(Stimulus {
                source: u64::from(other.id.0),
                position: other.position,
                strength: 0.5,
                sensed_at: now,
                sense: Sense::Sight,
            });
        }
    }
    for noise in &w.noises {
        if noise.source == me.id || !w.hostile(me.id, noise.source) {
            continue;
        }
        let observer = Observer {
            position: me.eye,
            forward: me.facing,
            sight_range: 1.0,
            half_angle_degrees: 180.0,
            hearing_range: p.hearing * (0.5 + noise.loudness * 2.0),
        };
        if let Some(s) = observer.hear(
            u64::from(noise.source.0),
            noise.position,
            noise.loudness.max(0.05),
            now,
        ) {
            mind.memory.remember(s);
        }
    }
    mind.memory.expire(now);
    // Companions fight whoever threatens their leader first.
    let mut best: Option<Stimulus> = None;
    if let Some(strongest) = mind.memory.strongest(now) {
        best = Some(strongest);
    }
    let known = |id: u32| view.iter().any(|s| s.id.0 == id);
    match best {
        Some(s) if known(s.source as u32) => {
            let candidate = CombatantId(s.source as u32);
            let keep = mind
                .target
                .filter(|t| view.iter().any(|v| v.id == *t) && w.hostile(me.id, *t));
            mind.target = Some(keep.unwrap_or(candidate));
            mind.lead = Some(s.position);
        }
        Some(s) => {
            mind.target = None;
            mind.lead = Some(s.position);
        }
        None => {
            mind.target = mind
                .target
                .filter(|t| view.iter().any(|v| v.id == *t) && w.hostile(me.id, *t));
        }
    }
    if let Order::Attack(t) = mind.order
        && view.iter().any(|v| v.id == t)
    {
        mind.target = Some(t);
    }
    if mind.archetype == Archetype::Companion
        && let Some(leader) = mind.leader.and_then(|l| w.combatant(l))
    {
        let leash = mind.profile.leash.max(1.0);
        if let Some(t) = mind.target.and_then(|t| view.iter().find(|v| v.id == t))
            && flat(t.position - leader.position).length() > leash * 2.0
        {
            mind.target = None;
        }
        if mind.target.is_none()
            && let Some(attacker) = leader.last_attacker
            && view.iter().any(|v| v.id == attacker)
            && w.hostile(me.id, attacker)
            && w.time() - leader.last_hurt < 6.0
        {
            mind.target = Some(attacker);
        }
    }
}

/// Raise a guard on the beat or dodge a blow seen coming; side-step missiles.
/// Returns true when the reflex has the body this step (the tree waits).
fn reflex(
    w: &CombatWorld,
    me: &Seen,
    mind: &mut Mind,
    view: &[Seen],
    cmd: &mut Command,
    rng: &mut Rng,
) -> bool {
    let now = w.time();
    let Some(c) = w.combatant(me.id) else {
        return false;
    };
    if !c.status.can_act() || me.mounted {
        return false;
    }
    if now < mind.guard_until {
        cmd.guard = Press::hold();
        return true;
    }
    let free = matches!(c.action, Action::Idle | Action::Guard { .. })
        || matches!(&c.action, Action::Attack(a) if a.phase == Phase::Recovery);
    if !free {
        return false;
    }
    let parry_window = c.weapon.guard.parry_window * c.sword.map_or(1.0, |s| s.parry_scale());
    for other in view {
        let Some((left, dir, reach, serial)) = other.windup else {
            continue;
        };
        if !w.hostile(me.id, other.id) {
            continue;
        }
        let to_me = flat(me.position - other.position);
        if to_me.length() > reach + me.radius || dir.dot(to_me.normalize_or_zero()) < 0.5 {
            continue;
        }
        if mind.reacted.contains(&(other.id, serial)) {
            continue;
        }
        // Wait for the beat: a parry belongs at the end of the windup.
        let beat = if c.weapon.guard.can_parry {
            parry_window * 0.6
        } else {
            0.25
        };
        if left > beat {
            continue;
        }
        mind.reacted.push((other.id, serial));
        if mind.reacted.len() > 16 {
            mind.reacted.remove(0);
        }
        let skill = mind.profile.skill;
        let roll = rng.unit();
        if roll < skill * 0.6 {
            cmd.guard = Press::tap();
            mind.guard_until = now + f64::from(left + 0.35);
            mind.doing = "Guarding";
            return true;
        } else if roll < skill {
            cmd.dodge = true;
            cmd.move_dir = Vec3::new(-to_me.z, 0.0, to_me.x).normalize_or_zero()
                * if rng.chance(0.5) { 1.0 } else { -1.0 };
            mind.doing = "Dodging";
            return true;
        }
        return false;
    }
    // Missiles in flight toward me.
    for p in w.projectiles() {
        if p.owner.is_none_or(|o| !w.hostile(me.id, o)) {
            continue;
        }
        let to = me.chest - p.position;
        let speed = p.velocity.length();
        if speed < 1.0 {
            continue;
        }
        let closing = p.velocity.dot(to) / speed;
        let time = closing / speed;
        let miss = (to - p.velocity / speed * closing).length();
        if time > 0.0 && time < 0.5 && miss < me.radius + 0.4 {
            let key = (CombatantId(u32::MAX), p.id);
            if mind.reacted.contains(&key) {
                continue;
            }
            mind.reacted.push(key);
            if rng.chance(mind.profile.skill * 0.5) {
                cmd.dodge = true;
                let v = flat(p.velocity);
                cmd.move_dir = Vec3::new(-v.z, 0.0, v.x).normalize_or_zero();
                mind.doing = "Dodging";
                return true;
            }
            return false;
        }
    }
    false
}

/// Every brain's step: perceive (at ten hertz), react, decide.
pub(crate) fn think(
    w: &mut CombatWorld,
    dt: f32,
    query: &impl CombatQuery,
    nav: Option<&NavWorld>,
) {
    let view = snapshot(w);
    let ids: Vec<CombatantId> = w
        .combatants()
        .filter(|c| c.brain.is_some() && c.alive())
        .map(|c| c.id)
        .collect();
    let mut shouts = Vec::new();
    let mut casts: Vec<(CombatantId, SpellRequest)> = Vec::new();
    let mut events = Vec::new();
    let now = w.time();
    let max_attackers = w.rules.max_melee_attackers;
    let mut rng = std::mem::replace(&mut w.rng, Rng::new(1));
    let mut tactics = std::mem::take(&mut w.tactics);
    for id in ids {
        let Some(me) = view.iter().find(|s| s.id == id).copied() else {
            continue;
        };
        let Some(mut brain) = w.combatant_mut(id).and_then(|c| c.brain.take()) else {
            continue;
        };
        let mut cmd = w
            .combatant(id)
            .map(|c| c.command.clone())
            .unwrap_or_default();
        cmd.move_dir = Vec3::ZERO;
        cmd.sprint = false;
        cmd.attack = Press {
            pressed: false,
            held: false,
            released: false,
        };
        cmd.guard = Press::default();
        cmd.channel = false;
        cmd.climb = false;
        if let Some(c) = w.combatant(id) {
            brain.mind.ranged = matches!(c.weapon.kind, WeaponKind::Bow | WeaponKind::LatticeRifle);
            brain.mind.reach = c
                .weapon
                .light
                .first()
                .map_or(c.weapon.reach, reach_of)
                .max(c.weapon.reach);
            brain.mind.charges = c.weapon.charge.is_some();
            // Morale follows health and company.
            let friends = view
                .iter()
                .filter(|s| {
                    s.id != id && w.allied(id, s.id) && s.position.distance(me.position) < 15.0
                })
                .count();
            let foes = view
                .iter()
                .filter(|s| w.hostile(id, s.id) && s.position.distance(me.position) < 15.0)
                .count();
            let pressure = (foes as f32 - friends as f32 - 1.0).max(0.0) * 0.1;
            let ceiling = (c.vitals.health.fraction() + 0.35 - pressure).clamp(0.0, 1.0);
            brain.mind.morale = (brain.mind.morale + 0.02 * dt)
                .min(ceiling)
                .min(brain.mind.profile.morale.max(ceiling * 0.5));
        }
        brain.mind.think -= dt;
        if brain.mind.think <= 0.0 {
            // Staggered by id so a crowd does not all think on one step.
            brain.mind.think += THINK_INTERVAL + (id.0 % 5) as f32 * 0.004;
            perceive(w, &me, &mut brain.mind, &view, query);
        }
        let reflexed = reflex(w, &me, &mut brain.mind, &view, &mut cmd, &mut rng);
        let situation = u8::from(brain.mind.target.is_some())
            | u8::from(
                brain.mind.fleeing || {
                    let p = &brain.mind.profile;
                    p.flee_below > 0.0 && me.health < p.flee_below && brain.mind.morale < 0.6
                },
            ) << 1
            | u8::from(brain.mind.lead.is_some()) << 2
            | (match brain.mind.order {
                Order::Follow => 0,
                Order::Hold(_) => 1,
                Order::Attack(_) => 2,
                Order::FallBack => 3,
            }) << 3;
        let Some(c) = w.combatant(id) else {
            continue;
        };
        let action = c.action.clone();
        let caster = c.clone();
        let hostile = |o: CombatantId| w.hostile(id, o);
        let allied = |o: CombatantId| w.allied(id, o);
        let spells = |s: &str| w.spell(s).cloned();
        let mut my_casts = Vec::new();
        {
            let Brain { instance, mind } = &mut brain;
            let mut host = Host {
                me,
                view: &view,
                mind,
                cmd: &mut cmd,
                action: &action,
                hostile: &hostile,
                allied: &allied,
                tactics: &mut tactics,
                shouts: &mut shouts,
                casts: &mut my_casts,
                events: &mut events,
                nav,
                query,
                now,
                dt,
                max_attackers,
                rng: &mut rng,
                spells: &spells,
                caster: &caster,
            };
            if situation != host.mind.situation {
                host.mind.situation = situation;
                instance.reset(&mut host);
            }
            if !reflexed {
                let status = instance.tick(dt, &mut host);
                if status != Status::Running {
                    instance.reset(&mut host);
                }
            }
        }
        casts.extend(my_casts.into_iter().map(|r| (id, r)));
        if let Some(c) = w.combatant_mut(id) {
            c.command = cmd;
            c.brain = Some(brain);
        }
    }
    w.tactics = tactics;
    w.rng = rng;
    for (id, request) in casts {
        if let Some(c) = w.combatant_mut(id) {
            c.command.cast = Some(request);
        }
    }
    for e in events {
        w.emit(e);
    }
    for shout in shouts {
        let listeners: Vec<CombatantId> = w
            .combatants()
            .filter(|c| {
                c.id != shout.by
                    && c.alive()
                    && c.brain.is_some()
                    && c.position.distance(shout.at) <= shout.radius + 20.0
                    && c.team == shout.team
            })
            .map(|c| c.id)
            .collect();
        let at = shout.at;
        let mut heard = false;
        for l in listeners {
            let near = w
                .combatant(l)
                .zip(w.combatant(shout.by))
                .is_some_and(|(a, b)| a.position.distance(b.position) <= shout.radius);
            if !near {
                continue;
            }
            if let Some(c) = w.combatant_mut(l)
                && let Some(brain) = &mut c.brain
            {
                brain.mind.memory.remember(Stimulus {
                    source: u64::from(shout.target.0),
                    position: at,
                    strength: 0.8,
                    sensed_at: now,
                    sense: Sense::Hearing,
                });
                brain.mind.think = 0.0;
                heard = true;
            }
        }
        if heard {
            w.emit(CombatEvent::Alerted {
                by: shout.by,
                target: shout.target,
                at,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_archetype_tree_is_valid_and_names_only_known_tasks() {
        const KNOWN: &[&str] = &[
            "combat.has_target",
            "combat.has_lead",
            "combat.should_flee",
            "combat.too_close",
            "combat.alert_allies",
            "combat.engage",
            "combat.flank",
            "combat.attack",
            "combat.shoot",
            "combat.keep_distance",
            "combat.cast",
            "combat.heal_ally",
            "combat.flee",
            "combat.investigate",
            "combat.follow_leader",
            "combat.hold",
            "combat.idle",
        ];
        for a in Archetype::ALL {
            let tree = a.tree();
            for node in tree.nodes() {
                if let Node::Task(Task::Native(name)) = node {
                    assert!(KNOWN.contains(&name.as_str()), "{a:?} names {name}");
                }
            }
            // And it round-trips as an asset.
            let bytes = tree.encode().unwrap();
            assert_eq!(BehaviorTree::decode(&bytes).unwrap(), tree);
        }
    }

    #[test]
    fn tokens_limit_attackers_and_expire() {
        let mut t = Tactics::default();
        let target = CombatantId(1);
        assert!(t.request(target, CombatantId(2), 2, 0.0));
        assert!(t.request(target, CombatantId(3), 2, 0.0));
        assert!(!t.request(target, CombatantId(4), 2, 0.0));
        assert!(
            t.request(target, CombatantId(2), 2, 0.0),
            "a holder keeps its token"
        );
        t.release(target, CombatantId(2));
        assert!(t.request(target, CombatantId(4), 2, 0.0));
        t.update(10.0, &[], |_| true);
        assert_eq!(t.holders(target), 0, "stale tokens expire");
    }

    #[test]
    fn slots_spread_attackers_and_flankers_go_behind() {
        let mut t = Tactics::default();
        let target = CombatantId(1);
        let at = Vec3::ZERO;
        let facing = Vec3::Z;
        let a = t.slot(
            target,
            CombatantId(2),
            at,
            facing,
            Vec3::new(0.0, 0.0, 5.0),
            2.0,
            false,
        );
        let b = t.slot(
            target,
            CombatantId(3),
            at,
            facing,
            Vec3::new(0.0, 0.0, 5.0),
            2.0,
            false,
        );
        assert!(a.distance(b) > 1.0, "two attackers take two slots");
        let f = t.slot(
            target,
            CombatantId(4),
            at,
            facing,
            Vec3::new(0.0, 0.0, 5.0),
            2.0,
            true,
        );
        assert!(f.z < -1.0, "the flanker goes behind: {f}");
    }
}
