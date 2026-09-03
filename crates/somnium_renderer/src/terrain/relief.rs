//! The heightfield normal that survives distance (Phase TSUSHIMA-E).
//!
//! # The defect
//!
//! `mesh::build_chunk_vertices` takes vertex normals as central differences
//! over the **full-resolution** heightmap, which is right. `build_lod_indices`
//! then renders a coarser LOD by *skipping* vertices with stride `1 << lod`.
//!
//! So at LOD 3 the surface is shaded by point samples of a normal field taken
//! every eighth cell. That is not a filtered normal, it is an aliased one: the
//! relief between the samples is not softened, it is simply gone, and what is
//! left flickers as the camera moves because a different eighth cell wins.
//!
//! The correct LOD-n normal is the **average** of the normals it stands for,
//! plus the variance it threw away moved into roughness. That is exactly what
//! `water.wgsl` does to its own slope field — "a displaced vertex surface can
//! keep its large silhouette waves, while its sub-pixel slope energy must
//! migrate into roughness" — and it is why distant water does not turn into a
//! white moiré pattern while distant terrain turns into clay.
//!
//! # What this bakes
//!
//! A mip-chained terrain-space texture. Every level stores the filtered normal
//! *and* the length of the unnormalised mean that produced it:
//!
//! - **RG** — the mean normal's XZ, remapped to `[0, 1]`. Y is reconstructed in
//!   the shader; it is always positive on a heightfield, so nothing is lost.
//! - **B** — `|mean|` before normalising. This is Toksvig's measure: four
//!   agreeing normals sum to length 1, four disagreeing ones sum to much less,
//!   and the shortfall is precisely the roughness the coarse level owes the
//!   surface.
//! - **A** — unused, reserved for LEAN's second moments if B ever proves too
//!   coarse.
//!
//! Mips are generated here rather than by the hardware because the quantity
//! that has to survive downsampling is the *unnormalised sum*, and a hardware
//! mip of an already-normalised normal map throws away the only channel that
//! makes the roughness widening possible.

use rayon::prelude::*;

/// Texels per edge of the relief normal map's level 0.
///
/// One texel per metre on a 1 km terrain — the same density as the heightfield
/// itself, so level 0 loses nothing, and the mip chain then carries every
/// coarser scale the camera might need.
pub const RELIEF_SIZE: u32 = 1024;

/// A baked relief normal chain, ready to upload.
pub struct ReliefMaps {
    /// One RGBA8 buffer per mip, level 0 first.
    pub levels: Vec<Vec<u8>>,
    pub size: u32,
}

impl ReliefMaps {
    pub fn mip_count(&self) -> u32 {
        self.levels.len() as u32
    }
}

/// Central-difference normal of the heightfield at integer sample `(x, z)`.
///
/// Matches `mesh::build_chunk_vertices` exactly — same formula, same edge
/// clamp — because a disagreement between the two would show as a seam where
/// the sampled normal takes over from the interpolated one.
fn heightfield_normal(
    heights: &[f32],
    total_x: u32,
    total_z: u32,
    x: i64,
    z: i64,
    cell_size: f32,
    height_scale: f32,
) -> [f32; 3] {
    let at = |xi: i64, zi: i64| -> f32 {
        let xi = xi.clamp(0, total_x as i64 - 1) as usize;
        let zi = zi.clamp(0, total_z as i64 - 1) as usize;
        heights[zi * total_x as usize + xi]
    };
    let dx = (at(x + 1, z) - at(x - 1, z)) * height_scale / (2.0 * cell_size);
    let dz = (at(x, z + 1) - at(x, z - 1)) * height_scale / (2.0 * cell_size);
    let len = (dx * dx + 1.0 + dz * dz).sqrt();
    [-dx / len, 1.0 / len, -dz / len]
}

