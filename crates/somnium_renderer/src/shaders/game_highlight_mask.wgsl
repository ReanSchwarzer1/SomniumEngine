//!include "global_pool.wgsl"

@group(1) @binding(0) var highlight_vis: texture_2d<u32>;
@group(1) @binding(1) var<storage, read> highlighted: array<u32>;
@group(1) @binding(2) var highlight_sampler: sampler;

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_opaque(@builtin(position) at: vec4<f32>) -> @location(0) f32 {
    let id = textureLoad(highlight_vis, vec2<i32>(at.xy), 0).x;
    if id == 0u || id > arrayLength(&highlighted) {
        return 0.0;
    }
    return select(0.0, 1.0, highlighted[id - 1u] != 0u);
}

struct HighlightVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) material: u32,
}

@vertex
fn vs_transparent(@builtin(vertex_index) index: u32, @builtin(instance_index) instance_id: u32) -> HighlightVertex {
    let instance = instances[instance_id];
    let vertex = vertices[instance.vertex_offset + indices[instance.index_offset + index]];
    var out: HighlightVertex;
    out.position = view.view_proj * instance.model * vec4<f32>(vertex.pos_x, vertex.pos_y, vertex.pos_z, 1.0);
    out.uv = vec2<f32>(vertex.u, vertex.v);
    out.material = instance.material_id;
    return out;
}

@fragment
fn fs_transparent(in: HighlightVertex) -> @location(0) f32 {
    let material = materials[in.material];
    let dx = dpdx(in.uv);
    let dy = dpdy(in.uv);
    if material.albedo_map >= 0 {
        let alpha = textureSampleGrad(textures[material.albedo_map], highlight_sampler, in.uv, dx, dy).a;
        // Preserve cut-away texture regions; untextured clear glass still has a physical surface.
        if alpha < max(material.alpha_cutoff, 0.001) {
            discard;
        }
    }
    return 1.0;
}
