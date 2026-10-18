//! Mounts, riders and mounted combat (ENGINE_ROADMAP 2.3: horse, frost-elk,
//! strider; rider IK).
//!
//! A mount is a combatant like any other: it has health, can be struck and can
//! die. While ridden it takes its rider's movement (WASD, sprint to gallop) and
//! the rider sits in the saddle, carried: the rider can still swing, guard and
//! cast from the saddle, and Gale Edge's forms come alive there (Kulan's Run
//! strikes harder, every cut flies as a Wind Blade). A galloping mount
//! tramples who it runs down. A heavy blow that floors the rider, or a mount
//! that dies, throws the rider.
//!
//! For the rider's pose the host asks [`rider_targets`] for the seat, the
//! stirrups and the reins, and [`pose_rider`] bends the legs (and the rein arm)
//! onto them with the engine's two-bone IK (`somnium_anim::TwoBoneIk`).
//!
//! Left alone, a mount keeps its own [`Herd`] mind: it grazes, ambles a few
//! metres from where it was left, bolts from whoever hurts it, and comes to a
//! [`whistle`] (its last rider's first, from up to [`WHISTLE_RANGE`]). It never
//! walks off a drop on its own; ridden, it goes where the rider sends it, and
//! [`CombatWorld::landed`] hurts a hard landing and throws the rider.
//!
//! On real ground the host moves a mount with its own controller, as it moves
//! people: [`MountDef::max_slope`], [`MountDef::step`] and
//! [`MountDef::safe_fall`] are its footing, and [`seat_riders`] puts riders
//! back in the saddle after the host has moved the mounts.
use crate::damage::{DamagePart, DamageType, Hit, HitFlags};
use crate::geometry::{flat, planar_dir};
use crate::world::{Body, CombatEvent, CombatWorld, Combatant, CombatantDesc};
use crate::{CombatQuery, CombatantId, Resistances, StatusKind, TeamId, Vitals};
use glam::Vec3;

/// How far a whistle carries, metres: across a field, well short of the next
/// village (about 570 m off at Rimeborn's scale).
pub const WHISTLE_RANGE: f32 = 150.0;
/// How far an unridden mount ambles from where it was left, metres.
pub const WANDER_RADIUS: f32 = 6.0;
/// The deepest drop an unridden mount steps down on its own, metres.
const SAFE_STEP_DOWN: f32 = 1.2;

/// Kinds of mount.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum MountKind {
    /// A horse: fast on open ground.
    Horse,
    /// A frost-elk: Tyffir's mount; sure on ice and snow, antlers in a charge.
    FrostElk,
    /// A strider: tall, long-legged, steady.
    Strider,
}

/// A mount's numbers.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MountDef {
    /// Kind.
    pub kind: MountKind,
    /// Walking speed, metres per second.
    pub speed: f32,
    /// Gallop multiplier while sprinting.
    pub gallop: f32,
    /// Saddle height above the ground.
    pub saddle: f32,
    /// Body length (nose to tail), metres.
    pub length: f32,
    /// Blunt harm when galloping into someone.
    pub trample: f32,
    /// Grip on ice (1 ignores it).
    pub sure_footed: f32,
    /// Steepest ground it climbs, degrees (a host's controller reads it).
    pub max_slope: f32,
    /// Highest step it walks over, metres.
    pub step: f32,
    /// Fastest landing that does not hurt it, metres per second (about a
    /// 4.5 m drop for a horse).
    pub safe_fall: f32,
}

