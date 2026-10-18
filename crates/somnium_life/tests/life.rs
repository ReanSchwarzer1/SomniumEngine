//! Villagers keep their day, go indoors under the Lid, beasts keep to their
//! pens, carts keep to the road and to their distance; far away all of it
//! steps rarely and still keeps time.
use glam::{Vec2, Vec3};
use somnium_life::place::{AreaRef, BuildingRef};
use somnium_life::{
    Activity, BeastKind, Clock, Flat, LaneGraph, LifeWorld, LodConfig, Place, Pose, Role, Spot,
    SpotKind, Tier, VillagerDesc,
};

const DT: f32 = 0.5;

fn flat(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.z - b.z).length()
}

/// A lane east-west along z 0 with a lane north from its middle; five homes,
/// a smithy, a church and an inn on it; a green, a well, a field up the north
/// lane and a pen to the west.
fn village(at: Vec3) -> Place {
    let p = |x: f32, z: f32| at + Vec3::new(x, 0.0, z);
    let mut spots: Vec<Spot> = (0..5)
        .map(|i| Spot::new(SpotKind::Home, p(-40.0 + 15.0 * i as f32, -4.0)))
        .collect();
    spots.push(Spot::new(SpotKind::Work, p(40.0, 4.0)));
    spots.push(Spot::new(SpotKind::Chapel, p(-45.0, 4.0)));
    spots.push(Spot::new(SpotKind::Inn, p(8.0, -4.0)));
    spots.push(Spot {
        radius: 6.0,
        ..Spot::new(SpotKind::Green, p(0.0, 18.0))
    });
    spots.push(Spot::new(SpotKind::Well, p(10.0, 6.0)));
    spots.push(Spot {
        radius: 10.0,
        ..Spot::new(SpotKind::Field, p(0.0, 55.0))
    });
    spots.push(Spot {
        radius: 6.0,
        ..Spot::new(SpotKind::Pen, p(-30.0, 25.0))
    });
    Place {
        name: "Testby".into(),
        spots,
        lanes: vec![
            vec![p(-60.0, 0.0), p(0.0, 0.0), p(60.0, 0.0)],
            vec![p(0.0, 0.0), p(0.0, 25.0), p(0.0, 45.0)],
        ],
        roads: vec![vec![p(-60.0, 0.0), p(60.0, 0.0)]],
    }
}

fn spot(place: &Place, kind: SpotKind) -> usize {
    place.spots_of(kind).next().expect("spot")
}

/// Run from `from` to `to` o'clock, an hour every 50 seconds of real time.
fn day(
    w: &mut LifeWorld,
    from: f32,
    to: f32,
    eclipse: f32,
    focus: &[Vec3],
    mut each: impl FnMut(&LifeWorld, f32),
) {
    let per_step = DT / 50.0;
    let mut hour = from;
    while hour < to {
        w.step(DT, Clock { hour, eclipse }, focus, &Flat);
        each(w, hour);
        hour += per_step;
    }
}

#[test]
fn routines_say_what_to_do_each_hour() {
    let farmer = Role::Farmer.routine();
    assert_eq!(farmer.at(3.0), Activity::Sleep);
    assert_eq!(farmer.at(7.0), Activity::Farm);
    assert_eq!(farmer.at(12.5), Activity::Eat);
    assert_eq!(farmer.at(23.0), Activity::Sleep);
    assert_eq!(farmer.at(-1.0), Activity::Sleep, "wraps past midnight");
    assert_eq!(
        Role::Keeper.routine().at(22.0),
        Activity::Work,
        "the keeper keeps late"
    );
    assert_eq!(Role::Priest.routine().at(5.5), Activity::Worship);
}

