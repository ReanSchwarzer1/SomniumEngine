//! Animation/physics seam. Collision consumes this frame's root displacement;
//! ragdoll poses are world-space snapshots converted before animation blending.
use glam::{Mat4, Vec3};
use somnium_anim::{
    AppliedMotion, MotionError, MotionHit, Pose, PoseEditError, RootMotion, Skeleton,
};

/// An upright kinematic capsule whose pose is owned by animation/gameplay.
/// A body handle, when present, is ignored by sweeps and updated after movement.
#[derive(Clone, Copy, Debug)]
pub struct RootMotionCharacter {
    /// Half the capsule's cylindrical height, in metres.
    pub half_height: f32,
    /// Capsule radius, in metres.
    pub radius: f32,
    /// Clearance retained before a blocking contact.
    pub skin: f32,
    /// Optional live kinematic body to update after the sweep.
    pub body: Option<somnium_physics::body::BodyId>,
}

/// Move the character through actual Jolt capsule sweeps and commit accepted motion.
pub fn move_character_root_motion(
    transform: &mut crate::Transform,
    motion: RootMotion,
    physics: &mut somnium_physics::world::PhysicsWorld,
    character: RootMotionCharacter,
) -> Result<AppliedMotion, MotionError> {
    if !character.half_height.is_finite()
        || character.half_height <= 0.0
        || !character.radius.is_finite()
        || character.radius <= 0.0
        || character.body.is_some_and(|body| !body.is_valid())
    {
        return Err(MotionError::InvalidTransform);
    }
    let result = apply_root_motion(
        transform,
        motion,
        character.skin,
        |position, displacement| match physics.cast_capsule(
            somnium_physics::animation::CapsuleSweep {
                position,
                displacement,
                rotation: glam::Quat::IDENTITY,
                half_height: character.half_height,
                radius: character.radius,
                ignore_body: character.body,
            },
        ) {
            Ok(hit) => hit.map(|h| MotionHit {
                fraction: h.fraction,
                normal: h.normal,
            }),
            Err(_) => Some(MotionHit {
                fraction: f32::NAN,
                normal: Vec3::ZERO,
            }),
        },
    )?;
    if let Some(body) = character.body {
        physics.set_position(body, transform.translation, true);
        physics.set_rotation(body, transform.rotation, true);
    }
    Ok(result)
}

#[derive(Clone, Debug, PartialEq)]
/// Failure at the physics-to-animation pose seam.
pub enum RagdollBlendError {
    /// The ragdoll handle or physical snapshot was invalid.
    Physics(somnium_physics::animation::AnimationPhysicsError),
    /// Joint mapping or model-space pose conversion failed.
    Pose(PoseEditError),
}

/// Blend a world-owned, constrained Jolt ragdoll. Mapping entries are
/// (skeleton joint, ragdoll part, body-to-joint offset). A stale ragdoll handle
/// is rejected before a body query, including after explicit destruction.
pub fn blend_jolt_ragdoll(
    animation: &Pose,
    skeleton: &Skeleton,
    physics: &somnium_physics::world::PhysicsWorld,
    ragdoll: somnium_physics::animation::RagdollId,
    character_world: Mat4,
    bindings: &[(somnium_anim::JointIndex, usize, Mat4)],
    weight: f32,
) -> Result<Pose, RagdollBlendError> {
    let bodies = physics
        .ragdoll_pose(ragdoll)
        .map_err(RagdollBlendError::Physics)?;
    if !character_world.is_finite() || character_world.determinant().abs() < 1e-8 {
        return Err(RagdollBlendError::Pose(PoseEditError::InvalidTarget));
    }
    let inverse = character_world.inverse();
    let mut model = vec![None; skeleton.len()];
    for &(joint, body, offset) in bindings {
        let world = bodies
            .get(body)
            .ok_or(RagdollBlendError::Pose(PoseEditError::InvalidJoint))?;
        let slot = model
            .get_mut(joint as usize)
            .ok_or(RagdollBlendError::Pose(PoseEditError::InvalidJoint))?;
        if slot.is_some() || !offset.is_finite() {
            return Err(RagdollBlendError::Pose(PoseEditError::InvalidTarget));
        }
        *slot = Some(inverse * *world * offset);
    }
    somnium_anim::blend_ragdoll(animation, skeleton, &model, weight)
        .map_err(RagdollBlendError::Pose)
}

/// Apply local root motion using the same shape sweep that moves the character.
/// Rotation is accepted in full; translational collision rejection is discarded
/// rather than carried to the next animation frame. No renderer is involved.
pub fn apply_root_motion(
    transform: &mut crate::Transform,
    motion: RootMotion,
    skin: f32,
    sweep: impl FnMut(Vec3, Vec3) -> Option<MotionHit>,
) -> Result<AppliedMotion, MotionError> {
    if !motion.is_finite()
        || !transform.translation.is_finite()
        || !transform.rotation.is_finite()
        || (transform.rotation.length_squared() - 1.0).abs() > 1e-3
    {
        return Err(MotionError::InvalidTransform);
    }
    let result = somnium_anim::collide_and_slide(
        transform.translation,
        transform.rotation * motion.translation,
        skin,
        sweep,
    )?;
    transform.translation += result.applied;
    transform.rotation = (transform.rotation * motion.rotation).normalize();
    Ok(result)
}

/// Read already simulated Jolt body transforms into a pose. Each mapping names
/// a skeleton joint, a live physics body and its fixed body-to-joint transform.
/// Ownership of those bodies stays with the caller's ragdoll lifecycle.
pub fn blend_physics_bodies(
    animation: &Pose,
    skeleton: &Skeleton,
    physics: &somnium_physics::world::PhysicsWorld,
    character_world: Mat4,
    bindings: &[(
        somnium_anim::JointIndex,
        somnium_physics::body::BodyId,
        Mat4,
    )],
    weight: f32,
) -> Result<Pose, PoseEditError> {
    if !character_world.is_finite() || character_world.determinant().abs() < 1e-8 {
        return Err(PoseEditError::InvalidTarget);
    }
    let inverse = character_world.inverse();
    let mut model = vec![None; skeleton.len()];
    for &(joint, body, body_to_joint) in bindings {
        let slot = model
            .get_mut(joint as usize)
            .ok_or(PoseEditError::InvalidJoint)?;
        if slot.is_some() || !body.is_valid() || !body_to_joint.is_finite() {
            return Err(PoseEditError::InvalidTarget);
        }
        let world =
            Mat4::from_rotation_translation(physics.get_rotation(body), physics.get_position(body));
        *slot = Some(inverse * world * body_to_joint);
    }
    somnium_anim::blend_ragdoll(animation, skeleton, &model, weight)
}