impl MountKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 3] = [Self::Horse, Self::FrostElk, Self::Strider];

    /// Display names, in [`Self::ALL`] order.
    pub const NAMES: [&'static str; 3] = ["Horse", "Frost-elk", "Strider"];

    /// The library numbers.
    #[must_use]
    pub fn def(self) -> MountDef {
        match self {
            Self::Horse => MountDef {
                kind: self,
                speed: 4.5,
                gallop: 2.6,
                saddle: 1.5,
                length: 2.4,
                trample: 30.0,
                sure_footed: 0.0,
                max_slope: 34.0,
                step: 0.45,
                safe_fall: 9.5,
            },
            Self::FrostElk => MountDef {
                kind: self,
                speed: 4.2,
                gallop: 2.3,
                saddle: 1.45,
                length: 2.3,
                trample: 38.0,
                sure_footed: 1.0,
                max_slope: 40.0,
                step: 0.5,
                safe_fall: 10.0,
            },
            Self::Strider => MountDef {
                kind: self,
                speed: 4.0,
                gallop: 2.0,
                saddle: 2.3,
                length: 1.8,
                trample: 24.0,
                sure_footed: 0.5,
                max_slope: 30.0,
                step: 0.6,
                safe_fall: 8.5,
            },
        }
    }

    /// A combatant description for this mount, unridden, on `team`.
    #[must_use]
    pub fn desc(self, name: &str, team: TeamId, position: Vec3) -> CombatantDesc {
        let def = self.def();
        let mut desc = CombatantDesc::new(name, team, position, crate::weapon::unarmed());
        desc.body = Body {
            radius: 0.65,
            height: def.saddle + 0.3,
            mass: 450.0,
            speed: def.speed,
        };
        desc.vitals = Vitals::new(220.0, 160.0, 0.0, 90.0);
        desc.resist = Resistances::default().with_armor(6.0);
        desc.mount = Some(def);
        desc
    }
}

/// Where the rider's body should be: for the host's IK.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiderTargets {
    /// Pelvis.
    pub seat: Vec3,
    /// Left and right stirrup (foot targets).
    pub stirrups: [Vec3; 2],
    /// Reins (hand target for the free hand).
    pub reins: Vec3,
    /// The knees bend toward these (poles for the leg IK).
    pub knee_poles: [Vec3; 2],
}

/// Rider IK targets from a mount's pose.
#[must_use]
pub fn rider_targets(position: Vec3, facing: Vec3, def: &MountDef) -> RiderTargets {
    let forward = planar_dir(facing, Vec3::Z);
    let right = forward.cross(Vec3::Y);
    let seat = position + Vec3::Y * def.saddle;
    let foot =
        |side: f32| seat + right * side * 0.28 - Vec3::Y * (def.saddle * 0.55) + forward * 0.1;
    RiderTargets {
        seat,
        stirrups: [foot(-1.0), foot(1.0)],
        reins: seat + forward * 0.55 + Vec3::Y * 0.35,
        knee_poles: [
            seat + forward * 1.0 - right * 0.3,
            seat + forward * 1.0 + right * 0.3,
        ],
    }
}

/// The rider's IK chains: hip, knee and ankle for each leg, shoulder, elbow
/// and wrist for the hand on the reins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiderRig {
    /// Left leg.
    pub left_leg: somnium_anim::TwoBoneIk,
    /// Right leg.
    pub right_leg: somnium_anim::TwoBoneIk,
    /// The arm that holds the reins, if the rider has a hand free.
    pub reins_arm: Option<somnium_anim::TwoBoneIk>,
}

/// Bend a rider's legs onto the stirrups (knees forward) and the rein hand onto
/// the reins. `rider_world` is the rider's model-to-world matrix; `weight`
/// blends from the animated pose (0) to fully seated (1), so mounting and
/// dismounting ease in and out.
pub fn pose_rider(
    pose: &mut somnium_anim::Pose,
    skeleton: &somnium_anim::Skeleton,
    rig: &RiderRig,
    targets: &RiderTargets,
    rider_world: glam::Mat4,
    weight: f32,
) -> Result<(), somnium_anim::PoseEditError> {
    if !rider_world.is_finite() || rider_world.determinant().abs() < 1e-8 {
        return Err(somnium_anim::PoseEditError::InvalidTarget);
    }
    let to_model = rider_world.inverse();
    let m = |p: Vec3| to_model.transform_point3(p);
    rig.left_leg.solve(
        pose,
        skeleton,
        m(targets.stirrups[0]),
        m(targets.knee_poles[0]),
        weight,
    )?;
    rig.right_leg.solve(
        pose,
        skeleton,
        m(targets.stirrups[1]),
        m(targets.knee_poles[1]),
        weight,
    )?;
    if let Some(arm) = rig.reins_arm {
        let elbow_pole = targets.reins - Vec3::Y * 0.5;
        arm.solve(pose, skeleton, m(targets.reins), m(elbow_pole), weight)?;
    }
    Ok(())
}

