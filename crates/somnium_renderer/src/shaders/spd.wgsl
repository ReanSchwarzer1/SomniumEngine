// Somnium Engine — single-pass downsample (Phase 24AC, the SPD half).
//
// The Hi-Z pyramid used to cost one dispatch per mip: eleven of them at
// 1280x720, each a full pipeline barrier behind the last, each reading a
// texture the previous one had just written. The work is trivial and the
// *dependency chain* is the cost.
//
// SPD's observation is that a workgroup which owns a 64x64 tile of the source
// can compute six mip levels of that tile entirely in its own shared memory,
// because after the first reduction the whole tile fits there. Only when a
// level needs data from more than one tile does a dispatch boundary become
// necessary — and by then the image is 64x smaller.
//
// # Reference
//
// `SpartanEngine-master/data/shaders/amd_fidelity_fx/ffx_spd.h` and `spd.hlsl`
// (AMD FidelityFX SPD, MIT). Specifically `SpdDownsampleMips_0_1_LDS` — the
// no-wave-operations path, which is the one to port because WGSL has no
// subgroup quad swizzles.
//
// # What is deliberately different
//
// **The last-workgroup trick is not ported.** SPD does the whole pyramid in one
// dispatch: a global atomic counter elects the workgroup that finishes last,
// and that one reads mip 6 — written by *other* workgroups — and carries on to
// mip 12. That requires `globallycoherent` storage images, and **WGSL has no
// such qualifier**: its memory model gives no way to make one workgroup's
// texture writes visible to another within a dispatch. `storageBarrier` is
// workgroup-scoped. Relying on it working anyway would be a race that happens
// to pass on one driver.
//
// So the same shader is dispatched twice instead, which a real barrier
// separates: once over the whole image to produce six mips, and once with a
// single workgroup to finish the tail from the sixth. Eleven dispatches become
// three (the depth copy, then these two), the structure is SPD's, and nothing
// depends on undefined behaviour.
//
// # Reduction
//
// `max`, because this is a Hi-Z pyramid: a texel holds the FURTHEST depth of
// the region it covers, and occlusion culling is only conservative if the
// reduction can never make an occluder look further away than it is.
//
// **Odd sizes widen to three.** Halving 5 gives 2, and a plain 2x2 reduction
// would drop source column 4 entirely — a real occluder vanishing from the
// pyramid, which is the one error direction that rejects visible geometry. The
// original per-mip shader handled this and so does this one.

