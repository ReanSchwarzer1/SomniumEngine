//! Cover for ranged fighters: behind something while reloading, then out
//! again to where the target is in sight. The spots and their judging are
//! `somnium_ai::cover`'s (rings around the body, snapped to the navmesh when
//! there is one, judged by the host's own sight lines, so pillars, walls and
//! ledges all count); a fighter keeps only those inside its shooting band
//! with room for its body.
use super::{Host, Seen};
use crate::CombatQuery;
use crate::geometry::flat;
use crate::world::Action;
use glam::Vec3;
use somnium_ai::behavior::Status;
use somnium_ai::cover::{Stance, Threat, candidates, cover, firing, judge};

/// Seconds a chosen spot is trusted before it is judged again.
const SPOT_SECONDS: f64 = 1.0;
/// Rings sampled around the body, nearest first, eight spots each.
const RINGS: [f32; 3] = [1.5, 3.0, 4.5];

impl<Q: CombatQuery> Host<'_, Q> {
    /// The nearest spot inside the shooting band with room to stand that
    /// is `hidden` from the target, or else has a clear shot at it.
    fn spot(&self, t: &Seen, hidden: bool) -> Option<Vec3> {
        let me = self.me.position;
        let band = self.band();
        let spots: Vec<Vec3> = candidates(me, &RINGS, 8, self.nav)
            .into_iter()
            .filter(|&p| {
                let d = flat(t.position - p).length();
                // Room for the body: nothing between knee and over the head.
                d >= band[0]
                    && d <= band[1]
                    && self
                        .query
                        .sweep(p + Vec3::Y * 2.6, p + Vec3::Y * 0.4, self.me.radius)
                        .is_none()
            })
            .collect();
        let stance = |s: &Seen| Stance {
            eye: s.eye.y - s.position.y,
            chest: s.chest.y - s.position.y,
        };
        let threat = Threat {
            feet: t.position,
            stance: stance(t),
        };
        let judged = judge(me, &spots, stance(&self.me), &[threat], |a, b| {
            self.query.blocked(a, b)
        });
        if hidden {
            cover(&judged, 1)
        } else {
            firing(&judged)
        }
        .map(|s| s.point)
    }

    /// [`Self::spot`], kept for [`SPOT_SECONDS`].
    fn kept_spot(&mut self, t: &Seen, hidden: bool) -> Option<Vec3> {
        if let Some((p, h, at)) = self.mind.spot
            && h == hidden
            && self.now - at < SPOT_SECONDS
        {
            return Some(p);
        }
        let p = self.spot(t, hidden)?;
        self.mind.spot = Some((p, hidden, self.now));
        Some(p)
    }

    /// While reloading, get behind something the target cannot see through.
    /// Fails when not reloading or when nothing nearby hides it, so the tree
    /// goes on to shoot.
    pub(super) fn take_cover(&mut self) -> Status {
        let Some(t) = self.target() else {
            return Status::Failure;
        };
        if !self.mind.ranged
            || !matches!(self.action, Action::Reload { .. })
            || self.formation_row().is_some()
        {
            return Status::Failure;
        }
        self.face(t.chest);
        let Some(p) = self.kept_spot(&t, true) else {
            return Status::Failure;
        };
        self.mind.doing = "Taking cover";
        if self.go(p, false) {
            Status::Success
        } else {
            Status::Running
        }
    }

    /// The target is out of sight (behind cover, a pillar between): step to
    /// the nearest spot with a clear shot. Fails when the shot is already
    /// clear, or there is no such spot near.
    pub(super) fn find_shot(&mut self) -> Status {
        let Some(t) = self.target() else {
            return Status::Failure;
        };
        if !self.mind.ranged || !self.query.blocked(self.me.eye, t.chest) {
            return Status::Failure;
        }
        let Some(p) = self.kept_spot(&t, false) else {
            return Status::Failure;
        };
        self.face(t.chest);
        self.mind.doing = "Stepping out";
        if self.go(p, false) {
            Status::Success
        } else {
            Status::Running
        }
    }
}
