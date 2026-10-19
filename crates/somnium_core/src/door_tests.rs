use super::*;
use crate::{WorldTransform, propagate_transforms};
use crate::interact::{KIND_DOOR, Used};
use somnium_physics::config::PhysicsConfig;

fn place(translation: Vec3, rotation: Quat) -> Transform {
    Transform { translation, rotation, scale: Vec3::ONE }
}

fn used(entity: Entity) -> Used {
    Used { entity, kind: KIND_DOOR, refused: false, trigger: String::new(), at: Vec3::ZERO }
}

/// The church of Aldermere as the composer lays it out: a turned root, `LOD0`, a cutaway group, and the door's two
/// material parts at identity (their vertices are in the building's frame), plus a door entity from its record.
struct Church {
    world: World,
    root: Entity,
    door: Entity,
    leaves: [Entity; 2],
    /// Distractors: the same names under another building, and a non-door part.
    other_leaf: Entity,
}

const HINGE: [f32; 3] = [-9.49, 0.01, 0.582];
const WIDTH: f32 = 1.224;

fn church() -> Church {
    let mut world = World::new();
    let root_at = place(Vec3::new(40.0, 3.0, -12.0), Quat::from_rotation_y(0.7));
    let root = world.spawn((root_at, WorldTransform(Mat4::IDENTITY), Name::new("church")));
    let lod0 = world.spawn((place(Vec3::ZERO, Quat::IDENTITY), WorldTransform(Mat4::IDENTITY), Name::new("LOD0"), Parent { entity: root }));
    let group = world.spawn((place(Vec3::ZERO, Quat::IDENTITY), WorldTransform(Mat4::IDENTITY), Name::new("church cutaway ground"), Parent { entity: lod0 }));
    let part = |world: &mut World, name: &str, parent: Entity| {
        world.spawn((place(Vec3::ZERO, Quat::IDENTITY), WorldTransform(Mat4::IDENTITY), Name::new(name), Parent { entity: parent }))
    };
    let leaves = [part(&mut world, "Door / iron", group), part(&mut world, "Door / paint_blue", lod0)];
    part(&mut world, "Walls / ashlar", lod0);
    let other = world.spawn((place(Vec3::new(80.0, 0.0, 0.0), Quat::IDENTITY), WorldTransform(Mat4::IDENTITY), Name::new("cottage")));
    let other_leaf = part(&mut world, "Door / iron", other);
    let (t, d) = door_at_hinge(Vec3::from(HINGE), std::f32::consts::FRAC_PI_2, WIDTH, 2.5948, -1.0, "Door / ");
    let door = world.spawn((t, WorldTransform(Mat4::IDENTITY), Name::new("church door"), Parent { entity: root }, d));
    propagate_transforms(&mut world);
    Church { world, root, door, leaves, other_leaf }
}

fn world_of(world: &World, e: Entity) -> Mat4 {
    world.get::<WorldTransform>(e).unwrap().0
}

/// Jolt keeps process-wide state: two physics worlds at once (tests run in parallel) abort it. Each test holds this
/// while its world lives.
static JOLT: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn physics() -> (std::sync::MutexGuard<'static, ()>, PhysicsWorld) {
    let guard = JOLT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    (guard, PhysicsWorld::new(PhysicsConfig::default()))
}

/// Run `seconds` of frames.
fn run(doors: &mut Doors, world: &mut World, physics: &mut PhysicsWorld, near: &[Vec3], seconds: f32) -> Vec<DoorEvent> {
    let mut events = Vec::new();
    for _ in 0..(seconds * 60.0).round() as usize {
        events.extend(doors.update(world, physics, None, near, 1.0 / 60.0));
        propagate_transforms(world);
    }
    events
}

