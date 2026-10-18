//! Arrows, rounds, thrown knives, wind cuts and spell bolts.
//!
//! Each step a projectile sweeps a segment and takes the nearest of three
//! contacts: the host's world (through [`CombatQuery`]), a spell wall, or a
//! body. Walls answer by element (stone and ice stop it, a gale throws it
//! aside, fire lights an arrow); a body in its parry window with the Meridian's
//! Mirror sends it back.
use super::{CombatEvent, CombatWorld};
use crate::damage::{Hit, HitFlags};
use crate::geometry::{flat, planar_dir, ray_capsule};
use crate::schools::Form;
use crate::spell::Element;
use crate::weapon::{ProjectileDef, ProjectileKind};
use crate::{CombatQuery, CombatantId, StatusKind};
use glam::{Quat, Vec3};

const GRAVITY: f32 = 9.81;

/// A spell bolt's extra meaning.
#[derive(Clone, Debug, PartialEq)]
pub struct SpellPayload {
    /// Asset id.
    pub spell: String,
    /// Element.
    pub element: Element,
    /// Measure size, for the ground reaction where it lands.
    pub size: f32,
    /// Heals whom it strikes (Mending) instead of harming.
    pub heals: bool,
}

/// A projectile in flight.
#[derive(Clone, Debug, PartialEq)]
pub struct Projectile {
    /// Id.
    pub id: u32,
    /// Who loosed it (changes when reflected).
    pub owner: Option<CombatantId>,
    /// Ballistics and harm.
    pub def: ProjectileDef,
    /// Position.
    pub position: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Metres flown.
    pub travelled: f32,
    /// Bodies passed through.
    pub pierced: u8,
    /// Bodies already struck.
    pub hit: Vec<CombatantId>,
    /// Damage multiplier (draw strength, rank).
    pub scale: f32,
    /// Flags every hit carries.
    pub flags: HitFlags,
    /// Spell meaning, if a spell.
    pub spell: Option<SpellPayload>,
    /// Lit by a flame wall: burns what it strikes.
    pub burning: bool,
    /// Turns toward this body (a bolt cast at a target).
    pub homing: Option<CombatantId>,
}

/// Launch velocity to strike `target` from `from` at `speed` under `gravity`
/// (metres per second squared): the low arc, or a 45-degree lob when out of reach.
#[must_use]
pub fn ballistic(from: Vec3, target: Vec3, speed: f32, gravity: f32) -> Vec3 {
    let delta = target - from;
    let horizontal = flat(delta);
    let d = horizontal.length();
    let dir = horizontal.try_normalize().unwrap_or(Vec3::Z);
    if gravity <= 1e-4 || d < 1e-3 {
        return delta.try_normalize().unwrap_or(dir) * speed;
    }
    let (v2, h) = (speed * speed, delta.y);
    let disc = v2 * v2 - gravity * (gravity * d * d + 2.0 * h * v2);
    let angle = if disc >= 0.0 {
        ((v2 - disc.sqrt()) / (gravity * d)).atan()
    } else {
        std::f32::consts::FRAC_PI_4
    };
    (dir * angle.cos() + Vec3::Y * angle.sin()) * speed
}

