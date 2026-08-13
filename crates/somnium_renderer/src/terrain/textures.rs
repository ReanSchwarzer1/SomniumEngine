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
/// Phase XV: sixteen global materials, carried by **four** RGBA8 splatmaps.
/// At most four weights are stored per texel (see [`super::splat`]); the shader
/// selects the strongest four before PBR sampling. Indices 0–7 stay
/// compatibility-locked with Phase 25L.
pub const TERRAIN_LAYER_COUNT: u32 = 16;

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
    /// False when the packed assets were missing and the procedural fallback
    /// was generated instead.
    pub from_assets: bool,
    /// True when the GPU arrays are BC7 rather than RGBA8 (Phase XV-E).
    /// Mutually exclusive with the RGBA8 path — never both resident.
    pub compressed: bool,
    /// Edge length of the uploaded arrays.
    pub resolution: u32,
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

/// Raw BC7 mip chain: concatenated levels, each `(w/4)*(h/4)*16` bytes.
fn load_bc7_mips(material: &str, suffix: &str, size: u32) -> Result<Vec<Vec<u8>>, String> {
    let path = bc7_pack_path(material, suffix);
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let mut mips = Vec::new();
    let mut offset = 0usize;
    let mut w = size;
    let mut h = size;
    loop {
        let blocks = ((w.max(4) / 4) * (h.max(4) / 4)) as usize;
        let len = blocks * 16;
        let end = offset
            .checked_add(len)
            .ok_or_else(|| format!("{path}: overflow"))?;
        if end > bytes.len() {
            return Err(format!(
                "{path}: truncated at mip {w}x{h} (need {end}, have {})",
                bytes.len()
            ));
        }
        mips.push(bytes[offset..end].to_vec());
        offset = end;
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    if offset != bytes.len() {
        return Err(format!(
            "{path}: leftover {} bytes after mip chain for {size}",
            bytes.len() - offset
        ));
    }
    Ok(mips)
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
                    width: w,
                    height: h,
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
        let size = std::env::var("SOMNIUM_TERRAIN_RES")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| v.is_power_of_two() && *v >= 256)
            .unwrap_or(2048);

        if bc_supported && bc7_packs_complete() {
            match Self::load_bc7_layers(device, queue, size) {
                Ok(loaded) => {
                    tracing::info!(
                        "terrain: BC7 packs resident at {size}x{size} (RGBA8 not uploaded)"
                    );
                    return loaded;
                }
                Err(e) => tracing::warn!("terrain: BC7 packs unusable ({e}); RGBA8 fallback"),
            }
        } else if bc_supported {
            tracing::info!("terrain: BC7 supported but packs absent; RGBA8 residency");
        } else {
            tracing::info!("terrain: BC compression unavailable; RGBA8 fallback");
        }

        match Self::load_packed_layers(device, queue, size) {
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
        size: u32,
    ) -> Result<Self, String> {
        let mut albedos = Vec::with_capacity(LAYER_MATERIALS.len());
        let mut surfaces = Vec::with_capacity(LAYER_MATERIALS.len());
        let mut photographed = 0usize;
        for (i, material) in LAYER_MATERIALS.iter().enumerate() {
            match (
                load_packed(material, "albedo", size),
                load_packed(material, "surface", size),
            ) {
                (Ok(a), Ok(s)) => {
                    photographed += 1;
                    albedos.push(a);
                    surfaces.push(s);
                }
                (albedo_err, surface_err) => {
                    tracing::warn!(
                        "terrain: layer {i} `{material}` packed PNG missing ({:?} / {:?}); procedural fallback",
                        albedo_err.err(),
                        surface_err.err()
                    );
                    let (a, n, r) = generate_layer(&RECIPES[i]);
                    let mut surface = Vec::with_capacity(a.len());
                    for j in (0..n.len()).step_by(4) {
                        surface.extend([n[j], n[j + 1], r[j], 255]);
                    }
                    // Procedural maps are LAYER_TEXTURE_SIZE; resize to `size`.
                    albedos.push(resize_rgba(&a, LAYER_TEXTURE_SIZE, size));
                    surfaces.push(resize_rgba(&surface, LAYER_TEXTURE_SIZE, size));
                }
            }
        }
        let from_assets = photographed == LAYER_MATERIALS.len();
        tracing::info!(
            "terrain: {photographed}/{} photographed layers at {size}x{size} (~{:.0} MiB RGBA8 mips)",
            LAYER_MATERIALS.len(),
            rgba8_residency_mib(size, LAYER_MATERIALS.len() as u32 * 2),
        );

        // Albedo is sRGB; the surface pack is linear data — a normal, a
        // roughness and an occlusion, none of which are colours.
        let (albedo, albedo_view) = create_array_texture(
            device,
            queue,
            "Terrain Albedo+Height Array",
            wgpu::TextureFormat::Rgba8UnormSrgb,
            size,
            &albedos,
            super::mips::PackedKind::AlbedoHeight,
        );
        let (surface, surface_view) = create_array_texture(
            device,
            queue,
            "Terrain Surface Array",
            wgpu::TextureFormat::Rgba8Unorm,
            size,
            &surfaces,
            super::mips::PackedKind::Surface,
        );
        let mean_albedo = std::array::from_fn(|i| {
            albedos
                .get(i)
                .map_or([0.5, 0.5, 0.5, 1.0], |a| mean_linear_albedo(a))
        });
        Ok(Self {
            albedo,
            albedo_view,
            surface,
            surface_view,
            from_assets,
            compressed: false,
            resolution: size,
            mean_albedo,
        })
    }

    fn load_bc7_layers(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: u32,
    ) -> Result<Self, String> {
        let mut albedos = Vec::with_capacity(LAYER_MATERIALS.len());
        let mut surfaces = Vec::with_capacity(LAYER_MATERIALS.len());
        for material in LAYER_MATERIALS {
            albedos.push(load_bc7_mips(material, "albedo", size)?);
            surfaces.push(load_bc7_mips(material, "surface", size)?);
        }
        let (albedo, albedo_view) = create_bc7_array_texture(
            device,
            queue,
            "Terrain Albedo+Height Array BC7",
            wgpu::TextureFormat::Bc7RgbaUnormSrgb,
            size,
            &albedos,
        );
        let (surface, surface_view) = create_bc7_array_texture(
            device,
            queue,
            "Terrain Surface Array BC7",
            wgpu::TextureFormat::Bc7RgbaUnorm,
            size,
            &surfaces,
        );
        tracing::info!(
            "terrain: BC7 residency ~{:.0} MiB at {size}x{size}",
            bc7_residency_mib(size, LAYER_MATERIALS.len() as u32 * 2),
        );
        Ok(Self {
            albedo,
            albedo_view,
            surface,
            surface_view,
            from_assets: true,
            compressed: true,
            resolution: size,
            mean_albedo: [[0.5, 0.5, 0.5, 1.0]; TERRAIN_LAYER_COUNT as usize],
        })
    }

    /// Generate the four default procedural layers and upload them.
    pub fn generate_default(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let mut albedos = Vec::new();
        let mut surfaces = Vec::new();
        for recipe in &RECIPES {
            let (a, n, r) = generate_layer(recipe);
            // Match the packed layout the shader expects: the procedural
            // albedo already carries its noise "height" in alpha, and the
            // surface pack is normal XY, roughness, and a fully-open occlusion.
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
            LAYER_TEXTURE_SIZE,
            &albedos,
            super::mips::PackedKind::AlbedoHeight,
        );
        let (surface, surface_view) = create_array_texture(
            device,
            queue,
            "Terrain Surface Array",
            wgpu::TextureFormat::Rgba8Unorm,
            LAYER_TEXTURE_SIZE,
            &surfaces,
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
            from_assets: false,
            compressed: false,
            resolution: LAYER_TEXTURE_SIZE,
            mean_albedo,
        }
    }
}