/// Climb into the nearest free saddle within reach, or leave the one you are in.
pub fn toggle(w: &mut CombatWorld, rider: CombatantId) -> Result<(), MountError> {
    let r = w.combatant(rider).ok_or(MountError::Missing)?;
    if r.mount.is_some() {
        dismount(w, rider);
        return Ok(());
    }
    let at = r.position;
    let nearest = w
        .combatants()
        .filter(|c| c.mount_def.is_some() && c.rider.is_none() && c.alive() && c.id != rider)
        .min_by(|a, b| {
            flat(a.position - at)
                .length()
                .total_cmp(&flat(b.position - at).length())
        })
        .map(|c| c.id)
        .ok_or(MountError::Missing)?;
    mount(w, rider, nearest)
}

/// Why mounting failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountError {
    /// No such rider or mount.
    Missing,
    /// The mount is not a mount.
    NotAMount,
    /// Someone is in the saddle.
    Taken,
    /// Too far away.
    TooFar,
    /// Dead, down, busy or already riding.
    Busy,
}

/// Make an already spawned combatant a mount of `kind` ([`MountKind::desc`]
/// already does this for a new one).
pub fn make_mount(w: &mut CombatWorld, id: CombatantId, kind: MountKind) {
    if let Some(c) = w.combatant_mut(id) {
        c.mount_def = Some(kind.def());
        c.herd.get_or_insert(Herd::new(c.position));
    }
}

/// Climb into the saddle.
pub fn mount(
    w: &mut CombatWorld,
    rider: CombatantId,
    steed: CombatantId,
) -> Result<(), MountError> {
    let r = w.combatant(rider).ok_or(MountError::Missing)?;
    let m = w.combatant(steed).ok_or(MountError::Missing)?;
    if m.mount_def.is_none() {
        return Err(MountError::NotAMount);
    }
    if m.rider.is_some() {
        return Err(MountError::Taken);
    }
    if flat(r.position - m.position).length() > 2.8 {
        return Err(MountError::TooFar);
    }
    if !r.alive() || !m.alive() || r.mount.is_some() || !r.status.can_act() {
        return Err(MountError::Busy);
    }
    if let Some(r) = w.combatant_mut(rider) {
        r.mount = Some(steed);
    }
    if let Some(m) = w.combatant_mut(steed) {
        m.rider = Some(rider);
        // A ridden mount fights for its rider.
        m.brain = None;
    }
    w.emit(CombatEvent::Mounted {
        rider,
        mount: Some(steed),
    });
    sync_riders(w);
    Ok(())
}

/// Leave the saddle (or be thrown from it), landing beside the mount.
pub fn dismount(w: &mut CombatWorld, rider: CombatantId) {
    let Some(steed) = w.combatant(rider).and_then(|r| r.mount) else {
        return;
    };
    let (beside, ground) = w
        .combatant(steed)
        .map(|m| (m.position + m.facing.cross(Vec3::Y) * -1.1, m.position.y))
        .unwrap_or((Vec3::ZERO, 0.0));
    if let Some(r) = w.combatant_mut(rider) {
        r.mount = None;
        r.position = Vec3::new(beside.x, ground, beside.z);
    }
    let now = w.time();
    if let Some(m) = w.combatant_mut(steed) {
        m.rider = None;
        m.command = Default::default();
        // Left here: it grazes here and answers this rider's whistle first.
        m.herd = Some(Herd {
            home: m.position,
            owner: Some(rider),
            doing: Herding::Grazing,
            until: now + 4.0,
        });
    }
    w.emit(CombatEvent::Mounted { rider, mount: None });
}

/// What an unridden mount is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Herding {
    /// Head down, standing.
    Grazing,
    /// Ambling to a spot near home.
    Wandering(Vec3),
    /// Answering a whistle.
    Coming(CombatantId),
    /// Running from harm, this way.
    Bolting(Vec3),
}

impl Herding {
    /// A word for the host's animation and readouts.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Grazing => "grazing",
            Self::Wandering(_) => "wandering",
            Self::Coming(_) => "coming",
            Self::Bolting(_) => "bolting",
        }
    }
}

/// An unridden mount's own mind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Herd {
    /// Where it was left; it ambles near here.
    pub home: Vec3,
    /// Whose whistle it answers first: its last rider.
    pub owner: Option<CombatantId>,
    /// What it is doing.
    pub doing: Herding,
    /// World time the current doing ends.
    pub until: f64,
}

impl Herd {
    /// Grazing where it stands, for a few seconds.
    #[must_use]
    pub fn new(home: Vec3) -> Self {
        Self {
            home,
            owner: None,
            doing: Herding::Grazing,
            until: 3.0,
        }
    }
}

