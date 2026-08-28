//! MORROWIND-U — Seam 7: a pose is data, and the renderer does not know what
//! produced it.
//!
//! # The census finding this crate answers
//!
//! `MORROWIND-A`'s census found **zero occurrences of `bone` or `armature`** in
//! the entire tree. Somnium could render an open world with ray-traced global
//! illumination and could not move a character's arm.
//!
//! # The seam, stated once
//!
//! ```text
//!   somnium_anim          somnium_renderer
//!   ────────────          ────────────────
//!   Skeleton   ─┐
//!   Pose       ─┼──> [Mat4] ──> SkinningPalettes ──> skinning.wgsl
//!   (blend tree)┘     ^
//!                     └── the only thing that crosses
//! ```
//!
//! The renderer takes a flat array of matrices. It does not know whether they
//! came from a clip, a blend tree, a ragdoll, an IK solver or a test that typed
//! them in. **That is the entire seam**, and it is what lets MORROWIND-V add
//! blend trees and MORROWIND-W add IK without the renderer learning anything.
//!
//! # The invariant everything here depends on
//!
//! **`parents[i] < i` for every joint.** A skeleton is stored so that a parent
//! always precedes its children, which makes [`Pose::to_model_space`] one
//! forward pass with no recursion, no stack and no visited set. Import
//! guarantees it ([`Skeleton::new`] sorts, and [`Skeleton::is_well_formed`]
//! checks), because a glTF file is under no obligation to.

use glam::{Mat4, Quat, Vec3};

mod runtime;

pub use runtime::*;

/// A joint's local transform. Separate components rather than a matrix, because
/// blending is defined on these and not on matrices: interpolating two matrices
/// componentwise shears anything that was rotated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    #[must_use]
    pub fn to_matrix(self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Linear blend toward `other`.
    ///
    /// Translation and scale lerp; **rotation slerps**, and the difference is
    /// visible: lerping two quaternions 90° apart and normalising passes
    /// through the short way at the wrong *speed*, which is what makes a
    /// cheaply blended turn look like it hesitates in the middle.
    ///
    /// MORROWIND-V's blend trees are built on this and nothing else.
    #[must_use]
    pub fn blend(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            translation: self.translation.lerp(other.translation, t),
            rotation: self.rotation.slerp(other.rotation, t),
            scale: self.scale.lerp(other.scale, t),
        }
    }
}

/// Which skeleton a pose belongs to.
///
/// Checked rather than assumed: applying a pose to the wrong skeleton produces
/// a character folded inside out, which looks like a maths bug and is not one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SkeletonId(pub u32);

/// A joint index. `u16` because a skeleton with more than 65,535 joints is not
/// a skeleton, and because the GPU-side joint indices are `u16` too.
pub type JointIndex = u16;

/// The index that means "this joint has no parent".
pub const NO_PARENT: JointIndex = JointIndex::MAX;

/// A hierarchy of joints and its bind pose.
#[derive(Clone, Debug, PartialEq)]
pub struct Skeleton {
    id: SkeletonId,
    names: Vec<String>,
    /// Parent index per joint, or [`NO_PARENT`]. **Invariant: `parents[i] < i`.**
    parents: Vec<JointIndex>,
    /// Model-space-to-joint-space at bind time, one per joint.
    inverse_bind: Vec<Mat4>,
    /// The pose the skeleton was authored in. The origin every blend that names
    /// no other source starts from.
    rest: Vec<Transform>,
}

