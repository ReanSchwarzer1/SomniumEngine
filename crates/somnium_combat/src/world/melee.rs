//! Commands into actions: swings and chains, the heavy, the charge, guards and
//! feints, dodges, shots, school arts; and the melee hit-volume tests.
use super::{
    Action, AttackState, CombatEvent, CombatWorld, Combatant, Command, DodgeState, MoveSource,
    Phase, cast,
};
use crate::damage::{Hit, HitFlags};
use crate::geometry::{Arc, Capsule, flat, planar_dir};
use crate::schools::Form;
use crate::spell::Element;
use crate::weapon::{HitShape, MoveDef, MoveKind, ProjectileDef, ProjectileKind, WeaponKind};
use crate::{CombatQuery, CombatantId, DamagePart, DamageType, StatusKind};
use glam::Vec3;

/// Seconds the attack button must be held for a light opener to become the
/// heavy. Shorter than every heavy-capable opener's windup, so the swing that
/// started on the click turns into the heavy before it lands.
pub const HEAVY_HOLD: f32 = 0.22;
/// Stamina a dodge costs.
pub const DODGE_STAMINA: f32 = 18.0;
/// Stamina a feint costs.
pub const FEINT_STAMINA: f32 = 8.0;

pub(super) fn handle_command(
    w: &mut CombatWorld,
    id: CombatantId,
    dt: f32,
    _query: &impl CombatQuery,
) {
    let Some(c) = w.get_mut(id) else {
        return;
    };
    if !c.alive() {
        c.command = Command::default();
        return;
    }
    let cmd = c.command.clone();
    c.command.consume_edges();
    if cmd.attack.held {
        c.attack_held += dt;
    } else {
        c.attack_held = 0.0;
    }

    if cmd.kindle {
        let on = !c.kindled && c.vitals.mana.current >= 1.0;
        if on != c.kindled {
            c.kindled = on;
            w.emit(CombatEvent::Kindled { who: id, on });
        }
    }
    if cmd.lock {
        toggle_lock(w, id, &cmd);
    }
    if cmd.focus {
        w.toggle_focus(id);
    }
    if cmd.mount && w.get(id).is_some_and(|c| c.status.can_act()) {
        let _ = crate::mount::toggle(w, id);
    }
    if cmd.whistle {
        let _ = crate::mount::whistle(w, id);
    }

    cast::handle_spell_input(w, id, &cmd);

    if cmd.dodge {
        try_dodge(w, id, &cmd);
    }
    if cmd.guard.pressed {
        try_guard(w, id);
    } else if !cmd.guard.held
        && let Some(c) = w.get_mut(id)
        && matches!(c.action, Action::Guard { .. })
    {
        c.action = Action::Idle;
    }
    if cmd.attack.pressed {
        try_attack(w, id, &cmd);
    }
    if cmd.attack.released {
        release_shot(w, id);
    }
    if cmd.attack.held && w.get(id).is_some_and(|c| c.attack_held >= HEAVY_HOLD) {
        convert_to_heavy(w, id);
    }
    if cmd.art {
        try_art(w, id, &cmd);
    }
}

fn toggle_lock(w: &mut CombatWorld, id: CombatantId, cmd: &Command) {
    let Some(me) = w.get(id) else {
        return;
    };
    let target = if me.lock.is_some() {
        None
    } else {
        let toward = cmd.aim.unwrap_or(me.position + me.facing * 5.0);
        let ids = w.ids();
        ids.into_iter()
            .filter(|&o| o != id && w.hostile(id, o))
            .filter_map(|o| w.get(o))
            .filter(|o| o.alive() && o.position.distance(me.position) <= 18.0)
            .min_by(|a, b| {
                a.position
                    .distance(toward)
                    .total_cmp(&b.position.distance(toward))
            })
            .map(|o| o.id)
    };
    if let Some(c) = w.get_mut(id) {
        c.lock = target;
    }
    w.emit(CombatEvent::LockChanged { who: id, target });
}

/// The charge this fighter can make: the weapon's own, or a sword's when the
/// Aetherlance's Charge is known.
fn charge_move(c: &Combatant) -> Option<MoveDef> {
    if let Some(m) = &c.weapon.charge {
        return Some(m.clone());
    }
    if c.weapon.kind == WeaponKind::Sword && c.knows(Form::TheCharge) {
        let mut m = crate::weapon::lance_glaive().charge?;
        m.name = "sword_charge".into();
        m.animation = "sword_charge".into();
        m.damage = vec![DamagePart::new(DamageType::Pierce, 36.0)];
        m.lunge = 6.0;
        return Some(m);
    }
    None
}

