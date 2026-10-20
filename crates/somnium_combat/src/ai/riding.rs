//! Brains in the saddle. A rider on the road walks its horse. Against a foe,
//! one with a lance-glaive or a spear ([`fights_mounted`]) charges at the
//! gallop, lowers its point [`STRIKE_RANGE`] out, rides on through and wheels
//! about for the next pass; one with anything shorter rides up and gets down
//! [`DISMOUNT_RANGE`] out to fight on foot, its horse left grazing where it
//! stood. A rider that breaks runs as its tree sends it.
use super::{Mind, Seen, find};
use crate::geometry::flat;
use crate::weapon::WeaponKind;
use crate::world::{Command, Press};
use glam::Vec3;

/// A walking horse's share of its pace (about 1.8 m/s for a horse: a patrol's walk).
const WALK: f32 = 0.4;
/// A short-armed rider gets down this far from its foe, metres.
pub const DISMOUNT_RANGE: f32 = 10.0;
/// A lancer lowers its point this far out at the gallop; the windup carries it the rest.
pub const STRIKE_RANGE: f32 = 6.5;
/// Seconds a lancer rides on after striking, before it wheels about.
const RIDE_ON: f64 = 1.6;
/// Closer than this with no pass under way, a lancer rides off for a run-up.
const RUN_UP: f32 = 3.5;

/// Where a mounted brain is in its passes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum Pass {
    /// Riding at the foe.
    #[default]
    Run,
    /// Riding on this way until then (through after a strike, or off for a run-up).
    On(Vec3, f64),
}

/// Whether a rider with this weapon fights from the saddle (a lance-glaive or a
/// spear) rather than getting down.
#[must_use]
pub fn fights_mounted(kind: WeaponKind) -> bool {
    matches!(kind, WeaponKind::LanceGlaive | WeaponKind::Spear)
}

/// This step's command for a rider, over what its tree asked. True: get down now.
pub(super) fn steer(
    me: &Seen,
    mind: &mut Mind,
    view: &[Seen],
    cmd: &mut Command,
    lancer: bool,
    now: f64,
) -> bool {
    let target = mind.target.and_then(|t| find(view, t)).copied();
    let Some(t) = target.filter(|_| !mind.fleeing) else {
        // On the road at a walk; running, as the tree runs.
        if !cmd.sprint {
            cmd.move_dir *= WALK;
        }
        mind.pass = Pass::Run;
        return false;
    };
    let to = flat(t.position - me.position);
    let d = to.length();
    let toward = to / d.max(1e-3);
    cmd.guard = Press::default();
    cmd.attack = Press::default();
    if !lancer {
        if d <= DISMOUNT_RANGE {
            mind.doing = "Dismounting";
            return true;
        }
        cmd.move_dir = toward;
        cmd.sprint = d > 20.0;
        cmd.aim = Some(t.chest);
        mind.doing = "Riding up";
        return false;
    }
    if let Pass::On(way, until) = mind.pass {
        if now < until {
            cmd.move_dir = way;
            cmd.sprint = true;
            cmd.aim = None;
            mind.doing = "Riding on";
            return false;
        }
        mind.pass = Pass::Run;
    }
    if d < RUN_UP {
        mind.pass = Pass::On(-toward, now + RIDE_ON);
        cmd.move_dir = -toward;
        cmd.sprint = true;
        mind.doing = "Riding off";
        return false;
    }
    cmd.move_dir = toward;
    cmd.sprint = true;
    cmd.aim = Some(t.chest);
    mind.doing = "Charging";
    if d <= STRIKE_RANGE && !me.busy {
        // Point down at the gallop (the weapon's charge), and on through.
        cmd.attack = Press::tap();
        mind.pass = Pass::On(toward, now + RIDE_ON);
    }
    false
}
