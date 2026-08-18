// Phase 27-A (Styx) — instanced primitive-quad UI shader.
//
// Replaces the Phase 12B-1 sample-and-tint shader. One pipeline draws every UI
// shape: the vertex stage generates a unit quad from `vertex_index` and expands
// it by the instance's `expand` margin, and the fragment stage evaluates a
// signed distance field for a rounded box, deriving fill, gradient, border,
// shadow, glow and inset analytically from that single distance.
//
// Antialiasing is analytic (`fwidth` on the distance). There is no MSAA and no
// supersampled target. An axis-aligned rect on integer boundaries still
// resolves to exactly full coverage inside and zero outside, which is what lets
// the pre-Styx golden fills stay byte-identical.
//
// Colour contract (phase_26_Zeta §2.2, phase_27 §6.2): instance colours are
// authored sRGB with straight alpha. They are decoded to linear EXACTLY ONCE,
// here, before any gradient interpolation and before the blend. When the target
// is not an sRGB view the hardware performs no encode on write, so the decode
// is skipped instead of being applied twice.
//
// Group 0 binding 0: Globals (ortho projection + text gamma).
// Group 1 binding 0/1: texture2d + sampler — white 1x1 for solid shapes,
//   font/icon atlas for coverage masks (RGB = 255, A = coverage).

// UiPass replaces this declaration with `false` only for a non-sRGB surface.
const OUTPUT_IS_SRGB: bool = true;

const FLAG_TEXTURED: u32 = 1u;
const FLAG_TEXT:     u32 = 2u;
const FLAG_SHADOW:   u32 = 4u;
const FLAG_GLOW:     u32 = 8u;
const FLAG_INSET:    u32 = 16u;
const FLAG_GRADIENT: u32 = 32u;

struct Globals {
    proj: mat4x4<f32>,
    // Exponent applied to glyph coverage before blending (Phase 27-B). 1.0 is
    // an exact no-op and reproduces pre-Styx text.
    text_gamma: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0) var<uniform> globals: Globals;

@group(1) @binding(0) var t_font: texture_2d<f32>;
@group(1) @binding(1) var t_icon: texture_2d<f32>;
@group(1) @binding(2) var s_tex: sampler;

// Texture-layer selector, matching `primitive::TEX_SHIFT` / `TEX_MASK` and the
// historical texture_id constants: 0 = font atlas, 1 = icon atlas. Carrying it
// per instance is what lets one bind group serve the whole pass.
const TEX_SHIFT: u32 = 8u;
const TEX_MASK:  u32 = 255u;

fn sample_atlas(layer: u32, uv: vec2<f32>) -> vec4<f32> {
    if layer == 1u {
        return textureSample(t_icon, s_tex, uv);
    }
    return textureSample(t_font, s_tex, uv);
}

struct InstanceIn {
    @location(0)  rect:         vec4<f32>,
    @location(1)  uv:           vec4<f32>,
    @location(2)  radii:        vec4<f32>,
    @location(3)  shadow:       vec4<f32>,
    @location(4)  grad_axis:    vec2<f32>,
    @location(5)  border_width: f32,
    @location(6)  expand:       f32,
    @location(7)  fill_a:       vec4<f32>,
    @location(8)  fill_b:       vec4<f32>,
    @location(9)  border_color: vec4<f32>,
    @location(10) shadow_color: vec4<f32>,
    @location(11) flags:        u32,
}

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv:                              vec2<f32>,
    // Fragment position relative to the shape centre, in pixels.
    @location(1) local:                           vec2<f32>,
    @location(2) @interpolate(flat) half_extent:  vec2<f32>,
    @location(3) @interpolate(flat) radii:        vec4<f32>,
    @location(4) @interpolate(flat) shadow:       vec4<f32>,
    @location(5) @interpolate(flat) fill_a:       vec4<f32>,
    @location(6) @interpolate(flat) fill_b:       vec4<f32>,
    @location(7) @interpolate(flat) border_color: vec4<f32>,
    @location(8) @interpolate(flat) shadow_color: vec4<f32>,
    @location(9) @interpolate(flat) grad_axis:    vec2<f32>,
    @location(10) @interpolate(flat) border_width: f32,
    @location(11) @interpolate(flat) flags:       u32,
}

