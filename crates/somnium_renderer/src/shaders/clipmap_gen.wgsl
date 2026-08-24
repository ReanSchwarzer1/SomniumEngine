// MORROWIND-C: composition is declared here rather than assembled by a
// `format!` of `include_str!` calls at this pass's construction site. The
// resolver (`somnium_shader`) emits each module once, in this order, and
// hoists every `enable` above everything.
//!include "global_pool.wgsl"
//!include "hextile.wgsl"
//!include "terrain_material.wgsl"

// Somnium Engine — Terrain clipmap generate (Phase DF).
//
// Fragment pass, not compute. Live shading already samples bindless layers in
// the fragment stage; compute `textureSampleGrad` on that array wrote black
// (Dbg 32 silhouettes) even after copying the storage images. UE5 RVT and
// this engine's other caches (water HDR copy, G-buffer) are color attachments
// for the same reason. Concatenated after `global_pool`, `hextile`, and
// `terrain_material`. Group 1 is params + `default_sampler`.
//
// World XZ only — no view vector, no POM. Shading marches the baked height.

struct ClipmapGenParams {
    terrain_index: u32,
    ring: u32,
    rect_min: vec2<u32>,
    rect_max: vec2<u32>,
    center: vec2<f32>,
    origin_uv: vec2<f32>,
    texels_per_m: f32,
    clipmap_size: f32,
    hex: u32,
    _pad: u32,
    _pad2: vec2<u32>,
}

@group(1) @binding(0) var<uniform> clipmap_gen: ClipmapGenParams;
@group(1) @binding(1) var default_sampler: sampler;

struct ClipmapFsOut {
    @location(0) albedo: vec4<f32>,
    @location(1) surface: vec4<f32>,
}

@vertex
fn clipmap_vs(@builtin(vertex_index) vid: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vid << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vid & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn clipmap_generate(@builtin(position) pos: vec4<f32>) -> ClipmapFsOut {
    let tex = vec2<u32>(u32(pos.x), u32(pos.y));
    var out: ClipmapFsOut;
    out.albedo = vec4<f32>(0.0);
    out.surface = vec4<f32>(0.5, 0.5, 0.8, 1.0);
    if tex.x < clipmap_gen.rect_min.x || tex.y < clipmap_gen.rect_min.y
        || tex.x >= clipmap_gen.rect_max.x || tex.y >= clipmap_gen.rect_max.y
    {
        return out;
    }
    let size = clipmap_gen.clipmap_size;
    let physical = (vec2<f32>(f32(tex.x), f32(tex.y)) + 0.5) / size;
    let logical = fract(physical - clipmap_gen.origin_uv);
    let extent = size / clipmap_gen.texels_per_m;
    let world_xz = clipmap_gen.center + (logical - vec2<f32>(0.5)) * extent;
    let texel_m = 1.0 / clipmap_gen.texels_per_m;
    let packed = terrain_generate_texel(
        clipmap_gen.terrain_index,
        world_xz,
        vec2<f32>(texel_m, 0.0),
        vec2<f32>(0.0, texel_m),
        clipmap_gen.hex != 0u,
    );
    // Store albedo **perceptually**, not linearly.
    //
    // The cache is `Rgba8Unorm`. Terrain albedo is dark — forest floor and wet
    // rock sit around 0.02–0.05 linear — so writing linear values spent about
    // five of 256 codes on the whole range the ground actually occupies. That
    // is visible banding, and because quantisation of a convex curve does not
    // average out, it is also a systematic brightness offset against the live
    // path. `clipmap_tap_detail` / `clipmap_tap_macro` square it back.
    //
    // A `*Srgb` view would do the same job in hardware, but the surface target
    // shares this pipeline's colour state and its normal/roughness/AO channels
    // must stay linear.
    // Alpha carries the already-evaluated wet factor. The shading path uses it
    // to reproduce live terrain's dielectric F0 instead of silently dropping
    // the wet specular term whenever a clipmap ring is active.
    out.albedo = vec4<f32>(sqrt(max(packed.albedo.rgb, vec3<f32>(0.0))), packed.albedo.a);
    // Alpha carries occlusion, and **zero is reserved** as "this texel has
    // never been generated" — wgpu zero-fills a new texture and a ring can
    // briefly hold a strip generate has not reached yet, which shading has to
    // be able to tell apart from data. One code of headroom is far below what
    // an 8-bit AO channel resolves, and a real AO map never reaches a true
    // zero anyway; without the floor a genuinely dark crevice would be thrown
    // away as if it were missing.
    out.surface = vec4<f32>(
        packed.surface.rgb,
        max(packed.surface.a, 1.0 / 255.0),
    );
    return out;
}
