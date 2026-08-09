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
//! Phase 25A-2: terrain is no longer outside the visibility-buffer pipeline.
//! Chunk vertices live in the global [`crate::geometry::GeometryPool`], chunks
//! are submitted as ordinary `DrawCommand`s, and they shade in `shading.wgsl`
//! like every other surface — which is what gives terrain GTAO, contact
//! shadows, traced visibility, IBL and correct TAA, and what retires the
//! duplicated shadow and cluster code the old terrain pass carried.

pub mod brush;
pub mod collider;
pub mod foliage;
pub mod foliage_paint;
pub mod heightmap;
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
    /// Offset of this chunk's full-resolution vertex grid in the global pool.
    ///
    /// Reserved once and rewritten in place forever (Phase 25A-2): the vertex
    /// count is `(chunk_cells + 1)²` and never changes, because sculpting
    /// rewrites height values rather than counts and a coarser LOD skips
    /// vertices through the index buffer. `u32::MAX` means the pool was full at
    /// terrain creation and this chunk draws nothing.
    pub vertex_offset: u32,
    /// Heights changed — vertices must be regenerated before the next draw.
    pub dirty: bool,
    /// LOD selected for the current frame.
    pub lod: u8,
    /// Edge-stitch mask for the current frame (see `mesh::EDGE_*`).
    pub edge_mask: u8,
}

/// Offset value meaning "this chunk has no pool space".
pub const UNALLOCATED: u32 = u32::MAX;

/// A material layer (Phase 14A-2). Texture data lives in the shared
/// `TerrainLayerTextures` arrays; this carries the per-layer parameters.
pub struct TerrainLayer {
    pub name: String,
    /// UV repeats per metre when sampling this layer's textures.
    pub tiling: f32,
}

/// Everything `shading.wgsl` needs to evaluate a terrain surface, mirrored by
/// `TerrainMaterial` in `terrain_material.wgsl` (112 bytes).
///
/// Phase 25A-2 moved this out of a per-terrain uniform and into a storage-buffer
/// array indexed by `Material::terrain_index`, because terrain now shades in the
/// same pass as everything else and there is no per-terrain bind group left to
/// hang a uniform on. The texture fields are bindless indices into the global
/// texture array rather than views: the splatmap and the four layers' albedo,
/// normal and roughness maps are registered there at terrain creation.
///
/// **Every `vec4` member sits at a 16-byte offset**, which is what keeps Rust's
/// `repr(C)` packing and WGSL's alignment rules agreeing — the same trap that
/// silently mis-decoded `GpuMaterial` when `emissive` was a `vec3`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuTerrainMaterial {
    /// UV repeats per metre, one per layer.                    offset 0  (32)
    pub layer_tiling: [f32; 8],
    /// xy = brush world XZ, z = radius, w = mode
    /// (0 off, 1 sculpt, 2 paint, 3 foliage).                  offset 32
    pub brush: [f32; 4],
    /// Bindless index of each layer's albedo+height map.       offset 48 (32)
    pub albedo_maps: [i32; 8],
    /// Bindless index of each layer's packed surface map
    /// (normal XY, roughness, occlusion).                      offset 80 (32)
    pub surface_maps: [i32; 8],
    /// World XZ of terrain-local (0, 0).                       offset 112
    pub terrain_origin: [f32; 2],
    /// 1 / world size, for the splat lookup.                   offset 120
    pub inv_world_size: [f32; 2],
    /// Bindless index of the layer 0-3 weights.                offset 128
    pub splat_map: i32,
    /// Bindless index of the layer 4-7 weights (Phase 25L).    offset 132
    pub splat_map_hi: i32,
    /// Layer index used for triplanar cliff projection (rock = 2).
    pub cliff_layer: u32,
    /// Phase 25F: non-zero applies stochastic hex-tiling to the layer maps.
    pub hex_tiling: u32,
}

/// Bindless indices of one terrain's textures, filled in at creation.
#[derive(Clone, Copy, Debug)]
pub struct TerrainTextureIds {
    pub splat_map: i32,
    pub splat_map_hi: i32,
    pub albedo: [i32; TERRAIN_LAYER_COUNT as usize],
    pub surface: [i32; TERRAIN_LAYER_COUNT as usize],
}

impl Default for TerrainTextureIds {
    fn default() -> Self {
        Self {
            splat_map: -1,
            splat_map_hi: -1,
            albedo: [-1; TERRAIN_LAYER_COUNT as usize],
            surface: [-1; TERRAIN_LAYER_COUNT as usize],
        }
    }
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

    /// Shared LOD index spans in the global index pool, keyed by
    /// `(lod, edge_mask)` and holding `(index_offset, index_count)`.
    ///
    /// One span serves every chunk at that LOD and stitch mask, because the
    /// index data is chunk-*relative*: the visibility pass reads
    /// `vertices[vertex_offset + indices[index_offset + i]]`, so the chunk's own
    /// offset supplies the difference. Allocated on first use and never
    /// rewritten — at most 5 LODs × 16 masks exist.
    index_blocks: HashMap<(u8, u8), (u32, u32)>,

