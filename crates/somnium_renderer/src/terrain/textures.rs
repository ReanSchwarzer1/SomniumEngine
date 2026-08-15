//! Procedural terrain layer textures + splatmap GPU resources (Phase 14C/14E).
//!
//! The four default PBR layers (grass / dirt / rock / snow) are generated
//! procedurally with hash-based value noise (original code, same family as
//! `somnium_voxel::terrain`) so the terrain works without any asset files.
//! Layers are stored as `texture_2d_array`s — one array each for albedo,
//! normal, and roughness — following the array-texture material layout of
//! `example_repo/bevy-plugins/bevy_triplanar_splatting-main/src/`.

/// Side length of each generated layer texture.
pub const LAYER_TEXTURE_SIZE: u32 = 256;
/// Number of material layers.
///
/// Phase XV-Zeta: thirty-two global materials, carried by **eight** RGBA8
/// splatmaps. At most four weights are stored per texel (see [`super::splat`]);
/// the shader selects the strongest four before PBR sampling. Indices 0–7 stay
/// compatibility-locked with Phase 25L; 8–15 with XV-C.
pub const TERRAIN_LAYER_COUNT: u32 = 32;
/// Photographed layers 0–15 live in the hero array; 16–31 in the extra array
/// so the two banks can load at different resolutions (XVI RGBA8 budget).
pub const TERRAIN_HERO_LAYERS: u32 = 16;

/// One splatmap texel: a weight per material layer.
///
/// Named because it crosses a crate boundary — the editor's undo commands carry
/// blocks of these — and Phase 25L widened it from 4 to 8. A bare `[u8; 4]` in
/// `somnium_core` is exactly the kind of thing that silently disagrees.
pub type SplatTexel = [u8; TERRAIN_LAYER_COUNT as usize];

/// Wang-style integer hash → [0, 1).
fn hash2(ix: i32, iz: i32, seed: u32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x85EB_CA6B)
        ^ (iz as u32).wrapping_mul(0xC2B2_AE35)
        ^ seed.wrapping_mul(0x27D4_EB2F);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h & 0x00FF_FFFF) as f32 / 16_777_216.0
}