// IEC 61966-2-1 sRGB -> linear. Applied once, and only when the target view
// will re-encode on write.
fn decode_srgb(c: vec3<f32>) -> vec3<f32> {
    if !OUTPUT_IS_SRGB {
        return c;
    }
    let low  = c / vec3<f32>(12.92);
    let high = pow((c + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

// Radius of the corner the fragment belongs to. `r` is tl, tr, br, bl.
fn corner_radius(p: vec2<f32>, r: vec4<f32>) -> f32 {
    let right  = p.x > 0.0;
    let bottom = p.y > 0.0;
    let top_r = select(r.x, r.y, right);
    let bot_r = select(r.w, r.z, right);
    return select(top_r, bot_r, bottom);
}

// Signed distance to a rounded box centred at the origin (Inigo Quilez).
// Negative inside, positive outside, in pixels.
fn sd_rounded_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let rr = min(r, min(b.x, b.y));
    let q = abs(p) - b + vec2<f32>(rr, rr);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0, 0.0))) - rr;
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: InstanceIn) -> VertexOutput {
    // Two triangles over corners 0=TL 1=TR 2=BL 3=BR: (0,1,2) then (2,1,3).
    var c = vi;
    if vi == 3u {
        c = 2u;
    } else if vi == 4u {
        c = 1u;
    } else if vi == 5u {
        c = 3u;
    }
    let corner = vec2<f32>(f32(c & 1u), f32((c >> 1u) & 1u));

    let half_extent = inst.rect.zw * 0.5;
    let centre      = inst.rect.xy + half_extent;
    let grown       = half_extent + vec2<f32>(inst.expand, inst.expand);
    let local       = (corner * 2.0 - vec2<f32>(1.0, 1.0)) * grown;

    var out: VertexOutput;
    out.clip_pos     = globals.proj * vec4<f32>(centre + local, 0.0, 1.0);
    out.uv           = mix(inst.uv.xy, inst.uv.zw, corner);
    out.local        = local;
    out.half_extent  = half_extent;
    out.radii        = inst.radii;
    out.shadow       = inst.shadow;
    out.fill_a       = inst.fill_a;
    out.fill_b       = inst.fill_b;
    out.border_color = inst.border_color;
    out.shadow_color = inst.shadow_color;
    out.grad_axis    = inst.grad_axis;
    out.border_width = inst.border_width;
    out.flags        = inst.flags;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let flags  = in.flags;
    let radius = corner_radius(in.local, in.radii);
    let d      = sd_rounded_box(in.local, in.half_extent, radius);
    let aa     = max(fwidth(d) * 0.5, 0.0001);

    // ── Shadow, glow and inset instances ─────────────────────────────────────
    // These paint no fill; they exist purely to mark elevation. A glow is a
    // coloured outer halo through the same path — the pipeline is straight-alpha
    // blended, so it lightens by occlusion, not by addition.
    if (flags & FLAG_SHADOW) != 0u {
        let offset = in.shadow.xy;
        let blur   = in.shadow.z;
        let spread = in.shadow.w;
        var cov: f32;

        if (flags & FLAG_INSET) != 0u {
            // Darkening that hugs the inside of the edge nearest `offset`.
            let d_off  = sd_rounded_box(in.local - offset, in.half_extent, radius);
            let inside = 1.0 - smoothstep(-aa, aa, d);
            cov = inside * clamp(smoothstep(-max(blur, 0.5), 0.0, d_off), 0.0, 1.0);
        } else {
            let grown = in.half_extent + vec2<f32>(spread, spread);
            let d_sh  = sd_rounded_box(in.local - offset, grown, radius + spread);
            // smoothstep approximates the Gaussian falloff closely enough at
            // the 8-32 px blurs the elevation ladder asks for, and costs no
            // extra pass. Clamped so a zero blur still resolves to a hard edge.
            let soft = max(blur, aa);
            cov = 1.0 - smoothstep(-soft, soft, d_sh);
        }

        let rgb = decode_srgb(in.shadow_color.rgb);
        return vec4<f32>(rgb, in.shadow_color.a * clamp(cov, 0.0, 1.0));
    }

    // ── Fill ─────────────────────────────────────────────────────────────────
    // Decode happens here, once. Gradients interpolate on the decoded values,
    // so a 50 % stop is the linear-space mean and never the sRGB mean.
    var rgb: vec3<f32>;
    var alpha: f32;
    if (flags & FLAG_GRADIENT) != 0u {
        let denom = max(in.half_extent, vec2<f32>(0.0001, 0.0001));
        let n     = in.local / denom;
        let t     = clamp(dot(n, in.grad_axis) * 0.5 + 0.5, 0.0, 1.0);
        rgb   = mix(decode_srgb(in.fill_a.rgb), decode_srgb(in.fill_b.rgb), t);
        alpha = mix(in.fill_a.a, in.fill_b.a, t);
    } else {
        rgb   = decode_srgb(in.fill_a.rgb);
        alpha = in.fill_a.a;
    }

    // ── Texture ──────────────────────────────────────────────────────────────
    if (flags & FLAG_TEXTURED) != 0u {
        let tex = sample_atlas((flags >> TEX_SHIFT) & TEX_MASK, in.uv);
        if (flags & FLAG_TEXT) != 0u {
            // Glyph coverage. The hardware blends in linear on an sRGB target,
            // which renders light-on-dark stems heavier than the rasterizer
            // intended; the exponent thins them back. 1.0 is a no-op.
            alpha = alpha * pow(clamp(tex.a, 0.0, 1.0), globals.text_gamma);
        } else {
            rgb   = rgb * tex.rgb;
            alpha = alpha * tex.a;
        }
    }

    // ── Border ───────────────────────────────────────────────────────────────
    // An inset band whose outer edge is the shape edge, composited "over" the
    // fill in linear with straight alpha.
    if in.border_width > 0.0 {
        let half_w = in.border_width * 0.5;
        let d_band = abs(d + half_w) - half_w;
        let b_cov  = 1.0 - smoothstep(-aa, aa, d_band);
        let src_a  = in.border_color.a * clamp(b_cov, 0.0, 1.0);
        if src_a > 0.0 {
            let src_rgb = decode_srgb(in.border_color.rgb);
            let out_a   = src_a + alpha * (1.0 - src_a);
            if out_a > 0.0001 {
                rgb = (src_rgb * src_a + rgb * alpha * (1.0 - src_a)) / out_a;
            }
            alpha = out_a;
        }
    }

    // ── Shape coverage ───────────────────────────────────────────────────────
    // Textured quads are masked by their own alpha and must not be clipped a
    // second time by the box, or a glyph would lose its outermost row.
    if (flags & FLAG_TEXTURED) == 0u {
        alpha = alpha * (1.0 - smoothstep(-aa, aa, d));
    }

    return vec4<f32>(rgb, alpha);
}
