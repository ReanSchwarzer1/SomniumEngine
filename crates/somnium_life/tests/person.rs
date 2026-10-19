//! A person with a day of his own (the dual protagonists' off-screen man):
//! a villager of whichever adopted place his schedule has him in, drawn as
//! himself; away everywhere else; found by the schedule where nothing is
//! adopted; let go when the player takes him.
use glam::{Vec2, Vec3};
use somnium_life::{
    Activity, Clock, Flat, LifeTable, LifeWorld, LodConfig, Place, ScheduleEntry, Spot, SpotKind,
    SpotRef, TablePlace,
};

fn town(name: &str, at: Vec3) -> Place {
    let p = |x: f32, z: f32| at + Vec3::new(x, 0.0, z);
    Place {
        name: name.into(),
        spots: vec![
            Spot::new(SpotKind::Home, p(-20.0, -4.0)),
            Spot::new(SpotKind::Work, p(20.0, 4.0)),
            Spot::new(SpotKind::Inn, p(0.0, -4.0)),
        ],
        lanes: vec![vec![p(-30.0, 0.0), p(0.0, 0.0), p(30.0, 0.0)]],
        roads: Vec::new(),
    }
}

fn run(life: &mut LifeWorld, hour: f32, seconds: f32) {
    for _ in 0..(seconds / 0.5) as usize {
        life.step(
            0.5,
            Clock::at(hour),
            &[Vec3::ZERO, Vec3::new(500.0, 0.0, 0.0)],
            &Flat,
        );
    }
}

#[test]
fn a_person_keeps_his_own_day_across_places() {
    let mut life = LifeWorld::new(LodConfig::default());
    let (a, b) = (Vec3::ZERO, Vec3::new(500.0, 0.0, 0.0));
    let (ka, kb, kc) = (
        Place::key("Eirmere", a),
        Place::key("Kaelmar", b),
        Place::key("Vespergate", Vec3::X * 9e3),
    );
    life.adopt_place(ka, town("Eirmere", a), None);
    life.adopt_place(kb, town("Kaelmar", b), None);
    let entry = |from_hour, to_hour, place, kind, activity| ScheduleEntry {
        from_hour,
        to_hour,
        place,
        spot: SpotRef::Kind(kind),
        activity,
    };
    let caelan = life.adopt_person(
        "Caelan",
        Some("characters/caelan_adult/caelan_adult.glb"),
        vec![
            entry(6.0, 12.0, ka, SpotKind::Work, Activity::Work),
            entry(12.0, 18.0, kb, SpotKind::Inn, Activity::Drink),
            entry(18.0, 6.0, kc, SpotKind::Home, Activity::Sleep),
        ],
    );
    assert!(life.villagers().is_empty(), "nobody until a step");

    run(&mut life, 8.0, 60.0);
    let him: Vec<_> = life
        .villagers()
        .iter()
        .filter(|v| v.name == "Caelan")
        .collect();
    assert_eq!(him.len(), 1, "a villager of Eirmere");
    assert_eq!(
        him[0].body.as_deref(),
        Some("characters/caelan_adult/caelan_adult.glb")
    );
    let work = Vec3::new(20.0, 0.0, 4.0);
    assert!(
        Vec2::new(him[0].position.x - work.x, him[0].position.z - work.z).length() < 4.0,
        "at work"
    );
    let at = life.person_at(caelan, Clock::at(8.0)).unwrap();
    assert_eq!(at.place, ka);
    assert!(at.position.unwrap().distance(him[0].position) < 1e-3);

    run(&mut life, 14.0, 60.0);
    let bodies: Vec<_> = life
        .villagers()
        .iter()
        .filter(|v| v.name == "Caelan")
        .collect();
    assert_eq!(bodies.len(), 2, "and now one of Kaelmar");
    let in_eirmere = bodies.iter().find(|v| v.position.x < 250.0).unwrap();
    assert!(!in_eirmere.visible, "away from Eirmere");
    assert_eq!(life.person_at(caelan, Clock::at(14.0)).unwrap().place, kb);

    run(&mut life, 20.0, 10.0);
    let night = life.person_at(caelan, Clock::at(20.0)).unwrap();
    assert_eq!(
        (night.place, night.position),
        (kc, None),
        "in Vespergate, not adopted: by the schedule only"
    );
    assert!(
        life.villagers()
            .iter()
            .filter(|v| v.name == "Caelan")
            .all(|v| !v.visible)
    );

    assert!(life.release_person(caelan));
    assert!(life.person_at(caelan, Clock::at(8.0)).is_none());
    run(&mut life, 8.0, 30.0);
    assert!(
        life.villagers()
            .iter()
            .filter(|v| v.name == "Caelan")
            .all(|v| !v.visible),
        "the player has him"
    );
    assert!(!life.release_person(caelan));
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
