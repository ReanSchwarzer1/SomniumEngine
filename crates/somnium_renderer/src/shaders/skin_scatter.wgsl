//!include "global_pool.wgsl"

// Somnium Engine — screen-space skin scattering.
//
// Light that enters skin comes out a few millimetres away, red furthest. The
// shading pass lights one point at a time and cannot know what the skin beside
// it received, so its shadow edges and pores on a face are as hard as on
// plaster. This pass does the gathering the shading pass cannot: on pixels
// whose material is skin (`Material.subsurface`), the lit colour is blurred
// along the surface, one axis at a time, by a Gaussian that is a different
// width for each colour. Red spreads its full reach, green under half of it,
// blue hardly at all: the detail stays in the blue and the softness in the
// red, which is what skin under a light looks like.
//
// The reach is a length on the surface, so the kernel is sized in pixels from
// the pixel's distance, and a tap is refused when it is not skin or lies at
// another depth (the nose against the cheek behind it, a hand against a wall).
//
// It blurs what the shading pass wrote, highlights included. Splitting the
// diffuse light from the specular is the proper form (Jimenez, "Separable
// Subsurface Scattering") and needs a second target from the shading pass.

struct ScatterParams {
    // (1, 0) for the first run, (0, 1) for the second.
    direction: vec2<f32>,
    // Pixels a metre covers at a metre's distance.
    pixels_per_metre: f32,
    // Metres the red channel spreads at scatter 1.
    reach: f32,
}

@group(1) @binding(0) var colour_tex: texture_2d<f32>;
@group(1) @binding(1) var depth_tex: texture_depth_2d;
@group(1) @binding(2) var vis_tex: texture_2d<u32>;
@group(1) @binding(3) var<uniform> params: ScatterParams;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

// How much of a pixel's material is skin; 0 for the sky and for everything else.
fn scatter_of(coord: vec2<i32>) -> f32 {
    let vis = textureLoad(vis_tex, coord, 0);
    if vis.x == 0u {
        return 0.0;
    }
    return materials[instances[vis.x - 1u].material_id].subsurface;
}

fn distance_at(coord: vec2<i32>, size: vec2<f32>) -> f32 {
    let uv = (vec2<f32>(coord) + 0.5) / size;
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, textureLoad(depth_tex, coord, 0), 1.0);
    let world = view.inv_view_proj * ndc;
    return length(world.xyz / world.w - view.camera_pos);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let coord = vec2<i32>(position.xy);
    let centre = textureLoad(colour_tex, coord, 0);
    let scatter = scatter_of(coord);
    if scatter <= 0.0 {
        return centre;
    }
    let size = vec2<f32>(textureDimensions(colour_tex));
    let here = distance_at(coord, size);
    let reach = params.reach * scatter;
    // The red channel's spread in pixels here. Under a pixel there is nothing
    // to gather; past two dozen the taps would stride over whole features.
    let spread = min(reach * params.pixels_per_metre / max(here, 0.05), 24.0);
    if spread < 0.75 {
        return centre;
    }
    // One standard deviation for each colour, in pixels.
    let sigma = vec3<f32>(1.0, 0.42, 0.20) * spread * 0.5;
    let stride = spread / 4.0;
    var sum = centre.rgb;
    var weight = vec3<f32>(1.0);
    let limit = vec2<i32>(size) - vec2<i32>(1);
    for (var tap = 1; tap <= 5; tap++) {
        let reach_px = f32(tap) * stride;
        let fall = exp(-vec3<f32>(reach_px * reach_px) / (2.0 * sigma * sigma));
        for (var side = -1; side <= 1; side += 2) {
            let at = clamp(coord + vec2<i32>(round(params.direction * reach_px * f32(side))), vec2<i32>(0), limit);
            if scatter_of(at) <= 0.0 {
                continue;
            }
            // The same surface: within a few reaches of this pixel's depth.
            let apart = abs(distance_at(at, size) - here) / (reach * 3.0);
            let same = exp(-apart * apart);
            let w = fall * same;
            sum += textureLoad(colour_tex, at, 0).rgb * w;
            weight += w;
        }
    }
    return vec4<f32>(sum / weight, centre.a);
}
