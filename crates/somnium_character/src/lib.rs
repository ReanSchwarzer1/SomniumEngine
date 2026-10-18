//! Kinematic character locomotion: walk, run, step up, slopes, jump, fall,
//! mantle and climb ledges, swim (ENGINE_ROADMAP 2.3, "character controller").
//!
//! One [`Controller`] per body. The host hands it the velocity it wants on
//! the ground plane (from the player's keys, an AI's intent, or
//! `somnium_combat`'s movement intent) and a [`Terrain`] that sweeps capsules
//! through the world (Jolt in the engine, boxes in tests). It answers with a
//! new feet position and the moments the animation needs: jumped, landed,
//! started a vault or a climb (with the ledge's edge for the hands' IK),
//! entered or left deep water.
//!
//! A sledge is a flag on the ground state ([`Controller::sledging`]): the
//! slope pulls, the snow barely holds it back, the wanted direction steers
//! and pulling back brakes.
//!
//! ```text
//!            ┌── jump ──▶ Air ── lands ──▶ Ground
//!  Ground ───┤── walks off an edge ──▶ Air (holding climb over a gap with ground across: a leap)
//!            ├── pushes into a ledge it can reach ──▶ Mantle (vault ≤ 1.25 m, climb ≤ 2.3 m) ──▶ Ground
//!            └── water deeper than the chest ──▶ Swim ── shore underfoot ──▶ Ground
//!                                                  └── a bank it can reach ──▶ Mantle
//! ```
//!
//! Horizontal motion goes through `somnium_anim::collide_and_slide`, the
//! engine's one slide solver, so a character slides along walls and up and
//! down gentle slopes the same way root motion does. Low obstacles are stepped
//! over; steep ground is slid down; deep water floats the chest at the surface.
#![deny(missing_docs)]

use glam::{Quat, Vec3};
use somnium_anim::{MotionHit, collide_and_slide};

/// A contact from a capsule sweep.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    /// Fraction of the displacement travelled before contact, 0 to 1.
    pub fraction: f32,
    /// Outward surface normal.
    pub normal: Vec3,
}

/// The world, as locomotion asks about it.
pub trait Terrain {
    /// Sweep an upright capsule standing on `feet` (`radius`, overall
    /// `height`) by `delta`; the first contact that opposes the motion.
    fn sweep(&self, feet: Vec3, delta: Vec3, radius: f32, height: f32) -> Option<Contact>;

    /// The water surface's height at a point on the ground plane, if there is water.
    fn water_surface(&self, _x: f32, _z: f32) -> Option<f32> {
        None
    }
}

/// Tunables.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControllerConfig {
    /// Capsule radius.
    pub radius: f32,
    /// Standing height.
    pub height: f32,
    /// Highest obstacle walked straight over.
    pub step: f32,
    /// Steepest walkable slope, degrees.
    pub max_slope: f32,
    /// Ground acceleration toward the wanted velocity, metres per second squared.
    pub acceleration: f32,
    /// Fraction of the ground acceleration available in the air.
    pub air_control: f32,
    /// Gravity, metres per second squared.
    pub gravity: f32,
    /// Take-off speed of a jump.
    pub jump_speed: f32,
    /// Highest ledge vaulted over (a quick hop).
    pub vault_max: f32,
    /// Highest ledge climbed (hands on the edge, pulled up).
    pub climb_max: f32,
    /// Metres a second a climb rises.
    pub climb_speed: f32,
    /// Water at least this deep (above the feet) floats the body.
    pub swim_depth: f32,
    /// Fastest swim.
    pub swim_speed: f32,
    /// How far below the feet ground is still stood on (stairs down, bumps).
    pub snap: f32,
    /// Clearance kept from surfaces.
    pub skin: f32,
    /// Sledge: fraction of speed lost to the snow per second.
    pub sledge_friction: f32,
    /// Sledge: fastest turn, radians a second.
    pub sledge_turn: f32,
    /// Sledge: fraction of speed shed per second while pulling back.
    pub sledge_brake: f32,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            radius: 0.35,
            height: 1.8,
            step: 0.4,
            max_slope: 46.0,
            acceleration: 40.0,
            air_control: 0.25,
            gravity: 9.81,
            jump_speed: 4.6,
            vault_max: 1.25,
            climb_max: 2.3,
            climb_speed: 2.6,
            swim_depth: 1.3,
            swim_speed: 2.0,
            snap: 0.35,
            skin: 0.02,
            sledge_friction: 0.04,
            sledge_turn: 1.2,
            sledge_brake: 1.5,
        }
    }
}

