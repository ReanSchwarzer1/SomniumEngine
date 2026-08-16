// Prepare FSR's linear HDR color and float depth inputs.
//
// FSR's HDR contract requires a linear signal plus the matching pre-exposure
// value. The former Karis-compressed input violated that contract and made the
// backend's separate low-light failure impossible to diagnose independently.

const HDR_CEILING: f32 = 60000.0;

struct Exposure {
    /// Linear multiplier, locked for the FSR history — not the adapting meter.
    value: f32,
    _p0: f32,
    _p1: f32,
    _p2: f32,
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> exposure: Exposure;
@group(0) @binding(3) var depth_tex: texture_depth_2d;
@group(0) @binding(4) var depth_out: texture_storage_2d<r32float, write>;

fn sanitize(c: vec3<f32>) -> vec3<f32> {
    let finite = select(vec3<f32>(0.0), c, c == c);
    return clamp(finite, vec3<f32>(0.0), vec3<f32>(HDR_CEILING));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dim = textureDimensions(src);
    if id.x >= dim.x || id.y >= dim.y {
        return;
    }
    let coord = vec2<i32>(id.xy);
    let c = textureLoad(src, coord, 0);
    let pre_exposed = sanitize(c.rgb) * max(exposure.value, 1e-8);
    textureStore(dst, coord, vec4<f32>(pre_exposed, c.a));
    textureStore(depth_out, coord, vec4<f32>(textureLoad(depth_tex, coord, 0), 0.0, 0.0, 0.0));
}