/// The move a source names, with school forms applied.
fn resolve_move(c: &Combatant, source: MoveSource) -> Option<MoveDef> {
    let mut m = match source {
        MoveSource::Light(i) => c.weapon.light.get(i as usize).cloned()?,
        MoveSource::Heavy => c.weapon.heavy.clone()?,
        MoveSource::Charge => charge_move(c)?,
    };
    if let Some(s) = c.sword {
        let reach = s.reach_scale();
        if let HitShape::Arc { radius, .. } = &mut m.shape {
            *radius *= reach;
        }
        let finisher = matches!(source, MoveSource::Light(_)) && m.next.is_none();
        if finisher && s.knows(Form::HrimgardsTooth) {
            m.status.push((StatusKind::Chill, 60.0));
        }
        if source == MoveSource::Light(0) && s.knows(Form::LowMoon) {
            m.kind = MoveKind::Sweep;
            m.heavy = true;
            if let HitShape::Arc { low, high, .. } = &mut m.shape {
                *low = -1.4;
                *high = -0.3;
            }
        }
        if c.mount.is_some() && s.knows(Form::KulansRun) {
            for part in &mut m.damage {
                part.amount *= 1.5;
            }
        }
    }
    Some(m)
}

pub(super) fn start_move(w: &mut CombatWorld, id: CombatantId, source: MoveSource) -> bool {
    let now = w.time();
    let riposte = w.rules.riposte;
    let Some(c) = w.get_mut(id) else {
        return false;
    };
    let Some(def) = resolve_move(c, source) else {
        return false;
    };
    if !c.vitals.spend_stamina(def.stamina) {
        return false;
    }
    let mut flags = HitFlags::MELEE;
    let mut scale = c.sword.map_or(1.0, |s| s.damage_scale()) * c.kindle_strength();
    let is_riposte = now < c.riposte_until;
    if is_riposte {
        flags |=
            HitFlags::RIPOSTE | HitFlags::CRITICAL | HitFlags::UNBLOCKABLE | HitFlags::UNPARRYABLE;
        scale *= riposte * c.sword.map_or(1.0, |s| s.riposte_scale());
        c.riposte_until = f64::NEG_INFINITY;
    }
    if def.heavy {
        flags |= HitFlags::HEAVY;
    }
    let aim = c
        .command
        .aim
        .map(|p| p - c.position)
        .filter(|d| flat(*d).length_squared() > 0.01);
    let dir = planar_dir(aim.unwrap_or(c.facing), c.facing);
    c.facing = dir;
    let (name, animation) = (def.name.clone(), def.animation.clone());
    c.swings = c.swings.wrapping_add(1);
    c.action = Action::Attack(Box::new(AttackState {
        source,
        def,
        phase: Phase::Windup,
        t: 0.0,
        hit: Vec::new(),
        flags,
        scale,
        chain: false,
        dir,
        travelled: 0.0,
    }));
    let position = c.position;
    w.emit(CombatEvent::AttackStarted {
        attacker: id,
        name,
        animation,
        riposte: is_riposte,
    });
    w.noise(id, position, 0.25);
    true
}

fn try_attack(w: &mut CombatWorld, id: CombatantId, cmd: &Command) {
    let Some(c) = w.get_mut(id) else {
        return;
    };
    if !c.status.can_act() {
        return;
    }
    c.shaping = None;
    match &mut c.action {
        Action::Attack(a) => {
            // Buffered: chained when the recovery's window opens.
            a.chain = true;
            return;
        }
        Action::Idle | Action::Guard { .. } => {}
        _ => return,
    }
    let shoots = matches!(c.weapon.kind, WeaponKind::Bow | WeaponKind::LatticeRifle)
        && c.weapon.ranged.is_some();
    if shoots {
        if c.ammo == 0 {
            let reload = c.weapon.ranged.as_ref().map_or(0.0, |r| r.reload);
            c.action = Action::Reload { left: reload };
        } else {
            c.action = Action::Aim { t: 0.0 };
        }
        return;
    }
    let moving = cmd.sprint && cmd.move_dir.length_squared() > 0.25;
    if moving && charge_move(c).is_some() {
        start_move(w, id, MoveSource::Charge);
        return;
    }
    start_move(w, id, MoveSource::Light(0));
}