#[test]
fn a_building_door_finds_its_named_parts_wherever_they_sit_and_swings_them_about_the_hinge() {
    let Church { mut world, root, door, leaves, other_leaf } = church();
    let (_jolt, mut physics) = physics();
    let mut doors = Doors::default();
    let bodies = physics.num_bodies();
    doors.update(&mut world, &mut physics, None, &[], 1.0 / 60.0);
    assert_eq!(doors.len(), 1);
    assert_eq!(physics.num_bodies(), bodies + 1, "a closed door blocks");
    // The free edge at hand height, in the building's frame: the leaf lies along -Z from the hinge (yaw 90 degrees).
    let edge = Vec3::new(HINGE[0], 1.0, HINGE[2] - WIDTH);
    let root_frame = world_of(&world, root);
    let edge_now = |world: &World, leaf: Entity| root_frame.inverse().transform_point3(world_of(world, leaf).transform_point3(edge));
    assert!(edge_now(&world, leaves[0]).abs_diff_eq(edge, 1e-4));

    let events = doors.update(&mut world, &mut physics, Some(&used(door)), &[], 1.0 / 60.0);
    assert_eq!(events, vec![DoorEvent::Opening(door)]);
    assert!(world.get::<Interactable>(door).unwrap().open, "the live state is written");
    assert_eq!(physics.num_bodies(), bodies, "the box goes as it starts to open");
    run(&mut doors, &mut world, &mut physics, &[], 1.0);
    let hinge = Vec3::from(HINGE);
    for leaf in leaves {
        let open = edge_now(&world, leaf);
        assert!(((open - hinge).length() - (edge - hinge).length()).abs() < 1e-3, "it keeps its width");
        assert!((open.y - edge.y).abs() < 1e-4, "it turns about the vertical");
        // `opens` -1 turns clockwise from above by 100 degrees: into the building (+X from the west wall).
        let turned = Quat::from_rotation_y(-100f32.to_radians()) * (edge - hinge) + hinge;
        assert!(open.abs_diff_eq(turned, 1e-3), "{open} vs {turned}");
    }
    assert_eq!(world.get::<Transform>(other_leaf).unwrap().translation, Vec3::ZERO, "another building's parts stay");

    doors.update(&mut world, &mut physics, Some(&used(door)), &[], 1.0 / 60.0);
    let events = run(&mut doors, &mut world, &mut physics, &[], 1.0);
    assert_eq!(events, vec![DoorEvent::Closed(door)]);
    assert!(edge_now(&world, leaves[0]).abs_diff_eq(edge, 1e-4), "closed is where it started");
    assert_eq!(physics.num_bodies(), bodies + 1, "closed, it blocks again");
    assert_eq!(doors.active(), 0, "nothing left to do");
}

#[test]
fn a_door_spawned_this_frame_is_placed_by_its_local_transforms() {
    let Church { mut world, root, .. } = church();
    let (t, d) = door_at_hinge(Vec3::new(1.0, 0.0, 2.0), 0.0, 1.0, 2.0, 1.0, "");
    // Not propagated: its WorldTransform is still the identity.
    let late = world.spawn((t, WorldTransform(Mat4::IDENTITY), Parent { entity: root }, d));
    let ((_jolt, mut physics), mut doors) = (physics(), Doors::default());
    doors.update(&mut world, &mut physics, None, &[], 1.0 / 60.0);
    let expected = world_of(&world, root).transform_point3(Vec3::new(1.5, 1.0, 2.0));
    let at = doors.pass(&mut world, &mut physics, expected);
    assert_eq!(at, Some(late), "its doorway is where the building puts it, not at the origin");
}

