//! Shared harness for the behaviour tests: a world that moves its own bodies,
//! fighters at fixed spots, and a fixed 60 Hz step.
#![allow(dead_code)]
use glam::Vec3;
use somnium_combat::weapon::WeaponDef;
use somnium_combat::{
    Attitude, CombatEvent, CombatRules, CombatWorld, CombatantDesc, CombatantId, Command,
    OpenField, TeamId,
};

pub const DT: f32 = 1.0 / 60.0;
pub const PARTY: TeamId = TeamId(0);
pub const RAIDERS: TeamId = TeamId(1);

pub fn world() -> CombatWorld {
    let mut w = CombatWorld::new(CombatRules {
        integrate_motion: true,
        ..CombatRules::default()
    });
    w.relations.set(PARTY, RAIDERS, Attitude::Hostile);
    w
}

pub fn fighter(
    w: &mut CombatWorld,
    name: &str,
    team: TeamId,
    at: Vec3,
    facing: Vec3,
    weapon: WeaponDef,
) -> CombatantId {
    let mut desc = CombatantDesc::new(name, team, at, weapon);
    desc.facing = facing;
    w.spawn(desc)
}

pub fn spawn(w: &mut CombatWorld, desc: CombatantDesc) -> CombatantId {
    w.spawn(desc)
}

/// Step `seconds`, calling `each` before every step with the step index.
pub fn run(
    w: &mut CombatWorld,
    seconds: f32,
    mut each: impl FnMut(&mut CombatWorld, usize),
) -> Vec<CombatEvent> {
    let steps = (seconds / DT).round() as usize;
    let mut events = Vec::new();
    for i in 0..steps {
        each(w, i);
        w.step(DT, &OpenField, None);
        events.extend(w.drain_events());
    }
    events
}

pub fn idle(w: &mut CombatWorld, seconds: f32) -> Vec<CombatEvent> {
    run(w, seconds, |_, _| {})
}

pub fn aim_at(w: &CombatWorld, target: CombatantId) -> Option<Vec3> {
    w.combatant(target).map(|t| t.position)
}

/// A command that aims at `target` and does nothing else.
pub fn facing(w: &CombatWorld, target: CombatantId) -> Command {
    Command {
        aim: aim_at(w, target),
        ..Command::default()
    }
}

pub fn health(w: &CombatWorld, id: CombatantId) -> f32 {
    w.combatant(id).map_or(0.0, |c| c.vitals.health.current)
}

pub fn damage_to(events: &[CombatEvent], target: CombatantId) -> f32 {
    events
        .iter()
        .filter_map(|e| match e {
            CombatEvent::Damaged {
                target: t, amount, ..
            } if *t == target => Some(*amount),
            _ => None,
        })
        .sum()
}

pub fn count(events: &[CombatEvent], pred: impl Fn(&CombatEvent) -> bool) -> usize {
    events.iter().filter(|e| pred(e)).count()
}
