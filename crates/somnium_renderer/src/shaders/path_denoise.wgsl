// The path tracer's denoiser (17 Oct 2026; path_tracer.wgsl writes one noisy path a pixel a frame).
//
// 1. `temporal`: last frame's accumulated light, reprojected through the motion vectors (velocity pass) and checked
//    tap by tap against the surface it lands on (normal and distance), blended with this frame's; a pixel newly in
//    view starts over. Up to `max_history` frames, so a still view converges and a moving one keeps its detail.
// 2. `atrous`: an edge-stopping a-trous filter (Dammertz 2010, as SVGF uses it), a 5 x 5 B3 kernel at growing steps,
//    weighed by distance, normal and light; it filters harder where the history is short.
// 3. `remodulate`: the albedo multiplied back in (with the material's occlusion and the fog's transmittance, mode 0).
// 4. `composite_vs`/`composite_fs` (mode 0): that added over the raster's picture, which holds the direct light, the
//    glow, the sky and the fog's own light but no ambient while the path tracer runs (shading.wgsl).
// The light is demodulated (divided by the surface's albedo) all through, so texture stays sharp.
//
// Reflections (`_pad.y` is 1 + the roughness limit; path_tracer.wgsl mode 1): the albedo is white with the surface's
// roughness in its alpha;
// the a-trous steps skip a surface too smooth for their spread (a mirror is not filtered at all), and the output's
// alpha fades out toward the tracer's roughness limit, where shading's environment specular takes over.

