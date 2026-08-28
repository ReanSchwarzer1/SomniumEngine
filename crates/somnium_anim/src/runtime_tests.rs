//! MORROWIND-V runtime and reviewer-regression tests.

use glam::{Mat4, Quat, Vec2, Vec3};

use super::*;

fn skeleton() -> Skeleton {
    Skeleton::new(
        SkeletonId(11),
        vec!["root".into(), "spine".into(), "hand".into()],
        vec![NO_PARENT, 0, 1],
        vec![Mat4::IDENTITY; 3],
        vec![Transform::IDENTITY; 3],
    )
    .unwrap()
    .0
}

fn sync(duration: f32, contacts: [f32; 2]) -> SyncTrack {
    SyncTrack::new(
        "locomotion",
        duration,
        vec![
            SyncMarker::new("left_contact", contacts[0]),
            SyncMarker::new("right_contact", contacts[1]),
        ],
    )
    .unwrap()
}

fn clip(
    skeleton: &Skeleton,
    id: u64,
    duration: f32,
    distance: f32,
    contacts: [f32; 2],
) -> AnimationClip {
    AnimationClip::new(
        ClipId(id),
        skeleton,
        duration,
        vec![TransformTrack {
            joint: 0,
            translation: vec![
                Keyframe::new(0.0, Vec3::ZERO),
                Keyframe::new(duration, Vec3::X * distance),
            ],
            rotation: vec![
                Keyframe::new(0.0, Quat::IDENTITY),
                Keyframe::new(duration, Quat::from_rotation_z(1.0)),
            ],
            scale: vec![],
        }],
        vec![sync(duration, contacts)],
    )
    .unwrap()
}

fn schema(id: u64) -> ParameterSchema {
    ParameterSchema::new(
        ParameterSchemaId(id),
        vec![
            ParameterDefinition::new("speed", ParameterValue::Float(0.0)),
            ParameterDefinition::new("x", ParameterValue::Float(0.0)),
            ParameterDefinition::new("y", ParameterValue::Float(0.0)),
            ParameterDefinition::new("layer", ParameterValue::Float(0.0)),
            ParameterDefinition::new("go", ParameterValue::Trigger(false)),
        ],
    )
    .unwrap()
}

#[test]
fn clip_sampling_interpolates_and_keeps_unkeyed_rest_channels() {
    let skeleton = skeleton();
    let clip = clip(&skeleton, 1, 2.0, 4.0, [0.0, 1.0]);
    let pose = clip.sample(&skeleton, 0.5, Playback::ONCE).unwrap();
    assert!((pose.local[0].translation.x - 1.0).abs() < 1e-5);
    assert!((pose.local[0].rotation.to_axis_angle().1 - 0.25).abs() < 1e-5);
    assert_eq!(pose.local[1], Transform::IDENTITY);
}

#[test]
fn playback_and_sync_apis_reject_nan_infinity_and_invalid_structures() {
    assert_eq!(
        Playback::new(true, f32::NAN),
        Err(TimeError::NonFiniteScale)
    );
    assert_eq!(SyncTrack::new("", 1.0, vec![]), Err(SyncError::EmptyName));
    assert_eq!(
        SyncTrack::new(
            "feet",
            f32::INFINITY,
            vec![SyncMarker::new("a", 0.0), SyncMarker::new("b", 0.5)]
        ),
        Err(SyncError::InvalidDuration)
    );
    let track = sync(1.0, [0.0, 0.5]);
    assert_eq!(track.phase_at(f32::NAN), Err(SyncError::NonFiniteTime));
    assert_eq!(
        track.time_at_phase(f32::INFINITY),
        Err(SyncError::NonFinitePhase)
    );
    let skeleton = skeleton();
    let clip = clip(&skeleton, 1, 1.0, 1.0, [0.0, 0.5]);
    assert_eq!(
        clip.local_time(f32::NAN, Playback::LOOPING),
        Err(ClipError::Time(TimeError::NonFiniteElapsed))
    );
}

