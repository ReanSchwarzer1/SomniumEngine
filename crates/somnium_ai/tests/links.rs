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