/// Tileable bilinear value noise on a `period`-cell lattice, output [0, 1].
fn tileable_noise(u: f32, v: f32, period: i32, seed: u32) -> f32 {
    let x = u * period as f32;
    let z = v * period as f32;
    let (ix, iz) = (x.floor() as i32, z.floor() as i32);
    let (fx, fz) = (x - x.floor(), z - z.floor());
    let (ux, uz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
    let wrap = |i: i32| i.rem_euclid(period);

    let a = hash2(wrap(ix), wrap(iz), seed);
    let b = hash2(wrap(ix + 1), wrap(iz), seed);
    let c = hash2(wrap(ix), wrap(iz + 1), seed);
    let d = hash2(wrap(ix + 1), wrap(iz + 1), seed);
    a + (b - a) * ux + (c - a) * uz + (a - b - c + d) * ux * uz
}

/// 3-octave tileable FBM, output [0, 1].
fn fbm(u: f32, v: f32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut norm = 0.0;
    let mut period = 8;
    for octave in 0..3u32 {
        sum += tileable_noise(u, v, period, seed.wrapping_add(octave * 7919)) * amp;
        norm += amp;
        amp *= 0.5;
        period *= 2;
    }
    sum / norm
}

/// Per-layer procedural recipe: two albedo tones blended by noise + roughness.
struct LayerRecipe {
    tone_a: [f32; 3],
    tone_b: [f32; 3],
    roughness: f32,
    /// Strength of the noise-derived bump in the generated normal map.
    bump: f32,
    seed: u32,
}

const RECIPES: [LayerRecipe; TERRAIN_LAYER_COUNT as usize] = [
    // 0: Grass
    LayerRecipe {
        tone_a: [0.16, 0.34, 0.10],
        tone_b: [0.28, 0.46, 0.14],
        roughness: 0.90,
        bump: 0.6,
        seed: 11,
    },
    // 1: Dirt
    LayerRecipe {
        tone_a: [0.30, 0.21, 0.13],
        tone_b: [0.42, 0.31, 0.20],
        roughness: 0.95,
        bump: 0.8,
        seed: 23,
    },
    // 2: Rock (also the triplanar cliff layer)
    LayerRecipe {
        tone_a: [0.32, 0.32, 0.33],
        tone_b: [0.52, 0.51, 0.50],
        roughness: 0.80,
        bump: 1.0,
        seed: 37,
    },
    // 3: Snow
    LayerRecipe {
        tone_a: [0.82, 0.85, 0.90],
        tone_b: [0.95, 0.96, 1.00],
        roughness: 0.35,
        bump: 0.3,
        seed: 53,
    },
    // 4: Meadow — a lighter, yellower grass than layer 0.
    LayerRecipe {
        tone_a: [0.22, 0.36, 0.12],
        tone_b: [0.38, 0.48, 0.18],
        roughness: 0.92,
        bump: 0.5,
        seed: 67,
    },
    // 5: Mud
    LayerRecipe {
        tone_a: [0.20, 0.14, 0.09],
        tone_b: [0.32, 0.24, 0.16],
        roughness: 0.70,
        bump: 0.9,
        seed: 79,
    },
    // 6: Sand
    LayerRecipe {
        tone_a: [0.62, 0.54, 0.38],
        tone_b: [0.76, 0.68, 0.50],
        roughness: 0.85,
        bump: 0.4,
        seed: 89,
    },
    // 7: Gravel
    LayerRecipe {
        tone_a: [0.34, 0.32, 0.30],
        tone_b: [0.55, 0.52, 0.48],
        roughness: 0.88,
        bump: 1.0,
        seed: 97,
    },
    // 8: Dry beach sand
    LayerRecipe {
        tone_a: [0.72, 0.64, 0.46],
        tone_b: [0.84, 0.76, 0.58],
        roughness: 0.82,
        bump: 0.25,
        seed: 113,
    },
    // 9: Damp shoreline sand
    LayerRecipe {
        tone_a: [0.48, 0.40, 0.28],
        tone_b: [0.60, 0.50, 0.36],
        roughness: 0.55,
        bump: 0.3,
        seed: 127,
    },
    // 10: Dry earth
    LayerRecipe {
        tone_a: [0.38, 0.28, 0.18],
        tone_b: [0.52, 0.40, 0.26],
        roughness: 0.90,
        bump: 0.7,
        seed: 139,
    },
    // 11: Red mineral clay
    LayerRecipe {
        tone_a: [0.48, 0.22, 0.14],
        tone_b: [0.62, 0.32, 0.18],
        roughness: 0.78,
        bump: 0.85,
        seed: 149,
    },
    // 12: Sparse grass
    LayerRecipe {
        tone_a: [0.28, 0.32, 0.14],
        tone_b: [0.40, 0.30, 0.16],
        roughness: 0.88,
        bump: 0.45,
        seed: 163,
    },
    // 13: Mossy rock
    LayerRecipe {
        tone_a: [0.22, 0.28, 0.16],
        tone_b: [0.36, 0.38, 0.28],
        roughness: 0.70,
        bump: 0.95,
        seed: 179,
    },
    // 14: Vertical cliff
    LayerRecipe {
        tone_a: [0.30, 0.26, 0.22],
        tone_b: [0.48, 0.42, 0.36],
        roughness: 0.75,
        bump: 1.1,
        seed: 181,
    },
    // 15: Talus / river stone
    LayerRecipe {
        tone_a: [0.36, 0.34, 0.30],
        tone_b: [0.58, 0.54, 0.46],
        roughness: 0.86,
        bump: 1.05,
        seed: 197,
    },
    // 16: Lush lawn — saturated green, the distant meadow identity.
    LayerRecipe {
        tone_a: [0.12, 0.38, 0.10],
        tone_b: [0.22, 0.52, 0.14],
        roughness: 0.88,
        bump: 0.45,
        seed: 211,
    },
    // 17: Dark conifer duff
    LayerRecipe {
        tone_a: [0.10, 0.12, 0.08],
        tone_b: [0.18, 0.20, 0.12],
        roughness: 0.92,
        bump: 0.7,
        seed: 223,
    },
    // 18: Cool gray aerial rock
    LayerRecipe {
        tone_a: [0.38, 0.40, 0.42],
        tone_b: [0.55, 0.57, 0.60],
        roughness: 0.78,
        bump: 1.0,
        seed: 227,
    },
    // 19: Dark slate wall
    LayerRecipe {
        tone_a: [0.16, 0.17, 0.20],
        tone_b: [0.28, 0.30, 0.34],
        roughness: 0.72,
        bump: 1.05,
        seed: 233,
    },
    // 20: Green moss carpet
    LayerRecipe {
        tone_a: [0.10, 0.28, 0.08],
        tone_b: [0.20, 0.42, 0.14],
        roughness: 0.80,
        bump: 0.55,
        seed: 239,
    },
    // 21: Pale limestone
    LayerRecipe {
        tone_a: [0.62, 0.58, 0.48],
        tone_b: [0.82, 0.78, 0.66],
        roughness: 0.70,
        bump: 0.8,
        seed: 241,
    },
    // 22: Dark wet loam
    LayerRecipe {
        tone_a: [0.12, 0.10, 0.08],
        tone_b: [0.22, 0.18, 0.12],
        roughness: 0.68,
        bump: 0.6,
        seed: 251,
    },
    // 23: Pine-needle litter
    LayerRecipe {
        tone_a: [0.22, 0.18, 0.10],
        tone_b: [0.36, 0.28, 0.14],
        roughness: 0.90,
        bump: 0.65,
        seed: 257,
    },
    // 24: Bright wildgrass / meadow
    LayerRecipe {
        tone_a: [0.28, 0.46, 0.12],
        tone_b: [0.48, 0.62, 0.18],
        roughness: 0.86,
        bump: 0.4,
        seed: 263,
    },
    // 25: Wetland / peat
    LayerRecipe {
        tone_a: [0.14, 0.18, 0.12],
        tone_b: [0.22, 0.28, 0.16],
        roughness: 0.75,
        bump: 0.5,
        seed: 269,
    },
    // 26: Gray granite talus
    LayerRecipe {
        tone_a: [0.42, 0.42, 0.44],
        tone_b: [0.62, 0.62, 0.64],
        roughness: 0.84,
        bump: 1.1,
        seed: 271,
    },
    // 27: Light cool dune
    LayerRecipe {
        tone_a: [0.70, 0.68, 0.58],
        tone_b: [0.88, 0.86, 0.74],
        roughness: 0.80,
        bump: 0.25,
        seed: 277,
    },
    // 28: Lichen rock
    LayerRecipe {
        tone_a: [0.30, 0.34, 0.22],
        tone_b: [0.48, 0.50, 0.32],
        roughness: 0.76,
        bump: 0.9,
        seed: 281,
    },
    // 29: Autumn leaf litter
    LayerRecipe {
        tone_a: [0.42, 0.22, 0.08],
        tone_b: [0.62, 0.36, 0.12],
        roughness: 0.88,
        bump: 0.6,
        seed: 283,
    },
    // 30: Packed pale path
    LayerRecipe {
        tone_a: [0.48, 0.44, 0.38],
        tone_b: [0.64, 0.60, 0.52],
        roughness: 0.65,
        bump: 0.35,
        seed: 293,
    },
    // 31: Hard wind-crust snow
    LayerRecipe {
        tone_a: [0.88, 0.90, 0.94],
        tone_b: [0.98, 0.99, 1.00],
        roughness: 0.28,
        bump: 0.2,
        seed: 307,
    },
];

/// Names matching the recipe order, used by the layer-management UI.
pub const LAYER_NAMES: [&str; TERRAIN_LAYER_COUNT as usize] = [
    "Grass",
    "Forest Floor",
    "Rock",
    "Snow",
    "Meadow",
    "Mud",
    "Sand",
    "Gravel",
    "Dry Sand",
    "Damp Sand",
    "Dry Earth",
    "Red Clay",
    "Sparse Grass",
    "Mossy Rock",
    "Cliff",
    "Talus",
    "Lush Lawn",
    "Conifer Duff",
    "Cool Gray Rock",
    "Dark Slate",
    "Moss Carpet",
    "Limestone",
    "Dark Loam",
    "Pine Litter",
    "Wildgrass",
    "Wetland",
    "Gray Talus",
    "Light Dune",
    "Lichen Rock",
    "Autumn Litter",
    "Packed Path",
    "Hard Snow",
];

/// UV repeats per metre. Layers 0–7 stay at the shipping 0.25 (4 m tile) so
/// old scenes do not retile. Layers 8–15 and photographed 16–31 use
/// `1 / physical_width_m` (aerial scans clamped to 15 m). Slots 16 and 24 are
/// procedural lawn/wildgrass.
pub const LAYER_TILING: [f32; TERRAIN_LAYER_COUNT as usize] = [
    0.25,
    0.25,
    0.25,
    0.25,
    0.25,
    0.25,
    0.25,
    0.25,
    1.0 / 15.0,
    1.0 / 15.0,
    1.0 / 3.0,
    1.0 / 2.0,
    1.0 / 2.0,
    1.0 / 3.0,
    1.0 / 2.7,
    1.0 / 2.16,
    1.0 / 2.0,
    1.0 / 12.6,
    1.0 / 15.0,
    1.0 / 20.0,
    1.0 / 20.0,
    1.0 / 18.0,
    1.0 / 20.7,
    1.0 / 30.0,
    1.0 / 2.0,
    1.0 / 21.1,
    1.0 / 18.0,
    1.0 / 15.0,
    1.0 / 20.0,
    1.0 / 21.4,
    1.0 / 20.0,
    1.0 / 20.0,
];

/// Moisture affinity 0..1 from the XV-A manifest (porous-wetting weights).
pub const LAYER_MOISTURE: [f32; TERRAIN_LAYER_COUNT as usize] = [
    0.55, 0.70, 0.25, 0.15, 0.60, 0.95, 0.45, 0.20, 0.35, 0.90, 0.40, 0.50, 0.55, 0.85, 0.20, 0.40,
    0.65, 0.75, 0.20, 0.15, 0.90, 0.30, 0.85, 0.60, 0.55, 0.95, 0.20, 0.25, 0.40, 0.70, 0.15, 0.10,
];

/// Short inspector labels (XV-I palette).
pub const LAYER_SHORT: [&str; TERRAIN_LAYER_COUNT as usize] = [
    "Grass", "Forest", "Rock", "Snow", "Meadow", "Mud", "Coast", "Gravel", "DrySd", "DampSd",
    "Earth", "Clay", "Sparse", "Moss", "Cliff", "Talus", "Lawn", "Duff", "GrayRk", "Slate",
    "MossC", "Lime", "Loam", "Pine", "Wild", "Peat", "Gran", "Dune", "Lichen", "Autumn", "Path",
    "Crust",
];

fn noise_height(u: f32, v: f32, recipe: &LayerRecipe) -> f32 {
    fbm(u, v, recipe.seed)
}

/// Generate the (albedo, normal, roughness) texel data for one layer.
/// Each returned `Vec` is `LAYER_TEXTURE_SIZE²` RGBA8 texels, row-major.
fn generate_layer(recipe: &LayerRecipe) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let n = LAYER_TEXTURE_SIZE as usize;
    let mut albedo = Vec::with_capacity(n * n * 4);
    let mut normal = Vec::with_capacity(n * n * 4);
    let mut rough = Vec::with_capacity(n * n * 4);
    let inv = 1.0 / n as f32;

    for z in 0..n {
        for x in 0..n {
            let (u, v) = (x as f32 * inv, z as f32 * inv);
            let t = noise_height(u, v, recipe);

            let col = [
                recipe.tone_a[0] + (recipe.tone_b[0] - recipe.tone_a[0]) * t,
                recipe.tone_a[1] + (recipe.tone_b[1] - recipe.tone_a[1]) * t,
                recipe.tone_a[2] + (recipe.tone_b[2] - recipe.tone_a[2]) * t,
            ];
            // Albedo alpha carries the noise "height" for height-based blending.
            albedo.extend([
                (col[0] * 255.0) as u8,
                (col[1] * 255.0) as u8,
                (col[2] * 255.0) as u8,
                (t * 255.0) as u8,
            ]);

            // Tangent-space normal from noise gradient (finite differences).
            let e = inv;
            let dx = (noise_height(u + e, v, recipe) - noise_height(u - e, v, recipe))
                * recipe.bump
                * 8.0;
            let dz = (noise_height(u, v + e, recipe) - noise_height(u, v - e, recipe))
                * recipe.bump
                * 8.0;
            let nv = glam::Vec3::new(-dx, -dz, 1.0).normalize();
            normal.extend([
                ((nv.x * 0.5 + 0.5) * 255.0) as u8,
                ((nv.y * 0.5 + 0.5) * 255.0) as u8,
                ((nv.z * 0.5 + 0.5) * 255.0) as u8,
                255,
            ]);

            // Roughness varies slightly with the noise for visual richness.
            let r = (recipe.roughness + (t - 0.5) * 0.1).clamp(0.05, 1.0);
            rough.extend([(r * 255.0) as u8, (r * 255.0) as u8, (r * 255.0) as u8, 255]);
        }
    }
    (albedo, normal, rough)
}