/// Four RGBA weight textures controlling sixteen-layer blending (Phase XV).
///
/// Channels of each map weight four consecutive layers. The CPU copy is the
/// paint target; dirty regions are re-uploaded with `upload_dirty`.
pub struct Splatmap {
    /// Layers 0-3.
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// Layers 4-7 (Phase 25L).
    pub texture_hi: wgpu::Texture,
    pub view_hi: wgpu::TextureView,
    /// Layers 8-11 (Phase XV).
    pub texture_2: wgpu::Texture,
    pub view_2: wgpu::TextureView,
    /// Layers 12-15 (Phase XV).
    pub texture_3: wgpu::Texture,
    pub view_3: wgpu::TextureView,
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
        let make = |label: &str| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
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
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        };
        let (texture, view) = make("Terrain Splatmap 0-3");
        let (texture_hi, view_hi) = make("Terrain Splatmap 4-7");
        let (texture_2, view_2) = make("Terrain Splatmap 8-11");
        let (texture_3, view_3) = make("Terrain Splatmap 12-15");
        let mut splat = Self {
            texture,
            view,
            texture_hi,
            view_hi,
            texture_2,
            view_2,
            texture_3,
            view_3,
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

    /// Upload the dirty region to all four splat textures (whole rows).
    pub fn upload_dirty(&mut self, queue: &wgpu::Queue) {
        let Some((_, z0, _, z1)) = self.dirty.take() else {
            return;
        };
        let rows = z1 - z0 + 1;
        let offset = (z0 * self.width) as usize;
        let texels = (rows * self.width) as usize;
        let slice = &self.data[offset..offset + texels];

        let mut groups = [
            Vec::with_capacity(texels * 4),
            Vec::with_capacity(texels * 4),
            Vec::with_capacity(texels * 4),
            Vec::with_capacity(texels * 4),
        ];
        for texel in slice {
            let g = super::splat::deinterleave(texel);
            for i in 0..4 {
                groups[i].extend_from_slice(&g[i]);
            }
        }

        let textures = [
            &self.texture,
            &self.texture_hi,
            &self.texture_2,
            &self.texture_3,
        ];
        for (texture, bytes) in textures.iter().zip(groups.iter()) {
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
