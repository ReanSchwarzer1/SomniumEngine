//! Resolving queued hits, in the order they were queued:
//! dodge frames, ward, parry, block, harm (resistances, statuses, shatter),
//! poise, interruption, provocation, death.
use super::{Action, CombatEvent, CombatWorld, DeathRagdoll, Phase, cast::CastPhase};
use crate::damage::{Hit, HitFlags};
use crate::geometry::planar_dir;
use crate::schools::Form;
use crate::spell::{CastSpeed, Element};
use crate::{CombatantId, DamageType, StatusKind};
use glam::Vec3;

/// Seconds a parried attacker reels.
pub const PARRIED_STAGGER: f32 = 0.9;
/// Seconds a broken guard reels.
pub const GUARD_BREAK_STAGGER: f32 = 1.2;
/// Seconds a poise break staggers.
pub const POISE_STAGGER: f32 = 0.7;
/// Seconds a heavy poise break knocks down.
pub const KNOCKDOWN: f32 = 1.6;
/// Share of maximum health one blow must take to break a short-chant.
pub const CHANT_BREAK: f32 = 0.08;
/// A parry this soon after raising the guard is "at the instant" (Noon).
pub const NOON_WINDOW: f32 = 0.06;

pub(super) fn resolve_pending(w: &mut CombatWorld) {
    for (target, source, amount) in std::mem::take(&mut w.pending_heals) {
        if let Some(c) = w.get_mut(target) {
            let healed = c.vitals.heal(amount);
            if healed > 0.0 {
                w.emit(CombatEvent::Healed {
                    target,
                    source,
                    amount: healed,
                });
            }
        }
    }
    // Counters (Noon) queue further hits; a few rounds settle them.
    for _ in 0..4 {
        let pending = std::mem::take(&mut w.pending);
        if pending.is_empty() {
            break;
        }
        for (target, hit, part) in pending {
            resolve_hit(w, target, hit, part);
        }
    }
}

fn resolve_hit(w: &mut CombatWorld, target: CombatantId, mut hit: Hit, part: Option<String>) {
    let now = w.time();
    let Some(t) = w.get(target) else {
        return;
    };
    if !t.alive() {
        return;
    }
    let passive = hit.flags.intersects(HitFlags::ENVIRONMENT | HitFlags::TICK);

    // Dodge frames: everything but the ground itself passes through.
    if t.invulnerable() && !passive {
        w.emit(CombatEvent::Dodged {
            target,
            source: hit.source,
        });
        if let Some(t) = w.get_mut(target) {
            t.vitals.focus.current = (t.vitals.focus.current + 8.0).min(t.vitals.focus.max);
        }
        return;
    }

    let attacker = hit
        .source
        .and_then(|s| w.get(s))
        .map(|a| (a.id, a.position));
    let toward_attacker = attacker.map_or(planar_dir(-hit.direction, t.facing), |(_, p)| {
        planar_dir(p - t.position, t.facing)
    });
    let guard = t.weapon.guard;
    let front = t.facing.dot(toward_attacker) >= (guard.angle * 0.5).to_radians().cos();
    let parry_window = guard.parry_window * t.sword.map_or(1.0, |s| s.parry_scale());
    let guard_time = t.guard_time();
    let melee = hit.flags.contains(HitFlags::MELEE);
    let rimeguard = t.knows(Form::Rimeguard);
    let noon = t.knows(Form::Noon);

    // Parry.
    if melee
        && !hit.flags.contains(HitFlags::UNPARRYABLE)
        && guard.can_parry
        && front
        && guard_time.is_some_and(|g| g <= parry_window)
        && let Some((attacker_id, _)) = attacker
    {
        let perfect = noon && guard_time.is_some_and(|g| g <= NOON_WINDOW);
        let riposte_window = w.rules.riposte_window;
        if let Some(t) = w.get_mut(target) {
            t.riposte_until = now + f64::from(riposte_window);
        }
        w.emit(CombatEvent::Parried {
            defender: target,
            attacker: attacker_id,
            point: hit.point,
            perfect,
        });
        w.apply_status(attacker_id, StatusKind::Stagger, PARRIED_STAGGER);
        if rimeguard {
            w.apply_status(attacker_id, StatusKind::Chill, 25.0);
        }
        if perfect {
            counter(w, target, attacker_id);
        }
        return;
    }

    // Ward.
    if !passive && let Some(mut ward) = w.get(target).and_then(|t| t.ward) {
        let raw = hit.raw_total();
        let absorbed = raw.min(ward.strength);
        ward.strength -= absorbed;
        if raw > 0.0 {
            hit.scale *= (raw - absorbed) / raw;
        }
        if melee && let Some((attacker_id, _)) = attacker {
            match ward.element {
                Element::Flame => w.apply_status(attacker_id, StatusKind::Burn, 5.0),
                Element::Frost => w.apply_status(attacker_id, StatusKind::Chill, 20.0),
                _ => {}
            }
        }
        let broke = ward.strength <= 0.0;
        if let Some(t) = w.get_mut(target) {
            t.ward = (!broke).then_some(ward);
        }
        if broke {
            w.emit(CombatEvent::WardBroken { target });
        }
        if hit.raw_total() <= 1e-4 {
            w.emit(CombatEvent::Blocked {
                target,
                source: hit.source,
                point: hit.point,
            });
            return;
        }
    }

    // Block.
    if !passive
        && !hit.flags.intersects(HitFlags::UNBLOCKABLE | HitFlags::AREA)
        && guard_time.is_some()
        && front
    {
        let stability = if hit.flags.contains(HitFlags::SPELL) {
            guard.stability * 0.5
        } else {
            guard.stability
        };
        let cost = hit.raw_total() * guard.stamina_per_damage;
        let held = w
            .get_mut(target)
            .is_some_and(|t| t.vitals.drain_stamina(cost));
        hit.scale *= 1.0 - stability;
        hit.poise *= 0.3;
        for s in &mut hit.status {
            s.1 *= 0.3;
        }
        hit.knockback *= 0.5;
        if held {
            w.emit(CombatEvent::Blocked {
                target,
                source: hit.source,
                point: hit.point,
            });
        } else {
            w.emit(CombatEvent::GuardBroken { target });
            w.apply_status(target, StatusKind::Stagger, GUARD_BREAK_STAGGER);
        }
        if rimeguard
            && melee
            && let Some((attacker_id, _)) = attacker
        {
            w.apply_status(attacker_id, StatusKind::Chill, 25.0);
        }
    }

    apply_harm(w, target, &hit, part.as_deref());
}