/// Upload one array texture with a full **semantic** mip chain (Phase XV-B).
fn create_array_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    format: wgpu::TextureFormat,
    size: u32,
    layers: &[Vec<u8>],
    kind: super::mips::PackedKind,
) -> (wgpu::Texture, wgpu::TextureView) {
    let mip_level_count = (size as f32).log2().floor() as u32 + 1;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: layers.len() as u32,
        },
        mip_level_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (i, data) in layers.iter().enumerate() {
        for (level, (lw, lh, bytes)) in super::mips::build_mip_chain(data, size, size, kind)
            .iter()
            .enumerate()
        {
            // write_texture pads rows to COPY_BYTES_PER_ROW_ALIGNMENT; the
            // small mips of a 4K texture fall below it.
            let row = lw * 4;
            let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let padded = row.div_ceil(align) * align;
            let staged: std::borrow::Cow<[u8]> = if padded == row {
                std::borrow::Cow::Borrowed(bytes)
            } else {
                let mut buf = vec![0u8; padded as usize * *lh as usize];
                for y in 0..*lh as usize {
                    let (s, d) = (y * row as usize, y * padded as usize);
                    buf[d..d + row as usize].copy_from_slice(&bytes[s..s + row as usize]);
                }
                std::borrow::Cow::Owned(buf)
            };
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: i as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &staged,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(*lh),
                },
                wgpu::Extent3d {
                    width: *lw,
                    height: *lh,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    (texture, view)
}

/// The GPU array textures shared by every layer of one terrain.
///
/// Phase 25K packs four source maps into two (see `assets/LICENSE.md`), so this
/// is two arrays rather than three: `albedo` carries colour plus **height** in
/// alpha, and `surface` carries normal XY, roughness and ambient occlusion.
pub struct TerrainLayerTextures {
    /// RGB = albedo, A = height.
    pub albedo: wgpu::Texture,
    pub albedo_view: wgpu::TextureView,
    /// R,G = normal XY (Z reconstructed), B = roughness, A = occlusion.
    pub surface: wgpu::Texture,
    pub surface_view: wgpu::TextureView,
    /// Layers 16–31, possibly at a lower resolution than the hero bank.
    pub albedo_extra: wgpu::Texture,
    pub albedo_extra_view: wgpu::TextureView,
    pub surface_extra: wgpu::Texture,
    pub surface_extra_view: wgpu::TextureView,
    /// False when the packed assets were missing and the procedural fallback
    /// was generated instead.
    pub from_assets: bool,
    /// True when the GPU arrays are BC7 rather than RGBA8 (Phase XV-E).
    /// Mutually exclusive with the RGBA8 path — never both resident.
    pub compressed: bool,
    /// Edge length of the uploaded hero arrays (layers 0–15).
    pub resolution: u32,
    /// Edge length of layers 16–31 (often 1024 to stay in the RGBA8 budget).
    pub extra_resolution: u32,
    /// Mean **linear** albedo of each layer (Phase 24L).
    ///
    /// A ray that bounces off the ground has to pick up the ground's colour,
    /// and there is no affordable way to evaluate the full eight-layer
    /// composite at a ray hit — that is a splatmap fetch, eight texture reads
    /// and a height blend, per bounce, per pixel. One mean colour per layer
    /// blended by the splat weights costs two texture reads and is right to
    /// within the layer's own variation, which is far below what a single
    /// diffuse bounce can carry.
    pub mean_albedo: [[f32; 4]; TERRAIN_LAYER_COUNT as usize],
}

/// Mean linear-space colour of an sRGB-encoded RGBA8 image.
///
/// Decoded before averaging, not after: the mean of sRGB bytes is not the sRGB
/// of the mean, and a bounce computed from the former comes out visibly too
/// bright — the error is in the same direction as the gamma curve's bow.
fn mean_linear_albedo(texels: &[u8]) -> [f32; 4] {
    if texels.is_empty() {
        return [0.5, 0.5, 0.5, 1.0];
    }
    let srgb_to_linear = |c: u8| {
        let s = f32::from(c) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    let mut sum = [0.0f64; 3];
    let n = texels.len() / 4;
    for t in texels.chunks_exact(4) {
        for c in 0..3 {
            sum[c] += f64::from(srgb_to_linear(t[c]));
        }
    }
    let inv = 1.0 / n as f64;
    [
        (sum[0] * inv) as f32,
        (sum[1] * inv) as f32,
        (sum[2] * inv) as f32,
        1.0,
    ]
}

/// Packed materials the sixteen layers load, in the layer order the rest of the
/// system assumes.
///
/// **Layer 2 stays rock** for v2 scene compatibility (legacy `cliff_layer`).
/// Phase XV-F's dedicated cliff face is **layer 14** (`rock_face_03`).
pub const LAYER_MATERIALS: [&str; TERRAIN_LAYER_COUNT as usize] = [
    "aerial_grass_rock",    // 0 grass — the default ground
    "forrest_ground_01",    // 1 forest floor
    "aerial_rocks_04",      // 2 rock — legacy cliff
    "snow_02",              // 3 snow, the high band
    "leafy_grass",          // 4 second grass, coarser
    "brown_mud",            // 5 wet soil
    "coast_sand_rocks_02",  // 6 sand, the low band
    "gravel_floor",         // 7 gravel
    "aerial_sand",          // 8 dry beach
    "coast_sand_01",        // 9 damp shoreline
    "dry_mud_field_001",    // 10 dry earth
    "cracked_red_ground",   // 11 red mineral clay
    "sparse_grass",         // 12 sparse grass
    "mossy_rock",           // 13 mossy mountain rock
    "rock_face_03",         // 14 dedicated cliff face
    "ganges_river_pebbles", // 15 talus / river stone
    "procedural_lush_lawn", // 16 — grass_path_3 failed ochre ΔE
    "leaves_forest_ground", // 17 dark conifer duff
    "aerial_rocks_01",      // 18 cool gray aerial rock
    "rock_wall_02",         // 19 dark slate wall
    "forest_ground_05",     // 20 green moss / forest carpet
    "rock_boulder_dry",     // 21 pale limestone
    "dirt_floor",           // 22 dark wet loam
    "forest_leaves_02",     // 23 pine-needle litter
    "procedural_wildgrass", // 24 — grass_path_2 failed ochre ΔE
    "forest_ground_06",     // 25 wetland / peat
    "gray_rocks",           // 26 gray granite talus
    "aerial_beach_01",      // 27 light cool dune
    "lichen_rock",          // 28 lichen rock
    "forest_floor",         // 29 autumn leaf litter
    "grassy_cobblestone",   // 30 packed pale path
    "snow_01",              // 31 hard wind-crust snow
];

/// Where the packed layer materials live, relative to the working directory.
const TERRAIN_ASSET_DIR: &str = "assets/terrain";
const TERRAIN_BC7_DIR: &str = "assets/terrain/bc7";

fn rgba8_residency_mib(size: u32, arrays: u32) -> f32 {
    // Full mip chain is 4/3 of level-0 RGBA8.
    arrays as f32 * size as f32 * size as f32 * 4.0 * (4.0 / 3.0) / (1024.0 * 1024.0)
}

fn bc7_residency_mib(size: u32, arrays: u32) -> f32 {
    rgba8_residency_mib(size, arrays) / 4.0
}

fn bc7_pack_path(material: &str, suffix: &str) -> String {
    format!("{TERRAIN_BC7_DIR}/{material}_{suffix}.bc7")
}

fn bc7_packs_complete() -> bool {
    LAYER_MATERIALS.iter().all(|m| {
        std::path::Path::new(&bc7_pack_path(m, "albedo")).is_file()
            && std::path::Path::new(&bc7_pack_path(m, "surface")).is_file()
    })
}

fn bc7_mip_bytes(w: u32, h: u32) -> usize {
    ((w.max(4) / 4) * (h.max(4) / 4) * 16) as usize
}

fn bc7_chain_bytes(size: u32) -> usize {
    let mut total = 0usize;
    let mut w = size;
    loop {
        total += bc7_mip_bytes(w, w);
        if w == 1 {
            break;
        }
        w = (w / 2).max(1);
    }
    total
}

/// Split a concatenated BC7 mip chain encoded at `size`.
fn parse_bc7_chain(bytes: &[u8], size: u32, path: &str) -> Result<Vec<Vec<u8>>, String> {
    let mut mips = Vec::new();
    let mut offset = 0usize;
    let mut w = size;
    loop {
        let len = bc7_mip_bytes(w, w);
        let end = offset
            .checked_add(len)
            .ok_or_else(|| format!("{path}: overflow"))?;
        if end > bytes.len() {
            return Err(format!(
                "{path}: truncated at mip {w}x{w} (need {end}, have {})",
                bytes.len()
            ));
        }
        mips.push(bytes[offset..end].to_vec());
        offset = end;
        if w == 1 {
            break;
        }
        w = (w / 2).max(1);
    }
    if offset != bytes.len() {
        return Err(format!(
            "{path}: leftover {} bytes after mip chain for {size}",
            bytes.len() - offset
        ));
    }
    Ok(mips)
}

/// Raw BC7 mip chain: concatenated levels, each `(w.max(4)/4)² * 16` bytes.
///
/// A file encoded at a higher power-of-two edge can satisfy a smaller load:
/// leading mips are skipped. Encoding 2048 then loading 1024 is the RGBA8
/// budget-drop case.
fn load_bc7_mips(material: &str, suffix: &str, size: u32) -> Result<Vec<Vec<u8>>, String> {
    let path = bc7_pack_path(material, suffix);
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let mut encoded = size;
    while encoded <= 4096 {
        if bytes.len() == bc7_chain_bytes(encoded) {
            let mips = parse_bc7_chain(&bytes, encoded, &path)?;
            if encoded == size {
                return Ok(mips);
            }
            let skip = (encoded.ilog2() - size.ilog2()) as usize;
            return Ok(mips[skip..].to_vec());
        }
        encoded = encoded.saturating_mul(2);
        if encoded == 0 {
            break;
        }
    }
    parse_bc7_chain(&bytes, size, &path)
}

fn create_bc7_array_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    format: wgpu::TextureFormat,
    size: u32,
    layers: &[Vec<Vec<u8>>],
) -> (wgpu::Texture, wgpu::TextureView) {
    let mip_level_count = (size as f32).log2().floor() as u32 + 1;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: layers.len() as u32,
        },
        mip_level_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (i, mips) in layers.iter().enumerate() {
        let mut w = size;
        let mut h = size;
        for (level, bytes) in mips.iter().enumerate() {
            let row_blocks = (w.max(4) / 4) * 16;
            let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let padded = row_blocks.div_ceil(align) * align;
            let rows = h.max(4) / 4;
            let staged: std::borrow::Cow<[u8]> = if padded == row_blocks {
                std::borrow::Cow::Borrowed(bytes)
            } else {
                let mut buf = vec![0u8; padded as usize * rows as usize];
                for y in 0..rows as usize {
                    let (s, d) = (y * row_blocks as usize, y * padded as usize);
                    buf[d..d + row_blocks as usize]
                        .copy_from_slice(&bytes[s..s + row_blocks as usize]);
                }
                std::borrow::Cow::Owned(buf)
            };
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: i as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &staged,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(rows),
                },
                wgpu::Extent3d {
                    // wgpu requires compressed copies to be a multiple of the
                    // 4×4 block even on 2×2 / 1×1 mips (the file already stores
                    // one block for those levels).
                    width: w.max(4),
                    height: h.max(4),
                    depth_or_array_layers: 1,
                },
            );
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    (texture, view)
}

