//! Heightmap terrain system (Phase 14).
//!
//! ## Reference Architecture
//!
//! - `example_repo/fyrox/Fyrox-master/fyrox-impl/src/scene/terrain/mod.rs` —
//!   chunked heightmap layout, height accessors, ray ↔ heightmap intersection,
//!   brush stroke flow (Phase 14A/14D).
//! - `example_repo/fyrox/Fyrox-master/fyrox-impl/src/scene/terrain/quadtree.rs` —
//!   distance-based LOD selection (adapted from per-chunk quadtrees to a flat
//!   per-chunk LOD with a ≤1-level neighbor constraint).
//! - `example_repo/CDLOD-master` — log2 distance → LOD mapping.
//!
//! Unlike meshes in the visibility-buffer pipeline, terrain owns dedicated
//! per-chunk vertex buffers and renders through [`crate::pass::terrain::TerrainPass`]
//! directly into the HDR target (same integration point as the water pass).

pub mod brush;
pub mod mesh;
pub mod textures;

use std::collections::HashMap;

use mesh::{EDGE_EAST, EDGE_NORTH, EDGE_SOUTH, EDGE_WEST, MAX_TERRAIN_LOD};
use textures::{Splatmap, TerrainLayerTextures, LAYER_NAMES, TERRAIN_LAYER_COUNT};

/// Static terrain configuration (Phase 14A-1). The matching ECS component
/// `somnium_core::TerrainComponent` stores a copy of these plus the terrain id.
#[derive(Debug, Clone, Copy)]
pub struct TerrainDescriptor {
    /// Cells (quads) per chunk edge. Must be a power of two ≥ `1 << MAX_TERRAIN_LOD`.
    pub chunk_cells: u32,
    /// Number of chunks along X and Z.
    pub grid_size: [u32; 2],
    /// World-space distance between adjacent vertices (metres).
    pub cell_size: f32,
    /// World-space multiplier applied to raw heightmap values.
    pub height_scale: f32,
    /// Camera distance at which LOD 0 ends; each further LOD doubles the range.
    pub lod_base_range: f32,
}

impl Default for TerrainDescriptor {
    fn default() -> Self {
        Self {
            chunk_cells: 64,
            grid_size: [16, 16],
            cell_size: 1.0,
            height_scale: 1.0,
            lod_base_range: 96.0,
        }
    }
}

impl TerrainDescriptor {
    /// Total vertices along X (`grid_size[0] * chunk_cells + 1`).
    pub fn total_vertices_x(&self) -> u32 {
        self.grid_size[0] * self.chunk_cells + 1
    }
    /// Total vertices along Z.
    pub fn total_vertices_z(&self) -> u32 {
        self.grid_size[1] * self.chunk_cells + 1
    }
    /// World-space size of the whole terrain (metres).
    pub fn world_size(&self) -> [f32; 2] {
        [
            (self.grid_size[0] * self.chunk_cells) as f32 * self.cell_size,
            (self.grid_size[1] * self.chunk_cells) as f32 * self.cell_size,
        ]
    }
}

/// One renderable terrain chunk (Phase 14A-2).
pub struct TerrainChunk {
    pub grid_pos: [u32; 2],
    /// Terrain-local AABB, updated on rebuild (used for LOD distance).
    pub aabb_min: glam::Vec3,
    pub aabb_max: glam::Vec3,
    /// Full-resolution vertex grid for this chunk.
    pub vertex_buffer: wgpu::Buffer,
    /// Heights changed — vertices must be regenerated before the next draw.
    pub dirty: bool,
    /// LOD selected for the current frame.
    pub lod: u8,
    /// Edge-stitch mask for the current frame (see `mesh::EDGE_*`).
    pub edge_mask: u8,
}

/// A material layer (Phase 14A-2). Texture data lives in the shared
/// `TerrainLayerTextures` arrays; this carries the per-layer parameters.
pub struct TerrainLayer {
    pub name: String,
    /// UV repeats per metre when sampling this layer's textures.
    pub tiling: f32,
}

