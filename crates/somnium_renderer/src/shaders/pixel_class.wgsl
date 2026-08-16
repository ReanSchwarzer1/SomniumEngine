// Somnium Engine — what kind of surface a visibility-buffer pixel holds.
//
// One definition, shared by the census (Phase DOOM-B, which counts pixels) and
// the tile classifier (Phase DOOM-C, which routes tiles to shading pipelines).
// They ask different questions and apply different thresholds, but if they ever
// disagreed about what "terrain" means the census would be describing a
// taxonomy the renderer does not use — and the cost table built from it would
// be quietly wrong.
//
// Concatenated after `global_pool.wgsl`, and it needs a `vis_buffer` and a
// `class_depth` texture bound by whoever includes it.

const PC_SKY:     u32 = 0u;
const PC_MESH:    u32 = 1u;
const PC_FOLIAGE: u32 = 2u;
const PC_TERRAIN: u32 = 3u;

struct PixelClass {
    kind: u32,
    /// Distance from the camera in metres. Zero for sky, which has none.
    distance: f32,
}

fn pc_world_at(coord: vec2<i32>, depth: f32, dims: vec2<u32>) -> vec3<f32> {
    let uv = (vec2<f32>(coord) + vec2<f32>(0.5)) / vec2<f32>(f32(dims.x), f32(dims.y));
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth);
    let h = view.inv_view_proj * vec4<f32>(ndc, 1.0);
    return h.xyz / h.w;
}

/// Classify one pixel. Mirrors `shading.wgsl`'s branch structure exactly:
/// the sky sentinel, then `terrain_index`, then `alpha_cutoff`.
fn pc_classify(coord: vec2<i32>, dims: vec2<u32>) -> PixelClass {
    var out: PixelClass;
    out.distance = 0.0;

    let vis = textureLoad(vis_buffer, coord, 0).rg;
    if vis.x == 0u {
        out.kind = PC_SKY;
        return out;
    }
    let instance = instances[vis.x - 1u];
    let material = materials[instance.material_id];

    if material.terrain_index >= 0 {
        out.kind = PC_TERRAIN;
        let depth = textureLoad(class_depth, coord, 0);
        out.distance = length(pc_world_at(coord, depth, dims) - view.camera_pos);
        return out;
    }
    // Cutout materials are the foliage path: the visibility pass discards below
    // `alpha_cutoff`, and shading pays for a second alpha fetch plus the
    // two-sided normal flip. Everything else is an ordinary opaque surface.
    if material.alpha_cutoff > 0.0 {
        out.kind = PC_FOLIAGE;
        return out;
    }
    out.kind = PC_MESH;
    return out;
}