/// Load one packed layer, resized to `size`, as RGBA8.
fn load_packed(material: &str, suffix: &str, size: u32) -> Result<Vec<u8>, String> {
    let path = format!("{TERRAIN_ASSET_DIR}/{material}_{suffix}.png");
    let img = image::open(&path).map_err(|e| format!("{path}: {e}"))?;
    let img = if img.width() == size && img.height() == size {
        img
    } else {
        img.resize_exact(size, size, image::imageops::FilterType::Lanczos3)
    };
    Ok(img.to_rgba8().into_raw())
}

fn resize_rgba(data: &[u8], src: u32, dst: u32) -> Vec<u8> {
    if src == dst {
        return data.to_vec();
    }
    let img =
        image::RgbaImage::from_raw(src, src, data.to_vec()).expect("procedural layer is RGBA8");
    image::imageops::resize(&img, dst, dst, image::imageops::FilterType::Lanczos3).into_raw()
}

fn overbudget_allowed() -> bool {
    std::env::var("SOMNIUM_TERRAIN_ALLOW_OVERBUDGET").as_deref() == Ok("1")
}

fn force_rgba8() -> bool {
    std::env::var("SOMNIUM_TERRAIN_FORCE_RGBA8").as_deref() == Ok("1")
}

fn choose_runtime_resolutions(compressed: bool) -> (u32, u32) {
    let requested = std::env::var("SOMNIUM_TERRAIN_RES")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| v.is_power_of_two() && *v >= 256)
        .unwrap_or(2048);
    let extra = 1024.min(requested);
    let hero_maps = TERRAIN_HERO_LAYERS * 2;
    let extra_maps = (TERRAIN_LAYER_COUNT - TERRAIN_HERO_LAYERS) * 2;
    if compressed {
        let mib = bc7_residency_mib(requested, hero_maps) + bc7_residency_mib(extra, extra_maps);
        // 32-layer mixed 2048+1024 is ~213 MiB. The original 16-layer 2K BC7
        // budget was 200; 220 is the 32-layer mixed ceiling so hero 2K returns.
        const BC7_BUDGET: f32 = 220.0;
        if mib > BC7_BUDGET && !overbudget_allowed() {
            let hero = 1024.min(requested);
            tracing::warn!(
                "terrain: projected {mib:.0} MiB BC7 exceeds {BC7_BUDGET:.0} MiB; loading 0–15 at {hero} and 16–31 at {extra}"
            );
            (hero, extra.min(hero))
        } else {
            tracing::info!(
                "terrain: projected {mib:.0} MiB BC7 (0–15 at {requested}, 16–31 at {extra})"
            );
            (requested, extra)
        }
    } else {
        let mib =
            rgba8_residency_mib(requested, hero_maps) + rgba8_residency_mib(extra, extra_maps);
        const BUDGET: f32 = 700.0;
        if mib > BUDGET && !overbudget_allowed() {
            let hero = 1024.min(requested);
            tracing::warn!(
                "terrain: projected {mib:.0} MiB RGBA8 exceeds 700 MiB; loading 0–15 at {hero} and 16–31 at {extra}"
            );
            (hero, extra.min(hero))
        } else {
            tracing::info!(
                "terrain: projected {mib:.0} MiB RGBA8 (0–15 at {requested}, 16–31 at {extra})"
            );
            (requested, extra)
        }
    }
}

