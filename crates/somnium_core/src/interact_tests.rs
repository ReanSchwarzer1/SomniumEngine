use super::*;
use crate::{KeyCode, Transform, propagate_transforms};
use glam::Quat;

fn spawn_thing(world: &mut World, at: Vec3, thing: Interactable) -> Entity {
    let t = Transform { translation: at, rotation: Quat::IDENTITY, scale: Vec3::ONE };
    world.spawn((t, WorldTransform(Mat4::IDENTITY), thing))
}

fn press(interactor: &mut Interactor, key: KeyCode, down: bool) {
    let state = if down { InputState::Pressed } else { InputState::Released };
    interactor.on_event(&EngineEvent::KeyInput { key, state });
}

/// Frames until the use commits (or a second passes).
fn until_used(interactor: &mut Interactor, world: &World, actor: Actor, allowed: &dyn Fn(&str) -> bool) -> Option<(Used, f32)> {
    let mut t = 0.0;
    while t < 1.0 {
        t += 1.0 / 60.0;
        if let Some(used) = interactor.update(world, Some(actor), allowed, 1.0 / 60.0) {
            return Some((used, t));
        }
    }
    None
}

/// Eye at 1 m, facing +Z.
const ACTOR: Actor = Actor { eye: Vec3::new(0.0, 1.0, 0.0), forward: Vec3::Z };

#[test]
fn the_pick_is_the_nearest_in_front_within_reach_of_the_eye() {
    let mut world = World::new();
    let ahead = spawn_thing(&mut world, Vec3::new(0.0, 1.0, 1.5), Interactable::default());
    let nearer_behind = spawn_thing(&mut world, Vec3::new(0.0, 1.0, -0.8), Interactable::default());
    let too_far = spawn_thing(&mut world, Vec3::new(0.0, 1.0, 2.2), Interactable::default());
    let upstairs = spawn_thing(&mut world, Vec3::new(0.3, 3.5, 0.6), Interactable::default());
    propagate_transforms(&mut world);
    assert_eq!(pick(&world, ACTOR, None).map(|p| p.0), Some(ahead));
    let turned = Actor { forward: Vec3::NEG_Z, ..ACTOR };
    assert_eq!(pick(&world, turned, None).map(|p| p.0), Some(nearer_behind));
    assert_eq!(pick(&world, Actor { forward: Vec3::ZERO, ..ACTOR }, None).map(|p| p.0), Some(nearer_behind));
    world.get_mut::<Interactable>(ahead).unwrap().enabled = false;
    assert_eq!(pick(&world, ACTOR, None), None, "{too_far:?} is out of reach, {upstairs:?} a floor up");
}

#[test]
fn standing_in_a_doorway_the_facing_no_longer_matters() {
    let mut world = World::new();
    let beside = spawn_thing(&mut world, Vec3::new(0.3, 1.0, -0.1), Interactable::default());
    propagate_transforms(&mut world);
    assert_eq!(pick(&world, ACTOR, None).map(|p| p.0), Some(beside));
}

#[test]
fn the_target_holds_until_another_is_clearly_better() {
    let mut world = World::new();
    let left = spawn_thing(&mut world, Vec3::new(-0.1, 1.0, 1.0), Interactable::default());
    let right = spawn_thing(&mut world, Vec3::new(0.1, 1.0, 1.0), Interactable::default());
    propagate_transforms(&mut world);
    assert_eq!(pick(&world, ACTOR, Some(left)).map(|p| p.0), Some(left));
    assert_eq!(pick(&world, ACTOR, Some(right)).map(|p| p.0), Some(right));
    let walked = Actor { eye: Vec3::new(0.0, 1.0, 0.9), ..ACTOR };
    world.get_mut::<Transform>(left).unwrap().translation = Vec3::new(-0.1, 1.0, 2.6);
    propagate_transforms(&mut world);
    assert_eq!(pick(&world, walked, Some(left)).map(|p| p.0), Some(right), "the old one is now far behind the new");
}

