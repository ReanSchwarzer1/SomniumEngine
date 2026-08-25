//! MORROWIND-U tests for the pose seam.

use super::*;

/// A three-joint chain given to `Skeleton::new` in the *wrong* order — child
/// before parent — which is what a glTF file is under no obligation not to do.
fn out_of_order_chain() -> (Skeleton, Vec<JointIndex>) {
    // Authored order: 0 = hand (child of 1), 1 = forearm (child of 2), 2 = arm.
    Skeleton::new(
        SkeletonId(1),
        vec!["hand".into(), "forearm".into(), "arm".into()],
        vec![1, 2, NO_PARENT],
        vec![Mat4::IDENTITY; 3],
        vec![
            Transform {
                translation: Vec3::new(1.0, 0.0, 0.0),
                ..Transform::IDENTITY
            },
            Transform {
                translation: Vec3::new(1.0, 0.0, 0.0),
                ..Transform::IDENTITY
            },
            Transform::IDENTITY,
        ],
    )
    .expect("a valid chain")
}

// ── the invariant ───────────────────────────────────────────────────────────

#[test]
fn a_skeleton_is_reordered_so_parents_come_first() {
    let (skeleton, remap) = out_of_order_chain();
    assert!(skeleton.is_well_formed());
    assert_eq!(skeleton.names(), ["arm", "forearm", "hand"]);
    // The remap is what a caller needs to fix its vertex joint indices.
    assert_eq!(remap, vec![2, 1, 0]);
    assert_eq!(skeleton.parents(), [NO_PARENT, 0, 1]);
}

#[test]
fn a_skeleton_already_in_order_is_left_alone() {
    let (skeleton, remap) = Skeleton::new(
        SkeletonId(0),
        vec!["root".into(), "a".into(), "b".into()],
        vec![NO_PARENT, 0, 0],
        vec![Mat4::IDENTITY; 3],
        vec![Transform::IDENTITY; 3],
    )
    .expect("valid");
    assert_eq!(remap, vec![0, 1, 2], "a stable order changed");
    assert_eq!(skeleton.names(), ["root", "a", "b"]);
}

#[test]
fn siblings_keep_their_authored_order() {
    // Three children of one root. A skeleton whose joint order changes between
    // runs would invalidate every cooked clip that indexes into it.
    let (skeleton, _) = Skeleton::new(
        SkeletonId(0),
        vec![
            "root".into(),
            "first".into(),
            "second".into(),
            "third".into(),
        ],
        vec![NO_PARENT, 0, 0, 0],
        vec![Mat4::IDENTITY; 4],
        vec![Transform::IDENTITY; 4],
    )
    .expect("valid");
    assert_eq!(skeleton.names(), ["root", "first", "second", "third"]);
}

#[test]
fn a_cycle_is_rejected_rather_than_looping_forever() {
    assert!(
        Skeleton::new(
            SkeletonId(0),
            vec!["a".into(), "b".into()],
            vec![1, 0],
            vec![Mat4::IDENTITY; 2],
            vec![Transform::IDENTITY; 2],
        )
        .is_none()
    );
}

#[test]
fn a_joint_parented_to_itself_or_to_nothing_is_rejected() {
    let self_parented = Skeleton::new(
        SkeletonId(0),
        vec!["a".into()],
        vec![0],
        vec![Mat4::IDENTITY],
        vec![Transform::IDENTITY],
    );
    assert!(self_parented.is_none());

    let out_of_range = Skeleton::new(
        SkeletonId(0),
        vec!["a".into(), "b".into()],
        vec![NO_PARENT, 7],
        vec![Mat4::IDENTITY; 2],
        vec![Transform::IDENTITY; 2],
    );
    assert!(out_of_range.is_none());
}

#[test]
fn mismatched_array_lengths_are_rejected() {
    assert!(
        Skeleton::new(
            SkeletonId(0),
            vec!["a".into(), "b".into()],
            vec![NO_PARENT],
            vec![Mat4::IDENTITY; 2],
            vec![Transform::IDENTITY; 2],
        )
        .is_none()
    );
}

// ── composition ─────────────────────────────────────────────────────────────

#[test]
fn a_chain_accumulates_down_the_hierarchy() {
    let (skeleton, _) = out_of_order_chain();
    let pose = skeleton.rest_pose();
    let mut out = vec![Mat4::ZERO; skeleton.len()];
    assert!(pose.to_model_space(&skeleton, &mut out));

    // arm at the origin, forearm 1 along x, hand 2 along x.
    let at = |m: Mat4| m.transform_point3(Vec3::ZERO);
    assert!(
        (at(out[0]) - Vec3::ZERO).length() < 1e-5,
        "{:?}",
        at(out[0])
    );
    assert!((at(out[1]) - Vec3::X).length() < 1e-5, "{:?}", at(out[1]));
    assert!(
        (at(out[2]) - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-5,
        "{:?}",
        at(out[2])
    );
}

#[test]
fn rotating_a_parent_carries_its_children() {
    let (skeleton, _) = out_of_order_chain();
    let mut pose = skeleton.rest_pose();
    // Turn the arm 90° about z: the hand should swing from +2x to +2y.
    pose.local[0].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);

    let mut out = vec![Mat4::ZERO; skeleton.len()];
    assert!(pose.to_model_space(&skeleton, &mut out));
    let hand = out[2].transform_point3(Vec3::ZERO);
    assert!(
        (hand - Vec3::new(0.0, 2.0, 0.0)).length() < 1e-4,
        "the hand did not follow the arm: {hand:?}"
    );
}

