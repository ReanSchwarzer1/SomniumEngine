// Phase 15B: GPU instance frustum culling.
// Phase 15E2: plus two-phase Hi-Z occlusion culling.
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
    // Phase 15F normal cone: local-space axis in xyz, backface threshold in w.
    // w = 2.0 disables the test, which whole-mesh draws use.
    cone: vec4<f32>,
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
    // 0 = phase one, 1 = phase two. See the note above `cs_main`.
    phase: u32,
    occlusion_enabled: u32,
    view_proj: mat4x4<f32>,
    hiz_size: vec2<f32>,
    hiz_mip_count: u32,
    _pad: u32,
    camera_pos: vec4<f32>,
}

@group(0) @binding(0) var<storage, read>       instances: array<Instance>;
@group(0) @binding(1) var<storage, read>       aabbs:     array<CullAabb>;
@group(0) @binding(2) var<storage, read_write> draws:     array<DrawArgs>;
@group(0) @binding(3) var<uniform>             params:    CullParams;
@group(0) @binding(4) var                      hiz:       texture_2d<f32>;
/// Per-draw record of what phase one rejected *on occlusion*. Frustum rejects
/// are not recorded: they are still off-screen in phase two, and resurrecting
/// them would draw geometry outside the view.
@group(0) @binding(5) var<storage, read_write> occluded_flags: array<u32>;

/// Is this world AABB hidden behind what the pyramid already records?
///
/// A transliteration of `is_occluded`/`project_aabb_to_screen`/`hiz_mip_level`
/// in `culling.rs`, which carry the unit tests. Every ambiguous case answers
/// "not occluded": a wrong `false` costs one wasted draw, a wrong `true`
/// deletes geometry from the image.
fn is_occluded(world_min: vec3<f32>, world_max: vec3<f32>) -> bool {
    var lo = vec2<f32>(1e30);
    var hi = vec2<f32>(-1e30);
    var min_z = 1e30;

    for (var c = 0u; c < 8u; c = c + 1u) {
        let corner = vec3<f32>(
            select(world_min.x, world_max.x, (c & 1u) != 0u),
            select(world_min.y, world_max.y, (c & 2u) != 0u),
            select(world_min.z, world_max.z, (c & 4u) != 0u),
        );
        let clip = params.view_proj * vec4<f32>(corner, 1.0);
        // At or behind the eye the perspective divide is meaningless, so the
        // box is treated as visible rather than guessed at.
        if clip.w <= 1e-6 {
            return false;
        }
        let ndc = clip.xyz / clip.w;
        // Screen V runs downward, hence the flip.
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 1.0 - (ndc.y * 0.5 + 0.5));
        lo = min(lo, uv);
        hi = max(hi, uv);
        min_z = min(min_z, ndc.z);
    }

    lo = clamp(lo, vec2<f32>(0.0), vec2<f32>(1.0));
    hi = clamp(hi, vec2<f32>(0.0), vec2<f32>(1.0));
    min_z = max(min_z, 0.0);

    // Pick the level where the footprint spans at most 2x2 texels, so four
    // samples cover any candidate however large it is on screen.
    let extent = max(max((hi.x - lo.x) * params.hiz_size.x,
                         (hi.y - lo.y) * params.hiz_size.y), 1.0);
    let level = clamp(i32(ceil(log2(extent))) - 1, 0, i32(params.hiz_mip_count) - 1);

    let dims = vec2<f32>(textureDimensions(hiz, level));
    let t0 = vec2<i32>(clamp(lo * dims, vec2<f32>(0.0), dims - vec2<f32>(1.0)));
    let t1 = vec2<i32>(clamp(hi * dims, vec2<f32>(0.0), dims - vec2<f32>(1.0)));

    var furthest = 0.0;
    furthest = max(furthest, textureLoad(hiz, vec2<i32>(t0.x, t0.y), level).r);
    furthest = max(furthest, textureLoad(hiz, vec2<i32>(t1.x, t0.y), level).r);
    furthest = max(furthest, textureLoad(hiz, vec2<i32>(t0.x, t1.y), level).r);
    furthest = max(furthest, textureLoad(hiz, vec2<i32>(t1.x, t1.y), level).r);

    // A region still at the far plane records no occluder at all.
    if furthest >= 1.0 {
        return false;
    }
    return min_z > furthest;
}

