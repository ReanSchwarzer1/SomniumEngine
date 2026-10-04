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
///
/// 8 MB. A room of 64 practicals around the camera needs ~690 k at 1600p even
/// with each slice binned by its own cross-section (the near slices are thin
/// and every light the camera stands in fills them). Past the cap the far
/// froxels lose every light at once, so this is sized for the worst room.
pub const MAX_LIGHT_INDICES: usize = 2 * 1024 * 1024;

/// Maximum number of froxels the offset buffer can hold.
///
/// Must be ≥ `grid_w * grid_h * NUM_DEPTH_SLICES` for any supported resolution,
/// otherwise froxels past the end would read stale GPU data. 262 144 covers 4K
/// at `TILE_SIZE = 32` (120 × 68 × 24 = 195 840) with headroom.
pub const MAX_FROXELS: usize = 262_144;

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
    /// Packed shading flags. Bit 0 = cel, bit 1 = PCSS, bit 2 = contact shadows,
    /// bit 3 = analytic grads, bit 4 = ReSTIR DI, bit 5 = DREAMS-B terrain STF.
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

/// Something with a world-space bounding sphere that can be binned into the
/// froxel grid.
///
/// Phase CONTROL-O. The binning was written for lights in 13C and the plan
/// (§8, CONTROL-O) names decals as "the second consumer it was shaped for", so
/// the *code* is what gets reused rather than being copied with two words
/// changed. One trait, two implementations, one counting sort.
pub trait ClusterVolume {
    /// World-space centre of the bounding sphere.
    fn centre_ws(&self) -> [f32; 3];
    /// Radius of the bounding sphere, metres.
    fn bounding_radius(&self) -> f32;
    /// World-space corners of a tighter box, when the volume has one. A flat
    /// ground decal's sphere is mostly empty air: projected at a grazing
    /// angle it covers several times the froxels its box does.
    fn corners_ws(&self) -> Option<[glam::Vec3; 8]> {
        None
    }
}

impl ClusterVolume for GpuLocalLight {
    fn centre_ws(&self) -> [f32; 3] {
        self.position_ws
    }
    fn bounding_radius(&self) -> f32 {
        // A tube lights everything within `range` of its segment, so its ends
        // reach half its length further than the centre's sphere.
        if self.light_type == 4 {
            self.range + self._pad[0].max(0.05)
        } else {
            self.range
        }
    }
}

/// Positive view depth where `slice` starts; the inverse of [`depth_slice`].
fn slice_depth(slice: u32, near: f32, far: f32) -> f32 {
    near * (far / near).powf(slice as f32 / NUM_DEPTH_SLICES as f32)
}