/// What kind of ledge move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MantleKind {
    /// A hop over or onto something waist-high.
    Vault,
    /// Hands on the edge, pulled up.
    Climb,
}

/// A ledge move in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mantle {
    /// Vault or climb.
    pub kind: MantleKind,
    /// Feet where it began.
    pub from: Vec3,
    /// Feet where it ends, standing on the top.
    pub to: Vec3,
    /// Where the hands go: the ledge's edge (for the arms' IK).
    pub edge: Vec3,
    /// The wall's outward normal.
    pub wall_normal: Vec3,
    /// Seconds in.
    pub t: f32,
    /// Seconds long.
    pub duration: f32,
}

/// Fraction of a mantle spent rising along the wall, hands on the edge; the
/// rest carries the feet over the top.
pub const MANTLE_LIFT: f32 = 0.65;

/// Longest jump [`Controller::find_landing`] follows, seconds.
pub const LEAP_SECONDS: f32 = 1.5;

impl Mantle {
    /// How far through the move, 0 at the start to 1 standing on top.
    #[must_use]
    pub fn progress(&self) -> f32 {
        (self.t / self.duration.max(1e-3)).clamp(0.0, 1.0)
    }

    /// Seconds until the body stands on top.
    #[must_use]
    pub fn remaining(&self) -> f32 {
        (self.duration - self.t).max(0.0)
    }

    /// Hands on the edge and rising, the feet not yet over the top: the body
    /// can neither strike nor guard, and is easiest to knock off.
    #[must_use]
    pub fn vulnerable(&self) -> bool {
        self.progress() < MANTLE_LIFT
    }

    /// Feet position along the move: up the wall first, then over the top.
    #[must_use]
    pub fn position(&self) -> Vec3 {
        let u = self.progress();
        let lift = MANTLE_LIFT;
        if u < lift {
            let k = u / lift;
            let k = k * k * (3.0 - 2.0 * k);
            Vec3::new(
                self.from.x,
                self.from.y + (self.to.y - self.from.y) * k,
                self.from.z,
            )
        } else {
            let k = (u - lift) / (1.0 - lift);
            let k = k * k * (3.0 - 2.0 * k);
            let up = Vec3::new(self.from.x, self.to.y, self.from.z);
            up.lerp(self.to, k)
        }
    }
}

/// Locomotion state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// Standing on walkable ground.
    Ground,
    /// Jumping, falling, sliding down something too steep.
    Air,
    /// Vaulting or climbing a ledge.
    Mantle(Mantle),
    /// Afloat.
    Swim,
}

/// What the host asks for this step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveInput {
    /// Wanted velocity on the ground plane, metres per second.
    pub velocity: Vec3,
    /// Jump pressed: facing a reachable ledge it vaults or climbs instead.
    pub jump: bool,
    /// Held: pushing into a reachable ledge mantles it (on foot, mid-air,
    /// swimming), and walking off an edge over a gap with ground to land on
    /// across it ([`Controller::find_landing`]) leaps it. Water below an
    /// edge is never leapt this way: it is dropped into, swum, and climbed
    /// out of (press `jump` to jump it).
    pub climb: bool,
}

/// Moments for animation, sound and gameplay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LocomotionEvent {
    /// Left the ground on a jump.
    Jumped,
    /// Touched down; the speed is how fast it was falling (fall damage, a roll).
    Landed {
        /// Downward speed at touch-down.
        speed: f32,
    },
    /// Began a vault or a climb: play the clip, put the hands on `edge`.
    MantleStarted {
        /// Vault or climb.
        kind: MantleKind,
        /// Height of the ledge above the feet.
        height: f32,
        /// The edge the hands take.
        edge: Vec3,
        /// The wall's outward normal.
        wall_normal: Vec3,
    },
    /// Finished a vault or climb, standing on top.
    MantleEnded,
    /// Water deep enough to swim.
    EnteredWater,
    /// Feet back on the bottom.
    LeftWater,
}

/// One body's locomotion.
#[derive(Clone, Debug, PartialEq)]
pub struct Controller {
    /// Tunables.
    pub config: ControllerConfig,
    /// Feet position.
    pub feet: Vec3,
    /// Current velocity.
    pub velocity: Vec3,
    /// State.
    pub mode: Mode,
    /// The ground's normal while standing.
    pub ground_normal: Vec3,
    /// Riding a sledge (the host sets it).
    pub sledging: bool,
}

