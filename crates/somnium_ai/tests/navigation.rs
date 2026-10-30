use glam::Vec3;
use somnium_ai::navigation::*;
use somnium_jobs::{JobDesc, JobPriority, JobSystem};
fn plane(x: f32, z: f32, w: f32, h: f32, y: f32) -> Vec<Triangle> {
    vec![
        [[x, y, z], [x + w, y, z], [x + w, y, z + h]],
        [[x, y, z], [x + w, y, z + h], [x, y, z + h]],
    ]
}
fn input() -> BakeInput {
    BakeInput {
        cell: [0, 0, 0],
        bounds: Bounds {
            min: [0.0, -1.0, 0.0],
            max: [10.0, 8.0, 10.0],
        },
        triangles: plane(0.0, 0.0, 10.0, 10.0, 0.0),
        obstacles: vec![],
        settings: BakeSettings {
            voxel_size: 1.0,
            agent_radius: 0.0,
            ..BakeSettings::default()
        },
    }
}
#[test]
fn baked_obstacle_detour_has_clear_smoothed_segments() {
    let mut data = input();
    data.obstacles.push(Bounds {
        min: [4.0, -0.5, 2.0],
        max: [6.0, 3.0, 8.0],
    });
    let tile = bake(&data).unwrap();
    assert!(!tile.contours().is_empty());
    assert_eq!(tile.triangles().count(), tile.polygons().len() * 2);
    assert_eq!(NavTile::decode(&tile.encode().unwrap()).unwrap(), tile);
    let mut nav = NavWorld::default();
    nav.install(tile);
    let from = Vec3::new(1.0, 0.0, 5.0);
    let to = Vec3::new(9.0, 0.0, 5.0);
    assert!(nav.raycast(from, to, 0.1).is_some());
    let path = nav.path(from, to, 0.1).unwrap();
    assert!(
        path.points.len() >= 3,
        "detour must bend around obstacle: {:?}",
        path.points
    );
    assert!(path.points.iter().any(|p| p.z <= 2.0 || p.z >= 8.0));
    for segment in path.points.windows(2) {
        // Corners on a wall are valid funnel points. Offset the endpoints a
        // tiny amount into the segment so ownership is unambiguous.
        let a = segment[0].lerp(segment[1], 0.0001);
        let b = segment[1].lerp(segment[0], 0.0001);
        assert!(
            nav.raycast(a, b, 0.1).is_none(),
            "smoothed segment crosses a wall: {a:?} -> {b:?}; path {:?}",
            path.points
        );
    }
}
#[test]
fn jobs_are_deterministic_and_tile_replacement_preserves_neighbors() {
    let mut jobs = JobSystem::single_threaded();
    let data = input();
    let handle = submit_bake(
        &mut jobs,
        data.clone(),
        JobDesc::new("navigation.test").priority(JobPriority::Visible),
    )
    .unwrap();
    let tile = handle.try_take().unwrap().unwrap();
    assert_eq!(tile, bake(&data).unwrap());
    let mut nav = NavWorld::default();
    nav.install(tile);
    let mut next = data.clone();
    next.cell = [1, 0, 0];
    next.bounds.min[0] = 10.0;
    next.bounds.max[0] = 20.0;
    next.triangles = plane(10.0, 0.0, 10.0, 10.0, 0.0);
    nav.install(bake(&next).unwrap());
    assert!(
        nav.path(Vec3::new(1.0, 0.0, 5.0), Vec3::new(19.0, 0.0, 5.0), 0.1)
            .is_some()
    );
    let neighbor_bytes = nav.tile([1, 0, 0]).unwrap().encode().unwrap();
    let obstacle = Bounds {
        min: [2.0, -0.5, 2.0],
        max: [4.0, 3.0, 4.0],
    };
    assert_eq!(nav.affected_cells(obstacle), vec![[0, 0, 0]]);
    let mut changed = data;
    changed.obstacles.push(obstacle);
    nav.install(bake(&changed).unwrap());
    assert_eq!(
        nav.tile([1, 0, 0]).unwrap().encode().unwrap(),
        neighbor_bytes
    );
    nav.unload([1, 0, 0]);
    assert!(
        nav.path(Vec3::new(1.0, 0.0, 5.0), Vec3::new(19.0, 0.0, 5.0), 0.1)
            .is_none()
    );
}
#[test]
fn stacked_floors_remain_disconnected_until_a_directional_link_is_added() {
    let mut data = input();
    data.triangles.extend(plane(0.0, 0.0, 10.0, 10.0, 4.0));
    let mut nav = NavWorld::default();
    nav.install(bake(&data).unwrap());
    let start = Vec3::new(1.0, 0.0, 1.0);
    let end = Vec3::new(1.0, 4.0, 1.0);
    assert_eq!(nav.nearest(end + Vec3::Y * 0.1, 0.2), Some(end));
    assert!(nav.path(start, end, 0.1).is_none());
    nav.set_link(OffMeshLink {
        id: 4,
        start,
        end,
        bidirectional: false,
        radius: 0.1,
        cost: 1.0,
    })
    .unwrap();
    let path = nav.path(start, end, 0.1).unwrap();
    assert_eq!(path.links, vec![4]);
    assert!(path.points.contains(&end));
    assert!(nav.path(end, start, 0.1).is_none());
}
#[test]
fn clearance_radius_vertical_walls_and_invalid_inputs_are_enforced() {
    let mut data = input();
    data.triangles.extend(plane(0.0, 0.0, 10.0, 10.0, 1.0));
    let tile = bake(&data).unwrap();
    assert!(tile.polygons().iter().all(|p| p.vertices[0][1] >= 1.0));
    let mut data = input();
    data.triangles.extend([
        [[5.0, 0.0, 0.0], [5.0, 3.0, 0.0], [5.0, 3.0, 10.0]],
        [[5.0, 0.0, 0.0], [5.0, 3.0, 10.0], [5.0, 0.0, 10.0]],
    ]);
    data.settings.agent_radius = 0.2;
    let mut nav = NavWorld::default();
    nav.install(bake(&data).unwrap());
    assert!(
        nav.path(Vec3::new(2.0, 0.0, 5.0), Vec3::new(8.0, 0.0, 5.0), 0.1)
            .is_none()
    );
    data.settings.voxel_size = 0.0;
    assert!(bake(&data).is_err());
    assert!(NavTile::decode(b"{}").is_err());
    assert!(nav.nearest(Vec3::NAN, 1.0).is_none());
}
#[test]
fn steering_slows_at_arrival_and_separates_neighbors() {
    let slow = steer(Vec3::ZERO, Vec3::X * 0.1, 3.0, 0.5, &[]);
    assert!((slow.length() - 0.1).abs() < 0.001);
    let avoid = steer(
        Vec3::ZERO,
        Vec3::X * 10.0,
        3.0,
        0.5,
        &[Neighbor {
            position: Vec3::X * 0.3,
            velocity: Vec3::ZERO,
            radius: 0.5,
        }],
    );
    assert!(avoid.length() < 3.0);
    assert!(avoid.is_finite());
}