#[test]
fn a_door_swings_its_own_children() {
    let mut world = World::new();
    let (t, d) = door_at_hinge(Vec3::new(2.0, 0.0, 0.0), 0.0, 1.0, 2.0, 1.0, "");
    let door = world.spawn((t, WorldTransform(Mat4::IDENTITY), d));
    let leaf = world.spawn((place(Vec3::new(0.5, 1.0, 0.0), Quat::IDENTITY), WorldTransform(Mat4::IDENTITY), Parent { entity: door }));
    propagate_transforms(&mut world);
    let ((_jolt, mut physics), mut doors) = (physics(), Doors::default());
    doors.update(&mut world, &mut physics, Some(&used(door)), &[], 1.0 / 60.0);
    // A door that arrives this frame is bound before the key is read, so the use lands.
    assert!(world.get::<Interactable>(door).unwrap().open);
    run(&mut doors, &mut world, &mut physics, &[], 1.0);
    let centre = world_of(&world, leaf).transform_point3(Vec3::ZERO);
    let expected = Vec3::new(2.0, 1.0, 0.0) + Quat::from_rotation_y(100f32.to_radians()) * Vec3::new(0.5, 0.0, 0.0);
    assert!(centre.abs_diff_eq(expected, 1e-3), "{centre} vs {expected}");
}

#[test]
fn a_door_that_closes_by_itself_waits_until_nobody_is_in_its_way() {
    let mut world = World::new();
    let (t, mut d) = door_at_hinge(Vec3::ZERO, 0.0, 1.0, 2.0, 1.0, "");
    d.close_after = 2.0;
    let door = world.spawn((t, WorldTransform(Mat4::IDENTITY), d));
    propagate_transforms(&mut world);
    let ((_jolt, mut physics), mut doors) = (physics(), Doors::default());
    doors.update(&mut world, &mut physics, Some(&used(door)), &[], 1.0 / 60.0);
    let standing = [Vec3::new(0.5, 0.0, 0.3)];
    let events = run(&mut doors, &mut world, &mut physics, &standing, 4.0);
    assert!(!events.contains(&DoorEvent::Closing(door)), "not on someone in the doorway");
    let events = run(&mut doors, &mut world, &mut physics, &[Vec3::new(6.0, 0.0, 0.0)], 0.2);
    assert_eq!(events, vec![DoorEvent::Closing(door)], "once they have gone, after its time");
    let events = run(&mut doors, &mut world, &mut physics, &[], 1.0);
    assert_eq!(events, vec![DoorEvent::Closed(door)]);
    assert!(!world.get::<Interactable>(door).unwrap().open);
}

#[test]
fn a_walker_opens_the_door_it_passes_and_it_closes_behind_them_locked_or_not() {
    let Church { mut world, door, .. } = church();
    world.get_mut::<Interactable>(door).unwrap().requires = "church_key".into();
    let ((_jolt, mut physics), mut doors) = (physics(), Doors::default());
    doors.update(&mut world, &mut physics, None, &[], 1.0 / 60.0);
    let doorway = world_of(&world, door).transform_point3(Vec3::new(0.5 * WIDTH, 1.0, 0.0));
    assert_eq!(doors.pass(&mut world, &mut physics, doorway + Vec3::new(5.0, 0.0, 0.0)), None, "too far");
    assert_eq!(doors.pass(&mut world, &mut physics, doorway + Vec3::new(0.6, 3.0, 0.0)), None, "a storey up");
    // The life sim's door point: on the ground, 0.6 m out from the front's middle, which at a cottage is 0.73 m
    // aside from the leaf's middle (rb-combat, 19 Oct).
    let root = world.get::<Parent>(door).unwrap().entity;
    let out = world_of(&world, root).transform_vector3(Vec3::new(-0.6, 0.0, 0.73));
    assert_eq!(doors.pass(&mut world, &mut physics, doorway + out - Vec3::Y), Some(door));
    assert!(world.get::<Interactable>(door).unwrap().open);
    let events = run(&mut doors, &mut world, &mut physics, &[], 1.0);
    assert_eq!(events, vec![DoorEvent::Opening(door)], "still open for them");
    let events = run(&mut doors, &mut world, &mut physics, &[], 3.0);
    assert_eq!(events, vec![DoorEvent::Closing(door), DoorEvent::Closed(door)]);
}

