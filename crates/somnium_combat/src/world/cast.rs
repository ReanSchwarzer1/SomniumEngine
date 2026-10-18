//! Casting: shaping a request on a held key, gathering, release, channel,
//! recovery; paying by address; and turning each shape into its body in the
//! world.
//!
//! The player holds a number key to shape the spell: the cursor sets the
//! bearing, the wheel edits one measure (cycled with the measure key), and the
//! request is fitted to what the caster can say after every change (rote
//! measures snap; numbers do not). Releasing the key casts. A tap casts the
//! slot as it was last left.
use super::effects::{Bloom, Wave, raise_wall, spell_hit};
use super::projectile::{SpellPayload, spawn};
use super::{Action, CombatEvent, CombatWorld, Command, DodgeState};
use crate::damage::HitFlags;
use crate::geometry::{flat, planar_dir};
use crate::spell::{
    Address, Bearing, CastSpeed, Element, MeasureAxis, Shape, SpellAsset, SpellCost, SpellError,
    SpellRequest,
};
use crate::vitals::ManaSpend;
use crate::weapon::{ProjectileDef, ProjectileKind};
use crate::{CombatantId, StatusKind};
use glam::{Quat, Vec3};

/// The phases of a cast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastPhase {
    /// Speaking (or thinking) the verse.
    Gather,
    /// Holding a released wall or ward.
    Channel,
    /// After release.
    Recover,
}

/// A cast in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct CastState {
    /// The request.
    pub request: SpellRequest,
    /// What it costs and takes.
    pub cost: SpellCost,
    /// Phase.
    pub phase: CastPhase,
    /// Seconds in this phase.
    pub t: f32,
    /// The hotbar key holding a channel, if cast from one.
    pub slot: Option<u8>,
}

/// A request being shaped on a held key, with what it would cost now.
#[derive(Clone, Debug, PartialEq)]
pub struct Shaping {
    /// Hotbar slot.
    pub slot: u8,
    /// The request as shaped so far.
    pub request: SpellRequest,
    /// Which measure the wheel edits.
    pub axis: MeasureAxis,
    /// Whether it would be accepted now, and what it would cost.
    pub preview: Result<SpellCost, SpellError>,
}

fn adjust(request: &mut SpellRequest, axis: MeasureAxis, steps: f32) {
    if steps == 0.0 || !steps.is_finite() {
        return;
    }
    let factor = 1.15f32.powf(steps);
    let m = &mut request.measure;
    match axis {
        MeasureAxis::Size => m.size *= factor,
        MeasureAxis::Range => m.range *= factor,
        MeasureAxis::Speed => m.speed *= factor,
        MeasureAxis::Duration => m.duration *= factor,
        MeasureAxis::Count => {
            m.count =
                (f32::from(m.count) + steps.signum() * steps.abs().ceil()).clamp(1.0, 12.0) as u8;
        }
    }
}

/// The bearing a held key aims with now.
fn bearing_for(shape: Shape, aim: Option<Vec3>, lock: Option<CombatantId>) -> Bearing {
    match (shape, lock, aim) {
        (Shape::Ward, _, _) => Bearing::AroundSelf,
        (Shape::Step, _, Some(p)) => Bearing::Cursor(p),
        (_, Some(t), _) => Bearing::Target(t),
        (_, None, Some(p)) => Bearing::Cursor(p),
        _ => Bearing::AroundSelf,
    }
}

fn preview(
    w: &CombatWorld,
    id: CombatantId,
    request: &SpellRequest,
) -> Result<SpellCost, SpellError> {
    let asset = w.spell(&request.spell).ok_or(SpellError::UnknownSpell)?;
    let c = w.get(id).ok_or(SpellError::Busy)?;
    asset.validate(request, &c.magic)?;
    Ok(asset.cost(request))
}

