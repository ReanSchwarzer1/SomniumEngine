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
///
/// 32 px keeps the grid coarse enough that the per-frame offset upload stays
/// small (a 1080p grid is ~49 k froxels ≈ 392 KB, versus ~196 k ≈ 1.5 MB at
/// 16 px). Clustered-forward renderers typically use coarse grids — Doom 2016
/// shipped with 16×8×24 *total* clusters — and a coarser tile only means a few
/// more false-positive lights per pixel, which is far cheaper than the upload.
pub const TILE_SIZE: u32 = 32;

/// Number of exponential depth slices along the view-space Z axis.
pub const NUM_DEPTH_SLICES: u32 = 24;

/// Maximum total number of (froxel → light) index entries across all froxels.
pub const MAX_LIGHT_INDICES: usize = 256 * 1024;

/// Maximum number of froxels the offset buffer can hold.
///
/// Must be ≥ `grid_w * grid_h * NUM_DEPTH_SLICES` for any supported resolution,
/// otherwise froxels past the end would read stale GPU data. 262 144 covers 4K
/// at `TILE_SIZE = 32` (120 × 68 × 24 = 195 840) with headroom.
const MAX_FROXELS: usize = 262_144;

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
    /// `0` = point, `1` = spot, `2` = rect, `3` = disc, `4` = tube.
    pub light_type: u32,
    /// Spot/area axis (world space). Disc uses this as the emitting-plane normal;
    /// tube uses it as the capsule axis. Ignored for point lights.
    pub direction_ws: [f32; 3],
    /// `cos(outer cone angle)` for spot lights.
    pub spot_cos_outer: f32,
    /// `cos(inner cone angle)` for spot lights.
    pub spot_cos_inner: f32,
    /// Radius of the emitting surface, metres (Phase 24V).
    ///
    /// Real fixtures are not points. A bulb is a few centimetres across, and
    /// that size is what gives its highlight area and its shadow a penumbra.
    /// Rides in what was padding, so the struct stays 64 bytes.
    pub radius: f32,
    /// Padding to 64 bytes.
    pub _pad: [f32; 2],
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
    /// Packed shading flags. Bit 0 = cel, bit 1 = PCSS, bit 2 = contact shadows, bit 3 = analytic grads.
    pub shading_mode: u32,
    pub num_local_lights: u32,
}

/// Per-froxel offset into the flat light-index list.
///
/// **Size**: 8 bytes.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Pod, Zeroable)]
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

    // ── Scratch, reused every frame ──────────────────────────────────────────
    // Kept on the struct so assignment does no per-frame heap allocation. The
    // original implementation built a `Vec<Vec<u32>>` (one Vec per froxel),
    // which meant a separate malloc for every froxel a light touched — tens of
    // thousands per light per frame once local lights actually existed.
    /// Per-froxel light count, then reused as the per-froxel write cursor.
    counts: Vec<u32>,
    /// Per-froxel (offset, count) table uploaded to the GPU.
    offsets: Vec<ClusterOffset>,
    /// Flattened froxel → light index list.
    index_list: Vec<u32>,
}

/// The tile / depth-slice span a light's bounding sphere covers on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FroxelBounds {
    tile_min_x: u32,
    tile_max_x: u32,
    tile_min_y: u32,
    tile_max_y: u32,
    slice_min: u32,
    slice_max: u32,
}