fn convert_to_heavy(w: &mut CombatWorld, id: CombatantId) {
    let Some(c) = w.get(id) else {
        return;
    };
    let convertible = matches!(&c.action, Action::Attack(a)
        if a.source == MoveSource::Light(0) && a.phase == Phase::Windup)
        && c.weapon.heavy.is_some();
    if convertible {
        // Refund the opener's stamina; the heavy charges its own.
        let refund = match &c.action {
            Action::Attack(a) => a.def.stamina,
            _ => 0.0,
        };
        if let Some(c) = w.get_mut(id) {
            c.vitals.stamina.current =
                (c.vitals.stamina.current + refund).min(c.vitals.stamina.max);
            c.action = Action::Idle;
            c.attack_held = f32::NEG_INFINITY;
        }
        start_move(w, id, MoveSource::Heavy);
    }
}

fn try_guard(w: &mut CombatWorld, id: CombatantId) {
    let Some(c) = w.get_mut(id) else {
        return;
    };
    if !c.status.can_act() {
        return;
    }
    let feint = matches!(&c.action, Action::Attack(a) if a.phase == Phase::Windup)
        && c.knows(Form::GrievesTurn);
    if feint {
        if c.vitals.spend_stamina(FEINT_STAMINA) {
            c.action = Action::Guard { t: 0.0 };
            w.emit(CombatEvent::Feinted { attacker: id });
        }
        return;
    }
    let free = match &c.action {
        Action::Idle | Action::Aim { .. } | Action::Recover { .. } => true,
        Action::Attack(a) => {
            a.phase == Phase::Recovery && a.t >= a.def.recovery * a.def.dodge_cancel
        }
        _ => false,
    };
    if free {
        c.shaping = None;
        c.action = Action::Guard { t: 0.0 };
    }
}

fn try_dodge(w: &mut CombatWorld, id: CombatantId, cmd: &Command) {
    let lock_target = w
        .get(id)
        .and_then(|c| c.lock)
        .and_then(|t| w.get(t))
        .map(|t| (t.position, t.facing));
    let Some(c) = w.get_mut(id) else {
        return;
    };
    if !c.status.can_act() || c.mount.is_some() {
        return;
    }
    let free = match &c.action {
        Action::Idle | Action::Guard { .. } | Action::Aim { .. } | Action::Recover { .. } => true,
        Action::Attack(a) => {
            a.phase == Phase::Recovery && a.t >= a.def.recovery * a.def.dodge_cancel
        }
        Action::Cast(s) => s.phase == cast::CastPhase::Recover,
        _ => false,
    };
    if !free || !c.vitals.spend_stamina(DODGE_STAMINA) {
        return;
    }
    c.shaping = None;
    let scale = c.sword.map_or(1.0, |s| s.dodge_scale());
    let mut dodge = DodgeState {
        t: 0.0,
        duration: 0.42,
        dir: planar_dir(cmd.move_dir, -c.facing),
        distance: 3.2 * scale,
        iframes: [0.04, 0.32],
        trail: None,
        from: c.position,
    };
    if c.knows(Form::FrostStep) {
        dodge.trail = Some(Element::Frost);
        dodge.iframes[1] += 0.05;
    }
    if c.hollowborn
        && c.knows(Form::ShadowStep)
        && let Some((tp, tf)) = lock_target
    {
        let behind = tp - planar_dir(tf, Vec3::Z) * 1.2;
        let to = flat(behind - c.position);
        if to.length() <= 7.0 {
            dodge.dir = to.try_normalize().unwrap_or(dodge.dir);
            dodge.distance = to.length();
            dodge.duration = 0.2;
            dodge.iframes = [0.0, 0.2];
        }
    }
    let dir = dodge.dir;
    c.action = Action::Dodge(dodge);
    w.emit(CombatEvent::DodgeStarted { who: id, dir });
}