#[test]
fn lanes_lead_the_way_and_near_spots_go_straight() {
    let place = village(Vec3::ZERO);
    let graph = LaneGraph::new(&place.lanes);
    assert_eq!(graph.len(), 5, "the junction is shared");
    let path = graph.path(Vec3::new(-50.0, 0.0, -3.0), Vec3::new(0.0, 0.0, 50.0));
    assert!(path.contains(&Vec3::ZERO), "by the junction: {path:?}");
    assert_eq!(*path.last().unwrap(), Vec3::new(0.0, 0.0, 50.0));
    assert_eq!(graph.path(Vec3::ZERO, Vec3::new(3.0, 0.0, 4.0)).len(), 1);
}

#[test]
fn a_farmer_goes_to_the_field_by_day_and_home_to_bed() {
    let mut w = LifeWorld::new(LodConfig::default());
    let place = village(Vec3::ZERO);
    let (home, field) = (spot(&place, SpotKind::Home), spot(&place, SpotKind::Field));
    let field_at = place.spots[field].position;
    let home_at = place.spots[home].position;
    let p = w.add_place(place);
    w.spawn_villager(VillagerDesc {
        name: "Gwen".into(),
        role: Role::Farmer,
        place: p,
        home,
        work: Some(field),
    });
    let mut seen_farming = false;
    let mut asleep_at_dawn = false;
    day(&mut w, 4.0, 23.5, 0.0, &[], |w, hour| {
        let v = &w.villagers()[0];
        if hour < 5.0 {
            asleep_at_dawn |= !v.visible;
        }
        if (8.5..11.5).contains(&hour) && v.pose == Pose::Farm {
            assert!(
                flat(v.position, field_at) < 10.5,
                "in the field: {}",
                v.position
            );
            seen_farming = true;
        }
    });
    let v = &w.villagers()[0];
    assert!(asleep_at_dawn, "indoors before dawn");
    assert!(seen_farming, "farming in the morning");
    assert!(!v.visible && v.pose == Pose::Inside, "abed at half eleven");
    assert!(flat(v.position, home_at) < 0.1, "at home: {}", v.position);
}

#[test]
fn when_the_lid_falls_everyone_goes_indoors() {
    let mut w = LifeWorld::new(LodConfig::default());
    let p = w.add_place(village(Vec3::ZERO));
    let spawned = w.populate(p, 7);
    assert!(w.villagers().len() >= 5, "a household or more in each home");
    assert!(spawned.len() > w.villagers().len(), "and beasts and a cart");
    day(&mut w, 10.0, 11.0, 0.0, &[], |_, _| {});
    let out = w.villagers().iter().filter(|v| v.visible).count();
    assert!(
        out * 2 > w.villagers().len(),
        "a busy late morning: {out} out"
    );
    day(&mut w, 11.0, 13.0, 1.0, &[], |_, _| {});
    let place = w.place(p).unwrap().clone();
    for v in w.villagers() {
        assert!(!v.visible, "{} is still out at {}", v.name, v.position);
        assert_eq!(v.target, Some(v.home), "{} went home", v.name);
        assert!(place.spots[v.home].kind.indoor());
    }
}

#[test]
fn livestock_keep_to_their_pen_and_graze() {
    let mut w = LifeWorld::new(LodConfig::default());
    let place = village(Vec3::ZERO);
    let pen = spot(&place, SpotKind::Pen);
    let (centre, radius) = (place.spots[pen].position, place.spots[pen].radius);
    let p = w.add_place(place);
    for _ in 0..6 {
        w.spawn_beast(BeastKind::Sheep, p, pen);
    }
    let start: Vec<Vec3> = w.beasts().iter().map(|b| b.position).collect();
    let mut grazed = 0;
    day(&mut w, 9.0, 12.0, 0.0, &[], |w, _| {
        for b in w.beasts() {
            assert!(
                flat(b.position, centre) <= radius,
                "in the pen: {}",
                b.position
            );
            grazed += usize::from(b.pose == Pose::Graze);
        }
    });
    let moved = w
        .beasts()
        .iter()
        .zip(&start)
        .filter(|(b, s)| flat(b.position, **s) > 0.5)
        .count();
    assert!(moved >= 4, "they wander: {moved}");
    assert!(grazed > 6 * 100, "and mostly graze: {grazed}");
}

