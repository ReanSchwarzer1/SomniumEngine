// Somnium Engine — Terrain clipmap generate (Phase DF).
//
// Writes one dirty rectangle of one ring. Concatenated after `global_pool`,
// `hextile`, and `terrain_material`, which supply `textures`, the splat blend,
// and `terrain_generate_texel`. `default_sampler` is this group's binding 3
// so the same hex / layer path the fragment shader uses can run in compute
// with explicit derivatives (the texel size of this ring).
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

@group(1) @binding(0) var clipmap_albedo_out: texture_storage_2d_array<rgba8unorm, write>;
@group(1) @binding(1) var clipmap_surface_out: texture_storage_2d_array<rgba8unorm, write>;
@group(1) @binding(2) var<uniform> clipmap_gen: ClipmapGenParams;
@group(1) @binding(3) var default_sampler: sampler;

@compute @workgroup_size(8, 8, 1)
fn clipmap_generate(@builtin(global_invocation_id) gid: vec3<u32>) {
    let tex = clipmap_gen.rect_min + gid.xy;
    if tex.x >= clipmap_gen.rect_max.x || tex.y >= clipmap_gen.rect_max.y {
        return;
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
    textureStore(
        clipmap_albedo_out,
        vec2<i32>(i32(tex.x), i32(tex.y)),
        clipmap_gen.ring,
        packed.albedo,
    );
    textureStore(
        clipmap_surface_out,
        vec2<i32>(i32(tex.x), i32(tex.y)),
        clipmap_gen.ring,
        packed.surface,
    );
}
