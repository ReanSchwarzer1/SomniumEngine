// Phase 15B: GPU instance frustum culling.
//
// One thread per draw. Transforms the draw's local AABB into world space,
// tests it against the six frustum planes, and writes the verdict straight
// into the indirect draw arguments as `instance_count` (1 = draw, 0 = skip).
//
// Nothing is removed from the argument array — indices stay stable, so a
// culled draw simply costs nothing on the GPU and the CPU never learns about
// it. This mirrors UE5's instance-culling pass, which likewise flags draws in
// place rather than compacting them.
//
// The maths here is a direct transliteration of `culling.rs`, which carries
// the unit tests.

struct Instance {
    model: mat4x4<f32>,
    material_id: u32,
    vertex_offset: u32,
    index_offset: u32,
    _padding: u32,
}

struct CullAabb {
    min: vec4<f32>,
    max: vec4<f32>,
}

/// Must match `indirect::DrawIndirectArgs` byte-for-byte.
struct DrawArgs {
    vertex_count: u32,
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
}

struct CullParams {
    // left, right, bottom, top, near, far — each (nx, ny, nz, d), normalized.
    planes: array<vec4<f32>, 6>,
    draw_count: u32,
    disabled: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<storage, read>       instances: array<Instance>;
@group(0) @binding(1) var<storage, read>       aabbs:     array<CullAabb>;
@group(0) @binding(2) var<storage, read_write> draws:     array<DrawArgs>;
@group(0) @binding(3) var<uniform>             params:    CullParams;

@compute @workgroup_size(64)
fn cs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if i >= params.draw_count {
        return;
    }

    // Escape hatch: keep everything (used to A/B the culling result).
    if params.disabled != 0u {
        draws[i].instance_count = 1u;
        return;
    }

    let local_min = aabbs[i].min.xyz;
    let local_max = aabbs[i].max.xyz;

    // Empty/degenerate bounds (min > max) never draw.
    if local_min.x > local_max.x || local_min.y > local_max.y || local_min.z > local_max.z {
        draws[i].instance_count = 0u;
        return;
    }

    // Local AABB -> world AABB via centre/extent: the transformed extent is the
    // absolute-valued 3x3 basis applied to the local extent. Cheaper than
    // transforming all eight corners, and gives the same axis-aligned bound.
    let model = instances[i].model;
    let centre = (local_min + local_max) * 0.5;
    let extent = (local_max - local_min) * 0.5;

    let world_centre = (model * vec4<f32>(centre, 1.0)).xyz;
    let world_extent =
        abs(model[0].xyz) * extent.x +
        abs(model[1].xyz) * extent.y +
        abs(model[2].xyz) * extent.z;

    let world_min = world_centre - world_extent;
    let world_max = world_centre + world_extent;

    // Conservative test: cull only when the box is fully behind some plane.
    var visible = true;
    for (var p = 0u; p < 6u; p = p + 1u) {
        let plane = params.planes[p];
        let n = plane.xyz;
        // "Positive vertex" — the corner furthest along the plane normal.
        let pv = vec3<f32>(
            select(world_min.x, world_max.x, n.x >= 0.0),
            select(world_min.y, world_max.y, n.y >= 0.0),
            select(world_min.z, world_max.z, n.z >= 0.0),
        );
        if dot(n, pv) + plane.w < 0.0 {
            visible = false;
            break;
        }
    }

    draws[i].instance_count = select(0u, 1u, visible);
}