#[test]
fn a_use_commits_at_the_sessions_contact_and_hands_back_the_trigger() {
    let mut world = World::new();
    let chest = spawn_thing(
        &mut world,
        Vec3::new(0.0, 0.5, 1.0),
        Interactable { kind: 1, prompt: String::new(), trigger: "chest_01".into(), ..Default::default() },
    );
    propagate_transforms(&mut world);
    let mut interactor = Interactor::default();
    assert_eq!(interactor.key_label(), "E");
    assert_eq!(interactor.update(&world, Some(ACTOR), &|_| true, 0.016), None, "no key, no use");
    assert_eq!(interactor.target().map(|t| t.prompt.as_str()), Some("Take"), "an empty prompt says the kind");
    press(&mut interactor, KeyCode::KeyE, true);
    let (used, after) = until_used(&mut interactor, &world, ACTOR, &|_| true).expect("it commits");
    assert_eq!(used, Used { entity: chest, kind: 1, refused: false, trigger: "chest_01".into(), at: Vec3::new(0.0, 0.5, 1.0) });
    // The reach takes half the interactable's duration (0.65 s by default) before the commit.
    assert!((after - 0.325).abs() < 0.03, "{after}");
    assert!(interactor.session().0.busy(), "the hand is on its way back");
    // Held, it does not use again.
    assert_eq!(until_used(&mut interactor, &world, ACTOR, &|_| true), None);
}

#[test]
fn locked_or_a_false_condition_refuses_at_once() {
    let mut world = World::new();
    let gate = spawn_thing(&mut world, Vec3::new(0.0, 1.0, 1.0), Interactable { requires: "has_key".into(), ..Default::default() });
    propagate_transforms(&mut world);
    let mut interactor = Interactor::default();
    let no_key = |_: &str| false;
    interactor.update(&world, Some(ACTOR), &no_key, 0.016);
    let target = interactor.target().unwrap();
    assert_eq!((target.entity, target.prompt.as_str(), target.refused), (gate, "Locked", true));
    press(&mut interactor, KeyCode::KeyE, true);
    let used = interactor.update(&world, Some(ACTOR), &no_key, 0.016).unwrap();
    assert!(used.refused, "refused on the press, no reach");
    assert!(!interactor.session().0.busy());
    // With the key in hand it is the interactable's own prompt again; `locked` refuses whatever the condition.
    interactor.update(&world, Some(ACTOR), &|c| c == "has_key", 0.016);
    assert_eq!(interactor.target().unwrap().prompt, "Operate");
    world.get_mut::<Interactable>(gate).unwrap().locked = true;
    interactor.update(&world, Some(ACTOR), &|_| true, 0.016);
    assert!(interactor.target().unwrap().refused);
}

#[test]
fn walking_off_mid_reach_lets_go_without_committing() {
    let mut world = World::new();
    spawn_thing(&mut world, Vec3::new(0.0, 1.0, 1.0), Interactable::default());
    propagate_transforms(&mut world);
    let mut interactor = Interactor::default();
    press(&mut interactor, KeyCode::KeyE, true);
    interactor.update(&world, Some(ACTOR), &|_| true, 0.016);
    assert!(interactor.session().0.busy());
    // The player stops being an actor (a dialogue opened, Play stopped): the reach is cancelled.
    let mut used = None;
    for _ in 0..90 {
        used = used.or(interactor.update(&world, None, &|_| true, 1.0 / 60.0));
    }
    assert_eq!(used, None);
    assert!(!interactor.session().0.busy());
}

#[test]
fn an_open_door_offers_to_close() {
    let mut world = World::new();
    let (t, d) = crate::door::door_at_hinge(Vec3::new(-0.5, 0.0, 1.0), 0.0, 1.0, 2.0, 1.0, "");
    let door = world.spawn((t, WorldTransform(Mat4::IDENTITY), d));
    propagate_transforms(&mut world);
    let mut interactor = Interactor::default();
    interactor.update(&world, Some(ACTOR), &|_| true, 0.016);
    assert_eq!(interactor.target().map(|t| t.prompt.as_str()), Some("Open"));
    world.get_mut::<Interactable>(door).unwrap().open = true;
    interactor.update(&world, Some(ACTOR), &|_| true, 0.016);
    assert_eq!(interactor.target().map(|t| t.prompt.as_str()), Some("Close"));
}