/// Whistle for a mount: the whistler's own (the last it carried) if it is in
/// earshot, else the nearest unridden mount of the whistler's side. It comes
/// at a walk, galloping when far. Everyone near hears the whistle too.
pub fn whistle(w: &mut CombatWorld, who: CombatantId) -> Option<CombatantId> {
    let c = w.combatant(who)?;
    if c.mount.is_some() || !c.alive() {
        return None;
    }
    let (at, team) = (c.position, c.team);
    let distance = |m: &Combatant| flat(m.position - at).length();
    let free = |m: &&Combatant| {
        m.herd.is_some() && m.rider.is_none() && m.alive() && distance(m) <= WHISTLE_RANGE
    };
    let nearest = |own: bool| {
        w.combatants()
            .filter(free)
            .filter(|m| {
                if own {
                    m.herd.is_some_and(|h| h.owner == Some(who))
                } else {
                    m.team == team
                }
            })
            .min_by(|a, b| distance(a).total_cmp(&distance(b)))
            .map(|m| m.id)
    };
    let called = nearest(true).or_else(|| nearest(false));
    let now = w.time();
    if let Some(herd) = called
        .and_then(|id| w.combatant_mut(id))
        .and_then(|m| m.herd.as_mut())
    {
        herd.doing = Herding::Coming(who);
        herd.until = now + 45.0;
        herd.owner = Some(who);
    }
    w.noise(who, at, 0.9);
    w.emit(CombatEvent::Whistled { who, mount: called });
    called
}

/// Unridden mounts graze, amble near home, bolt from harm and answer
/// whistles; none walks off a drop on its own (before commands are read).
pub(crate) fn herd(w: &mut CombatWorld, query: &impl CombatQuery) {
    let now = w.time();
    let ids: Vec<CombatantId> = w
        .combatants()
        .filter(|c| c.herd.is_some() && c.rider.is_none() && c.alive())
        .map(|c| c.id)
        .collect();
    for id in ids {
        let Some(c) = w.combatant(id) else {
            continue;
        };
        let Some(mut herd) = c.herd else {
            continue;
        };
        let (at, facing) = (c.position, c.facing);
        let reach = c.mount_def.map_or(1.2, |d| d.length * 0.5) + 0.6;
        let attacker = c
            .last_attacker
            .and_then(|a| w.combatant(a))
            .map(|a| a.position);
        let struck = now - c.last_hurt < 0.25;
        let caller = match herd.doing {
            Herding::Coming(who) => w
                .combatant(who)
                .filter(|c| c.alive() && c.mount.is_none())
                .map(|c| c.position),
            _ => None,
        };
        if struck && !matches!(herd.doing, Herding::Bolting(_)) {
            let from = attacker.unwrap_or(at + facing);
            herd.doing = Herding::Bolting(planar_dir(at - from, -facing));
            herd.until = now + 3.0;
        }
        let settle = |herd: &mut Herd, rest: f64| {
            herd.doing = Herding::Grazing;
            herd.home = at;
            herd.until = now + rest;
        };
        let mut aim = None;
        let (mut dir, sprint) = match herd.doing {
            Herding::Grazing => {
                if now >= herd.until {
                    let a = w.rng.unit() * std::f32::consts::TAU;
                    let r = WANDER_RADIUS * (0.3 + 0.7 * w.rng.unit());
                    let to = herd.home + Vec3::new(a.cos() * r, 0.0, a.sin() * r);
                    herd.doing = Herding::Wandering(Vec3::new(to.x, at.y, to.z));
                    herd.until = now + 15.0;
                }
                (Vec3::ZERO, false)
            }
            Herding::Wandering(to) => {
                let d = flat(to - at);
                if d.length() < 0.5 || now >= herd.until {
                    herd.doing = Herding::Grazing;
                    herd.until = now + 4.0 + 6.0 * f64::from(w.rng.unit());
                    (Vec3::ZERO, false)
                } else {
                    // An amble: a little under half a walk.
                    (d.normalize() * 0.4, false)
                }
            }
            Herding::Coming(_) => match caller {
                Some(p) if flat(p - at).length() < 3.0 => {
                    aim = Some(p);
                    settle(&mut herd, 8.0);
                    (Vec3::ZERO, false)
                }
                Some(p) if now < herd.until => {
                    let d = flat(p - at);
                    (d.normalize(), d.length() > 12.0)
                }
                _ => {
                    settle(&mut herd, 4.0);
                    (Vec3::ZERO, false)
                }
            },
            Herding::Bolting(away) => {
                if now >= herd.until {
                    settle(&mut herd, 5.0);
                    (Vec3::ZERO, false)
                } else {
                    (away, true)
                }
            }
        };
        // Never off a drop: stop at the edge, and give up on the spot (an
        // answered whistle waits there instead).
        if dir.length_squared() > 1e-4 && !footing(query, at, planar_dir(dir, facing), reach) {
            dir = Vec3::ZERO;
            if !matches!(herd.doing, Herding::Coming(_)) {
                settle(&mut herd, 3.0);
            }
        }
        if let Some(c) = w.combatant_mut(id) {
            c.herd = Some(herd);
            c.command.move_dir = dir;
            c.command.sprint = sprint;
            c.command.aim = aim;
        }
    }
}

