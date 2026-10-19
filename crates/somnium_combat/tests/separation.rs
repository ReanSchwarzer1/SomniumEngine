//! A crowd's pushes from the grid are the same bits as one body's from
//! everyone: a packed crowd of mixed builds, some fallen, every push equal.
mod common;
use common::*;
use glam::Vec3;
use somnium_combat::world::Action;
use somnium_combat::{CombatWorld, CombatantDesc, CombatantId, OpenField, weapon};

#[test]
fn a_crowd_is_pushed_exactly_as_separation_pushes_each_body() {
    let mut w: CombatWorld = world();
    let mut ids: Vec<CombatantId> = Vec::new();
    let mut seed = 0x9e37_79b9_u32;
    let mut unit = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    for i in 0..300 {
        let at = Vec3::new(unit() * 30.0 - 15.0, 0.0, unit() * 30.0 - 15.0);
        let mut desc = CombatantDesc::new("Body", RAIDERS, at, weapon::axe());
        desc.body.radius = 0.25 + unit() * 0.6;
        let id = w.spawn(desc);
        if i % 9 == 0 {
            w.combatant_mut(id).unwrap().action = Action::Dead;
        }
        ids.push(id);
    }
    let fast = w.separations(&ids);
    let mut pushed = 0;
    for (id, push) in fast {
        let slow = w.separation(id);
        assert_eq!(push.to_array().map(f32::to_bits), slow.to_array().map(f32::to_bits), "{id:?}");
        pushed += usize::from(push != Vec3::ZERO);
    }
    assert!(pushed > 100, "a packed crowd pushes: {pushed}");
    // And the world's own step moves them apart as before.
    w.step(DT, &OpenField, None);
}
