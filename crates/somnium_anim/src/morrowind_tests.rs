use super::*;
use glam::{Mat4, Quat, Vec3};

fn chain() -> Skeleton {
    Skeleton::new(
        SkeletonId(9),
        vec!["hip".into(), "knee".into(), "foot".into()],
        vec![NO_PARENT, 0, 1],
        vec![Mat4::IDENTITY; 3],
        vec![
            Transform::IDENTITY,
            Transform {
                translation: Vec3::Y,
                ..Transform::IDENTITY
            },
            Transform {
                translation: Vec3::Y,
                ..Transform::IDENTITY
            },
        ],
    )
    .unwrap()
    .0
}
fn walk(s: &Skeleton, rotation: Quat) -> AnimationClip {
    AnimationClip::new(
        ClipId(1),
        s,
        1.0,
        vec![TransformTrack {
            joint: 0,
            translation: vec![Keyframe::new(0.0, Vec3::ZERO), Keyframe::new(1.0, Vec3::X)],
            rotation: vec![
                Keyframe::new(0.0, Quat::IDENTITY),
                Keyframe::new(1.0, rotation),
            ],
            scale: vec![],
        }],
        vec![],
    )
    .unwrap()
}
#[test]
fn root_motion_accounts_for_many_loops_reverse_and_rotating_cycles() {
    let s = chain();
    let clip = walk(&s, Quat::IDENTITY);
    let (pose, delta) = clip
        .sample_root_motion(&s, 0, 0.75, 3.25, Playback::LOOPING)
        .unwrap();
    assert!(delta.translation.abs_diff_eq(Vec3::X * 2.5, 1e-5));
    assert_eq!(pose.local[0].translation, Vec3::ZERO);
    let (_, backwards) = clip
        .sample_root_motion(&s, 0, 3.25, 0.75, Playback::LOOPING)
        .unwrap();
    assert!(backwards.translation.abs_diff_eq(-delta.translation, 1e-5));
    let (_, clamped) = clip
        .sample_root_motion(&s, 0, 0.75, 3.25, Playback::ONCE)
        .unwrap();
    assert!(clamped.translation.abs_diff_eq(Vec3::X * 0.25, 1e-5));
    let clip = walk(&s, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
    let (_, turn) = clip
        .sample_root_motion(&s, 0, 0.0, 2.0, Playback::LOOPING)
        .unwrap();
    assert!(turn.translation.abs_diff_eq(Vec3::X + Vec3::Y, 1e-5));
    assert!(
        clip.sample_root_motion(&s, 1, 0.0, 1.0, Playback::LOOPING)
            .is_err()
    );
    assert!(
        clip.sample_root_motion(&s, 0, f32::NAN, 1.0, Playback::LOOPING)
            .is_err()
    );
}
#[test]
fn events_are_partition_invariant_ordered_and_bounded() {
    let events = EventTrack::new(
        1.0,
        [0.0, 0.25, 0.75]
            .into_iter()
            .map(|time| AnimationEvent {
                time,
                name: format!("foot-{time}"),
                payload: "left".into(),
            })
            .collect(),
    )
    .unwrap();
    let all = events.sample(0.75, 3.25, Playback::LOOPING, 8).unwrap();
    assert_eq!(all.len(), 8);
    let mut split = events.sample(0.75, 2.0, Playback::LOOPING, 8).unwrap();
    split.extend(events.sample(2.0, 3.25, Playback::LOOPING, 8).unwrap());
    assert_eq!(all, split);
    assert_eq!(
        events.sample(0.75, 3.25, Playback::LOOPING, 7),
        Err(MotionError::EventBudgetExceeded)
    );
    let reverse = events.sample(3.25, 0.75, Playback::LOOPING, 8).unwrap();
    assert_eq!(reverse.len(), 8);
    assert_eq!(reverse.last().unwrap().playback_time, 0.75);
    assert!(
        reverse
            .windows(2)
            .all(|w| w[0].playback_time >= w[1].playback_time)
    );
    assert_eq!(
        events.sample(0.25, 3.0, Playback::ONCE, 10).unwrap().len(),
        1
    );
    assert!(
        events
            .sample(0.0, 1.0, Playback::new(true, 0.0).unwrap(), 0)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn collision_slides_along_wall_without_motion_debt() {
    let sweep = |position: Vec3, displacement: Vec3| {
        if displacement.x > 1e-6 && position.x + displacement.x >= 1.0 {
            Some(MotionHit {
                fraction: (1.0 - position.x) / displacement.x,
                normal: -Vec3::X,
            })
        } else {
            None
        }
    };
    let result = collide_and_slide(Vec3::ZERO, Vec3::new(2.0, 0.0, 3.0), 0.0, sweep).unwrap();
    assert!(result.applied.abs_diff_eq(Vec3::new(1.0, 0.0, 3.0), 1e-5));
    let next = collide_and_slide(result.applied, Vec3::Z, 0.0, sweep).unwrap();
    assert_eq!(next.applied, Vec3::Z);
    assert!(
        collide_and_slide(Vec3::ZERO, Vec3::X, 0.0, |_, _| Some(MotionHit {
            fraction: f32::NAN,
            normal: Vec3::X
        }))
        .is_err()
    );
}
#[test]
fn limb_ik_preserves_lengths_and_reaches_or_clamps() {
    let s = chain();
    let ik = TwoBoneIk {
        root: 0,
        middle: 1,
        end: 2,
    };
    let mut pose = s.rest_pose();
    let target = Vec3::new(1.0, 1.0, 0.0);
    let result = ik.solve(&mut pose, &s, target, Vec3::Z, 1.0).unwrap();
    assert!(result.reached.abs_diff_eq(target, 1e-4));
    assert!(!result.clamped);
    assert_eq!(pose.local[1].translation, Vec3::Y);
    assert_eq!(pose.local[2].translation, Vec3::Y);
    let result = ik
        .solve(&mut pose, &s, Vec3::X * 10.0, Vec3::Z, 1.0)
        .unwrap();
    assert!(result.clamped);
    assert!(result.reached.abs_diff_eq(Vec3::X * 2.0, 1e-4));
    let saved = pose.clone();
    assert!(ik.solve(&mut pose, &s, Vec3::NAN, Vec3::Z, 1.0).is_err());
    assert_eq!(pose, saved);
}
#[test]
fn foot_ground_normal_and_look_at_angle_are_applied() {
    let s = chain();
    let ik = TwoBoneIk {
        root: 0,
        middle: 1,
        end: 2,
    };
    let mut pose = s.rest_pose();
    let normal = Vec3::new(0.0, 1.0, 0.3).normalize();
    ik.adapt_foot(
        &mut pose,
        &s,
        Some(GroundContact {
            position: Vec3::Y * 1.5,
            normal,
            sole_height: 0.1,
            local_up: Vec3::Y,
        }),
        Vec3::Z,
        1.0,
    )
    .unwrap();
    let mut matrices = vec![Mat4::IDENTITY; 3];
    pose.to_model_space(&s, &mut matrices);
    assert!(
        matrices[2]
            .transform_vector3(Vec3::Y)
            .abs_diff_eq(normal, 1e-4)
    );
    let mut pose = s.rest_pose();
    look_at(&mut pose, &s, 0, Vec3::Y, Vec3::X, 0.25, 1.0).unwrap();
    assert!((pose.local[0].rotation.to_axis_angle().1.abs() - 0.25).abs() < 1e-5);
    let before = pose.clone();
    assert_eq!(
        ik.adapt_foot(&mut pose, &s, None, Vec3::Z, 1.0).unwrap(),
        None
    );
    assert_eq!(pose, before);
}
#[test]
fn ragdoll_blend_round_trips_model_snapshot_and_recovers() {
    let s = chain();
    let pose = s.rest_pose();
    let model = [
        Some(Mat4::from_translation(Vec3::X * 3.0)),
        None,
        Some(Mat4::from_translation(Vec3::new(3.0, 1.0, 1.0))),
    ];
    let physical = blend_ragdoll(&pose, &s, &model, 1.0).unwrap();
    let mut result = vec![Mat4::IDENTITY; 3];
    physical.to_model_space(&s, &mut result);
    assert!(result[2].abs_diff_eq(model[2].unwrap(), 1e-5));
    assert_eq!(blend_ragdoll(&pose, &s, &model, 0.0).unwrap(), pose);
    let middle = blend_ragdoll(&pose, &s, &model, 0.5).unwrap();
    assert_eq!(middle.local[0].translation, Vec3::X * 1.5);
    assert!(blend_ragdoll(&pose, &s, &[Some(Mat4::ZERO); 3], 1.0).is_err());
}
#[test]
fn compression_reduces_curves_and_honors_dense_sampling_error() {
    let s = chain();
    let keys: Vec<_> = (0..=32).map(|i| i as f32 / 32.0).collect();
    let clip = AnimationClip::new(
        ClipId(7),
        &s,
        1.0,
        vec![TransformTrack {
            joint: 0,
            translation: keys
                .iter()
                .map(|&t| Keyframe::new(t, Vec3::new(t, (t * 4.0).sin(), 0.0)))
                .collect(),
            rotation: keys
                .iter()
                .map(|&t| Keyframe::new(t, Quat::from_rotation_y(t)))
                .collect(),
            scale: vec![],
        }],
        vec![],
    )
    .unwrap();
    let budget = CompressionBudget {
        translation: 0.01,
        rotation: 0.01,
        scale: 0.0,
    };
    let (compressed, report) = clip.compress(&s, budget).unwrap();
    assert!(report.retained_keys < report.source_keys / 2);
    assert!(report.retained_bytes < report.source_bytes);
    assert_eq!(clip.compress(&s, budget).unwrap().0, compressed);
    for i in 0..=1000 {
        let time = i as f32 / 1000.0;
        let a = clip.sample_local(&s, time).unwrap().local[0];
        let b = compressed.sample_local(&s, time).unwrap().local[0];
        assert!(a.translation.distance(b.translation) <= budget.translation + 1e-5);
        assert!(a.rotation.angle_between(b.rotation) <= budget.rotation + 1e-5);
    }
    assert_eq!(
        clip.compress(
            &s,
            CompressionBudget {
                translation: 0.0,
                rotation: 0.0,
                scale: 0.0
            }
        )
        .unwrap()
        .0,
        clip
    );
    assert!(
        clip.compress(
            &s,
            CompressionBudget {
                translation: f32::NAN,
                ..budget
            }
        )
        .is_err()
    );
}
