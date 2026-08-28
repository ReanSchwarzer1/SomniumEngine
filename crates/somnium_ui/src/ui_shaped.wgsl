// MORROWIND-D — the shaped UI pipeline.
//
// Companion to `ui_pass.wgsl`, **not a replacement**. The quad pipeline draws
// every axis-aligned rounded box analytically from one 100-byte instance and is
// frozen; this one draws tessellated geometry — strokes, paths, arcs, rotated
// and skewed shapes — from a per-vertex buffer plus a per-shape storage array.
//
// Both run in the same render pass, against the same target, interleaved in
// paint order. `DrawingContext` keeps one ordered command list and each command
// names its stream, so `draw_over` survives having two pipelines.
//
// Colour contract (phase_26_Zeta S2.2, phase_27 S6.2), inherited unchanged:
// instance colours are authored sRGB with straight alpha, decoded to linear
// EXACTLY ONCE, here, before any gradient interpolation and before the blend.
// A gradient interpolated in sRGB and then decoded is a different colour from
// one decoded and then interpolated, and the second is the correct one.

enable wgpu_binding_array;

// UiPass replaces this declaration with `false` only for a non-sRGB surface.
const OUTPUT_IS_SRGB: bool = true;

const GRAD_LINEAR:  u32 = 1u;
const GRAD_RADIAL:  u32 = 2u;
const GRAD_ANGULAR: u32 = 4u;
const TEXTURED:     u32 = 8u;
const COVERAGE:     u32 = 16u;

const NO_TEXTURE: u32 = 0xffffffffu;
const NO_MASK:    u32 = 0xffffffffu;

struct Globals {
    proj: mat4x4<f32>,
    text_gamma: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

// Mirrors `shaped::ShapedInstance`. 64 bytes; the trailing `_pad` is explicit
// on both sides so the storage stride cannot be inferred differently by two
// compilers. A mismatch here decodes every instance after the first from the
// wrong offset, which renders as "everything after the first shape is garbage".
struct ShapedInstance {
    // A 2x3 affine cannot be a mat2x3 here: WGSL aligns a mat3x2/mat2x3 column
    // to 8 bytes and pads, so the Rust `[f32; 6]` would not line up. Six scalars
    // are unambiguous.
    xform_a: f32,
    xform_b: f32,
    xform_c: f32,
    xform_d: f32,
    xform_tx: f32,
    xform_ty: f32,
    // Four scalars, not a `vec4<f32>`, for the same reason the affine is six
    // scalars: WGSL aligns a vec4 to 16 bytes and Rust aligns `[f32; 4]` to 4,
    // so the vec4 form put `grad` at offset 32 here and 24 there and made the
    // struct 80 bytes against 64. `tests/shaders_validate.rs` caught it; the
    // scalar form has no alignment of its own to disagree about.
    grad_x: f32,
    grad_y: f32,
    grad_z: f32,
    grad_w: f32,
    fill_a: u32,   // packed RGBA8, authored sRGB
    fill_b: u32,
    texture: u32,
    mask: u32,
    flags: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> globals: Globals;

@group(1) @binding(2) var s_tex: sampler;
// The bindless array MORROWIND-D replaces the three fixed bindings with.
// Slots 0/1/2 are the font, icon and thumbnail atlases, so the existing bind
// group's *semantics* survive the change; 3.. are registered by games.
@group(1) @binding(4) var ui_textures: binding_array<texture_2d<f32>, 64>;
@group(2) @binding(0) var<storage, read> shapes: array<ShapedInstance>;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) instance: u32,
};

@vertex
fn vs_shaped(
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) instance: u32,
) -> VsOut {
    let s = shapes[instance];
    // Row-major 2x3: (a*x + c*y + tx, b*x + d*y + ty).
    let world = vec2<f32>(
        s.xform_a * pos.x + s.xform_c * pos.y + s.xform_tx,
        s.xform_b * pos.x + s.xform_d * pos.y + s.xform_ty,
    );
    var out: VsOut;
    out.clip = globals.proj * vec4<f32>(world, 0.0, 1.0);
    // Local, not world: gradients and masks are authored in the shape's own
    // frame, so a rotated widget's gradient rotates with it rather than staying
    // pinned to the screen.
    out.local = pos;
    out.uv = uv;
    out.instance = instance;
    return out;
}