#[test]
fn the_key_rebinds() {
    let mut world = World::new();
    spawn_thing(&mut world, Vec3::new(0.0, 1.0, 1.0), Interactable::default());
    propagate_transforms(&mut world);
    let mut interactor = Interactor::default();
    somnium_input::rebind::rebind(interactor.input_mut().maps_mut(), MAP, ACTION, ControlPath::keyboard("f")).unwrap();
    assert_eq!(interactor.key_label(), "F");
    press(&mut interactor, KeyCode::KeyE, true);
    assert_eq!(until_used(&mut interactor, &world, ACTOR, &|_| true), None, "E no longer uses");
    press(&mut interactor, KeyCode::KeyF, true);
    assert!(until_used(&mut interactor, &world, ACTOR, &|_| true).is_some(), "F does");
}

#[test]
fn tsf_defaults_are_unchanged_by_the_new_fields() {
    // TSF's hand interactions read these; the appended fields must not move them.
    let d = Interactable::default();
    assert_eq!((d.kind, d.prompt.as_str(), d.enabled, d.locked), (3, "Operate", true, false));
    assert_eq!((d.reach, d.duration, d.open_angle, d.hand_grip), (2.0, 0.65, 90.0, -1.0));
    assert_eq!((d.requires.as_str(), d.open), ("", false));
}

// ── The verbs beyond doors ─────────────────────────────────────────────────────────────────────────────────────────

use crate::door::tests::physics;
use crate::story_actor::CastMemberComponent;
use crate::{Name, Parent};

fn cast(world: &mut World, at: Vec3, speaker: &str) -> Entity {
    let t = Transform { translation: at, rotation: Quat::IDENTITY, scale: Vec3::ONE };
    world.spawn((t, WorldTransform(Mat4::IDENTITY), CastMemberComponent { speaker: speaker.into(), ..Default::default() }))
}

/// The player's eye 1.6 m above the feet at the origin, facing +Z.
const STANDING: Actor = Actor { eye: Vec3::new(0.0, 1.6, 0.0), forward: Vec3::Z };

#[test]
fn a_cast_member_is_talked_to_by_their_speaker_id_and_their_own_interactable_wins() {
    let mut world = World::new();
    let seren = cast(&mut world, Vec3::new(0.0, 0.0, 2.0), "seren");
    cast(&mut world, Vec3::new(0.3, 0.0, 1.0), ""); // no speaker: nobody to talk to
    propagate_transforms(&mut world);
    let mut interactor = Interactor::default();
    interactor.update(&world, Some(STANDING), &|_| true, 0.016);
    let target = interactor.target().unwrap();
    assert_eq!((target.entity, target.kind, target.prompt.as_str()), (seren, KIND_TALK, "Talk"));
    assert!((target.at - Vec3::new(0.0, CAST_HEIGHT, 2.0)).length() < 1e-5, "at the chest");
    press(&mut interactor, KeyCode::KeyE, true);
    let (used, after) = until_used(&mut interactor, &world, STANDING, &|_| true).expect("it commits");
    assert_eq!((used.entity, used.kind, used.trigger.as_str()), (seren, KIND_TALK, "seren"));
    assert!(after < 0.2, "talking starts at once: {after}");
    // Their own interactable (a shopkeeper at a counter, say) is what they are used as.
    world.insert_component(seren, Interactable { prompt: "Barter".into(), ..interactable(KIND_TALK, "seren_shop") }).unwrap();
    let mut interactor = Interactor::default();
    interactor.update(&world, Some(STANDING), &|_| true, 0.016);
    assert_eq!(interactor.target().map(|t| t.prompt.as_str()), Some("Barter"));
}

