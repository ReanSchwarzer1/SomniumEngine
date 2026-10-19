//! Off-mesh link kinds and path legs: how gameplay learns to cross a link.
use glam::Vec3;
use somnium_ai::navigation::*;

fn plane(x: f32, z: f32, w: f32, h: f32, y: f32) -> Vec<Triangle> {
    vec![
        [[x, y, z], [x + w, y, z], [x + w, y, z + h]],
        [[x, y, z], [x + w, y, z + h], [x, y, z + h]],
    ]
}

/// Two 10 m floors 2 m apart across x, the far one `rise` higher.
fn two_floors(rise: f32) -> NavWorld {
    let mut triangles = plane(0.0, 0.0, 10.0, 10.0, 0.0);
    triangles.extend(plane(12.0, 0.0, 10.0, 10.0, rise));
    let tile = bake(&BakeInput {
        cell: [0, 0, 0],
        bounds: Bounds {
            min: [0.0, -1.0, 0.0],
            max: [22.0, 4.0, 10.0],
        },
        triangles,
        obstacles: vec![],
        settings: BakeSettings {
            voxel_size: 0.5,
            agent_radius: 0.0,
            ..BakeSettings::default()
        },
    })
    .unwrap();
    let mut nav = NavWorld::default();
    nav.install(tile);
    nav
}

fn link(nav: &mut NavWorld, id: u64, rise: f32) {
    nav.set_link(OffMeshLink {
        id,
        start: Vec3::new(9.5, 0.0, 5.0),
        end: Vec3::new(12.5, rise, 5.0),
        bidirectional: true,
        radius: 0.5,
        cost: 0.0,
    })
    .unwrap();
}

/// The kinds of the legs crossing links, in order.
fn crossings(nav: &NavWorld, from: Vec3, to: Vec3) -> Vec<(u64, LinkKind)> {
    let path = nav.path(from, to, 0.5).expect("a route");
    path.legs(nav).iter().filter_map(|l| l.link).collect()
}

#[test]
fn an_unset_link_is_crossed_by_its_rise() {
    assert_eq!(
        LinkKind::Auto.resolve(Vec3::ZERO, Vec3::Y * 2.0),
        LinkKind::Climb
    );
    assert_eq!(
        LinkKind::Auto.resolve(Vec3::Y * 2.0, Vec3::ZERO),
        LinkKind::Drop
    );
    assert_eq!(
        LinkKind::Auto.resolve(Vec3::ZERO, Vec3::new(3.0, 0.3, 0.0)),
        LinkKind::Jump
    );
    assert_eq!(
        LinkKind::Swim.resolve(Vec3::ZERO, Vec3::Y * 2.0),
        LinkKind::Swim
    );

    let (low, high) = (Vec3::new(2.0, 0.0, 5.0), Vec3::new(20.0, 2.0, 5.0));
    let mut nav = two_floors(2.0);
    link(&mut nav, 7, 2.0);
    // Up the ledge it is a climb; back down the same link, a drop.
    assert_eq!(crossings(&nav, low, high), [(7, LinkKind::Climb)]);
    assert_eq!(crossings(&nav, high, low), [(7, LinkKind::Drop)]);

    // A level gap is a jump.
    let mut nav = two_floors(0.0);
    link(&mut nav, 8, 0.0);
    let far = Vec3::new(20.0, 0.0, 5.0);
    assert_eq!(crossings(&nav, low, far), [(8, LinkKind::Jump)]);
}

#[test]
fn a_set_kind_wins_survives_a_resync_and_leaves_paths_alone() {
    let (near, far) = (Vec3::new(2.0, 0.0, 5.0), Vec3::new(20.0, 0.0, 5.0));
    let mut nav = two_floors(0.0);
    link(&mut nav, 9, 0.0);
    let before = nav.path(near, far, 0.5).unwrap();
    // The far floor is a pool's bank: in and out by swimming.
    nav.set_link_kind(9, LinkKind::Swim);
    assert_eq!(nav.link_kind(9), LinkKind::Swim);
    assert_eq!(crossings(&nav, near, far), [(9, LinkKind::Swim)]);
    assert_eq!(
        nav.path(near, far, 0.5).unwrap(),
        before,
        "costs do not change"
    );
    // An editor moving the link removes and sets it again: the kind stays.
    nav.remove_link(9);
    link(&mut nav, 9, 0.0);
    assert_eq!(crossings(&nav, near, far), [(9, LinkKind::Swim)]);
    // Auto forgets it.
    nav.set_link_kind(9, LinkKind::Auto);
    assert_eq!(crossings(&nav, near, far), [(9, LinkKind::Jump)]);
}