#[test]
fn a_walker_can_ask_through_the_world() {
    let Church { mut world, door, .. } = church();
    let ((_jolt, mut physics), mut doors) = (physics(), Doors::default());
    doors.update(&mut world, &mut physics, None, &[], 1.0 / 60.0);
    let doorway = world_of(&world, door).transform_point3(Vec3::new(0.5 * WIDTH, 1.0, 0.0));
    request_pass(&mut world, doorway + Vec3::new(0.0, 0.0, 0.5));
    let events = doors.update(&mut world, &mut physics, None, &[], 1.0 / 60.0);
    assert_eq!(events, vec![DoorEvent::Opening(door)], "it creaks for them as for the player");
    assert!(world.get::<Interactable>(door).unwrap().open);
    // Asked once, answered once.
    run(&mut doors, &mut world, &mut physics, &[], 4.0);
    assert!(!world.get::<Interactable>(door).unwrap().open);
}

#[test]
fn a_refused_use_says_so_and_leaves_the_door_shut() {
    let Church { mut world, door, .. } = church();
    let ((_jolt, mut physics), mut doors) = (physics(), Doors::default());
    let refused = Used { refused: true, ..used(door) };
    let events = doors.update(&mut world, &mut physics, Some(&refused), &[], 1.0 / 60.0);
    assert_eq!(events, vec![DoorEvent::Refused(door)]);
    assert!(!world.get::<Interactable>(door).unwrap().open);
}

#[test]
fn setting_open_from_outside_swings_it_and_an_authored_open_door_starts_open() {
    let mut world = World::new();
    let (t, mut d) = door_at_hinge(Vec3::ZERO, 0.0, 1.0, 2.0, 1.0, "");
    let shut = world.spawn((t, WorldTransform(Mat4::IDENTITY), d.clone()));
    d.open = true;
    let ajar = world.spawn((t, WorldTransform(Mat4::IDENTITY), d));
    let leaf = world.spawn((place(Vec3::new(0.5, 0.0, 0.0), Quat::IDENTITY), WorldTransform(Mat4::IDENTITY), Parent { entity: ajar }));
    propagate_transforms(&mut world);
    let ((_jolt, mut physics), mut doors) = (physics(), Doors::default());
    let bodies = physics.num_bodies();
    doors.update(&mut world, &mut physics, None, &[], 1.0 / 60.0);
    assert_eq!(physics.num_bodies(), bodies + 1, "only the shut one blocks");
    let turned = Quat::from_rotation_y(100f32.to_radians()) * Vec3::new(0.5, 0.0, 0.0);
    assert!(world.get::<Transform>(leaf).unwrap().translation.abs_diff_eq(turned, 1e-4), "open from the start");
    world.get_mut::<Interactable>(shut).unwrap().open = true;
    let events = run(&mut doors, &mut world, &mut physics, &[], 0.1);
    assert_eq!(events, vec![DoorEvent::Opening(shut)]);
    assert_eq!(physics.num_bodies(), bodies);
}

#[test]
fn a_door_that_goes_takes_its_box_and_stop_takes_the_rest() {
    let Church { mut world, door, .. } = church();
    let ((_jolt, mut physics), mut doors) = (physics(), Doors::default());
    let bodies = physics.num_bodies();
    let (t, d) = door_at_hinge(Vec3::new(5.0, 0.0, 0.0), 0.0, 1.0, 2.0, 1.0, "");
    let second = world.spawn((t, WorldTransform(Mat4::IDENTITY), d));
    propagate_transforms(&mut world);
    doors.update(&mut world, &mut physics, None, &[], 1.0 / 60.0);
    assert_eq!((doors.len(), physics.num_bodies()), (2, bodies + 2));
    world.despawn(door);
    doors.update(&mut world, &mut physics, None, &[], 1.0 / 60.0);
    assert_eq!((doors.len(), physics.num_bodies()), (1, bodies + 1), "a streamed-out door's box goes with it");
    assert!(world.is_alive(second));
    doors.stop(&mut physics);
    assert_eq!((doors.len(), physics.num_bodies()), (0, bodies));
}