/// Whether there is ground `reach` ahead no deeper than a safe step down. A
/// world that reports no ground under the body itself (an open test field)
/// counts as flat.
fn footing(query: &impl CombatQuery, at: Vec3, dir: Vec3, reach: f32) -> bool {
    let (lift, radius) = (1.5, 0.15);
    let depth = lift + SAFE_STEP_DOWN + 0.5;
    let ground = |p: Vec3| {
        let top = p + Vec3::Y * lift;
        query
            .sweep(top, top - Vec3::Y * depth, radius)
            .map(|h| top.y - depth * h.fraction - radius)
    };
    let Some(here) = ground(at) else {
        return true;
    };
    ground(at + dir * reach).is_some_and(|there| there > here - SAFE_STEP_DOWN)
}

/// Put riders in their saddles where their mounts now stand. [`CombatWorld::step`]
/// does it; a host whose controller moves the mounts after the step calls it
/// again once they have moved.
pub fn seat_riders(w: &mut CombatWorld) {
    let pairs: Vec<(CombatantId, CombatantId)> = w
        .combatants()
        .filter_map(|c| Some((c.id, c.mount?)))
        .collect();
    for (rider, steed) in pairs {
        let Some(seat) = w
            .combatant(steed)
            .and_then(|m| Some(rider_targets(m.position, m.facing, &m.mount_def?).seat))
        else {
            continue;
        };
        if let Some(r) = w.combatant_mut(rider) {
            // The rider's feet hang from the seat; its capsule rides above the mount's.
            r.position = seat - Vec3::Y * r.body.height * 0.45;
        }
    }
}

/// Hand each ridden mount its rider's movement (before commands are read).
pub(crate) fn drive(w: &mut CombatWorld) {
    let pairs: Vec<(CombatantId, CombatantId)> = w
        .combatants()
        .filter_map(|c| Some((c.id, c.mount?)))
        .collect();
    for (rider, steed) in pairs {
        let Some(r) = w.combatant(rider) else {
            continue;
        };
        let (move_dir, sprint, aim) = (r.command.move_dir, r.command.sprint, r.command.aim);
        if let Some(m) = w.combatant_mut(steed) {
            m.command.move_dir = move_dir;
            m.command.sprint = sprint;
            m.command.aim = aim.filter(|_| move_dir.length_squared() < 0.01);
            if let Some(def) = m.mount_def {
                m.body.speed = def.speed;
            }
        }
    }
}

/// Seat riders on their mounts, throw floored riders, and trample (after movement).
pub(crate) fn sync_riders(w: &mut CombatWorld) {
    let pairs: Vec<(CombatantId, CombatantId)> = w
        .combatants()
        .filter_map(|c| Some((c.id, c.mount?)))
        .collect();
    seat_riders(w);
    for (rider, steed) in pairs {
        let thrown = w
            .combatant(rider)
            .is_some_and(|r| r.status.active(StatusKind::Knockdown) || !r.alive());
        if thrown {
            dismount(w, rider);
            continue;
        }
        let Some(m) = w.combatant(steed) else {
            continue;
        };
        let Some(def) = m.mount_def else {
            continue;
        };
        let (facing, galloping, velocity, position) =
            (m.facing, m.intent.sprinting, m.intent.velocity, m.position);
        if galloping && velocity.length() > def.speed * 1.4 {
            trample(
                w,
                rider,
                steed,
                position,
                planar_dir(velocity, facing),
                &def,
            );
        }
    }
}

