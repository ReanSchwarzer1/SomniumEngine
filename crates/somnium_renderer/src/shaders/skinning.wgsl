// Somnium Engine — GPU skinning (Phase MORROWIND, MORROWIND-U).
//
// **Skin-to-buffer.** A compute pass reads a mesh's rest vertices and its skin
// bindings, applies the character's palette, and writes posed vertices into a
// transient slice of the *same* `GeometryPool` every static mesh lives in.
//
// The alternative — skinning inside the visibility pass's vertex stage — was
// not taken, and the reason is downstream rather than here. Somnium's pipeline
// assumes geometry is static in three separate places: `meshlet.rs` precomputes
// bounds, `cull.wgsl` tests them, and **ray tracing reads positions straight
// out of the shared pool** (`geometry.rs:122`). Skinning in the vertex stage
// leaves all three reading unposed data, so it needs conservative meshlet
// bounds *and* a BLAS rebuild anyway — which is most of skin-to-buffer's cost
// without its property that everything downstream keeps working unchanged.
//
// The cost that buys: one posed vertex buffer per skinned instance, and one
// read-modify-write of it per frame. See `skinning.rs` for the measured number.

//!include "global_pool.wgsl"

// One 4x4 per joint per skinned instance, packed end to end. `base` in
// `SkinInstance` is where this instance's joints start.
@group(1) @binding(0) var<storage, read> palettes: array<mat4x4<f32>>;

// Per-vertex binding, parallel to the *rest* vertices. Joints are packed two
// per u32 because a joint index is u16 — the whole point of MORROWIND-U's
// `JointIndex` being u16 rather than u32 is that this array is per-vertex and
// halving it is 8 bytes a vertex on every character in the scene.
struct SkinVertex {
    joints_01: u32,
    joints_23: u32,
    weights_01: u32,   // two f16
    weights_23: u32,
}
@group(1) @binding(1) var<storage, read> skin_vertices: array<SkinVertex>;

// What to skin, and where to put it.
struct SkinInstance {
    // Where this instance's rest vertices start, in `vertices`.
    rest_offset: u32,
    // Where its posed vertices go, in the same buffer. Reserved once at bind.
    posed_offset: u32,
    vertex_count: u32,
    // Where this instance's joints start, in `palettes`.
    palette_base: u32,
}
@group(1) @binding(2) var<storage, read> instances_in: array<SkinInstance>;

// The pool, read and written. Binding the same buffer twice — once read-only
// for the rest vertices and once read-write for the posed span — is what makes
// this pass a no-op for every consumer downstream: they keep reading the one
// buffer they always read.
@group(1) @binding(3) var<storage, read_write> pool: array<Vertex>;

fn unpack_joints(lo: u32, hi: u32) -> vec4<u32> {
    return vec4<u32>(lo & 0xffffu, lo >> 16u, hi & 0xffffu, hi >> 16u);
}

fn unpack_weights(lo: u32, hi: u32) -> vec4<f32> {
    let a = unpack2x16float(lo);
    let b = unpack2x16float(hi);
    return vec4<f32>(a.x, a.y, b.x, b.y);
}

// The join, in one function. Called from the compute entry point below; if a
// later sub-phase ever does want skin-in-shader, this is the function it calls
// from the vertex stage instead, which is why it takes a base rather than
// reaching for a global.
fn skin_matrix(joints: vec4<u32>, weights: vec4<f32>, base: u32) -> mat4x4<f32> {
    var m = palettes[base + joints.x] * weights.x;
    m += palettes[base + joints.y] * weights.y;
    m += palettes[base + joints.z] * weights.z;
    m += palettes[base + joints.w] * weights.w;
    return m;
}

@compute @workgroup_size(64)
fn skin(@builtin(global_invocation_id) gid: vec3<u32>) {
    let instance_index = gid.y;
    if (instance_index >= arrayLength(&instances_in)) {
        return;
    }
    let inst = instances_in[instance_index];
    let vertex = gid.x;
    if (vertex >= inst.vertex_count) {
        return;
    }

    let src = inst.rest_offset + vertex;
    let dst = inst.posed_offset + vertex;
    let rest = pool[src];
    let binding = skin_vertices[src];

    let m = skin_matrix(
        unpack_joints(binding.joints_01, binding.joints_23),
        unpack_weights(binding.weights_01, binding.weights_23),
        inst.palette_base,
    );

    let position = m * vec4<f32>(rest.pos_x, rest.pos_y, rest.pos_z, 1.0);

    // Normals want the inverse transpose. This uses the skinning matrix's own
    // upper 3x3 instead, which is exact for rigid joints (rotation plus
    // translation) and wrong only in proportion to non-uniform scale — and a
    // non-uniformly scaled joint is a thing riggers avoid precisely because it
    // breaks normals everywhere. Every shipping engine makes this trade; it is
    // written down here so a reader does not have to wonder whether it was an
    // oversight.
    let n = normalize(
        (mat3x3<f32>(m[0].xyz, m[1].xyz, m[2].xyz)
            * vec3<f32>(rest.norm_x, rest.norm_y, rest.norm_z))
    );

    var posed = rest;
    posed.pos_x = position.x;
    posed.pos_y = position.y;
    posed.pos_z = position.z;
    posed.norm_x = n.x;
    posed.norm_y = n.y;
    posed.norm_z = n.z;
    // UVs are untouched: skinning moves a vertex, it does not re-parameterise
    // the surface.
    pool[dst] = posed;
}