fn procedural_pair(i: usize, size: u32) -> (Vec<u8>, Vec<u8>) {
    let (a, n, r) = generate_layer(&RECIPES[i]);
    let mut surface = Vec::with_capacity(a.len());
    for j in (0..n.len()).step_by(4) {
        surface.extend([n[j], n[j + 1], r[j], 255]);
    }
    (
        resize_rgba(&a, LAYER_TEXTURE_SIZE, size),
        resize_rgba(&surface, LAYER_TEXTURE_SIZE, size),
    )
}

/// Packed albedo+height / surface pair for one layer, resized to `size`.
///
/// Photographed PNGs win; procedural slots 16 and 24 (and any missing pack)
/// use the hash-noise fallback. Used by the RGBA8 loader and the offline
/// BC7 encoder example.
pub fn layer_packed_rgba(index: usize, size: u32) -> (Vec<u8>, Vec<u8>, bool) {
    let material = LAYER_MATERIALS[index];
    match (
        load_packed(material, "albedo", size),
        load_packed(material, "surface", size),
    ) {
        (Ok(a), Ok(s)) => (a, s, true),
        (albedo_err, surface_err) => {
            if material.starts_with("procedural_") {
                tracing::info!(
                    "terrain: layer {index} `{material}` is a procedural slot (no CC0 scan passed ΔE)"
                );
            } else {
                tracing::warn!(
                    "terrain: layer {index} `{material}` packed PNG missing ({:?} / {:?}); procedural fallback",
                    albedo_err.err(),
                    surface_err.err()
                );
            }
            let (a, s) = procedural_pair(index, size);
            (a, s, false)
        }
    }
}

