// Phase 11.5H: Editor infinite-grid overlay shader.
//
// Renders an anti-aliased XZ-plane grid via a full-screen triangle.
// Ray–XZ-plane intersection is reconstructed from the 208-byte view buffer.
// No depth testing — this is a pure overlay composited with alpha blending.
//
// View buffer layout (must match renderer.rs upload):
//   offset   0: view_proj     mat4x4<f32>  (64 bytes)
//   offset  64: inv_view_proj mat4x4<f32>  (64 bytes)
//   offset 128: view          mat4x4<f32>  (64 bytes)
//   offset 192: camera_pos    vec3<f32>    (12 bytes)
//   offset 204: cascade_debug f32          ( 4 bytes)

struct GridView {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    view:          mat4x4<f32>,
    camera_pos:    vec3<f32>,
    _pad:          f32,
}

@group(0) @binding(0) var<storage, read> gv: GridView;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0)       ndc:  vec2<f32>,
}

// Full-screen triangle — covers the viewport with 3 verts and no VBO.
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VOut {
    var xs = array<f32, 3>(-1.0,  3.0, -1.0);
    var ys = array<f32, 3>(-3.0,  1.0,  1.0);
    let p = vec2(xs[vid], ys[vid]);
    return VOut(vec4(p, 0.0, 1.0), p);
}

// Anti-aliased grid: returns 1.0 on line edges, 0.0 between lines.
// `cell` is the grid spacing in world units.
fn aa_grid(xz: vec2<f32>, cell: f32) -> f32 {
    let coord = xz / cell;
    let deriv = fwidth(coord);
    let g     = abs(fract(coord - 0.5) - 0.5) / deriv;
    return 1.0 - clamp(min(g.x, g.y), 0.0, 1.0);
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    // Reconstruct world-space near/far points from NDC via inv_view_proj.
    let np   = gv.inv_view_proj * vec4(in.ndc, 0.0, 1.0);
    let fp   = gv.inv_view_proj * vec4(in.ndc, 1.0, 1.0);
    let near = np.xyz / np.w;
    let far  = fp.xyz / fp.w;

    // Ray–XZ-plane (y = 0) intersection.
    let denom = far.y - near.y;
    if abs(denom) < 1e-5 { return vec4(0.0); }
    let t = -near.y / denom;
    if t < 0.0 { return vec4(0.0); }

    let hit = near + t * (far - near);

    // Fade with horizontal distance from camera (unaffected by camera height).
    let dist = length(hit.xz - gv.camera_pos.xz);
    let fade = 1.0 - smoothstep(50.0, 100.0, dist);

    // Grid levels.
    let minor = aa_grid(hit.xz, 1.0);
    let major = aa_grid(hit.xz, 10.0);

    // Axis highlights — derivative-scaled so they stay ~2 px wide at all distances.
    let x_axis = clamp(1.0 - abs(hit.z) / max(fwidth(hit.z) * 2.0, 0.001), 0.0, 1.0);
    let z_axis = clamp(1.0 - abs(hit.x) / max(fwidth(hit.x) * 2.0, 0.001), 0.0, 1.0);

    // Layer alphas.
    let m_a  = minor  * 0.35 * fade;
    let M_a  = major  * 0.65 * fade;
    let xa_a = x_axis * 0.90 * fade;
    let za_a = z_axis * 0.90 * fade;

    // Composite colors in priority order (higher-priority overrides lower).
    var col = vec3(0.0);
    col = mix(col, vec3(0.25), m_a);
    col = mix(col, vec3(0.50), M_a);
    col = mix(col, vec3(0.60, 0.10, 0.10), xa_a);   // X-axis: red   (z = 0 line)
    col = mix(col, vec3(0.10, 0.10, 0.60), za_a);   // Z-axis: blue  (x = 0 line)

    let alpha = max(max(m_a, M_a), max(xa_a, za_a));
    return vec4(col, alpha);
}