    /// Vertices reserved per chunk — `(chunk_cells + 1)²`, fixed for life.
    chunk_vertex_capacity: u32,

    /// Bindless indices of the splatmap and the layer maps.
    pub texture_ids: TerrainTextureIds,
    /// This terrain's entry in the `MaterialPool`, carried by every chunk draw.
    pub material_id: u32,
    /// This terrain's slot in the terrain-material storage buffer.
    pub terrain_index: u32,

    /// Phase 25F: break the layer maps' visible repetition by hex-tiling them.
    ///
    /// **On by default since Phase 25K**, which is what made the judgement
    /// possible. Against the old procedural layers there was no repetition to
    /// remove and the technique only showed its own lattice, so it shipped
    /// switched off. With photographed layers the tiling grid is immediately
    /// visible as bands marching to the horizon, and hex-tiling removes them —
    /// the same code, the same parameters, opposite verdict, decided by the
    /// content. `SOMNIUM_HEXTILE=0` turns it off.
    pub hex_tiling: bool,

    /// Model matrix submitted for the current frame.
    pub model: glam::Mat4,
    /// Brush cursor uniform state (set by the editor each frame).
    pub brush_cursor: [f32; 4],
    /// Phase 17F: foliage painted onto this terrain by hand.
    ///
    /// Lives with the terrain rather than in the ECS for the same reason the
    /// heightmap does: there are thousands of instances, they are edited by
    /// brush strokes, and every one of them appearing in the outliner would
    /// make the editor unusable.
    pub painted_foliage: Vec<foliage_paint::PaintedFoliage>,

    /// Counter bumped on every sculpt or paint edit (Phase 17A).
    ///
    /// Foliage placement depends on the heightmap and the splatmap, so it has
    /// to re-scatter when either changes — but re-scattering every frame would
    /// mean a full pass over the terrain per frame. Comparing a counter is the
    /// cheap way to tell "nothing has changed" from "everything has".
    pub edit_revision: u64,
}