#[test]
fn the_verbs_say_themselves_and_a_refusal_can_say_why() {
    for (kind, word) in [(KIND_COLLECT, "Take"), (KIND_INSPECT, "Read"), (KIND_OPERATE, "Use"), (KIND_TALK, "Talk"), (KIND_SIT, "Sit")] {
        let mut world = World::new();
        spawn_thing(&mut world, Vec3::new(0.0, 1.0, 1.0), interactable(kind, "x"));
        propagate_transforms(&mut world);
        let mut interactor = Interactor::default();
        interactor.update(&world, Some(STANDING), &|_| true, 0.016);
        assert_eq!(interactor.target().map(|t| t.prompt.as_str()), Some(word));
        // Every kind begins a session (5 and 6 are new to it).
        press(&mut interactor, KeyCode::KeyE, true);
        assert!(until_used(&mut interactor, &world, STANDING, &|_| true).is_some(), "{word} commits");
    }
    let mut world = World::new();
    let barred =
        Interactable { requires: "day".into(), refused_prompt: "Barred for the night".into(), ..interactable(KIND_OPERATE, "gate") };
    spawn_thing(&mut world, Vec3::new(0.0, 1.0, 1.0), barred);
    propagate_transforms(&mut world);
    let mut interactor = Interactor::default();
    interactor.update(&world, Some(STANDING), &|_| false, 0.016);
    assert_eq!(interactor.target().map(|t| (t.prompt.as_str(), t.refused)), Some(("Barred for the night", true)));
}

#[test]
fn a_seat_puts_the_sitter_at_its_anchor_facing_its_front() {
    let mut world = World::new();
    let bench =
        Transform { translation: Vec3::new(4.0, 0.0, 2.0), rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), scale: Vec3::ONE };
    let seat_at = Interactable { anchor: Vec3::new(0.0, 0.45, 0.1), ..interactable(KIND_SIT, "bench") };
    let e = world.spawn((bench, WorldTransform(Mat4::IDENTITY), seat_at));
    propagate_transforms(&mut world);
    let (hips, facing) = seat(&world, e).unwrap();
    // A quarter turn about +Y takes the bench's +Z to the world's +X.
    assert!((hips - Vec3::new(4.1, 0.45, 2.0)).length() < 1e-5, "{hips}");
    assert!((facing - Vec3::X).length() < 1e-5, "{facing}");
}

/// A frame of play with the player at `actor`.
fn frame(
    interaction: &mut Interaction,
    world: &mut World,
    physics: &mut somnium_physics::world::PhysicsWorld,
    actor: Actor,
    allowed: &dyn Fn(&str) -> bool,
) -> Option<Used> {
    let used = interaction.play(world, physics, Some(actor), allowed, 1.0 / 60.0);
    propagate_transforms(world);
    used
}

#[test]
fn a_take_removes_the_thing_and_what_is_under_it_once_the_game_has_seen_it() {
    let mut world = World::new();
    let purse = spawn_thing(&mut world, Vec3::new(0.0, 0.5, 1.0), interactable(KIND_COLLECT, "purse_01"));
    let coins = world.spawn((Transform::default(), WorldTransform(Mat4::IDENTITY), Parent { entity: purse }, Name::new("coins")));
    let stays = spawn_thing(&mut world, Vec3::new(5.0, 0.5, 1.0), interactable(KIND_COLLECT, "elsewhere"));
    propagate_transforms(&mut world);
    let (_jolt, mut physics) = physics();
    let mut interaction = Interaction::default();
    frame(&mut interaction, &mut world, &mut physics, STANDING, &|_| true);
    press(&mut interaction.interactor, KeyCode::KeyE, true);
    let mut used = None;
    for _ in 0..60 {
        used = frame(&mut interaction, &mut world, &mut physics, STANDING, &|_| true);
        if used.is_some() {
            break;
        }
    }
    let used = used.expect("taken");
    assert_eq!((used.kind, used.trigger.as_str()), (KIND_COLLECT, "purse_01"));
    assert!(world.is_alive(purse), "the game can still read what it took this frame");
    frame(&mut interaction, &mut world, &mut physics, STANDING, &|_| true);
    assert!(!world.is_alive(purse) && !world.is_alive(coins), "gone with what was under it");
    assert!(world.is_alive(stays));
}

#[test]
fn a_thing_already_taken_is_gone_as_it_appears() {
    let (_jolt, mut physics) = physics();
    let mut world = World::new();
    let mut interaction = Interaction::default();
    frame(&mut interaction, &mut world, &mut physics, STANDING, &|_| true);
    // A cell streams in with two purses, one of them taken on an earlier visit.
    let taken = Interactable { gone_when: "taken.purse_01".into(), ..interactable(KIND_COLLECT, "purse_01") };
    let kept = Interactable { gone_when: "taken.purse_02".into(), ..interactable(KIND_COLLECT, "purse_02") };
    let a = spawn_thing(&mut world, Vec3::new(10.0, 0.0, 0.0), taken);
    let b = spawn_thing(&mut world, Vec3::new(11.0, 0.0, 0.0), kept);
    let facts = |c: &str| c == "taken.purse_01";
    frame(&mut interaction, &mut world, &mut physics, STANDING, &facts);
    assert!(!world.is_alive(a), "taken before: gone at once");
    assert!(world.is_alive(b));
}