#[test]
fn legs_follow_the_corners_and_only_link_legs_name_a_link() {
    let mut nav = two_floors(2.0);
    link(&mut nav, 7, 2.0);
    let path = nav
        .path(Vec3::new(2.0, 0.0, 1.0), Vec3::new(20.0, 2.0, 9.0), 0.5)
        .unwrap();
    let legs = path.legs(&nav);
    assert_eq!(legs.len(), path.points.len() - 1);
    for (leg, w) in legs.iter().zip(path.points.windows(2)) {
        assert_eq!((leg.from, leg.to), (w[0], w[1]));
    }
    assert_eq!(legs.iter().filter(|l| l.link.is_some()).count(), 1);
    let crossing = legs.iter().find(|l| l.link.is_some()).unwrap();
    assert!(
        crossing.to.y - crossing.from.y > 1.5,
        "the climbing leg: {crossing:?}"
    );
}

/// What a navmesh costs as it grows: a flat square at the sandbox's 0.25 m
/// voxels, a nearest-point query and a corner-to-corner path. Run with
/// `cargo test --release -p somnium_ai --test links nav_cost -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement"]
fn nav_cost() {
    for side in [20.0f32, 40.0, 80.0] {
        let started = std::time::Instant::now();
        let tile = bake(&BakeInput {
            cell: [0, 0, 0],
            bounds: Bounds {
                min: [0.0, -1.0, 0.0],
                max: [side, 2.0, side],
            },
            triangles: plane(0.0, 0.0, side, side, 0.0),
            obstacles: vec![],
            settings: BakeSettings {
                voxel_size: 0.25,
                agent_radius: 0.35,
                ..BakeSettings::default()
            },
        })
        .unwrap();
        let baked = started.elapsed().as_secs_f64() * 1e3;
        let polygons = tile.polygons().len();
        let mut nav = NavWorld::default();
        let started = std::time::Instant::now();
        nav.install(tile);
        let installed = started.elapsed().as_secs_f64() * 1e3;
        let started = std::time::Instant::now();
        for i in 0..100 {
            let x = (i as f32 * 0.37) % side;
            nav.nearest(Vec3::new(x, 0.0, side - x), 1.0);
        }
        let nearest = started.elapsed().as_secs_f64() * 1e3 / 100.0;
        let started = std::time::Instant::now();
        let path = nav.path(
            Vec3::splat(1.0),
            Vec3::new(side - 1.0, 0.0, side - 1.0),
            1.0,
        );
        let pathed = started.elapsed().as_secs_f64() * 1e3;
        // A chase's replan: five metres across the middle.
        let mid = side * 0.5;
        let started = std::time::Instant::now();
        for _ in 0..20 {
            nav.path(
                Vec3::new(mid, 0.0, mid),
                Vec3::new(mid + 4.0, 0.0, mid + 3.0),
                1.0,
            );
        }
        let short = started.elapsed().as_secs_f64() * 1e3 / 20.0;
        println!(
            "{side} m: {polygons} polygons, bake {baked:.1} ms, install {installed:.1} ms, \
             nearest {nearest:.3} ms, path {pathed:.2} ms ({} points), 5 m path {short:.3} ms",
            path.map_or(0, |p| p.points.len())
        );
    }
}