#[test]
fn carts_drive_the_road_keep_their_distance_and_stop_at_night() {
    let mut w = LifeWorld::new(LodConfig::default());
    let p = w.add_place(village(Vec3::ZERO));
    w.spawn_cart(p, 0, 0.10);
    w.spawn_cart(p, 0, 0.12);
    let mut closest = f32::MAX;
    let mut turned = false;
    let mut last_x = w.carts()[0].position.x;
    day(&mut w, 8.0, 10.0, 0.0, &[], |w, hour| {
        let [a, b] = [&w.carts()[0], &w.carts()[1]];
        if hour > 8.1 && a.facing.dot(b.facing) > 0.9 {
            closest = closest.min(flat(a.position, b.position));
        }
        turned |= a.position.x < last_x - 0.01;
        last_x = a.position.x;
    });
    assert!(closest > 6.5, "the second held back: {closest}");
    assert!(turned, "the first reached the end and came back");
    for c in w.carts() {
        assert!(c.position.z.abs() > 0.5, "each keeps to a side of the road");
    }
    let parked: Vec<Vec3> = w.carts().iter().map(|c| c.position).collect();
    day(&mut w, 21.0, 22.0, 0.0, &[], |_, _| {});
    // One more step at night moves nobody.
    let before: Vec<Vec3> = w.carts().iter().map(|c| c.position).collect();
    day(&mut w, 22.0, 23.0, 0.0, &[], |_, _| {});
    for (a, b) in before.iter().zip(w.carts().iter().map(|c| c.position)) {
        assert_eq!(*a, b, "parked for the night");
    }
    let _ = parked;
}

#[test]
fn far_away_a_village_steps_rarely_and_still_keeps_its_day() {
    let mut w = LifeWorld::new(LodConfig::default());
    let far = Vec3::new(2000.0, 0.0, 0.0);
    let p = w.add_place(village(far));
    w.populate(p, 3);
    let n = w.villagers().len() + w.beasts().len() + w.carts().len();
    let focus = [Vec3::ZERO];
    let mut steps = 0usize;
    let mut ticks = 0usize;
    day(&mut w, 6.0, 9.5, 0.0, &focus, |w, _| {
        let s = w.stats();
        assert_eq!(s.far, n, "all far");
        steps += s.stepped;
        ticks += 1;
    });
    let per_tick = steps as f32 / ticks as f32;
    assert!(per_tick < n as f32 / 20.0, "{per_tick} of {n} a tick");
    // Coarse, but on time: the farmers are out at the fields by half nine.
    let place = w.place(p).unwrap();
    let field = place.spots[spot(place, SpotKind::Field)].clone();
    let farmers: Vec<_> = w
        .villagers()
        .iter()
        .filter(|v| v.role == Role::Farmer)
        .collect();
    assert!(!farmers.is_empty());
    for f in farmers {
        assert_eq!(f.tier, Tier::Far);
        assert!(
            flat(f.position, field.position) <= field.radius + 0.5,
            "{} at {}",
            f.name,
            f.position
        );
    }
}

#[test]
fn a_tight_budget_defers_steps_and_they_catch_up() {
    let lod = LodConfig {
        budget: 3,
        mid_stride: 1,
        ..LodConfig::default()
    };
    let mut w = LifeWorld::new(lod);
    let p = w.add_place(village(Vec3::new(100.0, 0.0, 0.0)));
    w.populate(p, 5);
    let mut deferred = 0;
    day(&mut w, 9.0, 10.0, 0.0, &[Vec3::ZERO], |w, _| {
        assert!(w.stats().stepped - w.stats().near <= 3);
        deferred += w.stats().deferred;
    });
    assert!(deferred > 0, "the budget held some back");
    // Everyone still got their morning: nobody abed at ten.
    let abed = w
        .villagers()
        .iter()
        .filter(|v| v.activity == Activity::Sleep)
        .count();
    assert_eq!(abed, 0);
}