impl Skeleton {
    /// Build a skeleton, **reordering joints so every parent precedes its
    /// children**.
    ///
    /// Returns the new index of each input joint alongside the skeleton, since
    /// vertex joint indices in the source data refer to the *old* order and
    /// have to be remapped. Returning the map rather than remapping internally
    /// is deliberate: this crate does not know what a vertex is, and a caller
    /// that forgets to remap gets a compile error rather than a silently
    /// scrambled character.
    ///
    /// Returns `None` for a cycle or an out-of-range parent — a malformed file,
    /// not a panic.
    pub fn new(
        id: SkeletonId,
        names: Vec<String>,
        parents: Vec<JointIndex>,
        inverse_bind: Vec<Mat4>,
        rest: Vec<Transform>,
    ) -> Option<(Self, Vec<JointIndex>)> {
        let count = names.len();
        if parents.len() != count || inverse_bind.len() != count || rest.len() != count {
            return None;
        }
        if count > JointIndex::MAX as usize {
            return None;
        }

        // Topological order, parents first. Kahn's algorithm over a forest,
        // which also detects the cycle a hand-edited file can contain.
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
        let mut ready: Vec<usize> = Vec::new();
        for (index, &parent) in parents.iter().enumerate() {
            if parent == NO_PARENT {
                ready.push(index);
            } else {
                let parent = parent as usize;
                if parent >= count || parent == index {
                    return None;
                }
                children[parent].push(index);
            }
        }

        let mut order = Vec::with_capacity(count);
        while let Some(joint) = ready.pop() {
            order.push(joint);
            // Reversed, so siblings keep their authored order once `pop`
            // reverses them again. A skeleton whose joint order changes between
            // runs would make every cooked animation clip invalid.
            for &child in children[joint].iter().rev() {
                ready.push(child);
            }
        }
        if order.len() != count {
            // Unreached joints are in a cycle.
            return None;
        }

        let mut remap = vec![NO_PARENT; count];
        for (new_index, &old_index) in order.iter().enumerate() {
            remap[old_index] = new_index as JointIndex;
        }

        let skeleton = Self {
            id,
            names: order.iter().map(|&i| names[i].clone()).collect(),
            parents: order
                .iter()
                .map(|&i| {
                    let parent = parents[i];
                    if parent == NO_PARENT {
                        NO_PARENT
                    } else {
                        remap[parent as usize]
                    }
                })
                .collect(),
            inverse_bind: order.iter().map(|&i| inverse_bind[i]).collect(),
            rest: order.iter().map(|&i| rest[i]).collect(),
        };
        debug_assert!(skeleton.is_well_formed());
        Some((skeleton, remap))
    }

    /// The invariant, checkable.
    ///
    /// Public because it is the precondition [`Pose::to_model_space`]'s single
    /// forward pass rests on, and a caller constructing a skeleton by other
    /// means should be able to assert it rather than discover it.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        self.parents.iter().enumerate().all(|(index, &parent)| {
            parent == NO_PARENT || ((parent as usize) < index && (parent as usize) < self.len())
        })
    }

    #[must_use]
    pub fn id(&self) -> SkeletonId {
        self.id
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    #[must_use]
    pub fn parents(&self) -> &[JointIndex] {
        &self.parents
    }

    #[must_use]
    pub fn inverse_bind(&self) -> &[Mat4] {
        &self.inverse_bind
    }

    #[must_use]
    pub fn rest(&self) -> &[Transform] {
        &self.rest
    }

    /// Find a joint by name.
    ///
    /// Linear, and that is correct here: skeletons are tens to low hundreds of
    /// joints, this is called at bind time rather than per frame, and a
    /// `HashMap` on every skeleton would cost more memory than it saves time.
    /// MORROWIND-W's IK looks up by name once and holds the index.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<JointIndex> {
        self.names
            .iter()
            .position(|n| n == name)
            .map(|i| i as JointIndex)
    }

    /// The rest pose, as a pose.
    #[must_use]
    pub fn rest_pose(&self) -> Pose {
        Pose {
            skeleton: self.id,
            local: self.rest.clone(),
        }
    }
}

/// A skeleton's joints, in their local spaces, at one instant.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub skeleton: SkeletonId,
    pub local: Vec<Transform>,
}

impl Pose {
    /// A pose at the skeleton's rest.
    #[must_use]
    pub fn rest(skeleton: &Skeleton) -> Self {
        skeleton.rest_pose()
    }

    /// Compose local transforms into model space.
    ///
    /// **One forward pass, correct only because of the `parents[i] < i`
    /// invariant** — by the time joint `i` is reached, `out[parents[i]]` is
    /// already its final model-space matrix.
    ///
    /// Returns `false` and leaves `out` untouched when the pose does not match
    /// the skeleton or `out` is too small. A wrong-length pose is a bug
    /// somewhere upstream, and half-writing the output would hide it behind a
    /// character that looks *almost* right.
    pub fn to_model_space(&self, skeleton: &Skeleton, out: &mut [Mat4]) -> bool {
        if self.skeleton != skeleton.id
            || self.local.len() != skeleton.len()
            || out.len() < skeleton.len()
        {
            return false;
        }
        for index in 0..self.local.len() {
            let local = self.local[index].to_matrix();
            let parent = skeleton.parents[index];
            out[index] = if parent == NO_PARENT {
                local
            } else {
                out[parent as usize] * local
            };
        }
        true
    }

    /// Compose into the **skinning palette** the GPU wants.
    ///
    /// `model_space[i] * inverse_bind[i]` — the matrix that takes a vertex from
    /// its bind position to where the joint has moved it. This, and only this,
    /// is what crosses into the renderer.
    ///
    /// Same failure contract as [`Self::to_model_space`]: all or nothing.
    pub fn to_palette(&self, skeleton: &Skeleton, out: &mut [Mat4]) -> bool {
        if !self.to_model_space(skeleton, out) {
            return false;
        }
        for (index, matrix) in out.iter_mut().enumerate().take(skeleton.len()) {
            *matrix *= skeleton.inverse_bind[index];
        }
        true
    }