/// The palette is what actually crosses into the renderer, and the identity it
/// must satisfy is that a rest pose moves nothing.
#[test]
fn a_rest_pose_produces_an_identity_palette() {
    let count = 3;
    let rest = vec![
        Transform {
            translation: Vec3::new(0.0, 1.0, 0.0),
            rotation: Quat::from_rotation_x(0.3),
            scale: Vec3::ONE,
        };
        count
    ];
    // Inverse bind is, by definition, the inverse of the rest model-space
    // matrix. Build it that way so the test is asserting the composition rather
    // than a coincidence of identity matrices.
    let parents = vec![NO_PARENT, 0, 1];
    let mut model = vec![Mat4::IDENTITY; count];
    for i in 0..count {
        let local = rest[i].to_matrix();
        model[i] = if parents[i] == NO_PARENT {
            local
        } else {
            model[parents[i] as usize] * local
        };
    }
    let inverse_bind: Vec<Mat4> = model.iter().map(|m| m.inverse()).collect();

    let (skeleton, _) = Skeleton::new(
        SkeletonId(0),
        vec!["a".into(), "b".into(), "c".into()],
        parents,
        inverse_bind,
        rest,
    )
    .expect("valid");

    let mut palette = vec![Mat4::ZERO; count];
    assert!(skeleton.rest_pose().to_palette(&skeleton, &mut palette));
    for (index, matrix) in palette.iter().enumerate() {
        let delta = *matrix - Mat4::IDENTITY;
        let worst = (0..4)
            .flat_map(|c| (0..4).map(move |r| (c, r)))
            .map(|(c, r)| delta.col(c)[r].abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "joint {index} moved at rest: {matrix:?}");
    }
}

#[test]
fn a_pose_for_the_wrong_skeleton_is_refused_rather_than_folded_inside_out() {
    let (skeleton, _) = out_of_order_chain();
    let mut wrong = skeleton.rest_pose();
    wrong.skeleton = SkeletonId(99);
    let mut out = vec![Mat4::ZERO; skeleton.len()];
    assert!(!wrong.to_model_space(&skeleton, &mut out));
    assert_eq!(out, vec![Mat4::ZERO; skeleton.len()], "it wrote anyway");
}

#[test]
fn a_short_output_buffer_is_refused_and_left_untouched() {
    let (skeleton, _) = out_of_order_chain();
    let mut out = vec![Mat4::ZERO; skeleton.len() - 1];
    assert!(!skeleton.rest_pose().to_model_space(&skeleton, &mut out));
    assert!(out.iter().all(|m| *m == Mat4::ZERO));
}

// ── blending ────────────────────────────────────────────────────────────────

#[test]
fn blending_endpoints_are_exact() {
    let a = Transform {
        translation: Vec3::X,
        rotation: Quat::from_rotation_z(0.5),
        scale: Vec3::splat(2.0),
    };
    let b = Transform::IDENTITY;
    assert_eq!(a.blend(b, 0.0), a);
    let at_one = a.blend(b, 1.0);
    assert!((at_one.translation - b.translation).length() < 1e-6);
    assert!((at_one.scale - b.scale).length() < 1e-6);
    assert!(at_one.rotation.abs_diff_eq(b.rotation, 1e-6));
}

/// The reason rotation slerps rather than lerps, stated as a measurement: a
/// componentwise lerp of two quaternions 90° apart lands short of half way.
#[test]
fn rotation_blends_at_a_constant_rate_which_a_lerp_would_not() {
    let a = Transform::IDENTITY;
    let b = Transform {
        rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        ..Transform::IDENTITY
    };
    let half = a.blend(b, 0.5).rotation;
    let angle = half.to_axis_angle().1;
    assert!(
        (angle - std::f32::consts::FRAC_PI_4).abs() < 1e-4,
        "half way through a 90° turn should be 45°, got {}°",
        angle.to_degrees()
    );

    // What a lerp would have given, for contrast.
    let lerped = Quat::from_xyzw(
        (a.rotation.x + b.rotation.x) * 0.5,
        (a.rotation.y + b.rotation.y) * 0.5,
        (a.rotation.z + b.rotation.z) * 0.5,
        (a.rotation.w + b.rotation.w) * 0.5,
    )
    .normalize();
    // A lerp of these two happens to land on 45° as well — the two agree at the
    // midpoint of a 90° arc. The difference is at the quarter point.
    let quarter_slerp = a.blend(b, 0.25).rotation.to_axis_angle().1;
    let quarter_lerp = Quat::from_xyzw(
        a.rotation.x * 0.75 + b.rotation.x * 0.25,
        a.rotation.y * 0.75 + b.rotation.y * 0.25,
        a.rotation.z * 0.75 + b.rotation.z * 0.25,
        a.rotation.w * 0.75 + b.rotation.w * 0.25,
    )
    .normalize()
    .to_axis_angle()
    .1;
    assert!(
        (quarter_slerp - quarter_lerp).abs() > 1e-3,
        "slerp and lerp agreed at the quarter point, so this test proves nothing"
    );
    let _ = lerped;
}

