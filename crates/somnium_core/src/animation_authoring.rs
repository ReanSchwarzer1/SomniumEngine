//! Designer-facing animation preview and runtime binding. Details edits use the
//! regular reflected property path; a preview rig supplies visible joints and
//! draggable targets. Bound game clips use the same controls and pose runtime.
use crate::{Name, Parent, Transform};
use glam::{Mat4, Quat, Vec3};
use somnium_anim::*;
use somnium_ecs::{Component, Entity, TypeRegistry, World, component_schema};
use somnium_physics::{
    animation::{CapsuleSweep, RagdollId, RagdollPart},
    world::PhysicsWorld,
};
use std::{collections::HashMap, sync::Arc};

/// Persisted, grouped Details controls for clip preview and procedural posing.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationAuthoring {
    /// Advance the preview clock.
    pub playing: bool,
    /// Use the shared pose job graph.
    pub pose_jobs: bool,
    /// Playback rate, including reverse travel.
    pub speed: f32,
    /// Duration of the built-in preview clip.
    pub duration: f32,
    /// Preview root displacement per cycle.
    pub stride: Vec3,
    /// Preview root turn per cycle, in radians.
    pub turn: f32,
    /// Apply extracted root movement through Jolt collision.
    pub root_motion: bool,
    /// Skeleton root index.
    pub root_joint: u32,
    /// Root capsule half-height.
    pub capsule_half_height: f32,
    /// Root capsule radius.
    pub capsule_radius: f32,
    /// Limb root index.
    pub ik_root: u32,
    /// Limb middle index.
    pub ik_middle: u32,
    /// Limb tip index.
    pub ik_end: u32,
    /// Drag this entity in the viewport to pose the limb.
    pub reach_target: Entity,
    /// Bend-plane point in the rig's model space.
    pub pole: Vec3,
    /// Contribution of the limb solver.
    pub ik_weight: f32,
    /// Probe real scene geometry below the foot.
    pub foot_ik: bool,
    /// Clearance above the ground contact.
    pub sole_height: f32,
    /// Entity to aim the selected look joint toward.
    pub look_target: Entity,
    /// Aiming joint index.
    pub look_joint: u32,
    /// Aiming contribution.
    pub look_weight: f32,
    /// Maximum aiming cone, radians.
    pub look_limit: f32,
    /// Blend into physical simulation; zero recovers animation.
    pub ragdoll_weight: f32,
    /// Ragdoll swing cone, radians.
    pub ragdoll_swing: f32,
    /// Ragdoll twist limit, radians.
    pub ragdoll_twist: f32,
    /// Fit the source clip before evaluating it.
    pub compression: bool,
    /// Local translation error budget, metres.
    pub translation_error: f32,
    /// Angular error budget, radians.
    pub rotation_error: f32,
    /// Scale-vector error budget.
    pub scale_error: f32,
    /// Existing animation timeline asset; its markers become animation events.
    pub events_timeline: String,
    /// First contact time when no timeline is selected.
    pub left_contact: f32,
    /// Second contact time when no timeline is selected.
    pub right_contact: f32,
    /// Explicit asset reload generation, changed by the Reload command.
    pub reload_revision: u32,
    /// Read-only source/reduced payload report.
    pub compression_report: String,
    /// Read-only current preview time.
    pub preview_time: f32,
    /// Read-only event count since reset.
    pub event_count: u32,
    /// Read-only last dispatched marker label.
    pub last_event: String,
    /// Read-only active state or actionable validation error.
    pub status: String,
}
impl Component for AnimationAuthoring {}
impl Default for AnimationAuthoring {
    fn default() -> Self {
        Self {
            playing: true,
            pose_jobs: true,
            speed: 1.0,
            duration: 2.0,
            stride: Vec3::X * 2.0,
            turn: 0.0,
            root_motion: false,
            root_joint: 0,
            capsule_half_height: 0.5,
            capsule_radius: 0.2,
            ik_root: 0,
            ik_middle: 1,
            ik_end: 2,
            reach_target: Entity::DANGLING,
            pole: Vec3::Z * 2.0,
            ik_weight: 0.0,
            foot_ik: false,
            sole_height: 0.08,
            look_target: Entity::DANGLING,
            look_joint: 2,
            look_weight: 0.0,
            look_limit: 1.2,
            ragdoll_weight: 0.0,
            ragdoll_swing: 0.6,
            ragdoll_twist: 0.3,
            compression: true,
            translation_error: 0.005,
            rotation_error: 0.005,
            scale_error: 0.001,
            events_timeline: String::new(),
            left_contact: 0.25,
            right_contact: 0.75,
            reload_revision: 0,
            compression_report: String::new(),
            preview_time: 0.0,
            event_count: 0,
            last_event: String::new(),
            status: "Ready".into(),
        }
    }
}