fn unpack_srgb(packed: u32) -> vec4<f32> {
    return vec4<f32>(
        f32((packed >> 0u) & 255u),
        f32((packed >> 8u) & 255u),
        f32((packed >> 16u) & 255u),
        f32((packed >> 24u) & 255u),
    ) / 255.0;
}

// The exact sRGB transfer function, not the 2.2 approximation. Phase 27 uses
// this one, and two decodes that disagree by a fraction of a level are visible
// as a seam where a shaped shape meets a quad one.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let cutoff = c <= vec3<f32>(0.04045);
    let low = c / 12.92;
    let high = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, cutoff);
}

fn decode(packed: u32) -> vec4<f32> {
    let raw = unpack_srgb(packed);
    if OUTPUT_IS_SRGB {
        return vec4<f32>(srgb_to_linear(raw.rgb), raw.a);
    }
    // A non-sRGB view performs no encode on write, so decoding here would apply
    // the transfer twice.
    return raw;
}

/// Gradient parameter in 0..1, by kind. Flat fill returns 0 and never reads
/// `fill_b`.
fn gradient_t(s: ShapedInstance, local: vec2<f32>) -> f32 {
    if (s.flags & GRAD_LINEAR) != 0u {
        // `grad.xy` is an axis in local space; the projection is normalised by
        // the axis length, so authoring a longer axis spreads the ramp.
        let axis = vec2<f32>(s.grad_x, s.grad_y);
        let len_sq = dot(axis, axis);
        if len_sq < 1e-8 {
            return 0.0;
        }
        return clamp(dot(local, axis) / len_sq, 0.0, 1.0);
    }
    if (s.flags & GRAD_RADIAL) != 0u {
        // `grad.z` is clamped above zero on the Rust side, so this cannot
        // divide by zero.
        return clamp(distance(local, vec2<f32>(s.grad_x, s.grad_y)) / s.grad_z, 0.0, 1.0);
    }
    if (s.flags & GRAD_ANGULAR) != 0u {
        let d = local - vec2<f32>(s.grad_x, s.grad_y);
        // `atan2(0, 0)` is undefined; a point exactly at the centre of a conic
        // gradient is a real case (the centre pixel of a radial menu).
        if dot(d, d) < 1e-12 {
            return 0.0;
        }
        let angle = atan2(d.y, d.x) - s.grad_z;
        return fract(angle / 6.2831853 + 1.0);
    }
    return 0.0;
}

@fragment
fn fs_shaped(in: VsOut) -> @location(0) vec4<f32> {
    let s = shapes[in.instance];

    let a = decode(s.fill_a);
    var color: vec4<f32>;
    if (s.flags & (GRAD_LINEAR | GRAD_RADIAL | GRAD_ANGULAR)) != 0u {
        let b = decode(s.fill_b);
        color = mix(a, b, gradient_t(s, in.local));
    } else {
        color = a;
    }

    if (s.flags & TEXTURED) != 0u && s.texture != NO_TEXTURE {
        let sampled = textureSample(ui_textures[s.texture], s_tex, in.uv);
        if (s.flags & COVERAGE) != 0u {
            // The glyph/icon contract: RGB is 255 and alpha is coverage, so the
            // authored colour survives and only alpha is modulated. Gamma
            // matches the quad pipeline's text path exactly — a shaped label
            // and a quad label at the same size must weigh the same.
            color.a *= pow(sampled.a, globals.text_gamma);
        } else {
            color *= sampled;
        }
    }

    if s.mask != NO_MASK {
        // Red channel, not alpha: a mask authored as a coverage texture is
        // single-channel, and R is what a single-channel upload lands in.
        color.a *= textureSample(ui_textures[s.mask], s_tex, in.uv).r;
    }

    // Straight (unassociated) alpha, matching `ui_pass.wgsl` exactly. Both
    // pipelines share one blend descriptor -- SrcAlpha / OneMinusSrcAlpha --
    // which is what lets them interleave in a single pass. Premultiplying here
    // would double the alpha term and make every shaped shape darker than the
    // quad shape beside it, which is exactly the kind of difference that looks
    // like a colour-space bug and is not one.
    return color;
}