fn try_art(w: &mut CombatWorld, id: CombatantId, cmd: &Command) {
    let now = w.time();
    let Some(c) = w.get(id) else {
        return;
    };
    if !c.status.can_act() || !matches!(c.action, Action::Idle | Action::Guard { .. }) {
        return;
    }
    let throws = c.weapon.kind == WeaponKind::Knife && c.weapon.ranged.is_some() && c.ammo > 0;
    let sword = c.sword;
    let charges = charge_move(c).is_some();
    // A knife-fighter throws.
    if throws {
        fire(w, id, 1.0);
        return;
    }
    let Some(sword) = sword else {
        return;
    };
    if sword.knows(Form::TheCharge) && charges {
        start_move(w, id, MoveSource::Charge);
        return;
    }
    if sword.knows(Form::LanternOut) {
        if let Some(c) = w.get_mut(id)
            && c.vitals.spend_stamina(10.0)
        {
            c.concealed_until = now + 6.0;
            c.action = Action::Recover { left: 0.3 };
            w.emit(CombatEvent::Concealed {
                who: id,
                seconds: 6.0,
            });
        }
        return;
    }
    if sword.knows(Form::WindBlade) {
        let ok = w.get_mut(id).is_some_and(|c| c.vitals.spend_stamina(12.0));
        if ok {
            wind_cut(w, id, cmd.aim, 14.0, 9.0);
            if let Some(c) = w.get_mut(id) {
                c.action = Action::Recover { left: 0.35 };
            }
        }
    }
}

/// A cut that flies (Gale Edge).
fn wind_cut(w: &mut CombatWorld, id: CombatantId, aim: Option<Vec3>, damage: f32, range: f32) {
    let def = ProjectileDef {
        kind: ProjectileKind::WindCut,
        speed: 26.0,
        gravity: 0.0,
        radius: 0.45,
        range,
        damage: vec![DamagePart::new(DamageType::Gale, damage)],
        poise: 14.0,
        status: Vec::new(),
        pierce: 2,
        knockback: 2.0,
        head_multiplier: 1.0,
        wind_factor: 0.0,
    };
    super::projectile::launch(w, id, def, aim, 1.0, None, HitFlags::MELEE);
}

/// Loose a shot at `draw` (0 to 1).
pub(super) fn fire(w: &mut CombatWorld, id: CombatantId, draw: f32) {
    let Some(c) = w.get(id) else {
        return;
    };
    let Some(ranged) = c.weapon.ranged.clone() else {
        return;
    };
    let aim = c
        .lock
        .and_then(|t| w.get(t))
        .map(|t| t.chest())
        .or(c.command.aim.map(|p| p + Vec3::Y * 1.1));
    let draw = draw.clamp(0.0, 1.0);
    let strength = ranged.weak_shot + (1.0 - ranged.weak_shot) * draw;
    let mut def = ranged.projectile.clone();
    if ranged.release_to_fire {
        def.speed *= 0.6 + 0.4 * draw;
    }
    let spread = ranged.spread * (2.0 - draw);
    super::projectile::launch(w, id, def, aim, strength, Some(spread), HitFlags::default());
    let position = w.get(id).map_or(Vec3::ZERO, |c| c.position);
    w.noise(id, position, ranged.loudness);
    if let Some(c) = w.get_mut(id) {
        c.vitals.spend_stamina(ranged.stamina);
        c.ammo = c.ammo.saturating_sub(1);
        c.action = if c.ammo == 0 && ranged.reload > 0.0 {
            Action::Reload {
                left: ranged.recovery + ranged.reload,
            }
        } else {
            if ranged.reload <= 0.0 {
                c.ammo = ranged.magazine;
            }
            Action::Recover {
                left: ranged.recovery,
            }
        };
    }
}

fn release_shot(w: &mut CombatWorld, id: CombatantId) {
    let Some(c) = w.get(id) else {
        return;
    };
    let Action::Aim { t } = c.action else {
        return;
    };
    let Some(r) = &c.weapon.ranged else {
        return;
    };
    if !r.release_to_fire {
        return;
    }
    let draw = (t / r.draw.max(0.01)).min(1.0);
    if draw >= r.min_draw {
        fire(w, id, draw);
    } else if let Some(c) = w.get_mut(id) {
        c.action = Action::Idle;
    }
}