#[test]
fn clip_joint_indices_are_validated_at_construction() {
    let skeleton = skeleton();
    let result = AnimationClip::new(
        ClipId(1),
        &skeleton,
        1.0,
        vec![TransformTrack {
            joint: 99,
            ..Default::default()
        }],
        vec![],
    );
    assert_eq!(result, Err(ClipError::JointOutOfRange));
}

#[test]
fn sync_phase_round_trips_through_the_wrapped_segment() {
    let track = SyncTrack::new(
        "feet",
        1.0,
        vec![
            SyncMarker::new("left", 0.25),
            SyncMarker::new("right", 0.75),
        ],
    )
    .unwrap();
    for time in [0.0, 0.25, 0.5, 0.75, 0.95] {
        let phase = track.phase_at(time).unwrap();
        let round_trip = track.time_at_phase(phase).unwrap();
        assert!((time - round_trip).abs() < 1e-5);
    }
}

#[test]
fn sync_tracks_on_and_off_produce_different_locomotion_blends() {
    let skeleton = skeleton();
    let walk = clip(&skeleton, 1, 2.0, 2.0, [0.0, 1.0]);
    let run = clip(&skeleton, 2, 1.0, 4.0, [0.0, 0.5]);
    let blend = Blend1D::new(
        vec![
            BlendSample1D {
                position: 1.0,
                clip: &walk,
                playback: Playback::LOOPING,
            },
            BlendSample1D {
                position: 4.0,
                clip: &run,
                playback: Playback::LOOPING,
            },
        ],
        0,
    )
    .unwrap();
    let off = blend.sample(&skeleton, 2.5, 0.75, None).unwrap();
    let on = blend
        .sample(&skeleton, 2.5, 0.75, Some("locomotion"))
        .unwrap();
    assert!((off.local[0].translation.x - on.local[0].translation.x).abs() > 0.25);
}

#[test]
fn fixed_sync_leader_is_continuous_when_the_active_1d_bracket_changes() {
    let skeleton = skeleton();
    let slow = clip(&skeleton, 1, 2.0, 2.0, [0.0, 1.0]);
    let medium = clip(&skeleton, 2, 1.0, 4.0, [0.0, 0.5]);
    let fast = clip(&skeleton, 3, 0.5, 8.0, [0.0, 0.25]);
    let blend = Blend1D::new(
        vec![
            BlendSample1D {
                position: 0.0,
                clip: &slow,
                playback: Playback::LOOPING,
            },
            BlendSample1D {
                position: 1.0,
                clip: &medium,
                playback: Playback::LOOPING,
            },
            BlendSample1D {
                position: 2.0,
                clip: &fast,
                playback: Playback::LOOPING,
            },
        ],
        0,
    )
    .unwrap();
    let left = blend
        .sample(&skeleton, 0.999, 0.37, Some("locomotion"))
        .unwrap();
    let right = blend
        .sample(&skeleton, 1.001, 0.37, Some("locomotion"))
        .unwrap();
    assert!((left.local[0].translation.x - right.local[0].translation.x).abs() < 0.02);
}

#[test]
fn triangulated_blend2d_clamps_to_the_hull_and_stays_local() {
    let skeleton = skeleton();
    let clips = [
        clip(&skeleton, 1, 1.0, 0.0, [0.0, 0.5]),
        clip(&skeleton, 2, 1.0, 10.0, [0.0, 0.5]),
        clip(&skeleton, 3, 1.0, 20.0, [0.0, 0.5]),
        clip(&skeleton, 4, 1.0, 100.0, [0.0, 0.5]),
    ];
    let blend = Blend2D::new(
        vec![
            BlendSample2D {
                position: Vec2::new(0.0, 0.0),
                clip: &clips[0],
                playback: Playback::ONCE,
            },
            BlendSample2D {
                position: Vec2::new(1.0, 0.0),
                clip: &clips[1],
                playback: Playback::ONCE,
            },
            BlendSample2D {
                position: Vec2::new(1.0, 1.0),
                clip: &clips[2],
                playback: Playback::ONCE,
            },
            BlendSample2D {
                position: Vec2::new(0.0, 1.0),
                clip: &clips[3],
                playback: Playback::ONCE,
            },
        ],
        vec![[0, 1, 2], [0, 2, 3]],
        0,
    )
    .unwrap();
    let pose = blend
        .sample(&skeleton, Vec2::new(2.0, 0.5), 1.0, None)
        .unwrap();
    assert!((pose.local[0].translation.x - 15.0).abs() < 1e-4);
}

