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
fn neighbours_idling_at_a_spot_stop_to_talk_face_to_face() {
    let mut w = LifeWorld::new(LodConfig::default());
    let place = w.add_place(village(Vec3::ZERO));
    for _ in 0..4 {
        w.populate(place, 11);
    }
    let mut talks = 0;
    day(&mut w, 6.0, 20.0, 0.0, &[Vec3::ZERO], |w, _| {
        let talking: Vec<_> = w
            .villagers()
            .iter()
            .filter(|v| v.pose == Pose::Talk)
            .collect();
        for v in &talking {
            // Each faces a neighbour within reach who is talking too.
            assert!(talking.iter().any(|o| {
                o.id != v.id && flat(o.position, v.position) <= 4.0 && {
                    let to = (o.position - v.position).normalize_or_zero();
                    v.facing.dot(to) > 0.99
                }
            }));
        }
        talks += talking.len();
    });
    assert!(talks > 0, "somebody stopped to talk");
}

#[test]
fn villagers_notice_the_player_passing_once_a_minute() {
    let mut w = LifeWorld::new(LodConfig::default());
    let place = w.add_place(village(Vec3::ZERO));
    w.populate(place, 5);
    day(&mut w, 6.0, 11.0, 0.0, &[Vec3::ZERO], |_, _| {});
    let (who, at) = w
        .villagers()
        .iter()
        .find(|v| v.visible && v.pose != Pose::Walk)
        .map(|v| (v.id, v.position))
        .expect("someone standing out of doors");
    let player = at + Vec3::new(2.0, 0.0, 0.0);
    let noticed = w.notice(player, 3.0);
    assert!(noticed.contains(&who));
    let v = w.villagers().iter().find(|v| v.id == who).unwrap();
    assert!(v.facing.dot(Vec3::X) > 0.99, "turned to face the player");
    assert!(!w.notice(player, 3.0).contains(&who), "not again at once");
    for _ in 0..130 {
        w.step(DT, Clock::at(11.0), &[at], &Flat);
    }
    let again = w.villagers().iter().find(|v| v.id == who).unwrap().position + Vec3::X * 2.0;
    assert!(w.notice(again, 3.0).contains(&who), "again after a minute");
    assert!(
        w.notice(Vec3::splat(1.0e4), 3.0).is_empty(),
        "nobody far off"
    );
}

#[test]
fn villagers_walk_round_the_player_not_through() {
    let mut w = LifeWorld::new(LodConfig::default());
    let place = w.add_place(village(Vec3::ZERO));
    w.populate(place, 9);
    // The player stands in the middle of the main lane all day.
    let player = Vec3::new(-20.0, 0.0, 0.0);
    let mut clear = true;
    let mut brushed = 0;
    day(&mut w, 6.0, 12.0, 0.0, &[player], |_, _| {});
    for _ in 0..2000 {
        w.step(DT, Clock::at(12.0), &[player], &Flat);
        brushed += w.keep_clear(player, 0.7);
        clear &= w
            .villagers()
            .iter()
            .filter(|v| v.visible)
            .all(|v| flat(v.position, player) >= 0.7 - 1e-4);
    }
    assert!(clear, "nobody stands in the player");
    assert!(brushed > 0, "people came by and went round");
    // And everyone still gets where they were going.
    let farmers_in_field = w
        .villagers()
        .iter()
        .filter(|v| v.role == Role::Farmer)
        .all(|v| v.activity != Activity::Farm || !v.visible || v.position.z > 30.0);
    assert!(farmers_in_field);
}

#[test]
fn some_households_keep_a_dog_that_roams_its_door() {
    let mut w = LifeWorld::new(LodConfig::default());
    let place = w.add_place(village(Vec3::ZERO));
    for seed in 0..6 {
        w.populate(place, seed);
    }
    let homes: Vec<Vec3> = w
        .place(place)
        .unwrap()
        .spots_of(SpotKind::Home)
        .map(|h| w.place(place).unwrap().spots[h].position)
        .collect();
    let dogs: Vec<_> = w
        .beasts()
        .iter()
        .filter(|b| b.kind == BeastKind::Dog)
        .map(|b| b.id)
        .collect();
    assert!(
        !dogs.is_empty() && dogs.len() < 6 * homes.len(),
        "some, not all: {}",
        dogs.len()
    );
    day(&mut w, 8.0, 12.0, 0.0, &[Vec3::ZERO], |w, _| {
        for b in w.beasts().iter().filter(|b| b.kind == BeastKind::Dog) {
            let near = homes
                .iter()
                .map(|h| flat(*h, b.position))
                .fold(f32::INFINITY, f32::min);
            assert!(near < 8.5, "a dog stays about its door: {near} m");
        }
    });
}

