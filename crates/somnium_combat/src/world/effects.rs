//! Spell bodies that stay in the world: walls, waves, blooms and wards.
//!
//! | Wall | Stops bodies | Stops missiles | Also |
//! |---|---|---|---|
//! | Stone | yes | yes | smothers fire at its foot |
//! | Ice (Frost) | yes | yes; flame melts it three times as fast | freezes water at its foot |
//! | Force (Warding) | yes | only an enemy's | |
//! | Gale | no | throws them up and aside | a wind across it: fire runs with it |
//! | Flame | no | lights arrows that pass | burns who crosses; fires the grass at its foot |
//! | Water | no | puts out lit arrows, swallows flame bolts | soaks who crosses |
use super::{CombatEvent, CombatWorld, Combatant};
use crate::damage::{Hit, HitFlags};
use crate::environment::WindVolume;
use crate::geometry::{Arc, Obb, planar_dir};
use crate::spell::Element;
use crate::{CombatantId, DamagePart, DamageType, StatusKind};
use glam::{Quat, Vec3};

/// A standing spell wall.
#[derive(Clone, Debug, PartialEq)]
pub struct Wall {
    /// Id.
    pub id: u32,
    /// Caster.
    pub owner: Option<CombatantId>,
    /// Element.
    pub element: Element,
    /// Box.
    pub obb: Obb,
    /// Hit points (missiles and blows wear it down).
    pub health: f32,
    /// Seconds left.
    pub time_left: f32,
    /// Harm per second to those inside (flame), or none.
    pub power: f32,
    /// Asset that raised it.
    pub spell: String,
    tick: f32,
}

impl Wall {
    /// Whether bodies are stopped by it.
    #[must_use]
    pub fn blocks_movement(&self) -> bool {
        matches!(
            self.element,
            Element::Stone | Element::Frost | Element::Force
        )
    }
}

/// A front sweeping outward.
#[derive(Clone, Debug, PartialEq)]
pub struct Wave {
    /// Caster.
    pub owner: Option<CombatantId>,
    /// Element.
    pub element: Element,
    /// Where it started.
    pub origin: Vec3,
    /// Planar direction of its middle.
    pub dir: Vec3,
    /// Half the arc, radians.
    pub half_angle: f32,
    /// Current radius.
    pub radius: f32,
    /// Where it stops.
    pub max_radius: f32,
    /// Metres per second.
    pub speed: f32,
    /// Harm (or healing) per body.
    pub power: f32,
    /// Status build-up per body.
    pub status: Vec<(StatusKind, f32)>,
    /// Poise damage.
    pub poise: f32,
    /// Push outward.
    pub knockback: f32,
    /// Bodies already struck.
    pub hit: Vec<CombatantId>,
}

/// A burst gathering at a point.
#[derive(Clone, Debug, PartialEq)]
pub struct Bloom {
    /// Caster.
    pub owner: Option<CombatantId>,
    /// Element.
    pub element: Element,
    /// Centre.
    pub center: Vec3,
    /// Radius.
    pub radius: f32,
    /// Seconds until it bursts.
    pub delay: f32,
    /// Harm (or healing) per body.
    pub power: f32,
    /// Status build-up.
    pub status: Vec<(StatusKind, f32)>,
    /// Poise damage.
    pub poise: f32,
    /// Push outward.
    pub knockback: f32,
}

/// A shield on a body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ward {
    /// Element: Force absorbs, Stone hardens, Flame burns and Frost chills who
    /// strike it, Gale turns missiles.
    pub element: Element,
    /// Harm it can still absorb.
    pub strength: f32,
    /// Seconds left.
    pub time_left: f32,
    /// Extra physical armour while it stands (Stone Skin).
    pub armor: f32,
}

fn element_damage(element: Element, amount: f32) -> Vec<DamagePart> {
    match element {
        Element::Stone => vec![
            DamagePart::new(DamageType::Stone, amount * 0.7),
            DamagePart::new(DamageType::Blunt, amount * 0.3),
        ],
        e => e
            .damage_type()
            .map(|k| vec![DamagePart::new(k, amount)])
            .unwrap_or_default(),
    }
}