pub(super) fn handle_spell_input(w: &mut CombatWorld, id: CombatantId, cmd: &Command) {
    if let Some(request) = cmd.cast.clone() {
        let _ = begin_cast(w, id, request, None);
    }
    // A channel ends when its key (or the AI's hold) lets go.
    if let Some(c) = w.get_mut(id)
        && let Action::Cast(state) = &mut c.action
        && state.phase == CastPhase::Channel
    {
        let held = state.slot.is_some_and(|s| cmd.spells[s as usize].held) || cmd.channel;
        if !held {
            state.phase = CastPhase::Recover;
            state.t = 0.0;
        }
    }
    for slot in 0..10u8 {
        let press = cmd.spells[slot as usize];
        if press.pressed {
            start_shaping(w, id, slot, cmd);
        }
        let shaping_this = w
            .get(id)
            .and_then(|c| c.shaping.as_ref())
            .is_some_and(|s| s.slot == slot);
        if !shaping_this {
            continue;
        }
        if press.held || press.pressed {
            reshape(w, id, cmd);
        }
        if press.released || (!press.held && !press.pressed) {
            let shaped = w.get_mut(id).and_then(|c| c.shaping.take());
            if let Some(shaped) = shaped {
                let _ = begin_cast(w, id, shaped.request, Some(slot));
            }
        }
    }
}

fn start_shaping(w: &mut CombatWorld, id: CombatantId, slot: u8, cmd: &Command) {
    let Some(c) = w.get(id) else {
        return;
    };
    let Some(entry) = c.magic.hotbar[slot as usize].clone() else {
        return;
    };
    let Some(asset) = w.spell(&entry.spell) else {
        return;
    };
    let request = SpellRequest {
        spell: entry.spell.clone(),
        address: Address::Well,
        element: asset.element,
        shape: entry.shape,
        measure: asset.fit(entry.measure, &c.magic, 0),
        bearing: bearing_for(entry.shape, cmd.aim, c.lock),
        release: entry.release,
    };
    let preview = preview(w, id, &request);
    if let Some(c) = w.get_mut(id) {
        c.shaping = Some(super::Shaping {
            slot,
            request,
            axis: MeasureAxis::Size,
            preview,
        });
    }
}

fn reshape(w: &mut CombatWorld, id: CombatantId, cmd: &Command) {
    let Some(c) = w.get(id) else {
        return;
    };
    let Some(mut shaping) = c.shaping.clone() else {
        return;
    };
    let Some(asset) = w.spell(&shaping.request.spell) else {
        return;
    };
    if cmd.measure_cycle {
        shaping.axis = shaping.axis.next();
    }
    adjust(&mut shaping.request, shaping.axis, cmd.measure_steps);
    let chorus = match &shaping.request.address {
        Address::Chorus(m) => m.len(),
        _ => 0,
    };
    shaping.request.measure = asset.fit(shaping.request.measure, &c.magic, chorus);
    shaping.request.bearing = bearing_for(shaping.request.shape, cmd.aim, c.lock);
    shaping.preview = preview(w, id, &shaping.request);
    let slot = shaping.slot as usize;
    let measure = shaping.request.measure;
    if let Some(c) = w.get_mut(id) {
        // The slot keeps the shape it was last left in.
        if let Some(entry) = &mut c.magic.hotbar[slot] {
            entry.measure = measure;
        }
        c.shaping = Some(shaping);
    }
}

/// Start a cast. Refusals are reported as [`CombatEvent::SpellRefused`] too.
pub fn begin_cast(
    w: &mut CombatWorld,
    id: CombatantId,
    request: SpellRequest,
    slot: Option<u8>,
) -> Result<(), SpellError> {
    let result = check(w, id, &request);
    let cost = match result {
        Ok(cost) => cost,
        Err(error) => {
            w.emit(CombatEvent::SpellRefused {
                caster: id,
                spell: request.spell.clone(),
                error: error.clone(),
            });
            return Err(error);
        }
    };
    let position = w.get(id).map_or(Vec3::ZERO, |c| c.position);
    let loudness = w.spell(&request.spell).map_or(0.5, |a| a.loudness)
        * match request.release {
            CastSpeed::Incantation => 1.0,
            CastSpeed::ShortChant => 0.5,
            CastSpeed::Silent => 0.0,
        };
    if let Some(c) = w.get_mut(id) {
        if cost.focus > 0.0 {
            c.vitals.spend_focus(cost.focus);
        }
        c.shaping = None;
        c.action = Action::Cast(Box::new(CastState {
            request: request.clone(),
            cost,
            phase: CastPhase::Gather,
            t: 0.0,
            slot,
        }));
    }
    w.emit(CombatEvent::CastBegan {
        caster: id,
        spell: request.spell.clone(),
        release: request.release,
        gather: cost.gather,
    });
    w.noise(id, position, loudness);
    if cost.gather <= 0.0 {
        release_now(w, id);
    }
    Ok(())
}