/// A visible preview marker driven by one skeleton joint, serialized with the rig.
#[derive(Clone, Copy, Debug)]
pub struct AnimationPreviewJoint {
    /// Owning animation rig; remapped by scene copy/paste.
    pub owner: Entity,
    /// Index into the rig's skeleton.
    pub joint: u32,
}
impl Component for AnimationPreviewJoint {}
impl Default for AnimationPreviewJoint {
    fn default() -> Self {
        Self {
            owner: Entity::DANGLING,
            joint: 0,
        }
    }
}

/// Register author controls with the standard Details/undo/scene schema path.
pub fn register(registry: &mut TypeRegistry) {
    registry.register(component_schema! {
        AnimationAuthoring as "somnium.AnimationAuthoring", display "Animation", version 1,
        fields {
            playing { group: "Playback", doc: "Play or pause this rig preview without entering game Play." },
            pose_jobs { group: "Playback", display_name: "Evaluate on Jobs", doc: "Schedules clip and pose dependencies on the shared worker pool." },
            speed { min: -4.0, max: 4.0, step: 0.1, group: "Playback" },
            duration { min: 0.05, max: 60.0, unit: "s", group: "Preview Clip" },
            stride { group: "Preview Clip", unit: "m", doc: "Root displacement of the built-in preview clip. Bound imported clips retain their source tracks." },
            turn { min: -3.14, max: 3.14, unit: "rad", group: "Preview Clip" },
            root_motion { group: "Root Motion" }, root_joint { min: 0, max: 255, group: "Root Motion" },
            capsule_half_height { min: 0.01, max: 10.0, unit: "m", group: "Root Motion" },
            capsule_radius { min: 0.01, max: 10.0, unit: "m", group: "Root Motion" },
            ik_root { min: 0, max: 255, group: "Limb IK" }, ik_middle { min: 0, max: 255, group: "Limb IK" },
            ik_end { min: 0, max: 255, group: "Limb IK" }, reach_target { group: "Limb IK" },
            pole { group: "Limb IK", unit: "m" }, ik_weight { min: 0.0, max: 1.0, step: 0.05, group: "Limb IK" },
            foot_ik { group: "Ground Adaptation" }, sole_height { min: 0.0, max: 1.0, unit: "m", group: "Ground Adaptation" },
            look_target { group: "Look At" }, look_joint { min: 0, max: 255, group: "Look At" },
            look_weight { min: 0.0, max: 1.0, step: 0.05, group: "Look At" },
            look_limit { min: 0.0, max: 3.14, unit: "rad", group: "Look At" },
            ragdoll_weight { min: 0.0, max: 1.0, step: 0.05, group: "Ragdoll", doc: "Raise to enter constrained Jolt simulation; return to zero to recover animation." },
            ragdoll_swing { min: 0.0, max: 3.14, unit: "rad", group: "Ragdoll" },
            ragdoll_twist { min: 0.0, max: 3.14, unit: "rad", group: "Ragdoll" },
            compression { group: "Compression" },
            translation_error { min: 0.0, max: 1.0, step: 0.001, precision: 4, unit: "m", group: "Compression" },
            rotation_error { min: 0.0, max: 1.0, step: 0.001, precision: 4, unit: "rad", group: "Compression" },
            scale_error { min: 0.0, max: 1.0, step: 0.001, precision: 4, group: "Compression" },
            compression_report { group: "Compression", read_only: true, flags: FieldFlags::EDIT.union(FieldFlags::SCRIPT_READ) },
            events_timeline { group: "Events", doc: "Animation timeline asset path. Every named marker dispatches at its authored time. Reload after saving." },
            left_contact { min: 0.0, max: 0.99, group: "Events", doc: "Left foot contact as a fraction of the preview cycle when no timeline is selected." },
            right_contact { min: 0.0, max: 0.99, group: "Events", doc: "Right foot contact as a fraction of the preview cycle when no timeline is selected." },
            reload_revision { flags: FieldFlags::SERIALIZE },
            preview_time { group: "Preview", read_only: true, unit: "s", flags: FieldFlags::EDIT.union(FieldFlags::SCRIPT_READ) },
            event_count { group: "Preview", read_only: true, flags: FieldFlags::EDIT.union(FieldFlags::SCRIPT_READ) },
            last_event { group: "Preview", read_only: true, flags: FieldFlags::EDIT.union(FieldFlags::SCRIPT_READ) },
            status { group: "Preview", read_only: true, flags: FieldFlags::EDIT.union(FieldFlags::SCRIPT_READ) },
        }
    });
    registry.register(component_schema! { AnimationPreviewJoint as "somnium.AnimationPreviewJoint", display "Preview Joint", version 1,
        fields { owner { read_only: true }, joint { read_only: true } } });
}