impl Controller {
    /// A controller standing at `feet` (it settles onto the ground on its first step).
    #[must_use]
    pub fn new(config: ControllerConfig, feet: Vec3) -> Self {
        Self {
            config,
            feet,
            velocity: Vec3::ZERO,
            mode: Mode::Air,
            ground_normal: Vec3::Y,
            sledging: false,
        }
    }

    /// Whether standing on the ground.
    #[must_use]
    pub fn grounded(&self) -> bool {
        matches!(self.mode, Mode::Ground)
    }

    fn walkable(&self, normal: Vec3) -> bool {
        normal.y >= self.config.max_slope.to_radians().cos()
    }

    fn sweep(&self, terrain: &impl Terrain, feet: Vec3, delta: Vec3) -> Option<Contact> {
        terrain.sweep(feet, delta, self.config.radius, self.config.height)
    }

    /// Slide the capsule by `delta` from `feet`; returns (applied, blocked).
    /// On foot, anything too steep to stand on is a wall: it is slid along,
    /// never ridden up.
    fn slide(
        &self,
        terrain: &impl Terrain,
        feet: Vec3,
        delta: Vec3,
        on_foot: bool,
    ) -> (Vec3, Vec3) {
        let (r, h) = (self.config.radius, self.config.height);
        match collide_and_slide(feet, delta, self.config.skin, |from, d| {
            terrain.sweep(from, d, r, h).map(|c| {
                let flat = Vec3::new(c.normal.x, 0.0, c.normal.z);
                let at = from + d * c.fraction;
                let normal = if on_foot
                    && !self.walkable(c.normal)
                    && flat.length_squared() > 1e-6
                    && self.edge_top(terrain, at, c.normal).is_none()
                {
                    flat.normalize()
                } else {
                    c.normal
                };
                MotionHit {
                    fraction: c.fraction,
                    normal,
                }
            })
        }) {
            Ok(m) => (m.applied, m.blocked),
            Err(_) => (Vec3::ZERO, delta),
        }
    }

    /// The ground under `feet` within `reach`: (distance down, normal). Resting
    /// on the edge of something walkable counts as standing on its top.
    fn probe_ground(&self, terrain: &impl Terrain, feet: Vec3, reach: f32) -> Option<(f32, Vec3)> {
        let lift = self.config.skin * 2.0;
        let travel = reach + lift;
        let c = self.sweep(terrain, feet + Vec3::Y * lift, Vec3::NEG_Y * travel)?;
        let down = c.fraction * travel - lift;
        let normal = if self.walkable(c.normal) {
            c.normal
        } else {
            self.edge_top(terrain, feet - Vec3::Y * down, c.normal)
                .unwrap_or(c.normal)
        };
        Some((down, normal))
    }

    /// For a contact low on the capsule (on its bottom sphere, with the capsule
    /// standing at `feet`): the normal of the top just past the contact, when
    /// that top is walkable. A step's edge has one; a steep slope does not.
    fn edge_top(&self, terrain: &impl Terrain, feet: Vec3, normal: Vec3) -> Option<Vec3> {
        if normal.y <= 0.02 {
            return None;
        }
        let r = self.config.radius;
        let point = feet + Vec3::Y * r - normal * r;
        let inward = Vec3::new(-normal.x, 0.0, -normal.z).normalize_or_zero() * 0.03;
        let top = terrain.sweep(
            point + inward + Vec3::Y * 0.06,
            Vec3::NEG_Y * 0.12,
            0.01,
            0.02,
        )?;
        self.walkable(top.normal).then_some(top.normal)
    }

    /// Lift, move, drop: walk over something no higher than a step.
    fn step_up(&self, terrain: &impl Terrain, feet: Vec3, delta: Vec3) -> Option<Vec3> {
        let step = self.config.step;
        let rise = self
            .sweep(terrain, feet, Vec3::Y * step)
            .map_or(step, |c| (c.fraction * step - self.config.skin).max(0.0));
        if rise < 0.05 {
            return None;
        }
        let raised = feet + Vec3::Y * rise;
        let (applied, _) = self.slide(terrain, raised, delta, true);
        if applied.length() < delta.length() * 0.5 {
            return None;
        }
        let over = raised + applied;
        let (down, normal) = self.probe_ground(terrain, over, rise + self.config.snap)?;
        if !self.walkable(normal) {
            return None;
        }
        Some(over - Vec3::Y * (down - self.config.skin))
    }