fn check(
    w: &CombatWorld,
    id: CombatantId,
    request: &SpellRequest,
) -> Result<SpellCost, SpellError> {
    let c = w.get(id).ok_or(SpellError::Busy)?;
    if !c.alive() || !c.status.can_act() {
        return Err(SpellError::Busy);
    }
    if !matches!(
        c.action,
        Action::Idle | Action::Guard { .. } | Action::Recover { .. }
    ) {
        return Err(SpellError::Busy);
    }
    let asset = w.spell(&request.spell).ok_or(SpellError::UnknownSpell)?;
    asset.validate(request, &c.magic)?;
    if c.cooldowns
        .get(&request.spell)
        .is_some_and(|ready| *ready > w.time())
    {
        return Err(SpellError::Cooldown);
    }
    let cost = asset.cost(request);
    match request.bearing {
        Bearing::Target(t) => {
            let target = w.get(t).ok_or(SpellError::BadBearing)?;
            if !target.alive()
                || flat(target.position - c.position).length() > request.measure.range + 3.0
            {
                return Err(SpellError::BadBearing);
            }
        }
        Bearing::Cursor(p) if !p.is_finite() => return Err(SpellError::BadBearing),
        _ => {}
    }
    if cost.focus > 0.0 && c.vitals.focus.current < cost.focus {
        return Err(SpellError::NoFocus);
    }
    match &request.address {
        Address::Well => {
            let floor = if c.magic.allow_overdraw {
                -c.vitals.mana.max * c.vitals.overdraw_floor
            } else {
                0.0
            };
            if c.vitals.mana.current - cost.mana < floor {
                return Err(SpellError::NoMana);
            }
        }
        Address::Crystal => {
            if c.magic.crystal < cost.mana {
                return Err(SpellError::NoMana);
            }
        }
        Address::Chorus(members) => {
            let mut total = c.vitals.mana.current.max(0.0);
            for m in members {
                let member = w.get(*m).ok_or(SpellError::BadChorus)?;
                if !member.alive()
                    || !w.allied(id, *m)
                    || member.position.distance(c.position) > 10.0
                {
                    return Err(SpellError::BadChorus);
                }
                total += member.vitals.mana.current.max(0.0);
            }
            if total < cost.mana {
                return Err(SpellError::NoMana);
            }
        }
    }
    Ok(cost)
}

pub(super) fn advance(w: &mut CombatWorld, id: CombatantId, dt: f32) {
    let Some(c) = w.get_mut(id) else {
        return;
    };
    let Action::Cast(state) = &mut c.action else {
        return;
    };
    state.t += dt;
    match state.phase {
        CastPhase::Gather if state.t >= state.cost.gather => release_now(w, id),
        CastPhase::Channel => {
            let spell = state.request.spell.clone();
            let channel_cost = w.spell(&spell).map_or(0.0, |a| a.channel_cost) * dt;
            let Some(c) = w.get_mut(id) else {
                return;
            };
            let paid = c.vitals.spend_mana(channel_cost, false) == ManaSpend::Paid;
            if paid {
                for wall in &mut w.walls {
                    if wall.owner == Some(id) && wall.spell == spell {
                        wall.time_left = wall.time_left.max(0.5);
                    }
                }
                if let Some(c) = w.get_mut(id)
                    && let Some(ward) = &mut c.ward
                {
                    ward.time_left = ward.time_left.max(0.5);
                }
            } else if let Some(c) = w.get_mut(id)
                && let Action::Cast(state) = &mut c.action
            {
                state.phase = CastPhase::Recover;
                state.t = 0.0;
            }
        }
        CastPhase::Recover if state.t >= state.cost.recover => c.action = Action::Idle,
        _ => {}
    }
}