/// Break whatever the body was doing (stagger, knockdown, freeze).
pub(crate) fn interrupt(w: &mut CombatWorld, id: CombatantId) {
    let Some(c) = w.get_mut(id) else {
        return;
    };
    c.shaping = None;
    let spell = match &c.action {
        Action::Cast(s) if s.phase == cast::CastPhase::Gather => Some(s.request.spell.clone()),
        _ => None,
    };
    if !matches!(c.action, Action::Dead | Action::Dodge(_)) {
        c.action = Action::Idle;
    }
    if let Some(spell) = spell {
        w.emit(CombatEvent::SpellInterrupted { caster: id, spell });
    }
}

/// Phase hooks when a blow goes live.
fn on_active(w: &mut CombatWorld, id: CombatantId) {
    let Some(c) = w.get(id) else {
        return;
    };
    let Action::Attack(a) = &c.action else {
        return;
    };
    let (source, finisher) = (
        a.source,
        matches!(a.source, MoveSource::Light(_)) && a.def.next.is_none(),
    );
    let aim = c.command.aim;
    let Some(sword) = c.sword else {
        return;
    };
    let mounted = c.mount.is_some();
    if source == MoveSource::Heavy && sword.knows(Form::ShaftLance) {
        // An Ice Lance along the blade, if the well allows.
        let paid = w
            .get_mut(id)
            .is_some_and(|c| c.vitals.spend_mana(2.5, false) == crate::vitals::ManaSpend::Paid);
        if paid {
            let def = ProjectileDef {
                kind: ProjectileKind::Spell,
                speed: 50.0,
                gravity: 0.0,
                radius: 0.15,
                range: 30.0,
                damage: vec![DamagePart::new(DamageType::Frost, 22.0)],
                poise: 20.0,
                status: vec![(StatusKind::Chill, 35.0)],
                pierce: 2,
                knockback: 1.5,
                head_multiplier: 1.0,
                wind_factor: 0.0,
            };
            super::projectile::launch(w, id, def, aim, 1.0, None, HitFlags::SPELL);
        }
    }
    if source == MoveSource::Heavy && sword.knows(Form::SkyShrine) {
        wind_cut(w, id, aim, 24.0, 14.0);
    } else if (finisher || mounted) && sword.knows(Form::WindBlade) {
        wind_cut(w, id, aim, 14.0, 8.0);
    }
}

/// What [`advance`] must do once it has let go of the combatant.
enum After {
    Nothing,
    Fire(f32),
    Active,
    Chain(u8),
    CheckChain,
    Cast(f32),
    Trail(Element, Vec3, Vec3),
}