/// Loose a projectile from a combatant's chest toward `aim` (or straight ahead).
pub(crate) fn launch(
    w: &mut CombatWorld,
    owner: CombatantId,
    def: ProjectileDef,
    aim: Option<Vec3>,
    scale: f32,
    spread_degrees: Option<f32>,
    flags: HitFlags,
) -> Option<u32> {
    let c = w.get(owner)?;
    let from = c.chest() + c.facing * 0.5;
    let target = aim.unwrap_or(from + c.facing * def.range.min(30.0));
    let mut velocity = ballistic(from, target, def.speed, GRAVITY * def.gravity);
    if let Some(spread) = spread_degrees
        && spread > 0.0
    {
        let yaw = w.rng.signed() * spread.to_radians();
        let pitch = w.rng.signed() * spread.to_radians() * 0.5;
        let side = velocity.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
        velocity = Quat::from_axis_angle(side, pitch) * (Quat::from_rotation_y(yaw) * velocity);
    }
    Some(spawn(
        w, owner, def, from, velocity, scale, flags, None, None,
    ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn(
    w: &mut CombatWorld,
    owner: CombatantId,
    def: ProjectileDef,
    from: Vec3,
    velocity: Vec3,
    scale: f32,
    flags: HitFlags,
    spell: Option<SpellPayload>,
    homing: Option<CombatantId>,
) -> u32 {
    let id = w.next_projectile_id();
    let kind = def.kind;
    w.projectiles.push(Projectile {
        id,
        owner: Some(owner),
        def,
        position: from,
        velocity,
        travelled: 0.0,
        pierced: 0,
        hit: Vec::new(),
        scale,
        flags,
        spell,
        burning: false,
        homing,
    });
    w.emit(CombatEvent::ShotFired {
        shooter: owner,
        projectile: id,
        kind,
        from,
        velocity,
    });
    id
}

enum Contact {
    World(f32, Vec3),
    Wall(f32, usize, Vec3),
    Body(f32, CombatantId),
}

impl Contact {
    fn t(&self) -> f32 {
        match self {
            Self::World(t, _) | Self::Wall(t, _, _) | Self::Body(t, _) => *t,
        }
    }
}

pub(super) fn step(w: &mut CombatWorld, dt: f32, query: &impl CombatQuery) {
    let flying = std::mem::take(&mut w.projectiles);
    let mut kept = Vec::with_capacity(flying.len());
    for mut p in flying {
        if fly(w, &mut p, dt, query) {
            kept.push(p);
        }
    }
    // Launches during the step (wind cuts from a reflected strike) join the rest.
    kept.append(&mut w.projectiles);
    w.projectiles = kept;
}

fn end(
    w: &mut CombatWorld,
    p: &Projectile,
    point: Vec3,
    normal: Vec3,
    target: Option<CombatantId>,
) {
    if let Some(spell) = &p.spell
        && let Some(field) = &mut w.environment
        && target.is_none()
    {
        let ground = Vec3::new(point.x, 0.0, point.z);
        field.react(
            spell.element,
            ground,
            (spell.size * 0.75).max(0.5),
            planar_dir(p.velocity, Vec3::Z),
            &mut w.rng,
        );
    }
    if p.burning
        && target.is_none()
        && let Some(field) = &mut w.environment
    {
        field.react(Element::Flame, point, 0.5, Vec3::Z, &mut w.rng);
    }
    w.emit(CombatEvent::ProjectileEnded {
        projectile: p.id,
        point,
        normal,
        target,
        kind: p.def.kind,
        element: p.spell.as_ref().map(|s| s.element),
    });
}

/// Advance one projectile; false when it is spent.
fn fly(w: &mut CombatWorld, p: &mut Projectile, dt: f32, query: &impl CombatQuery) -> bool {
    p.velocity.y -= GRAVITY * p.def.gravity * dt;
    if p.def.wind_factor > 0.0
        && let Some(field) = &w.environment
    {
        p.velocity += field.wind_at(p.position) * p.def.wind_factor * dt * 2.0;
    }
    if let Some(target) = p.homing
        && let Some(t) = w.get(target)
        && t.alive()
    {
        let speed = p.velocity.length();
        let want = (t.chest() - p.position)
            .try_normalize()
            .unwrap_or(p.velocity / speed.max(1e-3));
        let now = p.velocity / speed.max(1e-3);
        p.velocity = now.lerp(want, (2.5 * dt).min(1.0)).normalize_or_zero() * speed;
    }
    let from = p.position;
    let delta = p.velocity * dt;
    let len = delta.length();
    let Some(dir) = delta.try_normalize() else {
        return false;
    };
    let mut best: Option<Contact> = query
        .sweep(from, from + delta, p.def.radius)
        .map(|h| Contact::World(h.fraction * len, h.normal));
    for (i, wall) in w.walls.iter().enumerate() {
        if let Some((t, n)) = wall.obb.ray(from, dir, len, p.def.radius)
            && best.as_ref().is_none_or(|b| t < b.t())
        {
            best = Some(Contact::Wall(t, i, n));
        }
    }
    let heals = p.spell.as_ref().is_some_and(|s| s.heals);
    for id in w.ids() {
        if p.hit.contains(&id) || p.owner == Some(id) {
            continue;
        }
        let Some(c) = w.get(id) else {
            continue;
        };
        if !c.alive() {
            continue;
        }
        let eligible = if heals {
            p.owner.is_some_and(|o| w.allied(o, id))
        } else {
            w.may_hit(p.owner, id, false)
        };
        if !eligible {
            continue;
        }
        if let Some(t) = ray_capsule(from, dir, len, &c.capsule(), p.def.radius)
            && best.as_ref().is_none_or(|b| t < b.t())
        {
            best = Some(Contact::Body(t, id));
        }
    }
    match best {
        None => {
            p.position += delta;
            p.travelled += len;
            if p.travelled >= p.def.range {
                end(w, p, p.position, Vec3::ZERO, None);
                return false;
            }
            true
        }
        Some(Contact::World(t, normal)) => {
            end(w, p, from + dir * t, normal, None);
            false
        }
        Some(Contact::Wall(t, i, normal)) => wall_contact(w, p, i, from + dir * t, normal),
        Some(Contact::Body(t, id)) => body_contact(w, p, id, from + dir * t),
    }
}

fn wall_contact(
    w: &mut CombatWorld,
    p: &mut Projectile,
    index: usize,
    point: Vec3,
    normal: Vec3,
) -> bool {
    let wall = &w.walls[index];
    let hostile_shot = match (wall.owner, p.owner) {
        (Some(a), Some(b)) => !w.allied(a, b),
        _ => true,
    };
    let element = wall.element;
    let raw = p.def.damage.iter().map(|d| d.amount).sum::<f32>() * p.scale;
    let flame = p
        .spell
        .as_ref()
        .is_some_and(|s| s.element == Element::Flame)
        || p.burning;
    match element {
        Element::Stone | Element::Frost => {
            let melt = if element == Element::Frost && flame {
                3.0
            } else {
                1.0
            };
            w.walls[index].health -= raw * melt;
            end(w, p, point, normal, None);
            false
        }
        Element::Force if hostile_shot => {
            w.walls[index].health -= raw;
            end(w, p, point, normal, None);
            false
        }
        Element::Gale => {
            // Thrown up and aside: reflect across the wall and lose half the speed.
            let v = p.velocity;
            p.velocity = (v - 2.0 * v.dot(normal) * normal) * 0.5 + Vec3::Y * 4.0;
            p.position = point + normal * 0.05;
            true
        }
        Element::Flame => {
            if matches!(p.def.kind, ProjectileKind::Arrow | ProjectileKind::Knife) {
                p.burning = true;
            }
            p.position =
                point + p.velocity.normalize_or_zero() * (w.walls[index].obb.half.z * 2.0 + 0.1);
            true
        }
        Element::Water => {
            p.burning = false;
            if p.spell
                .as_ref()
                .is_some_and(|s| s.element == Element::Flame)
            {
                end(w, p, point, normal, None);
                return false;
            }
            p.position =
                point + p.velocity.normalize_or_zero() * (w.walls[index].obb.half.z * 2.0 + 0.1);
            true
        }
        _ => {
            p.position =
                point + p.velocity.normalize_or_zero() * (w.walls[index].obb.half.z * 2.0 + 0.1);
            true
        }
    }
}

fn body_contact(w: &mut CombatWorld, p: &mut Projectile, target: CombatantId, point: Vec3) -> bool {
    let Some(t) = w.get(target) else {
        return false;
    };
    let from_dir = planar_dir(-p.velocity, t.facing);
    let front = t.facing.dot(from_dir) >= (t.weapon.guard.angle * 0.5).to_radians().cos();
    // Meridian's Mirror: a bolt met in the parry window flies back.
    let parry_window = t.weapon.guard.parry_window * t.sword.map_or(1.0, |s| s.parry_scale());
    let mirrors =
        t.knows(Form::TheMirror) && t.guard_time().is_some_and(|g| g <= parry_window) && front;
    if mirrors {
        let back_to = p.owner.and_then(|o| w.get(o)).map(|o| o.chest());
        let speed = p.velocity.length();
        p.velocity = back_to
            .and_then(|b| (b - point).try_normalize())
            .unwrap_or(-p.velocity / speed.max(1e-3))
            * speed;
        p.owner = Some(target);
        p.hit.clear();
        p.homing = None;
        p.position = point + p.velocity.normalize_or_zero() * 0.3;
        w.emit(CombatEvent::Reflected {
            defender: target,
            projectile: p.id,
        });
        return true;
    }
    let head = point.y >= t.position.y + t.body.height * 0.85;
    // A gale ward turns most missiles aside.
    let gale_ward = t.ward.is_some_and(|ward| ward.element == Element::Gale);
    if gale_ward && w.rng.chance(0.7) {
        let side = p.velocity.cross(Vec3::Y).normalize_or_zero();
        p.velocity = (p.velocity * 0.6 + side * p.velocity.length() * 0.6) + Vec3::Y * 3.0;
        p.position = point + p.velocity.normalize_or_zero() * 0.4;
        p.hit.push(target);
        return true;
    }
    let heals = p.spell.as_ref().is_some_and(|s| s.heals);
    if heals {
        let amount = p.def.damage.iter().map(|d| d.amount).sum::<f32>() * p.scale;
        w.queue_heal(target, p.owner, amount);
    } else {
        let mut hit = Hit::new(p.owner, p.def.damage.clone(), p.def.poise);
        hit.status = p.def.status.clone();
        if p.burning {
            hit.status.push((StatusKind::Burn, 5.0));
        }
        hit.knockback = p.velocity.normalize_or_zero() * p.def.knockback;
        hit.point = point;
        hit.direction = p.velocity.normalize_or_zero();
        hit.flags = p.flags | HitFlags::PROJECTILE;
        if p.spell.is_some() {
            hit.flags |= HitFlags::SPELL;
        }
        hit.scale = p.scale;
        if head && p.def.head_multiplier > 1.0 {
            hit.scale *= p.def.head_multiplier;
            hit.flags |= HitFlags::CRITICAL;
        }
        w.queue_hit(target, hit, None);
    }
    p.hit.push(target);
    p.pierced += 1;
    if p.pierced > p.def.pierce {
        end(w, p, point, Vec3::ZERO, Some(target));
        return false;
    }
    p.position = point + p.velocity.normalize_or_zero() * 0.05;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ballistic_lands_on_the_mark() {
        let from = Vec3::new(0.0, 1.5, 0.0);
        let target = Vec3::new(0.0, 1.2, 40.0);
        let g = GRAVITY * 0.5;
        let v = ballistic(from, target, 48.0, g);
        assert!((v.length() - 48.0).abs() < 1e-3);
        // Integrate to the target's distance and check the height there.
        let t = 40.0 / v.z;
        let y = from.y + v.y * t - 0.5 * g * t * t;
        assert!((y - 1.2).abs() < 0.05, "arrives at {y}");
        // Out of reach: a lob.
        let lob = ballistic(from, Vec3::new(0.0, 0.0, 900.0), 48.0, g);
        assert!((lob.y / lob.z - 1.0).abs() < 1e-3);
    }
}
