// Instance batches (`instance_batch.rs`, open-world render scale, 18-19 Oct 2026).
//
// Every copy of every part of a batch is a slot that stays on the GPU between frames. Two entry points:
//
//   expand       every frame: each slot's indirect argument, written after the frame's own arguments. The cull pass
//                zeroes culled counts in place, so the arguments are put back each frame. When the slots or the
//                frame's layout changed, also each slot's instance (for every raster pass) and cull bounds.
//   shadow_cull  each frame a cascade is redrawn: the shadow-casting slots inside it, as that cascade's draws.

struct Slot {
    model: mat4x4<f32>,
    material_id: u32,
    vertex_offset: u32,
    index_offset: u32,
    // 0: a free slot, never drawn.
    index_count: u32,
    // Local bounds. `aabb_min.w`: drawn while nearer than this (horizontal metres, 0 = no limit); `aabb_max.w`:
    // drawn from this distance. The cull pass reads both from the bounds it is given.
    aabb_min: vec4<f32>,
    aabb_max: vec4<f32>,
    // x: casts while nearer than this (0 = no limit); y: casts at all (0 or 1).
    shadow: vec4<f32>,
}

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
    cone: vec4<f32>,
}

struct DrawArgs {
    vertex_count: u32,
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
}

struct ExpandParams {
    slots: u32,
    // Slot j's instance is `instance_base + j`, its argument and bounds `arg_base + j`.
    instance_base: u32,
    arg_base: u32,
    write_static: u32,
}

struct ShadowParams {
    // Cascade c's six planes are 6c..6c+5, each (nx, ny, nz, d) as `culling::frustum_planes` writes them.
    planes: array<vec4<f32>, 24>,
    camera: vec4<f32>,
    slots: u32,
    // Each cascade's region of `shadow_args`, in arguments.
    capacity: u32,
    // Bit c: cascade c is redrawn this frame.
    dirty: u32,
    // 1: survivors appended and counted; 0: one argument per slot with a 0 or 1 count.
    compact: u32,
    instance_base: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<storage, read> slots: array<Slot>;
@group(0) @binding(1) var<uniform> expand_params: ExpandParams;
@group(0) @binding(2) var<storage, read_write> instances: array<Instance>;
@group(0) @binding(3) var<storage, read_write> args: array<DrawArgs>;
@group(0) @binding(4) var<storage, read_write> aabbs: array<CullAabb>;
@group(0) @binding(5) var<uniform> shadow_params: ShadowParams;
@group(0) @binding(6) var<storage, read_write> shadow_args: array<DrawArgs>;
@group(0) @binding(7) var<storage, read_write> shadow_counts: array<atomic<u32>, 4>;

@compute @workgroup_size(64)
fn expand(@builtin(global_invocation_id) gid: vec3<u32>) {
    let j = gid.x;
    if j >= expand_params.slots {
        return;
    }
    let s = slots[j];
    let a = expand_params.arg_base + j;
    if s.index_count == 0u {
        args[a] = DrawArgs(0u, 0u, 0u, 0u);
        if expand_params.write_static != 0u {
            // Inverted bounds: the cull pass never draws them.
            aabbs[a] = CullAabb(vec4<f32>(1.0, 1.0, 1.0, 0.0), vec4<f32>(-1.0, -1.0, -1.0, 0.0),
                                vec4<f32>(0.0, 0.0, 0.0, 2.0));
        }
        return;
    }
    let i = expand_params.instance_base + j;
    args[a] = DrawArgs(s.index_count, 1u, 0u, i);
    if expand_params.write_static != 0u {
        instances[i] = Instance(s.model, s.material_id, s.vertex_offset, s.index_offset, 0u);
        // Cone test off (w = 2): batches draw two-sided.
        aabbs[a] = CullAabb(s.aabb_min, s.aabb_max, vec4<f32>(0.0, 0.0, 0.0, 2.0));
    }
}

@compute @workgroup_size(64)
fn shadow_cull(@builtin(global_invocation_id) gid: vec3<u32>) {
    let j = gid.x;
    if j >= shadow_params.slots {
        return;
    }
    let s = slots[j];
    var casts = s.index_count != 0u && s.shadow.y != 0.0;
    if casts && s.shadow.x > 0.0 {
        // Horizontal, like the draw distance: climbing a hill does not switch the shadows below back on.
        let d = s.model[3].xz - shadow_params.camera.xz;
        casts = dot(d, d) <= s.shadow.x * s.shadow.x;
    }
    let centre = (s.aabb_min.xyz + s.aabb_max.xyz) * 0.5;
    let extent = (s.aabb_max.xyz - s.aabb_min.xyz) * 0.5;
    let world_centre = (s.model * vec4<f32>(centre, 1.0)).xyz;
    let world_extent = abs(s.model[0].xyz) * extent.x + abs(s.model[1].xyz) * extent.y + abs(s.model[2].xyz) * extent.z;
    let world_min = world_centre - world_extent;
    let world_max = world_centre + world_extent;
    let arg = DrawArgs(s.index_count, 1u, 0u, shadow_params.instance_base + j);
    for (var c = 0u; c < 4u; c = c + 1u) {
        if ((shadow_params.dirty >> c) & 1u) == 0u {
            continue;
        }
        var inside = casts;
        if inside {
            // The same test as `culling::aabb_in_frustum`, which the submitted casters pass through.
            for (var p = 0u; p < 6u; p = p + 1u) {
                let plane = shadow_params.planes[c * 6u + p];
                let n = plane.xyz;
                let pv = vec3<f32>(
                    select(world_min.x, world_max.x, n.x >= 0.0),
                    select(world_min.y, world_max.y, n.y >= 0.0),
                    select(world_min.z, world_max.z, n.z >= 0.0),
                );
                if dot(n, pv) + plane.w < 0.0 {
                    inside = false;
                    break;
                }
            }
        }
        let region = c * shadow_params.capacity;
        if shadow_params.compact != 0u {
            if inside {
                shadow_args[region + atomicAdd(&shadow_counts[c], 1u)] = arg;
            }
        } else if inside {
            shadow_args[region + j] = arg;
        } else {
            shadow_args[region + j] = DrawArgs(0u, 0u, 0u, 0u);
        }
    }
}