    /// Blend every joint toward `other`.
    ///
    /// The primitive MORROWIND-V's `Blend1D` and `Blend2D` are built from.
    /// Mismatched skeletons return `false` rather than producing a character
    /// folded inside out.
    pub fn blend_into(&self, other: &Pose, t: f32, out: &mut Pose) -> bool {
        if self.skeleton != other.skeleton
            || self.local.len() != other.local.len()
            || out.local.len() != self.local.len()
        {
            return false;
        }
        out.skeleton = self.skeleton;
        for index in 0..self.local.len() {
            out.local[index] = self.local[index].blend(other.local[index], t);
        }
        true
    }
}

/// How many joints one character contributes to the palette buffer.
///
/// The renderer allocates palette space in these units, so it is here rather
/// than in the renderer: a change to it is a change to what the animation side
/// promises, not to how the GPU stores it.
pub const MAX_JOINTS_PER_SKELETON: usize = 256;

/// Per-vertex skin binding: which joints move this vertex, and how much.
///
/// Four influences, which is what glTF guarantees in one set and what every
/// real-time skinning path assumes. A vertex with more is truncated to its four
/// heaviest and renormalised at import — losing the fifth influence is
/// invisible, and *not* renormalising is not: the vertex shrinks toward the
/// origin by exactly the weight that was dropped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkinBinding {
    pub joints: [JointIndex; 4],
    pub weights: [f32; 4],
}

impl Default for SkinBinding {
    fn default() -> Self {
        Self::UNSKINNED
    }
}

impl SkinBinding {
    /// Bound entirely to joint 0 — what an unweighted vertex in a skinned mesh
    /// means, and never a zero-weight binding, which would collapse the vertex
    /// to the origin.
    pub const UNSKINNED: Self = Self {
        joints: [0; 4],
        weights: [1.0, 0.0, 0.0, 0.0],
    };

    /// Keep the four heaviest influences and renormalise.
    ///
    /// Takes `(joint, weight)` pairs in any order and any number.
    #[must_use]
    pub fn from_influences(influences: &[(JointIndex, f32)]) -> Self {
        let mut kept: Vec<(JointIndex, f32)> = influences
            .iter()
            .copied()
            .filter(|(_, w)| *w > 0.0)
            .collect();
        // Descending by weight; ties broken by joint index so the result is
        // deterministic, which a cooked asset needs it to be.
        kept.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        kept.truncate(4);
        if kept.is_empty() {
            return Self::UNSKINNED;
        }

        let total: f32 = kept.iter().map(|(_, w)| *w).sum();
        let mut binding = Self {
            joints: [0; 4],
            weights: [0.0; 4],
        };
        for (slot, (joint, weight)) in kept.into_iter().enumerate() {
            binding.joints[slot] = joint;
            binding.weights[slot] = weight / total;
        }
        binding
    }

    /// Whether the weights sum to one within tolerance.
    #[must_use]
    pub fn is_normalised(&self) -> bool {
        (self.weights.iter().sum::<f32>() - 1.0).abs() < 1e-3
    }

    /// The highest joint index this vertex references.
    ///
    /// Used at bind time to check a mesh against its skeleton: a vertex naming
    /// a joint the skeleton does not have reads past the palette on the GPU,
    /// which on most drivers is a garbage matrix and on some is a hang.
    #[must_use]
    pub fn max_joint(&self) -> JointIndex {
        self.joints
            .iter()
            .zip(self.weights.iter())
            .filter(|(_, w)| **w > 0.0)
            .map(|(j, _)| *j)
            .max()
            .unwrap_or(0)
    }
}

/// A skinned mesh's binding to a skeleton.
#[derive(Clone, Debug, PartialEq)]
pub struct Skin {
    pub skeleton: SkeletonId,
    /// One per vertex, in the mesh's vertex order.
    pub bindings: Vec<SkinBinding>,
}

impl Skin {
    /// Whether every vertex references a joint this skeleton has.
    ///
    /// Checked at bind rather than trusted, because the failure mode on the GPU
    /// is an out-of-bounds palette read.
    #[must_use]
    pub fn fits(&self, skeleton: &Skeleton) -> bool {
        self.skeleton == skeleton.id
            && skeleton.len() <= MAX_JOINTS_PER_SKELETON
            && self
                .bindings
                .iter()
                .all(|b| (b.max_joint() as usize) < skeleton.len())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod runtime_tests;