#[test]
fn agent_follows_detour_and_replans_after_tile_unload() {
    let mut data = input();
    data.obstacles.push(Bounds {
        min: [4.0, -0.5, 2.0],
        max: [6.0, 3.0, 8.0],
    });
    let mut nav = NavWorld::default();
    nav.install(bake(&data).unwrap());
    let mut agent = NavAgent::new(3.0, 0.1).unwrap();
    let mut position = Vec3::new(1.0, 0.0, 5.0);
    let target = Vec3::new(9.0, 0.0, 5.0);
    assert!(agent.set_destination(&nav, position, target));
    for _ in 0..2400 {
        let movement = agent.update(&nav, position, 1.0 / 60.0, &[]);
        position += movement.velocity / 60.0;
        if movement.arrived {
            break;
        }
    }
    assert!(
        position.distance(target) < 0.06,
        "agent did not reach destination: {position:?}"
    );
    nav.unload([0, 0, 0]);
    let movement = agent.update(&nav, position, 0.1, &[]);
    assert_eq!(movement.velocity, Vec3::ZERO);
    assert!(!movement.arrived);
}
#[test]
fn a_wall_at_an_angle_blocks_only_its_own_footprint() {
    // A wall on the diagonal from (1, 1) to (4, 4): its bounding box covers the ground either side of it.
    let mut data = input();
    data.triangles.extend([
        [[1.0, 0.0, 1.0], [4.0, 0.0, 4.0], [4.0, 3.0, 4.0]],
        [[1.0, 0.0, 1.0], [4.0, 3.0, 4.0], [1.0, 3.0, 1.0]],
    ]);
    data.settings.agent_radius = 0.2;
    let mut nav = NavWorld::default();
    nav.install(bake(&data).unwrap());
    let (a, b) = (Vec3::new(1.5, 0.0, 3.5), Vec3::new(3.5, 0.0, 1.5));
    let path = nav.path(a, b, 0.1).expect("both sides of the wall are open ground");
    let length: f32 = path.points.windows(2).map(|w| w[0].distance(w[1])).sum();
    assert!(length > a.distance(b) + 1.0, "the way goes round the wall's end, not through it: {length}");
}
#[test]
fn a_kerb_lower_than_a_step_is_walked_up() {
    // A pavement 0.15 m high from x = 5: its riser is steep but lower than the step height.
    let mut data = input();
    data.triangles = plane(0.0, 0.0, 5.0, 10.0, 0.0);
    data.triangles.extend(plane(5.0, 0.0, 5.0, 10.0, 0.15));
    data.triangles.extend([
        [[5.0, 0.0, 0.0], [5.0, 0.15, 0.0], [5.0, 0.15, 10.0]],
        [[5.0, 0.0, 0.0], [5.0, 0.15, 10.0], [5.0, 0.0, 10.0]],
    ]);
    data.settings.agent_radius = 0.2;
    let mut nav = NavWorld::default();
    nav.install(bake(&data).unwrap());
    assert!(nav.path(Vec3::new(2.0, 0.0, 5.0), Vec3::new(8.0, 0.15, 5.0), 0.1).is_some());
}
fn cuboid(min: [f32; 3], max: [f32; 3]) -> Vec<Triangle> {
    let c = |i: usize| [[min[0], max[0]][i & 1], [min[1], max[1]][(i >> 1) & 1], [min[2], max[2]][(i >> 2) & 1]];
    [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]]
        .iter()
        .flat_map(|f| [[c(f[0]), c(f[1]), c(f[2])], [c(f[0]), c(f[2]), c(f[3])]])
        .collect()
}
#[test]
fn a_flight_of_stairs_between_walls_is_climbed() {
    // Nine 0.3 m treads with 0.167 m risers up to a landing 1.5 m high, each tread a box down below the ground (as a
    // level's collision builds them), between side walls 0.9 m above the treads. The riser after next is more than a
    // step above a tread, but only a step above the tread before it.
    let mut data = input();
    data.triangles = plane(0.0, 0.0, 10.0, 3.0, 0.0);
    for k in 0..9 {
        let (z, top) = (3.0 + 0.3 * k as f32, 1.5 * (k + 1) as f32 / 9.0);
        data.triangles.extend(cuboid([3.0, -0.2, z], [6.0, top, z + 0.3]));
        data.triangles.extend(cuboid([2.7, -0.2, z], [3.0, top + 0.9, z + 0.3]));
        data.triangles.extend(cuboid([6.0, -0.2, z], [6.3, top + 0.9, z + 0.3]));
    }
    data.triangles.extend(cuboid([0.0, -0.2, 5.7], [10.0, 1.5, 10.0]));
    for (voxel_size, agent_radius, max_step) in [(0.25, 0.3, 0.3), (0.5, 0.3, 0.4)] {
        data.settings = BakeSettings { voxel_size, agent_radius, max_step, max_slope_degrees: 40.0, ..BakeSettings::default() };
        let mut nav = NavWorld::default();
        nav.install(bake(&data).unwrap());
        let (foot, top) = (Vec3::new(4.5, 0.0, 1.0), Vec3::new(4.5, 1.5, 8.0));
        assert!(nav.path(foot, top, 0.2).is_some(), "up the flight at {voxel_size} m voxels");
        assert!(nav.path(top, foot, 0.2).is_some(), "down the flight at {voxel_size} m voxels");
        assert!(nav.nearest(Vec3::new(3.2, 0.5, 3.75), 0.1).is_none(), "the side wall stays a wall");
    }
}