/// Four 80 m tiles side by side (about 400,000 polygons, as an editor bake
/// over a stretch of open ground might be): a chase's 5 m replan in one corner.
#[test]
#[ignore = "a measurement"]
fn nav_cost_tiled() {
    let mut nav = NavWorld::default();
    for (i, (x, z)) in [(0.0, 0.0), (80.0, 0.0), (0.0, 80.0), (80.0, 80.0)]
        .into_iter()
        .enumerate()
    {
        let tile = bake(&BakeInput {
            cell: [i as i64, 0, 0],
            bounds: Bounds {
                min: [x, -1.0, z],
                max: [x + 80.0, 2.0, z + 80.0],
            },
            triangles: plane(x, z, 80.0, 80.0, 0.0),
            obstacles: vec![],
            settings: BakeSettings {
                voxel_size: 0.25,
                agent_radius: 0.35,
                ..BakeSettings::default()
            },
        })
        .unwrap();
        nav.install(tile);
    }
    let started = std::time::Instant::now();
    for _ in 0..20 {
        nav.path(Vec3::new(40.0, 0.0, 40.0), Vec3::new(44.0, 0.0, 43.0), 1.0);
    }
    let short = started.elapsed().as_secs_f64() * 1e3 / 20.0;
    let polygons: usize = nav
        .cells()
        .filter_map(|c| nav.tile(c))
        .map(|t| t.polygons().len())
        .sum();
    println!("four tiles: {polygons} polygons, 5 m path {short:.3} ms");
}

/// Thousands of nearest-point and path queries over floors with a hole, a
/// ramp and links, hashed to the bit. A change meant only to make queries
/// cheaper must print the same fingerprint before and after.
#[test]
#[ignore = "a fingerprint"]
fn nav_fingerprint() {
    use std::hash::{Hash, Hasher};
    let mut triangles = plane(0.0, 0.0, 30.0, 12.0, 0.0);
    triangles.extend(plane(33.0, 0.0, 14.0, 12.0, 1.0));
    // A ramp up to a deck over the first floor.
    triangles.push([[0.0, 0.0, 14.0], [12.0, 3.0, 14.0], [12.0, 3.0, 18.0]]);
    triangles.push([[0.0, 0.0, 14.0], [12.0, 3.0, 18.0], [0.0, 0.0, 18.0]]);
    triangles.extend(plane(12.0, 14.0, 10.0, 4.0, 3.0));
    let tile = bake(&BakeInput {
        cell: [0, 0, 0],
        bounds: Bounds {
            min: [0.0, -1.0, 0.0],
            max: [47.0, 5.0, 18.0],
        },
        triangles,
        obstacles: vec![Bounds {
            min: [10.0, -1.0, 4.0],
            max: [14.0, 3.0, 8.0],
        }],
        settings: BakeSettings {
            voxel_size: 0.25,
            agent_radius: 0.35,
            ..BakeSettings::default()
        },
    })
    .unwrap();
    let mut nav = NavWorld::default();
    nav.install(tile);
    nav.set_link(OffMeshLink {
        id: 7,
        start: Vec3::new(29.5, 0.0, 6.0),
        end: Vec3::new(33.5, 1.0, 6.0),
        bidirectional: true,
        radius: 0.6,
        cost: 1.0,
    })
    .unwrap();
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f32 / (1u64 << 53) as f32
    };
    let point = |next: &mut dyn FnMut() -> f32| {
        Vec3::new(next() * 52.0 - 2.0, next() * 4.0 - 0.5, next() * 22.0 - 2.0)
    };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let (mut found, mut paths) = (0, 0);
    for i in 0..3000 {
        let a = point(&mut next);
        let reach = [0.0, 0.3, 1.0, 3.0, 40.0][i % 5];
        let near = nav.nearest(a, reach);
        found += usize::from(near.is_some());
        format!("{:?}", near.map(|p| p.to_array().map(f32::to_bits))).hash(&mut h);
        if i % 3 == 0 {
            // Ends near a floor (the first, the second, the deck), snapped
            // from half a metre to three.
            let grounded = |next: &mut dyn FnMut() -> f32| {
                let y = [0.0, 1.0, 3.0][(next() * 2.999) as usize] + next() * 0.4;
                Vec3::new(next() * 50.0 - 1.0, y, next() * 20.0 - 1.0)
            };
            let (a, b) = (grounded(&mut next), grounded(&mut next));
            let path = nav.path(a, b, [0.5, 1.0, 3.0][(i / 3) % 3]);
            paths += usize::from(path.is_some());
            format!(
                "{:?}",
                path.map(|p| (
                    p.points
                        .iter()
                        .map(|v| v.to_array().map(f32::to_bits))
                        .collect::<Vec<_>>(),
                    p.links
                ))
            )
            .hash(&mut h);
        }
    }
    println!(
        "fingerprint {:016x}: {found} nearest found, {paths} paths",
        h.finish()
    );
}