    /// A ledge ahead along `dir` that can be vaulted or climbed from `feet`.
    #[must_use]
    pub fn find_ledge(&self, terrain: &impl Terrain, feet: Vec3, dir: Vec3) -> Option<Mantle> {
        let c = &self.config;
        let dir = Vec3::new(dir.x, 0.0, dir.z).try_normalize()?;
        let reach = c.radius * 2.0 + 0.15;
        // Thin probes down onto the top from above the highest climbable ledge:
        // just past a wall's face first (a fence is thin), then further on.
        let drop = c.climb_max + 0.05 - c.step * 0.5;
        let top = [c.radius + 0.06, c.radius + 0.2, c.radius + 0.4, reach]
            .into_iter()
            .find_map(|ahead| {
                let from = feet + dir * ahead + Vec3::Y * (c.climb_max + 0.05);
                let hit = terrain.sweep(from, Vec3::NEG_Y * drop, 0.03, 0.06)?;
                let top = from + Vec3::NEG_Y * drop * hit.fraction;
                let height = top.y - feet.y;
                (hit.fraction > 0.0
                    && self.walkable(hit.normal)
                    && height > c.step
                    && height <= c.climb_max)
                    .then_some(top)
            })?;
        let height = top.y - feet.y;
        // Room to rise along the wall and to stand on the top.
        let up = Vec3::Y * (height + c.skin * 2.0);
        if self.sweep(terrain, feet, up).is_some() {
            return None;
        }
        let raised = feet + up;
        let over = dir * reach;
        if self
            .sweep(terrain, raised, over)
            .is_some_and(|h| h.fraction < 0.95)
        {
            return None;
        }
        let kind = if height <= c.vault_max {
            MantleKind::Vault
        } else {
            MantleKind::Climb
        };
        let duration = match kind {
            MantleKind::Vault => height / (c.climb_speed * 1.6) + 0.2,
            MantleKind::Climb => height / c.climb_speed + 0.35,
        };
        Some(Mantle {
            kind,
            from: feet,
            to: Vec3::new(raised.x + over.x, top.y + c.skin, raised.z + over.z),
            edge: Vec3::new(feet.x, top.y, feet.z) + dir * (c.radius + 0.05),
            wall_normal: -dir,
            t: 0.0,
            duration,
        })
    }

    /// Where a jump from `feet` at the planar `velocity` comes down: walkable
    /// ground no lower than a step below the take-off, found by sweeping the
    /// capsule along the jump's arc for up to [`LEAP_SECONDS`]. None when the
    /// arc meets a wall or a roof first, or falls past that height. A brain
    /// can ask it too, to tell a gap it can leap from one it cannot.
    #[must_use]
    pub fn find_landing(&self, terrain: &impl Terrain, feet: Vec3, velocity: Vec3) -> Option<Vec3> {
        let c = &self.config;
        let mut v = Vec3::new(velocity.x, c.jump_speed, velocity.z);
        if !v.is_finite() || Vec3::new(v.x, 0.0, v.z).length_squared() < 0.25 {
            return None;
        }
        let lowest = feet.y - c.step;
        let dt = 1.0 / 30.0;
        let mut p = feet;
        for _ in 0..(LEAP_SECONDS / dt) as usize {
            let delta = v * dt;
            if let Some(hit) = self.sweep(terrain, p, delta) {
                let at = p + delta * hit.fraction;
                return (v.y < 0.0 && self.walkable(hit.normal) && at.y >= lowest).then_some(at);
            }
            p += delta;
            v.y -= c.gravity * dt;
            if v.y < 0.0 && p.y < lowest {
                return None;
            }
        }
        None
    }

    /// The vault or climb under way, if any: its [`Mantle::progress`] and
    /// [`Mantle::vulnerable`] are what a brain reads (a mantle, once begun,
    /// always runs to its end).
    #[must_use]
    pub fn mantle(&self) -> Option<&Mantle> {
        match &self.mode {
            Mode::Mantle(m) => Some(m),
            _ => None,
        }
    }

    fn start_mantle(&mut self, mantle: Mantle, events: &mut Vec<LocomotionEvent>) {
        events.push(LocomotionEvent::MantleStarted {
            kind: mantle.kind,
            height: mantle.to.y - mantle.from.y,
            edge: mantle.edge,
            wall_normal: mantle.wall_normal,
        });
        self.velocity = Vec3::ZERO;
        self.mode = Mode::Mantle(mantle);
    }

    fn depth(&self, terrain: &impl Terrain) -> Option<f32> {
        terrain
            .water_surface(self.feet.x, self.feet.z)
            .map(|s| s - self.feet.y)
    }

