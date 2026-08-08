// Ray query is an extension, not core WGSL, so it has to be enabled
// explicitly. Must be the first thing in the module.
enable wgpu_ray_query;

// Phase 24J: ray-traced shadow, as the acceptance test for the acceleration
// structures.
//
// Building a BLAS and a TLAS produces no visible output, so on its own 24J is
// unverifiable — it either works or it silently does not, and there is no way
// to tell by looking. This traces one shadow ray per pixel toward the sun and
// writes the result over the HDR target. If the structures are built, aligned
// and bound correctly, the scene shows hard ray-traced shadows. If any of that
// is wrong the image is uniformly lit or uniformly black, and which one it is
// says where the fault lies.
//
// It is also the smallest possible version of what 24K does properly: ReSTIR DI
// is this ray plus reservoir resampling over many lights.

struct RtParams {
    inv_view_proj: mat4x4<f32>,
    /// Direction *toward* the sun, world space.
    sun_direction: vec3<f32>,
    /// Offset along the normal before tracing, to avoid self-hits.
    ray_bias: f32,
}

@group(0) @binding(0) var accel:     acceleration_structure;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var out_tex:   texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> params: RtParams;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(depth_tex);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }
    let coord = vec2<i32>(gid.xy);
    let depth = textureLoad(depth_tex, coord, 0);

    // Sky: nothing to shadow, and reconstructing a position on the far plane
    // would put the ray origin somewhere arbitrary.
    if depth >= 1.0 {
        textureStore(out_tex, coord, vec4<f32>(0.25, 0.4, 0.8, 1.0));
        return;
    }

    let uv = (vec2<f32>(coord) + 0.5) / vec2<f32>(dims);
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world = params.inv_view_proj * ndc;
    let origin = world.xyz / world.w;

    // Offset along the ray rather than along the surface normal, which is not
    // available here. Enough to clear the surface the ray starts on; too small
    // and every pixel shadows itself, which is the classic first symptom of a
    // ray tracer that is otherwise working.
    let dir = normalize(params.sun_direction);
    var rq: ray_query;
    rayQueryInitialize(
        &rq,
        accel,
        RayDesc(
            // Terminate on the first hit: a shadow ray only needs to know
            // whether *anything* is in the way, not what or how far.
            0x4u,
            0xffu,
            params.ray_bias,
            10000.0,
            origin,
            dir,
        ),
    );
    rayQueryProceed(&rq);
    let hit = rayQueryGetCommittedIntersection(&rq);

    let lit = select(1.0, 0.15, hit.kind != RAY_QUERY_INTERSECTION_NONE);
    textureStore(out_tex, coord, vec4<f32>(vec3<f32>(lit), 1.0));
}