/// GPU uniform mirrored by `TerrainParams` in `terrain.wgsl` (80 bytes).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuTerrainParams {
    pub layer_tiling: [f32; 4],
    /// xy = brush world XZ, z = radius, w = mode (0 off, 1 sculpt, 2 paint).
    pub brush: [f32; 4],
    pub terrain_origin: [f32; 2],
    pub inv_world_size: [f32; 2],
    /// Layer index used for triplanar cliff projection (rock = 2).
    pub cliff_layer: u32,
    pub _pad: [u32; 3],
}

/// CPU + GPU state for one terrain (Phase 14A-2 `TerrainData`).
///
/// Owned by the renderer (like `GeometryPool`); the ECS only stores the
/// lightweight `TerrainComponent` with this terrain's id.
pub struct TerrainData {
    pub desc: TerrainDescriptor,
    /// Raw heights, row-major (Z-major, X-minor), `total_x * total_z` values.
    pub heightmap: Vec<f32>,
    pub chunks: Vec<TerrainChunk>,
    pub layers: Vec<TerrainLayer>,
    pub splatmap: Splatmap,
    pub layer_textures: TerrainLayerTextures,

    /// Shared LOD index buffers keyed by `(lod, edge_mask)`, lazily built.
    index_buffers: HashMap<(u8, u8), (wgpu::Buffer, u32)>,

    /// `TerrainParams` uniform (group 1, binding 0).
    pub params_buffer: wgpu::Buffer,
    /// Model matrix uniform (group 1, binding 1).
    pub model_buffer: wgpu::Buffer,
    /// Group-1 bind group for `TerrainPass`.
    pub bind_group: wgpu::BindGroup,

    /// Model matrix submitted for the current frame.
    pub model: glam::Mat4,
    /// Brush cursor uniform state (set by the editor each frame).
    pub brush_cursor: [f32; 4],
}