#[test]
fn a_place_from_a_settlement_layout_and_back_through_json() {
    let b = |kind: &str, x: f32| BuildingRef {
        kind: kind.into(),
        door: Vec3::new(x, 0.0, 0.0),
        label: format!("{kind} {x}"),
    };
    let buildings = [
        b("cottage", 0.0),
        b("cottage", 10.0),
        b("longhouse", 20.0),
        b("smithy", 30.0),
        b("church", 40.0),
        b("byre", 50.0),
        b("storehouse", 60.0),
        b("dovecote", 70.0),
    ];
    let areas = [
        AreaRef {
            name: "the green".into(),
            centre: Vec3::new(0.0, 0.0, 20.0),
            radius: 8.0,
        },
        AreaRef {
            name: "north field".into(),
            centre: Vec3::new(0.0, 0.0, 60.0),
            radius: 20.0,
        },
        AreaRef {
            name: "the keep clear".into(),
            centre: Vec3::ZERO,
            radius: 1.0,
        },
    ];
    let place = Place::from_layout(
        "Bryn",
        &buildings,
        &areas,
        vec![vec![Vec3::ZERO, Vec3::new(70.0, 0.0, 0.0)]],
    );
    let count = |k: SpotKind| place.spots_of(k).count();
    assert_eq!(count(SpotKind::Home), 3);
    assert_eq!(count(SpotKind::Work), 2);
    assert_eq!(count(SpotKind::Chapel), 1);
    assert_eq!(count(SpotKind::Pen), 1);
    assert_eq!(count(SpotKind::Green), 1);
    assert_eq!(count(SpotKind::Field), 1);
    assert_eq!(
        place.spots.len(),
        9,
        "the dovecote and the clearing mean nothing to life"
    );
    let json = serde_json::to_string(&place).unwrap();
    let back: Place = serde_json::from_str(&json).unwrap();
    assert_eq!(back, place);
    let mut w = LifeWorld::new(LodConfig::default());
    let p = w.add_place(place);
    w.populate(p, 11);
    let roles: Vec<Role> = w.villagers().iter().map(|v| v.role).collect();
    assert!(
        roles.contains(&Role::Smith)
            && roles.contains(&Role::Priest)
            && roles.contains(&Role::Herder),
        "{roles:?}"
    );
    assert!(w.beasts().len() >= 4, "beasts in the byre's yard");
    for v in w.villagers() {
        if let Some(work) = v.role.workplace() {
            if work != SpotKind::Field {
                let p = w.place(p).unwrap();
                assert!(
                    v.work
                        .is_some_and(|s| p.spots[s].kind == work || work == SpotKind::Work),
                    "{} works",
                    v.name
                );
            }
        }
    }
}

/// `cargo test -p somnium_life --release -- --ignored --nocapture`: the cost
/// of a crowd: fifty villages with their beasts and carts, the player in one.
#[test]
fn an_alarm_sends_villagers_running_home_and_they_come_out_after() {
    let mut w = LifeWorld::new(LodConfig::default());
    let place = w.add_place(village(Vec3::ZERO));
    w.populate(place, 3);
    day(&mut w, 6.0, 11.0, 0.0, &[Vec3::ZERO], |_, _| {});
    let out_before = w.villagers().iter().filter(|v| v.visible).count();
    assert!(out_before > 0, "out and about by eleven");
    // A fight breaks out on the road through the middle.
    let frightened = w.alarm(Vec3::ZERO, 200.0, 60.0);
    assert!(frightened >= out_before);
    let (mut ran, mut bolted) = (false, false);
    let clock = Clock::at(11.0);
    for _ in 0..80 {
        w.step(DT, clock, &[Vec3::ZERO], &Flat);
        ran |= w.villagers().iter().any(|v| v.pose == Pose::Run);
        bolted |= w.beasts().iter().any(|b| b.pose == Pose::Run);
        assert!(
            w.carts().iter().all(|c| c.pose != Pose::Drive),
            "carts stop"
        );
    }
    assert!(ran, "they ran");
    assert!(bolted, "the beasts bolted");
    assert!(
        w.villagers().iter().all(|v| !v.visible),
        "all indoors within forty seconds"
    );
    assert!(
        w.villagers()
            .iter()
            .all(|v| v.activity == Activity::Shelter)
    );
    // The danger passes: back to the day.
    for _ in 0..200 {
        w.step(DT, clock, &[Vec3::ZERO], &Flat);
    }
    assert!(w.villagers().iter().any(|v| v.visible), "out again");
    assert!(
        w.villagers()
            .iter()
            .all(|v| v.activity != Activity::Shelter)
    );
}

