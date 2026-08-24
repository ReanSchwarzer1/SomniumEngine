//! Phase CONTROL-O: deferred decals.
//!
//! # Why there is no pass here
//!
//! A deferred decal is not a draw. It is a box that, where it overlaps a
//! surface, replaces some of that surface's material before it is shaded — so
//! the cheapest correct place for it is *inside* the shading pass, right after
//! the material is read and before anything is lit. That is what this module
//! feeds: three storage buffers the shading pass binds, and a per-froxel index
//! list that tells each pixel which handful of decals it has to consider.
//!
//! # Reusing the clustering rather than copying it
//!
//! §8's CONTROL-O entry is explicit: *"the clustering is written, tested and
//! shipping, and decals are the second consumer it was shaped for."* So
//! `cluster.rs` grew a [`ClusterVolume`] trait and its counting sort became
//! generic, and this module supplies the second implementation. There is one
//! froxel binning algorithm in this engine, and it is tested once.
//!
//! # What a decal can change
//!
//! Base colour, normal and roughness — the three channels the shading pass
//! reads before it lights anything. Sorted by an authored priority so two
//! overlapping decals have a defined order rather than a buffer-order one, and
//! faded by the angle between the surface normal and the decal's own axis so a
//! projection does not smear down a wall it was never aimed at.

use bytemuck::{Pod, Zeroable};

use crate::cluster::ClusterVolume;

/// Decals a frame may carry.
///
/// Sized to match `MAX_LOCAL_LIGHTS`: the two share a screen and there is no
/// reason to believe a scene wants an order of magnitude more of one than the
/// other.
pub const MAX_DECALS: usize = 256;
/// Flattened froxel → decal index entries.
pub const MAX_DECAL_INDICES: usize = 64 * 1024;

/// One decal, as the shading pass reads it. **Size**: 128 bytes.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuDecal {
    /// World → decal space, where the decal occupies the unit cube centred on
    /// the origin. A pixel is inside the decal when every component of its
    /// decal-space position is within `±0.5`.
    ///
    /// The inverse is stored rather than the forward transform because the
    /// shader only ever needs this direction, and inverting a matrix per pixel
    /// per decal is not a thing to do.
    pub inv_transform: [[f32; 4]; 4],
    /// World-space centre. Also the clustering sphere's centre.
    pub position_ws: [f32; 3],
    /// Bounding-sphere radius, metres — half the box's diagonal.
    pub radius: f32,
    /// Tint multiplied into the decal's albedo. Alpha is the decal's opacity.
    pub base_color: [f32; 4],
    /// Bindless index of the albedo texture, or `-1` for a flat tint.
    pub albedo_map: i32,
    /// Bindless index of the normal map, or `-1`.
    pub normal_map: i32,
    /// Bindless index of the metallic-roughness map, or `-1`.
    pub orm_map: i32,
    /// Draw order. Higher wins where two decals overlap.
    pub priority: i32,
    /// How far the decal's normal may tip from the surface's before it fades
    /// out, as a cosine. `1` accepts nothing but a perfectly aligned surface;
    /// `0` accepts anything up to ninety degrees.
    pub angle_fade_cos: f32,
    /// Strength of the normal-map contribution, `0..1`.
    pub normal_strength: f32,
    /// Roughness the decal writes where it is fully opaque.
    pub roughness: f32,
    /// Padding to 128 bytes.
    pub _pad: f32,
}

/// Everything about a decal that is not its box.
///
/// A struct rather than nine positional parameters: three of them are `i32`
/// texture indices and three are `f32` in `0..1`, so a transposed pair would
/// compile and render wrong. Named fields make that a type error at the call
/// site instead of a screenshot somebody has to notice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecalLook {
    /// Tint multiplied into the albedo; alpha is the decal's opacity.
    pub base_color: [f32; 4],
    /// Bindless albedo index, or `-1`.
    pub albedo_map: i32,
    /// Bindless normal-map index, or `-1`.
    pub normal_map: i32,
    /// Bindless metallic-roughness index, or `-1`.
    pub orm_map: i32,
    /// Draw order; higher wins where two decals overlap.
    pub priority: i32,
    /// How far a surface may tip from the projection axis before fading out.
    pub angle_fade_degrees: f32,
    /// Strength of the normal-map contribution, `0..1`.
    pub normal_strength: f32,
    /// Roughness written where the decal is fully opaque.
    pub roughness: f32,
}

impl ClusterVolume for GpuDecal {
    fn centre_ws(&self) -> [f32; 3] {
        self.position_ws
    }
    fn bounding_radius(&self) -> f32 {
        self.radius
    }
}