impl TerrainData {
    /// Create a flat terrain with the default grass/dirt/rock/snow layers.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain_bgl: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        desc: TerrainDescriptor,
    ) -> Self {
        assert!(
            desc.chunk_cells.is_power_of_two() && desc.chunk_cells >= (1 << MAX_TERRAIN_LOD),
            "chunk_cells must be a power of two ≥ {}",
            1 << MAX_TERRAIN_LOD
        );

        let total = (desc.total_vertices_x() * desc.total_vertices_z()) as usize;
        let heightmap = vec![0.0f32; total];

        let verts_per_chunk = (desc.chunk_cells + 1) * (desc.chunk_cells + 1);
        let mut chunks = Vec::with_capacity((desc.grid_size[0] * desc.grid_size[1]) as usize);
        for cz in 0..desc.grid_size[1] {
            for cx in 0..desc.grid_size[0] {
                let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Terrain Chunk Vertices"),
                    size: verts_per_chunk as u64 * std::mem::size_of::<somnium_asset::Vertex>() as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                chunks.push(TerrainChunk {
                    grid_pos: [cx, cz],
                    aabb_min: glam::Vec3::ZERO,
                    aabb_max: glam::Vec3::ZERO,
                    vertex_buffer,
                    dirty: true,
                    lod: 0,
                    edge_mask: 0,
                });
            }
        }

        let layers = LAYER_NAMES
            .iter()
            .map(|name| TerrainLayer { name: (*name).to_string(), tiling: 0.25 })
            .collect();

        // One splat texel per heightmap cell; rows stay 256-byte aligned
        // because chunk_cells is a power of two ≥ 16.
        let splatmap = Splatmap::new(
            device,
            queue,
            desc.grid_size[0] * desc.chunk_cells,
            desc.grid_size[1] * desc.chunk_cells,
        );
        let layer_textures = TerrainLayerTextures::generate_default(device, queue);

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Terrain Params"),
            size: std::mem::size_of::<GpuTerrainParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let model_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Terrain Model"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Terrain Bind Group"),
            layout: terrain_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: model_buffer.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&splatmap.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&layer_textures.albedo_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&layer_textures.normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&layer_textures.roughness_view),
                },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        });

        Self {
            desc,
            heightmap,
            chunks,
            layers,
            splatmap,
            layer_textures,
            index_buffers: HashMap::new(),
            params_buffer,
            model_buffer,
            bind_group,
            model: glam::Mat4::IDENTITY,
            brush_cursor: [0.0; 4],
        }
    }

    // ── Height accessors (Phase 14A-3) ──────────────────────────────────────

    /// Raw height at a vertex (clamped to the grid).
    pub fn height_at(&self, xi: u32, zi: u32) -> f32 {
        let xi = xi.min(self.desc.total_vertices_x() - 1);
        let zi = zi.min(self.desc.total_vertices_z() - 1);
        self.heightmap[(zi * self.desc.total_vertices_x() + xi) as usize]
    }

    /// Set a raw height and mark every chunk containing that vertex dirty.
    pub fn set_height(&mut self, xi: u32, zi: u32, value: f32) {
        let tx = self.desc.total_vertices_x();
        if xi >= tx || zi >= self.desc.total_vertices_z() {
            return;
        }
        self.heightmap[(zi * tx + xi) as usize] = value;
        self.mark_region_dirty(xi, zi, xi, zi);
    }

    /// Mark all chunks intersecting the inclusive vertex region as dirty.
    /// Normals reach one vertex past the region, so the region is expanded.
    pub fn mark_region_dirty(&mut self, x0: u32, z0: u32, x1: u32, z1: u32) {
        let cells = self.desc.chunk_cells;
        let x0 = x0.saturating_sub(1) / cells;
        let z0 = z0.saturating_sub(1) / cells;
        let x1 = ((x1 + 1) / cells).min(self.desc.grid_size[0] - 1);
        let z1 = ((z1 + 1) / cells).min(self.desc.grid_size[1] - 1);
        for cz in z0..=z1 {
            for cx in x0..=x1 {
                self.chunks[(cz * self.desc.grid_size[0] + cx) as usize].dirty = true;
            }
        }
    }

    /// Scaled (world-space) height at an arbitrary terrain-local XZ position,
    /// bilinearly interpolated (Phase 14A-3 `world_height_at`).
    pub fn world_height_at(&self, local_x: f32, local_z: f32) -> f32 {
        let fx = (local_x / self.desc.cell_size)
            .clamp(0.0, (self.desc.total_vertices_x() - 1) as f32);
        let fz = (local_z / self.desc.cell_size)
            .clamp(0.0, (self.desc.total_vertices_z() - 1) as f32);
        let (x0, z0) = (fx.floor() as u32, fz.floor() as u32);
        let (tx, tz) = (fx - fx.floor(), fz - fz.floor());

        let h00 = self.height_at(x0, z0);
        let h10 = self.height_at(x0 + 1, z0);
        let h01 = self.height_at(x0, z0 + 1);
        let h11 = self.height_at(x0 + 1, z0 + 1);
        let h = h00 * (1.0 - tx) * (1.0 - tz)
            + h10 * tx * (1.0 - tz)
            + h01 * (1.0 - tx) * tz
            + h11 * tx * tz;
        h * self.desc.height_scale
    }

    // ── Per-frame update ─────────────────────────────────────────────────────

    /// Select per-chunk LODs from the camera position (terrain-local), clamp
    /// neighbor differences to ≤ 1 level, and derive edge stitch masks.
    pub fn select_lods(&mut self, local_camera_pos: glam::Vec3) {
        let [gx, gz] = self.desc.grid_size;
        let chunk_world = self.desc.chunk_cells as f32 * self.desc.cell_size;

        // 1. Distance-based LOD per chunk (Phase 14B-2 formula).
        for chunk in &mut self.chunks {
            let center = glam::Vec3::new(
                (chunk.grid_pos[0] as f32 + 0.5) * chunk_world,
                (chunk.aabb_min.y + chunk.aabb_max.y) * 0.5,
                (chunk.grid_pos[1] as f32 + 0.5) * chunk_world,
            );
            let dist = local_camera_pos.distance(center).max(0.01);
            let lod_f = (dist / self.desc.lod_base_range).log2().floor();
            chunk.lod = lod_f.clamp(0.0, MAX_TERRAIN_LOD as f32) as u8;
        }

        // 2. Relax until adjacent chunks differ by at most one level.
        let at = |chunks: &[TerrainChunk], x: i64, z: i64| -> Option<u8> {
            if x < 0 || z < 0 || x >= gx as i64 || z >= gz as i64 {
                return None;
            }
            Some(chunks[(z as u32 * gx + x as u32) as usize].lod)
        };
        loop {
            let mut changed = false;
            for i in 0..self.chunks.len() {
                let (cx, cz) = (self.chunks[i].grid_pos[0] as i64, self.chunks[i].grid_pos[1] as i64);
                let mut min_neighbor = u8::MAX;
                for (dx, dz) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                    if let Some(l) = at(&self.chunks, cx + dx, cz + dz) {
                        min_neighbor = min_neighbor.min(l);
                    }
                }
                if min_neighbor != u8::MAX && self.chunks[i].lod > min_neighbor + 1 {
                    self.chunks[i].lod = min_neighbor + 1;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        // 3. Edge mask: stitch toward any strictly coarser neighbor.
        for i in 0..self.chunks.len() {
            let (cx, cz) = (self.chunks[i].grid_pos[0] as i64, self.chunks[i].grid_pos[1] as i64);
            let lod = self.chunks[i].lod;
            let mut mask = 0u8;
            for (dx, dz, bit) in [
                (-1i64, 0i64, EDGE_WEST),
                (1, 0, EDGE_EAST),
                (0, -1, EDGE_NORTH),
                (0, 1, EDGE_SOUTH),
            ] {
                if at(&self.chunks, cx + dx, cz + dz) == Some(lod + 1) {
                    mask |= bit;
                }
            }
            self.chunks[i].edge_mask = mask;
        }
    }

    /// Regenerate vertices for dirty chunks and upload them (Phase 14B-1).
    pub fn rebuild_dirty_chunks(&mut self, queue: &wgpu::Queue) {
        let desc = self.desc;
        for chunk in &mut self.chunks {
            if !chunk.dirty {
                continue;
            }
            let vertices = mesh::build_chunk_vertices(
                &self.heightmap,
                desc.total_vertices_x(),
                desc.total_vertices_z(),
                desc.chunk_cells,
                chunk.grid_pos,
                desc.cell_size,
                desc.height_scale,
            );
            queue.write_buffer(&chunk.vertex_buffer, 0, bytemuck::cast_slice(&vertices));

            let mut min_h = f32::MAX;
            let mut max_h = f32::MIN;
            for v in &vertices {
                min_h = min_h.min(v.position[1]);
                max_h = max_h.max(v.position[1]);
            }
            let chunk_world = desc.chunk_cells as f32 * desc.cell_size;
            let base = glam::Vec2::new(
                chunk.grid_pos[0] as f32 * chunk_world,
                chunk.grid_pos[1] as f32 * chunk_world,
            );
            chunk.aabb_min = glam::Vec3::new(base.x, min_h, base.y);
            chunk.aabb_max = glam::Vec3::new(base.x + chunk_world, max_h, base.y + chunk_world);
            chunk.dirty = false;
        }
    }

    /// Lazily build the shared index buffers for every `(lod, mask)` pair
    /// the current chunk LOD assignment needs. Call before `TerrainPass::record`
    /// (which only does read-only lookups inside the render pass).
    pub fn ensure_index_buffers(&mut self, device: &wgpu::Device) {
        let cells = self.desc.chunk_cells;
        for i in 0..self.chunks.len() {
            let key = (self.chunks[i].lod, self.chunks[i].edge_mask);
            self.index_buffers.entry(key).or_insert_with(|| {
                let indices = mesh::build_lod_indices(cells, key.0, key.1);
                let buffer = wgpu::util::DeviceExt::create_buffer_init(
                    device,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("Terrain LOD Indices"),
                        contents: bytemuck::cast_slice(&indices),
                        usage: wgpu::BufferUsages::INDEX,
                    },
                );
                (buffer, indices.len() as u32)
            });
        }
    }

    /// Read-only index buffer lookup for a `(lod, mask)` pair.
    pub fn index_buffer_ref(&self, lod: u8, mask: u8) -> Option<(&wgpu::Buffer, u32)> {
        self.index_buffers.get(&(lod, mask)).map(|(b, n)| (b, *n))
    }

    /// Upload the params + model uniforms for this frame.
    pub fn upload_uniforms(&self, queue: &wgpu::Queue) {
        let [wx, wz] = self.desc.world_size();
        let origin = self.model.w_axis;
        let params = GpuTerrainParams {
            layer_tiling: [
                self.layers.first().map_or(0.25, |l| l.tiling),
                self.layers.get(1).map_or(0.25, |l| l.tiling),
                self.layers.get(2).map_or(0.25, |l| l.tiling),
                self.layers.get(3).map_or(0.25, |l| l.tiling),
            ],
            brush: self.brush_cursor,
            terrain_origin: [origin.x, origin.z],
            inv_world_size: [1.0 / wx, 1.0 / wz],
            cliff_layer: 2,
            _pad: [0; 3],
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&params));
        queue.write_buffer(&self.model_buffer, 0, bytemuck::bytes_of(&self.model.to_cols_array()));
    }

    // ── Raycast (Phase 14D-2 step 1) ─────────────────────────────────────────

    /// Intersect a world-space ray with the heightmap surface.
    ///
    /// Adapted from Fyrox `Terrain::raycast`: instead of testing every cell's
    /// two triangles, the ray is marched at half-cell steps until it crosses
    /// the surface, then bisected. Returns the world-space hit position.
    pub fn raycast(&self, origin: glam::Vec3, dir: glam::Vec3) -> Option<glam::Vec3> {
        let inv_model = self.model.inverse();
        let local_origin = inv_model.transform_point3(origin);
        let local_dir = inv_model.transform_vector3(dir).normalize_or_zero();
        if local_dir == glam::Vec3::ZERO {
            return None;
        }

        let [wx, wz] = self.desc.world_size();
        let step = self.desc.cell_size * 0.5;
        let max_t = 4096.0 * self.desc.cell_size;

        let above = |p: glam::Vec3| p.y - self.world_height_at(p.x, p.z);
        let in_bounds = |p: glam::Vec3| p.x >= 0.0 && p.z >= 0.0 && p.x <= wx && p.z <= wz;

        let mut t = 0.0f32;
        let mut prev_t = 0.0f32;
        let mut prev_sign = above(local_origin) >= 0.0;
        let mut was_inside = in_bounds(local_origin);

        while t < max_t {
            t += step;
            let p = local_origin + local_dir * t;
            if !in_bounds(p) {
                if was_inside {
                    return None; // exited the terrain footprint
                }
                prev_t = t;
                continue;
            }
            was_inside = true;
            let sign = above(p) >= 0.0;
            if sign != prev_sign {
                // Bisect between prev_t and t for a precise hit.
                let (mut lo, mut hi) = (prev_t, t);
                for _ in 0..16 {
                    let mid = (lo + hi) * 0.5;
                    let pm = local_origin + local_dir * mid;
                    if (above(pm) >= 0.0) == prev_sign {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                let hit = local_origin + local_dir * ((lo + hi) * 0.5);
                return Some(self.model.transform_point3(hit));
            }
            prev_sign = sign;
            prev_t = t;
        }
        None
    }

    /// Number of layers (always `TERRAIN_LAYER_COUNT` for now).
    pub fn layer_count(&self) -> u32 {
        (self.layers.len() as u32).min(TERRAIN_LAYER_COUNT)
    }

    // ── Binary sidecar I/O (Phase 14F-3) ─────────────────────────────────────
    //
    // The `.somnium` scene JSON stores only the terrain configuration; the
    // heightmap (`f32` LE) and splatmap (RGBA8) are written to a sidecar
    // binary because a 1041×1041 heightmap would bloat the JSON by megabytes.

    const SIDECAR_MAGIC: u32 = 0x5354_4552; // "STER"

    /// Write heightmap + splatmap to a sidecar binary file.
    pub fn save_binary(&self, path: &str) -> std::io::Result<()> {
        let mut out: Vec<u8> = Vec::with_capacity(self.heightmap.len() * 4 + 24);
        out.extend(Self::SIDECAR_MAGIC.to_le_bytes());
        out.extend(1u32.to_le_bytes()); // version
        out.extend(self.desc.total_vertices_x().to_le_bytes());
        out.extend(self.desc.total_vertices_z().to_le_bytes());
        out.extend(self.splatmap.width.to_le_bytes());
        out.extend(self.splatmap.height.to_le_bytes());
        out.extend(bytemuck::cast_slice::<f32, u8>(&self.heightmap));
        out.extend(bytemuck::cast_slice::<[u8; 4], u8>(&self.splatmap.data));
        std::fs::write(path, out)
    }

    /// Load heightmap + splatmap from a sidecar binary written by
    /// [`Self::save_binary`]. Dimensions must match this terrain's descriptor.
    pub fn load_binary(&mut self, path: &str) -> Result<(), String> {
        let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
        let u32_at = |off: usize| -> Result<u32, String> {
            bytes
                .get(off..off + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .ok_or_else(|| "truncated terrain sidecar".to_string())
        };
        if u32_at(0)? != Self::SIDECAR_MAGIC {
            return Err("not a terrain sidecar file".into());
        }
        let (tx, tz) = (u32_at(8)?, u32_at(12)?);
        let (sw, sh) = (u32_at(16)?, u32_at(20)?);
        if tx != self.desc.total_vertices_x()
            || tz != self.desc.total_vertices_z()
            || sw != self.splatmap.width
            || sh != self.splatmap.height
        {
            return Err("terrain sidecar dimensions do not match descriptor".into());
        }
        let h_bytes = (tx * tz) as usize * 4;
        let s_bytes = (sw * sh) as usize * 4;
        let body = bytes
            .get(24..24 + h_bytes + s_bytes)
            .ok_or("truncated terrain sidecar")?;
        self.heightmap
            .copy_from_slice(bytemuck::cast_slice(&body[..h_bytes]));
        self.splatmap
            .data
            .copy_from_slice(bytemuck::cast_slice(&body[h_bytes..]));
        for chunk in &mut self.chunks {
            chunk.dirty = true;
        }
        self.splatmap.mark_dirty(0, 0, sw - 1, sh - 1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_desc() -> TerrainDescriptor {
        TerrainDescriptor {
            chunk_cells: 16,
            grid_size: [4, 4],
            cell_size: 1.0,
            height_scale: 2.0,
            lod_base_range: 24.0,
        }
    }

    // CPU-only stand-in: exercise LOD math without a GPU device.
    fn lods_for(camera: glam::Vec3, desc: TerrainDescriptor) -> Vec<u8> {
        // Mirror of select_lods steps 1–2 on plain data.
        let [gx, gz] = desc.grid_size;
        let chunk_world = desc.chunk_cells as f32 * desc.cell_size;
        let mut lods = vec![0u8; (gx * gz) as usize];
        for cz in 0..gz {
            for cx in 0..gx {
                let center = glam::Vec3::new(
                    (cx as f32 + 0.5) * chunk_world, 0.0, (cz as f32 + 0.5) * chunk_world,
                );
                let dist = camera.distance(center).max(0.01);
                let lod_f = (dist / desc.lod_base_range).log2().floor();
                lods[(cz * gx + cx) as usize] = lod_f.clamp(0.0, MAX_TERRAIN_LOD as f32) as u8;
            }
        }
        loop {
            let mut changed = false;
            for cz in 0..gz as i64 {
                for cx in 0..gx as i64 {
                    let mut min_n = u8::MAX;
                    for (dx, dz) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                        let (nx, nz) = (cx + dx, cz + dz);
                        if nx >= 0 && nz >= 0 && nx < gx as i64 && nz < gz as i64 {
                            min_n = min_n.min(lods[(nz as u32 * gx + nx as u32) as usize]);
                        }
                    }
                    let i = (cz as u32 * gx + cx as u32) as usize;
                    if min_n != u8::MAX && lods[i] > min_n + 1 {
                        lods[i] = min_n + 1;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        lods
    }

    #[test]
    fn lod_increases_with_distance_and_neighbors_differ_by_at_most_one() {
        let desc = test_desc();
        let lods = lods_for(glam::Vec3::new(8.0, 0.0, 8.0), desc);
        let [gx, gz] = desc.grid_size;
        assert_eq!(lods[0], 0, "nearest chunk should be LOD 0");
        assert!(*lods.last().unwrap() > 0, "far chunk should be coarser");
        for cz in 0..gz {
            for cx in 0..gx {
                let l = lods[(cz * gx + cx) as usize] as i32;
                if cx + 1 < gx {
                    let r = lods[(cz * gx + cx + 1) as usize] as i32;
                    assert!((l - r).abs() <= 1);
                }
                if cz + 1 < gz {
                    let d = lods[((cz + 1) * gx + cx) as usize] as i32;
                    assert!((l - d).abs() <= 1);
                }
            }
        }
    }
}