/// Meridian's Noon: the parry itself strikes back with a riposte.
fn counter(w: &mut CombatWorld, defender: CombatantId, attacker: CombatantId) {
    let Some(d) = w.get(defender) else {
        return;
    };
    let Some(first) = d.weapon.light.first() else {
        return;
    };
    let mut hit = Hit::new(Some(defender), first.damage.clone(), first.poise * 2.0);
    hit.flags = HitFlags::MELEE
        | HitFlags::RIPOSTE
        | HitFlags::CRITICAL
        | HitFlags::UNBLOCKABLE
        | HitFlags::UNPARRYABLE;
    hit.scale = w.rules.riposte
        * d.sword
            .map_or(1.0, |s| s.riposte_scale() * s.damage_scale())
        * d.kindle_strength();
    hit.direction = d.facing;
    hit.point = d.chest() + d.facing * 0.8;
    hit.knockback = d.facing * 2.0;
    if let Some(d) = w.get_mut(defender) {
        d.riposte_until = f64::NEG_INFINITY;
    }
    w.queue_hit(attacker, hit, None);
}

/// Mitigate and apply harm, statuses, poise, interruption, knockback,
/// provocation and death. Returns health taken.
pub(super) fn apply_harm(
    w: &mut CombatWorld,
    id: CombatantId,
    hit: &Hit,
    part: Option<&str>,
) -> f32 {
    let now = w.time();
    let Some(c) = w.get(id) else {
        return 0.0;
    };
    if !c.alive() {
        return 0.0;
    }
    let part_scale = part
        .and_then(|name| c.parts.iter().find(|(p, _)| p.name == name))
        .map_or(1.0, |(p, _)| p.damage_scale);
    let kindle = c.kindle_strength();
    let mut resist = c.resist;
    if let Some(ward) = c.ward {
        resist.armor += ward.armor;
    }
    let frozen = c.status.active(StatusKind::Freeze);
    let mut total = 0.0;
    let mut dominant = (DamageType::Blunt, 0.0f32);
    let mut shatter = false;
    for p in &hit.parts {
        let mut amount = p.amount * hit.scale * part_scale * c.status.incoming_scale(p.kind);
        if p.kind.is_physical() && kindle > 1.0 {
            amount /= 1.0 + (kindle - 1.0) * 0.5;
        }
        if frozen
            && matches!(p.kind, DamageType::Blunt | DamageType::Stone)
            && p.amount * hit.scale >= 10.0
        {
            shatter = true;
        }
        let through = resist.mitigate(p.kind, amount);
        if through > dominant.1 {
            dominant = (p.kind, through);
        }
        total += through;
    }
    if shatter {
        total *= 1.5;
    }
    let hyper = match &c.action {
        Action::Attack(a) if matches!(a.phase, Phase::Windup | Phase::Active) => a.def.hyper_armor,
        _ => 0.0,
    };
    let mass = c.body.mass.max(1.0);
    let max_health = c.vitals.health.max;
    let chanting = matches!(&c.action, Action::Cast(s)
        if s.phase == CastPhase::Gather && s.request.release == CastSpeed::ShortChant);
    let source = hit.source;

    let Some(c) = w.get_mut(id) else {
        return 0.0;
    };
    let taken = c.vitals.damage(total);
    c.last_hurt = now;
    if source.is_some() {
        c.last_attacker = source;
    }
    if !hit.flags.contains(HitFlags::TICK) {
        c.knock += hit.knockback * (80.0 / mass) / (1.0 + hyper);
    }
    let mut part_broke = None;
    if let Some(name) = part
        && let Some((p, broken)) = c.parts.iter_mut().find(|(p, _)| p.name == name)
        && let Some(health) = &mut p.health
    {
        *health -= taken;
        if *health <= 0.0 && !*broken {
            *broken = true;
            part_broke = Some(p.name.clone());
        }
    }
    if taken > 0.0 || total > 0.0 {
        w.emit(CombatEvent::Damaged {
            target: id,
            source,
            amount: taken,
            kind: dominant.0,
            point: hit.point,
            critical: hit.flags.contains(HitFlags::CRITICAL),
            flags: hit.flags,
        });
    }
    if let Some(part) = part_broke {
        w.emit(CombatEvent::PartBroken { target: id, part });
    }
    if shatter {
        if let Some(c) = w.get_mut(id) {
            let mut changes = Vec::new();
            c.status.clear(StatusKind::Freeze, &mut changes);
            w.status_changes(id, changes);
        }
        w.emit(CombatEvent::Shattered { target: id });
    }
    if let Some(s) = source
        && s != id
        && !w.hostile(s, id)
        && !hit.flags.contains(HitFlags::AREA)
    {
        w.relations.provoke(s, id);
    }
    if let Some(s) = source {
        crate::ai::hurt_by(w, id, s);
    }
    if check_death(w, id, hit) {
        return taken;
    }
    for &(kind, amount) in &hit.status {
        w.apply_status(id, kind, amount);
    }
    if !hit.flags.contains(HitFlags::TICK) {
        let poise = hit.poise / (1.0 + hyper);
        let broke = w.get_mut(id).is_some_and(|c| c.vitals.break_poise(poise));
        if broke {
            let knockdown = hit.flags.contains(HitFlags::HEAVY);
            w.emit(CombatEvent::PoiseBroken {
                target: id,
                knockdown,
            });
            if knockdown {
                w.apply_status(id, StatusKind::Knockdown, KNOCKDOWN);
            } else {
                w.apply_status(id, StatusKind::Stagger, POISE_STAGGER);
            }
        } else if chanting && taken >= max_health * CHANT_BREAK {
            super::melee::interrupt(w, id);
        }
    }
    taken
}

