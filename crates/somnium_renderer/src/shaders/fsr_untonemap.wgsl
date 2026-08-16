// Restore linear scene radiance after FSR processed pre-exposed HDR.

struct Exposure {
    value: f32,
    sharpness: f32,
    _p1: f32,
    _p2: f32,
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> exposure: Exposure;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dim = textureDimensions(src);
    if id.x >= dim.x || id.y >= dim.y {
        return;
    }
    let coord = vec2<i32>(id.xy);
    let max_coord = vec2<i32>(dim) - vec2<i32>(1);
    let c = textureLoad(src, coord, 0);
    let north = textureLoad(src, clamp(coord + vec2<i32>(0, -1), vec2<i32>(0), max_coord), 0).rgb;
    let south = textureLoad(src, clamp(coord + vec2<i32>(0, 1), vec2<i32>(0), max_coord), 0).rgb;
    let east = textureLoad(src, clamp(coord + vec2<i32>(1, 0), vec2<i32>(0), max_coord), 0).rgb;
    let west = textureLoad(src, clamp(coord + vec2<i32>(-1, 0), vec2<i32>(0), max_coord), 0).rgb;
    let centre = max(c.rgb, vec3<f32>(0.0));
    let neighbourhood_min = max(min(centre, min(min(north, south), min(east, west))), vec3<f32>(0.0));
    let neighbourhood_max = max(centre, max(max(north, south), max(east, west)));
    let average = (north + south + east + west) * 0.25;
    // Bounded unsharp masking supplies the FSR sharpness control without the
    // old RCAS shader's negative-output failure on near-black night pixels.
    let sharpened = centre + (centre - average) * exposure.sharpness * 0.35;
    let pre_exposed = clamp(sharpened, neighbourhood_min, neighbourhood_max);
    textureStore(dst, coord, vec4<f32>(
        pre_exposed / max(exposure.value, 1e-8), c.a));
}