#[test]
fn blending_a_whole_pose_refuses_a_mismatched_skeleton() {
    let (skeleton, _) = out_of_order_chain();
    let a = skeleton.rest_pose();
    let mut b = skeleton.rest_pose();
    b.skeleton = SkeletonId(7);
    let mut out = skeleton.rest_pose();
    assert!(!a.blend_into(&b, 0.5, &mut out));
}

#[test]
fn blending_a_whole_pose_blends_every_joint() {
    let (skeleton, _) = out_of_order_chain();
    let a = skeleton.rest_pose();
    let mut b = skeleton.rest_pose();
    for joint in &mut b.local {
        joint.translation += Vec3::Y * 2.0;
    }
    let mut out = skeleton.rest_pose();
    assert!(a.blend_into(&b, 0.5, &mut out));
    for (index, joint) in out.local.iter().enumerate() {
        assert!(
            (joint.translation.y - (a.local[index].translation.y + 1.0)).abs() < 1e-5,
            "joint {index} did not blend"
        );
    }
}

// ── skin binding ────────────────────────────────────────────────────────────

#[test]
fn influences_are_kept_heaviest_first_and_renormalised() {
    let binding = SkinBinding::from_influences(&[(3, 0.1), (1, 0.5), (7, 0.2), (2, 0.05)]);
    assert_eq!(binding.joints, [1, 7, 3, 2]);
    assert!(binding.is_normalised(), "{:?}", binding.weights);
    assert!((binding.weights[0] - 0.5 / 0.85).abs() < 1e-5);
}

/// The bug this guards: dropping the fifth influence without renormalising
/// shrinks the vertex toward the origin by exactly the weight that was dropped.
#[test]
fn a_fifth_influence_is_dropped_and_the_rest_still_sum_to_one() {
    let binding =
        SkinBinding::from_influences(&[(0, 0.3), (1, 0.3), (2, 0.2), (3, 0.15), (4, 0.05)]);
    assert!(!binding.joints.contains(&4), "the lightest was kept");
    assert!(binding.is_normalised(), "sum was {:?}", binding.weights);
}

#[test]
fn a_vertex_with_no_influences_binds_to_joint_zero_rather_than_collapsing() {
    let binding = SkinBinding::from_influences(&[]);
    assert_eq!(binding, SkinBinding::UNSKINNED);
    assert!(binding.is_normalised());
    // Zero weights would put the vertex at the origin, which reads as a mesh
    // with a spike through it.
    assert!(binding.weights.iter().sum::<f32>() > 0.0);
}

#[test]
fn zero_weight_influences_do_not_take_a_slot() {
    let binding = SkinBinding::from_influences(&[(9, 0.0), (1, 1.0), (8, 0.0)]);
    assert_eq!(binding.joints[0], 1);
    assert_eq!(binding.max_joint(), 1, "a zero-weight joint was counted");
}

#[test]
fn the_influence_order_is_deterministic_for_equal_weights() {
    let first = SkinBinding::from_influences(&[(5, 0.5), (2, 0.5)]);
    let second = SkinBinding::from_influences(&[(2, 0.5), (5, 0.5)]);
    assert_eq!(
        first.joints, second.joints,
        "a cooked asset would differ between runs"
    );
    assert_eq!(first.joints[0], 2, "ties should break on the lower index");
}

#[test]
fn a_skin_naming_a_joint_the_skeleton_does_not_have_is_refused() {
    let (skeleton, _) = out_of_order_chain();
    let good = Skin {
        skeleton: skeleton.id(),
        bindings: vec![SkinBinding::from_influences(&[(2, 1.0)])],
    };
    assert!(good.fits(&skeleton));

    // Joint 3 in a three-joint skeleton reads past the palette on the GPU.
    let bad = Skin {
        skeleton: skeleton.id(),
        bindings: vec![SkinBinding::from_influences(&[(3, 1.0)])],
    };
    assert!(!bad.fits(&skeleton));

    let wrong_skeleton = Skin {
        skeleton: SkeletonId(42),
        bindings: vec![SkinBinding::UNSKINNED],
    };
    assert!(!wrong_skeleton.fits(&skeleton));
}

#[test]
fn joints_can_be_found_by_name() {
    let (skeleton, _) = out_of_order_chain();
    assert_eq!(skeleton.find("hand"), Some(2));
    assert_eq!(skeleton.find("arm"), Some(0));
    assert_eq!(skeleton.find("tail"), None);
}