/// Call `visit` with every froxel `volume` may touch.
///
/// Within one depth slice a sphere is only as wide as its cross-section
/// there, so each slice is binned by its own box rather than the whole
/// sphere's: a practical the camera stands in otherwise claims every tile of
/// every slice it spans, and a lit room overran the index list.
#[allow(clippy::too_many_arguments)]
fn for_each_froxel<V: ClusterVolume>(
    volume: &V,
    view: glam::Mat4,
    proj: glam::Mat4,
    sw: f32,
    sh: f32,
    near: f32,
    far: f32,
    grid_w: u32,
    grid_h: u32,
    total_froxels: usize,
    mut visit: impl FnMut(usize),
) {
    let Some(b) = volume_froxel_bounds(volume, view, proj, sw, sh, near, far, grid_w, grid_h) else {
        return;
    };
    let sphere = volume.corners_ws().is_none();
    let c = view * glam::Vec3::from_array(volume.centre_ws()).extend(1.0);
    let (depth, r) = (-c.z, volume.bounding_radius());
    let to_tile = |p: glam::Vec4| {
        // All corners sit at depth >= near, so w > 0.
        let clip = proj * p;
        let px = ((clip.x / clip.w * 0.5 + 0.5) * sw).clamp(0.0, sw - 1.0);
        let py = ((1.0 - (clip.y / clip.w * 0.5 + 0.5)) * sh).clamp(0.0, sh - 1.0);
        ((px as u32 / TILE_SIZE).min(grid_w - 1), (py as u32 / TILE_SIZE).min(grid_h - 1))
    };
    for sz in b.slice_min..=b.slice_max {
        let (mut x0, mut x1, mut y0, mut y1) = (b.tile_min_x, b.tile_max_x, b.tile_min_y, b.tile_max_y);
        if sphere {
            // Padded a little: `depth_slice` rounds through a logarithm.
            let z0 = (slice_depth(sz, near, far) * 0.999).max(depth - r).max(near);
            let z1 = if sz + 1 >= NUM_DEPTH_SLICES {
                depth + r
            } else {
                (slice_depth(sz + 1, near, far) * 1.001).min(depth + r)
            };
            let dz = (z0 - depth).max(depth - z1).max(0.0);
            let rs = (r * r - dz * dz).max(0.0).sqrt();
            let (mut t0, mut t1) = ((u32::MAX, u32::MAX), (0, 0));
            for corner in 0..8 {
                let pick = |bit: usize, lo: f32, hi: f32| if corner & bit == 0 { lo } else { hi };
                let (tx, ty) = to_tile(glam::Vec4::new(
                    pick(1, c.x - rs, c.x + rs),
                    pick(2, c.y - rs, c.y + rs),
                    -pick(4, z0, z1),
                    1.0,
                ));
                (t0, t1) = ((t0.0.min(tx), t0.1.min(ty)), (t1.0.max(tx), t1.1.max(ty)));
            }
            (x0, x1, y0, y1) = (x0.max(t0.0), x1.min(t1.0), y0.max(t0.1), y1.min(t1.1));
        }
        for ty in y0..=y1 {
            let row = (sz * grid_h * grid_w + ty * grid_w) as usize;
            for tx in x0..=x1 {
                if row + (tx as usize) < total_froxels {
                    visit(row + tx as usize);
                }
            }
        }
    }
}