/// Create a three-joint visual rig and two ordinary draggable target entities.
/// The app reconstructs the standard sphere MeshKind resources after this call.
pub fn create_preview_rig(world: &mut World, origin: Vec3) -> Entity {
    let root = world.spawn((
        Name::new("Animation Preview Rig"),
        Transform::from_translation(origin),
        crate::WorldTransform::identity(),
        AnimationAuthoring::default(),
    ));
    let mut children = crate::Children::empty();
    for i in 0..3 {
        let marker = world.spawn((
            Name::new(["Hip", "Knee", "Foot"][i]),
            Transform {
                translation: Vec3::Y * i as f32,
                rotation: Quat::IDENTITY,
                scale: Vec3::splat(0.16),
            },
            crate::WorldTransform::identity(),
            Parent { entity: root },
            crate::MeshKind::Sphere,
            crate::MaterialComponent::default(),
            AnimationPreviewJoint {
                owner: root,
                joint: i as u32,
            },
        ));
        children.push(marker);
    }
    let target = world.spawn((
        Name::new("IK Reach Target"),
        Transform {
            translation: Vec3::new(1.2, 1.2, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::splat(0.1),
        },
        crate::WorldTransform::identity(),
        crate::MeshKind::Sphere,
        crate::MaterialComponent::default(),
        Parent { entity: root },
    ));
    let look = world.spawn((
        Name::new("Look At Target"),
        Transform {
            translation: Vec3::new(1.0, 2.0, 2.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::splat(0.1),
        },
        crate::WorldTransform::identity(),
        crate::MeshKind::Sphere,
        crate::MaterialComponent::default(),
        Parent { entity: root },
    ));
    let settings = world.get_mut::<AnimationAuthoring>(root).unwrap();
    settings.reach_target = target;
    settings.look_target = look;
    children.push(target);
    children.push(look);
    world.insert_component(root, children).unwrap();
    crate::propagate_transforms(world);
    root
}

/// Open the rig's event asset on the existing visual timeline. An unbound rig
/// starts with editable left/right markers; the first save creates its asset.
pub fn event_timeline(
    world: &World,
    entity: Entity,
) -> Result<somnium_ui::timeline::TimelineDocument, String> {
    let settings = world
        .get::<AnimationAuthoring>(entity)
        .ok_or("Select an animation rig")?;
    let catalogue = somnium_ui::timeline::catalogues::animation();
    if !settings.events_timeline.is_empty() {
        let json = std::fs::read_to_string(&settings.events_timeline).map_err(|e| e.to_string())?;
        return somnium_ui::timeline::from_json(&json, &catalogue).map_err(|e| e.to_string());
    }
    let mut document = somnium_ui::timeline::TimelineDocument::new(catalogue.id, settings.duration);
    document
        .add_marker(settings.left_contact * settings.duration, "footstep.left")
        .map_err(|e| e.to_string())?;
    document
        .add_marker(settings.right_contact * settings.duration, "footstep.right")
        .map_err(|e| e.to_string())?;
    Ok(document)
}

/// Publish visual marker edits to a durable timeline and reload the rig. New
/// assets are named by persistent rig identity under assets/animation.
pub fn save_event_timeline(
    world: &mut World,
    entity: Entity,
    document: &somnium_ui::timeline::TimelineDocument,
) -> Result<String, String> {
    let settings = world
        .get::<AnimationAuthoring>(entity)
        .ok_or("Animation rig was removed")?;
    let mut path = settings.events_timeline.clone();
    let catalogue = somnium_ui::timeline::catalogues::animation();
    let json = somnium_ui::timeline::to_json(document).map_err(|e| e.to_string())?;
    somnium_ui::timeline::from_json(&json, &catalogue).map_err(|e| e.to_string())?;
    if document.markers().iter().any(|m| m.label.is_empty()) {
        return Err("Every animation event needs a marker name".into());
    }
    if path.is_empty() {
        let id = world
            .ensure_persistent_id(entity)
            .map_err(|e| e.to_string())?;
        path = format!("assets/animation/{id}-events.somtimeline");
    }
    crate::save_game::atomic_write(std::path::Path::new(&path), json.as_bytes())?;
    let settings = world.get_mut::<AnimationAuthoring>(entity).unwrap();
    settings.events_timeline = path.clone();
    settings.reload_revision = settings.reload_revision.wrapping_add(1);
    Ok(path)
}

struct BoundAnimation {
    skeleton: Arc<Skeleton>,
    source: AnimationClip,
    clip: AnimationClip,
    graph: Arc<AnimGraphAsset>,
    authored: AnimationAuthoring,
    time: f32,
    applied_time: f32,
    submitted_time: f32,
    pose: Pose,
    pending: Option<PoseEvaluation>,
    ragdoll: Option<RagdollId>,
    events: EventTrack,
    initial_transform: Transform,
    external: bool,
}

/// Runtime owning preview clocks and immutable bound clips; no second job pool.
#[derive(Default)]
pub struct AnimationAuthoringSystem {
    states: HashMap<Entity, BoundAnimation>,
}

fn source_settings(mut settings: AnimationAuthoring) -> AnimationAuthoring {
    settings.compression_report.clear();
    settings.status.clear();
    settings.last_event.clear();
    settings.preview_time = 0.0;
    settings.event_count = 0;
    settings
}
fn needs_rebuild(old: &AnimationAuthoring, new: &AnimationAuthoring, external: bool) -> bool {
    (!external
        && (old.duration != new.duration || old.stride != new.stride || old.turn != new.turn))
        || old.compression != new.compression
        || old.translation_error != new.translation_error
        || old.rotation_error != new.rotation_error
        || old.scale_error != new.scale_error
        || old.events_timeline != new.events_timeline
        || old.left_contact != new.left_contact
        || old.right_contact != new.right_contact
        || old.reload_revision != new.reload_revision
}
fn make_preview(settings: &AnimationAuthoring) -> Result<(Skeleton, AnimationClip), String> {
    let skeleton = Skeleton::new(
        SkeletonId(0),
        vec!["hip".into(), "knee".into(), "foot".into()],
        vec![NO_PARENT, 0, 1],
        vec![Mat4::IDENTITY; 3],
        vec![
            somnium_anim::Transform::IDENTITY,
            somnium_anim::Transform {
                translation: Vec3::Y,
                ..somnium_anim::Transform::IDENTITY
            },
            somnium_anim::Transform {
                translation: Vec3::Y,
                ..somnium_anim::Transform::IDENTITY
            },
        ],
    )
    .unwrap()
    .0;
    let samples: Vec<_> = (0..=32).map(|i| i as f32 / 32.0).collect();
    let root = TransformTrack {
        joint: 0,
        translation: samples
            .iter()
            .map(|t| Keyframe::new(t * settings.duration, settings.stride * *t))
            .collect(),
        rotation: samples
            .iter()
            .map(|t| {
                Keyframe::new(
                    t * settings.duration,
                    Quat::from_rotation_y(settings.turn * *t),
                )
            })
            .collect(),
        scale: vec![],
    };
    let knee = TransformTrack {
        joint: 1,
        rotation: samples
            .iter()
            .map(|t| {
                Keyframe::new(
                    t * settings.duration,
                    Quat::from_rotation_z((t * std::f32::consts::TAU).sin() * 0.45),
                )
            })
            .collect(),
        ..Default::default()
    };
    let clip = AnimationClip::new(
        ClipId(0),
        &skeleton,
        settings.duration,
        vec![root, knee],
        vec![],
    )
    .map_err(|e| format!("Preview clip: {e:?}"))?;
    Ok((skeleton, clip))
}
fn build_state(
    skeleton: Arc<Skeleton>,
    source: AnimationClip,
    settings: AnimationAuthoring,
    initial_transform: Transform,
    external: bool,
) -> Result<(BoundAnimation, String), String> {
    let (clip, report) = source
        .compress(
            &skeleton,
            CompressionBudget {
                translation: if settings.compression {
                    settings.translation_error
                } else {
                    0.0
                },
                rotation: if settings.compression {
                    settings.rotation_error
                } else {
                    0.0
                },
                scale: if settings.compression {
                    settings.scale_error
                } else {
                    0.0
                },
            },
        )
        .map_err(|e| format!("Compression: {e:?}"))?;
    let graph = Arc::new(
        AnimGraphAsset::new(
            GraphId(0),
            1,
            &skeleton,
            vec![clip.clone()],
            vec![AnimNode::Clip {
                clip: clip.id(),
                playback: Playback::LOOPING,
            }],
            ParameterSchema::new(ParameterSchemaId(0), vec![]).unwrap(),
            AnimNodeId(0),
        )
        .map_err(|e| format!("Graph: {e:?}"))?,
    );
    let mut events = if settings.events_timeline.is_empty() {
        vec![
            AnimationEvent {
                time: settings.left_contact * clip.duration(),
                name: "footstep.left".into(),
                payload: String::new(),
            },
            AnimationEvent {
                time: settings.right_contact * clip.duration(),
                name: "footstep.right".into(),
                payload: String::new(),
            },
        ]
    } else {
        let json = std::fs::read_to_string(&settings.events_timeline)
            .map_err(|e| format!("Event timeline: {e}"))?;
        let timeline =
            somnium_ui::timeline::from_json(&json, &somnium_ui::timeline::catalogues::animation())
                .map_err(|e| format!("Event timeline: {e}"))?;
        timeline
            .markers()
            .iter()
            .map(|m| AnimationEvent {
                time: (m.time / timeline.duration() * clip.duration()).rem_euclid(clip.duration()),
                name: m.label.clone(),
                payload: String::new(),
            })
            .collect()
    };
    events.sort_by(|a, b| a.time.total_cmp(&b.time));
    let events = EventTrack::new(clip.duration(), events).map_err(|e| format!("Events: {e:?}"))?;
    let pose = skeleton.rest_pose();
    let report = format!(
        "{} / {} keys · {} / {} bytes",
        report.retained_keys, report.source_keys, report.retained_bytes, report.source_bytes
    );
    Ok((
        BoundAnimation {
            skeleton,
            source,
            clip,
            graph,
            authored: source_settings(settings),
            time: 0.0,
            applied_time: 0.0,
            submitted_time: 0.0,
            pose,
            pending: None,
            ragdoll: None,
            events,
            initial_transform,
            external,
        },
        report,
    ))
}

impl AnimationAuthoringSystem {
    /// Bind an imported/game clip to an authored entity. Preview clip generation
    /// controls then leave its source tracks intact; compression and pose edits apply.
    pub fn bind_clip(
        &mut self,
        world: &mut World,
        entity: Entity,
        skeleton: Arc<Skeleton>,
        clip: AnimationClip,
    ) -> Result<(), String> {
        let settings = world
            .get::<AnimationAuthoring>(entity)
            .cloned()
            .ok_or("Entity needs Animation controls")?;
        let transform = world
            .get::<Transform>(entity)
            .copied()
            .ok_or("Entity needs a Transform")?;
        if self
            .states
            .get(&entity)
            .is_some_and(|s| s.ragdoll.is_some())
        {
            return Err("Reset physical preview before replacing its clip".into());
        }
        let (state, report) = build_state(skeleton, clip, settings, transform, true)?;
        self.states.insert(entity, state);
        world
            .get_mut::<AnimationAuthoring>(entity)
            .unwrap()
            .compression_report = report;
        Ok(())
    }
    /// Current fully evaluated pose for palette consumers.
    pub fn pose(&self, entity: Entity) -> Option<&Pose> {
        self.states.get(&entity).map(|s| &s.pose)
    }
    /// Release pending jobs and physical previews at a scene/play-state boundary.
    /// Resets derived Details feedback without changing authored transforms.
    pub fn clear(&mut self, world: &mut World, mut physics: Option<&mut PhysicsWorld>) {
        for (_, mut state) in self.states.drain() {
            if let (Some(id), Some(physics)) = (state.ragdoll.take(), physics.as_deref_mut()) {
                let _ = physics.destroy_ragdoll(id);
            }
        }
        let entities: Vec<_> = world.entities().collect();
        for entity in entities {
            if let Some(settings) = world.get_mut::<AnimationAuthoring>(entity) {
                settings.preview_time = 0.0;
                settings.event_count = 0;
                settings.last_event.clear();
                settings.status = "Ready".into();
            }
        }
    }
    /// Rewind the rig, clear event counters and release its physical preview.
    pub fn reset(
        &mut self,
        world: &mut World,
        mut physics: Option<&mut PhysicsWorld>,
        entity: Entity,
    ) {
        if let Some(state) = self.states.get_mut(&entity) {
            if let (Some(id), Some(p)) = (state.ragdoll.take(), physics.as_mut()) {
                let _ = p.destroy_ragdoll(id);
            }
            if let Some(transform) = world.get_mut::<Transform>(entity) {
                *transform = state.initial_transform;
            }
            state.time = 0.0;
            state.applied_time = 0.0;
            state.submitted_time = 0.0;
            state.pending = None;
            state.pose = state.skeleton.rest_pose();
        }
        if let Some(settings) = world.get_mut::<AnimationAuthoring>(entity) {
            settings.preview_time = 0.0;
            settings.event_count = 0;
            settings.last_event.clear();
            settings.status = "Ready".into();
        }
    }
    /// Advance live authored rigs and return named gameplay/sound hooks. The app
    /// may route the returned events; Details always displays the last event.
    pub fn update(
        &mut self,
        world: &mut World,
        mut physics: Option<&mut PhysicsWorld>,
        jobs: &mut somnium_jobs::JobSystem,
        delta_seconds: f32,
    ) -> Vec<(Entity, EventOccurrence)> {
        if !delta_seconds.is_finite() || delta_seconds < 0.0 {
            return Vec::new();
        }
        let entities: Vec<_> = world
            .entities()
            .filter(|e| world.get::<AnimationAuthoring>(*e).is_some())
            .collect();
        let removed: Vec<_> = self
            .states
            .keys()
            .filter(|e| !entities.contains(e))
            .copied()
            .collect();
        for entity in removed {
            self.reset(world, physics.as_deref_mut(), entity);
            self.states.remove(&entity);
        }
        let mut fired = Vec::new();
        for entity in entities {
            let settings = world.get::<AnimationAuthoring>(entity).unwrap().clone();
            let authored = source_settings(settings.clone());
            let rebuild = self
                .states
                .get(&entity)
                .is_none_or(|s| needs_rebuild(&s.authored, &authored, s.external));
            if rebuild {
                let transform = world.get::<Transform>(entity).copied().unwrap_or_default();
                let source = self
                    .states
                    .get(&entity)
                    .filter(|s| s.external)
                    .map(|s| (s.skeleton.clone(), s.source.clone()));
                let source = source
                    .map(Ok)
                    .unwrap_or_else(|| make_preview(&settings).map(|(s, c)| (Arc::new(s), c)));
                let result = source.and_then(|(s, c)| {
                    build_state(
                        s,
                        c,
                        settings.clone(),
                        transform,
                        self.states.get(&entity).is_some_and(|s| s.external),
                    )
                });
                match result {
                    Ok((mut state, report)) => {
                        if let Some(old) = self.states.remove(&entity) {
                            state.time = old.time;
                            state.applied_time = old.applied_time;
                            state.initial_transform = old.initial_transform;
                            state.ragdoll = old.ragdoll;
                        }
                        self.states.insert(entity, state);
                        world
                            .get_mut::<AnimationAuthoring>(entity)
                            .unwrap()
                            .compression_report = report;
                    }
                    Err(error) => {
                        world.get_mut::<AnimationAuthoring>(entity).unwrap().status = error;
                        continue;
                    }
                }
            }
            let state = self.states.get_mut(&entity).unwrap();
            if (state.authored.ragdoll_swing != authored.ragdoll_swing
                || state.authored.ragdoll_twist != authored.ragdoll_twist)
                && let (Some(id), Some(p)) = (state.ragdoll.take(), physics.as_deref_mut())
            {
                let _ = p.destroy_ragdoll(id);
            }
            state.authored = authored;
            let result = advance_rig(
                state,
                world,
                physics.as_deref_mut(),
                jobs,
                entity,
                &settings,
                delta_seconds,
            );
            let fields = world.get_mut::<AnimationAuthoring>(entity).unwrap();
            match result {
                Ok(events) => {
                    fields.preview_time = state.applied_time;
                    fields.status = if state.pending.is_some() {
                        "Evaluating pose jobs"
                    } else if settings.playing {
                        "Playing"
                    } else {
                        "Paused"
                    }
                    .into();
                    fields.event_count = fields.event_count.saturating_add(events.len() as u32);
                    for event in events {
                        fields.last_event = event.event.name.clone();
                        fired.push((entity, event));
                    }
                }
                Err(error) => {
                    // An invalid live edit must not build a movement/event debt
                    // that is replayed when the designer repairs the setting.
                    state.pending = None;
                    state.time = state.applied_time;
                    state.submitted_time = state.applied_time;
                    fields.status = error;
                }
            }
        }
        fired
    }
}

fn entity_matrix(
    world: &World,
    mut entity: Entity,
    moved_ancestor: Option<(Entity, Mat4)>,
) -> Option<Mat4> {
    // Evaluate current local edits, including this frame's accepted root movement,
    // rather than reading the previous transform-propagation cache.
    let mut matrix = Mat4::IDENTITY;
    let mut visited = std::collections::HashSet::new();
    loop {
        if !visited.insert(entity) || !world.is_alive(entity) {
            return None;
        }
        if let Some((ancestor, moved)) = moved_ancestor
            && ancestor == entity
        {
            return Some(moved * matrix);
        }
        matrix = world.get::<Transform>(entity)?.to_matrix() * matrix;
        match world.get::<Parent>(entity) {
            Some(parent) if parent.entity != Entity::DANGLING => entity = parent.entity,
            _ => return matrix.is_finite().then_some(matrix),
        }
    }
}
fn advance_rig(
    state: &mut BoundAnimation,
    world: &mut World,
    mut physics: Option<&mut PhysicsWorld>,
    jobs: &mut somnium_jobs::JobSystem,
    entity: Entity,
    settings: &AnimationAuthoring,
    dt: f32,
) -> Result<Vec<EventOccurrence>, String> {
    if settings.playing {
        state.time += dt * settings.speed;
    }
    if !state.time.is_finite() {
        return Err("Playback time must be finite".into());
    }
    let parameters = state.graph.parameters().instantiate();
    let (mut pose, time) = if settings.pose_jobs {
        if state.pending.is_none() {
            state.submitted_time = state.time;
            state.pending = Some(
                state
                    .graph
                    .pose_tasks(state.skeleton.clone(), &parameters, state.time)
                    .map_err(|e| format!("Pose jobs: {e:?}"))?
                    .start(4),
            );
        }
        let Some(pose) = state
            .pending
            .as_mut()
            .unwrap()
            .poll(jobs)
            .map_err(|e| format!("Pose jobs: {e:?}"))?
        else {
            return Ok(Vec::new());
        };
        state.pending = None;
        (pose, state.submitted_time)
    } else {
        state.pending = None;
        (
            state
                .graph
                .evaluate(
                    &state.skeleton,
                    &parameters,
                    state.time,
                    0,
                    &mut PoseCache::default(),
                )
                .map_err(|e| format!("Pose: {e:?}"))?,
            state.time,
        )
    };
    let local_transform = world
        .get::<Transform>(entity)
        .copied()
        .ok_or("Rig needs a Transform")?;
    let parent_world = match world.get::<Parent>(entity) {
        Some(parent) if parent.entity != Entity::DANGLING => {
            entity_matrix(world, parent.entity, None)
                .ok_or("Rig parent hierarchy needs live, acyclic Transforms")?
        }
        _ => Mat4::IDENTITY,
    };
    let mut character = parent_world * local_transform.to_matrix();
    if !character.is_finite() || character.determinant().abs() < 1e-8 {
        return Err("Rig world transform must be finite and invertible".into());
    }
    let mut final_local = local_transform;
    let root = u16::try_from(settings.root_joint).map_err(|_| "Root joint out of range")?;
    if settings.root_motion && settings.ragdoll_weight == 0.0 {
        let (scale, rotation, translation) = character.to_scale_rotation_translation();
        if !scale.abs_diff_eq(Vec3::ONE, 1e-4)
            || !Mat4::from_rotation_translation(rotation, translation).abs_diff_eq(character, 1e-4)
        {
            return Err("Root motion needs unit world scale; apply rig/parent scale first".into());
        }
        let mut transform = Transform {
            translation,
            rotation,
            scale,
        };
        let (_, motion) = state
            .clip
            .sample_root_motion(
                &state.skeleton,
                root,
                state.applied_time,
                time,
                Playback::LOOPING,
            )
            .map_err(|e| format!("Root motion: {e:?}"))?;
        let physics = physics
            .as_deref_mut()
            .ok_or("Root motion preview needs the physics world")?;
        crate::animation_motion::move_character_root_motion(
            &mut transform,
            motion,
            physics,
            crate::animation_motion::RootMotionCharacter {
                half_height: settings.capsule_half_height,
                radius: settings.capsule_radius,
                skin: 0.005,
                body: None,
            },
        )
        .map_err(|e| format!("Collision: {e:?}"))?;
        character = transform.to_matrix();
        let local = parent_world.inverse() * character;
        let (scale, rotation, translation) = local.to_scale_rotation_translation();
        final_local = Transform {
            translation,
            rotation,
            scale,
        };
        if !final_local.to_matrix().abs_diff_eq(local, 1e-4) {
            return Err(
                "Root movement creates parent scale shear; apply parent scale first".into(),
            );
        }
        let root = &mut pose.local[root as usize];
        let rest = state.skeleton.rest()[settings.root_joint as usize];
        root.translation = rest.translation;
        root.rotation = rest.rotation;
    }
    let inverse = character.inverse();
    let ik = TwoBoneIk {
        root: settings.ik_root as u16,
        middle: settings.ik_middle as u16,
        end: settings.ik_end as u16,
    };
    if settings.ik_weight > 0.0 {
        let target = entity_matrix(world, settings.reach_target, Some((entity, character)))
            .ok_or("Choose a live Reach Target")?
            .w_axis
            .truncate();
        ik.solve(
            &mut pose,
            &state.skeleton,
            inverse.transform_point3(target),
            settings.pole,
            settings.ik_weight,
        )
        .map_err(|e| format!("Limb IK: {e:?}"))?;
    }
    if settings.foot_ik {
        let physics = physics
            .as_deref_mut()
            .ok_or("Foot IK needs the physics world")?;
        let mut model = vec![Mat4::IDENTITY; state.skeleton.len()];
        pose.to_model_space(&state.skeleton, &mut model);
        let foot = model
            .get(settings.ik_end as usize)
            .ok_or("Foot joint out of range")?
            .w_axis
            .truncate();
        let origin = character.transform_point3(foot) + Vec3::Y * 0.5;
        let direction = -Vec3::Y * 1.5;
        let hit = physics
            .cast_capsule(CapsuleSweep {
                position: origin,
                rotation: Quat::IDENTITY,
                displacement: direction,
                half_height: 0.01,
                radius: 0.02,
                ignore_body: None,
            })
            .map_err(|e| format!("Ground probe: {e:?}"))?;
        let contact = hit.map(|h| GroundContact {
            position: inverse.transform_point3(
                origin + direction * h.fraction - Vec3::Y * 0.01 - h.normal * 0.02,
            ),
            // World-to-model normals use the transpose of model-to-world.
            normal: character
                .transpose()
                .transform_vector3(h.normal)
                .normalize(),
            sole_height: settings.sole_height,
            local_up: Vec3::Y,
        });
        ik.adapt_foot(&mut pose, &state.skeleton, contact, settings.pole, 1.0)
            .map_err(|e| format!("Foot IK: {e:?}"))?;
    }
    if settings.look_weight > 0.0 {
        let target = entity_matrix(world, settings.look_target, Some((entity, character)))
            .ok_or("Choose a live Look Target")?
            .w_axis
            .truncate();
        look_at(
            &mut pose,
            &state.skeleton,
            settings.look_joint as u16,
            Vec3::Z,
            inverse.transform_point3(target),
            settings.look_limit,
            settings.look_weight,
        )
        .map_err(|e| format!("Look at: {e:?}"))?;
    }
    if settings.ragdoll_weight > 0.0 {
        let (scale, rotation, translation) = character.to_scale_rotation_translation();
        if !scale.abs_diff_eq(Vec3::ONE, 1e-4)
            || !Mat4::from_rotation_translation(rotation, translation).abs_diff_eq(character, 1e-4)
        {
            return Err("Ragdoll needs unit world scale; apply rig/parent scale first".into());
        }
        let physics = physics
            .as_deref_mut()
            .ok_or("Ragdoll preview needs the physics world")?;
        if state.ragdoll.is_none() {
            let mut model = vec![Mat4::IDENTITY; state.skeleton.len()];
            pose.to_model_space(&state.skeleton, &mut model);
            let parts: Vec<_> = model
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let parent = state.skeleton.parents()[i];
                    let body = character * *m;
                    let (_, rotation, position) = body.to_scale_rotation_translation();
                    let anchor = if parent == NO_PARENT {
                        position
                    } else {
                        (position + (character * model[parent as usize]).w_axis.truncate()) * 0.5
                    };
                    RagdollPart {
                        parent: (parent != NO_PARENT).then_some(parent),
                        position,
                        rotation,
                        anchor,
                        half_height: 0.15,
                        radius: 0.1,
                        swing_limit: settings.ragdoll_swing,
                        twist_limit: settings.ragdoll_twist,
                    }
                })
                .collect();
            state.ragdoll = Some(
                physics
                    .create_ragdoll(&parts)
                    .map_err(|e| format!("Ragdoll: {e:?}"))?,
            );
        }
        let bindings: Vec<_> = (0..state.skeleton.len())
            .map(|i| (i as u16, i, Mat4::IDENTITY))
            .collect();
        pose = crate::animation_motion::blend_jolt_ragdoll(
            &pose,
            &state.skeleton,
            physics,
            state.ragdoll.unwrap(),
            character,
            &bindings,
            settings.ragdoll_weight,
        )
        .map_err(|e| format!("Ragdoll blend: {e:?}"))?;
    } else if let (Some(id), Some(physics)) = (state.ragdoll.take(), physics.as_deref_mut()) {
        let _ = physics.destroy_ragdoll(id);
    }
    let events = state
        .events
        .sample(state.applied_time, time, Playback::LOOPING, 256)
        .map_err(|e| format!("Events: {e:?}"))?;
    let mut matrices = vec![Mat4::IDENTITY; state.skeleton.len()];
    pose.to_model_space(&state.skeleton, &mut matrices);
    let markers: Vec<_> = world
        .entities()
        .filter_map(|e| {
            world
                .get::<AnimationPreviewJoint>(e)
                .filter(|j| j.owner == entity)
                .map(|j| (e, j.joint))
        })
        .collect();
    for (marker, joint) in markers {
        if let (Some(m), Some(t)) = (
            matrices.get(joint as usize),
            world.get_mut::<Transform>(marker),
        ) {
            let (_, rotation, translation) = m.to_scale_rotation_translation();
            t.translation = translation;
            t.rotation = rotation;
        }
    }
    *world.get_mut::<Transform>(entity).unwrap() = final_local;
    state.pose = pose;
    state.applied_time = time;
    Ok(events)
}