// Both phases run this entry point; `params.phase` selects the behaviour.
//
// Phase one tests frustum then occlusion against the pyramid left over from the
// previous frame, and records what it rejected on occlusion. Phase two re-tests
// exactly that set against the pyramid rebuilt from phase one's depth, which is
// what catches geometry that became visible this frame. Anything phase two is
// not re-testing has its instance count zeroed, or phase one's draws would be
// submitted a second time.
@compute @workgroup_size(64)
fn cs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if i >= params.draw_count {
        return;
    }

    // Escape hatch: keep everything (used to A/B the culling result).
    if params.disabled != 0u {
        // Only phase one draws when culling is off, so phase two must not
        // resubmit the same geometry.
        draws[i].instance_count = select(1u, 0u, params.phase == 1u);
        occluded_flags[i] = 0u;
        return;
    }

    if params.phase == 1u {
        if occluded_flags[i] == 0u {
            draws[i].instance_count = 0u;
            return;
        }
        occluded_flags[i] = 0u;
    }

    let local_min = aabbs[i].min.xyz;
    let local_max = aabbs[i].max.xyz;

    // Empty/degenerate bounds (min > max) never draw.
    if local_min.x > local_max.x || local_min.y > local_max.y || local_min.z > local_max.z {
        draws[i].instance_count = 0u;
        if params.phase == 0u {
            occluded_flags[i] = 0u;
        }
        return;
    }

    // Local AABB -> world AABB via centre/extent: the transformed extent is the
    // absolute-valued 3x3 basis applied to the local extent. Cheaper than
    // transforming all eight corners, and gives the same axis-aligned bound.
    // Phase 15F: a draw is now one CLUSTER, so the draw index is no longer the
    // instance index — several draws share an instance. The instance to shade
    // with is already in the indirect argument, which is also what the vertex
    // shader reads through `@builtin(instance_index)`.
    let model = instances[draws[i].first_instance].model;
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

    if !visible {
        draws[i].instance_count = 0u;
        if params.phase == 0u {
            occluded_flags[i] = 0u;
        }
        return;
    }

    // Phase 15F: reject the whole cluster when every triangle in it faces away.
    // Valid only because the visibility pass culls back faces; if it ever draws
    // them, this has to go. `cone.w = 2.0` is unreachable by a dot product, so
    // whole-mesh draws fall through without a branch.
    if aabbs[i].cone.w <= 1.0 {
        // A mirroring transform (negative determinant) flips which side is
        // front, so the stored axis would point the wrong way. Cheaper to skip
        // the test than to get it backwards and delete visible geometry.
        let det = dot(model[0].xyz, cross(model[1].xyz, model[2].xyz));
        if det > 0.0 {
            let axis_ws = normalize((model * vec4<f32>(aabbs[i].cone.xyz, 0.0)).xyz);
            let to_centre = world_centre - params.camera_pos.xyz;
            let dist = length(to_centre);
            // Widened by the bounding radius so a cluster is only rejected when
            // it is backfacing from every point the camera could be seeing it.
            let radius = length(world_extent);
            if dot(to_centre, axis_ws) >= aabbs[i].cone.w * dist + radius {
                draws[i].instance_count = 0u;
                if params.phase == 0u {
                    occluded_flags[i] = 0u;
                }
                return;
            }
        }
    }

    var hidden = false;
    if params.occlusion_enabled != 0u {
        hidden = is_occluded(world_min, world_max);
    }

    draws[i].instance_count = select(1u, 0u, hidden);
    if params.phase == 0u {
        // Remember it for phase two, which re-tests against a fresher pyramid.
        occluded_flags[i] = select(0u, 1u, hidden);
    }
}
