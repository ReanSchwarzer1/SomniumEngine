//! A person with a day of his own (the dual protagonists' off-screen man,
//! DUAL_PROTAGONIST_ENGINE_SPECS DP-4): a villager of whichever adopted place
//! his schedule has him in, drawn as himself; on the road between places; let
//! go when the player takes him and taking up his day when left; the story's
//! block over his schedule; sheltering in place; time skipped; never asleep.
use glam::{Vec2, Vec3};
use somnium_life::{
    Activity, AgentId, Clock, Flat, LifeTable, LifeWorld, LodConfig, Place, Schedule,
    ScheduleBlock, Spot, SpotKind, SpotRef, TablePlace, Tier,
};

const B: Vec3 = Vec3::new(500.0, 0.0, 0.0);

fn town(name: &str, at: Vec3) -> Place {
    let p = |x: f32, z: f32| at + Vec3::new(x, 0.0, z);
    Place {
        name: name.into(),
        spots: vec![
            Spot::new(SpotKind::Home, p(-20.0, -4.0)),
            Spot::new(SpotKind::Work, p(20.0, 4.0)),
            Spot::new(SpotKind::Inn, p(0.0, -4.0)),
            Spot {
                radius: 6.0,
                ..Spot::new(SpotKind::Green, p(0.0, 18.0))
            },
        ],
        lanes: vec![
            vec![p(-30.0, 0.0), p(0.0, 0.0), p(30.0, 0.0)],
            vec![p(0.0, 0.0), p(0.0, 18.0)],
        ],
        roads: Vec::new(),
    }
}

fn block(
    start_hour: f32,
    place: u64,
    kind: SpotKind,
    activity: Activity,
    label: &str,
) -> ScheduleBlock {
    ScheduleBlock {
        start_hour,
        place,
        spot: SpotRef::Kind(kind),
        activity,
        label: label.into(),
    }
}

/// Eirmere at the origin and Kaelmar 500 m east, both adopted; Caelan works in
/// Eirmere from six, drinks in Kaelmar from noon, is home in Eirmere from six.
fn world() -> (LifeWorld, AgentId, u64, u64) {
    let mut life = LifeWorld::new(LodConfig::default());
    let (ka, kb) = (Place::key("Eirmere", Vec3::ZERO), Place::key("Kaelmar", B));
    life.adopt_place(ka, town("Eirmere", Vec3::ZERO), None);
    life.adopt_place(kb, town("Kaelmar", B), None);
    let caelan = life.adopt_person(
        "Caelan",
        Schedule {
            blocks: vec![
                block(
                    6.0,
                    ka,
                    SpotKind::Work,
                    Activity::Work,
                    "At the estate office",
                ),
                block(
                    12.0,
                    kb,
                    SpotKind::Inn,
                    Activity::Drink,
                    "At the Sheaf in Kaelmar",
                ),
                block(18.0, ka, SpotKind::Home, Activity::Sleep, "Home at Eirmere"),
            ],
        },
    );
    assert!(life.set_person_body(caelan, "characters/caelan_adult/caelan_adult.glb"));
    (life, caelan, ka, kb)
}

/// Step from `from` to `to` (hours), a game minute a second.
fn day(life: &mut LifeWorld, from: f32, to: f32, mut each: impl FnMut(&LifeWorld, f32)) {
    let mut hour = from;
    while hour < to {
        life.step(1.0, Clock::at(hour), &[Vec3::ZERO, B], &Flat);
        each(life, hour);
        hour += 1.0 / 60.0;
    }
}

fn near(p: Vec3, q: Vec3, within: f32) -> bool {
    Vec2::new(p.x - q.x, p.z - q.z).length() < within
}

#[test]
fn he_keeps_his_day_across_two_places_and_travels_once_each_way() {
    let (mut life, caelan, ka, kb) = world();
    let (mut setting_out, mut was_travelling) = (0, false);
    let mut seen = Vec::new();
    day(&mut life, 5.5, 22.0, |life, hour| {
        let at = life.person_at(caelan, Clock::at(hour)).unwrap();
        if at.travelling_to.is_some() && !was_travelling {
            setting_out += 1;
        }
        was_travelling = at.travelling_to.is_some();
        for check in [11.0f32, 14.0, 21.0] {
            if (hour - check).abs() < 0.5 / 60.0 {
                seen.push((check, at.place, at.position, at.label.clone()));
            }
        }
    });
    assert_eq!(setting_out, 2, "to Kaelmar at noon and back at six");
    let [(_, p11, at11, l11), (_, p14, at14, _), (_, p21, at21, _)] = seen.as_slice() else {
        panic!("{seen:?}")
    };
    assert_eq!((*p11, l11.as_str()), (ka, "At the estate office"));
    assert!(
        near(at11.unwrap(), Vec3::new(20.0, 0.0, 4.0), 1.0),
        "at work: {at11:?}"
    );
    assert_eq!(*p14, kb);
    assert!(
        near(at14.unwrap(), B + Vec3::new(0.0, 0.0, -4.0), 1.0),
        "in Kaelmar's inn: {at14:?}"
    );
    assert_eq!(*p21, ka);
    assert!(
        near(at21.unwrap(), Vec3::new(-20.0, 0.0, -4.0), 1.0),
        "home: {at21:?}"
    );
    let bodies: Vec<_> = life
        .villagers()
        .iter()
        .filter(|v| v.name == "Caelan")
        .collect();
    assert_eq!(bodies.len(), 2, "one villager in each place, made once");
    assert!(
        bodies
            .iter()
            .all(|v| v.body.as_deref() == Some("characters/caelan_adult/caelan_adult.glb"))
    );
}