pub(crate) fn spell_hit(
    owner: Option<CombatantId>,
    element: Element,
    power: f32,
    poise: f32,
    status: &[(StatusKind, f32)],
) -> Hit {
    let mut hit = Hit::new(owner, element_damage(element, power), poise);
    hit.status = element
        .natural_status()
        .into_iter()
        .chain(status.iter().copied())
        .collect();
    hit.flags = HitFlags::SPELL;
    hit
}

/// Raise a wall and tell the host.
#[allow(clippy::too_many_arguments)]
pub(crate) fn raise_wall(
    w: &mut CombatWorld,
    owner: CombatantId,
    element: Element,
    base: Vec3,
    facing: Vec3,
    size: f32,
    duration: f32,
    power: f32,
    spell: &str,
) -> u32 {
    let id = w.next_wall_id();
    let width = (size * 2.0).max(1.0);
    let height = 2.5 + size * 0.25;
    let obb = Obb::wall(base, facing, width, height, 0.7);
    let along = Quat::from_rotation_arc(Vec3::Z, planar_dir(facing, Vec3::Z)) * Vec3::X;
    let (a, b) = (base - along * width * 0.5, base + along * width * 0.5);
    if let Some(field) = &mut w.environment {
        match element {
            Element::Flame => field.react_line(Element::Flame, a, b, 0.6),
            Element::Frost => field.react_line(Element::Frost, a, b, 0.8),
            Element::Stone | Element::Water => field.react_line(element, a, b, 0.8),
            Element::Gale => field.winds.push(WindVolume {
                center: base,
                radius: width.max(6.0),
                velocity: planar_dir(facing, Vec3::Z) * 9.0,
                time_left: duration,
            }),
            _ => {}
        }
    }
    let wall = Wall {
        id,
        owner: Some(owner),
        element,
        obb,
        health: 80.0 * size.max(0.5),
        time_left: duration,
        power,
        spell: spell.to_string(),
        tick: 0.0,
    };
    let blocks = wall.blocks_movement();
    w.walls.push(wall);
    w.emit(CombatEvent::WallRaised {
        wall: id,
        element,
        obb,
        blocks_movement: blocks,
    });
    id
}

/// Heal or harm everyone a predicate picks, with the spell's rules for allies.
#[allow(clippy::too_many_arguments)]
fn strike(
    w: &mut CombatWorld,
    owner: Option<CombatantId>,
    element: Element,
    targets: Vec<(CombatantId, Vec3, Vec3)>,
    power: f32,
    poise: f32,
    status: &[(StatusKind, f32)],
    knockback: f32,
) {
    for (id, point, push) in targets {
        if element == Element::Mending {
            if owner.is_none_or(|o| w.allied(o, id)) {
                w.queue_heal(id, owner, power);
            }
            continue;
        }
        if !w.may_hit(owner, id, true) {
            continue;
        }
        let mut hit = spell_hit(owner, element, power, poise, status);
        hit.flags |= HitFlags::AREA;
        hit.point = point;
        hit.direction = push;
        hit.knockback = push * knockback;
        if poise >= 30.0 {
            hit.flags |= HitFlags::HEAVY;
        }
        w.queue_hit(id, hit, None);
    }
}

/// What area effects need of a living body.
struct BodyRef {
    id: CombatantId,
    capsule: crate::geometry::Capsule,
    chest: Vec3,
    position: Vec3,
    facing: Vec3,
}

fn living(w: &CombatWorld) -> Vec<BodyRef> {
    w.combatants()
        .filter(|c| c.alive())
        .map(|c: &Combatant| BodyRef {
            id: c.id,
            capsule: c.capsule(),
            chest: c.chest(),
            position: c.position,
            facing: c.facing,
        })
        .collect()
}

pub(super) fn step(w: &mut CombatWorld, dt: f32) {
    if !w.walls.is_empty() {
        step_walls(w, dt);
    }
    if !w.waves.is_empty() {
        step_waves(w, dt);
    }
    if !w.blooms.is_empty() {
        step_blooms(w, dt);
    }
}