/// Damage that ignores resistances (a bleed burst).
pub(super) fn true_damage(w: &mut CombatWorld, id: CombatantId, amount: f32, hit: Hit) {
    let Some(c) = w.get_mut(id) else {
        return;
    };
    let taken = c.vitals.damage(amount);
    if taken > 0.0 {
        w.emit(CombatEvent::Damaged {
            target: id,
            source: hit.source,
            amount: taken,
            kind: DamageType::Cut,
            point: hit.point,
            critical: false,
            flags: hit.flags,
        });
    }
    check_death(w, id, &hit);
}

fn check_death(w: &mut CombatWorld, id: CombatantId, hit: &Hit) -> bool {
    let Some(c) = w.get(id) else {
        return false;
    };
    if c.vitals.alive() || !c.alive() {
        return false;
    }
    let mass = c.body.mass;
    let point = if hit.point == Vec3::ZERO {
        c.chest()
    } else {
        hit.point
    };
    let push = hit.knockback.length();
    let impulse = hit.direction.normalize_or_zero() * mass * (2.0 + push);
    let killer = hit.source.or(c.last_attacker);
    let (rider, mount) = (c.rider, c.mount);
    if let Some(c) = w.get_mut(id) {
        c.action = Action::Dead;
        c.shaping = None;
        c.ward = None;
        c.kindled = false;
        c.lock = None;
        c.death = Some(DeathRagdoll {
            t: 0.0,
            blend_in: 0.25,
            impulse,
            point,
        });
    }
    if w.focusing() == Some(id) {
        w.toggle_focus(id);
    }
    w.tactics.forget(id);
    w.emit(CombatEvent::Died {
        target: id,
        killer,
        impulse,
        point,
    });
    // A dead mount throws its rider; a dead rider falls from the saddle.
    if let Some(r) = rider {
        crate::mount::dismount(w, r);
        w.apply_status(r, StatusKind::Knockdown, KNOCKDOWN);
    }
    if mount.is_some() {
        crate::mount::dismount(w, id);
    }
    true
}