#[test]
fn a_lever_swings_like_a_door_and_blocks_nothing_and_a_well_does_not_swing() {
    let mut world = World::new();
    let lever = Interactable { swings: true, open_angle: -60.0, ..interactable(KIND_OPERATE, "sluice") };
    let base = spawn_thing(&mut world, Vec3::new(0.0, 1.0, 1.0), lever);
    let handle = world.spawn((Transform::from_translation(Vec3::Y * 0.5), WorldTransform(Mat4::IDENTITY), Parent { entity: base }));
    let well = spawn_thing(&mut world, Vec3::new(3.0, 1.0, 1.0), interactable(KIND_OPERATE, "well"));
    let bucket = world.spawn((Transform::from_translation(Vec3::Y * 0.5), WorldTransform(Mat4::IDENTITY), Parent { entity: well }));
    propagate_transforms(&mut world);
    let (_jolt, mut physics) = physics();
    let bodies = physics.num_bodies();
    let mut interaction = Interaction::default();
    frame(&mut interaction, &mut world, &mut physics, STANDING, &|_| true);
    assert_eq!(interaction.doors.len(), 1, "the lever swings; the well does not");
    assert_eq!(physics.num_bodies(), bodies, "and it blocks nothing");
    press(&mut interaction.interactor, KeyCode::KeyE, true);
    for _ in 0..120 {
        frame(&mut interaction, &mut world, &mut physics, STANDING, &|_| true);
    }
    let tip = world.get::<Transform>(handle).unwrap().translation;
    let turned = Quat::from_rotation_y((-60f32).to_radians()) * (Vec3::Y * 0.5);
    assert!((tip - turned).length() < 1e-4, "about its own vertical: {tip}");
    assert!(world.get::<Interactable>(base).unwrap().open, "pulled");
    assert_eq!(world.get::<Transform>(bucket).unwrap().translation, Vec3::Y * 0.5);
}

/// A city's density: `buildings` each with a door (two named leaves) and five other interactables (a stall, two
/// pick-ups, a sign, a seat), and `people` cast members, on a 14 m grid of plots.
fn city(world: &mut World, buildings: usize, people: usize, first: usize) {
    let plot = |b: usize| Vec3::new((b % 100) as f32 * 14.0, 0.0, (b / 100) as f32 * 14.0);
    for b in first..first + buildings {
        let at = plot(b);
        let root = world.spawn((Transform::from_translation(at), WorldTransform(Mat4::from_translation(at)), Name::new("house")));
        for m in ["Door / boards", "Door / iron"] {
            world.spawn((Transform::default(), WorldTransform(Mat4::IDENTITY), Name::new(m), Parent { entity: root }));
        }
        let (t, d) = crate::door::door_at_hinge(Vec3::new(0.3, 0.0, -3.0), 0.0, 0.9, 1.9, -1.0, "Door / ");
        world.spawn((t, WorldTransform(Mat4::IDENTITY), Parent { entity: root }, d));
        for (k, kind) in [KIND_OPERATE, KIND_COLLECT, KIND_COLLECT, KIND_INSPECT, KIND_SIT].into_iter().enumerate() {
            let p = Vec3::new(-4.0 + k as f32 * 1.5, 0.0, -5.0);
            world.spawn((Transform::from_translation(p), WorldTransform(Mat4::IDENTITY), Parent { entity: root }, interactable(kind, "x")));
        }
    }
    for p in 0..people {
        let at = plot(first + p * buildings.max(1) / people.max(1)) + Vec3::new(2.0, 0.0, -7.0);
        cast(world, at, "townsfolk");
    }
    propagate_transforms(world);
}

