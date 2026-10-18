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
//! stirrups and the reins and solves the legs and arms with the engine's
//! two-bone IK (`somnium_anim::TwoBoneIk`).
use crate::damage::{DamagePart, DamageType, Hit, HitFlags};
use crate::geometry::{flat, planar_dir};
use crate::world::{Body, CombatEvent, CombatWorld, CombatantDesc};
use crate::{CombatantId, Resistances, StatusKind, TeamId, Vitals};
use glam::Vec3;

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
}

impl MountKind {
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
            },
            Self::FrostElk => MountDef {
                kind: self,
                speed: 4.2,
                gallop: 2.3,
                saddle: 1.45,
                length: 2.3,
                trample: 38.0,
                sure_footed: 1.0,
            },
            Self::Strider => MountDef {
                kind: self,
                speed: 4.0,
                gallop: 2.0,
                saddle: 2.3,
                length: 1.8,
                trample: 24.0,
                sure_footed: 0.5,
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

/// Make a combatant a mount of `kind` (after spawning it from [`MountKind::desc`]).
pub fn make_mount(w: &mut CombatWorld, id: CombatantId, kind: MountKind) {
    if let Some(c) = w.combatant_mut(id) {
        c.mount_def = Some(kind.def());
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
    if let Some(m) = w.combatant_mut(steed) {
        m.rider = None;
        m.command = Default::default();
    }
    w.emit(CombatEvent::Mounted { rider, mount: None });
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
        let targets = rider_targets(m.position, m.facing, &def);
        let (facing, galloping, velocity, position) =
            (m.facing, m.intent.sprinting, m.intent.velocity, m.position);
        if let Some(r) = w.combatant_mut(rider) {
            // The rider's feet hang from the seat; its capsule rides above the mount's.
            r.position = targets.seat - Vec3::Y * r.body.height * 0.45;
            if !matches!(r.action, crate::Action::Attack(_)) && r.command.aim.is_none() {
                r.facing = facing;
            }
        }
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
    fn the_stirrups_hang_below_the_seat_either_side() {
        let def = MountKind::Horse.def();
        let t = rider_targets(Vec3::ZERO, Vec3::Z, &def);
        assert!((t.seat.y - def.saddle).abs() < 1e-5);
        assert!(t.stirrups[0].y < t.seat.y && t.stirrups[1].y < t.seat.y);
        assert!(t.stirrups[0].x * t.stirrups[1].x < 0.0, "one each side");
        assert!(t.reins.z > t.seat.z, "reins ahead");
    }
}