/// Let a gathered verse go (the gather finished, or the clip said `release`).
pub(super) fn release_now(w: &mut CombatWorld, id: CombatantId) {
    let Some(c) = w.get(id) else {
        return;
    };
    let Action::Cast(state) = &c.action else {
        return;
    };
    if state.phase != CastPhase::Gather {
        return;
    }
    let state = (**state).clone();
    let Some(asset) = w.spell(&state.request.spell).cloned() else {
        return;
    };
    // Pay by address.
    let refused = match &state.request.address {
        Address::Well => {
            let allow = c.magic.allow_overdraw;
            let c = w.get_mut(id).expect("present");
            match c.vitals.spend_mana(state.cost.mana, allow) {
                ManaSpend::Paid => false,
                ManaSpend::Overdrawn(debt) => {
                    w.emit(CombatEvent::Overdrawn { caster: id, debt });
                    false
                }
                ManaSpend::Refused => true,
            }
        }
        Address::Crystal => {
            let c = w.get_mut(id).expect("present");
            if c.magic.crystal >= state.cost.mana {
                c.magic.crystal -= state.cost.mana;
                false
            } else {
                true
            }
        }
        Address::Chorus(members) => {
            let members: Vec<CombatantId> =
                std::iter::once(id).chain(members.iter().copied()).collect();
            let capacity: f32 = members
                .iter()
                .filter_map(|m| w.get(*m))
                .map(|m| m.vitals.mana.max.max(1e-3))
                .sum();
            for m in &members {
                if let Some(member) = w.get_mut(*m) {
                    let share = state.cost.mana * member.vitals.mana.max.max(1e-3) / capacity;
                    member.vitals.mana.current -=
                        share.min(member.vitals.mana.current.max(0.0) + share);
                }
            }
            false
        }
    };
    if refused {
        if let Some(c) = w.get_mut(id) {
            c.action = Action::Idle;
        }
        w.emit(CombatEvent::SpellRefused {
            caster: id,
            spell: state.request.spell.clone(),
            error: SpellError::NoMana,
        });
        return;
    }
    let now = w.time();
    let channels =
        asset.channel_cost > 0.0 && matches!(state.request.shape, Shape::Wall | Shape::Ward);
    if let Some(c) = w.get_mut(id) {
        c.cooldowns
            .insert(state.request.spell.clone(), now + f64::from(asset.cooldown));
        c.action = Action::Cast(Box::new(CastState {
            phase: if channels {
                CastPhase::Channel
            } else {
                CastPhase::Recover
            },
            t: 0.0,
            ..state.clone()
        }));
    }
    let at = spawn_effect(w, id, &state.request, &asset, &state.cost);
    w.emit(CombatEvent::SpellReleased {
        caster: id,
        spell: state.request.spell.clone(),
        element: state.request.element,
        shape: state.request.shape,
        at,
    });
}

/// Where a bearing points, clamped to the request's range.
fn resolve_bearing(
    w: &CombatWorld,
    id: CombatantId,
    request: &SpellRequest,
) -> (Vec3, Option<CombatantId>) {
    let c = w.get(id).expect("caster present");
    match request.bearing {
        Bearing::Target(t) => (
            w.get(t).map_or(c.position + c.facing * 3.0, |t| t.position),
            Some(t),
        ),
        Bearing::Cursor(p) => {
            let delta = flat(p - c.position);
            let clamped = delta.clamp_length_max(request.measure.range);
            (
                Vec3::new(c.position.x + clamped.x, p.y, c.position.z + clamped.z),
                None,
            )
        }
        Bearing::AroundSelf => (c.position, None),
    }
}