/// Beyond `asleep` (its cells unloaded) a town is not stepped at all; when the
/// view comes back each wakes owed the hours and catches up at once, out at
/// the day's work rather than abed where the view left them.
#[test]
fn an_unloaded_town_sleeps_and_catches_up_when_the_view_returns() {
    let lod = LodConfig {
        asleep: 1_000.0,
        ..LodConfig::default()
    };
    let mut w = LifeWorld::new(lod);
    let town = Vec3::new(3_000.0, 0.0, 0.0);
    let place = w.add_place(village(town));
    w.populate(place, 5);
    let before: Vec<Vec3> = w.villagers().iter().map(|v| v.position).collect();
    day(&mut w, 5.0, 9.0, 0.0, &[Vec3::ZERO], |w, _| {
        let s = w.stats();
        assert_eq!((s.near, s.mid, s.far, s.stepped), (0, 0, 0, 0));
        assert!(s.asleep >= 5);
    });
    let now: Vec<Vec3> = w.villagers().iter().map(|v| v.position).collect();
    assert_eq!(now, before, "nobody moved while asleep");
    day(&mut w, 9.0, 9.1, 0.0, &[town], |_, _| {});
    assert_eq!(w.stats().asleep, 0);
    let abed = w
        .villagers()
        .iter()
        .filter(|v| v.activity == Activity::Sleep)
        .count();
    assert_eq!(abed, 0, "awake and about at nine");
    assert!(
        w.villagers()
            .iter()
            .zip(&before)
            .any(|(v, b)| flat(v.position, *b) > 5.0),
        "gone about their day"
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

/// A real realm's table: how long each town takes to house, and a whole day
/// with the view in its biggest town and every town housed
/// (`SOMNIUM_LIFE_TABLE=<table.json> cargo test -p somnium_life --release
/// --test life realm_table -- --ignored --nocapture`).
#[test]
#[ignore = "reads a game's life table; timing"]
fn realm_table() {
    let Ok(path) = std::env::var("SOMNIUM_LIFE_TABLE") else {
        return;
    };
    let table =
        somnium_life::LifeTable::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut w = LifeWorld::new(LodConfig::default());
    let mut slowest = (0.0_f64, String::new());
    let started = std::time::Instant::now();
    for p in &table.places {
        let t = std::time::Instant::now();
        w.adopt_place(p.key(), p.place.clone(), Some(p.seed));
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        if ms > slowest.0 {
            slowest = (ms, p.place.name.clone());
        }
    }
    eprintln!(
        "{} places housed in {:.1} ms; slowest {} {:.2} ms; {} villagers, {} beasts, {} carts",
        table.places.len(),
        started.elapsed().as_secs_f64() * 1000.0,
        slowest.1,
        slowest.0,
        w.villagers().len(),
        w.beasts().len(),
        w.carts().len()
    );
    let biggest = table
        .places
        .iter()
        .max_by_key(|p| p.place.spots.len())
        .unwrap();
    // Its densest 256 m cell (the most homes): the view stands at their middle.
    let mut cells: std::collections::HashMap<(i32, i32), Vec<Vec3>> = Default::default();
    for s in biggest
        .place
        .spots
        .iter()
        .filter(|s| s.kind == SpotKind::Home)
    {
        let key = (
            (s.position.x / 256.0).floor() as i32,
            (s.position.z / 256.0).floor() as i32,
        );
        cells.entry(key).or_default().push(s.position);
    }
    let homes = cells.values().max_by_key(|v| v.len()).unwrap();
    let view = homes.iter().copied().sum::<Vec3>() / homes.len() as f32;
    // Noon there: the villager pool (the nearest 160 out of doors) at WS3's
    // crowd sizes (adult 52,189 / 20,538 / 7,834 posed vertices; child 43,851
    // / 17,013 / 6,497), LOD 1 past 20 m and LOD 2 past 60 m, fitted to the
    // 1.6 M budget.
    let mut hour = 6.0_f32;
    while hour < 12.0 {
        w.step(0.25, Clock::at(hour), &[view], &Flat);
        hour += 0.25 / 60.0;
    }
    let mut out: Vec<(f32, bool)> = w
        .villagers()
        .iter()
        .filter(|v| v.visible)
        .map(|v| (v.position.distance(view), v.role == Role::Child))
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.truncate(160);
    let (adult, child) = (
        vec![52_189_u32, 20_538, 7_834],
        vec![43_851_u32, 17_013, 6_497],
    );
    let wants: Vec<usize> = out
        .iter()
        .map(|(d, _)| usize::from(*d > 20.0) + usize::from(*d > 60.0))
        .collect();
    let sizes: Vec<Vec<u32>> = out
        .iter()
        .map(|(_, c)| if *c { child.clone() } else { adult.clone() })
        .collect();
    let fitted = somnium_life::crowd::fit_levels(1_600_000, &wants, &sizes);
    let sum = |levels: &[usize]| {
        levels
            .iter()
            .zip(&sizes)
            .map(|(l, s)| u64::from(s[*l]))
            .sum::<u64>()
    };
    let count = |levels: &[usize], l: usize| levels.iter().filter(|x| **x == l).count();
    eprintln!(
        "{}'s densest cell ({} homes): {} out of doors in the pool at noon; by distance alone {} posed vertices (LOD 0/1/2: {}/{}/{}); fitted to 1.6 M: {} ({}/{}/{})",
        biggest.place.name,
        homes.len(),
        out.len(),
        sum(&wants),
        count(&wants, 0),
        count(&wants, 1),
        count(&wants, 2),
        sum(&fitted),
        count(&fitted, 0),
        count(&fitted, 1),
        count(&fitted, 2)
    );
    let dt = 1.0 / 60.0;
    let mut hour = 0.0_f32;
    let mut ticks = Vec::new();
    let mut near = 0;
    while hour < 24.0 {
        let t = std::time::Instant::now();
        w.step(dt, Clock::at(hour), &[view], &Flat);
        ticks.push(t.elapsed().as_secs_f64() * 1000.0);
        near = near.max(w.stats().near);
        hour += dt / 6.0;
    }
    let mean = ticks.iter().sum::<f64>() / ticks.len() as f64;
    ticks.sort_by(f64::total_cmp);
    let at = |q: f64| ticks[((ticks.len() - 1) as f64 * q) as usize];
    eprintln!(
        "a day in {} (view in its densest cell): mean {mean:.4} ms a tick, p99 {:.3}, p99.9 {:.3}, worst {:.3}; up to {near} near",
        biggest.place.name,
        at(0.99),
        at(0.999),
        ticks.last().unwrap()
    );
}
