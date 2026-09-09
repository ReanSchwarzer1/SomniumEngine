//! Jolt capsule sweeps and constrained ragdolls. Native ownership stays with
//! PhysicsWorld, so bodies/constraints are removed before the native world dies.
use crate::{body::BodyId, world::PhysicsWorld};
use glam::{Mat4, Quat, Vec3};
use somnium_physics_sys::*;
use std::{
    ffi::c_void,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RagdollId(u64);
pub(crate) struct NativeRagdoll {
    pub pointer: *mut c_void,
    bodies: Vec<BodyId>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RagdollPart {
    pub parent: Option<u16>,
    pub position: Vec3,
    pub rotation: Quat,
    pub half_height: f32,
    pub radius: f32,
    /// World-space swing/twist anchor shared by this body and its parent.
    pub anchor: Vec3,
    pub swing_limit: f32,
    pub twist_limit: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimationPhysicsError {
    InvalidDescriptor,
    UnknownRagdoll,
    BodyCapacity,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapsuleSweep {
    pub position: Vec3,
    pub rotation: Quat,
    pub displacement: Vec3,
    pub half_height: f32,
    pub radius: f32,
    pub ignore_body: Option<BodyId>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapsuleHit {
    pub fraction: f32,
    pub normal: Vec3,
    pub body: BodyId,
}
fn valid_rotation(rotation: Quat) -> bool {
    rotation.is_finite() && (rotation.length_squared() - 1.0).abs() < 1e-3
}
fn valid_shape(half_height: f32, radius: f32) -> bool {
    half_height.is_finite() && half_height > 0.0 && radius.is_finite() && radius > 0.0
}

impl PhysicsWorld {
    /// Create one capsule per joint with actual Jolt swing/twist constraints.
    /// Parents precede children and only index zero may be a root. Adjacent
    /// parts do not collide. Handles from another world are rejected.
    pub fn create_ragdoll(
        &mut self,
        parts: &[RagdollPart],
    ) -> Result<RagdollId, AnimationPhysicsError> {
        if parts.is_empty()
            || parts.len() > u16::MAX as usize
            || parts.iter().enumerate().any(|(i, p)| {
                (if i == 0 {
                    p.parent.is_some()
                } else {
                    p.parent.is_none_or(|parent| parent as usize >= i)
                }) || !p.position.is_finite()
                    || !valid_rotation(p.rotation)
                    || !p.anchor.is_finite()
                    || !valid_shape(p.half_height, p.radius)
                    || !p.swing_limit.is_finite()
                    || !p.twist_limit.is_finite()
                    || !(0.0..=std::f32::consts::PI).contains(&p.swing_limit)
                    || !(0.0..=std::f32::consts::PI).contains(&p.twist_limit)
            })
        {
            return Err(AnimationPhysicsError::InvalidDescriptor);
        }
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = RagdollId(NEXT_ID.fetch_add(1, Ordering::Relaxed));
        if id.0 > u32::MAX as u64 {
            return Err(AnimationPhysicsError::BodyCapacity);
        }
        let native: Vec<_> = parts
            .iter()
            .map(|p| JphRagdollPart {
                parent: p.parent.map_or(-1, i32::from),
                position: p.position.to_array(),
                rotation: p.rotation.to_array(),
                half_height: p.half_height,
                radius: p.radius,
                anchor: p.anchor.to_array(),
                swing_limit: p.swing_limit,
                twist_limit: p.twist_limit,
            })
            .collect();
        // SAFETY: descriptors are validated, the contiguous array lives for the
        // call, and Jolt copies settings. This world owns the returned reference.
        let pointer = unsafe {
            jph_ragdoll_create(
                self.system,
                native.as_ptr(),
                native.len() as u32,
                id.0 as u32,
            )
        };
        if pointer.is_null() {
            return Err(AnimationPhysicsError::BodyCapacity);
        }
        let bodies = (0..parts.len())
            .map(|i| BodyId::from_index(unsafe { jph_ragdoll_body(pointer, i as u32) }))
            .collect();
        self.ragdolls.insert(id, NativeRagdoll { pointer, bodies });
        Ok(id)
    }
    pub fn ragdoll_pose(&self, id: RagdollId) -> Result<Vec<Mat4>, AnimationPhysicsError> {
        let ragdoll = self
            .ragdolls
            .get(&id)
            .ok_or(AnimationPhysicsError::UnknownRagdoll)?;
        Ok(ragdoll
            .bodies
            .iter()
            .map(|&body| {
                Mat4::from_rotation_translation(self.get_rotation(body), self.get_position(body))
            })
            .collect())
    }
    /// Seed/recover a ragdoll from animation, clearing old velocities and warm
    /// start impulses. All transforms are validated before touching any body.
    pub fn set_ragdoll_pose(
        &mut self,
        id: RagdollId,
        matrices: &[Mat4],
    ) -> Result<(), AnimationPhysicsError> {
        let ragdoll = self
            .ragdolls
            .get(&id)
            .ok_or(AnimationPhysicsError::UnknownRagdoll)?;
        if matrices.len() != ragdoll.bodies.len() {
            return Err(AnimationPhysicsError::InvalidDescriptor);
        }
        let transforms: Result<Vec<_>, _> = matrices
            .iter()
            .map(|m| {
                if !m.is_finite() || m.determinant().abs() < 1e-8 {
                    return Err(AnimationPhysicsError::InvalidDescriptor);
                }
                let (scale, rotation, translation) = m.to_scale_rotation_translation();
                if !scale.abs_diff_eq(Vec3::ONE, 1e-4)
                    || !valid_rotation(rotation)
                    || !m.abs_diff_eq(Mat4::from_rotation_translation(rotation, translation), 1e-4)
                {
                    return Err(AnimationPhysicsError::InvalidDescriptor);
                }
                Ok((rotation, translation))
            })
            .collect();
        let transforms = transforms?;
        let bodies = ragdoll.bodies.clone();
        let pointer = ragdoll.pointer;
        for (body, (rotation, position)) in bodies.into_iter().zip(transforms) {
            self.set_position(body, position, true);
            self.set_rotation(body, rotation, true);
            self.set_linear_velocity(body, Vec3::ZERO);
            self.set_angular_velocity(body, Vec3::ZERO);
        }
        // SAFETY: the world-owned ragdoll remains alive throughout this method.
        unsafe {
            jph_ragdoll_reset(pointer);
        }
        Ok(())
    }
    pub fn destroy_ragdoll(&mut self, id: RagdollId) -> Result<(), AnimationPhysicsError> {
        let ragdoll = self
            .ragdolls
            .remove(&id)
            .ok_or(AnimationPhysicsError::UnknownRagdoll)?;
        // SAFETY: removing from the owner map transfers its sole native reference.
        unsafe {
            jph_ragdoll_destroy(ragdoll.pointer);
        }
        Ok(())
    }
    /// Sweep an upright or rotated capsule through real world geometry. Initial
    /// contacts opposing travel block; tangential/separating contacts are ignored.
    pub fn cast_capsule(
        &self,
        cast: CapsuleSweep,
    ) -> Result<Option<CapsuleHit>, AnimationPhysicsError> {
        if !cast.position.is_finite()
            || !cast.displacement.is_finite()
            || !valid_rotation(cast.rotation)
            || !valid_shape(cast.half_height, cast.radius)
        {
            return Err(AnimationPhysicsError::InvalidDescriptor);
        }
        if cast.displacement.length_squared() < 1e-12 {
            return Ok(None);
        }
        let input = JphCapsuleCast {
            position: cast.position.to_array(),
            rotation: cast.rotation.to_array(),
            displacement: cast.displacement.to_array(),
            half_height: cast.half_height,
            radius: cast.radius,
            ignore_body: cast.ignore_body.unwrap_or(BodyId::INVALID).index(),
        };
        let mut output = JphCastHit::default();
        // SAFETY: both pointers reference initialized stack objects and the query
        // reads the live world; Jolt acquires body locks during the query.
        let hit = unsafe { jph_cast_capsule(self.system, &input, &mut output) };
        Ok((hit != 0).then(|| CapsuleHit {
            fraction: output.fraction,
            normal: Vec3::from_array(output.normal),
            body: BodyId::from_index(output.body),
        }))
    }
}