#[test]
fn a_place_that_streams_back_keeps_its_people() {
    let mut w = LifeWorld::new(LodConfig::default());
    let at = Vec3::new(500.0, 0.0, 0.0);
    let key = Place::key("Testby", at);
    let (index, new) = w.adopt_place(key, village(at), Some(7));
    assert!(new);
    let people = w.villagers().len();
    assert!(people > 0);
    // Its cell unloads; the day goes on in life, far from the player.
    day(&mut w, 6.0, 11.0, 0.0, &[Vec3::ZERO], |_, _| {});
    let out = w.villagers().iter().filter(|v| v.visible).count();
    // Streamed back: same key, same place, nobody housed twice.
    assert_eq!(
        w.adopt_place(Place::key("Testby", at), village(at), Some(7)),
        (index, false)
    );
    assert_eq!(w.villagers().len(), people);
    assert_eq!(w.villagers().iter().filter(|v| v.visible).count(), out);
    assert_eq!(w.place_count(), 1);
    // Another place, or the same name elsewhere, is new.
    assert_ne!(Place::key("Testby", at + Vec3::X), key);
    let (other, new) = w.adopt_place(
        Place::key("Otherby", at),
        village(at + Vec3::Z * 300.0),
        Some(7),
    );
    assert!(new);
    assert_eq!(other, 1);
    assert!(w.villagers().len() > people);
}

#[test]
#[ignore]
fn crowd_cost() {
    let build = || {
        let mut w = LifeWorld::new(LodConfig::default());
        // Fifty villages 600 m apart (Tyffir's spacing), the player in the first.
        for i in 0..50 {
            let at = Vec3::new((i % 10) as f32 * 600.0, 0.0, (i / 10) as f32 * 600.0);
            let mut place = village(at);
            // Twenty homes a village.
            for k in 0..15 {
                place.spots.push(Spot::new(
                    SpotKind::Home,
                    at + Vec3::new(-50.0 + 7.0 * k as f32, 0.0, -10.0),
                ));
            }
            let p = w.add_place(place);
            w.populate(p, i as u64);
        }
        w
    };
    for (label, focus) in [
        ("everyone near (no LOD)", vec![]),
        ("player in one village", vec![Vec3::ZERO]),
    ] {
        let mut w = build();
        let agents = w.villagers().len() + w.beasts().len() + w.carts().len();
        let start = std::time::Instant::now();
        let ticks = 3000;
        let mut hour = 6.0;
        let mut stepped = 0;
        for _ in 0..ticks {
            w.step(1.0 / 60.0, Clock::at(hour), &focus, &Flat);
            stepped += w.stats().stepped;
            hour += 1.0 / 60.0 / 50.0;
        }
        let per = start.elapsed().as_secs_f64() * 1000.0 / ticks as f64;
        println!(
            "{label}: {agents} agents, {per:.4} ms a tick, {:.0} stepped a tick",
            stepped as f64 / ticks as f64
        );
    }
}