fn mean_albedo_from_sources() -> [[f32; 4]; TERRAIN_LAYER_COUNT as usize] {
    std::array::from_fn(|i| {
        let (a, _, _) = layer_packed_rgba(i, 256);
        mean_linear_albedo(&a)
    })
}

fn load_rgba_bank(range: std::ops::Range<usize>, size: u32) -> (Vec<Vec<u8>>, Vec<Vec<u8>>, usize) {
    let mut albedos = Vec::with_capacity(range.len());
    let mut surfaces = Vec::with_capacity(range.len());
    let mut photographed = 0usize;
    for i in range {
        let (a, s, from_png) = layer_packed_rgba(i, size);
        if from_png {
            photographed += 1;
        }
        albedos.push(a);
        surfaces.push(s);
    }
    (albedos, surfaces, photographed)
}

impl TerrainLayerTextures {
    /// Load the packed photographed layers, falling back to procedural ones.
    ///
    /// The fallback is not a courtesy: `assets/terrain` is ~650 MB and a clone
    /// without it must still start, exactly as the glTF demo falls back to
    /// procedural cubes.
    pub fn load_or_generate(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bc_supported: bool,
    ) -> Self {
        // A 4K RGBA8 array of four layers with mips is ~350 MB per array, and
        // there are two. 2K is the default because terrain is viewed from
        // metres away, not centimetres; `SOMNIUM_TERRAIN_RES=4096` spends the
        // memory for the full detail the committed assets carry.
        // Resolution is chosen *after* knowing whether BC7 will be used:
        // RGBA8 2048+1024 is 853 MiB (drops to 1K); BC7 of the same mix is
        // ~213 MiB and keeps hero 2K. `SOMNIUM_TERRAIN_FORCE_RGBA8=1` is the
        // A/B switch once packs exist.
        let want_bc7 = bc_supported && bc7_packs_complete() && !force_rgba8();
        if force_rgba8() {
            tracing::info!("terrain: SOMNIUM_TERRAIN_FORCE_RGBA8=1; skipping BC7");
        }
        let (mut hero, mut extra) = choose_runtime_resolutions(want_bc7);

        if want_bc7 {
            match Self::load_bc7_layers(device, queue, hero, extra) {
                Ok(loaded) => {
                    tracing::info!(
                        "terrain: BC7 packs resident (hero {hero}, extra {extra}; RGBA8 not uploaded)"
                    );
                    return loaded;
                }
                Err(e) => {
                    tracing::warn!("terrain: BC7 packs unusable ({e}); RGBA8 fallback");
                    (hero, extra) = choose_runtime_resolutions(false);
                }
            }
        } else if bc_supported && !force_rgba8() {
            tracing::info!("terrain: BC7 supported but packs absent; RGBA8 residency");
        } else if !bc_supported {
            tracing::info!("terrain: BC compression unavailable; RGBA8 fallback");
        }

        match Self::load_packed_layers(device, queue, hero, extra) {
            Ok(loaded) => loaded,
            Err(e) => {
                tracing::warn!(
                    "terrain: using procedural layers ({e}). Run \
                     cargo run -p somnium_asset --example fetch_terrain then pack_terrain \
                     for the photographed set."
                );
                Self::generate_default(device, queue)
            }
        }
    }

    fn load_packed_layers(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hero: u32,
        extra: u32,
    ) -> Result<Self, String> {
        let hero_n = TERRAIN_HERO_LAYERS as usize;
        let (a0, s0, p0) = load_rgba_bank(0..hero_n, hero);
        let (a1, s1, p1) = load_rgba_bank(hero_n..TERRAIN_LAYER_COUNT as usize, extra);
        let photographed = p0 + p1;
        let mib = rgba8_residency_mib(hero, hero_n as u32 * 2)
            + rgba8_residency_mib(extra, (TERRAIN_LAYER_COUNT - TERRAIN_HERO_LAYERS) * 2);
        tracing::info!(
            "terrain: {photographed}/{} photographed layers (0–15 at {hero}x{hero}, 16–31 at {extra}x{extra}, ~{mib:.0} MiB RGBA8 mips)",
            LAYER_MATERIALS.len(),
        );

        let (albedo, albedo_view) = create_array_texture(
            device,
            queue,
            "Terrain Albedo+Height Array",
            wgpu::TextureFormat::Rgba8UnormSrgb,
            hero,
            &a0,
            super::mips::PackedKind::AlbedoHeight,
        );
        let (surface, surface_view) = create_array_texture(
            device,
            queue,
            "Terrain Surface Array",
            wgpu::TextureFormat::Rgba8Unorm,
            hero,
            &s0,
            super::mips::PackedKind::Surface,
        );
        let (albedo_extra, albedo_extra_view) = create_array_texture(
            device,
            queue,
            "Terrain Albedo+Height Extra",
            wgpu::TextureFormat::Rgba8UnormSrgb,
            extra,
            &a1,
            super::mips::PackedKind::AlbedoHeight,
        );
        let (surface_extra, surface_extra_view) = create_array_texture(
            device,
            queue,
            "Terrain Surface Extra",
            wgpu::TextureFormat::Rgba8Unorm,
            extra,
            &s1,
            super::mips::PackedKind::Surface,
        );
        let mean_albedo = std::array::from_fn(|i| {
            if i < hero_n {
                a0.get(i)
                    .map_or([0.5, 0.5, 0.5, 1.0], |a| mean_linear_albedo(a))
            } else {
                a1.get(i - hero_n)
                    .map_or([0.5, 0.5, 0.5, 1.0], |a| mean_linear_albedo(a))
            }
        });
        Ok(Self {
            albedo,
            albedo_view,
            surface,
            surface_view,
            albedo_extra,
            albedo_extra_view,
            surface_extra,
            surface_extra_view,
            from_assets: photographed == LAYER_MATERIALS.len(),
            compressed: false,
            resolution: hero,
            extra_resolution: extra,
            mean_albedo,
        })
    }

