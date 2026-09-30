// Somnium Engine — Phase CONTROL-M: compositing the quarter-res cloud buffer
// over the HDR scene.
//
// ## Why this is a blend rather than a resolve
//
// The march writes premultiplied inscatter in RGB and transmittance in A, so
// the composite is exactly
//
//     hdr = cloud_scatter + hdr * cloud_transmittance
//
// which is a fixed-function blend of `One` and `SrcAlpha`. There is no read of
// the destination, no second full-resolution buffer, and no place for the
// composite to disagree with the march about what the alpha channel means.
//
// ## Why the upsample is a bilateral one
//
// A straight bilinear stretch of a quarter-res buffer bleeds cloud over the
// silhouette of anything in front of it — a mountain ridge grows a halo. The
// four contributing low-res texels are therefore weighted by how well their
// depth agrees with this pixel's, which is the standard fix and the reason the
// composite needs the depth buffer at all.
//
// Epic shipped a regression in exactly this stage in UE 5.6, which is why the
// evidence plan names a fast-camera-with-occlusion capture rather than trusting
// a still.

//!include "sky_eye.wgsl"

struct CompositeParams {
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    /// Cloud layer bottom, metres.
    layer_bottom: f32,
    /// Reciprocal of the low-resolution buffer's size, in texels.
    inv_low_size: vec2<f32>,
    /// Size of the low-resolution buffer, in texels.
    low_size: vec2<f32>,
    /// How strongly depth disagreement rejects a tap. 0 falls back to plain
    /// bilinear, which is the useful A/B when a halo is suspected.
    depth_sigma: f32,
    /// Cloud layer top, metres.
    layer_top: f32,
    _pad0: f32,
    _pad1: f32,
    // The sky eye, which burns through the deck: `eye_through` of it survives
    // where the cloud is opaque (the shading pass drew it under the cloud).
    eye_direction: vec3<f32>,
    eye_tan_half_width: f32,
    eye_color: vec3<f32>,
    eye_intensity: f32,
    eye_shape: vec4<f32>,
    eye_time: f32,
    eye_through: f32,
    _pad2: f32,
    _pad3: f32,
}

@group(0) @binding(0) var<uniform> comp: CompositeParams;
@group(0) @binding(1) var cloud_tex: texture_2d<f32>;
@group(0) @binding(2) var cloud_samp: sampler;
@group(0) @binding(3) var comp_depth: texture_depth_2d;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VOut {
    var xs = array<f32, 3>(-1.0, 3.0, -1.0);
    var ys = array<f32, 3>(-1.0, -1.0, 3.0);
    let p = vec2<f32>(xs[vid], ys[vid]);
    var out: VOut;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return out;
}

/// Distance along a ray to where it enters the cloud layer (huge if it never does).
fn layer_entry(origin: vec3<f32>, dir: vec3<f32>) -> f32 {
    if origin.y >= comp.layer_bottom && origin.y <= comp.layer_top {
        return 0.0;
    }
    if abs(dir.y) < 1e-4 {
        return 1e30;
    }
    let t0 = (comp.layer_bottom - origin.y) / dir.y;
    let t1 = (comp.layer_top - origin.y) / dir.y;
    let enter = min(t0, t1);
    if max(t0, t1) <= 0.0 {
        return 1e30;
    }
    return max(enter, 0.0);
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let depth_dims = textureDimensions(comp_depth);
    let here = textureLoad(comp_depth, vec2<i32>(in.uv * vec2<f32>(depth_dims)), 0);
    // Sky: the cloud is behind nothing, so a plain bilinear read — through a
    // canopy the march now carries cloud in every texel that sees any sky.
    if here <= 0.0 || here >= 1.0 {
        let cloud = textureSampleLevel(cloud_tex, cloud_samp, in.uv, 0.0);
        if comp.eye_intensity <= 0.0 {
            return cloud;
        }
        let sky_ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
        let near_p = comp.inv_view_proj * vec4<f32>(sky_ndc, 0.0, 1.0);
        let far_p = comp.inv_view_proj * vec4<f32>(sky_ndc, 1.0, 1.0);
        let dir = normalize(far_p.xyz / far_p.w - near_p.xyz / near_p.w);
        let eye = sky_eye(dir, normalize(comp.eye_direction), comp.eye_tan_half_width, comp.eye_color,
                          comp.eye_intensity, comp.eye_shape, comp.eye_time);
        // Through the deck: the cloud is darkened where the eye covers the sky
        // behind it, and a share of the eye's light comes through.
        return vec4<f32>(cloud.rgb * (1.0 - eye.a * comp.eye_through) + eye.rgb * comp.eye_through * (1.0 - cloud.a),
                         cloud.a);
    }
    if comp.depth_sigma <= 0.0 {
        return textureSampleLevel(cloud_tex, cloud_samp, in.uv, 0.0);
    }
    // Geometry nearer than the layer (a leaf, a roof, a ridge below the
    // clouds) has no cloud in front of it.
    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let wp = comp.inv_view_proj * vec4<f32>(ndc, here, 1.0);
    let to = wp.xyz / wp.w - comp.camera_pos;
    let distance = length(to);
    if distance <= layer_entry(comp.camera_pos, to / max(distance, 1e-6)) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    // The four low-resolution texels this pixel sits between.
    let low = in.uv * comp.low_size - 0.5;
    let base = floor(low);
    let f = low - base;

    var colour = vec4<f32>(0.0);
    var weight_sum = 0.0;
    for (var j = 0; j < 2; j = j + 1) {
        for (var i = 0; i < 2; i = i + 1) {
            let texel = base + vec2<f32>(f32(i), f32(j));
            let clamped = clamp(texel, vec2<f32>(0.0), comp.low_size - 1.0);
            let tap_uv = (clamped + 0.5) * comp.inv_low_size;
            let tap = textureSampleLevel(cloud_tex, cloud_samp, tap_uv, 0.0);

            let bilinear = mix(1.0 - f.x, f.x, f32(i)) * mix(1.0 - f.y, f.y, f32(j));
            let tap_depth = textureLoad(
                comp_depth, vec2<i32>(tap_uv * vec2<f32>(depth_dims)), 0);
            // Depth agreement. A tap from behind a ridge contributes nothing,
            // which is what removes the halo.
            let disagreement = abs(tap_depth - here) / comp.depth_sigma;
            let depth_weight = exp(-disagreement * disagreement);

            let w = bilinear * depth_weight + 1e-5;
            colour += tap * w;
            weight_sum += w;
        }
    }
    return colour / max(weight_sum, 1e-5);
}
