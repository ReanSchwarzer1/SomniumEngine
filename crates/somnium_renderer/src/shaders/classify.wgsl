// MORROWIND-C: composition is declared here rather than assembled by a
// `format!` of `include_str!` calls at this pass's construction site. The
// resolver (`somnium_shader`) emits each module once, in this order, and
// hoists every `enable` above everything.
//!include "global_pool.wgsl"
//!include "pixel_class.wgsl"

// Somnium Engine — tile classification for binned shading (Phase DOOM-C).
//
// Splits the screen into tiles, decides what each tile contains, and appends it
// to that bin's tile list along with the instance count of an indirect draw.
// The shading pass then issues one draw per bin against a pipeline compiled for
// exactly that bin's code — which is `ShadingSpec` moved from per-frame to
// per-tile.
//
// # Why this is worth doing at all, honestly
//
// DOOM-B measured the prize. On Coastal ground, terrain is 97.6% of the shading
// pass and sky — a quarter of the screen — is 0.3% of it, so separating the two
// is worth about 0.4 ms: 0.075 ms of sky execution plus a 0.36 ms occupancy tax.
// That is 1.5% of the shading pass, not the double-digit win the phase plan's §1
// assumed before there was a census to check it against.
//
// It is built anyway because it is the *mechanism* DOOM-E needs. Terrain at
// walking height costs 9.58 ns/pixel against 4.88 ns/pixel from the overview,
// and the only way to give those two a different shader is to give them a
// different pipeline — a different *branch* is explicitly forbidden (XV-Zeta
// §11.1: `close` / `use_maps` / `layer_budget` took walking from 20 to 27 ms).
//
// # A tile is a bin only if every pixel agrees
//
// A tile whose pixels disagree goes to `MIXED`, which runs today's full shader.
// Correctness first: a straddling tile shaded by the sky pipeline would render
// terrain as background. If MIXED ever dominates the screen the tile size is
// wrong, and that is a measurement, not a guess.
//
// Concatenated after `global_pool.wgsl` and `pixel_class.wgsl`.

const TILE_SKY:    u32 = 0u;
const TILE_MESH:   u32 = 1u;
const TILE_FOLIAGE: u32 = 2u;
const TILE_TERRAIN_NEAR: u32 = 3u;
const TILE_TERRAIN_AERIAL: u32 = 4u;
const TILE_MIXED:  u32 = 5u;
const TILE_BIN_COUNT: u32 = 6u;

/// Sentinel for "no tile class decided yet".
const TILE_UNSET: u32 = 0xFFFFFFFFu;

struct ClassifyParams {
    width: u32,
    height: u32,
    /// Tiles across the screen, so the shader and the CPU cannot disagree about
    /// where a bin's region of the tile buffer starts.
    tiles_x: u32,
    tile_capacity: u32,
    /// Camera distance past which terrain takes the aerial pipeline (DOOM-E).
    aerial_split: f32,
    /// Tile edge in pixels. Decoupled from the workgroup size, which is capped
    /// at 256 invocations — a 32-pixel tile is 1024 pixels and cannot be one
    /// thread each.
    tile_size: u32,
    _pad1: f32,
    _pad2: f32,
}

/// Matches `wgpu::util::DrawIndirectArgs`.
struct DrawArgs {
    vertex_count: u32,
    instance_count: atomic<u32>,
    first_vertex: u32,
    first_instance: u32,
}

@group(1) @binding(0) var vis_buffer: texture_2d<u32>;
@group(1) @binding(1) var class_depth: texture_depth_2d;
@group(1) @binding(2) var<storage, read_write> draw_args: array<DrawArgs, 6>;
@group(1) @binding(3) var<storage, read_write> tiles: array<u32>;
@group(1) @binding(4) var<uniform> params: ClassifyParams;

/// Agreed class across the group, or `TILE_MIXED`.
var<workgroup> group_kind: atomic<u32>;

fn tile_bin(coord: vec2<i32>) -> u32 {
    let pc = pc_classify(coord, vec2<u32>(params.width, params.height));
    switch pc.kind {
        case PC_SKY: {
            return TILE_SKY;
        }
        case PC_FOLIAGE: {
            return TILE_FOLIAGE;
        }
        case PC_TERRAIN: {
            if pc.distance < params.aerial_split {
                return TILE_TERRAIN_NEAR;
            }
            return TILE_TERRAIN_AERIAL;
        }
        default: {
            return TILE_MESH;
        }
    }
}

/// Threads per workgroup edge. Fixed at 8 (64 invocations) and independent of
/// `tile_size`: `max_compute_invocations_per_workgroup` is 256 on the wgpu
/// default limits, so a 32-pixel tile cannot be one thread per pixel. Each
/// thread strides over its share instead.
const CLASSIFY_THREADS: u32 = 8u;

@compute @workgroup_size(8, 8, 1)
fn cs_main(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    if lane == 0u {
        atomicStore(&group_kind, TILE_UNSET);
    }
    workgroupBarrier();

    let origin = wid.xy * params.tile_size;
    for (var dy = lid.y; dy < params.tile_size; dy += CLASSIFY_THREADS) {
        for (var dx = lid.x; dx < params.tile_size; dx += CLASSIFY_THREADS) {
            let p = origin + vec2<u32>(dx, dy);
            // Pixels off the right or bottom edge take no part in the vote. A
            // tile that is *entirely* off-screen is not emitted at all, because
            // nothing is ever stored and `group_kind` stays unset.
            if p.x >= params.width || p.y >= params.height {
                continue;
            }
            let mine = tile_bin(vec2<i32>(i32(p.x), i32(p.y)));
            // `atomicMax` against the sentinel would always win, so the vote is:
            // claim the class if nobody has, and demote to MIXED on any
            // disagreement. The common case — a tile entirely of one class —
            // resolves on the first compare-exchange.
            let prev = atomicCompareExchangeWeak(&group_kind, TILE_UNSET, mine);
            if !prev.exchanged && prev.old_value != mine {
                atomicStore(&group_kind, TILE_MIXED);
            }
        }
    }
    workgroupBarrier();

    if lane != 0u {
        return;
    }
    let kind = atomicLoad(&group_kind);
    if kind == TILE_UNSET {
        return;
    }

    // Each bin owns a fixed slice of the tile buffer, so a bin's tiles are
    // contiguous and the draw can read them with one base offset. Fixed rather
    // than packed because a packed layout would need a prefix sum over the bins
    // before any tile could be written, which is a second dispatch to save
    // about a megabyte.
    let slot = atomicAdd(&draw_args[kind].instance_count, 1u);
    if slot < params.tile_capacity {
        tiles[kind * params.tile_capacity + slot] = wid.y * params.tiles_x + wid.x;
    }
}