    fn load_bc7_layers(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hero: u32,
        extra: u32,
    ) -> Result<Self, String> {
        let hero_n = TERRAIN_HERO_LAYERS as usize;
        let mut a0 = Vec::with_capacity(hero_n);
        let mut s0 = Vec::with_capacity(hero_n);
        for material in &LAYER_MATERIALS[..hero_n] {
            a0.push(load_bc7_mips(material, "albedo", hero)?);
            s0.push(load_bc7_mips(material, "surface", hero)?);
        }
        let mut a1 = Vec::with_capacity(LAYER_MATERIALS.len() - hero_n);
        let mut s1 = Vec::with_capacity(LAYER_MATERIALS.len() - hero_n);
        for material in &LAYER_MATERIALS[hero_n..] {
            a1.push(load_bc7_mips(material, "albedo", extra)?);
            s1.push(load_bc7_mips(material, "surface", extra)?);
        }
        let (albedo, albedo_view) = create_bc7_array_texture(
            device,
            queue,
            "Terrain Albedo+Height Array BC7",
            wgpu::TextureFormat::Bc7RgbaUnormSrgb,
            hero,
            &a0,
        );
        let (surface, surface_view) = create_bc7_array_texture(
            device,
            queue,
            "Terrain Surface Array BC7",
            wgpu::TextureFormat::Bc7RgbaUnorm,
            hero,
            &s0,
        );
        let (albedo_extra, albedo_extra_view) = create_bc7_array_texture(
            device,
            queue,
            "Terrain Albedo+Height Extra BC7",
            wgpu::TextureFormat::Bc7RgbaUnormSrgb,
            extra,
            &a1,
        );
        let (surface_extra, surface_extra_view) = create_bc7_array_texture(
            device,
            queue,
            "Terrain Surface Extra BC7",
            wgpu::TextureFormat::Bc7RgbaUnorm,
            extra,
            &s1,
        );
        tracing::info!(
            "terrain: BC7 residency ~{:.0} MiB (hero {hero}, extra {extra})",
            bc7_residency_mib(hero, hero_n as u32 * 2)
                + bc7_residency_mib(extra, (TERRAIN_LAYER_COUNT - TERRAIN_HERO_LAYERS) * 2),
        );
        Ok(Self {
            albedo,
            albedo_view,
            surface,
            surface_view,
            albedo_extra,
            albedo_extra_view,
            surface_extra,
            surface_extra_view,
            from_assets: true,
            compressed: true,
            resolution: hero,
            extra_resolution: extra,
            mean_albedo: mean_albedo_from_sources(),
        })
    }

    /// Generate procedural layers and upload them as two 16-layer banks.
    pub fn generate_default(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let size = LAYER_TEXTURE_SIZE;
        let hero_n = TERRAIN_HERO_LAYERS as usize;
        let mut albedos = Vec::new();
        let mut surfaces = Vec::new();
        for recipe in &RECIPES {
            let (a, n, r) = generate_layer(recipe);
            let mut surface = Vec::with_capacity(a.len());
            for i in (0..n.len()).step_by(4) {
                surface.extend([n[i], n[i + 1], r[i], 255]);
            }
            albedos.push(a);
            surfaces.push(surface);
        }
        let (albedo, albedo_view) = create_array_texture(
            device,
            queue,
            "Terrain Albedo+Height Array",
            wgpu::TextureFormat::Rgba8UnormSrgb,
            size,
            &albedos[..hero_n],
            super::mips::PackedKind::AlbedoHeight,
        );
        let (surface, surface_view) = create_array_texture(
            device,
            queue,
            "Terrain Surface Array",
            wgpu::TextureFormat::Rgba8Unorm,
            size,
            &surfaces[..hero_n],
            super::mips::PackedKind::Surface,
        );
        let (albedo_extra, albedo_extra_view) = create_array_texture(
            device,
            queue,
            "Terrain Albedo+Height Extra",
            wgpu::TextureFormat::Rgba8UnormSrgb,
            size,
            &albedos[hero_n..],
            super::mips::PackedKind::AlbedoHeight,
        );
        let (surface_extra, surface_extra_view) = create_array_texture(
            device,
            queue,
            "Terrain Surface Extra",
            wgpu::TextureFormat::Rgba8Unorm,
            size,
            &surfaces[hero_n..],
            super::mips::PackedKind::Surface,
        );
        let mean_albedo = std::array::from_fn(|i| {
            albedos
                .get(i)
                .map_or([0.5, 0.5, 0.5, 1.0], |a| mean_linear_albedo(a))
        });
        Self {
            albedo,
            albedo_view,
            surface,
            surface_view,
            albedo_extra,
            albedo_extra_view,
            surface_extra,
            surface_extra_view,
            from_assets: false,
            compressed: false,
            resolution: size,
            extra_resolution: size,
            mean_albedo,
        }
    }
}