impl TerrainData {
    /// Create a flat terrain with the default grass/dirt/rock/snow layers.
    ///
    /// Chunks come back with no pool space; the renderer calls
    /// [`TerrainData::reserve_pool_spans`] once it can lend the geometry pool.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, desc: TerrainDescriptor) -> Self {
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
                chunks.push(TerrainChunk {
                    grid_pos: [cx, cz],
                    aabb_min: glam::Vec3::ZERO,
                    aabb_max: glam::Vec3::ZERO,
                    vertex_offset: UNALLOCATED,
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
        let layer_textures = TerrainLayerTextures::load_or_generate(device, queue);

        Self {
            desc,
            heightmap,
            chunks,
            layers,
            splatmap,
            layer_textures,
            index_blocks: HashMap::new(),
            chunk_vertex_capacity: verts_per_chunk,
            texture_ids: TerrainTextureIds::default(),
            material_id: 0,
            terrain_index: 0,
            hex_tiling: std::env::var("SOMNIUM_HEXTILE").as_deref() != Ok("0"),
            model: glam::Mat4::IDENTITY,
            brush_cursor: [0.0; 4],
            painted_foliage: Vec::new(),
            edit_revision: 0,
        }
    }

    /// Reserve one rewritable vertex span per chunk in the global pool.
    ///
    /// Called once, at creation. A chunk that cannot be placed keeps
    /// [`UNALLOCATED`] and is skipped when draws are built, so a full pool
    /// costs part of the terrain rather than corrupting the scene.
    pub fn reserve_pool_spans(&mut self, pool: &mut crate::geometry::GeometryPool) {
        let capacity = self.chunk_vertex_capacity;
        for chunk in &mut self.chunks {
            chunk.vertex_offset = pool.reserve_vertices(capacity).unwrap_or(UNALLOCATED);
        }
    }

    /// The GPU-side material for this terrain, rebuilt each frame.
    ///
    /// Cheap enough to rebuild rather than track dirty: the brush cursor moves
    /// every frame the editor is in terrain mode, and the model matrix arrives
    /// with the draw submission.
    pub fn gpu_material(&self) -> GpuTerrainMaterial {
        let [wx, wz] = self.desc.world_size();
        let origin = self.model.w_axis;
        GpuTerrainMaterial {
            layer_tiling: std::array::from_fn(|i| {
                self.layers.get(i).map_or(0.25, |l| l.tiling)
            }),
            brush: self.brush_cursor,
            albedo_maps: self.texture_ids.albedo,
            surface_maps: self.texture_ids.surface,
            terrain_origin: [origin.x, origin.z],
            inv_world_size: [1.0 / wx, 1.0 / wz],
            splat_map: self.texture_ids.splat_map,
            splat_map_hi: self.texture_ids.splat_map_hi,
            cliff_layer: 2,
            hex_tiling: u32::from(self.hex_tiling),
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
        self.edit_revision = self.edit_revision.wrapping_add(1);
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

    /// Replace the whole heightmap from a file (Phase 25L).
    ///
    /// `amplitude` is the metres of relief the source's full `0..=1` range maps
    /// to, before `height_scale`.
    pub fn load_heightmap_file(&mut self, path: &str, amplitude: f32) -> Result<(), String> {
        let image = heightmap::load(path)?;
        let (tx, tz) = (self.desc.total_vertices_x(), self.desc.total_vertices_z());
        self.heightmap = image.resample(tx, tz, amplitude);
        self.mark_all_dirty();
        tracing::info!(
            "terrain: heightmap {path} ({}x{}) resampled to {tx}x{tz}, {amplitude} m of relief",
            image.width,
            image.height,
        );
        Ok(())
    }

    /// Fill the heightmap with procedural FBM relief (Phase 25L).
    pub fn generate_relief(&mut self, seed: u32, amplitude: f32) {
        let (tx, tz) = (self.desc.total_vertices_x(), self.desc.total_vertices_z());
        self.heightmap = heightmap::fbm_relief(tx, tz, seed, amplitude);
        self.mark_all_dirty();
    }

    /// Mark every chunk for rebuild, and bump the edit revision so foliage and
    /// the collider notice.
    pub fn mark_all_dirty(&mut self) {
        self.edit_revision = self.edit_revision.wrapping_add(1);
        for chunk in &mut self.chunks {
            chunk.dirty = true;
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

    /// Ground query for the foliage brush (Phase 17F).
    pub fn ground_sample(&self, local_x: f32, local_z: f32) -> foliage_paint::GroundSample {
        let s = self.surface_sample(local_x, local_z, 0);
        foliage_paint::GroundSample { height: s.height, slope_cos: s.slope_cos }
    }

    /// Sample height, slope and paint-layer weight at a terrain-local point
    /// (Phase 17A). This is what foliage scattering places against.
    pub fn surface_sample(&self, local_x: f32, local_z: f32, layer: u8) -> foliage::SurfaceSample {
        let height = self.world_height_at(local_x, local_z);

        // Central difference over one cell. The gradient is in world units on
        // both axes because `world_height_at` already applies `height_scale`,
        // so the normal comes straight out of it.
        let d = self.desc.cell_size;
        let hx = self.world_height_at(local_x + d, local_z) - self.world_height_at(local_x - d, local_z);
        let hz = self.world_height_at(local_x, local_z + d) - self.world_height_at(local_x, local_z - d);
        let normal = glam::Vec3::new(-hx, 2.0 * d, -hz).normalize_or_zero();
        // Y of the unit normal IS the cosine of the slope from vertical.
        let slope_cos = if normal == glam::Vec3::ZERO { 1.0 } else { normal.y.abs() };

        let layer_weight = self.layer_weight_at(local_x, local_z, layer);
        foliage::SurfaceSample { height, slope_cos, layer_weight }
    }

    /// Splatmap weight of `layer` at a terrain-local point, `0..=1`.
    ///
    /// Nearest-texel rather than bilinear: the splatmap is far coarser than the
    /// scatter grid, and a hard edge is what a painted boundary should look
    /// like — foliage stopping exactly where the paint stops.
    pub fn layer_weight_at(&self, local_x: f32, local_z: f32, layer: u8) -> f32 {
        let [wx, wz] = self.desc.world_size();
        if wx <= 0.0 || wz <= 0.0 || u32::from(layer) >= TERRAIN_LAYER_COUNT {
            return 0.0;
        }
        let sm = &self.splatmap;
        let u = (local_x / wx).clamp(0.0, 1.0);
        let v = (local_z / wz).clamp(0.0, 1.0);
        let tx = ((u * sm.width as f32) as u32).min(sm.width.saturating_sub(1));
        let tz = ((v * sm.height as f32) as u32).min(sm.height.saturating_sub(1));
        let idx = (tz * sm.width + tx) as usize;
        match sm.data.get(idx) {
            Some(texel) => texel[layer as usize] as f32 / 255.0,
            None => 0.0,
        }
    }

    /// Resample this terrain into a Jolt heightfield (Phase 17B).
    ///
    /// Returns the samples and the world spacing between them.
    pub fn heightfield(&self) -> (Vec<f32>, u32, glam::Vec3) {
        let n = collider::sample_count_for(self.desc.total_vertices_x().min(self.desc.total_vertices_z()));
        let world = self.desc.world_size();
        let samples = collider::resample(n, world, |x, z| self.world_height_at(x, z));
        (samples, n, collider::heightfield_scale(world, n))
    }

    /// Scatter foliage over this terrain (Phase 17A).
    pub fn scatter_foliage(&self, params: &foliage::FoliageParams) -> Vec<foliage::FoliageInstance> {
        foliage::scatter(params, self.desc.world_size(), |x, z| {
            self.surface_sample(x, z, params.layer)
        })
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

    /// Regenerate vertices for dirty chunks and rewrite their pool spans
    /// (Phase 14B-1, moved into the global pool by 25A-2).
    /// `rewritten` collects the pool vertex offset of every chunk this call
    /// touched, so the caller can rebuild whatever else depends on the
    /// geometry — in Phase 25B, the chunk's bottom-level acceleration
    /// structure, which is otherwise built once and never notices a sculpt.
    pub fn rebuild_dirty_chunks(
        &mut self,
        queue: &wgpu::Queue,
        pool: &mut crate::geometry::GeometryPool,
        rewritten: &mut Vec<u32>,
    ) {
        let desc = self.desc;
        for chunk in &mut self.chunks {
            if !chunk.dirty || chunk.vertex_offset == UNALLOCATED {
                continue;
            }
            rewritten.push(chunk.vertex_offset);
            let vertices = mesh::build_chunk_vertices(
                &self.heightmap,
                desc.total_vertices_x(),
                desc.total_vertices_z(),
                desc.chunk_cells,
                chunk.grid_pos,
                desc.cell_size,
                desc.height_scale,
            );
            pool.write_vertices(queue, chunk.vertex_offset, &vertices);

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

    /// Reserve and fill the shared index span for every `(lod, mask)` pair the
    /// current LOD assignment needs. Call before draws are built.
    pub fn ensure_index_blocks(
        &mut self,
        queue: &wgpu::Queue,
        pool: &mut crate::geometry::GeometryPool,
    ) {
        let cells = self.desc.chunk_cells;
        // `(0, 0)` unconditionally: it is the geometry the ray-tracing BLAS is
        // built from (Phase 25B), and it must exist whether or not any chunk
        // happens to be drawing at full detail with no stitching this frame.
        // A BLAS is sized once at creation, so its index range cannot follow
        // the per-frame LOD — and it should not: traced shadows that popped
        // with LOD would be worse than ones that are slightly too detailed.
        let keys: Vec<(u8, u8)> = std::iter::once((0u8, 0u8))
            .chain(self.chunks.iter().map(|c| (c.lod, c.edge_mask)))
            .collect();
        for key in keys {
            if self.index_blocks.contains_key(&key) {
                continue;
            }
            let indices = mesh::build_lod_indices(cells, key.0, key.1);
            let Some(offset) = pool.reserve_indices(indices.len() as u32) else {
                continue; // pool full: this LOD/mask draws nothing this frame
            };
            pool.write_indices(queue, offset, &indices);
            self.index_blocks.insert(key, (offset, indices.len() as u32));
        }
    }

    /// Read-only lookup of the `(index_offset, index_count)` for a `(lod, mask)`.
    pub fn index_block(&self, lod: u8, mask: u8) -> Option<(u32, u32)> {
        self.index_blocks.get(&(lod, mask)).copied()
    }

    /// The geometry every chunk's ray-tracing BLAS is built from (Phase 25B):
    /// full detail, no edge stitching, identical for every chunk.
    pub fn rt_index_block(&self) -> Option<(u32, u32)> {
        self.index_block(0, 0)
    }

    /// Vertices in one chunk's grid — what a chunk BLAS is sized for.
    pub fn chunk_vertex_capacity(&self) -> u32 {
        self.chunk_vertex_capacity
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
        // Version 2: Phase 25L widened a splat texel from 4 weights to
        // TERRAIN_LAYER_COUNT. A v1 sidecar's splat block is a different size,
        // so it is refused rather than read as if it matched.
        out.extend(2u32.to_le_bytes()); // version
        out.extend(self.desc.total_vertices_x().to_le_bytes());
        out.extend(self.desc.total_vertices_z().to_le_bytes());
        out.extend(self.splatmap.width.to_le_bytes());
        out.extend(self.splatmap.height.to_le_bytes());
        out.extend(bytemuck::cast_slice::<f32, u8>(&self.heightmap));
        out.extend(bytemuck::cast_slice::<[u8; TERRAIN_LAYER_COUNT as usize], u8>(
            &self.splatmap.data,
        ));
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
        let version = u32_at(4)?;
        if version != 2 {
            return Err(format!(
                "terrain sidecar is version {version}; this build writes 2                  (Phase 25L widened splat texels from 4 layers to {})",
                TERRAIN_LAYER_COUNT,
            ));
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
        let s_bytes = (sw * sh) as usize * TERRAIN_LAYER_COUNT as usize;
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
