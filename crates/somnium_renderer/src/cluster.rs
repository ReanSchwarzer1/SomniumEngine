//! Froxel clustered lighting — CPU-side light assignment.
//!
//! Ported from Bevy's `assign_lights_to_clusters` approach.  Each frame the
//! screen is divided into a 2-D tile grid (TILE_SIZE × TILE_SIZE pixels) and
//! the view frustum is sliced into `NUM_DEPTH_SLICES` exponential depth slices.
//! The resulting 3-D grid cells ("froxels") each store a list of light indices
//! that influence them, which the GPU reads during the shading pass.
//!
//! ## GPU buffer layout
//!
//! | Buffer          | Binding purpose          | Element type       |
//! |-----------------|--------------------------|--------------------|
//! | `light_buffer`  | All local lights         | `GpuLocalLight`    |
//! | `index_buffer`  | Flat light-index list    | `u32`              |
//! | `offset_buffer` | Per-froxel (offset,count)| `ClusterOffset`    |
//! | `params_buffer` | Grid dimensions / config | `GpuClusterParams` |

use bytemuck::{Pod, Zeroable};

// ─── Constants ───────────────────────────────────────────────────────────────

/// Maximum number of local (point + spot) lights the cluster grid can hold.
pub const MAX_LOCAL_LIGHTS: usize = 256;

/// Tile size in pixels for the 2-D screen grid.
pub const TILE_SIZE: u32 = 16;

/// Number of exponential depth slices along the view-space Z axis.
pub const NUM_DEPTH_SLICES: u32 = 24;

/// Maximum total number of (froxel → light) index entries across all froxels.
pub const MAX_LIGHT_INDICES: usize = 256 * 1024;

/// Maximum number of froxels we pre-allocate the offset buffer for.
/// 200 000 froxels ≈ ~130×64×24 which covers up to ~2K×1K at TILE_SIZE=16.
const MAX_FROXELS: usize = 200_000;

// ─── GPU structs ─────────────────────────────────────────────────────────────

/// A single local (point or spot) light as uploaded to the GPU.
///
/// **Size**: 64 bytes (16-byte aligned).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuLocalLight {
    /// World-space position.
    pub position_ws: [f32; 3],
    /// Attenuation radius.
    pub range: f32,
    /// Linear RGB × intensity (pre-multiplied).
    pub color: [f32; 3],
    /// `0` = point, `1` = spot.
    pub light_type: u32,
    /// Spot-light direction (world space); ignored for point lights.
    pub direction_ws: [f32; 3],
    /// `cos(outer cone angle)` for spot lights.
    pub spot_cos_outer: f32,
    /// `cos(inner cone angle)` for spot lights.
    pub spot_cos_inner: f32,
    /// Padding to 64 bytes.
    pub _pad: [f32; 3],
}

/// Cluster grid parameters uploaded as a uniform / storage buffer.
///
/// **Size**: 32 bytes.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuClusterParams {
    pub grid_width: u32,
    pub grid_height: u32,
    pub num_slices: u32,
    pub tile_size: u32,
    pub near: f32,
    pub far: f32,
    /// `0` = PBR, `1` = Cel.
    pub shading_mode: u32,
    pub num_local_lights: u32,
}

/// Per-froxel offset into the flat light-index list.
///
/// **Size**: 8 bytes.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ClusterOffset {
    /// Start index into the global light-index list.
    pub offset: u32,
    /// Number of lights affecting this froxel.
    pub count: u32,
}

// ─── Depth-slice helper ──────────────────────────────────────────────────────

/// Map a positive view-space depth `z` to an exponential slice index in
/// `[0, NUM_DEPTH_SLICES)`.  Matches the GPU-side formula:
///
/// ```text
/// slice = floor(NUM_DEPTH_SLICES * ln(z / near) / ln(far / near))
/// ```
#[inline]
fn depth_slice(z: f32, near: f32, far: f32) -> u32 {
    if z <= near {
        return 0;
    }
    if z >= far {
        return NUM_DEPTH_SLICES - 1;
    }
    let log_ratio = (far / near).ln();
    let slice = (NUM_DEPTH_SLICES as f32 * (z / near).ln() / log_ratio) as u32;
    slice.min(NUM_DEPTH_SLICES - 1)
}

