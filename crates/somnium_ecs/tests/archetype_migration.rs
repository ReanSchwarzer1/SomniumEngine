//! Phase 16-A: runtime component insert/remove and archetype migration.
//!
//! Migration is the most safety-critical code in the ECS: it moves owned
//! values between two type-erased byte arrays without the compiler
//! watching. These tests exist to pin down the three things that are easy
//! to get wrong and impossible to notice until much later:
//!
//! 1. a **moved** component must not be dropped;
//! 2. a **removed** or **replaced** component must be dropped exactly once;
//! 3. the entity that swap-remove pulls into the vacated row must have its
//!    recorded location patched.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use somnium_ecs::{Component, ComponentId, EcsError, World};

// ── Fixtures ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
struct Pos {
    x: f32,
    y: f32,
}
impl Component for Pos {}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Vel {
    dx: f32,
    dy: f32,
}
impl Component for Vel {}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Tag;
impl Component for Tag {}

/// A non-`Copy` component that counts its own destructor calls into a
/// counter the test owns, so tests can run in parallel without sharing a
/// global.
#[derive(Debug)]
struct Tracked {
    label: String,
    drops: Arc<AtomicUsize>,
}
impl Component for Tracked {}
impl Drop for Tracked {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}

fn tracked(label: &str, drops: &Arc<AtomicUsize>) -> Tracked {
    Tracked {
        label: label.to_string(),
        drops: Arc::clone(drops),
    }
}

fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

// ── Basic migration ────────────────────────────────────────────────────

#[test]
fn insert_migrates_and_preserves_existing_components() {
    let mut world = World::new();
    let e = world.spawn((Pos { x: 1.0, y: 2.0 },));
    assert_eq!(world.archetype_count(), 1);

    world.insert_component(e, Vel { dx: 3.0, dy: 4.0 }).unwrap();

    assert_eq!(world.archetype_count(), 2);
    assert_eq!(world.get::<Pos>(e), Some(&Pos { x: 1.0, y: 2.0 }));
    assert_eq!(world.get::<Vel>(e), Some(&Vel { dx: 3.0, dy: 4.0 }));
    assert_eq!(world.entity_count(), 1);
}

#[test]
fn remove_migrates_back_to_the_original_archetype() {
    let mut world = World::new();
    let e = world.spawn((Pos { x: 1.0, y: 2.0 },));
    let before = world.archetype_count();

    world.insert_component(e, Vel { dx: 3.0, dy: 4.0 }).unwrap();
    assert!(world.remove_component::<Vel>(e).unwrap());

    // Round trip reuses the archetype it started in — the (Pos,) archetype
    // is found, not recreated.
    assert_eq!(world.archetype_count(), before + 1);
    assert_eq!(world.get::<Pos>(e), Some(&Pos { x: 1.0, y: 2.0 }));
    assert_eq!(world.get::<Vel>(e), None);
}

#[test]
fn remove_reports_false_for_a_component_that_was_never_there() {
    let mut world = World::new();
    let e = world.spawn((Pos { x: 0.0, y: 0.0 },));
    assert_eq!(world.remove_component::<Vel>(e), Ok(false));
}

