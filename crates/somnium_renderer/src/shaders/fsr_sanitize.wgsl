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
    history_valid: f32,
    _p2: f32,
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> exposure: Exposure;
@group(0) @binding(3) var depth_tex: texture_depth_2d;
@group(0) @binding(4) var depth_out: texture_storage_2d<r32float, write>;
@group(0) @binding(5) var visibility: texture_2d<u32>;
@group(0) @binding(6) var<storage, read> reactive_instances: array<u32>;
@group(0) @binding(7) var previous_coverage: texture_2d<f32>;
@group(0) @binding(8) var current_coverage: texture_storage_2d<r32float, write>;
@group(0) @binding(9) var reactive_mask: texture_storage_2d<r32float, write>;
@group(0) @binding(10) var velocity: texture_2d<f32>;
@group(0) @binding(11) var particle_coverage: texture_2d<f32>;
@group(0) @binding(12) var motion_out: texture_storage_2d<rg16float, write>;

/// How much FSR should trust this frame inside a moving object (a body, a held tool, a swaying lamp, an animated
/// plant) that was there last frame too. Since the velocity pass knows how these moved (C2, 16 Oct 2026) their
/// history is good, and the less of the raw frame shows, the less of the jitter shows: 0.35, and 1.0 before it, made
/// everything animated shimmer and pop under FSR while it held still with SMAA (the developer, 18 Oct 2026). A
/// little stays for what motion vectors cannot describe (lighting that changes as a body turns).
const MOVING_REACTIVE: f32 = 0.12;
/// Round a moving object's edge, and where it was last frame: what it uncovers has no history of its own. Full
/// rejection there (the old 1.0) made a ring round every swaying thing flicker with the jitter; FSR's own
/// disocclusion test catches the rest.
const EDGE_REACTIVE: f32 = 0.45;

/// Is the visible surface at exactly this pixel a reactive (camera-held) draw?
fn held(coord: vec2<i32>) -> bool {
    let packed = textureLoad(visibility, coord, 0).x;
    if packed == 0u { return false; }
    let instance = packed - 1u;
    return instance < arrayLength(&reactive_instances) && reactive_instances[instance] != 0u;
}

fn inside(coord: vec2<i32>, dims: vec2<i32>) -> bool {
    return all(coord >= vec2<i32>(0)) && all(coord < dims);
}

/// What makes this pixel's history doubtful now: particles (no motion of their own, so full strength) and, at the
/// edge strength, any moving object within the jitter's reach. Also what the next frame reads as "last frame".
fn dynamic_coverage(coord: vec2<i32>, dims: vec2<i32>) -> f32 {
    var coverage = 0.0;
    // Cover subpixel jitter and the temporal reconstruction filter footprint.
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let c = coord + vec2<i32>(x, y);
            if !inside(c, dims) { continue; }
            if inside(c, vec2<i32>(textureDimensions(particle_coverage))) {
                coverage = max(coverage, textureLoad(particle_coverage, c, 0).r);
            }
            if held(c) {
                coverage = max(coverage, EDGE_REACTIVE);
            }
        }
    }
    return coverage;
}

fn particles_here(coord: vec2<i32>) -> f32 {
    if !inside(coord, vec2<i32>(textureDimensions(particle_coverage))) { return 0.0; }
    return textureLoad(particle_coverage, coord, 0).r;
}

fn previous_dynamic(coord: vec2<i32>, dims: vec2<i32>) -> f32 {
    var coverage = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let c = coord + vec2<i32>(x, y);
            if inside(c, dims) {
                coverage = max(coverage, textureLoad(previous_coverage, c, 0).r);
            }
        }
    }
    return coverage;
}

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
    let dims = vec2<i32>(dim);
    let current = dynamic_coverage(coord, dims);
    var reactive = current;
    let motion = textureLoad(velocity, coord, 0).xy;
    let held_here = held(coord);
    if exposure.history_valid > 0.5 {
        let previous_pixel = vec2<i32>(floor(vec2<f32>(coord) + 0.5 + motion * vec2<f32>(dim)));
        // Camera reprojection covers a moving view; the screen-space union also
        // covers a removed foreground object whose replacement background has
        // a different depth/motion. Only current coverage enters next history,
        // so a vanished silhouette cannot keep propagating its own rejection.
        reactive = max(reactive, previous_dynamic(previous_pixel, dims));
        reactive = max(reactive, previous_dynamic(coord, dims));
        // Inside a moving object that was there last frame (where its own motion says it was): its history is
        // the object itself. Silhouette rims (the 3x3 dilation without a moving centre) and uncovered background
        // keep the edge strength.
        var interior = held_here;
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let c = coord + vec2<i32>(x, y);
                interior = interior && inside(c, dims) && held(c);
            }
        }
        if interior && previous_dynamic(previous_pixel, dims) > 0.0 {
            reactive = max(particles_here(coord), MOVING_REACTIVE);
        }
    }
    // Held pixels keep their motion: the velocity pass now knows how hands and tools moved (C2), where it used to
    // be zeroed on the guess that they ride with the camera.
    textureStore(motion_out, coord, vec4<f32>(motion, 0.0, 0.0));
    textureStore(current_coverage, coord, vec4<f32>(current, 0.0, 0.0, 0.0));
    textureStore(reactive_mask, coord, vec4<f32>(reactive, 0.0, 0.0, 0.0));
    let c = textureLoad(src, coord, 0);
    let pre_exposed = sanitize(c.rgb) * max(exposure.value, 1e-8);
    textureStore(dst, coord, vec4<f32>(pre_exposed, c.a));
    textureStore(depth_out, coord, vec4<f32>(textureLoad(depth_tex, coord, 0), 0.0, 0.0, 0.0));
}