#[test]
fn invalid_triangulations_are_rejected() {
    assert_eq!(
        Triangulation2D::new(vec![Vec2::ZERO, Vec2::X, Vec2::Y], vec![[0, 1, 1]]),
        Err(TriangulationError::DegenerateTriangle)
    );
    assert_eq!(
        Triangulation2D::new(vec![Vec2::ZERO, Vec2::X, Vec2::Y], vec![[0, 1, 9]]),
        Err(TriangulationError::BadIndex)
    );
}

#[test]
fn triangulation_rejects_shared_edge_same_side_containment() {
    let result = Triangulation2D::new(
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(0.0, 2.0),
            Vec2::new(0.5, 0.5),
        ],
        vec![[0, 1, 2], [0, 1, 3]],
    );
    assert_eq!(result, Err(TriangulationError::OverlappingTriangles));

    // A legitimate hull triangulation shares its diagonal with opposite
    // vertices on opposite sides and must remain accepted.
    assert!(
        Triangulation2D::new(
            vec![Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y],
            vec![[0, 1, 2], [0, 2, 3]],
        )
        .is_ok()
    );
}

#[test]
fn triangulation_boundary_must_be_the_single_point_set_hull() {
    // Four triangles form a connected ring between an outer and inner square.
    // Degree/manifold checks alone accept it, but the inner cycle is a hole in
    // the authored blend domain and must not become a clamping boundary.
    let points = vec![
        Vec2::new(-2.0, -2.0),
        Vec2::new(2.0, -2.0),
        Vec2::new(2.0, 2.0),
        Vec2::new(-2.0, 2.0),
        Vec2::new(-1.0, -1.0),
        Vec2::new(1.0, -1.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(-1.0, 1.0),
    ];
    let ring = vec![
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
    ];
    assert_eq!(
        Triangulation2D::new(points, ring),
        Err(TriangulationError::InvalidBoundary)
    );
}

#[test]
fn hull_validation_keeps_authored_samples_on_a_straight_boundary_edge() {
    let points = vec![
        Vec2::new(-1.0, -1.0),
        Vec2::new(0.0, -1.0),
        Vec2::new(1.0, -1.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(-1.0, 1.0),
    ];
    assert!(Triangulation2D::new(points, vec![[0, 1, 4], [1, 3, 4], [1, 2, 3]]).is_ok());
}

#[test]
fn layers_reject_nonfinite_weights() {
    let skeleton = skeleton();
    let pose = skeleton.rest_pose();
    assert_eq!(
        layer_poses(
            &pose,
            &[PoseLayer {
                pose: &pose,
                weight: f32::NAN,
                mask: None,
            }]
        ),
        Err(LayerError::NonFiniteWeight)
    );
}

fn compiled_graph(id: u64, version: u32) -> (Skeleton, AnimGraphAsset) {
    let skeleton = skeleton();
    let idle = clip(&skeleton, 1, 2.0, 0.0, [0.0, 1.0]);
    let run = clip(&skeleton, 2, 1.0, 4.0, [0.0, 0.5]);
    let aim = AnimationClip::new(
        ClipId(3),
        &skeleton,
        1.0,
        vec![TransformTrack {
            joint: 2,
            translation: vec![Keyframe::new(0.0, Vec3::Y * 2.0)],
            ..Default::default()
        }],
        vec![sync(1.0, [0.0, 0.5])],
    )
    .unwrap();
    let parameters = schema(id);
    let mask = BoneMask::new(&skeleton, vec![0.0, 0.0, 1.0]).unwrap();
    let graph = AnimGraphAsset::new(
        GraphId(id),
        version,
        &skeleton,
        vec![idle, run, aim],
        vec![
            AnimNode::Clip {
                clip: ClipId(1),
                playback: Playback::LOOPING,
            },
            AnimNode::Clip {
                clip: ClipId(2),
                playback: Playback::LOOPING,
            },
            AnimNode::Blend1D {
                parameter: "speed".into(),
                samples: vec![
                    NodeBlendSample1D {
                        position: 0.0,
                        node: AnimNodeId(0),
                    },
                    NodeBlendSample1D {
                        position: 4.0,
                        node: AnimNodeId(1),
                    },
                ],
                sync_track: Some("locomotion".into()),
                sync_leader: 0,
            },
            AnimNode::Clip {
                clip: ClipId(3),
                playback: Playback::ONCE,
            },
            AnimNode::Layer {
                base: AnimNodeId(2),
                layers: vec![NodeLayer {
                    node: AnimNodeId(3),
                    weight: LayerWeight::Parameter("layer".into()),
                    mask: Some(mask),
                }],
            },
            AnimNode::Cache {
                source: AnimNodeId(4),
            },
        ],
        parameters,
        AnimNodeId(5),
    )
    .unwrap();
    (skeleton, graph)
}

#[test]
fn compiled_graph_nodes_evaluate_clip_blend_layer_and_cache_without_ui_types() {
    let (skeleton, graph) = compiled_graph(7, ANIM_GRAPH_VERSION);
    let mut parameters = graph.parameters().instantiate();
    parameters.set("speed", ParameterValue::Float(2.0)).unwrap();
    parameters.set("layer", ParameterValue::Float(0.5)).unwrap();
    let mut cache = PoseCache::default();
    let pose = graph
        .evaluate(&skeleton, &parameters, 0.5, 9, &mut cache)
        .unwrap();
    assert!(pose.local[0].translation.x > 0.0);
    assert_eq!(pose.local[2].translation, Vec3::Y);
    assert_eq!(cache.len(), 1);
}

#[test]
fn compiled_graph_blend2d_uses_authored_triangles() {
    let skeleton = skeleton();
    let clips = [
        clip(&skeleton, 1, 1.0, 0.0, [0.0, 0.5]),
        clip(&skeleton, 2, 1.0, 10.0, [0.0, 0.5]),
        clip(&skeleton, 3, 1.0, 20.0, [0.0, 0.5]),
    ];
    let graph = AnimGraphAsset::new(
        GraphId(8),
        1,
        &skeleton,
        clips.into_iter().collect(),
        vec![
            AnimNode::Clip {
                clip: ClipId(1),
                playback: Playback::ONCE,
            },
            AnimNode::Clip {
                clip: ClipId(2),
                playback: Playback::ONCE,
            },
            AnimNode::Clip {
                clip: ClipId(3),
                playback: Playback::ONCE,
            },
            AnimNode::Blend2D {
                parameter_x: "x".into(),
                parameter_y: "y".into(),
                samples: vec![
                    NodeBlendSample2D {
                        position: Vec2::ZERO,
                        node: AnimNodeId(0),
                    },
                    NodeBlendSample2D {
                        position: Vec2::X,
                        node: AnimNodeId(1),
                    },
                    NodeBlendSample2D {
                        position: Vec2::Y,
                        node: AnimNodeId(2),
                    },
                ],
                triangles: vec![[0, 1, 2]],
                sync_track: None,
                sync_leader: 0,
            },
        ],
        schema(8),
        AnimNodeId(3),
    )
    .unwrap();
    let mut parameters = graph.parameters().instantiate();
    parameters.set("x", ParameterValue::Float(0.5)).unwrap();
    parameters.set("y", ParameterValue::Float(0.5)).unwrap();
    let pose = graph
        .evaluate(&skeleton, &parameters, 1.0, 1, &mut PoseCache::default())
        .unwrap();
    assert!((pose.local[0].translation.x - 15.0).abs() < 1e-4);
}

#[test]
fn graph_rejects_nonfinite_layer_constants_and_forward_references() {
    let skeleton = skeleton();
    let clip = clip(&skeleton, 1, 1.0, 1.0, [0.0, 0.5]);
    let bad_layer = AnimGraphAsset::new(
        GraphId(1),
        1,
        &skeleton,
        vec![clip.clone()],
        vec![
            AnimNode::Clip {
                clip: ClipId(1),
                playback: Playback::LOOPING,
            },
            AnimNode::Layer {
                base: AnimNodeId(0),
                layers: vec![NodeLayer {
                    node: AnimNodeId(0),
                    weight: LayerWeight::Constant(f32::INFINITY),
                    mask: None,
                }],
            },
        ],
        schema(1),
        AnimNodeId(1),
    );
    assert_eq!(bad_layer, Err(AnimGraphError::InvalidLayer));
    let forward = AnimGraphAsset::new(
        GraphId(1),
        1,
        &skeleton,
        vec![clip],
        vec![
            AnimNode::Cache {
                source: AnimNodeId(1),
            },
            AnimNode::Clip {
                clip: ClipId(1),
                playback: Playback::LOOPING,
            },
        ],
        schema(1),
        AnimNodeId(0),
    );
    assert_eq!(forward, Err(AnimGraphError::ForwardReference));
}

#[test]
fn outer_sync_rejects_missing_or_incompatible_nested_layer_branches() {
    fn graph_with_overlay_tracks(
        skeleton: &Skeleton,
        overlay_sync: Vec<SyncTrack>,
    ) -> Result<AnimGraphAsset, AnimGraphError> {
        let base = clip(skeleton, 1, 1.0, 1.0, [0.0, 0.5]);
        let overlay =
            AnimationClip::new(ClipId(2), skeleton, 1.0, Vec::new(), overlay_sync).unwrap();
        AnimGraphAsset::new(
            GraphId(90),
            1,
            skeleton,
            vec![base, overlay],
            vec![
                AnimNode::Clip {
                    clip: ClipId(1),
                    playback: Playback::LOOPING,
                },
                AnimNode::Clip {
                    clip: ClipId(2),
                    playback: Playback::LOOPING,
                },
                AnimNode::Layer {
                    base: AnimNodeId(0),
                    layers: vec![NodeLayer {
                        node: AnimNodeId(1),
                        weight: LayerWeight::Constant(1.0),
                        mask: None,
                    }],
                },
                AnimNode::Blend1D {
                    parameter: "speed".into(),
                    samples: vec![
                        NodeBlendSample1D {
                            position: 0.0,
                            node: AnimNodeId(0),
                        },
                        NodeBlendSample1D {
                            position: 1.0,
                            node: AnimNodeId(2),
                        },
                    ],
                    sync_track: Some("locomotion".into()),
                    sync_leader: 0,
                },
            ],
            schema(90),
            AnimNodeId(3),
        )
    }

    let skeleton = skeleton();
    assert_eq!(
        graph_with_overlay_tracks(&skeleton, Vec::new()),
        Err(AnimGraphError::MissingSyncTrack("locomotion".into()))
    );
    let incompatible = SyncTrack::new(
        "locomotion",
        1.0,
        vec![SyncMarker::new("toe", 0.0), SyncMarker::new("heel", 0.5)],
    )
    .unwrap();
    assert_eq!(
        graph_with_overlay_tracks(&skeleton, vec![incompatible]),
        Err(AnimGraphError::IncompatibleSyncTracks("locomotion".into()))
    );
}

#[test]
fn pose_cache_keys_require_generation_lane_graph_version_and_node() {
    let skeleton = skeleton();
    let pose = skeleton.rest_pose();
    let mut cache = PoseCache::default();
    cache.insert(
        PoseCacheKey::new(1, EvaluationLane::Output, GraphId(1), 7, AnimNodeId(2)),
        pose.clone(),
    );
    assert!(
        cache
            .get(PoseCacheKey::new(
                2,
                EvaluationLane::Output,
                GraphId(1),
                7,
                AnimNodeId(2)
            ))
            .is_none()
    );
    assert!(
        cache
            .get(PoseCacheKey::new(
                1,
                EvaluationLane::StateSource,
                GraphId(1),
                7,
                AnimNodeId(2)
            ))
            .is_none()
    );
    assert!(
        cache
            .get(PoseCacheKey::new(
                1,
                EvaluationLane::Output,
                GraphId(1),
                8,
                AnimNodeId(2)
            ))
            .is_none()
    );
    assert!(
        cache
            .get(PoseCacheKey::new(
                1,
                EvaluationLane::Output,
                GraphId(2),
                7,
                AnimNodeId(2)
            ))
            .is_none()
    );
    assert!(
        cache
            .get(PoseCacheKey::new(
                1,
                EvaluationLane::Output,
                GraphId(1),
                7,
                AnimNodeId(3)
            ))
            .is_none()
    );
    assert!(
        cache
            .get(PoseCacheKey::new(
                1,
                EvaluationLane::Output,
                GraphId(1),
                7,
                AnimNodeId(2)
            ))
            .is_some()
    );
}

#[test]
fn ordinary_evaluation_bounds_generations_and_discards_hot_reload_versions() {
    let (skeleton, graph_v1) = compiled_graph(71, 1);
    let parameters_v1 = graph_v1.parameters().instantiate();
    let mut cache = PoseCache::default();
    graph_v1
        .evaluate(&skeleton, &parameters_v1, 0.1, 1, &mut cache)
        .unwrap();
    graph_v1
        .evaluate(&skeleton, &parameters_v1, 0.2, 2, &mut cache)
        .unwrap();
    assert_eq!(cache.len(), 1);

    let (_, graph_v2) = compiled_graph(71, 2);
    let parameters_v2 = graph_v2.parameters().instantiate();
    graph_v2
        .evaluate(&skeleton, &parameters_v2, 0.3, 2, &mut cache)
        .unwrap();
    assert_eq!(cache.len(), 1);
    assert!(
        cache
            .get(PoseCacheKey::new(
                2,
                EvaluationLane::Output,
                GraphId(71),
                2,
                AnimNodeId(5),
            ))
            .is_some()
    );
}

#[test]
fn state_sampling_lane_cannot_alias_any_future_output_generation() {
    let transition_key =
        PoseCacheKey::new(4, EvaluationLane::StateTarget, GraphId(1), 1, AnimNodeId(9));
    for generation in [4, 5, 8, u64::MAX] {
        assert_ne!(
            transition_key,
            PoseCacheKey::new(
                generation,
                EvaluationLane::Output,
                GraphId(1),
                1,
                AnimNodeId(9),
            )
        );
    }
}

fn machine(graph: &AnimGraphAsset, version: u32) -> StateMachine {
    StateMachine::new(
        MachineId(44),
        version,
        graph,
        vec![
            AnimationState {
                id: StateId(0),
                name: "locomotion".into(),
                node: AnimNodeId(2),
            },
            AnimationState {
                id: StateId(1),
                name: "layered".into(),
                node: AnimNodeId(5),
            },
        ],
        vec![StateTransition {
            from: StateId(0),
            to: StateId(1),
            conditions: vec![Condition::Trigger {
                parameter: "go".into(),
            }],
            blend_seconds: 0.5,
            sync_track: Some("locomotion".into()),
        }],
        StateId(0),
    )
    .unwrap()
}

#[test]
fn states_reference_arbitrary_compiled_pose_nodes() {
    let (_skeleton, graph) = compiled_graph(7, 1);
    let machine = machine(&graph, 3);
    assert!(matches!(
        graph.nodes()[machine.states()[0].node.0 as usize],
        AnimNode::Blend1D { .. }
    ));
    assert!(matches!(
        graph.nodes()[machine.states()[1].node.0 as usize],
        AnimNode::Cache { .. }
    ));
}

#[test]
fn state_definitions_validate_conditions_schemas_and_sync_compatibility() {
    let (_skeleton, graph) = compiled_graph(7, 1);
    let invalid_condition = StateMachine::new(
        MachineId(1),
        1,
        &graph,
        vec![
            AnimationState {
                id: StateId(0),
                name: "a".into(),
                node: AnimNodeId(0),
            },
            AnimationState {
                id: StateId(1),
                name: "b".into(),
                node: AnimNodeId(1),
            },
        ],
        vec![StateTransition {
            from: StateId(0),
            to: StateId(1),
            conditions: vec![Condition::Bool {
                parameter: "speed".into(),
                value: true,
            }],
            blend_seconds: 0.1,
            sync_track: None,
        }],
        StateId(0),
    );
    assert_eq!(invalid_condition, Err(StateMachineError::InvalidCondition));

    let invalid_sync = StateMachine::new(
        MachineId(1),
        1,
        &graph,
        vec![
            AnimationState {
                id: StateId(0),
                name: "a".into(),
                node: AnimNodeId(0),
            },
            AnimationState {
                id: StateId(1),
                name: "aim".into(),
                node: AnimNodeId(3),
            },
        ],
        vec![StateTransition {
            from: StateId(0),
            to: StateId(1),
            conditions: vec![],
            blend_seconds: 0.1,
            sync_track: Some("missing_track".into()),
        }],
        StateId(0),
    );
    assert_eq!(invalid_sync, Err(StateMachineError::InvalidSyncTrack));
}

#[test]
fn sync_aligned_target_time_survives_transition_completion_without_a_pose_jump() {
    let (skeleton, graph) = compiled_graph(7, 1);
    let machine = machine(&graph, 1);
    let mut player = StateMachinePlayer::new(&machine);
    let mut parameters = graph.parameters().instantiate();
    parameters.set("speed", ParameterValue::Float(3.0)).unwrap();
    parameters.trigger("go").unwrap();
    player
        .advance(&machine, &graph, &mut parameters, 0.37)
        .unwrap();
    player
        .advance(&machine, &graph, &mut parameters, 0.49)
        .unwrap();
    let mut cache = PoseCache::default();
    let before = player
        .sample(&machine, &graph, &skeleton, &parameters, 1, &mut cache)
        .unwrap();
    player
        .advance(&machine, &graph, &mut parameters, 0.01)
        .unwrap();
    assert!(!player.is_transitioning());
    let after = player
        .sample(&machine, &graph, &skeleton, &parameters, 2, &mut cache)
        .unwrap();
    assert!((before.local[0].translation.x - after.local[0].translation.x).abs() < 0.05);
}

#[test]
fn stale_players_return_version_errors_instead_of_indexing_new_definitions() {
    let (skeleton, graph) = compiled_graph(7, 1);
    let old = machine(&graph, 1);
    let player = StateMachinePlayer::new(&old);
    let new = machine(&graph, 2);
    let parameters = graph.parameters().instantiate();
    let mut cache = PoseCache::default();
    assert_eq!(
        player.sample(&new, &graph, &skeleton, &parameters, 1, &mut cache),
        Err(StateMachineError::VersionMismatch)
    );
}

#[test]
fn parameter_sets_are_schema_bound_and_finite() {
    let schema = schema(1);
    let mut parameters = schema.instantiate();
    assert_eq!(
        parameters.set("speed", ParameterValue::Bool(true)),
        Err(ParameterError::TypeMismatch("speed".into()))
    );
    assert_eq!(
        parameters.set("speed", ParameterValue::Float(f32::NAN)),
        Err(ParameterError::NonFinite("speed".into()))
    );
}
