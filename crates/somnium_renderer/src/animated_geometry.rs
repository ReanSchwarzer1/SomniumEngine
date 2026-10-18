//! Live GPU deformation shared by games and editor previews. Registrations own
//! their posed allocation; removing one returns that span to the geometry pool.
//!
//! Each frame, before the skin pass overwrites the posed vertices, they are
//! copied aside: last frame's pose, which the velocity pass reads to give a
//! skinned surface its own motion (C2 in the engine features record).
//!
//! Only a registration whose palette changed is skinned again (open-world
//! render scale, 19 Oct 2026): a crowd sampled at 10 Hz in the distance held
//! the same pose for several frames and was re-skinned every one. The first
//! frame after a skin still copies its pose aside, so the previous pose then
//! equals the current one and the surface reads as still; after that a still
//! registration costs nothing until its palette changes.
use crate::{
    context::RenderContext,
    geometry::{GeometryPool, MeshAllocation},
    pass::skin::SkinPass,
    skinning::{SkinnedHandle, SkinningPalettes},
};
use glam::Mat4;
use somnium_anim::{Skeleton, Skin};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// Opaque registration identity; removed identities are never reused.
pub struct AnimatedMeshId(u64);
struct Entry {
    skeleton: Skeleton,
    skin: Skin,
    rest: u32,
    posed: MeshAllocation,
    bounds: ([f32; 3], [f32; 3]),
    matrices: Vec<Mat4>,
    handle: Option<SkinnedHandle>,
    /// Where its last pose lives in the previous-pose buffer, and whether that copy
    /// is still to be made (new or moved: it takes this frame's pose, so no motion).
    prev_base: u32,
    fresh: bool,
    /// The palette differs from the pose its posed vertices hold: skin it this frame.
    changed: bool,
    /// Skinned last frame: copy its pose aside once more, so the previous pose catches up with it.
    settle: bool,
}
#[derive(Default)]
/// Owns posed geometry and batches validated joint palettes for the GPU.
pub struct AnimatedGeometry {
    entries: BTreeMap<AnimatedMeshId, Entry>,
    next: u64,
    dirty: bool,
    palettes: SkinningPalettes,
    /// Last frame's posed vertices of every registration, laid end to end.
    prev: Option<wgpu::Buffer>,
    prev_capacity: u64,
    /// Bumped whenever `prev` is a new buffer, so bind groups that read it are remade.
    prev_generation: u64,
    /// The pool buffer the posed vertices were last written into (a new one holds none of them).
    pool_size: u64,
    /// Registrations skinned by the last [`Self::record`] (a measure for crowds).
    skinned: usize,
}

const VERTEX_BYTES: u64 = std::mem::size_of::<somnium_asset::Vertex>() as u64;
impl AnimatedGeometry {
    /// Register validated rest geometry; caller keeps its rest allocation alive until removal.
    pub fn register(
        &mut self,
        ctx: &RenderContext,
        pool: &mut GeometryPool,
        pass: &mut SkinPass,
        skeleton: &Skeleton,
        skin: &Skin,
        rest: u32,
        vertices: &[somnium_asset::Vertex],
    ) -> Result<(AnimatedMeshId, u32), String> {
        self.register_with(&ctx.device, &ctx.queue, pool, pass, skeleton, skin, rest, vertices)
    }

