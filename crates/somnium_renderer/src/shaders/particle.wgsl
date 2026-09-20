// Linear HDR sprites; texture alpha, depth, lifetime and colour are independent.
enable wgpu_binding_array;
struct ParticleView {
    view_proj: mat4x4<f32>,
    camera_right: vec3<f32>, _pad0: f32,
    camera_up: vec3<f32>, _pad1: f32,
}
struct GpuParticle {
    position: vec3<f32>, size: f32,
    color: vec4<f32>,
    tip_tint: vec3<f32>, aspect: f32,
    uv_rect: vec4<f32>,
    rotation: f32, texture_index: i32, flags: u32, flutter: f32,
}
@group(0) @binding(0) var<uniform> pview: ParticleView;
@group(0) @binding(1) var<storage, read> particles: array<GpuParticle>;
@group(0) @binding(2) var sprite_sampler: sampler;
@group(1) @binding(4) var textures: binding_array<texture_2d<f32>>;
const UV: array<vec2<f32>,6> = array<vec2<f32>,6>(vec2(0.,1.),vec2(1.,1.),vec2(1.,0.),vec2(0.,1.),vec2(1.,0.),vec2(0.,0.));
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
    @location(2) tip: vec3<f32>,
    @location(3) @interpolate(flat) texture_index: i32,
    @location(4) @interpolate(flat) flags: u32,
    @location(5) local_uv: vec2<f32>,
    @location(6) @interpolate(flat) uv_rect: vec4<f32>,
    @location(7) @interpolate(flat) flutter: f32,
}
@vertex fn vs_main(@builtin(vertex_index) vid: u32, @builtin(instance_index) iid: u32) -> Out {
    let p = particles[iid]; let uv = UV[vid];
    let xy = vec2((uv.x - .5) * p.size, (.5 - uv.y) * p.size * p.aspect);
    let c = cos(p.rotation); let s = sin(p.rotation);
    let offset = vec2(c*xy.x-s*xy.y,s*xy.x+c*xy.y);
    var right = pview.camera_right; var up = pview.camera_up;
    if (p.flags & 2u) != 0u {
        let horizontal = vec3(right.x, 0., right.z);
        right = horizontal / max(length(horizontal),0.00001);
        if length(horizontal) < 0.00001 { right = vec3(1.,0.,0.); }
        up = vec3(0.,1.,0.);
    }
    var out: Out;
    out.position = pview.view_proj * vec4(p.position + right*offset.x + up*offset.y,1.);
    out.uv = p.uv_rect.xy + uv*p.uv_rect.zw;
    out.local_uv = uv; out.colour = p.color; out.tip = p.tip_tint;
    out.uv_rect = p.uv_rect; out.flutter = p.flutter;
    out.texture_index = p.texture_index; out.flags = p.flags;
    return out;
}
struct Fragment {
    @location(0) colour: vec4<f32>,
    @location(1) reactive: vec4<f32>,
}
@fragment fn fs_main(in: Out) -> Fragment {
    var sprite = vec4(1.);
    if in.texture_index >= 0 {
        // Bend the textured silhouette, with zero displacement at the wick.
        // Sampling is clamped to this atlas cell, never its neighbour.
        let height = 1. - in.local_uv.y;
        let uv = vec2(select(in.local_uv.x, 1. - in.local_uv.x, (in.flags & 4u) != 0u), in.local_uv.y);
        let warped = uv - vec2(in.flutter * height * height, 0.);
        let sample_uv = in.uv_rect.xy + clamp(warped, vec2(.001), vec2(.999)) * in.uv_rect.zw;
        sprite = textureSampleLevel(textures[in.texture_index],sprite_sampler,sample_uv,0.);
        if any(warped < vec2(0.)) || any(warped > vec2(1.)) { sprite.a = 0.; }
        // Shared standalone asset textures are RGBA8 UNORM; colour input is sRGB.
        sprite = vec4(pow(max(sprite.rgb,vec3(0.)),vec3(2.2)),sprite.a);
    } else {
        sprite.a = 1. - smoothstep(.2,.5,distance(in.local_uv,vec2(.5)));
    }
    let alpha = clamp(in.colour.a * sprite.a,0.,1.);
    let tint = mix(vec3(1.),in.tip,smoothstep(.12,.72,1.-in.local_uv.y));
    let rgb = max(in.colour.rgb * tint * sprite.rgb,vec3(0.)) * alpha;
    var out: Fragment;
    out.colour = vec4(rgb,select(alpha,0.,(in.flags & 1u) != 0u));
    // Texture alpha and scene depth decide actual visible coverage. Empty sprite
    // corners leave static history intact, including for additive flame sprites.
    out.reactive = vec4(select(0.,1.,alpha > .001),0.,0.,0.);
    return out;
}