// ─── ClusterGrid ─────────────────────────────────────────────────────────────

/// Owns the four GPU buffers that back the froxel cluster grid and provides
/// the CPU-side `assign_and_upload` method that populates them each frame.
pub struct ClusterGrid {
    /// Storage buffer holding up to `MAX_LOCAL_LIGHTS` lights.
    pub light_buffer: wgpu::Buffer,
    /// Storage buffer holding the flat list of light indices
    /// (up to `MAX_LIGHT_INDICES` entries).
    pub index_buffer: wgpu::Buffer,
    /// Storage buffer holding one `ClusterOffset` per froxel.
    pub offset_buffer: wgpu::Buffer,
    /// Storage buffer holding the current `GpuClusterParams`.
    pub params_buffer: wgpu::Buffer,
}

impl ClusterGrid {
    /// Create the four cluster-grid GPU buffers.
    pub fn new(device: &wgpu::Device) -> Self {
        let light_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cluster Light Buffer"),
            size: (MAX_LOCAL_LIGHTS * std::mem::size_of::<GpuLocalLight>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cluster Index Buffer"),
            size: (MAX_LIGHT_INDICES * std::mem::size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let offset_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cluster Offset Buffer"),
            size: (MAX_FROXELS * std::mem::size_of::<ClusterOffset>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cluster Params Buffer"),
            size: std::mem::size_of::<GpuClusterParams>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            light_buffer,
            index_buffer,
            offset_buffer,
            params_buffer,
        }
    }

    /// Assign lights to froxels on the CPU and upload all four buffers.
    ///
    /// # Algorithm
    ///
    /// 1. Divide the screen into `grid_w × grid_h` tiles of `TILE_SIZE` px.
    /// 2. For each light compute its view-space position and a conservative
    ///    screen-space AABB from the bounding sphere.
    /// 3. Determine the min/max depth slice using exponential slicing.
    /// 4. Insert the light index into every froxel that the AABB overlaps.
    /// 5. Flatten per-froxel lists into a packed index buffer + offset table.
    /// 6. Upload everything via `queue.write_buffer`.
    #[allow(clippy::too_many_arguments)]
    pub fn assign_and_upload(
        &self,
        queue: &wgpu::Queue,
        lights: &[GpuLocalLight],
        view: glam::Mat4,
        proj: glam::Mat4,
        screen_width: u32,
        screen_height: u32,
        near: f32,
        far: f32,
        shading_mode: u32,
    ) {
        let num_lights = lights.len().min(MAX_LOCAL_LIGHTS);

        let grid_w = (screen_width + TILE_SIZE - 1) / TILE_SIZE;
        let grid_h = (screen_height + TILE_SIZE - 1) / TILE_SIZE;
        let total_froxels = (grid_w * grid_h * NUM_DEPTH_SLICES) as usize;

        // Per-froxel light lists (indices into `lights`).
        let mut froxel_lists: Vec<Vec<u32>> = vec![Vec::new(); total_froxels];

        let sw = screen_width as f32;
        let sh = screen_height as f32;

        for (light_idx, light) in lights[..num_lights].iter().enumerate() {
            let pos_ws = glam::Vec4::new(
                light.position_ws[0],
                light.position_ws[1],
                light.position_ws[2],
                1.0,
            );

            // ── Transform to view space ──────────────────────────────────
            let pos_vs = view * pos_ws;
            // In a right-handed view matrix the camera looks along -Z, so
            // `depth` (positive into the screen) is `-pos_vs.z`.
            let depth = -pos_vs.z;
            let range = light.range;

            // Skip lights entirely behind the camera.
            if depth + range < near {
                continue;
            }

            // ── Screen-space AABB of the bounding sphere ─────────────────
            // We project the four extremes of the sphere (center ± range on
            // each axis) through the projection matrix to obtain an
            // axis-aligned bounding rectangle in NDC, then convert to pixels.

            let center_vs = glam::Vec3::new(pos_vs.x, pos_vs.y, pos_vs.z);

            // Clamp the near side of the sphere so we don't project behind the camera.
            let z_min = (depth - range).max(near);
            let z_max = depth + range;

            // Project helper: view-space (x, y, z) → screen pixel coords.
            let project_to_screen = |vx: f32, vy: f32, vz: f32| -> (f32, f32) {
                let clip = proj * glam::Vec4::new(vx, vy, -vz, 1.0);
                if clip.w <= 0.0 {
                    // Degenerate — return screen centre so clamping keeps it
                    // within bounds.
                    return (sw * 0.5, sh * 0.5);
                }
                let ndc_x = clip.x / clip.w;
                let ndc_y = clip.y / clip.w;
                let px = (ndc_x * 0.5 + 0.5) * sw;
                let py = (1.0 - (ndc_y * 0.5 + 0.5)) * sh; // flip Y
                (px, py)
            };

            // Project the four axis-aligned extremes.
            let (x0, _) = project_to_screen(center_vs.x - range, center_vs.y, depth);
            let (x1, _) = project_to_screen(center_vs.x + range, center_vs.y, depth);
            let (_, y0) = project_to_screen(center_vs.x, center_vs.y - range, depth);
            let (_, y1) = project_to_screen(center_vs.x, center_vs.y + range, depth);

            let px_min_x = x0.min(x1).max(0.0);
            let px_max_x = x0.max(x1).min(sw);
            let px_min_y = y0.min(y1).max(0.0);
            let px_max_y = y0.max(y1).min(sh);

            if px_min_x >= px_max_x || px_min_y >= px_max_y {
                continue;
            }

            // Convert pixel bounds to tile coordinates.
            let tile_min_x = (px_min_x as u32) / TILE_SIZE;
            let tile_max_x = ((px_max_x as u32).saturating_sub(1)) / TILE_SIZE;
            let tile_min_y = (px_min_y as u32) / TILE_SIZE;
            let tile_max_y = ((px_max_y as u32).saturating_sub(1)) / TILE_SIZE;

            let tile_min_x = tile_min_x.min(grid_w - 1);
            let tile_max_x = tile_max_x.min(grid_w - 1);
            let tile_min_y = tile_min_y.min(grid_h - 1);
            let tile_max_y = tile_max_y.min(grid_h - 1);

            // Depth-slice range.
            let slice_min = depth_slice(z_min, near, far);
            let slice_max = depth_slice(z_max, near, far);

            // ── Insert into froxels ──────────────────────────────────────
            for sz in slice_min..=slice_max {
                for ty in tile_min_y..=tile_max_y {
                    for tx in tile_min_x..=tile_max_x {
                        let froxel = (sz * grid_h * grid_w + ty * grid_w + tx) as usize;
                        if froxel < total_froxels {
                            froxel_lists[froxel].push(light_idx as u32);
                        }
                    }
                }
            }
        }

        // ── Flatten ──────────────────────────────────────────────────────────
        let mut light_index_list: Vec<u32> = Vec::with_capacity(total_froxels);
        let mut cluster_offsets: Vec<ClusterOffset> = Vec::with_capacity(total_froxels);

        let mut running_offset: u32 = 0;
        for list in &froxel_lists {
            cluster_offsets.push(ClusterOffset {
                offset: running_offset,
                count: list.len() as u32,
            });
            light_index_list.extend_from_slice(list);
            running_offset += list.len() as u32;
        }

        // ── Upload ───────────────────────────────────────────────────────────
        // 1. Light data
        if num_lights > 0 {
            queue.write_buffer(
                &self.light_buffer,
                0,
                bytemuck::cast_slice(&lights[..num_lights]),
            );
        }

        // 2. Flat index list
        if !light_index_list.is_empty() {
            let max_upload = light_index_list.len().min(MAX_LIGHT_INDICES);
            queue.write_buffer(
                &self.index_buffer,
                0,
                bytemuck::cast_slice(&light_index_list[..max_upload]),
            );
        }

        // 3. Per-froxel offsets
        if !cluster_offsets.is_empty() {
            let max_upload = cluster_offsets.len().min(MAX_FROXELS);
            queue.write_buffer(
                &self.offset_buffer,
                0,
                bytemuck::cast_slice(&cluster_offsets[..max_upload]),
            );
        }

        // 4. Params uniform
        let params = GpuClusterParams {
            grid_width: grid_w,
            grid_height: grid_h,
            num_slices: NUM_DEPTH_SLICES,
            tile_size: TILE_SIZE,
            near,
            far,
            shading_mode,
            num_local_lights: num_lights as u32,
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&params));
    }
}