impl GpuDecal {
    /// Build from a world transform and the authored parameters.
    ///
    /// The transform's scale is the box's full size in metres, so a decal
    /// scaled `(2, 1, 0.5)` is two metres across, one tall and half a metre
    /// deep. Depth matters: it is how far behind the surface the projection
    /// still applies, and it is what stops a decal on a wall appearing on the
    /// floor two metres below it.
    #[must_use]
    pub fn new(transform: glam::Mat4, look: DecalLook) -> Self {
        let DecalLook {
            base_color,
            albedo_map,
            normal_map,
            orm_map,
            priority,
            angle_fade_degrees,
            normal_strength,
            roughness,
        } = look;
        let (scale, _, translation) = transform.to_scale_rotation_translation();
        // The bounding sphere has to contain the box at any rotation, so it is
        // the half-diagonal and not the largest half-extent. Getting this
        // wrong makes a rotated decal vanish at the edges of its own froxels.
        let radius = scale.abs().length() * 0.5;
        Self {
            inv_transform: transform.inverse().to_cols_array_2d(),
            position_ws: translation.to_array(),
            radius: radius.max(1e-3),
            base_color,
            albedo_map,
            normal_map,
            orm_map,
            priority,
            angle_fade_cos: angle_fade_degrees.clamp(0.0, 89.9).to_radians().cos(),
            normal_strength: normal_strength.clamp(0.0, 1.0),
            roughness: roughness.clamp(0.0, 1.0),
            _pad: 0.0,
        }
    }
}

/// The three buffers the shading pass binds, plus the count.
///
/// A sibling of [`ClusterGrid`](crate::cluster::ClusterGrid) rather than a
/// second copy of it: the froxel *geometry* is the light grid's, published
/// once in `GpuClusterParams`, and this only carries the decal list and its
/// own per-froxel table.
pub struct DecalGrid {
    /// `array<GpuDecal>`, sorted by priority.
    pub decal_buffer: wgpu::Buffer,
    /// Flattened froxel → decal index list.
    pub index_buffer: wgpu::Buffer,
    /// One `ClusterOffset` per froxel.
    pub offset_buffer: wgpu::Buffer,
    /// `[count, 0, 0, 0]`. Zero means the shading pass skips decals entirely.
    pub params_buffer: wgpu::Buffer,
    /// Scratch, reused every frame so binning allocates nothing.
    scratch: crate::cluster::BinScratch,
    sorted: Vec<GpuDecal>,
    last_count: u32,
}

impl DecalGrid {
    /// Allocate the buffers. They are sized once and never resized: 256 decals
    /// is 32 KB and the index list is 256 KB, which is not worth a growth path.
    #[must_use]
    pub fn new(device: &wgpu::Device) -> Self {
        let storage = |label: &str, size: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Self {
            decal_buffer: storage(
                "Decals",
                (MAX_DECALS * std::mem::size_of::<GpuDecal>()) as u64,
            ),
            index_buffer: storage("Decal Indices", (MAX_DECAL_INDICES * 4) as u64),
            offset_buffer: storage(
                "Decal Offsets",
                (crate::cluster::MAX_FROXELS * std::mem::size_of::<crate::cluster::ClusterOffset>())
                    as u64,
            ),
            params_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Decal Params"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            scratch: crate::cluster::BinScratch::default(),
            sorted: Vec::new(),
            last_count: 0,
        }
    }