/// What the layer costs a frame as a city fills (release, `--ignored --nocapture`): it should stay flat.
#[test]
#[ignore = "measure: cargo test --release -p somnium_core interact::tests::measure_city_density -- --ignored --nocapture"]
fn measure_city_density() {
    let (_jolt, mut physics) = physics();
    for (buildings, people) in [(400, 50), (4000, 500)] {
        let mut world = World::new();
        city(&mut world, buildings, people, 0);
        let mut interaction = Interaction::default();
        // Beside the door of a building in the middle.
        let b = buildings / 2;
        let eye = Vec3::new((b % 100) as f32 * 14.0 + 0.75, 1.6, (b / 100) as f32 * 14.0 - 2.5);
        let actor = Actor { eye, forward: Vec3::NEG_Z };
        let start = std::time::Instant::now();
        interaction.play(&mut world, &mut physics, Some(actor), &|_| true, 1.0 / 60.0);
        let first = start.elapsed();
        for _ in 0..10 {
            interaction.play(&mut world, &mut physics, Some(actor), &|_| true, 1.0 / 60.0);
        }
        let frames = 2000u32;
        let start = std::time::Instant::now();
        for _ in 0..frames {
            interaction.play(&mut world, &mut physics, Some(actor), &|_| true, 1.0 / 60.0);
        }
        let idle = start.elapsed() / frames;
        assert!(interaction.interactor.target().is_some(), "beside a door, something is picked");
        // Twenty walkers going in and out of their own doors somewhere in the city, every frame.
        let start = std::time::Instant::now();
        for _ in 0..frames {
            for w in 0..20usize {
                let b = (w * 7919) % buildings;
                let door = Vec3::new((b % 100) as f32 * 14.0 + 0.75, 1.0, (b / 100) as f32 * 14.0 - 2.4);
                crate::door::request_pass(&mut world, door);
            }
            interaction.play(&mut world, &mut physics, Some(actor), &|_| true, 1.0 / 60.0);
        }
        let walkers = start.elapsed() / frames;
        // A cell streams in: fifty more buildings.
        city(&mut world, 50, 5, buildings);
        let start = std::time::Instant::now();
        interaction.play(&mut world, &mut physics, Some(actor), &|_| true, 1.0 / 60.0);
        let install = start.elapsed();
        println!(
            "{} interactables, {people} cast, {} entities: first frame {first:?}; a frame {idle:?}; with 20 walkers' passes {walkers:?}; a 50-building cell arriving {install:?}",
            buildings * 6,
            world.entity_count()
        );
    }
}


#[test]
fn the_filing_follows_cells_coming_and_going_and_cast_members_walking() {
    let mut world = World::new();
    let mut interactor = Interactor::default();
    interactor.update(&world, Some(STANDING), &|_| true, 0.016);
    assert_eq!(interactor.target(), None);
    // A cell streams in with a sign in front of the player; it is found, unpropagated as it is.
    let t = Transform::from_translation(Vec3::new(0.0, 1.5, 1.0));
    let sign = world.spawn((t, WorldTransform(Mat4::IDENTITY), interactable(KIND_INSPECT, "notice")));
    interactor.update(&world, Some(STANDING), &|_| true, 0.016);
    assert_eq!(interactor.target().map(|t| t.entity), Some(sign));
    // The cell goes; the sign is never picked again, even as its index is reused far away.
    world.despawn(sign);
    let elsewhere = world.spawn((Transform::from_translation(Vec3::new(500.0, 0.0, 0.0)), WorldTransform(Mat4::IDENTITY), interactable(KIND_INSPECT, "far")));
    interactor.update(&world, Some(STANDING), &|_| true, 0.016);
    assert_eq!(interactor.target(), None, "{elsewhere:?} is half a kilometre off");
    // A cast member walks up to the player: they are looked at where they stand now.
    let walker = cast(&mut world, Vec3::new(0.0, 0.0, 30.0), "pilgrim");
    propagate_transforms(&mut world);
    interactor.update(&world, Some(STANDING), &|_| true, 0.016);
    assert_eq!(interactor.target(), None);
    world.get_mut::<Transform>(walker).unwrap().translation = Vec3::new(0.0, 0.0, 1.5);
    propagate_transforms(&mut world);
    interactor.update(&world, Some(STANDING), &|_| true, 0.016);
    assert_eq!(interactor.target().map(|t| t.entity), Some(walker));
}
