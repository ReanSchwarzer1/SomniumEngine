//! Simple formations: the melee abreast in a line, the ranged and casters
//! in a row behind it. The group walks its anchor toward the enemy any of
//! them is after, holds a few strides short, and the line breaks to fight
//! when an enemy comes close; the row behind shoots over it. The shape (the
//! places, the standing order, the walking anchor) is
//! `somnium_ai::formation`'s; this keeps who is in it and when it breaks.
use super::{Archetype, Host, Seen};
use crate::geometry::{flat, planar_dir};
use crate::{CombatQuery, CombatWorld, CombatantId};
use glam::Vec3;
use somnium_ai::behavior::Status;
use somnium_ai::formation::Formation as Shape;

/// The anchor stops this far from the enemy ...
const HOLD_AT: f32 = 9.0;
/// ... and walks toward it at this pace, slower than anyone walks, so the
/// group keeps together; it waits while anyone is more than
/// [`CATCH_UP`] from their place (more than the row's slack, so archers
/// loosing from their place do not stop the line).
const ADVANCE: f32 = 1.4;
const CATCH_UP: f32 = 3.0;
/// A melee member leaves the line for an enemy this close.
const BREAK: f32 = 5.0;
/// A member of the row is in place this near its spot.
const ROW_SLACK: f32 = 2.5;

/// A group holding a formation (see [`form_up`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Formation {
    /// Members, left to right as they stood when they formed up.
    pub members: Vec<CombatantId>,
    /// The line and the row: anchor, facing and spacing.
    pub shape: Shape,
    /// Each member's place, and whether it is in the line (not the row).
    pub slots: Vec<(CombatantId, Vec3, bool)>,
}

/// One member's place.
#[derive(Clone, Copy, Debug)]
pub(super) struct Place {
    pub slot: Vec3,
    pub front: bool,
}

/// Form `members` up: melee in a line, archers, riflemen and casters in a
/// row behind, around where the melee stand now and facing the way they
/// face. Members leave any formation they were in; fewer than two (or
/// none with a brain) form nothing.
pub fn form_up(w: &mut CombatWorld, members: &[CombatantId]) {
    let members: Vec<CombatantId> = members
        .iter()
        .copied()
        .filter(|id| w.combatant(*id).is_some_and(|c| c.brain.is_some()))
        .collect();
    for f in &mut w.tactics.formations {
        f.members.retain(|m| !members.contains(m));
    }
    w.tactics.formations.retain(|f| f.members.len() >= 2);
    if members.len() < 2 {
        return;
    }
    let line: Vec<&crate::world::Combatant> = members
        .iter()
        .filter_map(|id| w.combatant(*id))
        .filter(|c| !in_row(c))
        .collect();
    let pick = if line.is_empty() {
        members.iter().filter_map(|id| w.combatant(*id)).collect()
    } else {
        line
    };
    let anchor = pick.iter().map(|c| c.position).sum::<Vec3>() / pick.len() as f32;
    let facing = planar_dir(pick.iter().map(|c| c.facing).sum::<Vec3>(), Vec3::Z);
    let shape = Shape::new(anchor, facing);
    // Left to right as they stand, so nobody crosses the line to its place.
    let positions: Vec<Vec3> = members
        .iter()
        .map(|id| w.combatant(*id).map_or(anchor, |c| c.position))
        .collect();
    let members = shape
        .left_to_right(&positions)
        .into_iter()
        .map(|i| members[i])
        .collect();
    let mut f = Formation {
        members,
        shape,
        slots: Vec::new(),
    };
    place(w, &mut f);
    w.tactics.formations.push(f);
}

fn in_row(c: &crate::world::Combatant) -> bool {
    c.brain.as_ref().is_some_and(|b| {
        matches!(
            b.mind.archetype,
            Archetype::Archer | Archetype::Rifleman | Archetype::Caster
        )
    })
}

/// Everyone's places from the shape, in the members' order.
fn place(w: &CombatWorld, f: &mut Formation) {
    let (row, line): (Vec<CombatantId>, Vec<CombatantId>) = f
        .members
        .iter()
        .partition(|id| w.combatant(**id).is_some_and(in_row));
    let (line_slots, row_slots) = f.shape.slots(line.len(), row.len());
    f.slots = line
        .into_iter()
        .zip(line_slots)
        .map(|(id, p)| (id, p, true))
        .chain(row.into_iter().zip(row_slots).map(|(id, p)| (id, p, false)))
        .collect();
}

/// Once a step, before the brains think: drop the dead, face the nearest
/// enemy any member is after, walk the anchor up while everyone keeps
/// place, and lay the places out again.
pub(super) fn update(w: &mut CombatWorld, view: &[Seen], dt: f32) {
    let mut formations = std::mem::take(&mut w.tactics.formations);
    for f in &mut formations {
        f.members.retain(|id| view.iter().any(|s| s.id == *id));
        let anchor = f.shape.anchor;
        let enemy = f
            .members
            .iter()
            .filter_map(|id| w.combatant(*id)?.brain.as_ref()?.mind.target)
            .filter_map(|t| view.iter().find(|s| s.id == t))
            .map(|s| s.position)
            .min_by(|a, b| {
                a.distance_squared(anchor)
                    .total_cmp(&b.distance_squared(anchor))
            });
        if let Some(e) = enemy {
            let lagging = f
                .slots
                .iter()
                .filter_map(|(id, p, _)| {
                    let s = view.iter().find(|s| s.id == *id)?;
                    Some(flat(s.position - *p).length())
                })
                .fold(0.0, f32::max);
            let step = if lagging < CATCH_UP {
                ADVANCE * dt
            } else {
                0.0
            };
            f.shape.advance(e, HOLD_AT, step);
        }
        place(w, f);
    }
    formations.retain(|f| f.members.len() >= 2);
    w.tactics.formations = formations;
}

impl<Q: CombatQuery> Host<'_, Q> {
    /// This body's place in a formation, if it is in one.
    pub(super) fn formation_row(&self) -> Option<Place> {
        self.tactics
            .formations
            .iter()
            .flat_map(|f| &f.slots)
            .find(|(id, _, _)| *id == self.me.id)
            .map(|&(_, slot, front)| Place { slot, front })
    }

    /// Keep place in a formation. Succeeds at once outside one, for a
    /// member of the line with an enemy close (it breaks to fight), and for
    /// a member of the row near its spot (it shoots or casts from there);
    /// otherwise walks to the place, faces the enemy and runs.
    pub(super) fn hold_formation(&mut self) -> Status {
        let Some(place) = self.formation_row() else {
            return Status::Success;
        };
        let target = self.target();
        if place.front {
            if target.is_some_and(|t| self.distance(&t) < BREAK) {
                return Status::Success;
            }
        } else if flat(place.slot - self.me.position).length() < ROW_SLACK {
            return Status::Success;
        }
        self.mind.doing = "In formation";
        if let Some(t) = target {
            self.face(t.chest);
        }
        if self.go(place.slot, false) {
            self.cmd.move_dir = Vec3::ZERO;
        }
        Status::Running
    }
}
