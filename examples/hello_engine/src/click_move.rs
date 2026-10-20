//! The mouse as Diablo IV has it on PC, for the combat host's protagonist (the
//! developer, 20 Oct: "proper controls schemes from diablo iv and pillars of
//! eternity 2"):
//! - a left click on the ground walks him there, along the navmesh where there
//!   is one; held, he keeps steering for the cursor;
//! - a left click on a hostile walks him into reach and strikes it once; held,
//!   he keeps striking until it falls;
//! - WASD takes over at once (the click-move is dropped), and while it moves him
//!   the left button strikes toward the cursor as it always did.
//!
//! Each frame the host says what is under the cursor ([`pick`], and the ground
//! point) and [`ClickMove::steer`] writes the movement, aim and strike into the
//! frame's command. Right click (guard), 1-0, Space, Q and E are untouched.
use glam::Vec3;
use somnium_ai::navigation::NavWorld;
use somnium_combat::world::Phase;
use somnium_combat::{Action, CombatWorld, CombatantId, Command, Press};

/// Close enough to a clicked point to stop, metres.
const ARRIVE: f32 = 0.35;
/// A path corner counts as passed this near, metres.
const CORNER: f32 = 0.45;
/// Seconds a path is kept before it is planned again (at once if its goal moved a metre).
const REPLAN: f32 = 0.5;
/// How far off a body's axis a click still takes it, metres beyond its radius.
const PICK_SLACK: f32 = 0.3;
/// He stops this much inside his weapon's reach to strike.
const INSIDE: f32 = 0.3;

/// Where a click sent him.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Goal {
    /// A point on the ground.
    Ground(Vec3),
    /// A hostile to strike.
    Foe(CombatantId),
}

/// The left button's click-move and attack-move.
#[derive(Debug, Default)]
pub struct ClickMove {
    goal: Option<Goal>,
    held: bool,
    pressed: bool,
    /// A single click's blow is away.
    struck: bool,
    route: Vec<Vec3>,
    /// The goal the route was planned for, and its age.
    planned: Option<(Vec3, f32)>,
    hovered: Option<CombatantId>,
}

impl ClickMove {
    /// The left button went down (`true`) or up.
    pub fn button(&mut self, down: bool) {
        self.pressed |= down && !self.held;
        self.held = down;
    }

    /// Where the last click sent him, while he is still going (or striking).
    pub fn goal(&self) -> Option<Goal> {
        self.goal
    }

    /// The hostile under the cursor this frame (the host outlines it).
    pub fn hovered(&self) -> Option<CombatantId> {
        self.hovered
    }

    /// Drop the click-move (WASD, a load, a boat).
    pub fn cancel(&mut self) {
        self.goal = None;
        self.route.clear();
        self.planned = None;
    }

    /// This frame's command for `player`, over `cmd` as the keys made it:
    /// `under` is the hostile under the cursor, `ground` the cursor's point on
    /// the ground, `dt` the frame's seconds.
    #[allow(clippy::too_many_arguments)]
    pub fn steer(
        &mut self,
        w: &CombatWorld,
        nav: Option<&NavWorld>,
        player: CombatantId,
        under: Option<CombatantId>,
        ground: Option<Vec3>,
        dt: f32,
        cmd: &mut Command,
    ) {
        let pressed = std::mem::take(&mut self.pressed);
        self.hovered = under;
        if flat(cmd.move_dir).length_squared() > 0.01 {
            // WASD has him; the button strikes toward the cursor as it did.
            self.cancel();
            return;
        }
        cmd.attack = Press::default();
        if pressed {
            self.struck = false;
            self.goal = under.map(Goal::Foe).or(ground.map(Goal::Ground));
        } else if self.held
            && matches!(self.goal, Some(Goal::Ground(_)))
            && let Some(g) = ground
        {
            self.goal = Some(Goal::Ground(g));
        }
        let Some(me) = w.combatant(player) else {
            return;
        };
        match self.goal {
            None => {}
            Some(Goal::Ground(at)) => {
                if flat(at - me.position).length() < ARRIVE {
                    if !self.held {
                        self.cancel();
                    }
                    return;
                }
                cmd.move_dir = self.toward(nav, me.position, at, dt);
            }
            Some(Goal::Foe(id)) => {
                let Some(foe) = w.combatant(id).filter(|c| c.alive()) else {
                    self.cancel();
                    return;
                };
                cmd.aim = Some(foe.chest());
                let gap =
                    flat(foe.position - me.position).length() - foe.body.radius - me.body.radius;
                if gap > me.weapon.reach - INSIDE {
                    cmd.move_dir = self.toward(nav, me.position, foe.position, dt);
                    return;
                }
                self.route.clear();
                let idle = matches!(me.action, Action::Idle);
                if self.struck && !self.held {
                    // One click, one blow: done once it is through.
                    if idle {
                        self.cancel();
                    }
                    return;
                }
                // Held, the next blow chains from the last one's recovery.
                let chain = self.held
                    && matches!(&me.action, Action::Attack(a) if a.phase == Phase::Recovery);
                if idle || chain {
                    cmd.attack = Press::click();
                    self.struck = true;
                }
            }
        }
    }