fn encode(mean: [f32; 3], length: f32) -> [u8; 4] {
    let inv = 1.0 / length.max(1e-6);
    let n = [mean[0] * inv, mean[1] * inv, mean[2] * inv];
    [
        ((n[0] * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8,
        ((n[2] * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8,
        (length.clamp(0.0, 1.0) * 255.0).round() as u8,
        255,
    ]
}

/// Bake the relief normal chain for one terrain.
pub fn bake(
    heights: &[f32],
    total_x: u32,
    total_z: u32,
    cell_size: f32,
    height_scale: f32,
    size: u32,
) -> ReliefMaps {
    let n = size.max(1) as usize;
    if heights.is_empty() || total_x < 2 || total_z < 2 {
        // Flat up, zero variance: the identity this reduces to when there is
        // nothing to bake, and what an unbound map has to agree with.
        let flat = encode([0.0, 1.0, 0.0], 1.0);
        let mut levels = Vec::new();
        let mut edge = n;
        while edge >= 1 {
            levels.push(flat.repeat(edge * edge));
            if edge == 1 {
                break;
            }
            edge /= 2;
        }
        return ReliefMaps { levels, size };
    }

    // Level 0, in float, so the mip reduction can work on unnormalised sums
    // rather than on quantised bytes.
    let mut level: Vec<[f32; 3]> = vec![[0.0; 3]; n * n];
    level
        .par_chunks_mut(n)
        .enumerate()
        .for_each(|(z, row)| {
            for (x, texel) in row.iter_mut().enumerate() {
                // Terrain-space texel centre mapped onto heightfield samples.
                let hx = ((x as f32 + 0.5) / n as f32 * (total_x - 1) as f32).round() as i64;
                let hz = ((z as f32 + 0.5) / n as f32 * (total_z - 1) as f32).round() as i64;
                *texel = heightfield_normal(
                    heights, total_x, total_z, hx, hz, cell_size, height_scale,
                );
            }
        });

    let mut levels = Vec::new();
    let mut edge = n;
    loop {
        levels.push(
            level
                .iter()
                .flat_map(|v| {
                    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                    encode(*v, len)
                })
                .collect::<Vec<u8>>(),
        );
        if edge == 1 {
            break;
        }
        let next_edge = edge / 2;
        let mut next = vec![[0.0f32; 3]; next_edge * next_edge];
        for z in 0..next_edge {
            for x in 0..next_edge {
                // The mean of four *unnormalised* means. Averaging without
                // renormalising is the entire technique: the shrinking length
                // is the record of how much the four disagreed, and it is what
                // the shader turns back into roughness.
                let mut acc = [0.0f32; 3];
                for (ox, oz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + ox).min(edge - 1);
                    let sz = (z * 2 + oz).min(edge - 1);
                    let s = level[sz * edge + sx];
                    acc[0] += s[0];
                    acc[1] += s[1];
                    acc[2] += s[2];
                }
                next[z * next_edge + x] = [acc[0] * 0.25, acc[1] * 0.25, acc[2] * 0.25];
            }
        }
        level = next;
        edge = next_edge;
    }

    ReliefMaps { levels, size }
}

/// GPU residency for one terrain's relief chain.
pub struct ReliefGpu {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, maps: &ReliefMaps) -> ReliefGpu {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Terrain Relief Normal"),
        size: wgpu::Extent3d {
            width: maps.size,
            height: maps.size,
            depth_or_array_layers: 1,
        },
        mip_level_count: maps.mip_count(),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // Not sRGB: a direction and a length, not a colour.
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    write_levels(queue, &texture, maps);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    ReliefGpu { texture, view }
}

/// Rewrite every mip in place after a sculpt, keeping the bindless index.
pub fn rewrite(queue: &wgpu::Queue, gpu: &ReliefGpu, maps: &ReliefMaps) {
    write_levels(queue, &gpu.texture, maps);
}

fn write_levels(queue: &wgpu::Queue, texture: &wgpu::Texture, maps: &ReliefMaps) {
    for (mip, texels) in maps.levels.iter().enumerate() {
        let edge = (maps.size >> mip).max(1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: mip as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(edge * 4),
                rows_per_image: Some(edge),
            },
            wgpu::Extent3d {
                width: edge,
                height: edge,
                depth_or_array_layers: 1,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(texel: &[u8]) -> ([f32; 2], f32) {
        (
            [
                f32::from(texel[0]) / 255.0 * 2.0 - 1.0,
                f32::from(texel[1]) / 255.0 * 2.0 - 1.0,
            ],
            f32::from(texel[2]) / 255.0,
        )
    }

    #[test]
    fn flat_ground_is_up_with_no_variance_at_every_mip() {
        let total = 65u32;
        let maps = bake(&vec![0.0; (total * total) as usize], total, total, 1.0, 1.0, 32);
        for (mip, level) in maps.levels.iter().enumerate() {
            for texel in level.chunks(4) {
                let (xz, len) = decode(texel);
                assert!(xz[0].abs() < 0.01 && xz[1].abs() < 0.01, "mip {mip} not up");
                assert!(len > 0.99, "mip {mip} invented variance from flat ground");
            }
        }
    }

    #[test]
    fn the_chain_reaches_one_texel() {
        let total = 65u32;
        let maps = bake(&vec![0.0; (total * total) as usize], total, total, 1.0, 1.0, 64);
        assert_eq!(maps.mip_count(), 7, "64 -> 1 is seven levels");
        assert_eq!(maps.levels.last().unwrap().len(), 4);
    }

    #[test]
    fn disagreeing_normals_shorten_the_mean() {
        // The property the whole technique rests on. Relief whose normals
        // alternate cancels as it is averaged, and the shrinking length is the
        // roughness the coarse level owes the surface. If this ever returns 1
        // the bake has renormalised somewhere it should not have.
        //
        // A period-**four** triangle, not a period-two sawtooth. A central
        // difference reads `h[x+1] - h[x-1]`, and those two samples share a
        // parity — so a period-two signal has a central difference of exactly
        // zero everywhere and produces a perfectly flat normal field. The
        // first version of this test used one and failed against correct code.
        let total = 65u32;
        let mut h = vec![0.0f32; (total * total) as usize];
        for z in 0..total as usize {
            for x in 0..total as usize {
                h[z * total as usize + x] = match x % 4 {
                    0 => 0.0,
                    1 | 3 => 2.0,
                    _ => 4.0,
                };
            }
        }
        let maps = bake(&h, total, total, 1.0, 1.0, 64);
        let (_, fine) = decode(&maps.levels[0][..4]);
        let coarse_level = &maps.levels[3];
        let (_, coarse) = decode(&coarse_level[..4]);
        assert!(fine > 0.99, "level 0 is a single normal and cannot disagree");
        assert!(
            coarse < 0.9,
            "averaging a sawtooth must shorten the mean, got {coarse}"
        );
    }

    #[test]
    fn a_constant_slope_keeps_its_full_length() {
        // The other half of the same property: normals that agree must survive
        // downsampling at full length, or every hillside would be handed
        // roughness it has not earned and the whole terrain would go matte.
        let total = 65u32;
        let mut h = vec![0.0f32; (total * total) as usize];
        for z in 0..total as usize {
            for x in 0..total as usize {
                h[z * total as usize + x] = x as f32 * 0.5;
            }
        }
        let maps = bake(&h, total, total, 1.0, 1.0, 32);
        // Away from the clamped edges, where the slope really is constant.
        let n = 32usize;
        let idx = (16 * n + 16) * 4;
        let (xz, len) = decode(&maps.levels[3][idx.min(maps.levels[3].len() - 4)..]);
        assert!(len > 0.97, "a constant slope should not lose length: {len}");
        assert!(xz[0] < -0.1, "and it should still lean downhill: {xz:?}");
    }
}