struct SpdParams {
    /// Dimensions of the level this dispatch reads.
    src_size: vec2<u32>,
    /// How many mips to write. 1..=6.
    mip_count: u32,
    _pad: u32,
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> spd: SpdParams;
@group(0) @binding(2) var dst1: texture_storage_2d<r32float, write>;
@group(0) @binding(3) var dst2: texture_storage_2d<r32float, write>;
@group(0) @binding(4) var dst3: texture_storage_2d<r32float, write>;
@group(0) @binding(5) var dst4: texture_storage_2d<r32float, write>;
@group(0) @binding(6) var dst5: texture_storage_2d<r32float, write>;
@group(0) @binding(7) var dst6: texture_storage_2d<r32float, write>;

/// The 64x64 source tile, reduced once, is 32x32 — which is why six mips fit in
/// one workgroup and the seventh does not.
var<workgroup> tile: array<f32, 1024>;

fn lds(x: u32, y: u32) -> f32 {
    return tile[y * 32u + x];
}

fn lds_store(x: u32, y: u32, v: f32) {
    tile[y * 32u + x] = v;
}

/// Furthest depth of the 2x2 (or 3x2 / 2x3 / 3x3) source region under output
/// texel `p`, clamped to the real source size.
fn reduce_load(p: vec2<u32>) -> f32 {
    let s = spd.src_size;
    let base = p * 2u;
    // Widen on an axis whose source extent is odd, so the trailing texel is not
    // dropped. `wide` is 1 when this output texel is the last one on that axis
    // and there is a third source texel to account for.
    let out_size = max(s / 2u, vec2<u32>(1u));
    let wide = vec2<u32>(
        select(0u, 1u, (s.x & 1u) == 1u && p.x + 1u == out_size.x),
        select(0u, 1u, (s.y & 1u) == 1u && p.y + 1u == out_size.y),
    );
    var v = -1.0;
    for (var dy = 0u; dy <= 1u + wide.y; dy = dy + 1u) {
        for (var dx = 0u; dx <= 1u + wide.x; dx = dx + 1u) {
            let c = min(base + vec2<u32>(dx, dy), s - vec2<u32>(1u));
            v = max(v, textureLoad(src, vec2<i32>(c), 0).r);
        }
    }
    return v;
}

/// Store to the right mip, bounds-checked. WGSL has no array of storage
/// textures of differing sizes, so this is a switch rather than an index.
fn store_mip(level: u32, p: vec2<u32>, v: f32) {
    let c = vec2<i32>(p);
    switch level {
        case 1u: { textureStore(dst1, c, vec4<f32>(v, 0.0, 0.0, 0.0)); }
        case 2u: { textureStore(dst2, c, vec4<f32>(v, 0.0, 0.0, 0.0)); }
        case 3u: { textureStore(dst3, c, vec4<f32>(v, 0.0, 0.0, 0.0)); }
        case 4u: { textureStore(dst4, c, vec4<f32>(v, 0.0, 0.0, 0.0)); }
        case 5u: { textureStore(dst5, c, vec4<f32>(v, 0.0, 0.0, 0.0)); }
        default: { textureStore(dst6, c, vec4<f32>(v, 0.0, 0.0, 0.0)); }
    }
}

/// Size of the level `n` steps below `spd.src_size`.
fn level_size(n: u32) -> vec2<u32> {
    return max(spd.src_size >> vec2<u32>(n), vec2<u32>(1u));
}

@compute @workgroup_size(16, 16, 1)
fn main(
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
) {
    let x = lid.x;
    let y = lid.y;

    // ── Mip 1: four 2x2 reductions per thread ────────────────────────────────
    // Each workgroup owns a 64x64 tile of the source, so 16x16 threads each
    // produce four outputs — the same four-value scheme SPD's LDS path uses,
    // laid out as quadrants of the tile.
    let tile_origin = wg.xy * 32u;
    let m1 = level_size(1u);
    for (var q = 0u; q < 4u; q = q + 1u) {
        let off = vec2<u32>((q & 1u) * 16u, (q >> 1u) * 16u);
        let local = vec2<u32>(x, y) + off;
        let p = tile_origin + local;
        var v = -1.0;
        if p.x < m1.x && p.y < m1.y {
            v = reduce_load(p);
            store_mip(1u, p, v);
        }
        lds_store(local.x, local.y, v);
    }

    // ── Mips 2..6: reduce the tile in shared memory ──────────────────────────
    // No texture round-trip and no dispatch boundary; this is the whole point
    // of the technique. `-1.0` marks a texel outside the real mip, and `max`
    // ignores it — depth is in [0, 1], so a negative can never win.
    var size = 32u;
    for (var level = 2u; level <= spd.mip_count; level = level + 1u) {
        size = size / 2u;
        workgroupBarrier();
        var v = -1.0;
        let in_range = x < size && y < size;
        if in_range {
            v = max(
                max(lds(x * 2u, y * 2u), lds(x * 2u + 1u, y * 2u)),
                max(lds(x * 2u, y * 2u + 1u), lds(x * 2u + 1u, y * 2u + 1u)),
            );
        }
        // Read before write, or a thread would clobber a neighbour's input.
        workgroupBarrier();
        if in_range {
            lds_store(x, y, v);
            let dims = level_size(level);
            let p = wg.xy * size + vec2<u32>(x, y);
            if v >= 0.0 && p.x < dims.x && p.y < dims.y {
                store_mip(level, p, v);
            }
        }
    }
}