fn trample(
    w: &mut CombatWorld,
    rider: CombatantId,
    steed: CombatantId,
    position: Vec3,
    forward: Vec3,
    def: &MountDef,
) {
    let now = w.time();
    let victims: Vec<(CombatantId, Vec3)> = w
        .combatants()
        .filter(|c| c.alive() && c.id != rider && c.id != steed && c.mount.is_none())
        .filter(|c| w.hostile(rider, c.id))
        .filter(|c| {
            let to = flat(c.position - position);
            to.length() < def.length * 0.6 + c.body.radius
                && to.normalize_or_zero().dot(forward) > 0.3
        })
        .filter(|c| now - c.last_hurt > 1.0)
        .map(|c| (c.id, c.chest()))
        .collect();
    for (id, point) in victims {
        let mut hit = Hit::new(
            Some(rider),
            vec![DamagePart::new(DamageType::Blunt, def.trample)],
            80.0,
        );
        hit.flags = HitFlags::MELEE | HitFlags::HEAVY | HitFlags::UNPARRYABLE;
        hit.knockback = forward * 5.0;
        hit.direction = forward;
        hit.point = point;
        w.queue_hit(id, hit, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_riders_feet_find_the_stirrups() {
        use somnium_anim::{NO_PARENT, Pose, Skeleton, SkeletonId, Transform, TwoBoneIk};
        let t = |x: f32, y: f32| Transform {
            translation: Vec3::new(x, y, 0.0),
            ..Transform::IDENTITY
        };
        // Pelvis, then hip, knee and ankle on each side.
        let (skeleton, _) = Skeleton::new(
            SkeletonId(9),
            [
                "pelvis", "hip_l", "knee_l", "ankle_l", "hip_r", "knee_r", "ankle_r",
            ]
            .map(String::from)
            .to_vec(),
            vec![NO_PARENT, 0, 1, 2, 0, 4, 5],
            vec![glam::Mat4::IDENTITY; 7],
            vec![
                t(0.0, 1.0),
                // Facing +Z, the left side is +X.
                t(0.1, 0.0),
                t(0.0, -0.45),
                t(0.0, -0.45),
                t(-0.1, 0.0),
                t(0.0, -0.45),
                t(0.0, -0.45),
            ],
        )
        .unwrap();
        let def = MountKind::Horse.def();
        let targets = rider_targets(Vec3::ZERO, Vec3::Z, &def);
        // The model's pelvis sits on the seat.
        let rider_world = glam::Mat4::from_translation(targets.seat - Vec3::Y);
        let rig = RiderRig {
            left_leg: TwoBoneIk {
                root: 1,
                middle: 2,
                end: 3,
            },
            right_leg: TwoBoneIk {
                root: 4,
                middle: 5,
                end: 6,
            },
            reins_arm: None,
        };
        let mut pose = Pose::rest(&skeleton);
        pose_rider(&mut pose, &skeleton, &rig, &targets, rider_world, 1.0).unwrap();
        let mut model = vec![glam::Mat4::IDENTITY; skeleton.len()];
        assert!(pose.to_model_space(&skeleton, &mut model));
        for (ankle, stirrup) in [(3usize, targets.stirrups[0]), (6, targets.stirrups[1])] {
            let at = rider_world.transform_point3(model[ankle].w_axis.truncate());
            assert!(
                at.distance(stirrup) < 1e-3,
                "ankle {ankle} at {at}, stirrup {stirrup}"
            );
        }
        // Knees bend forward, toward the poles.
        let knee = rider_world.transform_point3(model[2].w_axis.truncate());
        assert!(knee.z > targets.seat.z, "the knee comes forward: {knee}");
    }

    #[test]
    fn the_stirrups_hang_below_the_seat_either_side() {
        let def = MountKind::Horse.def();
        let t = rider_targets(Vec3::ZERO, Vec3::Z, &def);
        assert!((t.seat.y - def.saddle).abs() < 1e-5);
        assert!(t.stirrups[0].y < t.seat.y && t.stirrups[1].y < t.seat.y);
        assert!(t.stirrups[0].x * t.stirrups[1].x < 0.0, "one each side");
        assert!(t.reins.z > t.seat.z, "reins ahead");
    }
}
