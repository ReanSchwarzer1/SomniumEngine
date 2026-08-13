// Inverse of `tonemap_for_blend` in fsr_sanitize.wgsl.
// FSR ran in exposure-normalised Karis space; ACES still wants linear cd/m².

struct Exposure {
    value: f32,
    _p0: f32,
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
    let c = textureLoad(src, coord, 0);
    let compressed = clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(0.995));
    let peak = max(max(compressed.r, compressed.g), compressed.b);
    let expanded = compressed / max(1.0 - peak, 1e-4);
    textureStore(dst, coord, vec4<f32>(expanded / max(exposure.value, 1e-8), c.a));
}