fn step_walls(w: &mut CombatWorld, dt: f32) {
    let bodies = living(w);
    let mut fallen = Vec::new();
    let mut burns = Vec::new();
    for wall in &mut w.walls {
        wall.time_left -= dt;
        if wall.time_left <= 0.0 || wall.health <= 0.0 {
            fallen.push(wall.id);
            continue;
        }
        if !matches!(wall.element, Element::Flame | Element::Water) {
            continue;
        }
        wall.tick += dt;
        if wall.tick < 0.5 {
            continue;
        }
        wall.tick -= 0.5;
        for c in &bodies {
            if wall.obb.touches_capsule(&c.capsule) {
                burns.push((wall.owner, wall.element, wall.power * 0.5, c.id, c.chest));
            }
        }
    }
    for (owner, element, power, id, point) in burns {
        let push = Vec3::ZERO;
        strike(
            w,
            owner,
            element,
            vec![(id, point, push)],
            power,
            0.0,
            &[],
            0.0,
        );
    }
    for id in fallen {
        w.walls.retain(|x| x.id != id);
        w.emit(CombatEvent::WallFell { wall: id });
    }
}

fn step_waves(w: &mut CombatWorld, dt: f32) {
    let bodies = living(w);
    let mut waves = std::mem::take(&mut w.waves);
    for wave in &mut waves {
        let inner = wave.radius;
        wave.radius = (wave.radius + wave.speed * dt).min(wave.max_radius);
        let arc = Arc {
            origin: wave.origin + Vec3::Y * 0.9,
            forward: wave.dir,
            radius: wave.radius,
            half_angle: wave.half_angle,
            low: -1.2,
            high: 1.6,
            inner: (inner - 0.6).max(0.0),
        };
        let mut targets = Vec::new();
        for c in &bodies {
            if Some(c.id) == wave.owner || wave.hit.contains(&c.id) {
                continue;
            }
            if let Some(point) = arc.hits(&c.capsule) {
                let push = planar_dir(c.position - wave.origin, wave.dir);
                targets.push((c.id, point, push));
                wave.hit.push(c.id);
            }
        }
        strike(
            w,
            wave.owner,
            wave.element,
            targets,
            wave.power,
            wave.poise,
            &wave.status,
            wave.knockback,
        );
        // The ground along the front.
        if let Some(field) = &mut w.environment {
            let steps = ((wave.radius * wave.half_angle * 2.0) / field.cell_size())
                .ceil()
                .max(1.0) as i32;
            for k in 0..=steps {
                let a = -wave.half_angle + 2.0 * wave.half_angle * k as f32 / steps as f32;
                let p = wave.origin + Quat::from_rotation_y(-a) * wave.dir * wave.radius;
                field.react(wave.element, p, field.cell_size(), wave.dir, &mut w.rng);
            }
        }
        // A gale front shoves missiles in its path.
        if wave.element == Element::Gale {
            for p in &mut w.projectiles {
                let arc = Arc { inner: 0.0, ..arc };
                let probe = crate::geometry::Capsule {
                    a: p.position,
                    b: p.position,
                    radius: p.def.radius,
                };
                if arc.hits(&probe).is_some() {
                    p.velocity += wave.dir * 12.0;
                }
            }
        }
    }
    waves.retain(|wave| wave.radius < wave.max_radius);
    waves.append(&mut w.waves);
    w.waves = waves;
}

fn step_blooms(w: &mut CombatWorld, dt: f32) {
    let bodies = living(w);
    let mut blooms = std::mem::take(&mut w.blooms);
    let mut burst = Vec::new();
    blooms.retain_mut(|b| {
        b.delay -= dt;
        if b.delay > 0.0 {
            return true;
        }
        burst.push(b.clone());
        false
    });
    for b in burst {
        let targets: Vec<_> = bodies
            .iter()
            .filter(|c| c.capsule.distance(b.center) <= b.radius)
            .map(|c| {
                (
                    c.id,
                    c.chest,
                    planar_dir(c.position - b.center, c.facing * -1.0),
                )
            })
            .collect();
        strike(
            w,
            b.owner,
            b.element,
            targets,
            b.power,
            b.poise,
            &b.status,
            b.knockback,
        );
        let facing = b.owner.and_then(|o| w.get(o)).map_or(Vec3::Z, |o| o.facing);
        if let Some(field) = &mut w.environment {
            field.react(b.element, b.center, b.radius, facing, &mut w.rng);
        }
        w.emit(CombatEvent::Bloomed {
            at: b.center,
            radius: b.radius,
            element: b.element,
        });
    }
    blooms.append(&mut w.blooms);
    w.blooms = blooms;
}