#[test]
fn idle_doors_are_not_looked_at() {
    let Church { mut world, door, .. } = church();
    let ((_jolt, mut physics), mut doors) = (physics(), Doors::default());
    doors.update(&mut world, &mut physics, None, &[], 1.0 / 60.0);
    let ticks = |world: &World| (world.change_tick(ComponentId::of::<Transform>()), world.change_tick(ComponentId::of::<Interactable>()));
    let before = ticks(&world);
    run(&mut doors, &mut world, &mut physics, &[Vec3::ZERO], 2.0);
    assert_eq!(ticks(&world), before, "nothing is written while every door rests");
    assert_eq!(doors.active(), 0);
    assert!(world.is_alive(door));
}

/// Hundreds of doors in a town cell, idle: what a frame costs (release, `--ignored --nocapture`).
#[test]
#[ignore = "measure: cargo test --release -p somnium_core door::tests::measure_idle_doors -- --ignored --nocapture"]
fn measure_idle_doors() {
    use crate::interact::{Actor, Interactor};
    let mut world = World::new();
    let (_jolt, mut physics) = physics();
    // 400 buildings of 12 parts each; one door a building, its two leaves named under LOD0.
    for b in 0..400 {
        let at = Vec3::new((b % 20) as f32 * 14.0, 0.0, (b / 20) as f32 * 14.0);
        let root = world.spawn((place(at, Quat::IDENTITY), WorldTransform(Mat4::IDENTITY), Name::new("house")));
        let lod0 = world.spawn((place(Vec3::ZERO, Quat::IDENTITY), WorldTransform(Mat4::IDENTITY), Name::new("LOD0"), Parent { entity: root }));
        for k in 0..10 {
            world.spawn((place(Vec3::ZERO, Quat::IDENTITY), WorldTransform(Mat4::IDENTITY), Name::new(&format!("Walls / {k}")), Parent { entity: lod0 }));
        }
        for m in ["Door / boards", "Door / iron"] {
            world.spawn((place(Vec3::ZERO, Quat::IDENTITY), WorldTransform(Mat4::IDENTITY), Name::new(m), Parent { entity: lod0 }));
        }
        let (t, d) = door_at_hinge(Vec3::new(0.3, 0.0, -3.0), 0.0, 0.9, 1.9, -1.0, "Door / ");
        world.spawn((t, WorldTransform(Mat4::IDENTITY), Name::new("front door"), Parent { entity: root }, d));
    }
    propagate_transforms(&mut world);
    let mut doors = Doors::default();
    let bind = std::time::Instant::now();
    doors.update(&mut world, &mut physics, None, &[], 1.0 / 60.0);
    let bind = bind.elapsed();
    assert_eq!(doors.len(), 400);
    let mut interactor = Interactor::default();
    // In the middle of the town, beside a door, so the pick has candidates.
    let actor = Actor { eye: Vec3::new(140.75, 1.6, 137.5), forward: Vec3::NEG_Z };
    let frames = 5000;
    let start = std::time::Instant::now();
    for _ in 0..frames {
        doors.update(&mut world, &mut physics, None, &[actor.eye], 1.0 / 60.0);
    }
    let idle = start.elapsed() / frames;
    let start = std::time::Instant::now();
    let mut picked = 0;
    for _ in 0..frames {
        interactor.update(&world, Some(actor), &|_| true, 1.0 / 60.0);
        picked += usize::from(interactor.target().is_some());
    }
    let pick = start.elapsed() / frames;
    assert_eq!(picked, frames as usize, "beside a door, it is picked");
    println!(
        "400 doors, {} entities: bind {bind:?}; idle doors {idle:?} a frame; pick among 400 interactables {pick:?} a frame",
        world.entity_count()
    );
}
