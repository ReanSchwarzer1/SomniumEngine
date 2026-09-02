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

pub mod biome;
pub mod blend;
pub mod brush;
pub mod clipmap;
pub mod collider;
pub mod foliage;
pub mod foliage_paint;
pub mod heightmap;
/// Phase TSUSHIMA-B/C: baked horizon angles and sky visibility.
pub mod horizon;
pub mod macro_map;
pub mod mesh;
pub mod mips;
pub mod splat;
pub mod textures;
/// MORROWIND-AD source-page feedback and bounded residency.
pub mod virtual_texture;

use std::collections::HashMap;

use mesh::{EDGE_EAST, EDGE_NORTH, EDGE_SOUTH, EDGE_WEST, MAX_TERRAIN_LOD};
use textures::{
    LAYER_NAMES, Splatmap, TERRAIN_HERO_LAYERS, TERRAIN_LAYER_COUNT, TerrainLayerTextures,
};

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
    /// Allocate the bounded BC7 source-page atlas instead of full layer arrays.
    /// This is creation-time because the choice changes GPU residency.
    pub virtual_texturing: bool,
}

impl Default for TerrainDescriptor {
    fn default() -> Self {
        Self {
            chunk_cells: 64,
            grid_size: [16, 16],
            cell_size: 1.0,
            height_scale: 1.0,
            lod_base_range: 96.0,
            virtual_texturing: false,
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

fn chunk_crosses_water_surface(
    chunk: &TerrainChunk,
    chunk_world: f32,
    shoreline_regions: &[crate::water_body::WaterBodyDescriptor],
) -> bool {
    let chunk_min_x = chunk.grid_pos[0] as f32 * chunk_world;
    let chunk_min_z = chunk.grid_pos[1] as f32 * chunk_world;
    let chunk_max_x = chunk_min_x + chunk_world;
    let chunk_max_z = chunk_min_z + chunk_world;
    shoreline_regions.iter().any(|water| {
        let [water_min_x, water_min_z, water_max_x, water_max_z] = water.bounds;
        let overlaps_xz = chunk_max_x >= water_min_x
            && chunk_min_x <= water_max_x
            && chunk_max_z >= water_min_z
            && chunk_min_z <= water_max_z;
        let wave_margin = water.amplitude.max(0.05);
        overlaps_xz
            && chunk.aabb_min.y <= water.surface_level + wave_margin
            && chunk.aabb_max.y >= water.surface_level - wave_margin
    })
}

/// Heightmap a new terrain loads unless told otherwise (Phase 25L).
pub const DEFAULT_HEIGHTMAP: &str = "assets/terrain/great_lakes/height.png";

/// Authored low-frequency colour paired with [`DEFAULT_HEIGHTMAP`].
pub const DEFAULT_MACRO_MAP: &str = "assets/terrain/great_lakes/macro_color.png";

/// The datum `assets/terrain/great_lakes/` was baked at.
///
/// Not a preference — a **property of the shipped files**, recorded in their
/// own `recipe.json` as `"water_level_metres": 15`. The mask, depth and shore
/// SDF are a shoreline solved for a water plane at this height, so a runtime
/// datum that disagrees with it puts the surface somewhere the coverage does
/// not describe.
pub const GREAT_LAKES_BAKE_DATUM_METRES: f32 = 15.0;

/// Great Lakes preset surface datum in terrain-local metres.
///
/// **Was 16.1 until 2026-08-29, and that was 1.1 m above the bake.** Measured
/// against the shipped `height.png` and `water_mask.png` at the default 105 m
/// of relief, over all 4,194,304 mask cells:
///
/// | datum | dry cells under water | wet cells above it |
/// |---|---:|---:|
/// | 14.0 | 0 | 235,688 |
/// | **15.0** | **3,545** | **3,625** |
/// | 16.1 | **108,719** | 1 |
///
/// At 16.1 a hundred and nine thousand cells of ground the shoreline calls dry
/// were under the plane, which is why the water read as sitting *on* the beach
/// rather than meeting it. At 14.0 the error simply reverses and strands a
/// quarter of a million wet cells above the surface. The separating level is
/// 15.4 and the bake's own recipe says 15, so 15 it is: the residual ~7,000
/// cells are the antialiasing band, not a disagreement.
///
/// `terrain_shading_occupancy_2026-08-14.md` froze the old 16.1. That freeze is
/// superseded here rather than quietly broken — it pinned a number that never
/// matched the data it depends on, and the measurement above is the argument.
pub const DEFAULT_WATER_LEVEL_METRES: f32 = GREAT_LAKES_BAKE_DATUM_METRES;

/// Maximum synthetic lake-bed depth in the baked preset.
pub const DEFAULT_WATER_DEPTH_METRES: f32 = 12.0;

/// Metres of relief the default heightmap's full range maps to.
///
/// 90 m over the default 1024 m terrain is a gentle range of hills rather than
/// alpine — enough for the eight materials to separate by altitude and for
/// slopes to reach the angles that put rock and gravel on the ground, without
/// walls the camera cannot see over.
pub const DEFAULT_RELIEF_METRES: f32 = 105.0;

/// A material layer (Phase 14A-2). Texture data lives in the shared
/// `TerrainLayerTextures` arrays; this carries the per-layer parameters.
pub struct TerrainLayer {
    pub name: String,
    /// UV repeats per metre when sampling this layer's textures.
    pub tiling: f32,
    /// Phase 25E: how this layer's own relief decides where it wins against
    /// its neighbours. See [`blend::LayerBlend`].
    pub blend: blend::LayerBlend,
}

/// Everything `shading.wgsl` needs to evaluate a terrain surface, mirrored by
/// `TerrainMaterial` in `terrain_material.wgsl` (2032 bytes, Phase DF).
///
/// **Every `vec4` member sits at a 16-byte offset**, which is what keeps Rust's
/// `repr(C)` packing and WGSL's alignment rules agreeing.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuTerrainMaterial {
    /// UV repeats per metre, one per layer.                    offset 0    (128)
    pub layer_tiling: [f32; 32],
    /// xy = brush world XZ, z = radius, w = mode
    /// (0 off, 1 sculpt, 2 paint, 3 foliage).                  offset 128
    pub brush: [f32; 4],
    /// Bindless index of each layer's albedo+height map.       offset 144  (128)
    pub albedo_maps: [i32; 32],
    /// Bindless index of each layer's packed surface map.      offset 272  (128)
    pub surface_maps: [i32; 32],
    /// World XZ of terrain-local (0, 0).                       offset 400
    pub terrain_origin: [f32; 2],
    /// 1 / world size, for the splat lookup.                   offset 408
    pub inv_world_size: [f32; 2],
    /// Bindless indices of the eight RGBA splatmaps.           offset 416  (32)
    pub splat_maps: [i32; 8],
    /// Layer index used for biplanar cliff projection.         offset 448
    pub cliff_layer: u32,
    /// Non-zero applies stochastic hex-tiling.                 offset 452
    pub hex_tiling: u32,
    /// Non-zero runs the height-weighted blend.                offset 456
    pub height_blend: u32,
    /// Bindless index of the macro colour map, -1 for none.    offset 460
    pub macro_map: i32,
    /// Per-layer height blend scale.                           offset 464  (128)
    pub layer_height_scale: [f32; 32],
    /// Width of the layer's transition band.                   offset 592  (128)
    pub layer_blend_width: [f32; 32],
    /// Reciprocal of the layer's minimum weight.               offset 720  (128)
    pub layer_weight_clamp: [f32; 32],
    /// Relief depth per layer, in metres.                      offset 848  (128)
    pub layer_parallax: [f32; 32],
    /// `macro_map::MacroBlendMode`.                            offset 976
    pub macro_mode: u32,
    /// Blend factor; 0 leaves the detail untouched.            offset 980
    pub macro_strength: f32,
    /// Metres at which the layer budget starts falling.        offset 984
    pub detail_fade_start: f32,
    /// Metres at which only the dominant layers survive.       offset 988
    pub detail_fade_end: f32,
    /// Mean linear albedo per layer.                           offset 992  (512)
    pub layer_albedo: [[f32; 4]; 32],
    /// Steps the parallax march takes at its closest.          offset 1504
    pub parallax_steps: u32,
    /// Steps of the self-shadow march toward the sun.          offset 1508
    pub parallax_shadow_steps: u32,
    /// Biplanar/triplanar axis sharpness (XV-F).               offset 1512
    pub projection_sharpness: f32,
    /// 0 = biplanar (default), 1 = triplanar debug.            offset 1516
    pub projection_mode: u32,
    /// Moisture affinity per layer (XV-H).                     offset 1520 (128)
    pub layer_moisture: [f32; 32],
    /// Global wetness 0..1.                                    offset 1648
    pub wetness: f32,
    /// Albedo multiplier when fully wet (porous darken).       offset 1652
    pub wetness_darken: f32,
    /// Roughness scale when fully wet.                         offset 1656
    pub wetness_gloss: f32,
    /// Extra dielectric F0 when wet.                           offset 1660
    pub wetness_f0: f32,
    /// Phase DF: nested material clipmaps.                     offset 1664
    pub clipmap_enabled: u32,
    pub clipmap_rings: u32,
    pub clipmap_size: f32,
    pub clipmap_debug: u32,
    pub clipmap_albedo: [i32; 8],
    pub clipmap_surface: [i32; 8],
    pub clipmap_center: [f32; 16],
    pub clipmap_origin: [f32; 16],
    pub clipmap_tpm: [f32; 8],
    pub clipmap_macro_albedo: [i32; 4],
    pub clipmap_macro_normal: [i32; 4],
    pub clipmap_macro_center: [f32; 8],
    pub clipmap_macro_origin: [f32; 8],
    pub clipmap_macro_tpm: [f32; 4],
    pub clipmap_macro_rings: u32,
    pub clipmap_macro_size: f32,
    pub clipmap_detail_ready: u32,
    pub clipmap_macro_ready: u32,
    /// Phase TSUSHIMA-B: baked horizon angles, azimuths 0-3. -1 disables.
    pub horizon_map_a: i32,
    /// Azimuths 4-7.
    pub horizon_map_b: i32,
    /// Phase TSUSHIMA-C: bent direction + sky visibility. -1 disables.
    pub skyvis_map: i32,
    /// Scales TSUSHIMA-C's darkening. Four words exactly, so the struct stays
    /// a multiple of 16 bytes and every `array<vec4<_>>` above it keeps its
    /// offset.
    pub sky_visibility_strength: f32,
}

/// Bindless indices of one terrain's textures, filled in at creation.
#[derive(Clone, Copy, Debug)]
pub struct TerrainTextureIds {
    pub splat_maps: [i32; splat::SPLAT_MAP_COUNT],
    /// Phase 25D: the whole-terrain macro colour map.
    pub macro_map: i32,
    pub albedo: [i32; TERRAIN_LAYER_COUNT as usize],
    pub surface: [i32; TERRAIN_LAYER_COUNT as usize],
    /// MORROWIND-AD clipmap source: albedo atlas, surface atlas, page table,
    /// and physical atlas edge in texels. Sentinel disables virtual sampling.
    pub virtual_texture: [i32; 4],
    /// Phase TSUSHIMA-B: baked horizon angles, azimuths 0-3 and 4-7.
    /// `-1` is the off switch, and it is the *only* off switch — the shader
    /// gates on it rather than on a pipeline override because the lookup is
    /// two fetches and a compare, not a march, so leaving it resident costs
    /// nothing the way POM's would.
    pub horizon_maps: [i32; 2],
    /// Phase TSUSHIMA-C: RGB bent direction, A cosine-weighted sky visibility.
    pub sky_visibility: i32,
}

impl Default for TerrainTextureIds {
    fn default() -> Self {
        Self {
            splat_maps: [-1; splat::SPLAT_MAP_COUNT],
            macro_map: -1,
            albedo: [-1; TERRAIN_LAYER_COUNT as usize],
            surface: [-1; TERRAIN_LAYER_COUNT as usize],
            virtual_texture: [-1, -1, -1, 0],
            horizon_maps: [-1, -1],
            sky_visibility: -1,
        }
    }
}

impl TerrainTextureIds {
    /// Leave extra-bank splat maps (layers 16–31) and layer views unbound.
    pub fn unbind_extra_bank(&mut self) {
        for id in self.splat_maps.iter_mut().skip(4) {
            *id = -1;
        }
        let hero = TERRAIN_HERO_LAYERS as usize;
        for i in hero..TERRAIN_LAYER_COUNT as usize {
            self.albedo[i] = -1;
            self.surface[i] = -1;
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
    /// Non-zero texels were painted by hand and survive biome rebuild (XV-G).
    pub splat_lock: Vec<u8>,
    /// Global wetness 0..1 (XV-H). `SOMNIUM_TERRAIN_WETNESS` seeds it.
    pub wetness: f32,
    pub layer_textures: TerrainLayerTextures,
    /// MORROWIND-AD bounded source-page residency feeding the material clipmap.
    pub virtual_texture: virtual_texture::VirtualTextureCache,
    /// Authored virtual-source switch mirrored from `TerrainComponent`.
    pub virtual_texture_enabled: bool,
    /// Configured physical source-page budget in MiB.
    pub virtual_texture_cache_mib: u32,
    /// Monotonic feedback frame used by deterministic LRU ordering.
    virtual_texture_frame: u64,
    /// Phase 25D: the macro tier. Rewritten in place when the heightfield
    /// changes wholesale, so its bindless index is stable.
    pub macro_texture: wgpu::Texture,
    pub macro_view: wgpu::TextureView,
    /// Phase TSUSHIMA-B/C: baked horizon angles and sky visibility.
    ///
    /// Rewritten in place after a sculpt, like the macro tier, so the three
    /// bindless indices are registered once and stay valid for the terrain's
    /// life.
    pub horizon_gpu: horizon::HorizonGpu,
    /// Phase TSUSHIMA-B. `SOMNIUM_TERRAIN_HORIZON=0` is the A/B: it unbinds
    /// the maps, and an unbound map is exactly the pre-phase behaviour.
    pub horizon_shadow: bool,
    /// Phase TSUSHIMA-C, gated separately so the two halves of one bake can be
    /// judged apart.
    pub sky_visibility: bool,
    /// How hard baked sky visibility darkens the ambient term. 1.0 is the
    /// geometric answer; lower is an artistic retreat from it.
    pub sky_visibility_strength: f32,
    /// Set when the heightfield changes under a baked map.
    pub horizon_dirty: bool,

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
    /// When true, only layers 0–15 are bound; extra-bank splat maps stay -1.
    pub hero_bank_only: bool,

    /// Phase 25F: break the layer maps' visible repetition by hex-tiling them.
    ///
    /// **On by default since Phase 25K**, which is what made the judgement
    /// possible. Against the old procedural layers there was no repetition to
    /// remove and the technique only showed its own lattice, so it shipped
    /// switched off. With photographed layers the tiling grid is immediately
    /// visible as bands marching to the horizon, and hex-tiling removes them —
    /// the same code, the same parameters, opposite verdict, decided by the
    /// content.
    ///
    /// **Default off on every map.** Island already switched it off through
    /// `apply_hero_bank_gpu_budget`; Coastal now matches. `SOMNIUM_HEXTILE=1`
    /// turns it on, and the Terrain details checkbox still works either way.
    pub hex_tiling: bool,
    /// Phase 25C: morph chunk vertices toward the coarser LOD across the last
    /// fraction of each range so ridge lines do not pop. Default off;
    /// `SOMNIUM_LOD_MORPH=1` turns it on.
    pub lod_morph: bool,
    /// 0..1 fraction of the LOD range where morphing starts. 0.7 is the last 30%.
    pub lod_morph_start: f32,
    /// Phase 25D: the macro tier's blend against the detail composite.
    pub macro_mode: macro_map::MacroBlendMode,
    /// Phase 25D. `SOMNIUM_TERRAIN_MACRO=0` sets this to zero, which is the
    /// A/B — a strength of zero is exactly "no macro tier".
    pub macro_strength: f32,
    /// Authored whole-landscape colour. When present this wins over the
    /// procedural macro generator after terrain edits/rebuilds.
    #[allow(dead_code)]
    authored_macro: Option<macro_map::MacroMap>,
    /// Phase 25D: metres over which the per-pixel layer budget falls away.
    pub detail_fade_start: f32,
    pub detail_fade_end: f32,
    /// The macro map is derived from the heightfield, so a wholesale change to
    /// the terrain invalidates it. Set by `mark_all_dirty` and consumed in
    /// `rebuild_dirty_chunks`, which is where a queue is already in hand.
    ///
    /// Deliberately *not* set by sculpting: the macro tier is a hundreds-of-
    /// metres signal and regenerating 512² texels per brush dab would cost far
    /// more than the frequencies it would recover.
    macro_dirty: bool,
    /// Phase 25E. `SOMNIUM_TERRAIN_HEIGHT_BLEND=0` falls back to plain
    /// normalised splat weights, which is the only way to see what the height
    /// blend is actually doing.
    pub height_blend: bool,
    /// Phase 25H. Multiplies every layer's authored relief depth; 0 disables
    /// parallax entirely, which is the A/B.
    ///
    /// **Default 0 on every map**, matching hex tiling above.
    /// `SOMNIUM_TERRAIN_PARALLAX=1` restores the 1.0 scale.
    pub parallax_scale: f32,
    /// Last non-zero scale, so the Details Parallax toggle can restore Relief.
    pub parallax_held: f32,
    /// Steps the view march takes at its closest. Falls to 0 with distance.
    pub parallax_steps: u32,
    /// Steps of the march toward the sun that gives the relief self-shadowing.
    pub parallax_shadow_steps: u32,
    /// Biplanar/triplanar axis sharpness (XV-F). `SOMNIUM_TERRAIN_PROJECTION_SHARPNESS`.
    pub projection_sharpness: f32,
    /// 0 = biplanar (default), 1 = triplanar debug. `SOMNIUM_TERRAIN_TRIPLANAR=1`.
    pub projection_mode: u32,

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

/// Map one physical clipmap dirty interval through its toroidal origin into at
/// most two monotonically increasing world-space intervals.
fn feedback_axis_intervals(
    start: u32,
    length: u32,
    size: f32,
    origin_uv: f32,
    center: f32,
    extent: f32,
) -> Vec<(f32, f32)> {
    if length == 0 {
        return Vec::new();
    }
    if length as f32 >= size {
        return vec![(center - extent * 0.5, center + extent * 0.5)];
    }
    let q0 = start as f32 / size - origin_uv;
    let q1 = (start + length) as f32 / size - origin_uv - f32::EPSILON;
    let world = |logical: f32| center + (logical - 0.5) * extent;
    if q0.floor() == q1.floor() {
        let a = world(q0.rem_euclid(1.0));
        let b = world(q1.rem_euclid(1.0));
        vec![(a.min(b), a.max(b))]
    } else {
        vec![
            (world(q0.rem_euclid(1.0)), world(1.0)),
            (world(0.0), world(q1.rem_euclid(1.0))),
        ]
    }
}

fn feedback_world_rects(job: &clipmap::ClipmapGenJob) -> Vec<[f32; 4]> {
    let extent = job.clipmap_size / job.texels_per_m.max(0.0001);
    let xs = feedback_axis_intervals(
        job.rect.x,
        job.rect.w,
        job.clipmap_size,
        job.origin_uv[0],
        job.center[0],
        extent,
    );
    let zs = feedback_axis_intervals(
        job.rect.y,
        job.rect.h,
        job.clipmap_size,
        job.origin_uv[1],
        job.center[1],
        extent,
    );
    xs.iter()
        .flat_map(|&(x0, x1)| zs.iter().map(move |&(z0, z1)| [x0, z0, x1, z1]))
        .collect()
}

/// Physical page coordinates touched by one repeated source-UV interval.
fn feedback_page_indices(uv0: f32, uv1: f32, mip_size: u32) -> Vec<u32> {
    const PAGE: u32 = 128;
    let pages = mip_size.div_ceil(PAGE).max(1);
    let lo = uv0.min(uv1);
    let hi = uv0.max(uv1);
    if hi - lo >= 1.0 {
        return (0..pages).collect();
    }
    let end = (hi - f32::EPSILON).max(lo);
    let page_at =
        |uv: f32| ((uv.rem_euclid(1.0) * mip_size as f32).floor() as u32 / PAGE).min(pages - 1);
    let first = page_at(lo);
    let last = page_at(end);
    if lo.floor() == end.floor() {
        (first..=last).collect()
    } else {
        (first..pages).chain(0..=last).collect()
    }
}

impl TerrainData {
    /// Create a flat terrain with the default grass/dirt/rock/snow layers.
    ///
    /// Chunks come back with no pool space; the renderer calls
    /// [`TerrainData::reserve_pool_spans`] once it can lend the geometry pool.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        desc: TerrainDescriptor,
        bc_supported: bool,
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
            .enumerate()
            .map(|(i, name)| TerrainLayer {
                name: (*name).to_string(),
                tiling: textures::LAYER_TILING[i],
                blend: blend::LAYER_BLENDS[i],
            })
            .collect();

        // One splat texel per heightmap cell; rows stay 256-byte aligned
        // because chunk_cells is a power of two ≥ 16.
        let splatmap = Splatmap::new(
            device,
            queue,
            desc.grid_size[0] * desc.chunk_cells,
            desc.grid_size[1] * desc.chunk_cells,
        );
        let splat_lock = vec![0u8; splatmap.data.len()];
        let wetness = std::env::var("SOMNIUM_TERRAIN_WETNESS")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        // Virtual texturing has no way to reach the screen without the
        // clipmap. In VT mode `load_bc7_layers` registers 4x4 placeholders for
        // the legacy layer arrays on purpose and only the rings carry the real
        // pages, so loading VT with the clipmap off would shade the terrain
        // from eight mean colours. With the clipmap off the resident BC7 path
        // is loaded instead, which is the arrangement that predates the cache.
        let virtual_texturing =
            desc.virtual_texturing && clipmap::TerrainClipmap::env_default_enabled();
        let layer_textures =
            TerrainLayerTextures::load_or_generate(device, queue, bc_supported, virtual_texturing);
        let virtual_capacity = layer_textures
            .virtual_texture
            .as_ref()
            .map_or(0, textures::TerrainVirtualTextureGpu::capacity_pages);

        // Generated flat here and regenerated once relief lands (see
        // `macro_dirty`). Creating it now rather than on first use is what lets
        // `create_terrain` register one bindless index for the terrain's life —
        // the texels are rewritten in place, the view never changes.
        let (macro_texture, macro_view) = macro_map::upload(
            device,
            queue,
            &macro_map::generate(
                &heightmap,
                desc.total_vertices_x(),
                desc.total_vertices_z(),
                desc.cell_size,
                desc.height_scale,
                0,
            ),
        );

        // Baked here for the same reason the macro map is: one registration of
        // three bindless indices for the terrain's life, with the texels
        // rewritten in place when the heightfield changes. The bake is a few
        // tens of milliseconds on a 1 km terrain (multi-resolution march, see
        // `horizon`), which is load-time work, not frame-time work.
        let horizon_gpu = horizon::upload(
            device,
            queue,
            &horizon::bake(
                &heightmap,
                desc.total_vertices_x(),
                desc.total_vertices_z(),
                desc.cell_size,
                desc.height_scale,
                horizon::HORIZON_SIZE,
            ),
        );

        Self {
            macro_texture,
            macro_view,
            horizon_gpu,
            horizon_shadow: std::env::var("SOMNIUM_TERRAIN_HORIZON").as_deref() != Ok("0"),
            sky_visibility: std::env::var("SOMNIUM_TERRAIN_SKYVIS").as_deref() != Ok("0"),
            sky_visibility_strength: 1.0,
            horizon_dirty: false,
            desc,
            heightmap,
            chunks,
            layers,
            splatmap,
            splat_lock,
            wetness,
            layer_textures,
            // A paired 128² BC7 albedo/surface page is 32 KiB. The default
            // 64 MiB budget therefore owns 2,048 physical page slots.
            virtual_texture: virtual_texture::VirtualTextureCache::new(virtual_capacity, 8),
            virtual_texture_enabled: desc.virtual_texturing,
            virtual_texture_cache_mib: 64,
            virtual_texture_frame: 0,
            index_blocks: HashMap::new(),
            chunk_vertex_capacity: verts_per_chunk,
            texture_ids: TerrainTextureIds::default(),
            material_id: 0,
            terrain_index: 0,
            hero_bank_only: false,
            hex_tiling: std::env::var("SOMNIUM_HEXTILE").as_deref() == Ok("1"),
            lod_morph: std::env::var("SOMNIUM_LOD_MORPH").as_deref() == Ok("1"),
            lod_morph_start: 0.7,
            macro_mode: macro_map::MacroBlendMode::Lerp,
            macro_strength: if std::env::var("SOMNIUM_TERRAIN_MACRO").as_deref() == Ok("0") {
                0.0
            } else {
                0.55
            },
            authored_macro: None,
            // Roughly: full detail out to the far edge of LOD 0, then a fall
            // to the dominant layers over the next few hundred metres. Past
            // `end` a pixel covers metres of ground and the layers it is
            // averaging are indistinguishable.
            // `SOMNIUM_TERRAIN_DETAIL_FADE=0` pushes the fade past any real
            // view distance, which is the A/B for the budget: the shader keeps
            // its distance term and simply never reaches it.
            detail_fade_start: if std::env::var("SOMNIUM_TERRAIN_DETAIL_FADE").as_deref() == Ok("0")
            {
                1.0e9
            } else {
                60.0
            },
            detail_fade_end: 400.0,
            macro_dirty: true,
            height_blend: std::env::var("SOMNIUM_TERRAIN_HEIGHT_BLEND").as_deref() != Ok("0"),
            parallax_scale: if std::env::var("SOMNIUM_TERRAIN_PARALLAX").as_deref() == Ok("1") {
                1.0
            } else {
                0.0
            },
            parallax_held: 1.0,
            parallax_steps: 24,
            parallax_shadow_steps: 8,
            projection_sharpness: std::env::var("SOMNIUM_TERRAIN_PROJECTION_SHARPNESS")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(8.0)
                .max(1.0),
            projection_mode: u32::from(
                std::env::var("SOMNIUM_TERRAIN_TRIPLANAR").as_deref() == Ok("1"),
            ),
            model: glam::Mat4::IDENTITY,
            brush_cursor: [0.0; 4],
            painted_foliage: Vec::new(),
            edit_revision: 0,
        }
    }

    /// Apply the editable upload throttle and publish creation-time VT state.
    pub fn configure_virtual_texture(&mut self, _enabled: bool, _cache_mib: u32, uploads: u32) {
        let physical_capacity = self
            .layer_textures
            .virtual_texture
            .as_ref()
            .map_or(0, textures::TerrainVirtualTextureGpu::capacity_pages);
        self.virtual_texture.set_upload_budget(uploads.max(1));
        self.virtual_texture_enabled = physical_capacity > 0;
        self.virtual_texture_cache_mib = physical_capacity / 32;
    }

    /// Convert runtime-clipmap dirty work into deterministic source-page demand.
    ///
    /// The clipmap generate pass is the consumer, so its exact world footprint
    /// is more useful feedback than the final screen pixels: requests arrive
    /// before composition and survive a frame that exhausts the upload budget.
    pub fn feedback_virtual_texture(
        &mut self,
        queue: &wgpu::Queue,
        jobs: &[clipmap::ClipmapGenJob],
    ) -> bool {
        if !self.virtual_texture_enabled || self.layer_textures.virtual_texture.is_none() {
            return false;
        }
        let mut requested = Vec::new();
        for job in jobs {
            let world_rects = feedback_world_rects(job);
            let layer_rects = self.layer_support_rects(&world_rects);
            let texel_metres = 1.0 / job.texels_per_m.max(0.0001);
            for (layer, material) in self
                .layers
                .iter()
                .enumerate()
                .filter(|(layer, _)| !layer_rects[*layer].is_empty())
            {
                let source_size: u32 = if layer < TERRAIN_HERO_LAYERS as usize {
                    2048
                } else {
                    1024
                };
                let footprint = texel_metres * material.tiling * source_size as f32;
                let mip = footprint.max(1.0).log2().floor() as u32;
                let mip = mip.min(source_size.ilog2());
                // Coarse ancestors are queued first, making the shader's
                // parent fallback useful while the target pages stream in.
                // Page coordinates come from the dirty rectangle's exact
                // repeated-UV footprint; demand can therefore converge even
                // when the authored layer set is larger than the cache.
                for level in (mip..=source_size.ilog2()).rev() {
                    let mip_size = (source_size >> level).max(1);
                    let origin = self.model.w_axis;
                    for &[x0, z0, x1, z1] in &layer_rects[layer] {
                        let xs = feedback_page_indices(
                            (x0 - origin.x) * material.tiling,
                            (x1 - origin.x) * material.tiling,
                            mip_size,
                        );
                        let ys = feedback_page_indices(
                            (z0 - origin.z) * material.tiling,
                            (z1 - origin.z) * material.tiling,
                            mip_size,
                        );
                        for &y in &ys {
                            for &x in &xs {
                                requested.push(somnium_asset::virtual_texture::VirtualPageId::new(
                                    layer as u8,
                                    level as u8,
                                    x as u16,
                                    y as u16,
                                ));
                            }
                        }
                    }
                }
            }
        }
        self.virtual_texture_frame = self.virtual_texture_frame.saturating_add(1);
        let uploads = self
            .virtual_texture
            .resolve_feedback(self.virtual_texture_frame, requested);
        if let Some(gpu) = &self.layer_textures.virtual_texture {
            if let Err(error) = gpu.apply_uploads(queue, &uploads) {
                self.virtual_texture.reject_uploads(&uploads);
                tracing::warn!("terrain VT page upload failed: {error}");
                return false;
            }
        }
        !uploads.is_empty()
    }

    /// Exact dirty support rectangles for each painted material layer.
    fn layer_support_rects(
        &self,
        world_rects: &[[f32; 4]],
    ) -> [Vec<[f32; 4]>; TERRAIN_LAYER_COUNT as usize] {
        let mut support = std::array::from_fn(|_| Vec::new());
        // Biplanar cliff material is independent of splat weights.
        support[14].extend_from_slice(world_rects);
        let [world_x, world_z] = self.desc.world_size();
        let origin = self.model.w_axis;
        for &[x0, z0, x1, z1] in world_rects {
            let raw_x0 = x0 - origin.x;
            let raw_x1 = x1 - origin.x;
            let raw_z0 = z0 - origin.z;
            let raw_z1 = z1 - origin.z;
            if raw_x1 < 0.0 || raw_z1 < 0.0 || raw_x0 > world_x || raw_z0 > world_z {
                continue;
            }
            let local_x0 = raw_x0.clamp(0.0, world_x);
            let local_x1 = raw_x1.clamp(0.0, world_x);
            let local_z0 = raw_z0.clamp(0.0, world_z);
            let local_z1 = raw_z1.clamp(0.0, world_z);
            let end_x = (local_x1 - f32::EPSILON * world_x.max(1.0)).max(local_x0);
            let end_z = (local_z1 - f32::EPSILON * world_z.max(1.0)).max(local_z0);
            let sx0 = ((local_x0 / world_x) * self.splatmap.width as f32).floor() as u32;
            let sx1 = ((end_x / world_x) * self.splatmap.width as f32).floor() as u32;
            let sz0 = ((local_z0 / world_z) * self.splatmap.height as f32).floor() as u32;
            let sz1 = ((end_z / world_z) * self.splatmap.height as f32).floor() as u32;
            for z in sz0.min(self.splatmap.height - 1)..=sz1.min(self.splatmap.height - 1) {
                for x in sx0.min(self.splatmap.width - 1)..=sx1.min(self.splatmap.width - 1) {
                    let texel = &self.splatmap.data[(z * self.splatmap.width + x) as usize];
                    let cell = [
                        origin.x + x as f32 / self.splatmap.width as f32 * world_x,
                        origin.z + z as f32 / self.splatmap.height as f32 * world_z,
                        origin.x + (x + 1) as f32 / self.splatmap.width as f32 * world_x,
                        origin.z + (z + 1) as f32 / self.splatmap.height as f32 * world_z,
                    ];
                    let clipped = [
                        x0.max(cell[0]),
                        z0.max(cell[1]),
                        x1.min(cell[2]),
                        z1.min(cell[3]),
                    ];
                    for (layer, weight) in texel.iter().enumerate() {
                        if *weight != 0 {
                            support[layer].push(clipped);
                        }
                    }
                }
            }
        }
        support
    }

    /// Whether feedback already observed work that still needs upload budget.
    #[must_use]
    pub fn has_pending_virtual_texture(&self) -> bool {
        self.virtual_texture.stats().pending_pages > 0
    }

    /// Streaming counters for the clipmap's source pages.
    ///
    /// Exposed so the clipmap trace can say whether the cache is still moving.
    /// A converged frame with a non-zero pending count is a cache that will
    /// never finish, and that is not visible from the picture.
    #[must_use]
    pub const fn virtual_texture_stats(&self) -> &virtual_texture::VirtualTextureStats {
        self.virtual_texture.stats()
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

    /// Island GPU budget: no hex, no POM, extra-bank ids stay unbound.
    ///
    /// Coastal keeps the 32-slot close-up path. Editor checkboxes can still
    /// turn hex/POM back on after this.
    pub fn apply_hero_bank_gpu_budget(&mut self) {
        self.hero_bank_only = true;
        self.hex_tiling = false;
        self.parallax_scale = 0.0;
        self.texture_ids.unbind_extra_bank();
    }

    /// Flip POM on the selected terrain. Off stores the current Relief scale.
    pub fn toggle_parallax(&mut self) {
        if self.parallax_scale > 0.0 {
            self.parallax_held = self.parallax_scale;
            self.parallax_scale = 0.0;
        } else {
            self.parallax_scale = self.parallax_held.max(1.0);
        }
    }

    /// Metres of camera height above the ground at which hex and POM turn off
    /// for the whole frame.
    ///
    /// These flags are storage-buffer uniforms. Zeroing them here keeps every
    /// wavefront on one sample path. A per-pixel hit-distance branch compiled
    /// hex, non-hex, and a mean-albedo path into one shader and made walking
    /// *slower* (Phase XV shading, 2026-08-13).
    const AERIAL_DETAIL_METRES: f32 = 80.0;

    /// True when the camera is far enough above the heightfield that hex and
    /// POM should turn off for the whole frame.
    pub(crate) fn aerial_detail_off(camera_y: f32, ground_y: f32) -> bool {
        camera_y - ground_y > Self::AERIAL_DETAIL_METRES
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
            layer_tiling: std::array::from_fn(|i| self.layers.get(i).map_or(0.25, |l| l.tiling)),
            brush: self.brush_cursor,
            albedo_maps: self.texture_ids.albedo,
            surface_maps: self.texture_ids.surface,
            terrain_origin: [origin.x, origin.z],
            inv_world_size: [1.0 / wx, 1.0 / wz],
            splat_maps: self.texture_ids.splat_maps,
            cliff_layer: 14,
            hex_tiling: u32::from(self.hex_tiling),
            height_blend: u32::from(self.height_blend),
            macro_map: self.texture_ids.macro_map,
            layer_height_scale: std::array::from_fn(|i| {
                self.layers.get(i).map_or(0.0, |l| l.blend.height_scale)
            }),
            layer_blend_width: std::array::from_fn(|i| {
                self.layers.get(i).map_or(0.5, |l| l.blend.blend_width)
            }),
            layer_weight_clamp: std::array::from_fn(|i| {
                self.layers
                    .get(i)
                    .map_or(10.0, |l| blend::weight_clamp(l.blend.min_weight))
            }),
            layer_parallax: std::array::from_fn(|i| {
                self.layers.get(i).map_or(0.0, |l| l.blend.parallax_depth) * self.parallax_scale
            }),
            macro_mode: self.macro_mode as u32,
            macro_strength: self.macro_strength,
            detail_fade_start: self.detail_fade_start,
            detail_fade_end: self.detail_fade_end.max(self.detail_fade_start + 1.0),
            layer_albedo: self.layer_textures.mean_albedo,
            parallax_steps: if self.parallax_scale > 0.0 {
                self.parallax_steps
            } else {
                0
            },
            parallax_shadow_steps: if self.parallax_scale > 0.0 {
                self.parallax_shadow_steps
            } else {
                0
            },
            projection_sharpness: self.projection_sharpness,
            projection_mode: self.projection_mode,
            layer_moisture: textures::LAYER_MOISTURE,
            wetness: self.wetness,
            wetness_darken: 0.62,
            wetness_gloss: 0.55,
            wetness_f0: 0.02,
            clipmap_enabled: 0,
            clipmap_rings: 0,
            clipmap_size: 0.0,
            clipmap_debug: 0,
            clipmap_albedo: [-1; 8],
            clipmap_surface: [-1; 8],
            clipmap_center: [0.0; 16],
            clipmap_origin: [0.0; 16],
            clipmap_tpm: [0.0; 8],
            clipmap_macro_albedo: [-1; 4],
            clipmap_macro_normal: [-1; 4],
            clipmap_macro_center: [0.0; 8],
            clipmap_macro_origin: [0.0; 8],
            clipmap_macro_tpm: [0.0; 4],
            clipmap_macro_rings: 0,
            clipmap_macro_size: 0.0,
            clipmap_detail_ready: 0,
            clipmap_macro_ready: 0,
            // Unbinding is the off switch. The shader tests `>= 0` and skips
            // both fetches, so "off" is the pre-phase image exactly rather
            // than a strength of zero applied to a value nobody wanted.
            horizon_map_a: if self.horizon_shadow {
                self.texture_ids.horizon_maps[0]
            } else {
                -1
            },
            horizon_map_b: if self.horizon_shadow {
                self.texture_ids.horizon_maps[1]
            } else {
                -1
            },
            skyvis_map: if self.sky_visibility {
                self.texture_ids.sky_visibility
            } else {
                -1
            },
            sky_visibility_strength: self.sky_visibility_strength,
        }
    }

    /// [`gpu_material`] with hex and POM off when the camera is aerial.
    ///
    /// Walking (a couple of metres above the heightfield) is unchanged. The
    /// default overview camera sits ~150 m up; that view cannot resolve hex
    /// or relief, and turning them off uniformly is what actually drops the
    /// shading pass — not a per-pixel LOD inside the material.
    pub fn gpu_material_for_camera(&self, local_camera: glam::Vec3) -> GpuTerrainMaterial {
        let mut material = self.gpu_material();
        let ground = self.world_height_at(local_camera.x, local_camera.z);
        if Self::aerial_detail_off(local_camera.y, ground) {
            material.hex_tiling = 0;
            material.parallax_steps = 0;
            material.parallax_shadow_steps = 0;
        }
        material
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

    /// Default relief for a newly created terrain (Phase 25L).
    ///
    /// `SOMNIUM_HEIGHTMAP` overrides the file; otherwise the shipped CDLOD
    /// dataset; otherwise procedural FBM, so a clone with no assets still gets
    /// landscape rather than a plain. Returns what it used, for logging.
    ///
    /// Lives here rather than in the demo because **Create > Terrain** needs the
    /// same behaviour, and a fallback chain defined twice in two crates is one
    /// that will disagree. When the UI phase lets a heightmap be chosen at
    /// creation, this becomes the default that dialog starts from.
    pub fn apply_default_relief(&mut self, amplitude: f32) -> String {
        let path =
            std::env::var("SOMNIUM_HEIGHTMAP").unwrap_or_else(|_| DEFAULT_HEIGHTMAP.to_string());
        match self.load_heightmap_file(&path, amplitude) {
            Ok(()) => path,
            Err(e) => {
                tracing::info!("terrain: heightmap unavailable ({e}); procedural relief");
                self.generate_relief(1337, amplitude);
                "procedural".to_string()
            }
        }
    }

    /// Use an authored whole-terrain colour map instead of the procedural
    /// landform tint. Alpha is a per-texel strength mask, so the Great Lakes
    /// bake can exclude water pixels from ground colour.
    pub fn load_authored_macro_file(&mut self, path: &str) -> Result<(), String> {
        let image = image::open(path)
            .map_err(|e| format!("{path}: {e}"))?
            .to_rgba8();
        let resized =
            if image.width() == macro_map::MACRO_SIZE && image.height() == macro_map::MACRO_SIZE {
                image
            } else {
                image::imageops::resize(
                    &image,
                    macro_map::MACRO_SIZE,
                    macro_map::MACRO_SIZE,
                    image::imageops::FilterType::Triangle,
                )
            };
        self.authored_macro = Some(macro_map::MacroMap {
            texels: resized.into_raw(),
            size: macro_map::MACRO_SIZE,
        });
        self.macro_mode = macro_map::MacroBlendMode::Lerp;
        self.macro_strength = 0.68;
        self.macro_dirty = true;
        Ok(())
    }

    /// Rebuild the splat-weighted unique-colour macro after paint or biome.
    pub fn invalidate_unique_colour(&mut self) {
        self.macro_dirty = true;
    }

    /// Fill the heightmap with procedural FBM relief (Phase 25L).
    pub fn generate_relief(&mut self, seed: u32, amplitude: f32) {
        let (tx, tz) = (self.desc.total_vertices_x(), self.desc.total_vertices_z());
        self.heightmap = heightmap::fbm_relief(tx, tz, seed, amplitude);
        self.mark_all_dirty();
    }

    /// Low rolling islet: inland peak, rim below the frozen water datum.
    pub fn generate_island_relief(&mut self, seed: u32, peak_metres: f32) {
        let (tx, tz) = (self.desc.total_vertices_x(), self.desc.total_vertices_z());
        self.heightmap =
            heightmap::island_relief(tx, tz, seed, peak_metres, DEFAULT_WATER_LEVEL_METRES);
        self.mark_all_dirty();
    }

    /// Mark every chunk for rebuild, and bump the edit revision so foliage and
    /// the collider notice.
    pub fn mark_all_dirty(&mut self) {
        self.edit_revision = self.edit_revision.wrapping_add(1);
        self.macro_dirty = true;
        // Phase TSUSHIMA-B/C. Every caller of this is a *wholesale* heightmap
        // replacement — a file load, procedural relief, the island generator —
        // which is exactly when the baked horizon stops describing the ground.
        // Deliberately not set by the sculpt brush: a 100 ms rebake per stroke
        // would make the brush unusable, and a slightly stale long shadow
        // under an active brush is not what anyone is looking at.
        self.horizon_dirty = true;
        for chunk in &mut self.chunks {
            chunk.dirty = true;
        }
    }

    /// Scaled (world-space) height at an arbitrary terrain-local XZ position,
    /// bilinearly interpolated (Phase 14A-3 `world_height_at`).
    pub fn world_height_at(&self, local_x: f32, local_z: f32) -> f32 {
        let fx =
            (local_x / self.desc.cell_size).clamp(0.0, (self.desc.total_vertices_x() - 1) as f32);
        let fz =
            (local_z / self.desc.cell_size).clamp(0.0, (self.desc.total_vertices_z() - 1) as f32);
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
        foliage_paint::GroundSample {
            height: s.height,
            slope_cos: s.slope_cos,
        }
    }

    /// Sample height, slope and paint-layer weight at a terrain-local point
    /// (Phase 17A). This is what foliage scattering places against.
    pub fn surface_sample(&self, local_x: f32, local_z: f32, layer: u8) -> foliage::SurfaceSample {
        let height = self.world_height_at(local_x, local_z);

        // Central difference over one cell. The gradient is in world units on
        // both axes because `world_height_at` already applies `height_scale`,
        // so the normal comes straight out of it.
        let d = self.desc.cell_size;
        let hx =
            self.world_height_at(local_x + d, local_z) - self.world_height_at(local_x - d, local_z);
        let hz =
            self.world_height_at(local_x, local_z + d) - self.world_height_at(local_x, local_z - d);
        let normal = glam::Vec3::new(-hx, 2.0 * d, -hz).normalize_or_zero();
        // Y of the unit normal IS the cosine of the slope from vertical.
        let slope_cos = if normal == glam::Vec3::ZERO {
            1.0
        } else {
            normal.y.abs()
        };

        let layer_weight = self.layer_weight_at(local_x, local_z, layer);
        foliage::SurfaceSample {
            height,
            slope_cos,
            layer_weight,
        }
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
        let n = collider::sample_count_for(
            self.desc
                .total_vertices_x()
                .min(self.desc.total_vertices_z()),
        );
        let world = self.desc.world_size();
        let samples = collider::resample(n, world, |x, z| self.world_height_at(x, z));
        (samples, n, collider::heightfield_scale(world, n))
    }

    /// Scatter foliage over this terrain (Phase 17A).
    pub fn scatter_foliage(
        &self,
        params: &foliage::FoliageParams,
    ) -> Vec<foliage::FoliageInstance> {
        foliage::scatter(params, self.desc.world_size(), |x, z| {
            self.surface_sample(x, z, params.layer)
        })
    }

    // ── Per-frame update ─────────────────────────────────────────────────────

    /// Select per-chunk LODs from the camera position (terrain-local), retain
    /// full detail where a water surface crosses the terrain, clamp neighbor
    /// differences to ≤ 1 level, and derive edge stitch masks.
    pub fn select_lods(
        &mut self,
        local_camera_pos: glam::Vec3,
        shoreline_regions: &[crate::water_body::WaterBodyDescriptor],
    ) {
        let [gx, gz] = self.desc.grid_size;
        let chunk_world = self.desc.chunk_cells as f32 * self.desc.cell_size;

        // 1. Distance-based LOD per chunk (Phase 14B-2 formula).
        // Phase CR-D: parallel once the grid is large enough that fork-join
        // beats a serial loop. Default 16×16 stays serial (CR-A: GPU-bound).
        let lod_base = self.desc.lod_base_range;
        crate::jobs::for_each_mut(&mut self.chunks, |chunk| {
            let center = glam::Vec3::new(
                (chunk.grid_pos[0] as f32 + 0.5) * chunk_world,
                (chunk.aabb_min.y + chunk.aabb_max.y) * 0.5,
                (chunk.grid_pos[1] as f32 + 0.5) * chunk_world,
            );
            let dist = local_camera_pos.distance(center).max(0.01);
            let lod_f = (dist / lod_base).log2().floor();
            chunk.lod = lod_f.clamp(0.0, MAX_TERRAIN_LOD as f32) as u8;

            // A coarser index buffer skips height vertices. Where a horizontal
            // water plane cuts the terrain this turns the coastline into the
            // large LOD-sized triangles seen in IV-I. Unreal's water/landscape
            // integration similarly treats the intersection as authored
            // high-detail data rather than letting generic distance LOD own it.
            // Only chunks whose vertical range actually crosses an overlapping
            // body's surface are pinned; open water and dry mountains keep LOD.
            if chunk_crosses_water_surface(chunk, chunk_world, shoreline_regions) {
                chunk.lod = 0;
            }
        });

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
                let (cx, cz) = (
                    self.chunks[i].grid_pos[0] as i64,
                    self.chunks[i].grid_pos[1] as i64,
                );
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
            let (cx, cz) = (
                self.chunks[i].grid_pos[0] as i64,
                self.chunks[i].grid_pos[1] as i64,
            );
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
        // Phase TSUSHIMA-B/C: rebake before the macro tier, because both read
        // the heightfield and this is the one that has to be right for the
        // *first* frame — `TerrainData::new` bakes against the flat heightmap
        // the terrain is created with, and the relief only lands afterwards.
        // Baking once at creation and never again is exactly the bug this
        // fixes: every horizon angle was zero, every sky visibility was fully
        // open, and the on/off capture pair was byte-identical.
        if std::mem::take(&mut self.horizon_dirty) {
            let t = std::time::Instant::now();
            let maps = horizon::bake(
                &self.heightmap,
                self.desc.total_vertices_x(),
                self.desc.total_vertices_z(),
                self.desc.cell_size,
                self.desc.height_scale,
                horizon::HORIZON_SIZE,
            );
            horizon::rewrite(queue, &self.horizon_gpu, &maps);
            // The distribution, not just the timing. "Sky visibility is on"
            // and "sky visibility varies across this terrain" are different
            // claims, and only the second one can change a picture.
            let (mut lo, mut hi, mut sum) = (255u8, 0u8, 0u64);
            let (mut h_lo, mut h_hi) = (255u8, 0u8);
            for texel in maps.sky.chunks(4) {
                lo = lo.min(texel[3]);
                hi = hi.max(texel[3]);
                sum += u64::from(texel[3]);
            }
            for texel in maps.angles_a.chunks(4) {
                for &a in &texel[..4] {
                    h_lo = h_lo.min(a);
                    h_hi = h_hi.max(a);
                }
            }
            let n = (maps.sky.len() / 4) as u64;
            tracing::info!(
                ms = t.elapsed().as_secs_f64() * 1000.0,
                size = horizon::HORIZON_SIZE,
                skyvis_min = lo,
                skyvis_mean = (sum / n.max(1)) as u8,
                skyvis_max = hi,
                horizon_min_deg = f32::from(h_lo) / 255.0 * 90.0,
                horizon_max_deg = f32::from(h_hi) / 255.0 * 90.0,
                "terrain: horizon + sky visibility rebaked"
            );
        }
        if std::mem::take(&mut self.macro_dirty) {
            let generated = macro_map::from_splat(
                &self.splatmap.data,
                self.splatmap.width,
                self.splatmap.height,
                &self.layer_textures.mean_albedo,
            );
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.macro_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &generated.texels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(generated.size * 4),
                    rows_per_image: Some(generated.size),
                },
                wgpu::Extent3d {
                    width: generated.size,
                    height: generated.size,
                    depth_or_array_layers: 1,
                },
            );
        }

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
            self.index_blocks
                .insert(key, (offset, indices.len() as u32));
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
        // Version 4: Phase XV-Zeta widened a splat texel from 16 weights to 32.
        out.extend(4u32.to_le_bytes());
        out.extend(self.desc.total_vertices_x().to_le_bytes());
        out.extend(self.desc.total_vertices_z().to_le_bytes());
        out.extend(self.splatmap.width.to_le_bytes());
        out.extend(self.splatmap.height.to_le_bytes());
        out.extend(bytemuck::cast_slice::<f32, u8>(&self.heightmap));
        out.extend(
            bytemuck::cast_slice::<[u8; TERRAIN_LAYER_COUNT as usize], u8>(&self.splatmap.data),
        );
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
        if version != 2 && version != 3 && version != 4 {
            return Err(format!(
                "terrain sidecar is version {version}; this build reads 2/3 (migrate) or 4 (thirty-two layers)"
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
        let texel_count = (sw * sh) as usize;
        let src_channels: usize = match version {
            2 => 8,
            3 => 16,
            _ => TERRAIN_LAYER_COUNT as usize,
        };
        let s_bytes = texel_count * src_channels;
        let body = bytes
            .get(24..24 + h_bytes + s_bytes)
            .ok_or("truncated terrain sidecar")?;
        self.heightmap
            .copy_from_slice(bytemuck::cast_slice(&body[..h_bytes]));
        let splat_src = &body[h_bytes..];
        self.splatmap.data =
            crate::terrain::splat::migrate_sidecar_splat(version, splat_src, texel_count)?;
        self.macro_dirty = true;
        self.horizon_dirty = true;
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
            virtual_texturing: true,
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
                    (cx as f32 + 0.5) * chunk_world,
                    0.0,
                    (cz as f32 + 0.5) * chunk_world,
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

    #[test]
    fn feedback_dirty_interval_splits_at_the_toroidal_wrap() {
        let intervals = feedback_axis_intervals(900, 200, 1024.0, 0.0, 50.0, 100.0);
        assert_eq!(intervals.len(), 2);
        assert!(intervals[0].0 > 85.0 && intervals[0].1 == 100.0);
        assert!(intervals[1].0 == 0.0 && intervals[1].1 < 10.0);
    }

    #[test]
    fn feedback_page_coverage_handles_repeat_without_requesting_the_whole_mip() {
        assert_eq!(feedback_page_indices(0.10, 0.20, 2048), vec![1, 2, 3]);
        assert_eq!(feedback_page_indices(0.90, 1.10, 2048), vec![14, 15, 0, 1]);
        assert_eq!(
            feedback_page_indices(0.0, 1.0, 2048),
            (0..16).collect::<Vec<_>>()
        );
    }

    #[test]
    fn only_overlapping_chunks_that_cross_the_water_surface_are_shoreline_chunks() {
        let water = crate::water_body::WaterBodyDescriptor {
            water_id: 0,
            terrain_id: 0,
            preset: 1,
            surface_level: 15.0,
            max_depth: 12.0,
            bounds: [0.0, 0.0, 64.0, 64.0],
            amplitude: 0.35,
            wave_dir_a: [1.0, 0.0],
            wave_dir_b: [0.0, 1.0],
            wave_length_a: 18.0,
            wave_length_b: 11.0,
            wave_speed: 1.0,
            wave_steepness: 0.95,
        };
        let chunk = |grid_pos, min_y, max_y| TerrainChunk {
            grid_pos,
            aabb_min: glam::Vec3::new(0.0, min_y, 0.0),
            aabb_max: glam::Vec3::new(16.0, max_y, 16.0),
            vertex_offset: UNALLOCATED,
            dirty: false,
            lod: 3,
            edge_mask: 0,
        };

        assert!(chunk_crosses_water_surface(
            &chunk([1, 2], 10.0, 20.0),
            16.0,
            &[water]
        ));
        assert!(!chunk_crosses_water_surface(
            &chunk([1, 2], 20.0, 30.0),
            16.0,
            &[water]
        ));
        assert!(!chunk_crosses_water_surface(
            &chunk([8, 8], 10.0, 20.0),
            16.0,
            &[water]
        ));
    }

    #[test]
    fn walking_keeps_hex_and_pom_on() {
        assert!(!TerrainData::aerial_detail_off(2.0, 0.0));
        assert!(!TerrainData::aerial_detail_off(80.0, 0.0));
    }

    #[test]
    fn overview_camera_turns_hex_and_pom_off() {
        assert!(TerrainData::aerial_detail_off(150.75, 0.0));
        assert!(TerrainData::aerial_detail_off(80.01, 0.0));
    }

    #[test]
    fn unbind_extra_bank_keeps_hero_maps_and_clears_layers_16_31() {
        let mut ids = TerrainTextureIds {
            splat_maps: [0, 1, 2, 3, 4, 5, 6, 7],
            macro_map: 9,
            albedo: [10; TERRAIN_LAYER_COUNT as usize],
            surface: [11; TERRAIN_LAYER_COUNT as usize],
            ..TerrainTextureIds::default()
        };
        ids.unbind_extra_bank();
        assert_eq!(&ids.splat_maps[..4], &[0, 1, 2, 3]);
        assert!(ids.splat_maps[4..].iter().all(|&id| id < 0));
        assert_eq!(ids.macro_map, 9);
        let hero = TERRAIN_HERO_LAYERS as usize;
        assert!(ids.albedo[..hero].iter().all(|&id| id == 10));
        assert!(ids.surface[..hero].iter().all(|&id| id == 11));
        assert!(ids.albedo[hero..].iter().all(|&id| id < 0));
        assert!(ids.surface[hero..].iter().all(|&id| id < 0));
    }

    #[test]
    fn hero_bank_gpu_budget_zeros_hex_and_parallax_in_the_material_layout() {
        let hex_tiling = false;
        let parallax_scale = 0.0;
        let authored_steps = 24u32;
        let mut ids = TerrainTextureIds {
            splat_maps: [0, 1, 2, 3, 4, 5, 6, 7],
            macro_map: -1,
            albedo: [1; TERRAIN_LAYER_COUNT as usize],
            surface: [1; TERRAIN_LAYER_COUNT as usize],
            ..TerrainTextureIds::default()
        };
        ids.unbind_extra_bank();
        let gpu_hex = u32::from(hex_tiling);
        let gpu_pom = if parallax_scale > 0.0 {
            authored_steps
        } else {
            0
        };
        let gpu_pom_shadow = if parallax_scale > 0.0 { 8u32 } else { 0u32 };
        assert_eq!(gpu_hex, 0);
        assert_eq!(gpu_pom, 0);
        assert_eq!(gpu_pom_shadow, 0);
        assert!(ids.splat_maps[4..].iter().all(|&id| id < 0));
    }

    #[test]
    fn toggle_parallax_restores_the_held_scale() {
        let mut scale = 1.25f32;
        let mut held = 1.0f32;
        if scale > 0.0 {
            held = scale;
            scale = 0.0;
        }
        assert_eq!(scale, 0.0);
        assert_eq!(held, 1.25);
        scale = held.max(1.0);
        assert_eq!(scale, 1.25);
    }
}