    /// Bin this frame's decals and upload them.
    ///
    /// Writes the count **every** frame, including when it is zero, so the
    /// shading pass can never read a stale decal list from a frame that had
    /// some.
    #[allow(clippy::too_many_arguments)]
    pub fn assign_and_upload(
        &mut self,
        queue: &wgpu::Queue,
        decals: &[GpuDecal],
        view: glam::Mat4,
        proj: glam::Mat4,
        screen_width: u32,
        screen_height: u32,
        near: f32,
        far: f32,
    ) {
        self.sorted.clear();
        self.sorted
            .extend_from_slice(&decals[..decals.len().min(MAX_DECALS)]);
        // Stable, ascending: the shader applies them in order and the last one
        // wins, so the highest priority has to be applied last. A sort that was
        // merely "some order" would make two overlapping decals flicker as the
        // world's iteration order changed.
        self.sorted.sort_by_key(|decal| decal.priority);

        #[allow(clippy::cast_possible_truncation)]
        let count = self.sorted.len() as u32;
        if count != self.last_count || count > 0 {
            queue.write_buffer(
                &self.params_buffer,
                0,
                bytemuck::bytes_of(&[count, 0, 0, 0]),
            );
            self.last_count = count;
        }
        if count == 0 {
            return;
        }

        let grid_w = screen_width.div_ceil(crate::cluster::TILE_SIZE).max(1);
        let grid_h = screen_height.div_ceil(crate::cluster::TILE_SIZE).max(1);
        let total_froxels = ((grid_w * grid_h * crate::cluster::NUM_DEPTH_SLICES) as usize)
            .min(crate::cluster::MAX_FROXELS);

        crate::cluster::bin_volumes(
            &self.sorted,
            view,
            proj,
            screen_width as f32,
            screen_height as f32,
            near,
            far,
            grid_w,
            grid_h,
            total_froxels,
            &mut self.scratch,
        );

        queue.write_buffer(&self.decal_buffer, 0, bytemuck::cast_slice(&self.sorted));
        if !self.scratch.index_list.is_empty() {
            let capped = self.scratch.index_list.len().min(MAX_DECAL_INDICES);
            queue.write_buffer(
                &self.index_buffer,
                0,
                bytemuck::cast_slice(&self.scratch.index_list[..capped]),
            );
        }
        queue.write_buffer(
            &self.offset_buffer,
            0,
            bytemuck::cast_slice(&self.scratch.offsets),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain look with only the angle fade varied, which is the one field
    /// these tests care about.
    fn probe_look(angle_fade_degrees: f32) -> DecalLook {
        DecalLook {
            base_color: [1.0; 4],
            albedo_map: -1,
            normal_map: -1,
            orm_map: -1,
            priority: 0,
            angle_fade_degrees,
            normal_strength: 1.0,
            roughness: 0.5,
        }
    }

    #[test]
    fn the_gpu_decal_is_the_size_the_shader_expects() {
        assert_eq!(std::mem::size_of::<GpuDecal>(), 128);
        assert_eq!(std::mem::size_of::<GpuDecal>() % 16, 0);
    }

    /// A rotated box must still fit inside its clustering sphere, or its
    /// corners fall outside the froxels it was binned into and the decal
    /// visibly clips against nothing.
    #[test]
    fn the_bounding_sphere_contains_the_box_at_any_rotation() {
        let scale = glam::Vec3::new(2.0, 1.0, 0.5);
        let transform = glam::Mat4::from_scale_rotation_translation(
            scale,
            glam::Quat::from_rotation_y(0.7),
            glam::Vec3::new(3.0, 1.0, -2.0),
        );
        let decal = GpuDecal::new(transform, probe_look(30.0));
        let half_diagonal = scale.length() * 0.5;
        assert!(decal.radius >= half_diagonal - 1e-4, "{}", decal.radius);
        assert_eq!(decal.position_ws, [3.0, 1.0, -2.0]);
    }

    /// The inverse transform must actually put the box's centre at the origin
    /// and its faces at ±0.5, because that is the test the shader makes.
    #[test]
    fn the_inverse_transform_maps_the_box_onto_the_unit_cube() {
        let transform = glam::Mat4::from_scale_rotation_translation(
            glam::Vec3::new(4.0, 2.0, 1.0),
            glam::Quat::IDENTITY,
            glam::Vec3::new(10.0, 0.0, 0.0),
        );
        let decal = GpuDecal::new(transform, probe_look(30.0));
        let inv = glam::Mat4::from_cols_array_2d(&decal.inv_transform);

        let centre = inv.transform_point3(glam::Vec3::new(10.0, 0.0, 0.0));
        assert!(centre.length() < 1e-4, "centre mapped to {centre}");

        let face = inv.transform_point3(glam::Vec3::new(12.0, 0.0, 0.0));
        assert!((face.x - 0.5).abs() < 1e-4, "face mapped to {face}");

        let outside = inv.transform_point3(glam::Vec3::new(13.0, 0.0, 0.0));
        assert!(
            outside.x > 0.5,
            "a point past the face must read as outside"
        );
    }

    /// Angle fade is stored as a cosine so the shader compares without a
    /// trigonometric call; the conversion has to be the right way round.
    #[test]
    fn angle_fade_is_stored_as_a_cosine() {
        let flat = GpuDecal::new(glam::Mat4::IDENTITY, probe_look(0.0));
        assert!((flat.angle_fade_cos - 1.0).abs() < 1e-5);

        let wide = GpuDecal::new(glam::Mat4::IDENTITY, probe_look(89.0));
        assert!(wide.angle_fade_cos < 0.05, "{}", wide.angle_fade_cos);
        assert!(
            wide.angle_fade_cos > 0.0,
            "a fade of 90 degrees is degenerate"
        );
    }
}