/// Compute the froxel span a light covers, or `None` if it is off-screen or
/// entirely behind the camera.
///
/// Split out of the assignment loop so it can be unit-tested without a GPU.
#[allow(clippy::too_many_arguments)]
fn light_froxel_bounds(
    light: &GpuLocalLight,
    view: glam::Mat4,
    proj: glam::Mat4,
    sw: f32,
    sh: f32,
    near: f32,
    far: f32,
    grid_w: u32,
    grid_h: u32,
) -> Option<FroxelBounds> {
    let pos_ws = glam::Vec4::new(
        light.position_ws[0],
        light.position_ws[1],
        light.position_ws[2],
        1.0,
    );

    // In a right-handed view matrix the camera looks along -Z, so `depth`
    // (positive into the screen) is `-pos_vs.z`.
    let pos_vs = view * pos_ws;
    let depth = -pos_vs.z;
    let range = light.range;

    // Entirely behind the camera.
    if depth + range < near {
        return None;
    }

    let z_min = (depth - range).max(near);
    let z_max = depth + range;

    // Project a view-space point to screen pixels.
    let project_to_screen = |vx: f32, vy: f32, vz: f32| -> (f32, f32) {
        let clip = proj * glam::Vec4::new(vx, vy, -vz, 1.0);
        if clip.w <= 0.0 {
            // Degenerate — return screen centre so clamping keeps it in bounds.
            return (sw * 0.5, sh * 0.5);
        }
        let px = (clip.x / clip.w * 0.5 + 0.5) * sw;
        let py = (1.0 - (clip.y / clip.w * 0.5 + 0.5)) * sh; // flip Y
        (px, py)
    };

    // Conservative screen AABB from the bounding sphere's axis extremes.
    let (x0, _) = project_to_screen(pos_vs.x - range, pos_vs.y, depth);
    let (x1, _) = project_to_screen(pos_vs.x + range, pos_vs.y, depth);
    let (_, y0) = project_to_screen(pos_vs.x, pos_vs.y - range, depth);
    let (_, y1) = project_to_screen(pos_vs.x, pos_vs.y + range, depth);

    let px_min_x = x0.min(x1).max(0.0);
    let px_max_x = x0.max(x1).min(sw);
    let px_min_y = y0.min(y1).max(0.0);
    let px_max_y = y0.max(y1).min(sh);

    if px_min_x >= px_max_x || px_min_y >= px_max_y {
        return None;
    }

    Some(FroxelBounds {
        tile_min_x: ((px_min_x as u32) / TILE_SIZE).min(grid_w - 1),
        tile_max_x: (((px_max_x as u32).saturating_sub(1)) / TILE_SIZE).min(grid_w - 1),
        tile_min_y: ((px_min_y as u32) / TILE_SIZE).min(grid_h - 1),
        tile_max_y: (((px_max_y as u32).saturating_sub(1)) / TILE_SIZE).min(grid_h - 1),
        slice_min: depth_slice(z_min, near, far),
        slice_max: depth_slice(z_max, near, far),
    })
}