    /// The way from `from` to `to`: the next corner of a path on `nav`, or straight there.
    fn toward(&mut self, nav: Option<&NavWorld>, from: Vec3, to: Vec3, dt: f32) -> Vec3 {
        let stale = self
            .planned
            .is_none_or(|(goal, age)| goal.distance(to) > 1.0 || age > REPLAN);
        if stale {
            self.route = nav
                .and_then(|n| n.path(from, to, 1.0))
                .map(|p| p.points)
                .unwrap_or_default();
            self.planned = Some((to, 0.0));
        }
        if let Some((_, age)) = &mut self.planned {
            *age += dt;
        }
        while self
            .route
            .first()
            .is_some_and(|c| flat(*c - from).length() < CORNER)
        {
            self.route.remove(0);
        }
        let next = self.route.first().copied().unwrap_or(to);
        flat(next - from).normalize_or_zero()
    }
}

/// The hostile (to `player`) whose body the ray from `origin` along `dir`
/// meets first: each a standing capsule from its feet to its head.
pub fn pick(w: &CombatWorld, player: CombatantId, origin: Vec3, dir: Vec3) -> Option<CombatantId> {
    let dir = dir.normalize_or_zero();
    w.combatants()
        .filter(|c| c.id != player && c.alive() && w.hostile(player, c.id))
        .filter_map(|c| {
            let top = c.position + Vec3::Y * c.body.height;
            let (t, d) = ray_to_segment(origin, dir, c.position, top);
            (d <= c.body.radius + PICK_SLACK).then_some((c.id, t))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// How far along a ray (from `o`, unit `d`) it comes nearest the segment `a`
/// to `b`, and how near.
fn ray_to_segment(o: Vec3, d: Vec3, a: Vec3, b: Vec3) -> (f32, f32) {
    let u = b - a;
    let w0 = o - a;
    let (dd, du, uu) = (d.dot(d), d.dot(u), u.dot(u));
    let (dw, uw) = (d.dot(w0), u.dot(w0));
    let denom = dd * uu - du * du;
    let s = if denom.abs() > 1e-6 {
        ((du * uw - uu * dw) / denom).max(0.0)
    } else {
        0.0
    };
    let t = ((uw + s * du) / uu.max(1e-6)).clamp(0.0, 1.0);
    let s = ((t * du - dw) / dd.max(1e-6)).max(0.0);
    (s, (o + d * s).distance(a + u * t))
}

fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ray_finds_the_nearest_point_of_a_standing_body() {
        // Straight down onto a body's axis from above, and past its side.
        let (t, d) = ray_to_segment(
            Vec3::new(0.0, 10.0, 0.0),
            -Vec3::Y,
            Vec3::ZERO,
            Vec3::Y * 1.8,
        );
        assert!((t - 8.2).abs() < 1e-4 && d < 1e-4, "{t} {d}");
        let (_, d) = ray_to_segment(
            Vec3::new(2.0, 1.0, -5.0),
            Vec3::Z,
            Vec3::ZERO,
            Vec3::Y * 1.8,
        );
        assert!((d - 2.0).abs() < 1e-4, "{d}");
    }

    #[test]
    fn wasd_drops_the_click_move() {
        let mut click = ClickMove::default();
        let mut w = CombatWorld::new(Default::default());
        let me = w.spawn(somnium_combat::CombatantDesc::new(
            "Hero",
            somnium_combat::TeamId(0),
            Vec3::ZERO,
            somnium_combat::weapon::sword(),
        ));
        click.button(true);
        click.button(false);
        let mut cmd = Command::default();
        click.steer(
            &w,
            None,
            me,
            None,
            Some(Vec3::new(5.0, 0.0, 0.0)),
            0.016,
            &mut cmd,
        );
        assert_eq!(click.goal(), Some(Goal::Ground(Vec3::new(5.0, 0.0, 0.0))));
        assert!(cmd.move_dir.x > 0.9, "off toward it");
        let mut keys = Command {
            move_dir: Vec3::Z,
            ..Command::default()
        };
        click.steer(&w, None, me, None, None, 0.016, &mut keys);
        assert_eq!(
            (click.goal(), keys.move_dir),
            (None, Vec3::Z),
            "the keys have him"
        );
    }
}
