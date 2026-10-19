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
    assert_eq!(used, Used { entity: chest, kind: 1, refused: false, trigger: "chest_01".into() });
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
