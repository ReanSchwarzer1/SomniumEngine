//! Navigation tiles from ground heights: open land without a triangle bake.
use glam::Vec3;
use somnium_ai::navigation::*;

const CELL: f32 = 2.0;

fn settings() -> BakeSettings {
    BakeSettings {
        voxel_size: CELL,
        agent_radius: 0.35,
        ..BakeSettings::default()
    }
}

fn tile(cell: i64, min: [f32; 2], side: f32, height: impl Fn(f32, f32) -> Option<f32>) -> NavTile {
    heightfield(
        [cell, 0, 0],
        Bounds {
            min: [min[0], -50.0, min[1]],
            max: [min[0] + side, 50.0, min[1] + side],
        },
        settings(),
        height,
    )
    .unwrap()
}

/// Two tiles side by side on an 11 degree hill join into one walk up it.
#[test]
fn two_tiles_on_a_hill_join() {
    let hill = |x: f32, _z: f32| Some(0.2 * x);
    let mut nav = NavWorld::default();
    nav.install(tile(0, [0.0, 0.0], 32.0, hill));
    nav.install(tile(1, [32.0, 0.0], 32.0, hill));
    let path = nav
        .path(Vec3::new(2.0, 0.4, 16.0), Vec3::new(62.0, 12.4, 16.0), 2.0)
        .expect("up the hill, across the seam");
    let end = *path.points.last().unwrap();
    assert!(end.distance(Vec3::new(62.0, 12.4, 16.0)) < 1.5, "{end}");
}

/// A cliff and a house (where a ray down meets the roof) are walked round,
/// and a cliff right across leaves no way at all.
#[test]
fn a_cliff_and_a_house_are_walked_round() {
    let house = |x: f32, z: f32| (20.0..28.0).contains(&x) && (8.0..24.0).contains(&z);
    let ground = move |x: f32, z: f32| Some(if house(x, z) { 6.0 } else { 0.0 });
    let mut nav = NavWorld::default();
    nav.install(tile(0, [0.0, 0.0], 48.0, ground));
    let path = nav
        .path(Vec3::new(10.0, 0.0, 16.0), Vec3::new(40.0, 0.0, 16.0), 1.0)
        .expect("round the house");
    // Round it, not through it (its corners and walls are touched: the
    // controller slides along them).
    let inside = |p: &Vec3| p.x > 20.0 && p.x < 28.0 && p.z > 8.0 && p.z < 24.0;
    assert!(
        path.points.iter().all(|p| !inside(p) && p.y.abs() < 0.01),
        "{:?}",
        path.points
    );
    assert!(path.points.len() > 2, "it turns: {:?}", path.points);
    let wall = |x: f32, _z: f32| Some(if x < 24.0 { 0.0 } else { 3.0 });
    let mut cut = NavWorld::default();
    cut.install(tile(0, [0.0, 0.0], 48.0, wall));
    assert!(
        cut.path(Vec3::new(10.0, 0.0, 16.0), Vec3::new(40.0, 3.0, 16.0), 1.0)
            .is_none(),
        "a 3 m cliff is not walked up"
    );
}

/// No ground in a pond: the way goes round it.
#[test]
fn a_pond_is_no_ground() {
    let pond = |x: f32, z: f32| (Vec3::new(x, 0.0, z) - Vec3::new(24.0, 0.0, 24.0)).length() < 9.0;
    let mut nav = NavWorld::default();
    nav.install(tile(0, [0.0, 0.0], 48.0, move |x, z| {
        (!pond(x, z)).then_some(0.0)
    }));
    let path = nav
        .path(Vec3::new(8.0, 0.0, 24.0), Vec3::new(40.0, 0.0, 24.0), 1.0)
        .expect("round the pond");
    assert!(
        path.points.iter().all(|p| !pond(p.x, p.z)),
        "{:?}",
        path.points
    );
    assert!(nav.nearest(Vec3::new(24.0, 0.0, 24.0), 1.0).is_none());
}

/// Build and install costs for a square kilometre of rolling land as
/// sixteen 256 m tiles, at 2 m and 1 m cells. Run with
/// `cargo test --release -p somnium_ai --test heightfield km_cost -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement"]
fn km_cost() {
    let land = |x: f32, z: f32| Some(4.0 * (x * 0.013).sin() * (z * 0.011).cos() + 0.002 * x);
    for cell in [2.0f32, 1.0] {
        let mut nav = NavWorld::default();
        let (mut build, mut install, mut last_install, mut polygons) = (0.0, 0.0, 0.0, 0);
        for i in 0..16 {
            let min = [(i % 4) as f32 * 256.0, (i / 4) as f32 * 256.0];
            let started = std::time::Instant::now();
            let t = heightfield(
                [i, 0, 0],
                Bounds {
                    min: [min[0], -50.0, min[1]],
                    max: [min[0] + 256.0, 50.0, min[1] + 256.0],
                },
                BakeSettings {
                    voxel_size: cell,
                    ..settings()
                },
                land,
            )
            .unwrap();
            build += started.elapsed().as_secs_f64() * 1e3;
            polygons += t.polygons().len();
            let started = std::time::Instant::now();
            if std::env::var("NAV_INSTALL").is_ok() {
                nav.install(t);
            } else {
                nav.stream_in(t);
            }
            last_install = started.elapsed().as_secs_f64() * 1e3;
            install += last_install;
        }
        let started = std::time::Instant::now();
        let path = nav.path(
            Vec3::new(10.0, 0.0, 10.0),
            Vec3::new(1000.0, 0.0, 1000.0),
            5.0,
        );
        let across = started.elapsed().as_secs_f64() * 1e3;
        let started = std::time::Instant::now();
        for _ in 0..20 {
            nav.path(
                Vec3::new(500.0, 0.0, 500.0),
                Vec3::new(530.0, 0.0, 520.0),
                5.0,
            );
        }
        let near = started.elapsed().as_secs_f64() * 1e3 / 20.0;
        let bytes = polygons * std::mem::size_of::<NavPolygon>();
        let started = std::time::Instant::now();
        nav.stream_out([5, 0, 0]);
        let out = started.elapsed().as_secs_f64() * 1e3;
        println!(
            "{cell} m cells: {polygons} polygons a km2 ({:.1} MB of polygons in the tiles), build {build:.1} ms, \
             install {install:.1} ms in all (the 16th {last_install:.1} ms), 1.4 km path {across:.1} ms ({}), \
             36 m path {near:.3} ms, one tile out {out:.1} ms",
            bytes as f64 / 1e6,
            if path.is_some() { "found" } else { "none" }
        );
    }
}