#[test]
fn has_component_and_component_ids_track_migration() {
    let mut world = World::new();
    let e = world.spawn((Pos { x: 0.0, y: 0.0 },));
    assert!(world.has_component(e, ComponentId::of::<Pos>()));
    assert!(!world.has_component(e, ComponentId::of::<Vel>()));

    world.insert_component(e, Vel { dx: 0.0, dy: 0.0 }).unwrap();
    let mut ids = world.component_ids(e).unwrap();
    let mut expected = vec![ComponentId::of::<Pos>(), ComponentId::of::<Vel>()];
    ids.sort_unstable();
    expected.sort_unstable();
    assert_eq!(ids, expected);

    // The list is sorted; reflection and serialization depend on that.
    let ids = world.component_ids(e).unwrap();
    assert!(ids.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn zero_sized_components_migrate() {
    let mut world = World::new();
    let e = world.spawn((Pos { x: 5.0, y: 6.0 },));
    world.insert_component(e, Tag).unwrap();
    assert!(world.has_component(e, ComponentId::of::<Tag>()));
    assert_eq!(world.get::<Pos>(e), Some(&Pos { x: 5.0, y: 6.0 }));
    assert!(world.remove_component::<Tag>(e).unwrap());
    assert_eq!(world.get::<Pos>(e), Some(&Pos { x: 5.0, y: 6.0 }));
}

// ── Row bookkeeping ────────────────────────────────────────────────────

#[test]
fn migrating_a_middle_row_patches_the_swapped_entity() {
    let mut world = World::new();
    let a = world.spawn((Pos { x: 0.0, y: 0.0 },));
    let b = world.spawn((Pos { x: 1.0, y: 1.0 },));
    let c = world.spawn((Pos { x: 2.0, y: 2.0 },));

    // `a` is row 0; migrating it swap-removes `c` into row 0.
    world.insert_component(a, Vel { dx: 9.0, dy: 9.0 }).unwrap();

    assert_eq!(world.get::<Pos>(a), Some(&Pos { x: 0.0, y: 0.0 }));
    assert_eq!(world.get::<Pos>(b), Some(&Pos { x: 1.0, y: 1.0 }));
    assert_eq!(world.get::<Pos>(c), Some(&Pos { x: 2.0, y: 2.0 }));
    assert_eq!(world.entity_count(), 3);
}

#[test]
fn migrating_the_last_row_needs_no_patch() {
    let mut world = World::new();
    let a = world.spawn((Pos { x: 0.0, y: 0.0 },));
    let b = world.spawn((Pos { x: 1.0, y: 1.0 },));

    world.insert_component(b, Vel { dx: 7.0, dy: 7.0 }).unwrap();

    assert_eq!(world.get::<Pos>(a), Some(&Pos { x: 0.0, y: 0.0 }));
    assert_eq!(world.get::<Pos>(b), Some(&Pos { x: 1.0, y: 1.0 }));
    assert_eq!(world.get::<Vel>(b), Some(&Vel { dx: 7.0, dy: 7.0 }));
}

#[test]
fn many_migrations_keep_every_entity_readable() {
    let mut world = World::new();
    let entities: Vec<_> = (0..200)
        .map(|i| {
            world.spawn((Pos {
                x: i as f32,
                y: 0.0,
            },))
        })
        .collect();

    // Migrate every third entity out, then a different third back and forth.
    for (i, &e) in entities.iter().enumerate() {
        if i % 3 == 0 {
            world.insert_component(e, Vel { dx: i as f32, dy: 0.0 }).unwrap();
        }
    }
    for (i, &e) in entities.iter().enumerate() {
        if i % 6 == 0 {
            assert!(world.remove_component::<Vel>(e).unwrap());
        }
    }

    for (i, &e) in entities.iter().enumerate() {
        assert_eq!(world.get::<Pos>(e).unwrap().x, i as f32, "entity {i}");
        let expect_vel = i % 3 == 0 && i % 6 != 0;
        assert_eq!(world.get::<Vel>(e).is_some(), expect_vel, "entity {i}");
    }
    assert_eq!(world.entity_count(), 200);
}

// ── Drop discipline ────────────────────────────────────────────────────

#[test]
fn a_moved_component_is_not_dropped() {
    let drops = counter();
    let mut world = World::new();
    let e = world.spawn((tracked("payload", &drops),));

    // Migrate the entity twice; the tracked value rides along both times.
    world.insert_component(e, Pos { x: 1.0, y: 1.0 }).unwrap();
    world.insert_component(e, Vel { dx: 1.0, dy: 1.0 }).unwrap();
    assert_eq!(drops.load(Ordering::Relaxed), 0, "migration must not drop");

    assert_eq!(world.get::<Tracked>(e).unwrap().label, "payload");

    world.despawn(e);
    assert_eq!(drops.load(Ordering::Relaxed), 1, "despawn drops exactly once");
}

#[test]
fn a_removed_component_is_dropped_exactly_once() {
    let drops = counter();
    let mut world = World::new();
    let e = world.spawn((Pos { x: 0.0, y: 0.0 }, tracked("removed", &drops)));

    assert!(world.remove_component::<Tracked>(e).unwrap());
    assert_eq!(drops.load(Ordering::Relaxed), 1);

    // The rest of the entity survived the migration.
    assert_eq!(world.get::<Pos>(e), Some(&Pos { x: 0.0, y: 0.0 }));

    world.despawn(e);
    assert_eq!(drops.load(Ordering::Relaxed), 1, "no second drop on despawn");
}

#[test]
fn replacing_a_component_drops_the_old_value_once() {
    let drops = counter();
    let mut world = World::new();
    let e = world.spawn((tracked("first", &drops),));
    assert_eq!(world.archetype_count(), 1);

    world.insert_component(e, tracked("second", &drops)).unwrap();

    // In-place replacement: no new archetype, old value dropped once.
    assert_eq!(world.archetype_count(), 1);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert_eq!(world.get::<Tracked>(e).unwrap().label, "second");

    world.despawn(e);
    assert_eq!(drops.load(Ordering::Relaxed), 2);
}

#[test]
fn dropping_the_world_drops_migrated_components_once() {
    let drops = counter();
    {
        let mut world = World::new();
        for i in 0..8 {
            let e = world.spawn((tracked(&format!("e{i}"), &drops),));
            world.insert_component(e, Pos { x: 0.0, y: 0.0 }).unwrap();
        }
        assert_eq!(drops.load(Ordering::Relaxed), 0);
    }
    assert_eq!(drops.load(Ordering::Relaxed), 8);
}

// ── Stale handles ──────────────────────────────────────────────────────

#[test]
fn insert_on_a_dead_entity_fails_and_drops_the_value() {
    let drops = counter();
    let mut world = World::new();
    let e = world.spawn((Pos { x: 0.0, y: 0.0 },));
    world.despawn(e);

    let result = world.insert_component(e, tracked("orphan", &drops));
    assert_eq!(result, Err(EcsError::DeadEntity));
    assert_eq!(
        drops.load(Ordering::Relaxed),
        1,
        "a rejected insert must not leak the value"
    );
}

#[test]
fn remove_and_queries_on_a_dead_entity_fail_cleanly() {
    let mut world = World::new();
    let e = world.spawn((Pos { x: 0.0, y: 0.0 },));
    world.despawn(e);

    assert_eq!(world.remove_component::<Pos>(e), Err(EcsError::DeadEntity));
    assert!(!world.has_component(e, ComponentId::of::<Pos>()));
    assert_eq!(world.component_ids(e), None);
}

#[test]
fn a_recycled_slot_does_not_answer_for_the_old_handle() {
    let mut world = World::new();
    let old = world.spawn((Pos { x: 1.0, y: 1.0 },));
    world.despawn(old);
    let new = world.spawn((Pos { x: 2.0, y: 2.0 },));
    assert_eq!(old.index(), new.index(), "slot should be recycled");

    assert_eq!(world.insert_component(old, Tag), Err(EcsError::DeadEntity));
    assert!(!world.has_component(old, ComponentId::of::<Tag>()));
    assert!(!world.has_component(new, ComponentId::of::<Tag>()));

    world.insert_component(new, Tag).unwrap();
    assert!(world.has_component(new, ComponentId::of::<Tag>()));
}