/// Compute the froxel span a volume covers, or `None` if it is off-screen or
/// entirely behind the camera.
///
/// Split out of the assignment loop so it can be unit-tested without a GPU.
#[allow(clippy::too_many_arguments)]
fn volume_froxel_bounds<V: ClusterVolume>(
    light: &V,
    view: glam::Mat4,
    proj: glam::Mat4,
    sw: f32,
    sh: f32,
    near: f32,
    far: f32,
    grid_w: u32,
    grid_h: u32,
) -> Option<FroxelBounds> {
    let centre = light.centre_ws();
    let pos_ws = glam::Vec4::new(centre[0], centre[1], centre[2], 1.0);

    // In a right-handed view matrix the camera looks along -Z, so `depth`
    // (positive into the screen) is `-pos_vs.z`.
    let pos_vs = view * pos_ws;
    let depth = -pos_vs.z;
    let range = light.bounding_radius();

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

    // Conservative screen AABB: the sphere's view-space box, clipped to the
    // near plane, projected through its eight corners. Every corner is in
    // front of the camera, and x/z and y/z are extremal at a box's corners,
    // so this bounds every visible point of the sphere.
    //
    // This used to project four points at the *centre's* depth. A sphere the
    // camera stands in, or that passes beside it, spreads far wider on screen
    // nearer the camera than at its centre, so the light was binned into a
    // rectangle of tiles and every pixel outside lost it: a hard, tile-aligned
    // dark block that slid with the view. Interiors hit it constantly — the
    // camera is nearly always inside a few practicals' 5 m ranges.
    let (mut sx0, mut sx1, mut sy0, mut sy1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for corner in 0..8 {
        let pick = |bit: usize, lo: f32, hi: f32| if corner & bit == 0 { lo } else { hi };
        let (px, py) = project_to_screen(
            pick(1, pos_vs.x - range, pos_vs.x + range),
            pick(2, pos_vs.y - range, pos_vs.y + range),
            pick(4, z_min, z_max),
        );
        (sx0, sx1, sy0, sy1) = (sx0.min(px), sx1.max(px), sy0.min(py), sy1.max(py));
    }
    let (mut z_min, mut z_max) = (z_min, z_max);

    // A box, when every corner is in front of the near plane: its projected
    // corners bound it exactly. A corner behind the camera does not project,
    // so a box the camera stands in keeps the sphere.
    if let Some(corners) = light.corners_ws() {
        let vs = corners.map(|c| view * c.extend(1.0));
        if vs.iter().all(|v| -v.z >= near) {
            (sx0, sx1, sy0, sy1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
            (z_min, z_max) = (f32::MAX, f32::MIN);
            for v in vs {
                let (px, py) = project_to_screen(v.x, v.y, -v.z);
                (sx0, sx1, sy0, sy1) = (sx0.min(px), sx1.max(px), sy0.min(py), sy1.max(py));
                (z_min, z_max) = (z_min.min(-v.z), z_max.max(-v.z));
            }
        }
    }

    let px_min_x = sx0.max(0.0);
    let px_max_x = sx1.min(sw);
    let px_min_y = sy0.max(0.0);
    let px_max_y = sy1.min(sh);

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

/// The three reusable buffers a binning pass fills.
///
/// Phase CONTROL-O pulled these off `ClusterGrid` into their own struct so a
/// second consumer — the decal grid — can own a set without owning a light
/// buffer it has no use for. `ClusterGrid` keeps its fields inline; this is
/// the shape the generic entry point takes.
#[derive(Default)]
pub struct BinScratch {
    /// Per-froxel count, then reused as the per-froxel write cursor.
    pub counts: Vec<u32>,
    /// Per-froxel `(offset, count)` table uploaded to the GPU.
    pub offsets: Vec<ClusterOffset>,
    /// Flattened froxel → volume index list.
    pub index_list: Vec<u32>,
}

/// Bin any [`ClusterVolume`] list into froxels.
///
/// The public face of the counting sort below, so decals get the same binning
/// as lights rather than a second implementation of it.
#[allow(clippy::too_many_arguments)]
pub fn bin_volumes<V: ClusterVolume>(
    volumes: &[V],
    view: glam::Mat4,
    proj: glam::Mat4,
    sw: f32,
    sh: f32,
    near: f32,
    far: f32,
    grid_w: u32,
    grid_h: u32,
    total_froxels: usize,
    scratch: &mut BinScratch,
) {
    assign_froxels(
        volumes,
        view,
        proj,
        sw,
        sh,
        near,
        far,
        grid_w,
        grid_h,
        total_froxels,
        &mut scratch.counts,
        &mut scratch.offsets,
        &mut scratch.index_list,
    );
}

/// Bin `lights` into froxels using a counting sort.
///
/// Fills `counts` (used as scratch, ends up holding each froxel's write
/// cursor), `offsets` (the per-froxel table uploaded to the GPU) and
/// `index_list` (the flattened froxel -> light indices). All three are reused
/// across frames, so this performs no heap allocation in the steady state.
#[allow(clippy::too_many_arguments)]
fn assign_froxels<V: ClusterVolume>(
    lights: &[V],
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
        for_each_froxel(light, view, proj, sw, sh, near, far, grid_w, grid_h, total_froxels, |f| {
            counts[f] += 1
        });
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
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if total_indices == MAX_LIGHT_INDICES
        && counts.iter().map(|&c| c as usize).sum::<usize>() > total_indices
        && !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed)
    {
        // Truncation drops every volume from the far froxels at once.
        tracing::warn!("cluster index list full ({MAX_LIGHT_INDICES}); far froxels lose their lights");
    }

    // ── 3. Fill pass ─────────────────────────────────────────────────────────
    // Reuse `counts` as the per-froxel write cursor.
    for (froxel, cursor) in counts.iter_mut().enumerate() {
        *cursor = offsets[froxel].offset;
    }

    index_list.clear();
    index_list.resize(total_indices, 0);

    for (light_idx, light) in lights.iter().enumerate() {
        for_each_froxel(light, view, proj, sw, sh, near, far, grid_w, grid_h, total_froxels, |f| {
            let cursor = &mut counts[f];
            // Respect the clamp applied during the prefix sum.
            if *cursor < offsets[f].offset + offsets[f].count {
                index_list[*cursor as usize] = light_idx as u32;
                *cursor += 1;
            }
        });
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
            for_each_froxel(l, view, proj, sw, sh, near, far, grid_w, grid_h, total_froxels, |f| {
                froxel_lists[f].push(light_idx as u32)
            });
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
        assign_froxels::<GpuLocalLight>(
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

    #[test]
    fn every_visible_point_of_a_light_lands_in_its_binned_tiles() {
        // A room's practicals mostly surround the camera: beside it, just
        // behind it, overhead. Each visible point of the sphere must fall in
        // the binned tiles and slices, or the pixel there silently loses the
        // light — the tile-aligned dark block from the Office review.
        let eye = glam::Vec3::new(0.0, 1.6, 0.0);
        let view = glam::Mat4::look_at_rh(eye, glam::Vec3::new(0.3, 1.4, -5.0), glam::Vec3::Y);
        let proj = glam::Mat4::perspective_rh(60.0_f32.to_radians(), 16.0 / 10.0, 0.1, 1000.0);
        let (sw, sh, near, far) = (2160.0_f32, 1350.0_f32, 0.1, 1000.0);
        let (gw, gh) = ((sw as u32).div_ceil(TILE_SIZE), (sh as u32).div_ceil(TILE_SIZE));
        for (pos, range) in [
            ([1.8, 2.7, -0.6], 4.6),  // overhead, beside the camera
            ([-1.2, 2.6, 1.0], 5.8),  // behind the shoulder
            ([3.0, 1.2, -1.0], 4.6),  // off-screen to the right, reaching across
            ([0.4, 2.5, -6.0], 4.6),  // well in front
        ] {
            let l = light(pos, range);
            let mut binned = std::collections::HashSet::new();
            let total = (gw * gh * NUM_DEPTH_SLICES) as usize;
            for_each_froxel(&l, view, proj, sw, sh, near, far, gw, gh, total, |f| {
                binned.insert(f);
            });
            let centre = glam::Vec3::from_array(pos);
            let mut checked = 0;
            for i in 0..4096 {
                // Deterministic lattice through the ball.
                let f = |k: u32| ((i as u32 * k) % 997) as f32 / 996.0 * 2.0 - 1.0;
                let offset = glam::Vec3::new(f(37), f(101), f(271));
                if offset.length() > 1.0 {
                    continue;
                }
                let p = centre + offset * range;
                let depth = -(view * p.extend(1.0)).z;
                let clip = proj * view * p.extend(1.0);
                if depth <= near || clip.w <= 0.0 {
                    continue;
                }
                let (ndc_x, ndc_y) = (clip.x / clip.w, clip.y / clip.w);
                if ndc_x.abs() >= 1.0 || ndc_y.abs() >= 1.0 {
                    continue;
                }
                let tx = (((ndc_x * 0.5 + 0.5) * sw) as u32 / TILE_SIZE).min(gw - 1);
                let ty = (((1.0 - (ndc_y * 0.5 + 0.5)) * sh) as u32 / TILE_SIZE).min(gh - 1);
                let slice = depth_slice(depth, near, far);
                assert!(
                    binned.contains(&((slice * gh * gw + ty * gw + tx) as usize)),
                    "light at {pos:?} r={range}: point {p} in tile ({tx},{ty}) slice {slice} not binned"
                );
                checked += 1;
            }
            assert!(checked > 50, "too few visible samples for {pos:?}");
        }
    }

    #[test]
    fn a_lit_office_fits_the_index_list() {
        // The Office: 64 practicals with 4-8 m ranges over the desks and the
        // camera standing among them at 1600p. A list truncated in froxel
        // order drops every light from the far slices at once: the hard dark
        // block that slid with the view in the 2026-10-03 review.
        let view = glam::Mat4::look_at_rh(
            glam::Vec3::new(0.0, 1.6, 0.0),
            glam::Vec3::new(0.5, 1.2, -10.0),
            glam::Vec3::Y,
        );
        let proj = glam::Mat4::perspective_rh(70.0_f32.to_radians(), 16.0 / 10.0, 0.1, 1000.0);
        let (sw, sh, near, far) = (2560.0_f32, 1600.0_f32, 0.1, 1000.0);
        let (gw, gh) = ((sw as u32).div_ceil(TILE_SIZE), (sh as u32).div_ceil(TILE_SIZE));
        let total = (gw * gh * NUM_DEPTH_SLICES) as usize;
        let lights: Vec<_> = (0..64)
            .map(|i| {
                let (col, row) = ((i % 8) as f32, (i / 8) as f32);
                light([col * 3.0 - 10.5, 2.7, 6.0 - row * 4.0], 4.0 + (i % 5) as f32)
            })
            .collect();
        let (mut counts, mut offsets, mut index_list) = (Vec::new(), Vec::new(), Vec::new());
        assign_froxels(
            &lights, view, proj, sw, sh, near, far, gw, gh, total, &mut counts, &mut offsets,
            &mut index_list,
        );
        let (reference, _) = reference_assign(&lights, view, proj, sw, sh, near, far, gw, gh, total);
        assert_eq!(index_list.len(), reference.len(), "the index list was truncated");
        assert!(index_list.len() * 2 < MAX_LIGHT_INDICES, "{} entries leaves no headroom", index_list.len());
    }

    /// A flat box on the ground, 2.6 x 0.2 x 0.8 m (a puddle decal).
    struct Slab {
        centre: glam::Vec3,
        boxed: bool,
    }

    impl ClusterVolume for Slab {
        fn centre_ws(&self) -> [f32; 3] {
            self.centre.to_array()
        }
        fn bounding_radius(&self) -> f32 {
            glam::Vec3::new(2.6, 0.2, 0.8).length() * 0.5
        }
        fn corners_ws(&self) -> Option<[glam::Vec3; 8]> {
            self.boxed.then(|| {
                std::array::from_fn(|i| {
                    let h = glam::Vec3::new(1.3, 0.1, 0.4);
                    let sign = |bit: usize| if i & bit == 0 { -1.0 } else { 1.0 };
                    self.centre + h * glam::Vec3::new(sign(1), sign(2), sign(4))
                })
            })
        }
    }

    #[test]
    fn a_ground_decal_is_binned_by_its_box_not_its_sphere() {
        // Eye height, looking down the street at a puddle 6 m ahead.
        let view = glam::Mat4::look_at_rh(
            glam::Vec3::new(0.0, 1.6, 0.0),
            glam::Vec3::new(0.0, 0.0, -10.0),
            glam::Vec3::Y,
        );
        let proj = glam::Mat4::perspective_rh(60.0_f32.to_radians(), 16.0 / 10.0, 0.1, 1000.0);
        let (sw, sh, near, far) = (2160.0, 1350.0, 0.1, 1000.0);
        let (gw, gh) = (68, 43);
        let froxels = |b: FroxelBounds| {
            (b.tile_max_x - b.tile_min_x + 1) * (b.tile_max_y - b.tile_min_y + 1) * (b.slice_max - b.slice_min + 1)
        };
        let centre = glam::Vec3::new(0.0, 0.0, -6.0);
        let sphere = volume_froxel_bounds(&Slab { centre, boxed: false }, view, proj, sw, sh, near, far, gw, gh)
            .expect("on screen");
        let boxed = volume_froxel_bounds(&Slab { centre, boxed: true }, view, proj, sw, sh, near, far, gw, gh)
            .expect("on screen");
        assert!(froxels(boxed) * 2 < froxels(sphere), "box {boxed:?} vs sphere {sphere:?}");
        // Still covers the puddle's own centre.
        let clip = proj * view * centre.extend(1.0);
        let px = ((clip.x / clip.w * 0.5 + 0.5) * sw) as u32 / TILE_SIZE;
        let py = ((1.0 - (clip.y / clip.w * 0.5 + 0.5)) * sh) as u32 / TILE_SIZE;
        assert!((boxed.tile_min_x..=boxed.tile_max_x).contains(&px));
        assert!((boxed.tile_min_y..=boxed.tile_max_y).contains(&py));
        // A box the camera stands in falls back to the sphere.
        let under = glam::Vec3::new(0.0, 1.6, 0.0);
        assert_eq!(
            volume_froxel_bounds(&Slab { centre: under, boxed: true }, view, proj, sw, sh, near, far, gw, gh),
            volume_froxel_bounds(&Slab { centre: under, boxed: false }, view, proj, sw, sh, near, far, gw, gh),
        );
    }
}
