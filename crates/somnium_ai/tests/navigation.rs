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