#[test]
fn released_he_waits_and_resumed_he_walks_on_from_where_he_was_left() {
    let (mut life, caelan, ka, _) = world();
    day(&mut life, 6.0, 8.0, |_, _| {});
    let taken = life.release_person(caelan).expect("there to take");
    assert_eq!(taken.place, ka);
    day(&mut life, 8.0, 9.0, |_, _| {});
    assert_eq!(
        life.person_at(caelan, Clock::at(9.0)).unwrap(),
        taken,
        "not stepped while played"
    );
    assert!(
        life.villagers()
            .iter()
            .filter(|v| v.name == "Caelan")
            .all(|v| !v.visible),
        "nor drawn"
    );
    let left = Vec3::new(-25.0, 0.0, 0.0);
    assert!(life.resume_person(caelan, left, Vec3::X));
    life.step(1.0, Clock::at(9.0), &[Vec3::ZERO, B], &Flat);
    let at = life.person_at(caelan, Clock::at(9.0)).unwrap();
    assert!(
        near(at.position.unwrap(), left, 3.0),
        "on from where he was left: {at:?}"
    );
    day(&mut life, 9.0, 9.6, |_, _| {});
    let at = life.person_at(caelan, Clock::at(9.6)).unwrap();
    assert!(
        near(at.position.unwrap(), Vec3::new(20.0, 0.0, 4.0), 1.0),
        "and back at work: {at:?}"
    );
}

#[test]
fn the_storys_block_wins_until_its_hour() {
    let (mut life, caelan, ka, _) = world();
    day(&mut life, 6.0, 7.0, |_, _| {});
    assert!(life.override_block(
        caelan,
        block(0.0, ka, SpotKind::Inn, Activity::Shelter, "Under arrest"),
        9.0
    ));
    day(&mut life, 7.0, 8.0, |_, _| {});
    let at = life.person_at(caelan, Clock::at(8.0)).unwrap();
    assert_eq!(at.label, "Under arrest");
    assert!(near(at.position.unwrap(), Vec3::new(0.0, 0.0, -4.0), 1.0));
    day(&mut life, 8.0, 9.5, |_, _| {});
    let at = life.person_at(caelan, Clock::at(9.5)).unwrap();
    assert_eq!(at.label, "At the estate office", "the schedule again");
    assert!(near(at.position.unwrap(), Vec3::new(20.0, 0.0, 4.0), 1.0));
}

#[test]
fn in_an_alarm_he_shelters_where_he_is() {
    let mut life = LifeWorld::new(LodConfig::default());
    let ka = Place::key("Eirmere", Vec3::ZERO);
    life.adopt_place(ka, town("Eirmere", Vec3::ZERO), None);
    let caelan = life.adopt_person(
        "Caelan",
        Schedule {
            blocks: vec![block(
                0.0,
                ka,
                SpotKind::Green,
                Activity::Wander,
                "On the green",
            )],
        },
    );
    day(&mut life, 10.0, 10.2, |_, _| {});
    let green = Vec3::new(0.0, 0.0, 18.0);
    life.alarm(green, 30.0, 60.0);
    day(&mut life, 10.2, 11.0, |_, _| {});
    let at = life.person_at(caelan, Clock::at(11.0)).unwrap();
    assert_eq!(at.place, ka, "still in Eirmere");
    assert!(
        near(at.position.unwrap(), green, 7.0),
        "at his spot, not run off: {at:?}"
    );
}

#[test]
fn time_skipped_puts_him_where_his_schedule_says() {
    let (mut life, caelan, _, kb) = world();
    day(&mut life, 6.0, 8.0, |_, _| {});
    // Six hours in one step (a sleep): in Kaelmar's inn already, the road taken.
    life.step(1.0, Clock::at(14.0), &[Vec3::ZERO, B], &Flat);
    let at = life.person_at(caelan, Clock::at(14.0)).unwrap();
    assert_eq!((at.place, at.travelling_to), (kb, None));
    assert!(
        near(at.position.unwrap(), B + Vec3::new(0.0, 0.0, -4.0), 0.5),
        "{at:?}"
    );
}

#[test]
fn he_is_never_asleep_wherever_the_view_is() {
    let (mut life, _, _, _) = world();
    for i in 0..120 {
        life.step(
            1.0,
            Clock::at(8.0 + i as f32 / 60.0),
            &[Vec3::new(9e4, 0.0, 9e4)],
            &Flat,
        );
    }
    let him: Vec<_> = life
        .villagers()
        .iter()
        .filter(|v| v.name == "Caelan")
        .collect();
    assert!(!him.is_empty());
    assert!(
        him.iter().all(|v| v.tier != Tier::Asleep),
        "far from any view, still living his day"
    );
}

#[test]
fn a_table_finds_a_schedules_place_and_spot() {
    let place = town("Vespergate", Vec3::new(-5600.0, 0.0, 3100.0));
    let table = LifeTable {
        version: 1,
        places: vec![TablePlace {
            centre: [-5600.0, 0.0, 3100.0],
            radius: 40.0,
            seed: 1,
            place,
            people: Default::default(),
        }],
        looks: None,
    };
    let key = table.key_of("vespergate").expect("by name");
    assert_eq!(
        table.spot_position(key, &SpotRef::Kind(SpotKind::Inn)),
        Some(Vec3::new(-5600.0, 0.0, 3096.0))
    );
    assert_eq!(table.spot_position(key, &SpotRef::Index(9)), None);
}
