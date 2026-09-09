//! Post-blend pose operations. All targets are in character model space;
//! physics adapters perform world conversion once before crossing this seam.
use crate::{JointIndex, NO_PARENT, Pose, Skeleton, Transform};
use glam::{Mat4, Quat, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoseEditError {
    SkeletonMismatch,
    InvalidJoint,
    InvalidChain,
    InvalidTarget,
    UnsupportedScale,
}

fn model(pose: &Pose, skeleton: &Skeleton) -> Result<Vec<Mat4>, PoseEditError> {
    let mut result = vec![Mat4::IDENTITY; skeleton.len()];
    if !pose.to_model_space(skeleton, &mut result) {
        return Err(PoseEditError::SkeletonMismatch);
    }
    if pose.local.iter().any(|t| {
        !t.translation.is_finite()
            || !t.rotation.is_finite()
            || (t.rotation.length_squared() - 1.0).abs() > 1e-3
    }) {
        return Err(PoseEditError::InvalidTarget);
    }
    // Reject scale/shear rather than stretching limbs or decomposing an invalid rotation.
    if pose
        .local
        .iter()
        .any(|t| !t.scale.abs_diff_eq(Vec3::ONE, 1e-5))
    {
        return Err(PoseEditError::UnsupportedScale);
    }
    Ok(result)
}
fn rotation(m: Mat4) -> Quat {
    m.to_scale_rotation_translation().1.normalize()
}
fn set_model_rotation(
    pose: &mut Pose,
    skeleton: &Skeleton,
    matrices: &[Mat4],
    joint: usize,
    value: Quat,
) {
    let parent = skeleton.parents()[joint];
    pose.local[joint].rotation = if parent == NO_PARENT {
        value
    } else {
        rotation(matrices[parent as usize]).conjugate() * value
    }
    .normalize();
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwoBoneIk {
    pub root: JointIndex,
    pub middle: JointIndex,
    pub end: JointIndex,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IkResult {
    pub requested: Vec3,
    pub reached: Vec3,
    pub clamped: bool,
}

impl TwoBoneIk {
    /// Rotate a parent/child/grandchild chain, preserving local translations and
    /// lengths. Pole is a model-space point defining the bend plane. Unreachable
    /// targets are clamped; zero-length chains fail without changing the pose.
    pub fn solve(
        self,
        pose: &mut Pose,
        skeleton: &Skeleton,
        target: Vec3,
        pole: Vec3,
        weight: f32,
    ) -> Result<IkResult, PoseEditError> {
        if !target.is_finite()
            || !pole.is_finite()
            || !weight.is_finite()
            || !(0.0..=1.0).contains(&weight)
        {
            return Err(PoseEditError::InvalidTarget);
        }
        let (r, m, e) = (self.root as usize, self.middle as usize, self.end as usize);
        if e >= skeleton.len() || m >= skeleton.len() || r >= skeleton.len() {
            return Err(PoseEditError::InvalidJoint);
        }
        if skeleton.parents()[m] != self.root || skeleton.parents()[e] != self.middle {
            return Err(PoseEditError::InvalidChain);
        }
        let mut matrices = model(pose, skeleton)?;
        let (a, b, c) = (
            matrices[r].w_axis.truncate(),
            matrices[m].w_axis.truncate(),
            matrices[e].w_axis.truncate(),
        );
        let (upper, lower) = (a.distance(b), b.distance(c));
        if upper < 1e-5 || lower < 1e-5 {
            return Err(PoseEditError::InvalidChain);
        }
        let requested_distance = a.distance(target);
        let direction = (target - a)
            .try_normalize()
            .unwrap_or_else(|| (c - a).try_normalize().unwrap_or(Vec3::X));
        let distance = requested_distance.clamp((upper - lower).abs().max(1e-5), upper + lower);
        let pole_direction = pole - a;
        let bend = (pole_direction - direction * direction.dot(pole_direction))
            .try_normalize()
            .unwrap_or_else(|| direction.any_orthonormal_vector());
        let along = (upper * upper + distance * distance - lower * lower) / (2.0 * distance);
        let height = (upper * upper - along * along).max(0.0).sqrt();
        let elbow = a + direction * along + bend * height;
        let reached_target = a + direction * distance;
        let mut solved = pose.clone();
        let root_rotation = Quat::from_rotation_arc((b - a).normalize(), (elbow - a).normalize())
            * rotation(matrices[r]);
        set_model_rotation(&mut solved, skeleton, &matrices, r, root_rotation);
        solved.to_model_space(skeleton, &mut matrices);
        let current_b = matrices[m].w_axis.truncate();
        let current_c = matrices[e].w_axis.truncate();
        let middle_rotation = Quat::from_rotation_arc(
            (current_c - current_b).normalize(),
            (reached_target - current_b).normalize(),
        ) * rotation(matrices[m]);
        set_model_rotation(&mut solved, skeleton, &matrices, m, middle_rotation);
        pose.local[r] = pose.local[r].blend(solved.local[r], weight);
        pose.local[m] = pose.local[m].blend(solved.local[m], weight);
        pose.to_model_space(skeleton, &mut matrices);
        Ok(IkResult {
            requested: target,
            reached: matrices[e].w_axis.truncate(),
            clamped: (requested_distance - distance).abs() > 1e-5,
        })
    }

    /// Adapt an ankle to a ground hit with sole clearance, then rotate the local
    /// foot-up axis onto the ground normal. No hit leaves the pose unchanged.
    pub fn adapt_foot(
        self,
        pose: &mut Pose,
        skeleton: &Skeleton,
        ground: Option<GroundContact>,
        pole: Vec3,
        weight: f32,
    ) -> Result<Option<IkResult>, PoseEditError> {
        let Some(ground) = ground else {
            return Ok(None);
        };
        if !ground.position.is_finite()
            || !ground.normal.is_finite()
            || ground.normal.length_squared() < 1e-8
            || !ground.sole_height.is_finite()
            || ground.sole_height < 0.0
            || !ground.local_up.is_finite()
            || ground.local_up.length_squared() < 1e-8
        {
            return Err(PoseEditError::InvalidTarget);
        }
        let mut edited = pose.clone();
        let normal = ground.normal.normalize();
        let result = self.solve(
            &mut edited,
            skeleton,
            ground.position + normal * ground.sole_height,
            pole,
            weight,
        )?;
        let matrices = model(&edited, skeleton)?;
        let joint = self.end as usize;
        let current = rotation(matrices[joint]);
        let target =
            Quat::from_rotation_arc((current * ground.local_up).normalize(), normal) * current;
        set_model_rotation(
            &mut edited,
            skeleton,
            &matrices,
            joint,
            current.slerp(target, weight),
        );
        *pose = edited;
        Ok(Some(result))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundContact {
    pub position: Vec3,
    pub normal: Vec3,
    pub sole_height: f32,
    pub local_up: Vec3,
}

/// Aim one joint's local forward axis at a model-space target with an angular
/// limit. The cone limit applies before the blend weight.
pub fn look_at(
    pose: &mut Pose,
    skeleton: &Skeleton,
    joint: JointIndex,
    local_forward: Vec3,
    target: Vec3,
    max_angle: f32,
    weight: f32,
) -> Result<(), PoseEditError> {
    if joint as usize >= skeleton.len() {
        return Err(PoseEditError::InvalidJoint);
    }
    if !local_forward.is_finite()
        || local_forward.length_squared() < 1e-8
        || !target.is_finite()
        || !max_angle.is_finite()
        || !(0.0..=std::f32::consts::PI).contains(&max_angle)
        || !weight.is_finite()
        || !(0.0..=1.0).contains(&weight)
    {
        return Err(PoseEditError::InvalidTarget);
    }
    let matrices = model(pose, skeleton)?;
    let joint = joint as usize;
    let current = rotation(matrices[joint]);
    let Some(direction) = (target - matrices[joint].w_axis.truncate()).try_normalize() else {
        return Ok(());
    };
    let delta = Quat::from_rotation_arc((current * local_forward).normalize(), direction);
    let angle = delta.to_axis_angle().1.abs();
    let fraction = if angle > 1e-6 {
        (max_angle / angle).min(1.0) * weight
    } else {
        0.0
    };
    set_model_rotation(
        pose,
        skeleton,
        &matrices,
        joint,
        Quat::IDENTITY.slerp(delta, fraction) * current,
    );
    Ok(())
}

/// Blend a physics snapshot into animation. `None` joints keep animated locals
/// under their already blended parent; mapped joints are supplied in model
/// space. The same weight ramp handles both entering and recovering from a
/// ragdoll. Physics remains authoritative over the snapshot, never the palette.
pub fn blend_ragdoll(
    animation: &Pose,
    skeleton: &Skeleton,
    physical_model: &[Option<Mat4>],
    weight: f32,
) -> Result<Pose, PoseEditError> {
    if !weight.is_finite()
        || !(0.0..=1.0).contains(&weight)
        || physical_model.len() != skeleton.len()
    {
        return Err(PoseEditError::InvalidTarget);
    }
    model(animation, skeleton)?;
    let mut result = animation.clone();
    let mut physical = vec![Mat4::IDENTITY; skeleton.len()];
    for i in 0..skeleton.len() {
        let parent = skeleton.parents()[i];
        let parent_matrix = if parent == NO_PARENT {
            Mat4::IDENTITY
        } else {
            physical[parent as usize]
        };
        physical[i] = physical_model[i].unwrap_or(parent_matrix * animation.local[i].to_matrix());
        if !physical[i].is_finite() || physical[i].determinant().abs() < 1e-8 {
            return Err(PoseEditError::InvalidTarget);
        }
        let local = parent_matrix.inverse() * physical[i];
        let (scale, rotation, translation) = local.to_scale_rotation_translation();
        if !scale.is_finite()
            || !scale.abs_diff_eq(Vec3::ONE, 1e-3)
            || !rotation.is_finite()
            || !local.abs_diff_eq(Mat4::from_rotation_translation(rotation, translation), 1e-3)
        {
            return Err(PoseEditError::UnsupportedScale);
        }
        result.local[i] = animation.local[i].blend(
            Transform {
                translation,
                rotation: rotation.normalize(),
                scale: Vec3::ONE,
            },
            weight,
        );
    }
    Ok(result)
}