    /// Advance one step.
    pub fn step(
        &mut self,
        dt: f32,
        input: &MoveInput,
        terrain: &impl Terrain,
    ) -> Vec<LocomotionEvent> {
        let mut events = Vec::new();
        if !dt.is_finite() || dt <= 0.0 {
            return events;
        }
        let want = Vec3::new(input.velocity.x, 0.0, input.velocity.z);
        let want = if want.is_finite() { want } else { Vec3::ZERO };
        match self.mode {
            Mode::Mantle(mut m) => {
                m.t += dt;
                self.feet = m.position();
                if m.t >= m.duration {
                    self.feet = m.to;
                    self.mode = Mode::Ground;
                    events.push(LocomotionEvent::MantleEnded);
                } else {
                    self.mode = Mode::Mantle(m);
                }
            }
            Mode::Ground => self.step_ground(dt, want, input, terrain, &mut events),
            Mode::Air => self.step_air(dt, want, input, terrain, &mut events),
            Mode::Swim => self.step_swim(dt, want, input, terrain, &mut events),
        }
        events
    }

    fn step_ground(
        &mut self,
        dt: f32,
        want: Vec3,
        input: &MoveInput,
        terrain: &impl Terrain,
        events: &mut Vec<LocomotionEvent>,
    ) {
        let c = self.config;
        let horizontal = Vec3::new(self.velocity.x, 0.0, self.velocity.z);
        let horizontal = if self.sledging {
            self.sled(dt, want, horizontal)
        } else {
            horizontal + (want - horizontal).clamp_length_max(c.acceleration * dt)
        };
        self.velocity = horizontal;
        if input.jump {
            if let Some(m) = self.find_ledge(terrain, self.feet, want) {
                self.start_mantle(m, events);
                return;
            }
            self.velocity.y = c.jump_speed;
            self.mode = Mode::Air;
            events.push(LocomotionEvent::Jumped);
            self.step_air(dt, want, input, terrain, events);
            return;
        }
        let delta = horizontal * dt;
        if delta.length_squared() > 1e-10 {
            let (applied, _) = self.slide(terrain, self.feet, delta, true);
            let stuck = Vec3::new(applied.x, 0.0, applied.z).length() < delta.length() * 0.5;
            if stuck {
                if let Some(over) = self.step_up(terrain, self.feet, delta) {
                    self.feet = over;
                } else {
                    self.feet += applied;
                    if input.climb
                        && let Some(m) = self.find_ledge(terrain, self.feet, delta)
                    {
                        self.start_mantle(m, events);
                        return;
                    }
                    // A wall: lose the speed into it.
                    self.velocity = applied / dt;
                }
            } else {
                self.feet += applied;
            }
        }
        match self.probe_ground(terrain, self.feet, c.snap) {
            Some((down, normal)) if self.walkable(normal) => {
                self.feet.y -= down - c.skin;
                self.ground_normal = normal;
            }
            _ => {
                self.mode = Mode::Air;
                self.velocity.y = 0.0;
                // Off an edge over a hole, holding climb, with ground to land
                // on across it: leap the gap instead of dropping into it.
                // Water below is dropped into (swum, then climbed out of).
                let ahead = self.feet + horizontal.normalize_or_zero() * c.radius * 2.0;
                let water = [self.feet, ahead]
                    .iter()
                    .any(|p| terrain.water_surface(p.x, p.z).is_some());
                if input.climb
                    && !water
                    && self.probe_ground(terrain, self.feet, c.step).is_none()
                    && self.find_landing(terrain, self.feet, horizontal).is_some()
                {
                    self.velocity.y = c.jump_speed;
                    events.push(LocomotionEvent::Jumped);
                }
            }
        }
        if let Some(depth) = self.depth(terrain)
            && depth >= c.swim_depth
        {
            self.mode = Mode::Swim;
            events.push(LocomotionEvent::EnteredWater);
        }
    }

    /// A sledge's ground velocity: gravity along the slope, steering, braking, snow.
    fn sled(&self, dt: f32, want: Vec3, v: Vec3) -> Vec3 {
        let c = &self.config;
        let n = self.ground_normal;
        // Gravity's pull along the slope, as seen on the ground plane.
        let mut v = v + Vec3::new(n.x, 0.0, n.z) * (c.gravity * n.y * dt);
        let speed = v.length();
        if let Some(w) = want.try_normalize() {
            match v.try_normalize() {
                Some(d) if w.dot(d) < -0.5 => v *= (1.0 - c.sledge_brake * dt).max(0.0),
                Some(d) => {
                    let angle = d.angle_between(w).min(c.sledge_turn * dt);
                    v = Quat::from_rotation_y(angle * d.cross(w).y.signum()) * d * speed;
                }
                None => {}
            }
            // Pushing off with the feet, on the flat or from a standstill.
            if speed < 2.0 {
                v += w * (2.0 * dt);
            }
        }
        v * (1.0 - c.sledge_friction * dt).max(0.0)
    }