pub(super) fn advance(w: &mut CombatWorld, id: CombatantId, dt: f32, _query: &impl CombatQuery) {
    let after = {
        let Some(c) = w.get_mut(id) else {
            return;
        };
        let scale = c.status.action_scale();
        match &mut c.action {
            Action::Idle | Action::Dead => After::Nothing,
            Action::Guard { t } => {
                *t += dt;
                After::Nothing
            }
            Action::Recover { left } => {
                *left -= dt;
                if *left <= 0.0 {
                    c.action = Action::Idle;
                }
                After::Nothing
            }
            Action::Reload { left } => {
                *left -= dt * scale;
                if *left <= 0.0 {
                    c.ammo = c.weapon.ranged.as_ref().map_or(0, |r| r.magazine);
                    c.action = Action::Idle;
                }
                After::Nothing
            }
            Action::Aim { t } => {
                *t += dt * scale;
                let t = *t;
                match c
                    .weapon
                    .ranged
                    .as_ref()
                    .map(|r| (r.release_to_fire, r.draw))
                {
                    None => {
                        c.action = Action::Idle;
                        After::Nothing
                    }
                    Some((false, draw)) if t >= draw => After::Fire(1.0),
                    Some((true, draw)) if t >= draw => {
                        // Holding a full draw tires the arm.
                        c.vitals.drain_stamina(4.0 * dt);
                        if c.vitals.stamina.current <= 0.0 {
                            After::Fire(0.6)
                        } else {
                            After::Nothing
                        }
                    }
                    Some(_) => After::Nothing,
                }
            }
            Action::Dodge(d) => {
                d.t += dt;
                if d.t >= d.duration {
                    let (trail, from, to) = (d.trail, d.from, c.position);
                    c.action = Action::Idle;
                    trail.map_or(After::Nothing, |e| After::Trail(e, from, to))
                } else {
                    After::Nothing
                }
            }
            Action::Attack(a) => {
                a.t += dt * scale;
                if a.def.driven_by_events {
                    let elapsed = match a.phase {
                        Phase::Windup => a.t,
                        Phase::Active => a.def.windup + a.t,
                        Phase::Recovery => a.def.windup + a.def.active + a.t,
                    };
                    if elapsed > a.def.duration() * 1.6 {
                        c.action = Action::Idle;
                        After::Nothing
                    } else {
                        After::CheckChain
                    }
                } else {
                    match a.phase {
                        Phase::Windup if a.t >= a.def.windup => {
                            a.t -= a.def.windup;
                            a.phase = Phase::Active;
                            After::Active
                        }
                        Phase::Active => {
                            if a.def.kind == MoveKind::Charge {
                                a.travelled += a.def.lunge / a.def.active.max(0.05) * dt * scale;
                            }
                            if a.t >= a.def.active {
                                a.t -= a.def.active;
                                a.phase = Phase::Recovery;
                            }
                            After::CheckChain
                        }
                        Phase::Recovery if a.t >= a.def.recovery => {
                            let (chain, next) = (a.chain, a.def.next);
                            c.action = Action::Idle;
                            // A chain asked for at the very end still carries on.
                            match next {
                                Some(n) if chain => After::Chain(n),
                                _ => After::Nothing,
                            }
                        }
                        _ => After::CheckChain,
                    }
                }
            }
            Action::Cast(_) => After::Cast(dt * scale),
        }
    };
    match after {
        After::Nothing => {}
        After::Fire(draw) => fire(w, id, draw),
        After::Active => on_active(w, id),
        After::Chain(n) => {
            start_move(w, id, MoveSource::Light(n));
        }
        After::CheckChain => check_chain(w, id),
        After::Cast(dt) => cast::advance(w, id, dt),
        After::Trail(element, from, to) => {
            if let Some(field) = &mut w.environment {
                field.react_line(element, from, to, 0.7);
            }
        }
    }
}

fn check_chain(w: &mut CombatWorld, id: CombatantId) {
    let Some(c) = w.get(id) else {
        return;
    };
    let Action::Attack(a) = &c.action else {
        return;
    };
    if a.phase != Phase::Recovery || !a.chain {
        return;
    }
    let frac = a.t / a.def.recovery.max(1e-3);
    if frac >= a.def.combo_window[0]
        && frac <= a.def.combo_window[1]
        && let Some(next) = a.def.next
    {
        if let Some(c) = w.get_mut(id) {
            c.action = Action::Idle;
        }
        start_move(w, id, MoveSource::Light(next));
    }
}

pub(super) fn animation_event(
    w: &mut CombatWorld,
    id: CombatantId,
    event: &somnium_anim::AnimationEvent,
) {
    let Some(c) = w.get_mut(id) else {
        return;
    };
    match (&mut c.action, event.name.as_str()) {
        (Action::Attack(a), "hit_on") if a.def.driven_by_events => {
            a.phase = Phase::Active;
            a.t = 0.0;
            on_active(w, id);
        }
        (Action::Attack(a), "hit_off") if a.def.driven_by_events => {
            a.phase = Phase::Recovery;
            a.t = 0.0;
        }
        (Action::Attack(a), "recover") if a.def.driven_by_events => {
            let (chain, next) = (a.chain, a.def.next);
            c.action = Action::Idle;
            if chain && let Some(n) = next {
                start_move(w, id, MoveSource::Light(n));
            }
        }
        (Action::Cast(_), "release") => cast::release_now(w, id),
        _ => {}
    }
}

/// The world-space volume a live blow sweeps.
enum Volume {
    Arc(Arc),
    Capsules(Vec<Capsule>),
}