/// Eight RGBA weight textures controlling thirty-two-layer blending (XV-Zeta).
///
/// Channels of each map weight four consecutive layers. The CPU copy is the
/// paint target; dirty regions are re-uploaded with `upload_dirty`.
pub struct Splatmap {
    pub textures: [wgpu::Texture; super::splat::SPLAT_MAP_COUNT],
    pub views: [wgpu::TextureView; super::splat::SPLAT_MAP_COUNT],
    /// CPU copy for painting: one weight per layer, row-major.
    pub data: Vec<SplatTexel>,
    pub width: u32,
    pub height: u32,
    /// Dirty texel region `(x_min, z_min, x_max, z_max)` inclusive, if any.
    pub dirty: Option<(u32, u32, u32, u32)>,
}

impl Splatmap {
    /// Create a splatmap fully weighted to layer 0 (grass).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) -> Self {
        let mut first = [0u8; TERRAIN_LAYER_COUNT as usize];
        first[0] = 255;
        let data = vec![first; (width * height) as usize];
        let labels = [
            "Terrain Splatmap 0-3",
            "Terrain Splatmap 4-7",
            "Terrain Splatmap 8-11",
            "Terrain Splatmap 12-15",
            "Terrain Splatmap 16-19",
            "Terrain Splatmap 20-23",
            "Terrain Splatmap 24-27",
            "Terrain Splatmap 28-31",
        ];
        let textures: [wgpu::Texture; super::splat::SPLAT_MAP_COUNT] = std::array::from_fn(|i| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(labels[i]),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        });
        let views: [wgpu::TextureView; super::splat::SPLAT_MAP_COUNT] = std::array::from_fn(|i| {
            textures[i].create_view(&wgpu::TextureViewDescriptor::default())
        });
        let mut splat = Self {
            textures,
            views,
            data,
            width,
            height,
            dirty: None,
        };
        splat.mark_dirty(0, 0, width - 1, height - 1);
        splat.upload_dirty(queue);
        splat
    }

    /// Grow the dirty rectangle to include the given texel region.
    pub fn mark_dirty(&mut self, x0: u32, z0: u32, x1: u32, z1: u32) {
        let r = (x0, z0, x1.min(self.width - 1), z1.min(self.height - 1));
        self.dirty = Some(match self.dirty {
            None => r,
            Some(d) => (d.0.min(r.0), d.1.min(r.1), d.2.max(r.2), d.3.max(r.3)),
        });
    }

    /// Upload the dirty region to all splat textures (whole rows).
    pub fn upload_dirty(&mut self, queue: &wgpu::Queue) {
        let Some((_, z0, _, z1)) = self.dirty.take() else {
            return;
        };
        let rows = z1 - z0 + 1;
        let offset = (z0 * self.width) as usize;
        let texels = (rows * self.width) as usize;
        let slice = &self.data[offset..offset + texels];

        let mut groups: [Vec<u8>; super::splat::SPLAT_MAP_COUNT] =
            std::array::from_fn(|_| Vec::with_capacity(texels * 4));
        for texel in slice {
            let g = super::splat::deinterleave(texel);
            for i in 0..super::splat::SPLAT_MAP_COUNT {
                groups[i].extend_from_slice(&g[i]);
            }
        }

        for (texture, bytes) in self.textures.iter().zip(groups.iter()) {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: z0, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.width * 4),
                    rows_per_image: Some(rows),
                },
                wgpu::Extent3d {
                    width: self.width,
                    height: rows,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
}

#[cfg(test)]
mod surface_pack_tests {
    use super::*;

    /// The shipped surface packs must carry usable ambient occlusion in alpha.
    ///
    /// `evaluate_terrain_material` reads `surf.a` as the layer's AO and blends
    /// it into `surface.occlusion`, which multiplies **both** halves of
    /// `evaluate_ibl`. An all-zero alpha channel therefore does not read as
    /// "slightly darker" — it removes every scrap of indirect light from the
    /// ground while leaving direct sun untouched, and the only way to see it is
    /// Dbg 8, which renders the terrain pure black.
    ///
    /// The procedural fallback writes 255 here, so this can only ever be wrong
    /// for the packed assets — which is exactly why it needs pinning.
    #[test]
    fn shipped_surface_packs_carry_occlusion_in_alpha() {
        let dir = std::path::Path::new("../../").join(TERRAIN_ASSET_DIR);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("skipping: {} not readable from the test cwd", dir.display());
            return;
        };
        let mut checked = 0usize;
        let mut dead = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if !name.ends_with("_surface.png") {
                continue;
            }
            let Ok(img) = image::open(&path) else {
                continue;
            };
            let rgba = img.to_rgba8();
            let max_alpha = rgba.pixels().map(|p| p.0[3]).max().unwrap_or(0);
            let mean_alpha = rgba
                .pixels()
                .map(|p| u64::from(p.0[3]))
                .sum::<u64>()
                .checked_div(rgba.pixels().len() as u64)
                .unwrap_or(0);
            checked += 1;
            // A real AO map is mostly bright with darker creases. Anything whose
            // *maximum* alpha is near zero is not AO at all.
            if max_alpha < 8 {
                dead.push(format!("{name} (max alpha {max_alpha}, mean {mean_alpha})"));
            }
        }
        assert!(
            checked > 0,
            "no surface packs found under {}",
            dir.display()
        );
        assert!(
            dead.is_empty(),
            "{} of {checked} surface packs have no occlusion in alpha, which zeroes \
             terrain indirect light: {dead:#?}",
            dead.len(),
        );
    }
}

#[cfg(test)]
mod bc7_pack_tests {
    use super::*;

    #[test]
    fn bc7_chain_bytes_counts_4x4_blocks_through_1x1() {
        // 4×4, 2×2, 1×1 each occupy one 16-byte block.
        assert_eq!(bc7_chain_bytes(4), 16 * 3);
        // 8×8 is four blocks, then the 4/2/1 tail.
        assert_eq!(bc7_chain_bytes(8), 16 * (4 + 1 + 1 + 1));
    }

    #[test]
    fn larger_encode_tail_is_the_smaller_chain() {
        assert_eq!(bc7_chain_bytes(8) - bc7_mip_bytes(8, 8), bc7_chain_bytes(4));
        assert_eq!(
            bc7_chain_bytes(2048) - bc7_mip_bytes(2048, 2048),
            bc7_chain_bytes(1024)
        );
    }

    #[test]
    fn parse_bc7_chain_rejects_leftover() {
        let mut bytes = vec![0u8; bc7_chain_bytes(4) + 1];
        assert!(parse_bc7_chain(&bytes, 4, "t.bc7").is_err());
        bytes.pop();
        assert_eq!(parse_bc7_chain(&bytes, 4, "t.bc7").unwrap().len(), 3);
    }
}
