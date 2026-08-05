// Phase 15E: Hi-Z depth pyramid.
//
// A mip chain where every texel holds the FURTHEST depth of the region it
// covers. Depth here is wgpu-style, 0 at the near plane and 1 at the far plane,
// so "furthest" is `max`. That direction is what makes the pyramid a safe
// occlusion test: a candidate is only rejected when its nearest point is behind
// *everything* in the region, and taking the max can only ever make the
// occluder look nearer than it is, which errs toward drawing.
//
// Level 0 copies the depth buffer. Depth textures cannot be bound as storage,
// so it goes through a compute pass rather than a blit.
//
// Odd mip sizes are the classic trap: halving 5 gives 2, and the last row would
// simply vanish from the pyramid, letting a real occluder go unrecorded. Where a
// dimension is odd the reduction widens to 3 texels on that axis so nothing is
// dropped.

@group(0) @binding(0) var src_depth: texture_depth_2d;
@group(0) @binding(1) var dst_level0: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8, 1)
fn copy_depth(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dst_level0);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let d = textureLoad(src_depth, vec2<i32>(id.xy), 0);
    textureStore(dst_level0, vec2<i32>(id.xy), vec4<f32>(d, 0.0, 0.0, 0.0));
}

@group(0) @binding(0) var src_mip: texture_2d<f32>;
@group(0) @binding(1) var dst_mip: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8, 1)
fn downsample(@builtin(global_invocation_id) id: vec3<u32>) {
    let dst_size = textureDimensions(dst_mip);
    if id.x >= dst_size.x || id.y >= dst_size.y {
        return;
    }
    let src_size = textureDimensions(src_mip);
    let base = vec2<i32>(id.xy) * 2;

    var furthest = 0.0;
    // Always the 2x2 block.
    for (var dy = 0; dy < 2; dy++) {
        for (var dx = 0; dx < 2; dx++) {
            let c = min(base + vec2<i32>(dx, dy), vec2<i32>(src_size) - vec2<i32>(1));
            furthest = max(furthest, textureLoad(src_mip, c, 0).r);
        }
    }

    // An odd source dimension leaves one trailing texel that no 2x2 block
    // reaches. Fold it in on whichever axis is odd, and the corner when both.
    let odd_x = (src_size.x & 1u) == 1u && id.x == dst_size.x - 1u;
    let odd_y = (src_size.y & 1u) == 1u && id.y == dst_size.y - 1u;
    if odd_x {
        for (var dy = 0; dy < 2; dy++) {
            let c = min(base + vec2<i32>(2, dy), vec2<i32>(src_size) - vec2<i32>(1));
            furthest = max(furthest, textureLoad(src_mip, c, 0).r);
        }
    }
    if odd_y {
        for (var dx = 0; dx < 2; dx++) {
            let c = min(base + vec2<i32>(dx, 2), vec2<i32>(src_size) - vec2<i32>(1));
            furthest = max(furthest, textureLoad(src_mip, c, 0).r);
        }
    }
    if odd_x && odd_y {
        let c = min(base + vec2<i32>(2, 2), vec2<i32>(src_size) - vec2<i32>(1));
        furthest = max(furthest, textureLoad(src_mip, c, 0).r);
    }

    textureStore(dst_mip, vec2<i32>(id.xy), vec4<f32>(furthest, 0.0, 0.0, 0.0));
}