struct DnParams {
    inv_res: vec2<f32>,
    step: u32,
    reset: u32,
    max_history: f32,
    sigma_light: f32,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> dn: DnParams;
@group(0) @binding(1) var lin_sampler: sampler;
@group(0) @binding(2) var input_tex: texture_2d<f32>;
@group(0) @binding(3) var gbuf: texture_2d<f32>;
@group(0) @binding(4) var gbuf_prev: texture_2d<f32>;
@group(0) @binding(5) var history: texture_2d<f32>;
@group(0) @binding(6) var velocity: texture_2d<f32>;
@group(0) @binding(7) var albedo: texture_2d<f32>;
@group(0) @binding(9) var out_tex: texture_storage_2d<rgba16float, write>;

fn dn_luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

/// Whether last frame's surface at `q` is the same one as this frame's (`g`).
fn dn_same(g: vec4<f32>, q: vec2<i32>, dims: vec2<i32>) -> bool {
    if any(q < vec2<i32>(0)) || any(q >= dims) {
        return false;
    }
    let p = textureLoad(gbuf_prev, q, 0);
    if p.w <= 0.0 {
        return false;
    }
    return dot(p.xyz, g.xyz) > 0.85 && abs(p.w - g.w) < 0.06 * g.w + 0.05;
}

@compute @workgroup_size(8, 8, 1)
fn temporal(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<i32>(textureDimensions(out_tex));
    let p = vec2<i32>(gid.xy);
    if any(p >= dims) {
        return;
    }
    let cur = textureLoad(input_tex, p, 0);
    let g = textureLoad(gbuf, p, 0);
    if cur.a < 0.5 || g.w <= 0.0 {
        textureStore(out_tex, p, vec4<f32>(0.0));
        return;
    }
    var hist = vec3<f32>(0.0);
    var len = 0.0;
    if dn.reset == 0u {
        let uv = (vec2<f32>(p) + 0.5) * dn.inv_res;
        let vel_dims = vec2<f32>(textureDimensions(velocity));
        let vel = textureLoad(velocity, vec2<i32>(uv * vel_dims), 0).xy;
        let prev = (uv + vel) / dn.inv_res - 0.5;
        let base = vec2<i32>(floor(prev));
        let f = prev - floor(prev);
        var total = 0.0;
        var sum = vec4<f32>(0.0);
        for (var k = 0; k < 4; k++) {
            let o = vec2<i32>(k & 1, k >> 1u);
            let q = base + o;
            let w = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
            if w > 1e-3 && dn_same(g, q, dims) {
                sum += textureLoad(history, q, 0) * w;
                total += w;
            }
        }
        if total > 0.05 {
            let h = sum / total;
            if all(h == h) {
                hist = h.rgb;
                len = h.a;
            }
        }
    }
    // A sample far brighter than this pixel's history is a firefly (one path that found a lamp through a gap): held to
    // six times the history once there is some. A light switched on still comes up within a few frames.
    var fresh = select(vec3<f32>(0.0), cur.rgb, cur.rgb == cur.rgb);
    let cap = max(dn_luma(hist) * 6.0, 1e-4);
    if len >= 3.0 && dn_luma(fresh) > cap {
        fresh *= cap / dn_luma(fresh);
    }
    len = min(len + 1.0, dn.max_history);
    let blended = mix(hist, fresh, 1.0 / len);
    textureStore(out_tex, p, vec4<f32>(blended, len));
}

@compute @workgroup_size(8, 8, 1)
fn atrous(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<i32>(textureDimensions(out_tex));
    let p = vec2<i32>(gid.xy);
    if any(p >= dims) {
        return;
    }
    let c = textureLoad(input_tex, p, 0);
    let g = textureLoad(gbuf, p, 0);
    if g.w <= 0.0 || (dn._pad.y > 0.5 && textureLoad(albedo, p, 0).a * 16.0 < f32(dn.step)) {
        textureStore(out_tex, p, c);
        return;
    }
    let len = max(textureLoad(history, p, 0).a, 1.0);
    let lc = dn_luma(c.rgb);
    let sigma = dn.sigma_light * (0.05 + lc) * (1.0 + 6.0 / sqrt(len));
    let kernel = array<f32, 5>(0.0625, 0.25, 0.375, 0.25, 0.0625);
    let step = i32(dn.step);
    var sum = vec3<f32>(0.0);
    var weights = 0.0;
    for (var dy = -2; dy <= 2; dy++) {
        for (var dx = -2; dx <= 2; dx++) {
            let q = clamp(p + vec2<i32>(dx, dy) * step, vec2<i32>(0), dims - vec2<i32>(1));
            let gq = textureLoad(gbuf, q, 0);
            if gq.w <= 0.0 {
                continue;
            }
            let cq = textureLoad(input_tex, q, 0);
            let wz = exp(-abs(gq.w - g.w) / (0.03 * g.w * f32(step) + 0.01));
            let wn = pow(max(dot(gq.xyz, g.xyz), 0.0), 32.0);
            let wl = exp(-abs(dn_luma(cq.rgb) - lc) / sigma);
            let w = kernel[dx + 2] * kernel[dy + 2] * wz * wn * wl;
            if !(w == w) || !all(cq.rgb == cq.rgb) {
                continue;
            }
            sum += cq.rgb * w;
            weights += w;
        }
    }
    textureStore(out_tex, p, vec4<f32>(sum / max(weights, 1e-6), c.a));
}

@compute @workgroup_size(8, 8, 1)
fn remodulate(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<i32>(textureDimensions(out_tex));
    let p = vec2<i32>(gid.xy);
    if any(p >= dims) {
        return;
    }
    let g = textureLoad(gbuf, p, 0);
    var color = vec3<f32>(0.0);
    if g.w > 0.0 {
        color = textureLoad(input_tex, p, 0).rgb * textureLoad(albedo, p, 0).rgb;
    }
    // SOMNIUM_PATH_DEBUG (path_tracer.rs): 1 magenta (is the output composited at all), 2 what the light is
    // multiplied back by, 3 the noisy light before any filtering (history holds it).
    let debug = u32(dn._pad.x);
    if debug == 1u {
        color = vec3<f32>(1.0, 0.0, 1.0) * 1000.0;
    } else if debug == 2u {
        color = textureLoad(albedo, p, 0).rgb;
    } else if debug == 3u {
        color = textureLoad(history, p, 0).rgb * textureLoad(albedo, p, 0).rgb;
    }
    var alpha = 1.0;
    if dn._pad.y > 0.5 {
        let limit = max(dn._pad.y - 1.0, 0.05);
        alpha = select(0.0, 1.0 - smoothstep(limit * 0.7, limit, textureLoad(albedo, p, 0).a), g.w > 0.0);
    }
    textureStore(out_tex, p, vec4<f32>(select(vec3<f32>(0.0), color, color == color), alpha));
}

// The composite (mode 0), a full-screen triangle over the scene's HDR target with additive blending (path_tracer.rs
// `composite`). Its layout binds only `input_tex`, the remodulated output, at the target's own size.
@vertex
fn composite_vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn composite_fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(textureLoad(input_tex, vec2<i32>(position.xy), 0).rgb, 0.0);
}