fn volume(c: &Combatant, a: &AttackState) -> Volume {
    let chest = c.chest();
    match a.def.shape {
        HitShape::Arc {
            radius,
            half_angle,
            low,
            high,
        } => Volume::Arc(Arc {
            origin: chest,
            forward: a.dir,
            radius,
            half_angle: half_angle.to_radians(),
            low,
            high,
            inner: 0.0,
        }),
        HitShape::Thrust { length, radius } => Volume::Capsules(vec![Capsule {
            a: chest + a.dir * 0.3,
            b: chest + a.dir * length,
            radius,
        }]),
        HitShape::Sphere { forward, radius } => {
            let p = chest + a.dir * forward - Vec3::Y * 0.3;
            Volume::Capsules(vec![Capsule { a: p, b: p, radius }])
        }
        HitShape::Blade { radius } => match (c.prev_blade, c.blade) {
            (prev, Some(now)) => {
                let prev = prev.unwrap_or(now);
                Volume::Capsules(
                    (0..=4)
                        .map(|i| {
                            let s = i as f32 / 4.0;
                            Capsule {
                                a: prev[0].lerp(now[0], s),
                                b: prev[1].lerp(now[1], s),
                                radius,
                            }
                        })
                        .collect(),
                )
            }
            _ => Volume::Arc(Arc {
                origin: chest,
                forward: a.dir,
                radius: c.weapon.reach,
                half_angle: 60f32.to_radians(),
                low: -1.0,
                high: 0.7,
                inner: 0.0,
            }),
        },
    }
}

/// Where a volume touches a target's body or one of its parts.
fn touch(volume: &Volume, target: &Combatant) -> Option<(Vec3, Option<usize>)> {
    let test = |cap: &Capsule| match volume {
        Volume::Arc(arc) => arc.hits(cap),
        Volume::Capsules(list) => list.iter().find_map(|v| v.overlaps(cap)),
    };
    for (i, (part, broken)) in target.parts.iter().enumerate() {
        if *broken {
            continue;
        }
        let rot = glam::Quat::from_rotation_arc(Vec3::Z, target.facing);
        let cap = Capsule {
            a: target.position + rot * part.a,
            b: target.position + rot * part.b,
            radius: part.radius,
        };
        if let Some(p) = test(&cap) {
            return Some((p, Some(i)));
        }
    }
    test(&target.capsule()).map(|p| (p, None))
}

pub(super) fn collect_hits(w: &mut CombatWorld) {
    let ids = w.ids();
    for id in &ids {
        let Some(c) = w.get(*id) else {
            continue;
        };
        let Action::Attack(a) = &c.action else {
            continue;
        };
        if a.phase != Phase::Active {
            continue;
        }
        let vol = volume(c, a);
        let mut struck = Vec::new();
        for other in &ids {
            if other == id || a.hit.contains(other) || !w.may_hit(Some(*id), *other, false) {
                continue;
            }
            let Some(t) = w.get(*other) else {
                continue;
            };
            if !t.alive() || t.mount == Some(*id) || t.rider == Some(*id) {
                continue;
            }
            if let Some((point, part)) = touch(&vol, t) {
                let behind = t.facing.dot(planar_dir(c.position - t.position, t.facing)) < -0.5
                    && t.guard_time().is_none();
                struck.push((*other, point, part, behind));
            }
        }
        if struck.is_empty() {
            continue;
        }
        let a = a.clone();
        let backstab = c.weapon.backstab;
        let kindle = c.kindle_strength();
        let charge = a.def.kind == MoveKind::Charge;
        let mut hits = Vec::new();
        for (target, point, part, behind) in &struck {
            let mut hit = Hit::new(Some(*id), a.def.damage.clone(), a.def.poise * kindle);
            hit.status = a.def.status.clone();
            hit.knockback = a.dir * a.def.knockback;
            hit.point = *point;
            hit.direction = a.dir;
            hit.flags = a.flags;
            hit.scale = a.scale;
            if *behind {
                hit.flags |= HitFlags::BACKSTAB | HitFlags::CRITICAL;
                hit.scale *= backstab;
            }
            let part_name = part.and_then(|i| w.get(*target).map(|t| t.parts[i].0.name.clone()));
            hits.push((*target, hit, part_name));
        }
        if let Some(c) = w.get_mut(*id)
            && let Action::Attack(a) = &mut c.action
        {
            a.hit.extend(struck.iter().map(|s| s.0));
            if charge {
                // A charge stops in the first body it meets.
                a.phase = Phase::Recovery;
                a.t = 0.0;
            }
        }
        for (target, hit, part) in hits {
            w.queue_hit(target, hit, part);
        }
    }
    // The blade sweep starts from where it ended this step.
    for id in ids {
        if let Some(c) = w.get_mut(id) {
            c.prev_blade = c.blade;
        }
    }
}