    fn step_air(
        &mut self,
        dt: f32,
        want: Vec3,
        input: &MoveInput,
        terrain: &impl Terrain,
        events: &mut Vec<LocomotionEvent>,
    ) {
        let c = self.config;
        let horizontal = Vec3::new(self.velocity.x, 0.0, self.velocity.z);
        let change = (want - horizontal).clamp_length_max(c.acceleration * c.air_control * dt);
        self.velocity.x += change.x;
        self.velocity.z += change.z;
        self.velocity.y -= c.gravity * dt;
        let falling = -self.velocity.y;
        let delta = self.velocity * dt;
        let (applied, blocked) = self.slide(terrain, self.feet, delta, false);
        self.feet += applied;
        if delta.y > 0.0 && blocked.y > delta.y * 0.5 {
            // Head on a ceiling.
            self.velocity.y = 0.0;
        }
        let horizontal_block = Vec3::new(blocked.x, 0.0, blocked.z);
        if (input.climb || input.jump)
            && horizontal_block.length() > 1e-4
            && self.velocity.y < 2.0
            && let Some(m) = self.find_ledge(terrain, self.feet, Vec3::new(delta.x, 0.0, delta.z))
        {
            self.start_mantle(m, events);
            return;
        }
        if self.velocity.y <= 0.0
            && let Some((down, normal)) = self.probe_ground(terrain, self.feet, c.skin * 3.0)
            && self.walkable(normal)
        {
            self.feet.y -= down - c.skin;
            self.ground_normal = normal;
            self.mode = Mode::Ground;
            self.velocity.y = 0.0;
            events.push(LocomotionEvent::Landed {
                speed: falling.max(0.0),
            });
        }
        if let Some(depth) = self.depth(terrain)
            && depth >= c.swim_depth
        {
            self.mode = Mode::Swim;
            self.velocity.y *= 0.3;
            events.push(LocomotionEvent::EnteredWater);
        }
    }

    fn step_swim(
        &mut self,
        dt: f32,
        want: Vec3,
        input: &MoveInput,
        terrain: &impl Terrain,
        events: &mut Vec<LocomotionEvent>,
    ) {
        let c = self.config;
        let Some(surface) = terrain.water_surface(self.feet.x, self.feet.z) else {
            self.mode = Mode::Air;
            events.push(LocomotionEvent::LeftWater);
            return;
        };
        // Float with the chest at the surface.
        let target = surface - (c.swim_depth - 0.1);
        self.velocity.y = ((target - self.feet.y) * 4.0).clamp(-2.0, 2.0);
        let want = want.clamp_length_max(c.swim_speed);
        let horizontal = Vec3::new(self.velocity.x, 0.0, self.velocity.z);
        let change = (want - horizontal).clamp_length_max(c.acceleration * 0.5 * dt);
        self.velocity.x += change.x;
        self.velocity.z += change.z;
        let delta = self.velocity * dt;
        let (applied, blocked) = self.slide(terrain, self.feet, delta, false);
        self.feet += applied;
        let horizontal_block = Vec3::new(blocked.x, 0.0, blocked.z);
        if (input.climb || input.jump)
            && horizontal_block.length() > 1e-4
            && let Some(m) = self.find_ledge(terrain, self.feet, Vec3::new(delta.x, 0.0, delta.z))
        {
            self.start_mantle(m, events);
            events.push(LocomotionEvent::LeftWater);
            return;
        }
        // The bottom underfoot in shallow enough water: stand.
        let depth = surface - self.feet.y;
        if depth < c.swim_depth - 0.15
            && let Some((down, normal)) = self.probe_ground(terrain, self.feet, c.skin * 3.0)
            && self.walkable(normal)
        {
            self.feet.y -= down - c.skin;
            self.mode = Mode::Ground;
            self.velocity.y = 0.0;
            events.push(LocomotionEvent::LeftWater);
        }
    }
}

/// A world of boxes (and a water rectangle), for tests and tools.
pub mod boxes;

#[cfg(test)]
mod tests;