    /// [`Self::register`] on a device and queue (a test has no window for a [`RenderContext`]).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn register_with(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool: &mut GeometryPool,
        pass: &mut SkinPass,
        skeleton: &Skeleton,
        skin: &Skin,
        rest: u32,
        vertices: &[somnium_asset::Vertex],
    ) -> Result<(AnimatedMeshId, u32), String> {
        if vertices.is_empty() {
            return Err("Animated mesh has no vertices".into());
        }
        let next = self
            .next
            .checked_add(1)
            .ok_or("Animated mesh identifier exhausted")?;
        let bounds = pool.mesh_aabb(rest).ok_or("Rest mesh has no bounds")?;
        // Validate budgets and bindings before allocating GPU memory.
        let mut validation = SkinningPalettes::default();
        for entry in self.entries.values() {
            validation
                .register(
                    &entry.skeleton,
                    &entry.skin,
                    entry.rest,
                    entry.posed.vertex_offset,
                    entry.skin.bindings.len() as u32,
                    entry.bounds,
                )
                .map_err(|e| format!("Skin registration: {e:?}"))?;
        }
        validation
            .register(skeleton, skin, rest, 0, vertices.len() as u32, bounds)
            .map_err(|e| format!("Skin registration: {e:?}"))?;
        let posed = pool.upload_mesh_pooled(queue, vertices, &[], 0);
        if posed.vertex_capacity < vertices.len() as u32 {
            return Err("Posed geometry budget exhausted".into());
        }
        pass.upload_bindings(device, queue, rest, &skin.bindings);
        self.next = next;
        let id = AnimatedMeshId(self.next);
        let offset = posed.vertex_offset;
        self.entries.insert(
            id,
            Entry {
                skeleton: skeleton.clone(),
                skin: skin.clone(),
                rest,
                posed,
                bounds,
                matrices: vec![Mat4::IDENTITY; skeleton.len()],
                handle: None,
                prev_base: 0,
                fresh: true,
                changed: true,
                settle: false,
            },
        );
        self.dirty = true;
        Ok((id, offset))
    }
    /// Posed mesh origins identify visible draws that lack deformation motion vectors.
    pub(crate) fn posed_offsets(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.values().map(|entry| entry.posed.vertex_offset)
    }
    /// Each posed mesh's vertex offset and vertex count (a bottom-level structure is built over them).
    pub(crate) fn posed_meshes(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.entries.values().map(|entry| (entry.posed.vertex_offset, entry.posed.vertex_count))
    }
    /// For each posed mesh (by its vertex offset), where its last pose starts in `previous_pose()`.
    pub(crate) fn previous_bases(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.entries.values().map(|entry| (entry.posed.vertex_offset, entry.prev_base))
    }
    /// The buffer of last frame's poses and its generation (it changes when the buffer is remade).
    pub(crate) fn previous_pose(&self) -> Option<(&wgpu::Buffer, u64)> {
        self.prev.as_ref().map(|buffer| (buffer, self.prev_generation))
    }
    /// Replace a finite palette with exactly the registered joint count.
    pub fn update(&mut self, id: AnimatedMeshId, matrices: &[Mat4]) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        if matrices.len() != entry.matrices.len() || matrices.iter().any(|m| !m.is_finite()) {
            return false;
        }
        if entry.matrices.as_slice() != matrices {
            entry.matrices.copy_from_slice(matrices);
            entry.changed = true;
        }
        true
    }

    /// Registrations the last frame skinned; the rest kept their pose.
    #[must_use]
    pub fn skinned_last_frame(&self) -> usize {
        self.skinned
    }
    /// Free posed geometry; the caller may now free the corresponding rest mesh.
    pub fn remove(&mut self, id: AnimatedMeshId, pool: &mut GeometryPool) -> bool {
        let Some(entry) = self.entries.remove(&id) else {
            return false;
        };
        pool.free_mesh(entry.posed);
        self.dirty = true;
        true
    }
    /// Rebuild changed registrations and refresh conservative posed bounds before culling.
    pub fn prepare(&mut self, pool: &mut GeometryPool) {
        if self.dirty {
            self.palettes = SkinningPalettes::default();
            for entry in self.entries.values_mut() {
                entry.handle = Some(
                    self.palettes
                        .register(
                            &entry.skeleton,
                            &entry.skin,
                            entry.rest,
                            entry.posed.vertex_offset,
                            entry.skin.bindings.len() as u32,
                            entry.bounds,
                        )
                        .expect("registration was validated"),
                );
            }
            // Lay last poses end to end; every one starts again from this frame's pose.
            let mut base = 0u32;
            for entry in self.entries.values_mut() {
                entry.prev_base = base;
                entry.fresh = true;
                entry.changed = true;
                base += entry.posed.vertex_count;
            }
            self.dirty = false;
        }
        // A still registration's palette and bounds are what they were.
        for entry in self.entries.values().filter(|entry| entry.changed) {
            if let Some(handle) = entry.handle {
                self.palettes.set_palette(handle, &entry.matrices);
                if let Some(bounds) = self.palettes.posed_bounds(handle) {
                    pool.set_mesh_bounds(entry.posed.vertex_offset, bounds);
                }
            }
        }
    }
    /// Record deformation before any render pass reads posed vertices. Last frame's
    /// pose is copied aside first (after, for a registration new this frame).
    pub fn record(
        &mut self,
        ctx: &RenderContext,
        encoder: &mut wgpu::CommandEncoder,
        pool: &GeometryPool,
        pass: &mut SkinPass,
    ) {
        self.record_with(&ctx.device, &ctx.queue, encoder, pool, pass);
    }

    /// [`Self::record`] on a device and queue.
    pub(crate) fn record_with(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pool: &GeometryPool,
        pass: &mut SkinPass,
    ) {
        let needed = self
            .entries
            .values()
            .map(|entry| u64::from(entry.posed.vertex_count) * VERTEX_BYTES)
            .sum::<u64>()
            .max(VERTEX_BYTES);
        if self.prev.is_none() || self.prev_capacity < needed {
            let capacity = needed.next_power_of_two();
            self.prev = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Previous posed vertices"),
                size: capacity,
                // COPY_SRC: read back by the tests.
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }));
            self.prev_capacity = capacity;
            self.prev_generation += 1;
            for entry in self.entries.values_mut() {
                entry.fresh = true;
                entry.changed = true;
            }
        }
        if self.pool_size != pool.vertex_buffer.size() {
            self.pool_size = pool.vertex_buffer.size();
            for entry in self.entries.values_mut() {
                entry.changed = true;
            }
        }
        let prev = self.prev.as_ref().expect("made above");
        let copy = |encoder: &mut wgpu::CommandEncoder, entry: &Entry| {
            encoder.copy_buffer_to_buffer(
                &pool.vertex_buffer,
                u64::from(entry.posed.vertex_offset) * VERTEX_BYTES,
                prev,
                u64::from(entry.prev_base) * VERTEX_BYTES,
                u64::from(entry.posed.vertex_count) * VERTEX_BYTES,
            );
        };
        // Last frame's pose aside: for a registration about to be skinned, and once for one skinned last frame.
        for entry in self.entries.values().filter(|entry| !entry.fresh && (entry.changed || entry.settle)) {
            copy(encoder, entry);
        }
        let instances = self.palettes.instances();
        let skin: Vec<_> = self
            .entries
            .values()
            .filter(|entry| entry.changed || entry.fresh)
            .filter_map(|entry| entry.handle.and_then(|h| instances.get(h.0 as usize)).copied())
            .collect();
        self.skinned = skin.len();
        pass.record_instances(
            device,
            queue,
            encoder,
            &pool.vertex_buffer,
            &self.palettes,
            &skin,
        );
        for entry in self.entries.values_mut() {
            if entry.fresh {
                copy(encoder, entry);
                (entry.fresh, entry.settle) = (false, false);
            } else {
                entry.settle = entry.changed;
            }
            entry.changed = false;
        }
    }
}

#[cfg(test)]
#[path = "animated_geometry_tests.rs"]
mod gpu_tests;