/// Bin `lights` into froxels using a counting sort.
///
/// Fills `counts` (used as scratch, ends up holding each froxel's write
/// cursor), `offsets` (the per-froxel table uploaded to the GPU) and
/// `index_list` (the flattened froxel -> light indices). All three are reused
/// across frames, so this performs no heap allocation in the steady state.
#[allow(clippy::too_many_arguments)]
fn assign_froxels(
    lights: &[GpuLocalLight],
    view: glam::Mat4,
    proj: glam::Mat4,
    sw: f32,
    sh: f32,
    near: f32,
    far: f32,
    grid_w: u32,
    grid_h: u32,
    total_froxels: usize,
    counts: &mut Vec<u32>,
    offsets: &mut Vec<ClusterOffset>,
    index_list: &mut Vec<u32>,
) {
    // ── 1. Count pass ────────────────────────────────────────────────────────
    counts.clear();
    counts.resize(total_froxels, 0);

    for light in lights {
        let Some(b) = light_froxel_bounds(light, view, proj, sw, sh, near, far, grid_w, grid_h)
        else {
            continue;
        };
        for sz in b.slice_min..=b.slice_max {
            for ty in b.tile_min_y..=b.tile_max_y {
                let row = (sz * grid_h * grid_w + ty * grid_w) as usize;
                for tx in b.tile_min_x..=b.tile_max_x {
                    let froxel = row + tx as usize;
                    if froxel < total_froxels {
                        counts[froxel] += 1;
                    }
                }
            }
        }
    }

    // ── 2. Prefix sum → per-froxel (offset, count) ───────────────────────────
    offsets.clear();
    offsets.reserve(total_froxels);
    let mut running: u32 = 0;
    for &c in counts.iter() {
        // Clamp so the flat list can never overrun the GPU buffer.
        let remaining = MAX_LIGHT_INDICES as u32 - running;
        let count = c.min(remaining);
        offsets.push(ClusterOffset {
            offset: running,
            count,
        });
        running += count;
    }
    let total_indices = running as usize;

    // ── 3. Fill pass ─────────────────────────────────────────────────────────
    // Reuse `counts` as the per-froxel write cursor.
    for (froxel, cursor) in counts.iter_mut().enumerate() {
        *cursor = offsets[froxel].offset;
    }

    index_list.clear();
    index_list.resize(total_indices, 0);

    for (light_idx, light) in lights.iter().enumerate() {
        let Some(b) = light_froxel_bounds(light, view, proj, sw, sh, near, far, grid_w, grid_h)
        else {
            continue;
        };
        for sz in b.slice_min..=b.slice_max {
            for ty in b.tile_min_y..=b.tile_max_y {
                let row = (sz * grid_h * grid_w + ty * grid_w) as usize;
                for tx in b.tile_min_x..=b.tile_max_x {
                    let froxel = row + tx as usize;
                    if froxel >= total_froxels {
                        continue;
                    }
                    let entry = offsets[froxel];
                    let cursor = &mut counts[froxel];
                    // Respect the clamp applied during the prefix sum.
                    if *cursor < entry.offset + entry.count {
                        index_list[*cursor as usize] = light_idx as u32;
                        *cursor += 1;
                    }
                }
            }
        }
    }
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
            counts: Vec::new(),
            offsets: Vec::new(),
            index_list: Vec::new(),
        }
    }

    /// Assign lights to froxels on the CPU and upload all four buffers.
    ///
    /// # Algorithm
    ///
    /// 1. Divide the screen into `grid_w x grid_h` tiles of `TILE_SIZE` px and
    ///    slice the frustum into `NUM_DEPTH_SLICES` exponential depth slices.
    /// 2. **Count** pass: for each light, add 1 to every froxel its bounding
    ///    sphere covers.
    /// 3. Prefix-sum the counts into the per-froxel `(offset, count)` table.
    /// 4. **Fill** pass: write light indices into the flat list at each
    ///    froxel's cursor.
    /// 5. Upload the four buffers.
    ///
    /// Counting sort into flat, reused buffers replaces the original
    /// `Vec<Vec<u32>>`, which allocated once per froxel *per frame*.
    /// With no lights the whole thing is skipped — the shader guards every
    /// cluster read behind `num_local_lights > 0`.
    #[allow(clippy::too_many_arguments)]
    pub fn assign_and_upload(
        &mut self,
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

        let grid_w = screen_width.div_ceil(TILE_SIZE).max(1);
        let grid_h = screen_height.div_ceil(TILE_SIZE).max(1);
        let total_froxels = ((grid_w * grid_h * NUM_DEPTH_SLICES) as usize).min(MAX_FROXELS);

        // Params always go up so the shader knows how many lights are live.
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

        // Nothing to bin: skip the grid work and the ~400 KB offset upload
        // entirely. The shader never reads the cluster buffers in this case.
        if num_lights == 0 {
            return;
        }

        assign_froxels(
            &lights[..num_lights],
            view,
            proj,
            screen_width as f32,
            screen_height as f32,
            near,
            far,
            grid_w,
            grid_h,
            total_froxels,
            &mut self.counts,
            &mut self.offsets,
            &mut self.index_list,
        );

        // ── 4. Upload ────────────────────────────────────────────────────────
        queue.write_buffer(
            &self.light_buffer,
            0,
            bytemuck::cast_slice(&lights[..num_lights]),
        );
        if !self.index_list.is_empty() {
            queue.write_buffer(
                &self.index_buffer,
                0,
                bytemuck::cast_slice(&self.index_list),
            );
        }
        queue.write_buffer(&self.offset_buffer, 0, bytemuck::cast_slice(&self.offsets));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_view_proj() -> (glam::Mat4, glam::Mat4) {
        let view = glam::Mat4::look_at_rh(
            glam::Vec3::new(0.0, 2.0, 8.0),
            glam::Vec3::ZERO,
            glam::Vec3::Y,
        );
        let proj = glam::Mat4::perspective_rh(45.0_f32.to_radians(), 16.0 / 9.0, 0.1, 1000.0);
        (view, proj)
    }

    fn light(pos: [f32; 3], range: f32) -> GpuLocalLight {
        GpuLocalLight {
            position_ws: pos,
            range,
            color: [1.0, 1.0, 1.0],
            light_type: 0,
            direction_ws: [0.0, -1.0, 0.0],
            spot_cos_outer: 0.7,
            spot_cos_inner: 0.9,
            radius: 0.0,
            _pad: [0.0; 2],
        }
    }

    /// The original implementation, kept as a reference oracle: one `Vec` per
    /// froxel, flattened in order. The counting sort must match it exactly.
    #[allow(clippy::too_many_arguments)]
    fn reference_assign(
        lights: &[GpuLocalLight],
        view: glam::Mat4,
        proj: glam::Mat4,
        sw: f32,
        sh: f32,
        near: f32,
        far: f32,
        grid_w: u32,
        grid_h: u32,
        total_froxels: usize,
    ) -> (Vec<u32>, Vec<ClusterOffset>) {
        let mut froxel_lists: Vec<Vec<u32>> = vec![Vec::new(); total_froxels];
        for (light_idx, l) in lights.iter().enumerate() {
            let Some(b) = light_froxel_bounds(l, view, proj, sw, sh, near, far, grid_w, grid_h)
            else {
                continue;
            };
            for sz in b.slice_min..=b.slice_max {
                for ty in b.tile_min_y..=b.tile_max_y {
                    for tx in b.tile_min_x..=b.tile_max_x {
                        let f = (sz * grid_h * grid_w + ty * grid_w + tx) as usize;
                        if f < total_froxels {
                            froxel_lists[f].push(light_idx as u32);
                        }
                    }
                }
            }
        }
        let mut index_list = Vec::new();
        let mut offsets = Vec::new();
        let mut running = 0u32;
        for list in &froxel_lists {
            offsets.push(ClusterOffset {
                offset: running,
                count: list.len() as u32,
            });
            index_list.extend_from_slice(list);
            running += list.len() as u32;
        }
        (index_list, offsets)
    }

    #[test]
    fn counting_sort_matches_the_naive_reference() {
        let (view, proj) = test_view_proj();
        let (sw, sh) = (1920.0_f32, 1080.0_f32);
        let grid_w = (sw as u32).div_ceil(TILE_SIZE);
        let grid_h = (sh as u32).div_ceil(TILE_SIZE);
        let total = (grid_w * grid_h * NUM_DEPTH_SLICES) as usize;

        // A mix: near/far, overlapping, off-screen, and behind the camera.
        let lights = [
            light([4.0, 3.0, 2.0], 12.0),
            light([-4.0, 6.0, 1.0], 20.0),
            light([0.0, 0.0, -50.0], 5.0),
            light([0.0, 0.0, 500.0], 1.0), // behind camera
            light([200.0, 0.0, 0.0], 2.0), // off to the side
        ];

        let (ref_idx, ref_off) = reference_assign(
            &lights, view, proj, sw, sh, 0.1, 1000.0, grid_w, grid_h, total,
        );

        let (mut counts, mut offsets, mut index_list) = (Vec::new(), Vec::new(), Vec::new());
        assign_froxels(
            &lights,
            view,
            proj,
            sw,
            sh,
            0.1,
            1000.0,
            grid_w,
            grid_h,
            total,
            &mut counts,
            &mut offsets,
            &mut index_list,
        );

        assert_eq!(offsets.len(), ref_off.len(), "froxel table size");
        assert_eq!(offsets, ref_off, "per-froxel (offset, count) table differs");
        assert_eq!(index_list, ref_idx, "flattened light index list differs");
    }

    #[test]
    fn no_lights_produces_no_indices() {
        let (view, proj) = test_view_proj();
        let (mut counts, mut offsets, mut index_list) = (Vec::new(), Vec::new(), Vec::new());
        assign_froxels(
            &[],
            view,
            proj,
            1920.0,
            1080.0,
            0.1,
            1000.0,
            60,
            34,
            60 * 34 * 24,
            &mut counts,
            &mut offsets,
            &mut index_list,
        );
        assert!(index_list.is_empty());
        assert!(offsets.iter().all(|o| o.count == 0));
    }

    #[test]
    fn every_index_entry_is_inside_its_froxel_slot() {
        // Guards the cursor logic: each froxel's entries must land within
        // [offset, offset + count) and reference a real light.
        let (view, proj) = test_view_proj();
        let lights = [light([0.0, 1.0, 0.0], 30.0), light([2.0, 1.0, 1.0], 25.0)];
        let (grid_w, grid_h) = (60u32, 34u32);
        let total = (grid_w * grid_h * NUM_DEPTH_SLICES) as usize;

        let (mut counts, mut offsets, mut index_list) = (Vec::new(), Vec::new(), Vec::new());
        assign_froxels(
            &lights,
            view,
            proj,
            1920.0,
            1080.0,
            0.1,
            1000.0,
            grid_w,
            grid_h,
            total,
            &mut counts,
            &mut offsets,
            &mut index_list,
        );

        for o in &offsets {
            let end = (o.offset + o.count) as usize;
            assert!(
                end <= index_list.len(),
                "froxel slot runs past the index list"
            );
            for &li in &index_list[o.offset as usize..end] {
                assert!((li as usize) < lights.len(), "bogus light index {li}");
            }
        }
        assert!(!index_list.is_empty(), "expected some assignments");
    }

    #[test]
    fn froxel_grid_fits_the_offset_buffer_at_4k() {
        // MAX_FROXELS must cover the largest grid we can produce, or froxels
        // past the end would read stale GPU memory.
        let grid_w = 3840u32.div_ceil(TILE_SIZE);
        let grid_h = 2160u32.div_ceil(TILE_SIZE);
        let total = (grid_w * grid_h * NUM_DEPTH_SLICES) as usize;
        assert!(
            total <= MAX_FROXELS,
            "4K grid is {total}, exceeds MAX_FROXELS {MAX_FROXELS}"
        );
    }
}