/// Turn a released request into its body in the world. Returns where it went.
fn spawn_effect(
    w: &mut CombatWorld,
    id: CombatantId,
    request: &SpellRequest,
    asset: &SpellAsset,
    cost: &SpellCost,
) -> Vec3 {
    let (at, target) = resolve_bearing(w, id, request);
    let Some(c) = w.get(id) else {
        return at;
    };
    let rank = c
        .magic
        .rank(asset.element.discipline())
        .unwrap_or(asset.rank);
    let power = cost.power * (1.0 + 0.1 * rank.steps_above(asset.rank) as f32);
    let (position, facing, chest) = (c.position, c.facing, c.chest());
    let dir = planar_dir(at - position, facing);
    let m = request.measure;
    let element = request.element;
    let status: Vec<(StatusKind, f32)> = asset.status.clone();
    match request.shape {
        Shape::Bolt | Shape::Lance => {
            let count = m.count.max(1);
            let heals = element == Element::Mending;
            let hit = spell_hit(Some(id), element, power, asset.poise, &status);
            for k in 0..count {
                let spread = (f32::from(k) - f32::from(count - 1) * 0.5) * 7f32.to_radians();
                let aim = match target {
                    Some(t) => w.get(t).map_or(at, |t| t.chest()),
                    None => at + Vec3::Y * 1.1,
                };
                let toward = (aim - chest).try_normalize().unwrap_or(dir);
                let velocity = Quat::from_rotation_y(spread) * toward * m.speed;
                let def = ProjectileDef {
                    kind: ProjectileKind::Spell,
                    speed: m.speed,
                    gravity: 0.0,
                    radius: 0.12 + m.size * 0.15,
                    range: m.range,
                    damage: if heals {
                        vec![crate::DamagePart::new(crate::DamageType::Mana, power)]
                    } else {
                        hit.parts.clone()
                    },
                    poise: asset.poise,
                    status: hit.status.clone(),
                    pierce: if request.shape == Shape::Lance { 3 } else { 0 },
                    knockback: asset.knockback,
                    head_multiplier: 1.0,
                    wind_factor: 0.0,
                };
                spawn(
                    w,
                    id,
                    def,
                    chest + dir * 0.4,
                    velocity,
                    1.0,
                    HitFlags::SPELL,
                    Some(SpellPayload {
                        spell: asset.id.clone(),
                        element,
                        size: m.size,
                        heals,
                    }),
                    target,
                );
            }
        }
        Shape::Wall => {
            let base = if target.is_some() || matches!(request.bearing, Bearing::Cursor(_)) {
                at
            } else {
                position + facing * 3.0
            };
            raise_wall(
                w,
                id,
                element,
                Vec3::new(base.x, position.y, base.z),
                dir,
                m.size,
                m.duration,
                power,
                &asset.id,
            );
        }
        Shape::Wave => {
            let half_angle = (m.size * 12.0).clamp(15.0, 75.0).to_radians();
            w.waves.push(Wave {
                owner: Some(id),
                element,
                origin: position,
                dir,
                half_angle,
                radius: 0.0,
                max_radius: m.range,
                speed: m.speed,
                power,
                status: element.natural_status().into_iter().chain(status).collect(),
                poise: asset.poise,
                knockback: asset.knockback,
                hit: Vec::new(),
            });
            w.emit(CombatEvent::WaveSent {
                origin: position,
                dir,
                element,
                half_angle,
                range: m.range,
            });
        }
        Shape::Bloom => {
            let center = if matches!(request.bearing, Bearing::AroundSelf) {
                position
            } else {
                at
            };
            let delay = 0.45;
            w.blooms.push(Bloom {
                owner: Some(id),
                element,
                center,
                radius: m.size,
                delay,
                power,
                status: element.natural_status().into_iter().chain(status).collect(),
                poise: asset.poise,
                knockback: asset.knockback,
            });
            w.emit(CombatEvent::BloomGathering {
                at: center,
                radius: m.size,
                element,
                delay,
            });
        }
        Shape::Step => {
            let to = flat(at - position);
            let distance = to.length().min(m.range);
            let duration = (distance / m.speed.max(1.0)).clamp(0.1, 1.2);
            if let Some(c) = w.get_mut(id) {
                c.action = Action::Dodge(DodgeState {
                    t: 0.0,
                    duration,
                    dir,
                    distance,
                    iframes: [0.0, 0.15],
                    trail: matches!(element, Element::Frost | Element::Flame | Element::Water)
                        .then_some(element),
                    from: position,
                });
            }
            w.emit(CombatEvent::Stepped {
                caster: id,
                from: position,
                to: position + dir * distance,
                element,
            });
        }
        Shape::Ward => {
            let bearer = target.filter(|t| w.allied(id, *t)).unwrap_or(id);
            let ward = super::Ward {
                element,
                strength: power,
                time_left: m.duration,
                armor: if element == Element::Stone {
                    power * 0.5
                } else {
                    0.0
                },
            };
            if let Some(b) = w.get_mut(bearer) {
                b.ward = Some(ward);
            }
            w.emit(CombatEvent::WardRaised {
                target: bearer,
                element,
                strength: power,
            });
        }
    }
    at
}
