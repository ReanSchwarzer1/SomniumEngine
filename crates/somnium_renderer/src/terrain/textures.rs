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
/// Number of material layers (one RGBA splatmap channel each).
pub const TERRAIN_LAYER_COUNT: u32 = 4;

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
    LayerRecipe { tone_a: [0.16, 0.34, 0.10], tone_b: [0.28, 0.46, 0.14], roughness: 0.90, bump: 0.6, seed: 11 },
    // 1: Dirt
    LayerRecipe { tone_a: [0.30, 0.21, 0.13], tone_b: [0.42, 0.31, 0.20], roughness: 0.95, bump: 0.8, seed: 23 },
    // 2: Rock (also the triplanar cliff layer)
    LayerRecipe { tone_a: [0.32, 0.32, 0.33], tone_b: [0.52, 0.51, 0.50], roughness: 0.80, bump: 1.0, seed: 37 },
    // 3: Snow
    LayerRecipe { tone_a: [0.82, 0.85, 0.90], tone_b: [0.95, 0.96, 1.00], roughness: 0.35, bump: 0.3, seed: 53 },
];

/// Names matching the recipe order, used by the layer-management UI.
pub const LAYER_NAMES: [&str; TERRAIN_LAYER_COUNT as usize] = ["Grass", "Dirt", "Rock", "Snow"];

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
            let dx = (noise_height(u + e, v, recipe) - noise_height(u - e, v, recipe)) * recipe.bump * 8.0;
            let dz = (noise_height(u, v + e, recipe) - noise_height(u, v - e, recipe)) * recipe.bump * 8.0;
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

fn create_array_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    format: wgpu::TextureFormat,
    layers: &[Vec<u8>],
) -> (wgpu::Texture, wgpu::TextureView) {
    let size = LAYER_TEXTURE_SIZE;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: layers.len() as u32 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (i, data) in layers.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: 0, z: i as u32 },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        );
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    (texture, view)
}

/// The three GPU array textures shared by every layer of one terrain.
pub struct TerrainLayerTextures {
    pub albedo: wgpu::Texture,
    pub albedo_view: wgpu::TextureView,
    pub normal: wgpu::Texture,
    pub normal_view: wgpu::TextureView,
    pub roughness: wgpu::Texture,
    pub roughness_view: wgpu::TextureView,
}

impl TerrainLayerTextures {
    /// Generate the four default procedural layers and upload them.
    pub fn generate_default(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let mut albedos = Vec::new();
        let mut normals = Vec::new();
        let mut roughs = Vec::new();
        for recipe in &RECIPES {
            let (a, n, r) = generate_layer(recipe);
            albedos.push(a);
            normals.push(n);
            roughs.push(r);
        }
        let (albedo, albedo_view) = create_array_texture(
            device, queue, "Terrain Albedo Array", wgpu::TextureFormat::Rgba8UnormSrgb, &albedos,
        );
        let (normal, normal_view) = create_array_texture(
            device, queue, "Terrain Normal Array", wgpu::TextureFormat::Rgba8Unorm, &normals,
        );
        let (roughness, roughness_view) = create_array_texture(
            device, queue, "Terrain Roughness Array", wgpu::TextureFormat::Rgba8Unorm, &roughs,
        );
        Self { albedo, albedo_view, normal, normal_view, roughness, roughness_view }
    }
}

/// RGBA weight texture controlling layer blending (Phase 14A-2 `Splatmap`).
///
/// One terrain-global splatmap (texel grid aligned to the heightmap cells);
/// channels R/G/B/A weight layers 0-3. The CPU copy is the paint target;
/// dirty regions are re-uploaded with `upload_dirty`.
pub struct Splatmap {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// CPU copy for painting, row-major `[r, g, b, a]` per texel.
    pub data: Vec<[u8; 4]>,
    pub width: u32,
    pub height: u32,
    /// Dirty texel region `(x_min, z_min, x_max, z_max)` inclusive, if any.
    pub dirty: Option<(u32, u32, u32, u32)>,
}

impl Splatmap {
    /// Create a splatmap fully weighted to layer 0 (grass).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) -> Self {
        let data = vec![[255u8, 0, 0, 0]; (width * height) as usize];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Terrain Splatmap"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut splat = Self { texture, view, data, width, height, dirty: None };
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

    /// Upload the dirty region (whole rows — keeps the copy layout simple).
    pub fn upload_dirty(&mut self, queue: &wgpu::Queue) {
        let Some((_, z0, _, z1)) = self.dirty.take() else { return };
        let rows = z1 - z0 + 1;
        let offset = (z0 * self.width) as usize;
        let slice = &self.data[offset..offset + (rows * self.width) as usize];
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: z0, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(slice),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width * 4),
                rows_per_image: Some(rows),
            },
            wgpu::Extent3d { width: self.width, height: rows, depth_or_array_layers: 1 },
        );
    }
}
